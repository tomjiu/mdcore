//! r31（#599）—— diff 同谱系 ID 匹配验收：重写块身份保持 + 精确 pass
//! 防覆盖。
//!
//! 验收对照（issue #599 四条）：
//! 1. 真同谱系（capture/restore 与 save→open meta 两条路径）：大幅
//!    重写块仍 Modified 且 old_id==new_id → `lineage_rewrite_*`
//! 2. 关键负例：同 ID 异文 + 重复文本，精确 pass 不覆盖配对、不复用
//!    new、不丢事件 → `id_pair_not_stolen_by_exact_text`
//! 3. 同 ID 改 BlockKind：显式拆分（Removed+Added 各携原 ID），不默默
//!    输出不一致类型 → `same_id_kind_change_splits`
//! 4. 默认入口零漂移：独立 parse 前插不产生假 Modified；
//!    移动/删除/前插分类与身份正确 → `default_entries_zero_drift`
//!    / `lineage_move_delete_insert_ids`
//! 5. 同谱系 10k 性能门 → `perf_10k_lineage_under_1s`
//!
//! 谱系纪律：`diff_documents_lineage` 是显式入口——调用方声明两侧
//! 同谱系（capture/restore、save→open meta v2 等 ID 同源路径）。两份
//! 独立 `from_markdown` 的 ID 空间各自位置序发号（#414/#415 在案），
//! 不得走谱系入口（错锚）。

use mdcore::anchor::TextTarget;
use mdcore::diff::{diff_documents, diff_documents_lineage, diff_markdown, BlockDiff, DiffKind};
use mdcore::tree::BlockKind;
use mdcore::Core;

fn kinds(diffs: &[BlockDiff]) -> Vec<DiffKind> {
    diffs.iter().map(|d| d.kind).collect()
}

/// 谱系副本路径 A：capture/restore（同树同 ID 重建）。
fn lineage_restore(old: &Core) -> Core {
    Core::restore(old.snapshot())
}

/// 谱系副本路径 B：save_document → open_document（meta v2 ID 持久化）。
fn lineage_reopen(old: &Core) -> Core {
    let (md, meta) = old.save_document();
    Core::open_document(&md, &meta).expect("save→open 谱系往返")
}

/// 全量替换某顶层块的文本（start..end 覆盖全串——不依赖长度细节）。
fn rewrite_block(core: &mut Core, top_index: usize, text: &str) {
    let len = core
        .tree
        .node(core.tree.node(core.tree.root()).children[top_index])
        .text
        .to_string()
        .chars()
        .count();
    core.replace_text(
        TextTarget::ByPath {
            path: vec![top_index],
            start: 0,
            end: len,
        },
        text.to_string(),
    )
    .expect("replace_text");
}

// ------------------------------------------------------------- 验收 1

/// 大幅重写（远低于相似阈值）→ 仍 Modified 且 old_id==new_id（身份
/// 不因文本大变丢失）。capture/restore 与 save→open 两路径都锁。
#[test]
fn lineage_rewrite_stays_modified_same_id() {
    let md = "\
# 报价方案

第一段正文，概要说明。

- 列表项甲
- 列表项乙

收尾段。
";
    let old = Core::from_markdown(md);
    // 先序 ID：[# 标题, 第一段, 项甲, 项乙, 收尾]——重写目标是第一段。
    let ids = old.tree.preorder_block_ids();
    assert_eq!(ids.len(), 5, "结构前提：5 块");

    for (name, make_new) in [
        ("capture/restore", lineage_restore as fn(&Core) -> Core),
        ("save→open meta", lineage_reopen as fn(&Core) -> Core),
    ] {
        let mut new = make_new(&old);
        rewrite_block(
            &mut new,
            1,
            "开标一览表（与原文完全无关的全新内容，验证身份不随文本丢失）。",
        );
        let d = diff_documents_lineage(&old, &new);
        let mods: Vec<&BlockDiff> = d.iter().filter(|x| x.kind == DiffKind::Modified).collect();
        assert_eq!(
            mods.len(),
            1,
            "{name}：恰一个 Modified（重写块），实际 {:?}",
            kinds(&d)
        );
        let m = mods[0];
        assert_eq!(m.old_id, Some(ids[1]), "{name}：Modified 携旧侧 ID");
        assert_eq!(
            m.new_id,
            Some(ids[1]),
            "{name}：Modified 携新侧 ID（同谱系身份保持）"
        );
        assert_eq!(m.old_id, m.new_id, "{name}：old_id == new_id");
        assert_eq!(m.block_kind, BlockKind::Paragraph, "{name}：双侧同型契约");
        // 其余四块 Unchanged；零 Added/Removed/Moved。
        let rest: Vec<DiffKind> = d
            .iter()
            .filter(|x| x.kind != DiffKind::Modified)
            .map(|x| x.kind)
            .collect();
        assert!(
            rest.iter().all(|k| *k == DiffKind::Unchanged),
            "{name}：重写不影响他块，实际 {rest:?}"
        );
        assert_eq!(d.len(), 5, "{name}：事件数 = 块数（无丢失）");
    }
}

