//! #630 —— TOC 原位整组替换验收测试（`DocxEditor::replace_toc_group`）。
//!
//! 覆盖 issue #630 验收四条：
//! 1. 非目标部件字节恒等（媒体/样式/页眉脚——zip 条目逐一比对）
//! 2. 同编辑器改标题→替换→重开：TOC = 新标题树 + PAGEREF 锚可达
//!    （解析 XML 验证 bookmarkStart/End 配平）
//! 3. Word 真机形态 TOC（纯文本结果段）同样可替换
//! 4. 既有测试面零回归（由 `cargo test -p office-io` 全量门守住——
//!    t8b/#196/#617/#627 在各自文件）
//!
//! 拒面：无 TOC 域 / 计划长度不一致（V13）/ 组内域编辑冲突 /
//! 过期计划锚定位失败（fail-closed——编辑器状态零变化）。

use std::io::Read as _;
use std::io::Write as _;

use mdc_office_io::docx::{DocxWriter, TocEntry};
use mdc_office_io::docx_edit::DocxEditor;
use mdc_office_io::docx_toc::TocRecompute;

/// 1×1 PNG（69B——真实媒体件，非骨架；与 docx_roundtrip_matrix 同源）。
const PNG_1X1: [u8; 69] = [
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0xC9, 0xFE, 0x92, 0xEF, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// 富内容基线：TOC + 三标题（书签锚定）+ 正文两段 + 图片 + 表格 +
/// 页眉/页脚（非目标部件保真面的全部载荷族）。
fn rich_doc() -> Vec<u8> {
    let entries = vec![
        TocEntry::new(1, "第一章 总则"),
        TocEntry::new(2, "第二章 报价构成"),
        TocEntry::new(2, "第三章 交付与验收"),
    ];
    let mut w = DocxWriter::new();
    let names = w.toc_entries(&entries, None);
    w.heading_bookmarked(1, "第一章 总则", &names[0]).unwrap();
    w.paragraph_text("总则正文：本项目按下列条款执行。");
    w.image(&PNG_1X1, 1, 1).unwrap();
    w.heading_bookmarked(2, "第二章 报价构成", &names[1])
        .unwrap();
    w.table(&[
        vec!["费用项".into(), "金额（元）".into()],
        vec!["设计费".into(), "120000".into()],
    ]);
    w.heading_bookmarked(2, "第三章 交付与验收", &names[2])
        .unwrap();
    w.paragraph_text("验收正文：按分项验收口径执行。");
    w.header(&[mdc_office_io::docx::RunSpec::text("投标文件页眉")]);
    w.footer(&[mdc_office_io::docx::RunSpec::text("第 1 页 共 1 页")]);
    w.save_to_vec().unwrap()
}

/// zip 条目名 → 字节（保真比对面）。
fn zip_entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut ar = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = Vec::new();
    for i in 0..ar.len() {
        let mut f = ar.by_index(i).unwrap();
        let mut v = Vec::new();
        f.read_to_end(&mut v).unwrap();
        out.push((f.name().to_string(), v));
    }
    out
}

/// document.xml 原文（锚可达性/配平断言面）。
fn doc_xml(bytes: &[u8]) -> String {
    let mut ar = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    ar.by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut s)
        .unwrap();
    s
}

/// bookmarkStart/End 配平 + 名字集合（解析面——非字符串计数）。
fn bookmark_balance(xml: &str) -> (bool, Vec<String>) {
    let mut starts: Vec<(u64, String)> = Vec::new();
    let mut ends: Vec<u64> = Vec::new();
    let mut reader = quick_xml::Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if name == "bookmarkStart" {
                    let mut id = None;
                    let mut nm = None;
                    for a in e.attributes() {
                        let a = a.unwrap();
                        match a.key.local_name().as_ref() {
                            b"id" => {
                                id = std::str::from_utf8(&a.value)
                                    .ok()
                                    .and_then(|v| v.parse().ok())
                            }
                            b"name" => nm = std::str::from_utf8(&a.value).ok().map(str::to_string),
                            _ => {}
                        }
                    }
                    if let (Some(id), Some(nm)) = (id, nm) {
                        starts.push((id, nm));
                    }
                } else if name == "bookmarkEnd" {
                    for a in e.attributes() {
                        let a = a.unwrap();
                        if a.key.local_name().as_ref() == b"id" {
                            if let Some(v) = std::str::from_utf8(&a.value)
                                .ok()
                                .and_then(|v| v.parse().ok())
                            {
                                ends.push(v);
                            }
                        }
                    }
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return (false, Vec::new()),
        }
    }
    let mut start_ids: Vec<u64> = starts.iter().map(|(i, _)| *i).collect();
    start_ids.sort_unstable();
    ends.sort_unstable();
    let balanced = start_ids == ends && start_ids.len() == starts.len();
    (balanced, starts.into_iter().map(|(_, n)| n).collect())
}

