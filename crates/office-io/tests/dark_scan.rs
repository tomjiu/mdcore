//! 暗标导出后扫描器（#502 / #161 P0-2）测试。
//!
//! 覆盖七验收：①脏 fixture（docx+xlsx）结构化清单逐项命中；②匿名
//! fixture 空清单；③缺 docProps 极简文件零发现；④非 zip typed 拒；
//! ⑤docx/xlsx 专属残留（批注作者/people vs comments 作者）分别命中；
//! ⑥wasm 端到端见 mdcore-wasm tests/dark_scan_wasm.rs；⑦全绿（CI）。
//!
//! fixture 全部手工构造（不 import 清理函数——扫描面独立基线，
//! 与 PR #501 无依赖关系）。

use std::io::Write as _;

use mdc_office_io::dark_scan::{
    scan_identity_residuals_docx, scan_identity_residuals_xlsx, DarkScanError,
};

/// 通用 zip 构造器（部件名 → 内容；docProps 身份面可注入）。
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

const DIRTY_CORE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>张三（某某建设集团）</dc:creator><cp:lastModifiedBy>李四</cp:lastModifiedBy></cp:coreProperties>"#;
const CLEAN_CORE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>unknown</dc:creator><cp:lastModifiedBy>unknown</cp:lastModifiedBy></cp:coreProperties>"#;
const DIRTY_APP: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Company>某某建设集团有限公司</Company><Manager>王五</Manager></Properties>"#;
const CLEAN_APP: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"/>"#;
const DIRTY_CUSTOM: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties"><Property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="投标单位"><vt:lpstr>某某建设集团有限公司</vt:lpstr></Property></Properties>"#;
const DIRTY_DOCX_COMMENTS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:comments xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:comment w:id="1" w:author="张三（某某建设集团）" w:initials="ZS" w:date="2026-10-01T00:00:00Z"><w:p><w:r><w:t>条款 3 需复核</w:t></w:r></w:p></w:comment></w:comments>"#;
const CLEAN_DOCX_COMMENTS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:comments xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:comment w:id="1" w:author="" w:initials="" w:date="1970-01-01T00:00:00Z"><w:p><w:r><w:t>条款 3 需复核</w:t></w:r></w:p></w:comment></w:comments>"#;
const DIRTY_PEOPLE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:people xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:person w:author="张三" w:displayName="张三（某某建设集团）"/></w:people>"#;
const DIRTY_XLSX_COMMENTS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><authors><author>张三（某某建设集团）</author><author>审核员赵六</author></authors><comment ref="A1" authorId="0"><text><r><t>注意：此价格含运费</t></r></text></comment></comments>"#;
const CLEAN_XLSX_COMMENTS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><authors><author></author></authors><comment ref="A1" authorId="0"><text><r><t>注意：此价格含运费</t></r></text></comment></comments>"#;

/// 验收 ①+⑤（DOCX 面）：全量身份字段逐项命中 + 专属面（批注归属/
/// people）覆盖。
#[test]
fn dirty_docx_full_hit() {
    let bytes = build_zip(&[
        ("docProps/core.xml", DIRTY_CORE),
        ("docProps/app.xml", DIRTY_APP),
        ("docProps/custom.xml", DIRTY_CUSTOM),
        ("word/document.xml", "<w:document/>"),
        ("word/comments.xml", DIRTY_DOCX_COMMENTS),
        ("word/people.xml", DIRTY_PEOPLE),
    ]);
    let found = scan_identity_residuals_docx(&bytes).unwrap();
    let summary: Vec<String> = found
        .iter()
        .map(|r| format!("{}:{}={}", r.part, r.field, r.value))
        .collect();
    // 逐项命中（part/field/value 三要素）。
    let expects = [
        ("docProps/core.xml", "dc:creator", "张三（某某建设集团）"),
        ("docProps/core.xml", "cp:lastModifiedBy", "李四"),
        ("docProps/app.xml", "Company", "某某建设集团有限公司"),
        ("docProps/app.xml", "Manager", "王五"),
        (
            "docProps/custom.xml",
            "Property:投标单位",
            "某某建设集团有限公司",
        ),
        ("word/comments.xml", "w:author", "张三（某某建设集团）"),
        ("word/comments.xml", "w:initials", "ZS"),
        ("word/people.xml", "w:person", "张三（某某建设集团）"),
    ];
    for (part, field, value) in expects {
        assert!(
            found
                .iter()
                .any(|r| r.part == part && r.field == field && r.value == value),
            "缺发现 {part}:{field}={value}，实得 {summary:?}"
        );
    }
    assert_eq!(found.len(), expects.len(), "无多余发现：{summary:?}");
    // hint 面非空（人读提示在）。
    assert!(found.iter().all(|r| !r.hint.is_empty()));
}

