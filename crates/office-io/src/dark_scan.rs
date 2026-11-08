//! 暗标导出后扫描器（#502 / #161 P0-2）——docx/xlsx 身份残留结构化枚举。
//!
//! ## 定位
//! 清理面（DOCX [`crate::docx::anonymize_docx_props`] / XLSX
//! `anonymize_xlsx_props`（PR #501））的**独立验证面**：对任意产物
//! 文件（不限于本引擎产物——外部工具/旧版本/手工处理均可）枚举
//! 身份残留，回答「这份产物真的清干净了吗」。
//!
//! 清理与扫描**互不依赖**（扫描独立解 zip/解析——测试 fixture 手工
//! 构造，不 import 清理产物）；两面上线后即形成「清理 → 扫描」
//! 闭环（暗标导出的验收链）。
//!
//! ## 残留判定面（与清理变换面对称）
//!
//! | 部件 | 字段 | 残留判据 |
//! |---|---|---|
//! | `docProps/core.xml` | `dc:creator` / `cp:lastModifiedBy` | 非 `unknown`（清理面匿名常量） |
//! | `docProps/app.xml` | `Company` / `Manager` | 非空 |
//! | `docProps/custom.xml` | 任何 `Property` | 存在即残留（自定义属性常载身份） |
//! | `word/comments.xml`（DOCX） | `w:comment/@w:author` / `@w:initials` | 非空 |
//! | `word/people.xml`（DOCX） | `w:person` | 存在即残留 |
//! | `xl/comments*.xml`（XLSX） | `<author>` | 非空文本 |
//!
//! 部件缺失 = 该面零发现（缺 docProps 本身就是干净的）；非 zip
//! 容器 = [`DarkScanError::NotZip`] 拒扫（fail-closed，与清理面同构）。
//! 输出确定性序（part/field/value 字典序）——断言与展示稳定。

use quick_xml::events::Event;
use quick_xml::Reader;

/// 身份残留发现（结构化——wasm/产品可直接 JSON 化展示）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityResidual {
    /// 部件名（zip 根相对路径）。
    pub part: String,
    /// 字段（XML 局部名/属性名——跨面稳定标识）。
    pub field: String,
    /// 残留值（非空；people/person 无值面为空串 + hint 说明）。
    pub value: String,
    /// 位置提示（人读——哪一类身份面）。
    pub hint: &'static str,
}

/// 扫描面错误（fail-closed：容器坏拒扫——不猜）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DarkScanError {
    /// 非 zip 容器（magic/结构坏）。
    NotZip(String),
}

impl std::fmt::Display for DarkScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DarkScanError::NotZip(m) => write!(f, "not a zip container: {m}"),
        }
    }
}

impl std::error::Error for DarkScanError {}

/// 读 zip 部件文本（缺部件 = None——零发现面）。
fn part_text(bytes: &[u8], name: &str) -> Result<Option<String>, DarkScanError> {
    use std::io::Read as _;
    let cursor = std::io::Cursor::new(bytes);
    let mut za =
        zip::ZipArchive::new(cursor).map_err(|e| DarkScanError::NotZip(format!("open: {e}")))?;
    let mut f = match za.by_name(name) {
        Ok(f) => f,
        Err(_) => return Ok(None),
    };
    let mut out = String::new();
    f.read_to_string(&mut out)
        .map_err(|e| DarkScanError::NotZip(format!("read {name}: {e}")))?;
    Ok(Some(out))
}

/// 枚举 zip 部件名（批注部件发现面——`xl/comments*.xml`）。
fn all_parts(bytes: &[u8]) -> Result<Vec<String>, DarkScanError> {
    let cursor = std::io::Cursor::new(bytes);
    let mut za =
        zip::ZipArchive::new(cursor).map_err(|e| DarkScanError::NotZip(format!("open: {e}")))?;
    let mut out = Vec::with_capacity(za.len());
    for i in 0..za.len() {
        let name = za
            .by_index_raw(i)
            .map_err(|e| DarkScanError::NotZip(format!("index: {e}")))?
            .name()
            .to_string();
        out.push(name);
    }
    Ok(out)
}

/// 元素/属性局部名（剥命名空间前缀：`dc:creator` → `creator`；
/// 无前缀原名直返——quick-xml `name()` 返回含前缀全名）。
fn local_name(raw: &[u8]) -> String {
    match raw.iter().position(|&b| b == b':') {
        Some(i) => String::from_utf8_lossy(&raw[i + 1..]).into_owned(),
        None => String::from_utf8_lossy(raw).into_owned(),
    }
}

