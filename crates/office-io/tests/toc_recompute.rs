//! #617/#627 —— TOC 自动重算验收测试（`DocxEditor::recompute_toc` +
//! `DocxWriter::toc_entries_recomputed` 往返闭环）。
//!
//! 覆盖 issue 验收四场景（增/删/改/层级移动）+ 稳定名断言 +
//! PAGEREF 缓存语义（保留 / None → "—" 显式未知——不产假页码）+
//! V8 域拒 + 计划防篡改 + 层级过滤 + 空退化。
//!
//! #627 审核返工三缺口（用户状态链）：
//! - **当前编辑消费**：同一 DocxEditor open→改标题→重算即时反映
//!   （不重开文档）；无隐式保存；存盘前后一致
//!   → `recompute_consumes_pending_edits_same_editor`
//! - **书签保真**：存续标题复用旧 `toc_N`/`toc_h…` 锚（层级变化/
//!   改文不换名）；新项内容寻址且避让全部已存在书签（含用户书签）
//!   → `surviving_headings_reuse_bookmark_names` /
//!   `level_change_preserves_bookmark_name` /
//!   `renamed_heading_keeps_bookmark_via_pending` /
//!   `fresh_names_avoid_existing_bookmarks`
//! - **outer TOC 实际组范围**：Word 真机形态（纯文本/超链接结果段
//!   无段内域标记）完整提取，域组后正文不入旧目录
//!   → `word_form_toc_full_range_extraction`
//! - **V14**：计划 entry 文本非法 XML 字符拒（已转义字面不拒）
//!   → `plan_text_illegal_chars_rejected`
//!
//! 既有 t8b / #196 域测试（`docx.rs` 单元面）不经本文件改动——
//! 零回归由 `cargo test -p office-io` 全量门守住。

use mdc_office_io::docx::{DocxWriter, TocEntry};
use mdc_office_io::docx_edit::DocxEditor;
use mdc_office_io::docx_toc::TocRecompute;

/// 三标题文档（基线——A1/B2/C2 + 静态 TOC 三项）。
fn base_doc(entries: &[TocEntry]) -> Vec<u8> {
    let mut w = DocxWriter::new();
    w.heading(1, "第一章 总则").unwrap();
    w.heading(2, "第二章 报价构成").unwrap();
    w.heading(2, "第三章 交付与验收").unwrap();
    w.toc_entries(entries, None);
    w.save_to_vec().unwrap()
}

fn recompute(bytes: &[u8]) -> TocRecompute {
    let ed = DocxEditor::open(bytes).unwrap();
    ed.recompute_toc(None).unwrap()
}

#[test]
fn round_trip_matches_heading_tree_with_cache() {
    let bytes = base_doc(&[
        TocEntry::new(1, "第一章 总则"),
        TocEntry::new(2, "第二章 报价构成"),
        TocEntry::new(2, "第三章 交付与验收"),
    ]);
    let plan = recompute(&bytes);
    // 重算后目录项与文档当前标题树一一对应（文本/层级）。
    assert_eq!(plan.entries.len(), 3);
    assert_eq!(plan.entries[0].text, "第一章 总则");
    assert_eq!(plan.entries[0].level, 1);
    assert_eq!(plan.entries[1].text, "第二章 报价构成");
    assert_eq!(plan.entries[1].level, 2);
    assert_eq!(plan.entries[2].text, "第三章 交付与验收");
    assert_eq!(plan.entries[2].level, 2);
    // 旧 TOC 在场：静态面缓存 "1" 逐项保留。
    assert!(plan.old_toc_present);
    assert_eq!(
        plan.cached_pages,
        vec![Some("1".into()), Some("1".into()), Some("1".into())]
    );
    // 无增删。
    assert!(plan.diff.added.is_empty());
    assert!(plan.diff.removed.is_empty());
}

