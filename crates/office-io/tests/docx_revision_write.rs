//! #413 —— DOCX 修订标记写面验收（w:ins/w:del 输出 + 读回往返）。
//!
//! 语义基线（读写对称——读面 #17 在档语义）：
//! - `w:ins` 内文本 = **当前生效文本**：读回提取 + run 带
//!   `revision_inserted = true`（Word 显示为插入修订）；
//! - `w:delText` = **已删除内容**：读回不提取 + 逐段登记
//!   `WarningKind::RevisionDelete`（Word 显示为删除线修订）。
//!
//! 验收对照（issue #413）：
//! 1. 写入 w:ins，Word 显示为插入修订 → XML 结构断言（w:ins 包裹
//!    w:r/w:t + author/date/id 属性）+ LO 跨消费者转换探针（ignore
//!    门控——需 LO 环境，本仓 CI 无 LO，与 g1_lo_probe 同纪律）；
//! 2. 写入 w:del，Word 显示删除线 → 同上（w:delText 承载）；
//! 3. 自己的读面能读回（往返一致）→ 常开断言（本文件主体）；
//! 4. author/date/id 正确保留 → XML 属性断言（id 由 docx-rs
//!    HistoryId 自动分配——断言在场 + 唯一，见模块文档偏差 2）。
//!
//! 偏差登记：issue 验收 1/2 的「Word 打开显示」在无 Word 环境下以
//! ① OOXML 结构逐字段断言 ② 本沙箱 LibreOffice headless 转换存活
//! （探针）双证据代偿，如实登记。

use mdc_office_io::docx::{read_docx_from_bytes_with_options, Block, DocxWriter};
use mdc_office_io::docx_revision::RevisionMark;
use mdc_office_io::io::{ReadOptions, WarningKind};

const INS_TEXT: &str = "新增：质保期延长至 5 年";
const DEL_TEXT: &str = "原报价有效期 30 天";
const AUTHOR: &str = "评审人A";
const DATE: &str = "2026-11-07T08:30:00Z";

fn mark() -> RevisionMark {
    RevisionMark::new(AUTHOR, DATE)
}

/// 三段修订文档：普通段 + 插入修订段 + 删除修订段（混合形态——
/// redline 的最小真实切片）。
fn mixed_revision_docx() -> Vec<u8> {
    let mut w = DocxWriter::new();
    w.heading(1, "第二章 报价构成").unwrap();
    w.paragraph_text("报价以最新澄清文件为准。");
    w.insert_run_revised(INS_TEXT, &mark());
    w.delete_run_revised(DEL_TEXT, &mark());
    w.save_to_vec().expect("修订文档应可保存")
}

/// 读 zip 内 document.xml 原文（XML 结构断言面——zip 为产品依赖）。
fn document_xml(bytes: &[u8]) -> String {
    use std::io::Read as _;
    let cursor = std::io::Cursor::new(bytes);
    let mut za = zip::ZipArchive::new(cursor).expect("产物应为合法 zip");
    let mut zf = za.by_name("word/document.xml").expect("document.xml 在场");
    let mut s = String::new();
    zf.read_to_string(&mut s).unwrap();
    s
}

// ----------------------------------------------------------- 往返读回

#[test]
fn ins_roundtrip_extracted_with_flag() {
    // 验收 3（ins 面）：写出的 w:ins 被自己的读面读回——文本提取 +
    // run 级 revision_inserted 标记（读面 #17 语义对称）。
    let bytes = mixed_revision_docx();
    let doc = read_docx_from_bytes_with_options(&bytes, &ReadOptions { compat: true })
        .expect("修订文档应可读回");

    let mut flagged = String::new();
    let mut ins_runs = 0usize;
    for b in &doc.blocks {
        if let Block::Paragraph(p) = b {
            for r in &p.runs {
                if r.revision_inserted {
                    ins_runs += 1;
                    flagged.push_str(&r.text);
                }
            }
        }
    }
    assert_eq!(flagged, INS_TEXT, "ins 文本完整提取且带标记");
    assert!(
        ins_runs >= 1,
        "至少一个 revision_inserted run（实测 {ins_runs}）"
    );
    // 普通段零误标。
    let normal_flagged = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p),
            _ => None,
        })
        .flat_map(|p| p.runs.iter())
        .filter(|r| !r.revision_inserted && r.text.contains("报价"))
        .count();
    assert!(normal_flagged >= 1, "普通段文本正常提取");
}

#[test]
fn del_roundtrip_skipped_with_warning() {
    // 验收 3（del 面）：delText 不在读回文本 + RevisionDelete 登记可见。
    let bytes = mixed_revision_docx();
    let doc = read_docx_from_bytes_with_options(&bytes, &ReadOptions { compat: true })
        .expect("修订文档应可读回");

    let all_text: Vec<String> = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p.text()),
            _ => None,
        })
        .collect();
    assert!(
        all_text.iter().all(|t| !t.contains(DEL_TEXT)),
        "已删除文本零提取: {all_text:?}"
    );
    let rev_warnings = doc
        .warnings
        .iter()
        .filter(|w| w.kind == WarningKind::RevisionDelete)
        .count();
    assert!(
        rev_warnings >= 1,
        "RevisionDelete warning ≥ 1（实测 {rev_warnings}）"
    );
}

#[test]
fn plain_paragraphs_untouched_by_revision_emission() {
    // 控制组：修订段发射不影响普通段/标题的读回（混合文档里普通
    // 面语义不变——门是加法，不是负担）。
    let bytes = mixed_revision_docx();
    let doc = read_docx_from_bytes_with_options(&bytes, &ReadOptions { compat: true }).unwrap();
    let all_text: Vec<String> = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p.text()),
            _ => None,
        })
        .collect();
    assert!(
        all_text
            .iter()
            .any(|t| t.contains("报价以最新澄清文件为准")),
        "普通段读回不变: {all_text:?}"
    );
}

