//! r29（#414）—— MD diff 引擎验收：块级对齐（L1）+ 行内 diff（L2）。
//!
//! 验收对照（issue #414 四条）：
//! 1. 两版 MD，块级 diff 正确识别新增/删除/修改 → `l1_add_remove_modify`
//! 2. 块内文字改动能定位到具体片段 → `l2_inline_locates_change`
//! 3. 大文件（10k 行）不卡（< 1s） → `perf_10k_lines_under_1s`
//! 4. 纯移动的块识别为 Moved（不是 删除+新增） → `l1_pure_move_is_moved`
//!
//! 补充锁（实现纪律面）：重复块按序配对（`l1_duplicate_blocks_pair_in_order`）、
//! 序列化面（wasm/wire 消费，`blockdiff_serializes_for_wire`）、确定性
//! （`deterministic_across_runs`）。

use mdcore::diff::{diff_inline, diff_markdown, BlockDiff, DiffKind, InlineDiff};
use mdcore::tree::BlockKind;

fn kinds(diffs: &[BlockDiff]) -> Vec<DiffKind> {
    diffs.iter().map(|d| d.kind).collect()
}

fn texts(diffs: &[BlockDiff]) -> Vec<String> {
    diffs
        .iter()
        .map(|d| {
            d.text_new
                .clone()
                .or_else(|| d.text_old.clone())
                .unwrap_or_default()
        })
        .collect()
}

// ------------------------------------------------------------- 验收 1

#[test]
fn l1_add_remove_modify() {
    let old_md = "\
# 报价方案

报价以澄清文件为准。

## 质保承诺

原质保期三年。
";
    let new_md = "\
# 报价方案

报价以最新澄清文件为准。

## 质保承诺

质保期延长至五年。

## 交付计划

签约后 60 日内交付。
";
    let d = diff_markdown(old_md, new_md);
    // 结构预期：标题不变 ×2、报价段 Modified、原质保段 Removed、
    // 新质保段 Added、交付计划标题 + 段 Added。
    assert_eq!(
        kinds(&d),
        vec![
            DiffKind::Unchanged, // # 报价方案
            DiffKind::Modified,  // 报价以…为准。
            DiffKind::Unchanged, // ## 质保承诺
            DiffKind::Removed,   // 原质保期三年。
            DiffKind::Added,     // 质保期延长至五年。
            DiffKind::Added,     // ## 交付计划
            DiffKind::Added,     // 签约后 60 日内交付。
        ],
        "五分类按新序：{:?}",
        texts(&d)
    );
    // Modified 条目双侧索引/文本/块型齐全（自包含——消费方免回源）。
    let m = &d[1];
    assert_eq!(m.block_kind, BlockKind::Paragraph);
    assert!(m.old_index.is_some() && m.new_index.is_some());
    assert_eq!(m.text_old.as_deref(), Some("报价以澄清文件为准。"));
    assert_eq!(m.text_new.as_deref(), Some("报价以最新澄清文件为准。"));
    assert!(m.old_id.is_some() && m.new_id.is_some());
    // Added 只有新侧面，Removed 只有旧侧面。
    assert!(d[4].old_index.is_none() && d[4].new_index.is_some());
    assert!(d[3].old_index.is_some() && d[3].new_index.is_none());
}

// ------------------------------------------------------------- 验收 2

#[test]
fn l2_inline_locates_change() {
    // 修改块（L1 Modified）的块内定位：报价有效期 30 → 90 天。
    let runs = diff_inline(
        "报价有效期 30 天，含安装调试。",
        "报价有效期 90 天，含安装调试。",
    );
    assert_eq!(
        runs,
        vec![
            InlineDiff {
                kind: DiffKind::Removed,
                text: "30".into()
            },
            InlineDiff {
                kind: DiffKind::Added,
                text: "90".into()
            },
        ],
        "最小 hunk（首尾等 run 削除）：{runs:?}"
    );
}

// ------------------------------------------------------------- 验收 3