#[test]
fn scenario_add_new_heading() {
    // 标题树新增 D3（旧 TOC 只有 3 项——过期形态）。
    let mut w = DocxWriter::new();
    w.heading(1, "第一章 总则").unwrap();
    w.heading(2, "第二章 报价构成").unwrap();
    w.heading(2, "第三章 交付与验收").unwrap();
    w.heading(3, "第四章 附则与保修").unwrap();
    w.toc_entries(
        &[
            TocEntry::new(1, "第一章 总则"),
            TocEntry::new(2, "第二章 报价构成"),
            TocEntry::new(2, "第三章 交付与验收"),
        ],
        None,
    );
    let bytes = w.save_to_vec().unwrap();
    let plan = recompute(&bytes);
    assert_eq!(plan.diff.added, vec!["第四章 附则与保修".to_string()]);
    assert!(plan.diff.removed.is_empty());
    // 新标题无旧缓存 → None（待重算标记）。
    let d = plan
        .entries
        .iter()
        .position(|e| e.text == "第四章 附则与保修")
        .unwrap();
    assert_eq!(plan.cached_pages[d], None);
    // 存量三项缓存保留。
    assert_eq!(plan.cached_pages[0], Some("1".to_string()));
}

#[test]
fn scenario_remove_heading() {
    // 标题树只剩 A/C（旧 TOC 三项——B 被删的过期形态）。
    let mut w = DocxWriter::new();
    w.heading(1, "第一章 总则").unwrap();
    w.heading(2, "第三章 交付与验收").unwrap();
    w.toc_entries(
        &[
            TocEntry::new(1, "第一章 总则"),
            TocEntry::new(2, "第二章 报价构成"),
            TocEntry::new(2, "第三章 交付与验收"),
        ],
        None,
    );
    let bytes = w.save_to_vec().unwrap();
    let plan = recompute(&bytes);
    assert_eq!(plan.diff.removed, vec!["第二章 报价构成".to_string()]);
    assert!(plan.diff.added.is_empty());
    assert_eq!(plan.entries.len(), 2);
}

#[test]
fn scenario_text_change() {
    // B 文本变更（旧 TOC 记旧文——文本身份语义：removed(旧)+added(新)）。
    let mut w = DocxWriter::new();
    w.heading(1, "第一章 总则").unwrap();
    w.heading(2, "第二章 报价构成（新）").unwrap();
    w.heading(2, "第三章 交付与验收").unwrap();
    w.toc_entries(
        &[
            TocEntry::new(1, "第一章 总则"),
            TocEntry::new(2, "第二章 报价构成（旧）"),
            TocEntry::new(2, "第三章 交付与验收"),
        ],
        None,
    );
    let bytes = w.save_to_vec().unwrap();
    let plan = recompute(&bytes);
    assert_eq!(plan.diff.removed, vec!["第二章 报价构成（旧）".to_string()]);
    assert_eq!(plan.diff.added, vec!["第二章 报价构成（新）".to_string()]);
    // 新文本无旧缓存 → None；旧文本缓存随 removed 丢弃。
    let new_idx = plan
        .entries
        .iter()
        .position(|e| e.text == "第二章 报价构成（新）")
        .unwrap();
    assert_eq!(plan.cached_pages[new_idx], None);
}

#[test]
fn scenario_level_move() {
    // B 层级 2 → 3：重算后 entries 反映**当前**层级（验收主判据）；
    // diff.level_changed 恒 0 是已登记诚实降级（旧目录项不承载层级）。
    let mut w = DocxWriter::new();
    w.heading(1, "第一章 总则").unwrap();
    w.heading(3, "第二章 报价构成").unwrap();
    w.heading(2, "第三章 交付与验收").unwrap();
    w.toc_entries(
        &[
            TocEntry::new(1, "第一章 总则"),
            TocEntry::new(2, "第二章 报价构成"),
            TocEntry::new(2, "第三章 交付与验收"),
        ],
        None,
    );
    let bytes = w.save_to_vec().unwrap();
    let plan = recompute(&bytes);
    let b = plan
        .entries
        .iter()
        .position(|e| e.text == "第二章 报价构成")
        .unwrap();
    assert_eq!(plan.entries[b].level, 3, "重算后层级反映当前标题树");
    assert_eq!(plan.diff.level_changed, 0, "已登记降级：旧目录项无层级承载");
    // 文本未变 → 缓存保留（层级移动不清缓存）。
    assert_eq!(plan.cached_pages[b], Some("1".to_string()));
}

#[test]
fn no_toc_yields_all_pending() {
    // 无 TOC 域：old_toc_present = false，全量待重算（无假缓存）。
    let mut w = DocxWriter::new();
    w.heading(1, "第一章 总则").unwrap();
    w.heading(2, "第二章 报价构成").unwrap();
    let bytes = w.save_to_vec().unwrap();
    let plan = recompute(&bytes);
    assert!(!plan.old_toc_present);
    assert_eq!(plan.entries.len(), 2);
    assert!(plan.cached_pages.iter().all(|c| c.is_none()));
    assert!(plan.diff.added.is_empty());
    assert!(plan.diff.removed.is_empty());
}

