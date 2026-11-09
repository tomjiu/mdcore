//! #460（#309 小件②收口片）—— DOCX 段落缩进写面验收。
//!
//! 读写对称（[`PageGeometry`] 先例）：写面消费读面 [`ParagraphIndent`]
//! 类型（r4-t3 已锁），写出的 `w:ind` 经本仓读面读回对账。
//!
//! 验收对照（issue #460 三条）：
//!
//! 1. 三槽映射 → `xml_attributes_three_slots`（w:left/w:right/
//!    w:firstLine；Hanging 变体另锁）
//! 2. 写→读回往返 → `full_slot_roundtrip_exact`（全槽 Some 逐字段相等）
//!    与 `partial_slots_roundtrip_semantic`（None 槽按语义等价归一：
//!    0 twips ≡ 无缩进——docx-rs 构造面 w:left/w:right 恒在场）
//! 3. 零回归 → `all_none_emits_no_ind`（全 None 不发射 w:ind——与
//!    `paragraph()` 段属性同构）
//!
//! 补充锁：负值粘性拒（V13——save 漏斗零产物）、LO 跨消费者探针
//! （ignore 门控，与 g1_lo_probe / docx_revision_write 同纪律）。

use mdc_office_io::docx::{
    read_docx_from_bytes, DocxWriter, ParagraphIndent, ParagraphIndentSpecial, RunSpec,
};

fn document_xml(bytes: &[u8]) -> String {
    let mut ar = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    use std::io::Read as _;
    let mut v = Vec::new();
    ar.by_name("word/document.xml")
        .unwrap()
        .read_to_end(&mut v)
        .unwrap();
    String::from_utf8(v).unwrap()
}

fn text(t: &str) -> Vec<RunSpec> {
    vec![RunSpec::text(t)]
}

/// 读回首个段落的缩进承载。
fn first_paragraph_indent(bytes: &[u8]) -> Option<ParagraphIndent> {
    let doc = read_docx_from_bytes(bytes).expect("产物应可读回");
    doc.blocks
        .iter()
        .find_map(|b| match b {
            mdc_office_io::docx::Block::Paragraph(p) => Some(p.indent),
            _ => None,
        })
        .flatten()
}

/// None ≡ Some(0) 语义归一（docx-rs 构造面 w:left/w:right 恒在场，
/// None 槽以 0 发射——0 twips ≡ 无缩进）。
fn normalize_zero(v: Option<i32>) -> Option<i32> {
    match v {
        None | Some(0) => None,
        other => other,
    }
}

fn normalized(p: ParagraphIndent) -> ParagraphIndent {
    ParagraphIndent {
        left: normalize_zero(p.left),
        right: normalize_zero(p.right),
        special: p.special,
    }
}

// ------------------------------------------------------------- 验收 1

#[test]
fn xml_attributes_three_slots() {
    let mut w = DocxWriter::new();
    w.paragraph_indented(
        &text("缩进段"),
        ParagraphIndent {
            left: Some(720),
            right: Some(360),
            special: Some(ParagraphIndentSpecial::FirstLine(480)),
        },
    );
    let xml = document_xml(&w.save_to_vec().unwrap());
    assert!(xml.contains("w:left=\"720\""), "left → w:left：{xml}");
    assert!(xml.contains("w:right=\"360\""), "right → w:right：{xml}");
    assert!(
        xml.contains("w:firstLine=\"480\""),
        "FirstLine → w:firstLine：{xml}"
    );
}

#[test]
fn hanging_variant_maps_to_w_hanging() {
    let mut w = DocxWriter::new();
    w.paragraph_indented(
        &text("悬挂段"),
        ParagraphIndent {
            left: Some(720),
            right: None,
            special: Some(ParagraphIndentSpecial::Hanging(360)),
        },
    );
    let xml = document_xml(&w.save_to_vec().unwrap());
    assert!(
        xml.contains("w:hanging=\"360\""),
        "Hanging → w:hanging：{xml}"
    );
}

// ------------------------------------------------------------- 验收 2

#[test]
fn full_slot_roundtrip_exact() {
    // 全槽 Some：写→读回逐字段相等（无归一——docx-rs 构造面保留
    // 全部值）。
    for special in [
        Some(ParagraphIndentSpecial::FirstLine(480)),
        Some(ParagraphIndentSpecial::Hanging(240)),
        None,
    ] {
        let indent = ParagraphIndent {
            left: Some(720),
            right: Some(360),
            special,
        };
        let mut w = DocxWriter::new();
        w.paragraph_indented(&text("全槽往返"), indent);
        let bytes = w.save_to_vec().unwrap();
        assert_eq!(
            first_paragraph_indent(&bytes),
            Some(indent),
            "全槽往返逐字段相等（special={special:?}）"
        );
    }
}