// ------------------------------------------------------------- 验收 2

/// 关键负例：同 ID 异文（重写成恰等于另一块文本）+ 重复文本在场。
/// ID pass 先占 (X,X')；精确 pass 处理 Y 时不得偷走已占的 X'——
/// 不覆盖配对、不复用 new 块、不丢事件。
#[test]
fn id_pair_not_stolen_by_exact_text() {
    let md = "\
甲段共享文本。

乙段独特文本。
";
    let old = Core::from_markdown(md);
    let ids = old.tree.preorder_block_ids();
    assert_eq!(ids.len(), 2, "结构前提：2 块");
    let (id_x, id_y) = (ids[0], ids[1]);

    let mut new = lineage_restore(&old);
    // X（甲段）重写为恰等于 Y 的文本——新侧两块文本完全相同。
    rewrite_block(&mut new, 0, "乙段独特文本。");
    // 现在两个 new 块的签名 (Paragraph, "乙段独特文本。") 相同。
    let d = diff_documents_lineage(&old, &new);

    // 事件完整：2 块 → 2 条（不丢事件）。
    assert_eq!(
        d.len(),
        2,
        "事件数 = 块数（不丢事件），实际 {:?}",
        kinds(&d)
    );
    // X = Modified（同 ID 异文——身份保持，不是 Removed+Added）。
    let m = d
        .iter()
        .find(|x| x.kind == DiffKind::Modified)
        .expect("X 应 Modified");
    assert_eq!(m.old_id, Some(id_x), "Modified 携 X 旧 ID");
    assert_eq!(
        m.new_id,
        Some(id_x),
        "Modified 携 X 新 ID（ID 配对未被偷走）"
    );
    assert_eq!(
        (m.text_old.as_deref(), m.text_new.as_deref()),
        (Some("甲段共享文本。"), Some("乙段独特文本。")),
        "Modified 双侧文本"
    );
    // Y = Unchanged（精确配对落在 Y' 上——不复用 X'）。
    let u = d
        .iter()
        .find(|x| x.kind == DiffKind::Unchanged)
        .expect("Y 应 Unchanged");
    assert_eq!(u.old_id, Some(id_y), "Unchanged 携 Y 旧 ID");
    assert_eq!(
        u.new_id,
        Some(id_y),
        "Unchanged 携 Y 新 ID（new 块未被复用错位）"
    );
    // 零 Added / Removed / Moved。
    assert!(
        !d.iter().any(|x| {
            matches!(
                x.kind,
                DiffKind::Added | DiffKind::Removed | DiffKind::Moved
            )
        }),
        "重复文本不得引入伪 Added/Removed/Moved：{:?}",
        kinds(&d)
    );
}

// ------------------------------------------------------------- 验收 3