// ------------------------------------------------------- XML 结构面

#[test]
fn ins_xml_shape_author_date_id() {
    // 验收 1 + 4：w:ins 包裹 w:r/w:t + w:author/w:date/w:id 属性在场。
    let xml = document_xml(&mixed_revision_docx());
    assert!(
        xml.contains("<w:ins "),
        "w:ins 元素在场（Word 插入修订承载）: 前 400 字符 {}",
        &xml[..xml.len().min(400)]
    );
    // author/date 属性（RevisionMark 直落——转义面由 docx-rs 内聚）。
    assert!(
        xml.contains("w:author=\"评审人A\""),
        "w:author 保留: {xml:?}"
    );
    assert!(
        xml.contains("w:date=\"2026-11-07T08:30:00Z\""),
        "w:date 保留（ISO 8601 直落）"
    );
    assert!(
        xml.contains("w:id="),
        "w:id 在场（docx-rs HistoryId 自动分配）"
    );
    // ins 文本在 w:t 承载（当前生效文本——非 delText）。
    let ins_pos = xml.find("<w:ins ").expect("ins 在场");
    let ins_end = xmlins_close(&xml, ins_pos);
    let ins_frag = &xml[ins_pos..ins_end];
    assert!(
        ins_frag.contains("<w:t") && ins_frag.contains(INS_TEXT),
        "ins 内为 w:t 承载当前生效文本: {ins_frag}"
    );
}

#[test]
fn del_xml_shape_deltext_not_wt() {
    // 验收 2 + 4：w:del 包裹 w:r/**w:delText**（非 w:t——OOXML 删除
    // 修订承载约定），author/date/id 同 ins。
    let xml = document_xml(&mixed_revision_docx());
    assert!(
        xml.contains("<w:del "),
        "w:del 元素在场（Word 删除线修订承载）"
    );
    let del_pos = xml.find("<w:del ").expect("del 在场");
    let del_end = xmlins_close(&xml, del_pos);
    let del_frag = &xml[del_pos..del_end];
    assert!(
        del_frag.contains("<w:delText"),
        "del 内为 w:delText 承载（w:t 在 w:del 内是非法形态）: {del_frag}"
    );
    assert!(
        del_frag.contains(DEL_TEXT),
        "删除文本在 delText 内: {del_frag}"
    );
    assert!(!del_frag.contains("<w:t>"), "del 容器内零 w:t: {del_frag}");
    assert!(
        del_frag.contains("w:author=\"评审人A\""),
        "del 的 author 保留"
    );
}

#[test]
fn ids_unique_across_marks() {
    // 验收 4（id 面）：多个修订标记的 w:id 互异（HistoryId 全局
    // 计数器——本断言锁「不撞号」，#415 的 MD 块 ID 撞号是另一面）。
    let mut w = DocxWriter::new();
    for i in 0..4 {
        w.insert_run_revised(&format!("新增条目 {i}"), &mark());
        w.delete_run_revised(&format!("删除条目 {i}"), &mark());
    }
    let xml = document_xml(&w.save_to_vec().unwrap());
    let ids: Vec<&str> = xml
        .split("<w:ins ")
        .skip(1)
        .filter_map(|frag| {
            let end = frag.find('>').unwrap_or(frag.len());
            frag[..end].split("w:id=\"").nth(1).map(|rest| {
                let id_end = rest.find('"').unwrap_or(rest.len());
                &rest[..id_end]
            })
        })
        .collect();
    assert_eq!(ids.len(), 4, "4 个 ins 标记全部带 id: {ids:?}");
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "id 互异: {ids:?}");
}

/// 从 `<w:ins `/`<w:del ` 起位找对应闭合标签（粗切——结构断言足够；
/// 不引 XML 解析依赖）。
fn xmlins_close(xml: &str, from: usize) -> usize {
    let tag = if xml[from..].starts_with("<w:ins ") {
        "</w:ins>"
    } else {
        "</w:del>"
    };
    xml[from..]
        .find(tag)
        .map(|p| from + p + tag.len())
        .unwrap_or(xml.len())
}

// ------------------------------------------------- LO 跨消费者探针

#[test]
#[ignore = "探针：需 LO 环境显式跑（cargo test -- --ignored，与 g1_lo_probe 同纪律）"]
fn lo_headless_conversion_smoke() {
    // 跨消费者证据：LibreOffice headless 能解析修订文档并转换 PDF
    // （弱于 Word 实开，强于零外部验证——如实定位）。产物落
    // /tmp/rev413/，断言 PDF 非空。
    let bytes = mixed_revision_docx();
    std::fs::create_dir_all("/tmp/rev413").unwrap();
    let docx_path = "/tmp/rev413/revision.docx";
    std::fs::write(docx_path, &bytes).unwrap();
    let out = std::process::Command::new("soffice")
        .args([
            "--headless",
            "--convert-to",
            "pdf",
            "--outdir",
            "/tmp/rev413",
            docx_path,
        ])
        .output()
        .expect("soffice 应可执行");
    assert!(
        out.status.success(),
        "LO 转换应成功: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let pdf = std::fs::read("/tmp/rev413/revision.pdf").expect("PDF 产物应在场");
    assert!(pdf.len() > 500, "PDF 非空（{} 字节）", pdf.len());
    println!("LO 探针通过：revision.pdf {} 字节", pdf.len());
}
