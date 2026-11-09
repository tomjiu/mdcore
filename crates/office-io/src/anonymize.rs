//! OOXML 元数据匿名化（暗标清理——#161 P0-2）。
//!
//! ## 定位
//! 共享 OPC 常量 + XLSX 面（[`anonymize_xlsx_props`]）。DOCX 面
//! [`crate::docx::anonymize_docx_props`] 的姊妹件——两者共用
//! `docProps` 三件套匿名常量（OPC core/app/custom schema 跨
//! OOXML 文档类型同构，字节级同一真源）。
//!
//! ## XLSX 变换面（`anonymize_xlsx_props`，#499）
//! - `docProps/core.xml` → 匿名常量（unknown 作者 + epoch 零时 +
//!   revision=1——与 DOCX 新建面逐字节同构）；
//! - `docProps/app.xml` / `docProps/custom.xml` → 空骨架（Company/
//!   Manager/Sender 等扩展与 VT 自定义属性整面清空）；
//! - `xl/comments*.xml` → `<authors>` 块作者名清空（**批注文本零
//!   触碰**；`authorId` 引用系按索引解析，名字清空后引用仍有效）；
//! - 其余部件 `raw_copy` 字节级透传（worksheet/样式/关系等零接触）；
//! - `xl/workbook.xml` 不在场 = 拒（非 xlsx 伪产物 fail-closed——
//!   与 DOCX 面「document.xml 不在场拒」同构）。
//!
//! **不变量（内容零触碰）**：worksheet 部件及除 docProps/comments
//! 外的一切部件原压缩流原样拷贝。容忍缺部件（无 docProps 的极简
//! zip 跳过对应面）。
//!
//! **边界**：`xl/threadedComments/*.xml`（线程评论 displayName）读面
//! 未消费，登记后续（#499「不改」节）；修订/保护等无作者面部件不涉及。

use std::io::Read as _;

use crate::xlsx_edit::XlsxEditError;

/// 匿名 core.xml（与引擎新建面 `DocxWriter::save_to_vec` 产物**逐字节
/// 同构**——探针 g3_docx_docprops_inventory 实测锚点。「新建」与
/// 「匿名导出」元数据面一致 = 单一真源语义：unknown 作者 + epoch
/// 零时 + revision=1）。OPC core-properties schema 跨 OOXML 通用
/// （docx/xlsx/pptx 同一部件定义）——本常量为 DOCX/XLSX 两面共享。
pub(crate) const ANON_CORE_XML: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<cp:coreProperties"#,
    r#" xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties""#,
    r#" xmlns:dc="http://purl.org/dc/elements/1.1/""#,
    r#" xmlns:dcterms="http://purl.org/dc/terms/""#,
    r#" xmlns:dcmitype="http://purl.org/dc/dcmitype/""#,
    r#" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">"#,
    r#"<dcterms:created xsi:type="dcterms:W3CDTF">1970-01-01T00:00:00Z</dcterms:created>"#,
    r#"<dc:creator>unknown</dc:creator>"#,
    r#"<cp:lastModifiedBy>unknown</cp:lastModifiedBy>"#,
    r#"<dcterms:modified xsi:type="dcterms:W3CDTF">1970-01-01T00:00:00Z</dcterms:modified>"#,
    r#"<cp:revision>1</cp:revision>"#,
    r#"</cp:coreProperties>"#,
);

/// 匿名 app.xml / custom.xml（空骨架——与引擎新建面同构；custom 的
/// Company/Sender 等 VT 自定义属性整面清空）。OPC 扩展/自定义属性
/// schema 跨 OOXML 通用——DOCX/XLSX 两面共享。
pub(crate) const ANON_APP_XML: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties""#,
    r#" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"/>"#,
);
pub(crate) const ANON_CUSTOM_XML: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties""#,
    r#" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"/>"#,
);