/// 同 ID 改 BlockKind（段落 → 标题）：显式拆分为 Removed（旧型）+
/// Added（新型），各携同一条目侧 ID；不输出双侧异型的 Modified。
#[test]
fn same_id_kind_change_splits() {
    let md = "普通段落文本。\n";
    let old = Core::from_markdown(md);
    let ids = old.tree.preorder_block_ids();
    assert_eq!(ids.len(), 1, "结构前提：单块");

    let mut new = lineage_restore(&old);
    new.set_block_type(0, vec![0], BlockKind::Heading)
        .expect("set_block_type");
    let d = diff_documents_lineage(&old, &new);

    // 无 Modified（不默默输出双侧异型条目）。
    assert!(
        !d.iter().any(|x| x.kind == DiffKind::Modified),
        "kind 变更不得配成 Modified：{:?}",
        kinds(&d)
    );
    // 拆分：Removed（Paragraph，旧 ID）+ Added（Heading，同一 ID）。
    assert_eq!(
        kinds(&d),
        vec![DiffKind::Removed, DiffKind::Added],
        "拆分序"
    );
    let r = &d[0];
    let a = &d[1];
    assert_eq!(r.block_kind, BlockKind::Paragraph, "Removed 取旧侧类型");
    assert_eq!(a.block_kind, BlockKind::Heading, "Added 取新侧类型");
    assert_eq!(r.old_id, Some(ids[0]), "Removed 携旧侧 ID");
    assert_eq!(
        a.new_id,
        Some(ids[0]),
        "Added 携新侧 ID（同 ID——身份可观察）"
    );
    assert_eq!(r.text_old.as_deref(), Some("普通段落文本。"));
    assert_eq!(a.text_new.as_deref(), Some("普通段落文本。"));
}

// ------------------------------------------------------------- 验收 4

/// 谱系内移动/删除/前插：分类与身份正确；同文 ID 对参与 Moved 判定。
#[test]
fn lineage_move_delete_insert_ids() {
    let md = "\
第一段。

第二段。

第三段。

第四段。
";
    let old = Core::from_markdown(md);
    let ids = old.tree.preorder_block_ids();
    assert_eq!(ids.len(), 4, "结构前提：4 段");

    let mut new = lineage_restore(&old);
    // 删除第一段（顶层 index 0）——gate 携当前修订号（每次编辑递增）。
    new.delete_block(new.revision(), vec![0]).expect("delete");
    // 把（原第三段，删除后 index 1）移到末尾（移除后列表 2 项——
    // 插入位 2 = 追加）。
    new.move_block(new.revision(), vec![1], vec![], 2)
        .expect("move");
    // 前插新块（顶层 index 0）。
    new.insert_block(
        new.revision(),
        vec![],
        0,
        BlockKind::Paragraph,
        Default::default(),
        "前插的全新段落。".into(),
    )
    .expect("insert");

    let d = diff_documents_lineage(&old, &new);
    // 事件面：1 Added（前插）+ 1 Removed（删除）+ 1 Moved + 2 Unchanged。
    // Moved 的最小移动解释：把第三段移到末尾 等价于 第四段前移一位
    // ——LIS 取最长递增于序列（[1,3,2] → LIS [1,3]），故 Moved 落在
    // 第四段（与移动操作语义等价的极小表示，非缺陷）。
    let ks = kinds(&d);
    assert_eq!(ks.len(), 5, "事件数完整（不丢事件）：{ks:?}");
    let added = d.iter().find(|x| x.kind == DiffKind::Added).expect("Added");
    let removed = d
        .iter()
        .find(|x| x.kind == DiffKind::Removed)
        .expect("Removed");
    let moved = d
        .iter()
        .find(|x| x.kind == DiffKind::Moved)
        .expect("Moved（同文 ID 对参与 LIS Moved 判定）");
    // 身份断言。
    assert_eq!(removed.old_id, Some(ids[0]), "Removed = 被删第一段");
    assert_eq!(
        moved.old_id,
        Some(ids[3]),
        "Moved = 位差块（第四段——最小移动解释）"
    );
    assert_eq!(moved.new_id, Some(ids[3]), "Moved 携同 ID（身份保持）");
    assert_eq!(moved.text_old, moved.text_new, "Moved 双侧同文");
    // 前插新块 ID ≠ 任何旧块 ID（新发号——不复用已删 ID）。
    let added_id = added.new_id.expect("Added 携新侧 ID");
    assert!(
        !ids.contains(&added_id),
        "前插块是全新 ID（不复用已删 ID：{added_id:?} ∉ {ids:?}）"
    );
    // 其余两段 Unchanged（第二段原位 + 第三段末位在 LIS 内——
    // [id2, id4, id3] 的新序中 id2/id3 递增）。
    let unchanged: Vec<_> = d.iter().filter(|x| x.kind == DiffKind::Unchanged).collect();
    assert_eq!(unchanged.len(), 2, "第二/三段 Unchanged");
    assert_eq!(unchanged[0].old_id, Some(ids[1]), "第二段 Unchanged");
    assert_eq!(
        unchanged[1].old_id,
        Some(ids[2]),
        "第三段 Unchanged（新序末位）"
    );
}

