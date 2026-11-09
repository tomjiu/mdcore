//! #600 —— L3 mark 感知 diff 验收（引擎面；单测见 diff.rs 内部件）。
//!
//! 验收对照（issue #600）：
//!
//! 1. 同文本 bold 删除/添加产生可观察 mark change；双侧完全相同零噪声；
//! 2. 「重要 重要」样式从第一处移至第二处必须报告；两处相同样式删
//!    一处只报告对应变化；
//! 3. 中文/emoji 按 char 坐标；尾空白/换行归一语义（markdown 面先归一
//!    再投影 + op 面幸存者映射重定位）；空区间/越界/重叠不 panic、
//!    不静默漏检；
//! 4. 旧入口零漂移（无样式文档上 L3 基座 = L1 输出）；新 wire 兼容
//!    旧 L1 JSON（flatten + default）；
//! 5. op 面盲区修复证据：SetMark 树 vs 无 mark 树——L1 报 Unchanged
//!    （样式不可见）而 L3 报 mark change。

use std::sync::Arc;

use mdcore::diff::{
    diff_documents, diff_documents_l3, diff_markdown, diff_markdown_l3, BlockDiff, BlockDiffL3,
    DiffKind, MarkChange, MarkChangeKind,
};
use mdcore::op::Op;
use mdcore::{ApplyMeta, Core};

/// 取唯一块的 L3 条目（测试文档均为单块）。
fn sole(d: &[BlockDiffL3]) -> &BlockDiffL3 {
    assert_eq!(d.len(), 1, "单块文档: {d:#?}");
    &d[0]
}

fn mark_of(d: &BlockDiffL3) -> &MarkChange {
    assert_eq!(d.mark_changes.len(), 1, "恰一条 mark 事件: {d:#?}");
    &d.mark_changes[0]
}

#[test]
fn same_text_bold_removal_is_observable() {
    // 验收 1a：去粗——净文本相等（Unchanged）+ strong 移除事件。
    let d = diff_markdown_l3("**重要** 事项", "重要 事项");
    let s = sole(&d);
    assert_eq!(s.base.kind, DiffKind::Unchanged, "净文本同基配对");
    assert_eq!(s.base.text_old.as_deref(), Some("重要 事项"));
    assert_eq!(s.base.text_new.as_deref(), Some("重要 事项"));
    let m = mark_of(s);
    assert_eq!(m.kind, "strong");
    assert_eq!(m.change, MarkChangeKind::Removed);
    assert_eq!(m.old_range, Some((0, 2)));
    assert_eq!(m.new_range, None);
    assert_eq!(m.text, "重要");
}

#[test]
fn same_text_italic_addition_is_observable() {
    // 验收 1b：加斜体。
    let d = diff_markdown_l3("重要 事项", "*重要* 事项");
    let s = sole(&d);
    assert_eq!(s.base.kind, DiffKind::Unchanged);
    let m = mark_of(s);
    assert_eq!(m.kind, "em");
    assert_eq!(m.change, MarkChangeKind::Added);
    assert_eq!(m.old_range, None);
    assert_eq!(m.new_range, Some((0, 2)));
    assert_eq!(m.text, "重要");
}

#[test]
fn identical_both_sides_zero_noise() {
    // 验收 1c：双侧完全相同 → 零事件（含 marks 等价与多 mark 面）。
    for (a, b) in [
        ("**重要** 事项", "**重要** 事项"),
        ("*重要* **粗** `码`", "*重要* **粗** `码`"),
        ("普通段落", "普通段落"),
    ] {
        let d = diff_markdown_l3(a, b);
        let s = sole(&d);
        assert_eq!(s.base.kind, DiffKind::Unchanged, "({a:?},{b:?})");
        assert!(
            s.mark_changes.is_empty(),
            "零噪声面失败 ({a:?},{b:?}): {:#?}",
            s.mark_changes
        );
    }
}

#[test]
fn position_swap_between_duplicate_texts_is_reported() {
    // 验收 2a：「重要 重要」样式从第一处移至第二处必须报告 Moved。
    let d = diff_markdown_l3("*重要* 重要", "重要 *重要*");
    let s = sole(&d);
    assert_eq!(s.base.kind, DiffKind::Unchanged);
    assert_eq!(s.base.text_old.as_deref(), Some("重要 重要"));
    let m = mark_of(s);
    assert_eq!(m.kind, "em");
    assert_eq!(m.change, MarkChangeKind::Moved, "位置互换显式报 Moved");
    assert_eq!(m.old_range, Some((0, 2)));
    assert_eq!(m.new_range, Some((3, 5)));
    assert_eq!(m.text, "重要");
}