fn para_of(ed: &DocxEditor, text: &str) -> usize {
    (0..ed.paragraphs().len())
        .find(|&i| ed.paragraph_text(i) == text)
        .unwrap_or_else(|| panic!("段落不存在：{text:?}"))
}

/// **验收 1+2（核心闭环）**：同编辑器改标题 → 重算 → 原位替换 →
/// 保存重开：TOC = 新标题树；非目标部件字节恒等；锚配平可达；
/// 未动段文本不变。
#[test]
fn inplace_replace_full_flow_with_package_fidelity() {
    let bytes = rich_doc();
    let before_parts = zip_entries(&bytes);

    let mut ed = DocxEditor::open(&bytes).unwrap();
    // 同一编辑器：第二章标题改文（pending——不保存）。
    let p = para_of(&ed, "第二章 报价构成");
    ed.set_paragraph_text(p, "第二章 价格与费率").unwrap();
    let plan = ed.recompute_toc(None).unwrap();
    assert_eq!(plan.entries.len(), 3);
    assert_eq!(plan.entries[1].text, "第二章 价格与费率");

    let report = ed.replace_toc_group(&plan, None).unwrap();
    // 三标题全部复用旧锚（toc_0/1/2——#627 书签保真），零新注册。
    assert_eq!(report.names_reused, 3);
    assert!(report.bookmarks_registered.is_empty());
    // 引擎形组：域头 + 3 目录项 + 域尾 = 5 段。
    assert_eq!(report.paragraphs_inserted, 5);

    let saved = ed.save_to_vec().unwrap();

    // ── 保真面：非 document.xml 部件字节恒等 ─────────────────────
    let after_parts = zip_entries(&saved);
    let mut before_map: std::collections::HashMap<String, Vec<u8>> =
        before_parts.into_iter().collect();
    // document.xml 是目标部件（预期变化）——从比对基准剔除。
    before_map.remove("word/document.xml");
    let mut target_seen = false;
    for (name, data) in &after_parts {
        if name == "word/document.xml" {
            target_seen = true;
            continue;
        }
        let orig = before_map.remove(name.as_str());
        assert_eq!(
            orig.as_ref(),
            Some(data),
            "非目标部件字节恒等：{name}（媒体/样式/页眉脚/编号等零触碰）"
        );
    }
    assert!(target_seen, "document.xml 在场");
    assert!(
        before_map.is_empty(),
        "无部件丢失：{:?}",
        before_map.keys().collect::<Vec<_>>()
    );

    // ── 重开：TOC = 新标题树 + 锚可达配平 ────────────────────────
    let xml = doc_xml(&saved);
    let (balanced, names) = bookmark_balance(&xml);
    assert!(balanced, "bookmarkStart/End id 配平");
    for n in &plan.names {
        assert!(
            names.contains(n),
            "PAGEREF 目标锚可达：{n}（计划名 ⊆ 文档锚集）"
        );
    }
    // 复用名保持（不因替换改锚）。
    assert!(names.contains(&"toc_0".to_string()));
    assert!(names.contains(&"toc_1".to_string()));

    let ed2 = DocxEditor::open(&saved).unwrap();
    let plan2 = ed2.recompute_toc(None).unwrap();
    assert_eq!(plan2.entries.len(), 3);
    assert_eq!(plan2.entries[1].text, "第二章 价格与费率", "TOC 反映新树");
    // 缓存语义：存续标题（第一/三章——文本未变）保留旧缓存 "1"；
    // 改名标题（第二章——文本身份语义的新项）无旧缓存 → "—" 显式
    // 待重算（不产假页码）。
    assert_eq!(
        plan2.cached_pages[0],
        Some("1".to_string()),
        "存续章缓存保留"
    );
    assert_eq!(
        plan2.cached_pages[1],
        Some("—".to_string()),
        "改名章显式待重算"
    );
    assert_eq!(
        plan2.cached_pages[2],
        Some("1".to_string()),
        "存续章缓存保留"
    );
    // 未动段不变（正文/表格内容在）。
    assert!(
        (0..ed2.paragraphs().len())
            .any(|i| ed2.paragraph_text(i) == "总则正文：本项目按下列条款执行。"),
        "未动正文段保持"
    );
    assert!(
        (0..ed2.paragraphs().len()).any(|i| ed2.paragraph_text(i).contains("设计费")),
        "表格内容保持"
    );
    assert_eq!(plan2.names, plan.names, "替换后重算名稳定（复用锚闭环）");
}

