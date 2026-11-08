//! XLSX 暗标元数据清理（#499 / #161 P0-2）测试。
//!
//! 覆盖六验收：①身份字段清零（docProps 三件套 + 批注作者名）+ 批注
//! 文本零触碰；②未触碰部件字节级不变（raw_copy 不变量）；③非 xlsx
//! 拒收（xl/workbook.xml 不在场）；④清理后产物读面重开（往返）；
//! ⑤极简 zip（无 docProps）容忍跳过；⑥rust_xlsxwriter 新建面同构
//! （rebuild 路径产物的清理面）。

use std::io::Read as _;
use std::io::Write as _;

use mdc_office_io::anonymize::anonymize_xlsx_props;
use mdc_office_io::io::read_book_from_bytes;

const NS_MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

/// 读 zip 部件（解压形态——断言面用）。
fn part_bytes(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    let cursor = std::io::Cursor::new(bytes);
    let mut za = zip::ZipArchive::new(cursor).ok()?;
    let mut f = za.by_name(name).ok()?;
    let mut out = Vec::new();
    f.read_to_end(&mut out).ok()?;
    Some(out)
}

fn part_names(bytes: &[u8]) -> Vec<String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut za = zip::ZipArchive::new(cursor).unwrap();
    (0..za.len())
        .map(|i| za.by_index_raw(i).unwrap().name().to_string())
        .collect()
}