#[test]
fn stable_names_across_recomputes() {
    // 内容寻址稳定名：同（层级,文本）标题跨文档/跨重算同名——
    // 与 TOC 状态无关（有/无旧 TOC 两文档同名）；两次重算幂等。
    let with_toc = base_doc(&[
        TocEntry::new(1, "第一章 总则"),
        TocEntry::new(2, "第二章 报价构成"),
        TocEntry::new(2, "第三章 交付与验收"),
    ]);
    let mut w = DocxWriter::new();
    w.heading(1, "第一章 总则").unwrap();
    w.heading(2, "第二章 报价构成").unwrap();
    w.heading(2, "第三章 交付与验收").unwrap();
    let without_toc = w.save_to_vec().unwrap();

    let p1 = recompute(&with_toc);
    let p2 = recompute(&without_toc);
    let p1_again = recompute(&with_toc);
    assert_eq!(p1.names, p2.names, "同标题树 → 同名（与 TOC 状态无关）");
    assert_eq!(p1.names, p1_again.names, "重算幂等");

    // 名域断言（V1-V6 构造面）：toc_h + 7 hex（可选 _N 去重后缀）。
    for n in &p1.names {
        assert!(n.starts_with("toc_h"), "内容寻址名形态：{n}");
        assert!(n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        assert!(n.chars().count() <= 40);
        assert!(n.chars().next().unwrap().is_ascii_alphabetic());
        assert!(!n.starts_with("_Toc"));
    }
    assert_eq!(p1.names.len(), 3);
    // 名字唯一。
    let mut uniq = p1.names.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), 3);
}

#[test]
fn duplicate_heading_texts_get_dedup_suffix() {
    // 同文**同层级**重复标题：先现者基名，后现者 _2/_3 后缀——域内
    // 唯一（同文异层级是不同内容寻址键——各自基名，不占去重位）。
    let mut w = DocxWriter::new();
    w.heading(1, "结论").unwrap();
    w.heading(1, "结论").unwrap();
    w.heading(1, "结论").unwrap();
    let bytes = w.save_to_vec().unwrap();
    let plan = recompute(&bytes);
    assert_eq!(plan.entries.len(), 3);
    let base = &plan.names[0];
    assert_eq!(plan.names[1], format!("{base}_2"));
    assert_eq!(plan.names[2], format!("{base}_3"));
}

#[test]
fn heading_range_filters_and_rejects() {
    let bytes = base_doc(&[
        TocEntry::new(1, "第一章 总则"),
        TocEntry::new(2, "第二章 报价构成"),
        TocEntry::new(2, "第三章 交付与验收"),
    ]);
    let ed = DocxEditor::open(&bytes).unwrap();
    // 层级过滤：1-1 只留一级标题。
    let plan = ed.recompute_toc(Some((1, 1))).unwrap();
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.entries[0].text, "第一章 总则");
    // V8 域拒：lo=0 / lo>hi —— fail-closed typed 拒。
    let e1 = ed.recompute_toc(Some((0, 3))).unwrap_err();
    assert!(e1.to_string().contains("V8"), "越域拒：{e1}");
    let e2 = ed.recompute_toc(Some((3, 1))).unwrap_err();
    assert!(e2.to_string().contains("V8"), "lo>hi 拒：{e2}");
}

#[test]
fn writer_emission_round_trip_with_stable_names() {
    // 完整闭环：读旧文档 → 重算 → 新写器发射（heading_bookmarked
    // 同名注册 + toc_entries_recomputed）→ 读回验证结构与稳定名。
    let old_bytes = base_doc(&[
        TocEntry::new(1, "第一章 总则"),
        TocEntry::new(2, "第二章 报价构成"),
        TocEntry::new(2, "第三章 交付与验收"),
    ]);
    let plan = recompute(&old_bytes);

    // 接线契约（与静态面同序）：先发射（引用侧）→ 再注册（定义侧）。
    let mut w = DocxWriter::new();
    let emitted = w.toc_entries_recomputed(&plan, None);
    assert_eq!(emitted, plan.names, "发射名 = 计划名（位置配对）");
    for (i, e) in plan.entries.iter().enumerate() {
        w.heading_bookmarked(e.level, &e.text, &plan.names[i])
            .unwrap();
    }
    let new_bytes = w.save_to_vec().unwrap();

    // 读回：TOC 域在场 + 目录项文本/缓存保留（"1"——旧缓存）。
    let ed = DocxEditor::open(&new_bytes).unwrap();
    assert!(ed
        .fields()
        .iter()
        .any(|f| f.kind == mdc_office_io::docx_edit::FieldKind::Toc));
    let plan2 = ed.recompute_toc(None).unwrap();
    assert_eq!(plan2.entries, plan.entries);
    assert_eq!(plan2.cached_pages, plan.cached_pages, "旧缓存经发射保留");
    // 稳定名端到端：新文档重算名 == 原计划名（同标题树）。
    assert_eq!(plan2.names, plan.names, "内容寻址名跨往返稳定");
    // 跳转闭环：定义侧（heading 书签）+ 引用侧（hyperlink/PAGEREF）
    // 同名——由 heading_bookmarked(…names[i]) 与发射名一致保证。
    assert_eq!(plan2.names.len(), 3);
}