/// 解析错误统一折叠为「部件文本面异常」（扫描不猜——非 zip 容器
/// 已在门上拒；部件级 XML 坏按 zip 读取面处理，这里只可能遇到
/// 读取 IO 错，归 NotZip 同族）。
fn io_err(part: &str, what: &str) -> DarkScanError {
    DarkScanError::NotZip(format!("{part}: {what}"))
}

/// docProps/core.xml 扫描：creator / lastModifiedBy 非 `unknown` 即残留。
fn scan_core(part: &str, xml: &str, out: &mut Vec<IdentityResidual>) -> Result<(), DarkScanError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut expect: Option<&'static str> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let n = local_name(e.name().as_ref());
                expect = match n.as_str() {
                    "creator" => Some("dc:creator"),
                    "lastModifiedBy" => Some("cp:lastModifiedBy"),
                    _ => None,
                };
            }
            Ok(Event::Text(t)) => {
                if let Some(field) = expect.take() {
                    let v = t.decode().unwrap_or_default().to_string();
                    if !v.is_empty() && v != "unknown" {
                        out.push(IdentityResidual {
                            part: part.to_string(),
                            field: field.to_string(),
                            value: v,
                            hint: "元数据作者身份（应为 unknown）",
                        });
                    }
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(io_err(part, &format!("xml parse: {e}"))),
        }
    }
    Ok(())
}

/// docProps/app.xml 扫描：Company / Manager 非空即残留。
fn scan_app(part: &str, xml: &str, out: &mut Vec<IdentityResidual>) -> Result<(), DarkScanError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut expect: Option<&'static str> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let n = local_name(e.name().as_ref());
                expect = match n.as_str() {
                    "Company" => Some("Company"),
                    "Manager" => Some("Manager"),
                    _ => None,
                };
            }
            Ok(Event::Text(t)) => {
                if let Some(field) = expect.take() {
                    let v = t.decode().unwrap_or_default().to_string();
                    if !v.is_empty() {
                        out.push(IdentityResidual {
                            part: part.to_string(),
                            field: field.to_string(),
                            value: v,
                            hint: "组织/管理者身份（应为空）",
                        });
                    }
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(io_err(part, &format!("xml parse: {e}"))),
        }
    }
    Ok(())
}

/// docProps/custom.xml 扫描：任何 Property 即残留（名 + 值双要素）。
fn scan_custom(
    part: &str,
    xml: &str,
    out: &mut Vec<IdentityResidual>,
) -> Result<(), DarkScanError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    // Property 元素文本（vt:lpstr 等）——Start(Property) 后首个 Text。
    let mut in_property_name: Option<String> = None;
    let mut want_value = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let n = local_name(e.name().as_ref());
                if n == "Property" {
                    // name 属性 = 自定义属性名（无前缀——custom.xml 约定）。
                    let mut pname = String::new();
                    for a in e.attributes().flatten() {
                        if local_name(a.key.as_ref()) == "name" {
                            pname = String::from_utf8_lossy(&a.value).into_owned();
                        }
                    }
                    in_property_name = Some(pname);
                    want_value = false;
                } else if in_property_name.is_some() {
                    want_value = true; // vt:* 值元素文本
                }
            }
            Ok(Event::Text(t)) => {
                if want_value {
                    let v = t.decode().unwrap_or_default().to_string();
                    if let Some(pname) = in_property_name.take() {
                        if !v.is_empty() {
                            out.push(IdentityResidual {
                                part: part.to_string(),
                                field: format!("Property:{pname}"),
                                value: v,
                                hint: "自定义属性（常载身份——应为空骨架）",
                            });
                        }
                    }
                    want_value = false;
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(io_err(part, &format!("xml parse: {e}"))),
        }
    }
    Ok(())
}

/// DOCX 面扫描：docProps 三件套 + word/comments.xml（w:author/w:initials）
/// + word/people.xml（w:person 存在即残留）。
pub fn scan_identity_residuals_docx(bytes: &[u8]) -> Result<Vec<IdentityResidual>, DarkScanError> {
    let mut out = Vec::new();
    if let Some(xml) = part_text(bytes, "docProps/core.xml")? {
        scan_core("docProps/core.xml", &xml, &mut out)?;
    }
    if let Some(xml) = part_text(bytes, "docProps/app.xml")? {
        scan_app("docProps/app.xml", &xml, &mut out)?;
    }
    if let Some(xml) = part_text(bytes, "docProps/custom.xml")? {
        scan_custom("docProps/custom.xml", &xml, &mut out)?;
    }
    if let Some(xml) = part_text(bytes, "word/comments.xml")? {
        scan_docx_comments(&xml, &mut out)?;
    }
    if let Some(xml) = part_text(bytes, "word/people.xml")? {
        scan_docx_people(&xml, &mut out)?;
    }
    sort_residuals(&mut out);
    Ok(out)
}