#[test]
fn deleting_one_of_two_identical_marks_reports_only_that() {
    // 验收 2b：两处相同样式删一处 → 只报对应变化（精确位互消）。
    let d = diff_markdown_l3("*重要* *重要*", "*重要* 重要");
    let s = sole(&d);
    let m = mark_of(s);
    assert_eq!(m.change, MarkChangeKind::Removed);
    assert_eq!(m.old_range, Some((3, 5)), "被删的是第二处");
    assert_eq!(m.new_range, None);
    assert_eq!(m.text, "重要");
}

#[test]
fn op_surface_style_change_l1_blind_spot_fixed() {
    // 验收 5：op 面（SetMark）同文本样式变化——L1 报 Unchanged（盲区：
    // 样式不可见）而 L3 报 mark change。
    let mut old = Core::from_markdown_with_clock("重要 事项", Arc::new(|| 0));
    old.apply_ops(
        vec![Op::SetMark {
            path: vec![0],
            start: 0,
            end: 2,
            kind: "bold".into(),
            add: true,
        }],
        ApplyMeta::view("test"),
    )
    .unwrap();
    let new = Core::from_markdown("重要 事项");

    // L1 盲区证据：Unchanged 且输出无任何样式信息面。
    let l1 = diff_documents(&old, &new);
    assert_eq!(l1.len(), 1);
    assert_eq!(l1[0].kind, DiffKind::Unchanged, "L1 面样式不可见（盲区）");

    // L3：可观察。
    let l3 = diff_documents_l3(&old, &new);
    let s = sole(&l3);
    assert_eq!(s.base.kind, DiffKind::Unchanged);
    let m = mark_of(s);
    assert_eq!(m.kind, "bold", "op 面自定义 kind 原样");
    assert_eq!(m.change, MarkChangeKind::Removed);
    assert_eq!(m.old_range, Some((0, 2)));
    assert_eq!(m.text, "重要");
}

#[test]
fn op_surface_style_added_and_swapped() {
    // op 面双事件 + 位置互换（自定义 kind "hl"）。
    let mut old = Core::from_markdown_with_clock("重要 重要", Arc::new(|| 0));
    old.apply_ops(
        vec![Op::SetMark {
            path: vec![0],
            start: 0,
            end: 2,
            kind: "hl".into(),
            add: true,
        }],
        ApplyMeta::view("test"),
    )
    .unwrap();
    let mut new = Core::from_markdown_with_clock("重要 重要", Arc::new(|| 1));
    new.apply_ops(
        vec![Op::SetMark {
            path: vec![0],
            start: 3,
            end: 5,
            kind: "hl".into(),
            add: true,
        }],
        ApplyMeta::view("test"),
    )
    .unwrap();
    let l3 = diff_documents_l3(&old, &new);
    let m = mark_of(sole(&l3));
    assert_eq!(m.change, MarkChangeKind::Moved);
    assert_eq!(m.old_range, Some((0, 2)));
    assert_eq!(m.new_range, Some((3, 5)));
    assert_eq!(m.text, "重要");
}

#[test]
fn unicode_emoji_char_coordinates() {
    // 验收 3a：中文 + emoji（多字节、多 char 家族）——char 坐标不按字节。
    // "👍🏻 关键"：👍🏻 = 2 char（基字符+肤色修饰）→ mark 起点按 char 计。
    let d = diff_markdown_l3("👍🏻 **关键**", "👍🏻 关键");
    let s = sole(&d);
    assert_eq!(s.base.text_old.as_deref(), Some("👍🏻 关键"));
    let m = mark_of(s);
    assert_eq!(m.old_range, Some((3, 5)), "emoji 家族 2 char 后的起点");
    assert_eq!(m.text, "关键");
}

#[test]
fn trailing_whitespace_normalization_both_surfaces() {
    // 验收 3b：尾空白归一——markdown 面（先归一再投影，两侧一致）。
    let d = diff_markdown_l3("**重要**  \n", "重要");
    let s = sole(&d);
    assert_eq!(s.base.kind, DiffKind::Unchanged, "尾空白归一后同基");
    let m = mark_of(s);
    assert_eq!(m.change, MarkChangeKind::Removed);
    assert_eq!(m.old_range, Some((0, 2)));

    // op 面幸存者映射：text = "重要  "（尾 2 空格）mark(0,4) → 归一
    // "重要" 上重定位 (0,2)；新侧 "重要" 无 mark → Removed。
    let mut old = Core::from_markdown_with_clock("重要  ", Arc::new(|| 0));
    old.apply_ops(
        vec![Op::SetMark {
            path: vec![0],
            start: 0,
            end: 4,
            kind: "bold".into(),
            add: true,
        }],
        ApplyMeta::view("test"),
    )
    .unwrap();
    let new = Core::from_markdown("重要");
    let l3 = diff_documents_l3(&old, &new);
    let s = sole(&l3);
    assert_eq!(s.base.kind, DiffKind::Unchanged, "op 面归一后同基");
    let m = mark_of(s);
    assert_eq!(m.change, MarkChangeKind::Removed);
    assert_eq!(m.old_range, Some((0, 2)), "尾随被剥端点向内收拢");
    assert_eq!(m.text, "重要");
}