/// XLSX 暗标元数据清理（#499 / #161 P0-2）——变换面与不变量见模块文档。
///
/// 与 DOCX 面 [`crate::docx::anonymize_docx_props`] 同构：入口字节 →
/// 清理后字节；**不校验内容语义**（读面/编辑面的职责），只做部件级
/// 替换与透传。
pub fn anonymize_xlsx_props(bytes: &[u8]) -> Result<Vec<u8>, XlsxEditError> {
    use std::io::Write as _;
    let cursor = std::io::Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| XlsxEditError::Io(format!("open zip: {e}")))?;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let mut has_workbook = false;
    for i in 0..archive.len() {
        // 两段借用：by_index_raw 仅用于名字发现与 raw_copy（未解压流）；
        // comments 需解压读改 → 独立 by_name（解压模式）。
        let name = {
            let entry = archive
                .by_index_raw(i)
                .map_err(|e| XlsxEditError::Io(format!("zip index: {e}")))?;
            entry.name().to_string()
        };
        if name == "xl/workbook.xml" {
            has_workbook = true;
        }
        match name.as_str() {
            "docProps/core.xml" => {
                writer
                    .start_file(name.clone(), zip::write::SimpleFileOptions::default())
                    .map_err(|e| XlsxEditError::Io(format!("zip start: {e}")))?;
                writer
                    .write_all(ANON_CORE_XML.as_bytes())
                    .map_err(|e| XlsxEditError::Io(format!("zip write: {e}")))?;
            }
            "docProps/app.xml" | "docProps/custom.xml" => {
                let skeleton = if name == "docProps/app.xml" {
                    ANON_APP_XML
                } else {
                    ANON_CUSTOM_XML
                };
                writer
                    .start_file(name.clone(), zip::write::SimpleFileOptions::default())
                    .map_err(|e| XlsxEditError::Io(format!("zip start: {e}")))?;
                writer
                    .write_all(skeleton.as_bytes())
                    .map_err(|e| XlsxEditError::Io(format!("zip write: {e}")))?;
            }
            _ if is_xlsx_comments_part(&name) => {
                let mut xml = String::new();
                archive
                    .by_name(&name)
                    .map_err(|e| XlsxEditError::Io(format!("zip by_name: {e}")))?
                    .read_to_string(&mut xml)
                    .map_err(|e| XlsxEditError::Io(format!("zip read: {e}")))?;
                let cleaned = anonymize_xlsx_comments_xml(&xml, &name)?;
                writer
                    .start_file(name.clone(), zip::write::SimpleFileOptions::default())
                    .map_err(|e| XlsxEditError::Io(format!("zip start: {e}")))?;
                writer
                    .write_all(cleaned.as_bytes())
                    .map_err(|e| XlsxEditError::Io(format!("zip write: {e}")))?;
            }
            _ => {
                let entry = archive
                    .by_index_raw(i)
                    .map_err(|e| XlsxEditError::Io(format!("zip index: {e}")))?;
                writer
                    .raw_copy_file(entry)
                    .map_err(|e| XlsxEditError::Io(format!("zip copy: {e}")))?;
            }
        }
    }
    if !has_workbook {
        return Err(XlsxEditError::InvalidInput(
            "xl/workbook.xml 不在场——非 xlsx 伪产物（fail-closed）".into(),
        ));
    }
    let out = writer
        .finish()
        .map_err(|e| XlsxEditError::Io(format!("zip finish: {e}")))?;
    Ok(out.into_inner())
}

/// 批注部件名判定（`xl/comments*.xml`——SS 部件名约定；根相对路径，
/// 与 repack_xlsx 的部件名口径一致）。
fn is_xlsx_comments_part(name: &str) -> bool {
    name.starts_with("xl/comments") && name.ends_with(".xml")
}

/// comments XML 作者名清空（quick-xml 事件流——`<author>文本</author>`
/// → `<author></author>`；**批注文本零触碰**；authorId 引用按索引
/// 解析，名字清空后引用仍指向有效槽位）。
fn anonymize_xlsx_comments_xml(xml: &str, part: &str) -> Result<String, XlsxEditError> {
    use quick_xml::events::Event;
    use quick_xml::Reader;
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut writer = quick_xml::Writer::new(Vec::new());
    let mut in_author = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                in_author = e.name().as_ref() == b"author";
                writer
                    .write_event(Event::Start(e))
                    .map_err(|e| XlsxEditError::InvalidPart {
                        part: part.to_string(),
                        detail: format!("xml write: {e}"),
                    })?;
            }
            Ok(Event::Text(t)) if in_author => {
                // 作者名 → 空文本（保持事件流形状——空元素合法）。
                let _ = t;
                writer
                    .write_event(Event::Text(quick_xml::events::BytesText::new("")))
                    .map_err(|e| XlsxEditError::InvalidPart {
                        part: part.to_string(),
                        detail: format!("xml write: {e}"),
                    })?;
            }
            Ok(Event::Eof) => break,
            Ok(ev) => {
                if matches!(ev, Event::End(_)) {
                    in_author = false;
                }
                writer
                    .write_event(ev)
                    .map_err(|e| XlsxEditError::InvalidPart {
                        part: part.to_string(),
                        detail: format!("xml write: {e}"),
                    })?;
            }
            Err(e) => {
                return Err(XlsxEditError::InvalidPart {
                    part: part.to_string(),
                    detail: format!("xml parse: {e}"),
                });
            }
        }
    }
    String::from_utf8(writer.into_inner()).map_err(|e| XlsxEditError::InvalidPart {
        part: part.to_string(),
        detail: format!("utf8: {e}"),
    })
}
