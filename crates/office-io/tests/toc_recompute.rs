//! #617 —— TOC 自动重算验收测试（`DocxEditor::recompute_toc` +
//! `DocxWriter::toc_entries_recomputed` 往返闭环）。
//!
//! 覆盖 issue 验收四场景（增/删/改/层级移动）+ 稳定名断言 +
//! PAGEREF 缓存语义（保留 / None → "—" 显式未知——不产假页码）+
//! V8 域拒 + 计划防篡改 + 层级过滤 + 空退化。
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