/// word/comments.xml 扫描：w:comment 的 w:author / w:initials 非空。
fn scan_docx_comments(xml: &str, out: &mut Vec<IdentityResidual>) -> Result<(), DarkScanError> {
    let part = "word/comments.xml";
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                if local_name(e.name().as_ref()) == "comment" {
                    for a in e.attributes().flatten() {
                        let key = local_name(a.key.as_ref());
                        let field = match key.as_str() {
                            "author" => "w:author",
                            "initials" => "w:initials",
                            _ => continue,
                        };
                        let v = String::from_utf8_lossy(&a.value).into_owned();
                        if !v.is_empty() {
                            out.push(IdentityResidual {
                                part: part.to_string(),
                                field: field.to_string(),
                                value: v,
                                hint: "批注归属身份（应为空）",
                            });
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(io_err(part, &format!("xml parse: {e}"))),
        }
    }
    Ok(())
}

/// word/people.xml 扫描：w:person 存在即残留。
fn scan_docx_people(xml: &str, out: &mut Vec<IdentityResidual>) -> Result<(), DarkScanError> {
    let part = "word/people.xml";
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    loop {
        match reader.read_event() {
            // w:person 常为自闭合（<w:person .../>）——Start/Empty 双面。
            Ok(ev @ (Event::Start(_) | Event::Empty(_))) => {
                let e = match ev {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                if local_name(e.name().as_ref()) == "person" {
                    let mut v = String::new();
                    for a in e.attributes().flatten() {
                        if local_name(a.key.as_ref()) == "displayName" {
                            v = String::from_utf8_lossy(&a.value).into_owned();
                        }
                    }
                    out.push(IdentityResidual {
                        part: part.to_string(),
                        field: "w:person".to_string(),
                        value: v,
                        hint: "批注人员名单（应为空骨架）",
                    });
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(io_err(part, &format!("xml parse: {e}"))),
        }
    }
    Ok(())
}

/// XLSX 面扫描：docProps 三件套 + xl/comments*.xml（`<author>` 非空）。
pub fn scan_identity_residuals_xlsx(bytes: &[u8]) -> Result<Vec<IdentityResidual>, DarkScanError> {
    let mut out = Vec::new();
    if let Some(xml) = part_text(bytes, "docProps/core.xml")? {
        scan_core("docProps/core.xml", &xml, &mut out)?;
    }
    if let Some(xml) = part_text(bytes, "docProps/app.xml")? {
        scan_app("docProps/app.xml", &xml, &mut out)?;
    }
    if let Some(xml) = part_text(bytes, "docProps/custom.xml")? {
        scan_custom("docProps/custom.xml", &xml, &mut out)?;
    }
    for name in all_parts(bytes)? {
        if is_xlsx_comments_part(&name) {
            if let Some(xml) = part_text(bytes, &name)? {
                scan_xlsx_comments(&name, &xml, &mut out)?;
            }
        }
    }
    sort_residuals(&mut out);
    Ok(out)
}

/// 批注部件名判定（`xl/comments*.xml`——与清理面 PR #501 同谓词；
/// 两 PR 独立基线，合并后可归一）。
fn is_xlsx_comments_part(name: &str) -> bool {
    name.starts_with("xl/comments") && name.ends_with(".xml")
}

/// xl/comments*.xml 扫描：`<author>` 非空文本即残留。
fn scan_xlsx_comments(
    part: &str,
    xml: &str,
    out: &mut Vec<IdentityResidual>,
) -> Result<(), DarkScanError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut in_author = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                in_author = local_name(e.name().as_ref()) == "author";
            }
            Ok(Event::Text(t)) => {
                if in_author {
                    let v = t.decode().unwrap_or_default().to_string();
                    if !v.is_empty() {
                        out.push(IdentityResidual {
                            part: part.to_string(),
                            field: "author".to_string(),
                            value: v,
                            hint: "批注作者名（应为空）",
                        });
                    }
                }
            }
            Ok(Event::Eof) => break,
            Ok(ev) => {
                if matches!(ev, Event::End(_)) {
                    in_author = false;
                }
            }
            Err(e) => return Err(io_err(part, &format!("xml parse: {e}"))),
        }
    }
    Ok(())
}

/// 确定性序（断言/展示稳定面）。
fn sort_residuals(out: &mut [IdentityResidual]) {
    out.sort_by(|a, b| (&a.part, &a.field, &a.value).cmp(&(&b.part, &b.field, &b.value)));
}