#[test]
fn writer_emission_pending_page_is_explicit_unknown() {
    // 新标题（无旧缓存）发射 "—"——显式未知，不产假页码 "1"。
    let mut w = DocxWriter::new();
    w.heading(1, "第一章 总则").unwrap();
    w.heading(2, "第二章 新增").unwrap();
    let bytes = w.save_to_vec().unwrap();
    let plan = recompute(&bytes);
    assert!(plan.cached_pages.iter().all(|c| c.is_none()));

    let mut w2 = DocxWriter::new();
    w2.toc_entries_recomputed(&plan, None);
    for (i, e) in plan.entries.iter().enumerate() {
        w2.heading_bookmarked(e.level, &e.text, &plan.names[i])
            .unwrap();
    }
    let out = w2.save_to_vec().unwrap();

    // 读回：目录项缓存 = "—"（非 "1"）。
    let ed = DocxEditor::open(&out).unwrap();
    let plan2 = ed.recompute_toc(None).unwrap();
    assert_eq!(
        plan2.cached_pages,
        vec![Some("—".to_string()), Some("—".to_string())],
        "待重算标记落显式未知"
    );
}

#[test]
fn writer_rejects_tampered_plan() {
    // 计划防篡改（公开输入兜底）：names 长度不一致 → save 漏斗零产物拒。
    let mut w = DocxWriter::new();
    let plan = TocRecompute {
        entries: vec![TocEntry::new(1, "A"), TocEntry::new(2, "B")],
        names: vec!["toc_h0000001".to_string()], // 长度不符
        cached_pages: vec![None, None],
        old_toc_present: false,
        diff: Default::default(),
    };
    w.toc_entries_recomputed(&plan, None);
    let err = w.save_to_vec().unwrap_err();
    assert!(err.to_string().contains("V13"), "计划长度拒：{err}");

    // 域外名（_Toc 保留前缀）→ V5 拒。
    let mut w2 = DocxWriter::new();
    let plan2 = TocRecompute {
        entries: vec![TocEntry::new(1, "A")],
        names: vec!["_TocEvil".to_string()],
        cached_pages: vec![None],
        old_toc_present: false,
        diff: Default::default(),
    };
    w2.toc_entries_recomputed(&plan2, None);
    let err2 = w2.save_to_vec().unwrap_err();
    assert!(err2.to_string().contains("V5"), "保留名拒：{err2}");
}

#[test]
fn empty_plan_degrades_to_placeholder() {
    // entries 空 = 旧形态退化：占位五连域段（返回空名，产物合法）。
    let mut w = DocxWriter::new();
    let plan = TocRecompute {
        entries: vec![],
        names: vec![],
        cached_pages: vec![],
        old_toc_present: false,
        diff: Default::default(),
    };
    let names = w.toc_entries_recomputed(&plan, None);
    assert!(names.is_empty());
    let bytes = w.save_to_vec().unwrap();
    let ed = DocxEditor::open(&bytes).unwrap();
    assert!(ed
        .fields()
        .iter()
        .any(|f| f.kind == mdc_office_io::docx_edit::FieldKind::Toc));
}

// --------------------------------------------------- #627 审核返工（缺口 1）