/// 验收 ①+⑤（XLSX 面）：docProps + comments 作者名命中。
#[test]
fn dirty_xlsx_full_hit() {
    let bytes = build_zip(&[
        ("docProps/core.xml", DIRTY_CORE),
        ("docProps/app.xml", DIRTY_APP),
        ("docProps/custom.xml", DIRTY_CUSTOM),
        ("xl/workbook.xml", "<workbook/>"),
        ("xl/comments1.xml", DIRTY_XLSX_COMMENTS),
    ]);
    let found = scan_identity_residuals_xlsx(&bytes).unwrap();
    let summary: Vec<String> = found
        .iter()
        .map(|r| format!("{}:{}={}", r.part, r.field, r.value))
        .collect();
    for (part, field, value) in [
        ("docProps/core.xml", "dc:creator", "张三（某某建设集团）"),
        ("docProps/core.xml", "cp:lastModifiedBy", "李四"),
        ("docProps/app.xml", "Company", "某某建设集团有限公司"),
        ("docProps/app.xml", "Manager", "王五"),
        (
            "docProps/custom.xml",
            "Property:投标单位",
            "某某建设集团有限公司",
        ),
        ("xl/comments1.xml", "author", "张三（某某建设集团）"),
        ("xl/comments1.xml", "author", "审核员赵六"),
    ] {
        assert!(
            found
                .iter()
                .any(|r| r.part == part && r.field == field && r.value == value),
            "缺发现 {part}:{field}={value}，实得 {summary:?}"
        );
    }
    assert_eq!(found.len(), 7, "无多余发现：{summary:?}");
}

/// 验收 ②：匿名形态（清理后形状——手工构造）→ 空清单（两文档面）。
#[test]
fn anonymous_shape_clean() {
    let docx = build_zip(&[
        ("docProps/core.xml", CLEAN_CORE),
        ("docProps/app.xml", CLEAN_APP),
        ("word/document.xml", "<w:document/>"),
        ("word/comments.xml", CLEAN_DOCX_COMMENTS),
    ]);
    assert!(
        scan_identity_residuals_docx(&docx).unwrap().is_empty(),
        "匿名 DOCX 应零残留"
    );
    let xlsx = build_zip(&[
        ("docProps/core.xml", CLEAN_CORE),
        ("docProps/app.xml", CLEAN_APP),
        ("xl/workbook.xml", "<workbook/>"),
        ("xl/comments1.xml", CLEAN_XLSX_COMMENTS),
    ]);
    assert!(
        scan_identity_residuals_xlsx(&xlsx).unwrap().is_empty(),
        "匿名 XLSX 应零残留"
    );
}

/// 验收 ③：缺 docProps 的极简文件 → 零发现（缺部件非错误）。
#[test]
fn minimal_without_docprops_zero_findings() {
    let docx = build_zip(&[("word/document.xml", "<w:document/>")]);
    assert!(scan_identity_residuals_docx(&docx).unwrap().is_empty());
    let xlsx = build_zip(&[("xl/workbook.xml", "<workbook/>")]);
    assert!(scan_identity_residuals_xlsx(&xlsx).unwrap().is_empty());
}

/// 验收 ④：非 zip 输入 → typed 拒收（fail-closed——不猜）。
#[test]
fn non_zip_rejected() {
    let not_zip = b"this is definitely not a zip file".as_slice();
    let e = scan_identity_residuals_docx(not_zip).unwrap_err();
    assert!(matches!(e, DarkScanError::NotZip(_)), "{e:?}");
    let e = scan_identity_residuals_xlsx(not_zip).unwrap_err();
    assert!(matches!(e, DarkScanError::NotZip(_)), "{e:?}");
}

/// 面交叉纪律：xlsx 扫描不消费 word/* 部件、docx 扫描不消费 xl/*
/// 部件（文档种类判定面互斥——外部杂件不误报）。
#[test]
fn kind_faces_disjoint() {
    let xlsx = build_zip(&[
        ("xl/workbook.xml", "<workbook/>"),
        ("word/comments.xml", DIRTY_DOCX_COMMENTS),
    ]);
    assert!(
        scan_identity_residuals_xlsx(&xlsx).unwrap().is_empty(),
        "xlsx 扫描不消费 word/* 面"
    );
    let docx = build_zip(&[
        ("word/document.xml", "<w:document/>"),
        ("xl/comments1.xml", DIRTY_XLSX_COMMENTS),
    ]);
    assert!(
        scan_identity_residuals_docx(&docx).unwrap().is_empty(),
        "docx 扫描不消费 xl/* 面"
    );
}

/// 批注文本非身份面：批注内容含人名不触发（只有归属字段触发——
/// 内容零触碰语义的扫描侧镜像）。
#[test]
fn comment_text_is_not_identity() {
    let comments = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:comments xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:comment w:id="1" w:author="" w:initials=""><w:p><w:r><w:t>经办人张三请确认</w:t></w:r></w:p></w:comment></w:comments>"#;
    let docx = build_zip(&[
        ("word/document.xml", "<w:document/>"),
        ("word/comments.xml", comments),
    ]);
    assert!(
        scan_identity_residuals_docx(&docx).unwrap().is_empty(),
        "批注正文内容不是归属身份面"
    );
}