/// **验收 2b（新标题锚注册）**：旧 TOC 过期（缺新标题）——替换后
/// 新标题获内容寻址锚（bookmarkStart 在标题段），报告登记注册清单。
#[test]
fn inplace_replace_registers_new_heading_anchors() {
    // 旧 TOC 只有三项；标题树有四项（第四章无锚——新标题）。
    let entries = vec![
        TocEntry::new(1, "第一章 总则"),
        TocEntry::new(2, "第二章 报价构成"),
        TocEntry::new(2, "第三章 交付与验收"),
    ];
    let mut w = DocxWriter::new();
    let names = w.toc_entries(&entries, None);
    w.heading_bookmarked(1, "第一章 总则", &names[0]).unwrap();
    w.heading_bookmarked(2, "第二章 报价构成", &names[1])
        .unwrap();
    w.heading_bookmarked(2, "第三章 交付与验收", &names[2])
        .unwrap();
    w.heading(3, "第四章 附则与保修").unwrap(); // 新标题——无锚
    let bytes = w.save_to_vec().unwrap();

    let mut ed = DocxEditor::open(&bytes).unwrap();
    let plan = ed.recompute_toc(None).unwrap();
    assert_eq!(plan.entries.len(), 4);
    let fresh = &plan.names[3];
    assert!(fresh.starts_with("toc_h"), "新项内容寻址名：{fresh}");

    let report = ed.replace_toc_group(&plan, None).unwrap();
    assert_eq!(report.names_reused, 3, "三章复用旧锚");
    assert_eq!(
        report.bookmarks_registered,
        vec![fresh.clone()],
        "第四章注册新锚"
    );

    let saved = ed.save_to_vec().unwrap();
    let xml = doc_xml(&saved);
    let (balanced, bm_names) = bookmark_balance(&xml);
    assert!(balanced);
    assert!(bm_names.contains(fresh), "新锚落盘（PAGEREF 可达）");
    // 锚在标题段内（bookmarkStart 后随第四章文本——邻近断言）。
    let at = xml.find(&format!("w:name=\"{fresh}\"")).unwrap();
    let tail = &xml[at..];
    let near = &tail[..tail.len().min(400)];
    assert!(
        near.contains("第四章 附则与保修"),
        "锚注册在标题段（非正文段）"
    );

    // 重开：四项 TOC + 全锚可达。
    let ed2 = DocxEditor::open(&saved).unwrap();
    let plan2 = ed2.recompute_toc(None).unwrap();
    assert_eq!(plan2.entries.len(), 4);
    assert_eq!(plan2.names, plan.names, "二次重算名稳定（锚注册闭环）");
}