/// 带书签的基线文档（存续标题锚定面——`heading_bookmarked` 注册
/// `toc_N`，与静态 TOC 引用侧配对）。
fn bookmarked_doc(levels: &[u8], texts: &[&str]) -> Vec<u8> {
    let entries: Vec<TocEntry> = levels
        .iter()
        .zip(texts)
        .map(|(l, t)| TocEntry::new(*l, *t))
        .collect();
    let mut w = DocxWriter::new();
    let names = w.toc_entries(&entries, None);
    for ((l, t), n) in levels.iter().zip(texts).zip(&names) {
        w.heading_bookmarked(*l, t, n).unwrap();
    }
    w.save_to_vec().unwrap()
}

/// 按文本定位段落号（编辑面公开坐标——`paragraph_text` 逐段比对）。
fn para_of(ed: &DocxEditor, text: &str) -> usize {
    (0..ed.paragraphs().len())
        .find(|&i| ed.paragraph_text(i) == text)
        .unwrap_or_else(|| panic!("段落不存在：{text:?}"))
}

/// **#627 缺口 1（审核反例原文）**：同一 `DocxEditor` open →
/// `set_paragraph_text` 把标题 A 改 A-new → `recompute_toc`——
/// 修改在 pending 未保存，重算必须消费**当前编辑快照**（读到
/// A-new，不读 open 时旧字节）。
#[test]
fn recompute_consumes_pending_edits_same_editor() {
    let bytes = bookmarked_doc(
        &[1, 2, 2],
        &["第一章 总则", "第二章 报价构成", "第三章 交付与验收"],
    );
    let mut ed = DocxEditor::open(&bytes).unwrap();
    // 同一编辑器实例：改第二个标题（不重开文档、不保存）。
    let p = para_of(&ed, "第二章 报价构成");
    ed.set_paragraph_text(p, "第二章 报价构成（修订）").unwrap();

    let plan = ed.recompute_toc(None).unwrap();
    // 标题树反映 pending 编辑：新文本在场，旧文本不在。
    assert_eq!(plan.entries.len(), 3);
    assert_eq!(plan.entries[1].text, "第二章 报价构成（修订）");
    assert!(
        !plan.entries.iter().any(|e| e.text == "第二章 报价构成"),
        "旧文本不得残留（open 时字节不得作为标题树源）"
    );
    // 差分：removed(旧) + added(新)——文本身份语义。
    assert_eq!(plan.diff.removed, vec!["第二章 报价构成".to_string()]);
    assert_eq!(plan.diff.added, vec!["第二章 报价构成（修订）".to_string()]);
    // 旧缓存按文本配对：改文后新文本无旧缓存（None）。
    assert_eq!(plan.cached_pages[1], None);

    // 无隐式保存：重算后 pending 仍在（段落文本仍是修订值），
    // 重复重算幂等（同一快照）。
    assert_eq!(ed.paragraph_text(p), "第二章 报价构成（修订）");
    let plan2 = ed.recompute_toc(None).unwrap();
    assert_eq!(plan2.entries, plan.entries);
    assert_eq!(plan2.names, plan.names);

    // 存盘前后重算结果一致：save 产物重开 → 重算 → 同 entries。
    let saved = ed.save_to_vec().unwrap();
    let ed_saved = DocxEditor::open(&saved).unwrap();
    let plan_saved = ed_saved.recompute_toc(None).unwrap();
    assert_eq!(plan_saved.entries, plan.entries, "存盘前后重算一致");
    assert_eq!(plan_saved.diff, plan.diff);
}

// --------------------------------------------------- #627 审核返工（缺口 2）

/// 存续标题复用旧锚：带 `toc_N` 书签的文档重算——names 即旧书签名
/// （不再内容寻址重命名，旧引用不悬空）。静态计数器从 `toc_0` 起
/// （`toc_bookmark_counter` 初值 0——先到先得递增）。
#[test]
fn surviving_headings_reuse_bookmark_names() {
    let bytes = bookmarked_doc(
        &[1, 2, 2],
        &["第一章 总则", "第二章 报价构成", "第三章 交付与验收"],
    );
    let plan = recompute(&bytes);
    // 静态发射名 toc_0/toc_1/toc_2 被复用（存续标题——零改名）。
    assert_eq!(
        plan.names,
        vec![
            "toc_0".to_string(),
            "toc_1".to_string(),
            "toc_2".to_string()
        ],
        "存续标题复用旧 toc_N 锚（不换名）"
    );
    // 名字唯一（V13 计划内重名面）。
    let mut uniq = plan.names.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), 3);
}