#[test]
fn perf_10k_lines_under_1s() {
    // 10k 段，对 5% 修改 + 2% 抽删 + 1% 新增。**计时面 = 引擎**
    // （diff_documents——产品路径：两版已是打开的文档，无重解析；
    // diff_markdown 的 parse 属装载成本，不计入「diff 不卡」判据——
    // 与 issue 验收 3 的「大文件 diff < 1s」对齐）。
    use mdcore::diff::diff_documents;
    use std::time::Instant;

    let mut old_lines = Vec::with_capacity(10_000);
    let mut new_lines = Vec::with_capacity(10_000);
    for i in 0..10_000 {
        let para = format!("第 {i} 条：本条款约定项目交付边界与验收口径，内容编号 {i:05}。");
        let old = if i % 20 == 3 {
            format!("第 {i} 条：本条款约定原交付边界（待修订），内容编号 {i:05}。")
        } else {
            para.clone()
        };
        old_lines.push(old.clone());
        if i % 50 != 7 {
            // 98% 保留；2% 抽删（新侧缺失）。
            let new = if i % 20 == 3 {
                para // 修改面：待修订 → 定稿口径。
            } else {
                old
            };
            new_lines.push(new);
        }
        if i % 100 == 11 {
            new_lines.push(format!("第 {i} 条：新增补充条款（本轮澄清引入）。"));
        }
    }
    let old_md = old_lines.join("\n\n");
    let new_md = new_lines.join("\n\n");
    assert!(old_md.lines().count() >= 10_000, "基线规模");
    // 双侧文档先行装载（装载成本不计入验收计时）。
    let old_core = mdcore::Core::from_markdown(&old_md);
    let new_core = mdcore::Core::from_markdown(&new_md);

    let started = Instant::now();
    let d = diff_documents(&old_core, &new_core);
    let elapsed = started.elapsed();
    assert!(!d.is_empty(), "有修改面：{} 条 diff（不应为空）", d.len());
    assert!(
        elapsed.as_secs_f64() < 1.0,
        "10k 行 < 1s（验收 3）：实测 {elapsed:?}（{} 块 diff）",
        d.len()
    );
    // 结果正确性抽样：Modified / Added / Removed 三类都在场。
    let has = |k: DiffKind| d.iter().any(|x| x.kind == k);
    assert!(has(DiffKind::Modified) && has(DiffKind::Added) && has(DiffKind::Removed));
    println!("10k 行 diff：{elapsed:?}（{} 块）", d.len());
}

// ------------------------------------------------------------- 验收 4

#[test]
fn l1_pure_move_is_moved() {
    // 末段整体上移到首部（内容逐字不变）——Moved 而非 删除+新增。
    let old_md = "开篇。\n\n中段。\n\n可移动条款甲。\n\n可移动条款乙。";
    let new_md = "可移动条款甲。\n\n可移动条款乙。\n\n开篇。\n\n中段。";
    let d = diff_markdown(old_md, new_md);
    let moved = d.iter().filter(|x| x.kind == DiffKind::Moved).count();
    assert!(moved >= 2, "纯移动识别为 Moved（非删+增）：{:?}", kinds(&d));
    // 移动块双侧索引都在（位移可观察）。
    for m in d.iter().filter(|x| x.kind == DiffKind::Moved) {
        assert!(m.old_index.is_some() && m.new_index.is_some());
        assert_ne!(m.old_index, m.new_index);
        assert_eq!(m.text_old, m.text_new, "Moved = 内容不变");
    }
}

// --------------------------------------------------------- 实现纪律锁

#[test]
fn l1_duplicate_blocks_pair_in_order() {
    // 同文重复块（合同常见——重复条款）：按序配对，多出的标 Added。
    let old_md = "重复条款。\n\n中间段。\n\n重复条款。";
    let new_md = "重复条款。\n\n中间段。\n\n重复条款。\n\n重复条款。";
    let d = diff_markdown(old_md, new_md);
    assert_eq!(
        kinds(&d),
        vec![
            DiffKind::Unchanged,
            DiffKind::Unchanged,
            DiffKind::Unchanged,
            DiffKind::Added, // 第三份重复条款
        ],
        "{:?}",
        texts(&d)
    );
}

#[test]
fn l1_identical_documents_all_unchanged() {
    let md = "# 标题\n\n正文一。\n\n- 列表项\n\n```rust\nfn main() {}\n```\n";
    let d = diff_markdown(md, md);
    assert!(
        d.iter().all(|x| x.kind == DiffKind::Unchanged),
        "同文档全 Unchanged：{:?}",
        kinds(&d)
    );
    assert_eq!(
        d.len(),
        4,
        "4 块（标题/正文/列表项/代码块各 1 块——解析器实际计数）: {:?}",
        texts(&d)
    );
}

#[test]
fn blockdiff_serializes_for_wire() {
    // wasm/JSON 序列化面（消费方：mdcore-wasm 桥 / 宿主协议）。
    let d = diff_markdown("旧文。", "新文。");
    let json = serde_json::to_string(&d).expect("BlockDiff 应可序列化");
    assert!(
        json.contains("\"kind\""),
        "kind 字段 snake_case 在场：{json}"
    );
    assert!(json.contains("\"modified\""), "枚举值 snake_case：{json}");
    let back: Vec<BlockDiff> = serde_json::from_str(&json).expect("应可反序列化");
    assert_eq!(back, d, "往返一致");
}

#[test]
fn deterministic_across_runs() {
    // 同输入恒同输出（哈希迭代序不入结果——配对/排序全确定性）。
    let old_md = "甲段。\n\n乙段（内容不同）。\n\n丙段。\n\n丁段（另一内容）。";
    let new_md = "丙段。\n\n乙段（内容改过）。\n\n戊段（全新）。\n\n甲段。";
    let a = diff_markdown(old_md, new_md);
    let b = diff_markdown(old_md, new_md);
    assert_eq!(a, b);
}

#[test]
fn empty_and_single_side() {
    assert_eq!(diff_markdown("", ""), Vec::<BlockDiff>::new());
    let from_empty = diff_markdown("", "新增段。");
    assert_eq!(kinds(&from_empty), vec![DiffKind::Added]);
    let to_empty = diff_markdown("删除段。", "");
    assert_eq!(kinds(&to_empty), vec![DiffKind::Removed]);
}