/// 带**全量身份元数据 + 批注**的脏 fixture（暗标场景的最小复刻）。
fn dirty_xlsx() -> Vec<u8> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/><Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/><Override PartName="/docProps/custom.xml" ContentType="application/vnd.openxmlformats-officedocument.custom-properties+xml"/><Override PartName="/xl/comments1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"/></Types>"#;
    let root_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/><Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties" Target="docProps/custom.xml"/></Relationships>"#;
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="{NS_MAIN}" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#
    );
    let wb_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="../comments1.xml"/></Relationships>"#;
    let sheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="{NS_MAIN}"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>报价明细</t></is></c><c r="B1"><v>100</v></c></row></sheetData></worksheet>"#
    );
    let core = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/"><dc:creator>张三（某某建设集团）</dc:creator><cp:lastModifiedBy>李四（标书部）</cp:lastModifiedBy><dcterms:created xsi:type="dcterms:W3CDTF" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">2026-10-01T09:00:00Z</dcterms:created><dcterms:modified xsi:type="dcterms:W3CDTF" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">2026-11-01T09:00:00Z</dcterms:modified><cp:revision>7</cp:revision></cp:coreProperties>"#;
    let app = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>Microsoft Excel</Application><Company>某某建设集团有限公司</Company><Manager>王五</Manager></Properties>"#;
    let custom = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><Property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="投标单位"><vt:lpstr>某某建设集团有限公司</vt:lpstr></Property></Properties>"#;
    let comments = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><authors><author>张三（某某建设集团）</author><author>审核员赵六</author></authors><comment ref="A1" authorId="0" guid="{11111111-2222-3333-4444-555555555555}"><text><r><t>注意：此价格含运费</t></r></text></comment><comment ref="B1" authorId="1" guid="{aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee}"><text><r><t>已复核</t></r></text></comment></comments>"#;

    let buf = std::io::Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(buf);
    let opts = zip::write::SimpleFileOptions::default();
    for (name, data) in [
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", root_rels),
        ("docProps/core.xml", core),
        ("docProps/app.xml", app),
        ("docProps/custom.xml", custom),
        ("xl/workbook.xml", &workbook),
        ("xl/_rels/workbook.xml.rels", wb_rels),
        ("xl/worksheets/sheet1.xml", &sheet),
        ("xl/comments1.xml", comments),
    ] {
        zip.start_file::<_, ()>(name, opts).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// 验收 1+2：身份清零 + 批注文本零触碰 + 未触碰部件字节级不变。
#[test]
fn identities_removed_content_intact() {
    let src = dirty_xlsx();
    let out = anonymize_xlsx_props(&src).unwrap();

    // ① core.xml：身份字段 → unknown + epoch + revision 1。
    let core = String::from_utf8(part_bytes(&out, "docProps/core.xml").unwrap()).unwrap();
    assert!(!core.contains("张三"), "creator 身份清除");
    assert!(!core.contains("李四"), "lastModifiedBy 身份清除");
    assert!(core.contains("<dc:creator>unknown</dc:creator>"), "{core}");
    assert!(
        core.contains("<cp:lastModifiedBy>unknown</cp:lastModifiedBy>"),
        "{core}"
    );
    assert!(
        core.contains("1970-01-01T00:00:00Z"),
        "时间 → epoch：{core}"
    );
    assert!(core.contains("<cp:revision>1</cp:revision>"), "{core}");

    // ① app.xml：Company/Manager 整面清空（空骨架）。
    let app = String::from_utf8(part_bytes(&out, "docProps/app.xml").unwrap()).unwrap();
    assert!(!app.contains("某某建设集团"), "{app}");
    assert!(!app.contains("王五"), "{app}");
    assert!(!app.contains("<Application>"), "扩展属性清空：{app}");

    // ① custom.xml：自定义属性（投标单位）整面清空。
    let custom = String::from_utf8(part_bytes(&out, "docProps/custom.xml").unwrap()).unwrap();
    assert!(!custom.contains("投标单位"), "{custom}");
    assert!(!custom.contains("vt:lpstr"), "{custom}");

    // ① comments：作者名清空 + 批注文本零触碰 + authorId 引用保持。
    let comments = String::from_utf8(part_bytes(&out, "xl/comments1.xml").unwrap()).unwrap();
    assert!(!comments.contains("张三"), "批注作者名清除：{comments}");
    assert!(!comments.contains("赵六"), "批注作者名清除：{comments}");
    assert!(
        comments.contains("注意：此价格含运费"),
        "批注文本零触碰：{comments}"
    );
    assert!(comments.contains("已复核"), "批注文本零触碰：{comments}");
    assert!(
        comments.contains("authorId=\"0\""),
        "authorId 引用系保持：{comments}"
    );
    assert!(
        comments.contains("authorId=\"1\""),
        "authorId 引用系保持：{comments}"
    );
    assert!(
        comments.contains("<author></author>") || comments.contains("<author/>"),
        "作者槽位保留（空文本）：{comments}"
    );

    // ② 未触碰部件字节级不变（raw_copy 不变量）。
    assert_eq!(
        part_names(&src),
        part_names(&out),
        "部件清单不变（只换不清不删）"
    );
    for part in [
        "[Content_Types].xml",
        "_rels/.rels",
        "xl/workbook.xml",
        "xl/_rels/workbook.xml.rels",
        "xl/worksheets/sheet1.xml",
    ] {
        assert_eq!(
            part_bytes(&src, part).unwrap(),
            part_bytes(&out, part).unwrap(),
            "{part} 字节级不变"
        );
    }
}

/// 验收 3：非 xlsx（缺 xl/workbook.xml）拒收 fail-closed。
#[test]
fn non_xlsx_rejected() {
    let buf = std::io::Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(buf);
    let opts = zip::write::SimpleFileOptions::default();
    zip.start_file::<_, ()>("docProps/core.xml", opts).unwrap();
    zip.write_all(b"<cp:coreProperties/>").unwrap();
    zip.start_file::<_, ()>("word/document.xml", opts).unwrap();
    zip.write_all(b"<w:document/>").unwrap();
    let docx_like = zip.finish().unwrap().into_inner();

    let err = anonymize_xlsx_props(&docx_like).unwrap_err();
    assert!(
        err.to_string().contains("workbook.xml"),
        "非 xlsx 拒收：{err}"
    );
}

/// 验收 4：清理后产物可被读面重开（往返——内容可见）。
#[test]
fn reopened_by_read_face() {
    let src = dirty_xlsx();
    let out = anonymize_xlsx_props(&src).unwrap();
    let book = read_book_from_bytes(&out).expect("读面重开清理后产物");
    assert_eq!(book.sheets.len(), 1);
    // 首格文本在（内容零丢失——读面形态断言）。
    let d = &book.sheets[0];
    assert!(
        format!("{:?}", d.typed.get_abs(0, 0)).contains("报价"),
        "A1 文本在（读面可见）：{:?}",
        d.typed.get_abs(0, 0)
    );
}

/// 验收 5：极简 zip（无 docProps/comments）容忍——对应面跳过。
#[test]
fn minimal_zip_without_docprops_tolerated() {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#;
    let root_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="{NS_MAIN}" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#
    );
    let wb_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
    let sheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="{NS_MAIN}"><sheetData/></worksheet>"#
    );

    let buf = std::io::Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(buf);
    let opts = zip::write::SimpleFileOptions::default();
    for (name, data) in [
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", root_rels),
        ("xl/workbook.xml", &workbook),
        ("xl/_rels/workbook.xml.rels", wb_rels),
        ("xl/worksheets/sheet1.xml", &sheet),
    ] {
        zip.start_file::<_, ()>(name, opts).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    let minimal = zip.finish().unwrap().into_inner();

    let out = anonymize_xlsx_props(&minimal).unwrap();
    assert!(read_book_from_bytes(&out).is_ok(), "极简产物读面可开");
}

/// 验收 6：rust_xlsxwriter 新建面产物（rebuild 路径形状）同样可清理
/// ——docProps 若在场即匿名（常量同构断言：unknown/epoch/revision 1）。
#[test]
fn writer_output_anonymizable() {
    let mut w = mdc_office_io::write::WorkbookWriter::new();
    let _ = w.add_sheet("S").unwrap();
    let bytes = w.save_to_vec().unwrap();
    let out = anonymize_xlsx_props(&bytes).unwrap();
    // 新建面本就近匿名（单一真源语义）——再过一遍 = 幂等可重开。
    assert!(read_book_from_bytes(&out).is_ok());
    if let Some(core) = part_bytes(&out, "docProps/core.xml") {
        let core = String::from_utf8(core).unwrap();
        assert!(
            !core.contains("rust_xlsxwriter") || core.contains("unknown"),
            "{core}"
        );
    }
}