/// **验收 3（Word 真机形态可替换）**：纯文本/超链接结果段的旧 TOC
/// 整组替换为引擎形——重开后提取/锚配平均正常。
#[test]
fn word_form_toc_replaceable() {
    let document = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
        "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">",
        "<w:body>",
        "<w:p><w:r><w:fldChar w:fldCharType=\"begin\"/></w:r>",
        "<w:r><w:instrText xml:space=\"preserve\"> TOC \\o \"1-3\" \\h \\z \\u </w:instrText></w:r>",
        "<w:r><w:fldChar w:fldCharType=\"separate\"/></w:r></w:p>",
        "<w:p><w:r><w:t>第一章 总则</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>3</w:t></w:r></w:p>",
        "<w:p><w:hyperlink w:anchor=\"_Toc123\" w:history=\"1\">",
        "<w:r><w:t>第二章 报价构成</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>5</w:t></w:r></w:hyperlink></w:p>",
        "<w:p><w:r><w:fldChar w:fldCharType=\"end\"/></w:r></w:p>",
        "<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t>第一章 总则</w:t></w:r></w:p>",
        "<w:p><w:pPr><w:pStyle w:val=\"Heading2\"/></w:pPr><w:r><w:t>第二章 报价构成</w:t></w:r></w:p>",
        "<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/></w:sectPr>",
        "</w:body></w:document>"
    );
    let ct = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
        "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
        "<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>",
        "<Default Extension=\"xml\" ContentType=\"application/xml\"/>",
        "<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>",
        "</Types>"
    );
    let rr = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
        "<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>",
        "</Relationships>"
    );
    let dr = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>"
    );
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (n, d) in [
        ("[Content_Types].xml", ct),
        ("_rels/.rels", rr),
        ("word/_rels/document.xml.rels", dr),
        ("word/document.xml", document),
    ] {
        zw.start_file(n, zip::write::SimpleFileOptions::default())
            .unwrap();
        zw.write_all(d.as_bytes()).unwrap();
    }
    let bytes = zw.finish().unwrap().into_inner();

    let mut ed = DocxEditor::open(&bytes).unwrap();
    let plan = ed.recompute_toc(None).unwrap();
    assert_eq!(
        plan.cached_pages,
        vec![Some("3".to_string()), Some("5".to_string())]
    );

    // 两标题无既有锚 → 全部注册新锚。
    let report = ed.replace_toc_group(&plan, None).unwrap();
    assert_eq!(report.names_reused, 0);
    assert_eq!(report.bookmarks_registered.len(), 2);

    let saved = ed.save_to_vec().unwrap();
    let xml = doc_xml(&saved);
    let (balanced, bm_names) = bookmark_balance(&xml);
    assert!(balanced, "Word 形态替换后锚配平");
    for n in &plan.names {
        assert!(bm_names.contains(n), "锚可达：{n}");
    }
    // 旧 _Toc123 超链接锚随组消亡（引擎域外悬空为登记局限——不在锚集）。
    let mut ed2 = DocxEditor::open(&saved).unwrap();
    let plan2 = ed2.recompute_toc(None).unwrap();
    assert_eq!(plan2.entries.len(), 2);
    assert_eq!(plan2.names, plan.names, "替换后名稳定");
    // 引擎形组可再次替换（幂等链路）。
    let r2 = ed2.replace_toc_group(&plan2, None).unwrap();
    assert_eq!(r2.names_reused, 2, "二次替换全复用（幂等）");
}