#[test]
fn partial_slots_roundtrip_semantic() {
    // 部分 Some：Some 槽精确往返；None 槽读回 Some(0)（构造面 0 发射）
    // ——按 0 ≡ None 语义归一后逐字段相等。
    let indent = ParagraphIndent {
        left: Some(960),
        right: None,
        special: Some(ParagraphIndentSpecial::FirstLine(480)),
    };
    let mut w = DocxWriter::new();
    w.paragraph_indented(&text("部分槽往返"), indent);
    let bytes = w.save_to_vec().unwrap();
    let read = first_paragraph_indent(&bytes).expect("w:ind 应在读回承载");
    assert_eq!(
        normalized(read),
        normalized(indent),
        "语义等价往返（None ≡ Some(0)）"
    );
    // Some 槽精确值（不经归一）。
    assert_eq!(read.left, Some(960));
    assert!(matches!(
        read.special,
        Some(ParagraphIndentSpecial::FirstLine(480))
    ));
}

// ------------------------------------------------------------- 验收 3

#[test]
fn all_none_emits_no_ind() {
    // 全 None = 不发射 w:ind——段落属性与 paragraph() 无差。
    //（docx-rs paraId 是进程级计数器，跨写器字节对比被其干扰——
    // 改为同写器双段对比：无缩进段 vs 全 None 缩进段的 pPr 应同构。）
    let mut w = DocxWriter::new();
    w.paragraph(&text("第一段"));
    w.paragraph_indented(
        &text("第二段"),
        ParagraphIndent {
            left: None,
            right: None,
            special: None,
        },
    );
    let xml = document_xml(&w.save_to_vec().unwrap());
    assert!(!xml.contains("w:ind"), "全 None 不发射 w:ind：{xml}");
    // 两段的 pPr 片段逐一提取（paraId 归一后）应恒等。
    let pprs: Vec<String> = xml
        .split("<w:p ")
        .skip(1)
        .map(|frag| {
            let ppr = frag.split("</w:pPr>").next().unwrap_or("");
            ppr.split("<w:pPr>")
                .nth(1)
                .map(|s| s.to_string())
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(pprs.len(), 2, "两段都在场");
    assert_eq!(
        pprs[0], pprs[1],
        "无缩进段与全 None 缩进段的 pPr 同构：{pprs:?}"
    );
}

// --------------------------------------------------------- 校验面（V13）

#[test]
fn negative_indent_sticky_reject_zero_product() {
    for bad in [
        ParagraphIndent {
            left: Some(-720),
            right: None,
            special: None,
        },
        ParagraphIndent {
            left: None,
            right: Some(-1),
            special: None,
        },
        ParagraphIndent {
            left: None,
            right: None,
            special: Some(ParagraphIndentSpecial::FirstLine(-480)),
        },
        ParagraphIndent {
            left: None,
            right: None,
            special: Some(ParagraphIndentSpecial::Hanging(-240)),
        },
    ] {
        let mut w = DocxWriter::new();
        w.paragraph_indented(&text("负值拒"), bad);
        let err = w.save_to_vec().expect_err("负值缩进应 save 拒");
        assert!(
            matches!(err, mdc_office_io::docx::DocxError::InvalidInput(_)),
            "粘性校验 InvalidInput（{bad:?}）：{err:?}"
        );
    }
    // 正值边界（0 合法——零缩进显式表达）。
    let mut w = DocxWriter::new();
    w.paragraph_indented(
        &text("零值合法"),
        ParagraphIndent {
            left: Some(0),
            right: None,
            special: None,
        },
    );
    assert!(w.save_to_vec().is_ok(), "0 twips 显式零缩进合法");
}

// ------------------------------------------------- LO 跨消费者探针

#[test]
#[ignore = "探针：需 LO 环境显式跑（cargo test -- --ignored，与 g1_lo_probe 同纪律）"]
fn lo_headless_conversion_smoke() {
    // 跨消费者证据：LibreOffice headless 解析含 w:ind 四槽文档并转 PDF
    // （Word 语义面无 Word 环境——LO 探针 + XML 结构断言双代偿，如实登记）。
    let mut w = DocxWriter::new();
    w.paragraph_indented(
        &text("首行缩进段：报价以最新澄清文件为准。"),
        ParagraphIndent {
            left: Some(720),
            right: Some(360),
            special: Some(ParagraphIndentSpecial::FirstLine(480)),
        },
    );
    w.paragraph_indented(
        &text("悬挂缩进段：质保期延长至五年。"),
        ParagraphIndent {
            left: Some(720),
            right: None,
            special: Some(ParagraphIndentSpecial::Hanging(360)),
        },
    );
    std::fs::create_dir_all("/tmp/ind460").unwrap();
    let docx_path = "/tmp/ind460/indent.docx";
    std::fs::write(docx_path, w.save_to_vec().unwrap()).unwrap();
    let out = std::process::Command::new("soffice")
        .args([
            "--headless",
            "--convert-to",
            "pdf",
            "--outdir",
            "/tmp/ind460",
            docx_path,
        ])
        .output()
        .expect("soffice 应可执行");
    assert!(
        out.status.success(),
        "LO 转换应成功：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let pdf = std::fs::read("/tmp/ind460/indent.pdf").expect("PDF 产物应在场");
    assert!(pdf.len() > 500, "PDF 非空（{} 字节）", pdf.len());
    println!("LO 探针通过：indent.pdf {} 字节", pdf.len());
}