/// **审核反例原文**：同文本 heading 层级 2→3——旧实现按 (level,text)
/// 重 hash 会改名（toc_h8e11da8→toc_h6746685）。修复后：文本不变 →
/// 旧锚复用，层级变化不换名（entries 层级反映当前树）。
#[test]
fn level_change_preserves_bookmark_name() {
    // 同文本同书签，唯一差异：第二标题层级 2 → 3。
    let before = bookmarked_doc(
        &[1, 2, 2],
        &["第一章 总则", "第二章 报价构成", "第三章 交付与验收"],
    );
    let after = bookmarked_doc(
        &[1, 3, 2],
        &["第一章 总则", "第二章 报价构成", "第三章 交付与验收"],
    );
    let p1 = recompute(&before);
    let p2 = recompute(&after);
    // 名单不变（书签保真——层级不入配对键）。
    assert_eq!(p2.names, p1.names, "层级 2→3 不换书签名");
    assert_eq!(
        p2.names,
        vec![
            "toc_0".to_string(),
            "toc_1".to_string(),
            "toc_2".to_string()
        ]
    );
    // 层级如实反映当前标题树。
    assert_eq!(p2.entries[1].level, 3);
    // 文本未变 → 旧缓存保留。
    assert_eq!(p2.cached_pages[1], Some("1".to_string()));
}

/// 改文标题保锚（pending 感知配对）：书签节点在段落树手术中原样
/// 保留，重算按**当前文本**配对——改文后的标题仍配自己的旧锚。
#[test]
fn renamed_heading_keeps_bookmark_via_pending() {
    let bytes = bookmarked_doc(&[1, 2], &["第一章 总则", "第二章 报价构成"]);
    let mut ed = DocxEditor::open(&bytes).unwrap();
    let p = para_of(&ed, "第二章 报价构成");
    ed.set_paragraph_text(p, "第二章 价格与费率").unwrap();
    let plan = ed.recompute_toc(None).unwrap();
    // 改文标题复用旧锚 toc_1（段落身份语义——pending 感知文本配对）。
    assert_eq!(plan.entries[1].text, "第二章 价格与费率");
    assert_eq!(
        plan.names[1], "toc_1",
        "改文标题不换锚（书签在段内原样保留）"
    );
    assert_eq!(plan.names[0], "toc_0");
}

/// 新项内容寻址 + 避让：同文重复标题增删场景 + 用户书签不消费不碰撞。
#[test]
fn fresh_names_avoid_existing_bookmarks() {
    // ① 无书签基线：两个同文标题 → 内容寻址 [N, N_2]。
    let mut w = DocxWriter::new();
    w.heading(1, "结论").unwrap();
    w.heading(1, "结论").unwrap();
    let base = w.save_to_vec().unwrap();
    let base_plan = recompute(&base);
    let n1 = base_plan.names[0].clone();
    let n2 = base_plan.names[1].clone();
    assert_eq!(n2, format!("{n1}_2"), "基线去重后缀形态");

    // ② 第一个「结论」带书签 N1、第二个无书签 + 用户书签 MyMark
    //    在另一标题：新项（第二个结论）避让 N1 → N1_2（= 基线 N2，
    //    复用不位移去重编号）；MyMark 不被消费（非引擎域）。
    let entries = vec![
        TocEntry::new(1, "结论"),
        TocEntry::new(1, "结论"),
        TocEntry::new(2, "附录"),
    ];
    let mut w2 = DocxWriter::new();
    let names = w2.toc_entries(&entries, None);
    w2.heading_bookmarked(1, "结论", &n1).unwrap(); // 旧锚（引擎重算形）
    w2.heading_bookmarked(1, "结论", &names[1]).unwrap(); // 静态形锚
    w2.heading_bookmarked(2, "附录", "MyMark").unwrap(); // 用户书签（非引擎域）
    let doc = w2.save_to_vec().unwrap();
    let plan = recompute(&doc);

    assert_eq!(plan.entries.len(), 3);
    // 第一个结论：复用重算形旧锚 N1（文档序先现先配）。
    assert_eq!(plan.names[0], n1, "先现者复用旧锚");
    // 第二个结论：静态形锚 toc_1 复用。
    assert_eq!(plan.names[1], "toc_1", "次现者复用静态形锚");
    // 附录：MyMark 非引擎域不被消费——内容寻址新名，且避让
    // 全部已存在名（N1/toc_2/MyMark）。
    assert_ne!(plan.names[2], "MyMark", "用户书签不消费");
    assert!(
        plan.names[2].starts_with("toc_h"),
        "新项内容寻址形态：{}",
        plan.names[2]
    );
    let mut uniq = plan.names.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), 3, "计划内名字唯一");

    // ③ 删除面：只剩 [结论, 附录]——先现结论配先现锚（确定性）。
    let mut w3 = DocxWriter::new();
    let e3 = vec![TocEntry::new(1, "结论"), TocEntry::new(2, "附录")];
    let names3 = w3.toc_entries(&e3, None);
    w3.heading_bookmarked(1, "结论", &n1).unwrap();
    w3.heading_bookmarked(2, "附录", &names3[1]).unwrap();
    let doc3 = w3.save_to_vec().unwrap();
    let plan3 = recompute(&doc3);
    assert_eq!(plan3.names[0], n1, "删除后存续者仍配旧锚");
    assert_eq!(plan3.names[1], "toc_1");
}