/// **拒面：无 TOC 域 / 计划长度不一致（V13）/ 组内域编辑冲突 /
/// 过期计划锚定位失败（状态零变化）**。
#[test]
fn reject_faces_are_fail_closed() {
    // ① 无 TOC 域。
    let mut w = DocxWriter::new();
    w.heading(1, "无目录文档").unwrap();
    let no_toc = w.save_to_vec().unwrap();
    let mut ed = DocxEditor::open(&no_toc).unwrap();
    let plan = ed.recompute_toc(None).unwrap();
    let err = ed.replace_toc_group(&plan, None).unwrap_err();
    assert!(err.to_string().contains("无 TOC 域"), "无目录拒：{err}");

    // ② 计划长度不一致（V13 同判据）。
    let bytes = rich_doc();
    let mut ed = DocxEditor::open(&bytes).unwrap();
    let plan = ed.recompute_toc(None).unwrap();
    let tampered = TocRecompute {
        entries: plan.entries.clone(),
        names: vec!["toc_h0000001".to_string()], // 长度不符
        cached_pages: plan.cached_pages.clone(),
        old_toc_present: plan.old_toc_present,
        diff: plan.diff.clone(),
    };
    let err = ed.replace_toc_group(&tampered, None).unwrap_err();
    assert!(err.to_string().contains("V13"), "计划长度拒：{err}");

    // ③ 组内域编辑冲突（旧 TOC 的 PAGEREF 显示值待落账）。
    let mut ed = DocxEditor::open(&bytes).unwrap();
    // 找 TOC 组内的可编辑域（目录项段的 PAGEREF——same_para 有结果区）。
    let in_group = {
        let toc = ed
            .fields()
            .into_iter()
            .find(|f| f.kind == mdc_office_io::docx_edit::FieldKind::Toc)
            .unwrap();
        ed.fields()
            .into_iter()
            .enumerate()
            .find(|(_, f)| f.editable && f.para > toc.para && f.para <= toc.end_para)
    };
    if let Some((idx, f)) = in_group {
        ed.set_field_value(f.para, f.field_idx, "9").unwrap();
        let plan = ed.recompute_toc(None).unwrap();
        let err = ed.replace_toc_group(&plan, None).unwrap_err();
        assert!(
            err.to_string().contains("互斥") || err.to_string().contains("冲突"),
            "组内域编辑冲突拒：{err}"
        );
        // 状态零变化：域编辑仍在（save 产物含编辑值）。
        let saved = ed.save_to_vec().unwrap();
        let xml = doc_xml(&saved);
        assert!(xml.contains("9"), "拒后 pending 域编辑保持（脏态）");
        let _ = idx;
    }

    // ④ 过期计划：recompute 后再改新标题文本（fresh 锚定位失败）。
    let entries = vec![TocEntry::new(1, "第一章 总则")];
    let mut w = DocxWriter::new();
    let names = w.toc_entries(&entries, None);
    w.heading_bookmarked(1, "第一章 总则", &names[0]).unwrap();
    w.heading(2, "第二章 附则").unwrap(); // 新标题（fresh 名）
    let bytes = w.save_to_vec().unwrap();
    let mut ed = DocxEditor::open(&bytes).unwrap();
    let plan = ed.recompute_toc(None).unwrap();
    assert!(plan.names[1].starts_with("toc_h"));
    // 计划过期：改第二章文本（fresh 条目文本不再可定位）。
    let p = para_of(&ed, "第二章 附则");
    ed.set_paragraph_text(p, "第二章 附则（修订）").unwrap();
    let err = ed.replace_toc_group(&plan, None).unwrap_err();
    assert!(err.to_string().contains("锚注册失败"), "过期计划拒：{err}");
    // 状态零变化：pending 编辑仍在、TOC 组未动（save 后重开验证：
    // 旧 TOC 域在场且仍是单项旧缓存——重算的 old 侧未被动过）。
    let saved = ed.save_to_vec().unwrap();
    let xml = doc_xml(&saved);
    assert!(
        xml.contains("第二章 附则（修订）"),
        "拒后 pending 文本编辑保持（脏态不丢）"
    );
    let ed2 = DocxEditor::open(&saved).unwrap();
    assert!(
        ed2.fields()
            .iter()
            .any(|f| f.kind == mdc_office_io::docx_edit::FieldKind::Toc),
        "旧 TOC 域未被动过"
    );
    let reopen = ed2.recompute_toc(None).unwrap();
    // 旧 TOC 仍是单项（第一章）且带旧缓存——替换确实未发生。
    assert!(reopen.old_toc_present, "旧 TOC 在场（未替换）");
    let ch1 = reopen
        .entries
        .iter()
        .position(|e| e.text == "第一章 总则")
        .unwrap();
    assert_eq!(
        reopen.cached_pages[ch1],
        Some("1".to_string()),
        "旧缓存原样"
    );
    // 第二章修订文本在标题树（编辑落盘）。
    assert!(reopen
        .entries
        .iter()
        .any(|e| e.text == "第二章 附则（修订）"));
}

/// 空计划替换：占位单段（与静态面空退化同构）。
#[test]
fn empty_plan_replaces_to_placeholder() {
    let bytes = rich_doc();
    let mut ed = DocxEditor::open(&bytes).unwrap();
    // heading_range 1-1 过滤：只有第一章入计划？——用空文档面构造：
    // 直接手造空计划（entries 空）。
    let empty_plan = TocRecompute {
        entries: vec![],
        names: vec![],
        cached_pages: vec![],
        old_toc_present: true,
        diff: Default::default(),
    };
    let report = ed.replace_toc_group(&empty_plan, None).unwrap();
    assert_eq!(report.paragraphs_inserted, 1, "占位五连域段单段");
    assert_eq!(report.names_reused, 0);
    let saved = ed.save_to_vec().unwrap();
    let ed2 = DocxEditor::open(&saved).unwrap();
    assert!(ed2
        .fields()
        .iter()
        .any(|f| f.kind == mdc_office_io::docx_edit::FieldKind::Toc));
    // 标题树不受替换影响（四标题段全在——重算恢复全量）。
    let plan2 = ed2.recompute_toc(None).unwrap();
    assert_eq!(plan2.entries.len(), 3, "标题树原样（替换只动 TOC 组）");
}