/// 默认入口零漂移：两侧独立 Markdown parse + 前插 → 前插块 Added、
/// 其余 Unchanged，零假 Modified（不自动猜谱系）。
#[test]
fn default_entries_zero_drift_on_prefix_insert() {
    let old_md = "\
# 总体方案

第一段正文。

- 列表项甲
- 列表项乙
";
    let new_md = "\
新增的引言段落。

# 总体方案

第一段正文。

- 列表项甲
- 列表项乙
";
    // diff_markdown（文本直入糖衣）。
    let d = diff_markdown(old_md, new_md);
    assert_eq!(
        kinds(&d),
        vec![
            DiffKind::Added,     // 新增的引言段落。（前插）
            DiffKind::Unchanged, // # 总体方案
            DiffKind::Unchanged, // 第一段正文。
            DiffKind::Unchanged, // - 列表项甲
            DiffKind::Unchanged, // - 列表项乙
        ],
        "前插 = Added + 全 Unchanged，零 Modified"
    );
    // diff_documents（双侧独立 from_markdown + 默认入口）同口径。
    let old = Core::from_markdown(old_md);
    let new = Core::from_markdown(new_md);
    let d2 = diff_documents(&old, &new);
    assert_eq!(kinds(&d2), kinds(&d), "两默认入口同语义（零漂移）");
    assert!(
        !d2.iter().any(|x| x.kind == DiffKind::Modified),
        "独立 parse 的位置序 ID 不得产生假 Modified"
    );
}

// ------------------------------------------------------------- 验收 5

/// 同谱系 10k 块性能门：save→open 谱系 + 5% 重写 → < 1s。
#[test]
fn perf_10k_lineage_under_1s() {
    use std::time::Instant;

    let mut old_lines = Vec::with_capacity(10_000);
    for i in 0..10_000 {
        old_lines.push(format!(
            "第 {i} 条：本条款约定项目交付边界与验收口径，内容编号 {i:05}。"
        ));
    }
    let old = Core::from_markdown(&old_lines.join("\n\n"));
    assert_eq!(old.tree.preorder_block_ids().len(), 10_000, "基线规模");

    let mut new = lineage_restore(&old);
    // 5% 大幅重写（身份保持面——文本与原文无关）。
    for i in (0..10_000).step_by(20) {
        rewrite_block(
            &mut new,
            i,
            &format!("第 {i} 条：重写后的全新条款内容（谱系身份验证），编号 {i:05}。"),
        );
    }

    let started = Instant::now();
    let d = diff_documents_lineage(&old, &new);
    let elapsed = started.elapsed();
    assert_eq!(d.len(), 10_000, "事件数 = 块数（无丢失）");
    let mods = d.iter().filter(|x| x.kind == DiffKind::Modified).count();
    assert_eq!(mods, 500, "重写块全数 Modified");
    assert!(
        d.iter()
            .filter(|x| x.kind == DiffKind::Modified)
            .all(|x| x.old_id == x.new_id),
        "Modified 全部身份保持"
    );
    assert!(
        elapsed.as_secs_f64() < 1.0,
        "同谱系 10k 块 < 1s：实测 {elapsed:?}（{} 块 diff）",
        d.len()
    );
    println!("10k 谱系 diff：{elapsed:?}（{mods} Modified）");
}
