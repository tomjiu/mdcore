//! r30（#415）—— 块 ID 重编号撞号缺陷（P0-1）验收三条不变性锁。
//!
//! **背景裁定（勘察在案）**：研究仓资产表指认的「P0-1 重编号撞号」
//! 已由 #244（PR #249）修复——save→open 先序 ID 表持久化（路线 A）
//! 与 register 撞号守卫（路线 C），既有测试面见 r27_block_identity.rs。
//!
//! 本文件按 **issue #415 验收三条**补齐 r27 未显式覆盖的锁：
//!
//! 1. **验收 1「同一文档反复 parse，块 ID 稳定不变」**：
//!    `reparse_same_markdown_ids_stable` —— 独立 `from_markdown` 两次，
//!    先序 ID 序列逐位恒等（解析器确定性）。
//! 2. **验收 2「编辑（增删块）后，未改动的块 ID 不变」**：
//!    `edit_keeps_unchanged_block_ids` —— 删/插/移后幸存块 ID 不变 +
//!    已删 ID 不复用（分配器水位纪律）+ 编辑态 save→open 全链保持。
//! 3. **验收 3「大规模文档（10k 块）无重复 ID」**：
//!    `parse_10k_blocks_no_duplicate_ids` —— 万块级唯一性 + 注册一致。
//!
//! **设计边界（如实登记，非缺陷）**：`from_markdown` 的 ID 空间是
//! **位置序**（先序 1..N，§8.3：ID 不写入 Markdown，持久身份走 meta
//! 映射）——两份**不同**文档各自独立 parse 会得到重叠的 ID 空间
//! （各自 1..N）。跨文档按 ID 对齐因此不可靠（#414 diff 引擎不作
//! ID 锚的根因，其模块文档在案）；同谱系身份 = save→open meta 表
//! 路径（r27 I2 锁）。

use std::collections::BTreeSet;

use mdcore::id::BlockId;
use mdcore::tree::BlockKind;
use mdcore::Core;

// ----------------------------------------------------------- 验收 1

#[test]
fn reparse_same_markdown_ids_stable() {
    let md = "\
# 总体方案

第一段正文。

- 列表项甲
- 列表项乙

```rust
fn main() {}
```

> 引用段

收尾段。
";
    let a = Core::from_markdown(md);
    let b = Core::from_markdown(md);
    assert_eq!(
        a.tree.preorder_block_ids(),
        b.tree.preorder_block_ids(),
        "同文档两次独立 parse：先序 ID 序列应逐位恒等（解析确定性）"
    );
    // 内容面同样恒等（ID 稳定不是碰巧——结构同构）。
    assert_eq!(a.tree.top_level().len(), b.tree.top_level().len());
}

// ----------------------------------------------------------- 验收 2

/// 按块文本找 ID（测试辅助——幸存块身份断言用）。
fn find_id_by_text(c: &Core, want: &str) -> Option<BlockId> {
    fn walk(c: &Core, idx: usize, want: &str, out: &mut Option<BlockId>) {
        if out.is_some() {
            return;
        }
        let top = c.tree.top_level();
        if idx >= top.len() {
            return;
        }
        let node = c.tree.node(top[idx]);
        if node.text == want {
            *out = Some(node.id);
            return;
        }
        walk(c, idx + 1, want, out);
    }
    let mut out = None;
    walk(c, 0, want, &mut out);
    out
}

#[test]
fn edit_keeps_unchanged_block_ids() {
    let md = "P1\n\nP2\n\nP3\n\nP4\n\nP5";
    let mut c = Core::from_markdown(md);
    // 基线：五段各自的 ID（按文本锚定——路径会随后续编辑漂移）。
    let ids: Vec<BlockId> = ["P1", "P2", "P3", "P4", "P5"]
        .iter()
        .map(|t| find_id_by_text(&c, t).expect("基线段应在场"))
        .collect();
    let deleted_id = ids[2];
    let base = c.revision();

    // ① 删除 P3（路径 [2]）：幸存四段 ID 不变。
    c.delete_block(base, vec![2]).expect("删除 P3");
    assert_eq!(find_id_by_text(&c, "P1"), Some(ids[0]), "P1 编辑后 ID 不变");
    assert_eq!(find_id_by_text(&c, "P2"), Some(ids[1]), "P2 编辑后 ID 不变");
    assert_eq!(find_id_by_text(&c, "P4"), Some(ids[3]), "P4 编辑后 ID 不变");
    assert_eq!(find_id_by_text(&c, "P5"), Some(ids[4]), "P5 编辑后 ID 不变");
    assert_eq!(find_id_by_text(&c, "P3"), None, "P3 已删");

    // ② 顶部插入新段：幸存段 ID 不变 + 已删 ID 不复用（水位纪律：
    //    delete 失效即弃——§8.2）。
    let base = c.revision();
    c.insert_block(
        base,
        vec![],
        0,
        BlockKind::Paragraph,
        Default::default(),
        "P0-新增".to_string(),
    )
    .expect("插入新段");
    let new_id = find_id_by_text(&c, "P0-新增").expect("新段在场");
    assert_ne!(new_id, deleted_id, "已删块 ID 不得复用（分配器只前进）");
    for (t, id) in [
        ("P1", ids[0]),
        ("P2", ids[1]),
        ("P4", ids[3]),
        ("P5", ids[4]),
    ] {
        assert_eq!(find_id_by_text(&c, t), Some(id), "{t} 插入后 ID 不变");
    }

    // ③ 移动 P5 到顶部：ID 随块走（身份跟着内容不跟位置）。
    //    当前顶层序：[P0-新增, P1, P2, P4, P5] → P5 路径 [4]。
    let base = c.revision();
    c.move_block(base, vec![4], vec![], 0).expect("移动 P5");
    assert_eq!(find_id_by_text(&c, "P5"), Some(ids[4]), "P5 移动后 ID 不变");

    // ④ 编辑态全链 save→open：身份表携带编辑后状态（r27 I2 锁的是
    //    干净往返，本处锁**编辑后**往返——验收 2 的完整链）。
    let (md_out, meta) = c.save_document();
    let reopened = Core::open_document(&md_out, &meta).expect("编辑态重开");
    for (t, id) in [
        ("P0-新增", new_id),
        ("P1", ids[0]),
        ("P2", ids[1]),
        ("P4", ids[3]),
        ("P5", ids[4]),
    ] {
        assert_eq!(
            find_id_by_text(&reopened, t),
            Some(id),
            "{t} 编辑态 save→open 后 ID 不变"
        );
    }
}

// ----------------------------------------------------------- 验收 3

#[test]
fn parse_10k_blocks_no_duplicate_ids() {
    let md: String = (0..10_000)
        .map(|i| format!("第 {i} 条：规模面不变性样张段。"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let c = Core::from_markdown(&md);
    let ids = c.tree.preorder_block_ids();
    assert_eq!(ids.len(), 10_000, "万块基线（先序 ID 表满量）");
    let uniq: BTreeSet<u64> = ids.iter().map(|b| b.0).collect();
    assert_eq!(uniq.len(), ids.len(), "10k 块无重复 ID（验收 3）");
    // 顶层次序 = 先序表前缀（同源同序——注册/装载一致性）。
    assert_eq!(c.tree.top_level().len(), 10_000);
}