// --------------------------------------------------- #627 审核返工（缺口 3）

/// Word 真机形态 TOC：结果段为**纯文本/超链接段（无段内域标记）**，
/// begin/separate 在首段、end 在末段——旧实现按段内 field_code 推进
/// 会在首个结果段提前截断。修复后按域 begin..end 实际组范围完整提取；
/// 域组后的正文（含 `\t` 形态段）不入旧目录。
#[test]
fn word_form_toc_full_range_extraction() {
    use std::io::Write as _;
    let document = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
        "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">",
        "<w:body>",
        // 域头段：begin + instr + separate（无目录项文本）。
        "<w:p><w:r><w:fldChar w:fldCharType=\"begin\"/></w:r>",
        "<w:r><w:instrText xml:space=\"preserve\"> TOC \\o \"1-3\" \\h \\z \\u </w:instrText></w:r>",
        "<w:r><w:fldChar w:fldCharType=\"separate\"/></w:r></w:p>",
        // 目录项段 1：纯文本（无域标记）——旧实现在此截断。
        "<w:p><w:pPr><w:tabs><w:tab w:val=\"right\" w:leader=\"dot\" w:pos=\"8290\"/></w:tabs></w:pPr>",
        "<w:r><w:t>第一章 总则</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>3</w:t></w:r></w:p>",
        // 目录项段 2：超链接包裹（Word 常见形态）。
        "<w:p><w:hyperlink w:anchor=\"_Toc123\" w:history=\"1\">",
        "<w:r><w:t>第二章 报价构成</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>5</w:t></w:r></w:hyperlink></w:p>",
        // 组内空段（separate 占位形态）——跳过不终止。
        "<w:p/>",
        // 目录项段 3：纯文本。
        "<w:p><w:r><w:t>第三章 交付与验收</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>7</w:t></w:r></w:p>",
        // 域尾段：end（末项与 end 分段——Word 亦常见同段，此处分段更严）。
        "<w:p><w:r><w:fldChar w:fldCharType=\"end\"/></w:r></w:p>",
        // 域组后的正文：含 \t 的段落不得混入旧目录。
        "<w:p><w:r><w:t>正文说明\t99</w:t></w:r></w:p>",
        // 标题（读面标题树——pStyle 紧凑形）。
        "<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t>第一章 总则</w:t></w:r></w:p>",
        "<w:p><w:pPr><w:pStyle w:val=\"Heading2\"/></w:pPr><w:r><w:t>第二章 报价构成</w:t></w:r></w:p>",
        "<w:p><w:pPr><w:pStyle w:val=\"Heading2\"/></w:pPr><w:r><w:t>第三章 交付与验收</w:t></w:r></w:p>",
        "<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/></w:sectPr>",
        "</w:body></w:document>"
    );
    let content_types = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
        "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
        "<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>",
        "<Default Extension=\"xml\" ContentType=\"application/xml\"/>",
        "<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>",
        "</Types>"
    );
    let root_rels = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
        "<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>",
        "</Relationships>"
    );
    // docx-rs 读模型要求 document.xml.rels 在场（探针实测：3 部件集
    // 报 "Failed to read from zip."，补空关系部件即过）。
    let doc_rels = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>"
    );
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in [
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", root_rels),
        ("word/_rels/document.xml.rels", doc_rels),
        ("word/document.xml", document),
    ] {
        zw.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zw.write_all(data.as_bytes()).unwrap();
    }
    let bytes = zw.finish().unwrap().into_inner();

    let ed = DocxEditor::open(&bytes).unwrap();
    // TOC 域在场且跨段（begin..end 组范围）。
    let toc = ed
        .fields()
        .into_iter()
        .find(|f| f.kind == mdc_office_io::docx_edit::FieldKind::Toc)
        .expect("TOC 域");
    assert!(!toc.same_para, "跨段 TOC 域");
    assert!(toc.end_para > toc.para, "end 段在 begin 段后");

    let plan = ed.recompute_toc(None).unwrap();
    // 旧目录完整提取：三项 + 各自缓存页码（旧实现在首个纯文本段截断）。
    assert!(plan.old_toc_present);
    assert_eq!(
        plan.cached_pages,
        vec![
            Some("3".to_string()),
            Some("5".to_string()),
            Some("7".to_string())
        ],
        "Word 形态三项完整提取（含超链接与组内空段跨越）"
    );
    // 标题树（读面）三项与旧目录配对——零增删。
    assert_eq!(plan.entries.len(), 3);
    assert!(plan.diff.added.is_empty());
    assert!(plan.diff.removed.is_empty());
    // 域组后的正文段（"正文说明\t99"）不得混入旧目录（无伪缓存 99）。
    assert!(
        !plan.cached_pages.iter().any(|c| c.as_deref() == Some("99")),
        "域组边界守恒：组后正文不入旧目录"
    );
}