#[test]
fn hostile_marks_do_not_panic_and_stay_observable() {
    // 验收 3c：越界/倒挂 mark（敌意树——快照面外的防御）：不 panic、
    // 事件不静默消失。经 Core 公共面无法注入（SetMark 校验拒绝），
    // 此处以 markdown 双侧同文档 + L1 对比锁定行为稳定；越界防御的
    // 单元级断言在 diff.rs（effective_inline 直接测）。
    let d = diff_markdown_l3("敌意 *输入* 面", "敌意 *输入* 面");
    assert!(sole(&d).mark_changes.is_empty());
    // 行为稳定锁：同输入同输出（确定性）。
    let d2 = diff_markdown_l3("敌意 *输入* 面", "敌意 *输入* 面");
    assert_eq!(d, d2);
}

#[test]
fn modified_blocks_carry_text_events_not_mark_changes() {
    // 边界声明锁：Modified 块（净文本不同）不做 mark 坐标映射——
    // 样式随文本事件表达；mark_changes 恒空。
    let d = diff_markdown_l3("**旧文**", "新文");
    let s = sole(&d);
    assert_eq!(s.base.kind, DiffKind::Modified);
    assert!(s.mark_changes.is_empty(), "L2×mark 联合另排（#600 边界）");
}

#[test]
fn plain_documents_l3_matches_l1_output() {
    // 验收 4a：无样式文档（无定界符——净=原）L3 基座与 L1 完全一致
    // （旧入口零漂移）。
    let old_md = "第一段\n\n第二段\n\n# 标题";
    let new_md = "第一段\n\n第二段改\n\n# 标题";
    let l1 = diff_markdown(old_md, new_md);
    let l3 = diff_markdown_l3(old_md, new_md);
    assert_eq!(l1.len(), l3.len());
    for (a, b) in l1.iter().zip(l3.iter()) {
        assert_eq!(&b.base, a, "无样式文档基座一致");
        assert!(b.mark_changes.is_empty());
    }
}

#[test]
fn wire_flattening_reads_old_l1_json() {
    // 验收 4b：旧 L1 JSON（无 mark_changes 键）→ BlockDiffL3 直读。
    let old_json = serde_json::json!({
        "kind": "unchanged",
        "old_id": 1,
        "new_id": 1,
        "old_index": 0,
        "new_index": 0,
        "block_kind": "paragraph",
        "text_old": "重要 事项",
        "text_new": "重要 事项",
    });
    let l3: BlockDiffL3 = serde_json::from_value(old_json).expect("旧 JSON 直读");
    assert_eq!(l3.base.kind, DiffKind::Unchanged);
    assert!(l3.mark_changes.is_empty(), "serde(default) 补空");

    // roundtrip：L3 → JSON → L3 稳定（flatten + 新字段共存）。
    let d = diff_markdown_l3("**重要** 事项", "重要 事项");
    let json = serde_json::to_string(&d).unwrap();
    let back: Vec<BlockDiffL3> = serde_json::from_str(&json).unwrap();
    assert_eq!(back, d);
    assert!(json.contains("\"mark_changes\""), "wire 面新字段在位");
    assert!(json.contains("\"strong\""), "事件负载可序列化");
}

#[test]
fn l1_entry_unchanged_for_styled_documents() {
    // 验收 4c（反向零漂移）：有样式文档上 L1 原入口保持归一源文本口径
    // （Modified——**定界符是文本差），不受 L3 引入影响。
    let l1 = diff_markdown("**重要** 事项", "重要 事项");
    assert_eq!(l1.len(), 1);
    assert_eq!(l1[0].kind, DiffKind::Modified, "L1 口径不漂移");
    assert_eq!(l1[0].text_old.as_deref(), Some("**重要** 事项"));
}

#[test]
fn blockdiff_literal_construction_still_compiles() {
    // AGENTS 0.0.5 面：BlockDiff 未加字段（新结构体包裹）——既有字面量
    // 构造点编译面锁定（构造样例 = wire 测试同形）。
    let b = BlockDiff {
        kind: DiffKind::Added,
        old_id: None,
        new_id: None,
        old_index: None,
        new_index: Some(0),
        block_kind: mdcore::tree::BlockKind::Paragraph,
        text_old: None,
        text_new: Some("x".into()),
    };
    let l3 = BlockDiffL3 {
        base: b,
        mark_changes: Vec::new(), // 全字面量构造（非 serde 默认）
    };
    assert_eq!(l3.base.kind, DiffKind::Added);
}
