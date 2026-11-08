//! dark_scan wasm 出口测试（#502——纯内核面，原生 cargo test）。
//! 覆盖验收 ⑥：两 kind × 脏/净四态 + 错误面（坏 kind / 非 zip）。

use std::io::Write as _;

use mdcore_wasm::dark_scan::dark_scan_core;

fn build_zip(parts: &[(&str, &str)]) -> Vec<u8> {
    let buf = std::io::Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(buf);
    let opts = zip::write::SimpleFileOptions::default();
    for (name, data) in parts {
        zip.start_file::<_, ()>(name, opts).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

const DIRTY_CORE: &str = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>张三</dc:creator><cp:lastModifiedBy>李四</cp:lastModifiedBy></cp:coreProperties>"#;
const CLEAN_CORE: &str = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>unknown</dc:creator><cp:lastModifiedBy>unknown</cp:lastModifiedBy></cp:coreProperties>"#;

#[test]
fn dark_scan_wasm_four_states() {
    // 脏 docx：JSON 数组命中（part/field/value/hint 四字段形态）。
    let dirty_docx = build_zip(&[
        ("word/document.xml", "<w:document/>"),
        ("docProps/core.xml", DIRTY_CORE),
    ]);
    let r = dark_scan_core(&dirty_docx, "docx").unwrap();
    assert!(r.contains(r#""part":"docProps/core.xml""#), "{r}");
    assert!(r.contains(r#""field":"dc:creator""#), "{r}");
    assert!(r.contains(r#""value":"张三""#), "{r}");
    assert!(r.contains(r#""hint":"#), "{r}");

    // 净 docx：空数组。
    let clean_docx = build_zip(&[
        ("word/document.xml", "<w:document/>"),
        ("docProps/core.xml", CLEAN_CORE),
    ]);
    assert_eq!(dark_scan_core(&clean_docx, "docx").unwrap(), "[]");

    // 脏 xlsx：命中；净 xlsx：空。
    let dirty_xlsx = build_zip(&[
        ("xl/workbook.xml", "<workbook/>"),
        ("docProps/core.xml", DIRTY_CORE),
    ]);
    let r = dark_scan_core(&dirty_xlsx, "xlsx").unwrap();
    assert!(r.contains(r#""field":"cp:lastModifiedBy""#), "{r}");
    assert!(r.contains(r#""value":"李四""#), "{r}");
    let clean_xlsx = build_zip(&[
        ("xl/workbook.xml", "<workbook/>"),
        ("docProps/core.xml", CLEAN_CORE),
    ]);
    assert_eq!(dark_scan_core(&clean_xlsx, "xlsx").unwrap(), "[]");
}

#[test]
fn dark_scan_wasm_error_surface() {
    // 坏 kind：typed 码。
    let bytes = build_zip(&[("xl/workbook.xml", "<workbook/>")]);
    let e = dark_scan_core(&bytes, "pptx").unwrap_err();
    assert_eq!(e.code, "dark_scan");
    assert!(e.message.contains("pptx"), "{}", e.message);

    // 非 zip：typed 拒。
    let e = dark_scan_core(b"not a zip", "docx").unwrap_err();
    assert_eq!(e.code, "dark_scan");
    assert!(e.message.contains("zip"), "{}", e.message);
}