// --------------------------------------------------- #627 审核返工（任务 5）

/// V14：公开手工计划的 entry 文本非法 XML 字符拒（save 漏斗零产物）；
/// 已转义字面（&amp; 等）是合法文本数据——不拒（审核钉定）。
#[test]
fn plan_text_illegal_chars_rejected() {
    // 非法控制字符 U+0001 → V14 拒。
    let mut w = DocxWriter::new();
    let plan = TocRecompute {
        entries: vec![TocEntry::new(1, "坏\u{0001}字符")],
        names: vec!["toc_h0000001".to_string()],
        cached_pages: vec![None],
        old_toc_present: false,
        diff: Default::default(),
    };
    w.toc_entries_recomputed(&plan, None);
    let err = w.save_to_vec().unwrap_err();
    assert!(err.to_string().contains("V14"), "控制字符拒：{err}");

    // 非字符 U+FFFE → V14 拒。
    let mut w2 = DocxWriter::new();
    let plan2 = TocRecompute {
        entries: vec![TocEntry::new(1, "非字符\u{FFFE}")],
        names: vec!["toc_h0000002".to_string()],
        cached_pages: vec![None],
        old_toc_present: false,
        diff: Default::default(),
    };
    w2.toc_entries_recomputed(&plan2, None);
    assert!(w2.save_to_vec().is_err(), "非字符拒");

    // 合法字面文本不拒（审核钉定：不得把「已转义」当非法拒）：
    // ① 原始 & < > 字符是合法文本（发射面转义处理——非非法字符），
    //    往返逐字保真；② 字面 "&amp;" 形态同样过 V14（只拒控制字符/
    //    非字符——探针钉定：docx_rs 读面双重反转义使 literal "&amp;"
    //    往返坍缩为 "&"，是 docx_rs 已知行为，与 V14 拒面无关）。
    let mut w3 = DocxWriter::new();
    let plan3 = TocRecompute {
        entries: vec![TocEntry::new(1, "A & B <协议>")],
        names: vec!["toc_h0000003".to_string()],
        cached_pages: vec![None],
        old_toc_present: false,
        diff: Default::default(),
    };
    w3.toc_entries_recomputed(&plan3, None);
    w3.heading_bookmarked(1, "A & B <协议>", "toc_h0000003")
        .unwrap();
    let bytes = w3.save_to_vec().expect("合法字符不拒（发射面转义）");
    let ed = DocxEditor::open(&bytes).unwrap();
    let back = ed.recompute_toc(None).unwrap();
    assert_eq!(back.entries.len(), 1);
    assert_eq!(back.entries[0].text, "A & B <协议>", "原始字符往返保真");

    let mut w4 = DocxWriter::new();
    let plan4 = TocRecompute {
        entries: vec![TocEntry::new(1, "A &amp; B")],
        names: vec!["toc_h0000004".to_string()],
        cached_pages: vec![None],
        old_toc_present: false,
        diff: Default::default(),
    };
    w4.toc_entries_recomputed(&plan4, None);
    assert!(
        w4.save_to_vec().is_ok(),
        "字面 &amp; 形态过 V14（非非法字符不拒）"
    );
}
