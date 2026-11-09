//! PDF /Info + XMP 暗标元数据清理（#505 / #161 P0-2）。
//!
//! ## 定位
//! DOCX [`crate::docx` 面的 `anonymize_docx_props`] / XLSX
//! `anonymize_xlsx_props`（office-io）的 PDF 姊妹件：导入 PDF → 批注
//! → 增量保存链上，trailer `/Info`（/Author /Title /Subject /
//! /Keywords /Creator /Producer）与 catalog `/Metadata` XMP 包
//! （dc:creator 等）原字节透传——投标暗标场景的身份泄露面。
//!
//! ## 机制（增量修订——与标注写器同基座）
//!
//! lopdf [`IncrementalDocument`] **同 id 克隆原位变更**（标注写器
//! `opt_clone_object_to_new_document` 同机制）：`/Info` 对象克隆后
//! 剥身份键；catalog 克隆后删 `/Metadata` 条目（XMP 整面下线——
//! Acrobat 读元数据 XMP 优先，只清 /Info 是半修）。产物 = 输入字节
//! 前缀 + 追加修订（前缀逐位保持是核心断言）。
//!
//! ## 写前门（fail-closed，同族纪律）
//!
//! - **S5 加密态拒**（`CommandError::SecurityDeny` /
//!   [`crate::security::REASON_ENCRYPTED_WRITE`]——标注写面同款）；
//! - **DocMDP 任何 P 级拒**（[`crate::docmdp::WriteOp::ContentRewrite`]
//!   复用——元数据改写不在 P=1/2/3 任何许可面；且暗标纪律 =
//!   **先匿名后签**——已签档不收）；
//! - **xref 链守卫**（重建装载档拒——增量语义要求 /Prev 可回指；
//!   2026-10-31 行尾污染事故防线同源）；
//! - **S3-L7 输入限额**（open 家族入口统一纪律）。
//!
//! ## 幂等与边界
//!
//! - 无 `/Info` 且无 `/Metadata` → 输入原样返回（零追加字节——
//!   幂等透传）；
//! - `/Info` 身份键六件（Author/Title/Subject/Keywords/Creator/
//!   Producer）删除；/CreationDate /ModDate /Trapped 等非身份键保留；
//! - 产物重载自检（页树可达——`writer_lopdf` 同纪律）。

use lopdf::{Document, IncrementalDocument};

use crate::command::CommandError;
use crate::docmdp::{self, WriteOp};
use crate::security::{Limits, REASON_ENCRYPTED_WRITE};

/// `/Info` 身份键集（剥除面——键名与 PDF spec Table 317 一致；
/// 大小写敏感 = trailer 字典键原样）。
pub const INFO_IDENTITY_KEYS: [&str; 6] = [
    "Author", "Title", "Subject", "Keywords", "Creator", "Producer",
];

/// PDF /Info + XMP 暗标元数据清理（#505）。
///
/// 输入字节 → 清理后字节（增量修订：前缀逐位保持）。守卫与幂等
/// 语义见模块文档。与标注写面互不依赖（独立函数非写器能力——
/// 标注修订与匿名化修订可在同一产物上先后共存，验收 ⑥）。
pub fn anonymize_pdf_info(pdf: &[u8]) -> Result<Vec<u8>, CommandError> {
    // S3-L7 输入限额（open 家族入口统一——与
    // LopdfWriter::from_bytes_with_limits 同纪律）。
    let limits = Limits::default();
    limits.check_open_input_bytes(pdf.len() as u64)?;
    let doc = Document::load_mem(pdf)
        .map_err(|e| CommandError::engine(format!("lopdf 装载失败：{e}")))?;
    // S5 加密态（仍加密或曾加密——标注写面同款双判）。
    if doc.is_encrypted() || doc.was_encrypted() {
        return Err(CommandError::SecurityDeny {
            reason: REASON_ENCRYPTED_WRITE,
        });
    }
    // xref 链守卫（#573 扩两档——判定与 writer_lopdf/forms 共享单点）：
    // 装载期有修复动作的档（扫描重建 / 偏移修正）无 /Prev 可信回指面
    // ——增量修订静默产毒面，入口响亮拒绝。
    if let Some(kind) = crate::writer_lopdf::xref_chain_untrusted_reason(&doc) {
        return Err(CommandError::engine(format!(
            "xref 链不可信（{kind}）——增量修订要求可信前序链"
        )));
    }
    // DocMDP 写前门：元数据改写 = ContentRewrite 类（任何 P 级拒——
    // 元数据不在 P=1/2/3 任何许可面；暗标纪律 = 先匿名后签）。
    docmdp::ensure_write_allowed(&doc, WriteOp::ContentRewrite).map_err(|v| {
        CommandError::SecurityDeny {
            reason: v.reason_code(),
        }
    })?;

    // 目标定位：/Info（trailer 引用）+ catalog /Metadata（XMP）。
    let info_id = doc
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|o| o.as_reference().ok());
    let catalog_id = doc
        .trailer
        .get(b"Root")
        .ok()
        .and_then(|o| o.as_reference().ok())
        .ok_or_else(|| CommandError::engine("trailer 无 /Root（catalog 引用缺失）"))?;
    let has_metadata = doc
        .get_object(catalog_id)
        .ok()
        .and_then(|o| o.as_dict().ok())
        .map(|c| c.get(b"Metadata").is_ok())
        .unwrap_or(false);

    // 幂等透传：两面都无 → 零追加（干净档原样返回）。
    if info_id.is_none() && !has_metadata {
        return Ok(pdf.to_vec());
    }

    // 增量修订：同 id 克隆原位变更。
    let mut inc = IncrementalDocument::create_from(pdf.to_vec(), doc);
    if let Some(id) = info_id {
        inc.opt_clone_object_to_new_document(id)
            .map_err(|e| CommandError::engine(format!("/Info 对象克隆失败：{e}")))?;
        let info = inc
            .new_document
            .get_object_mut(id)
            .map_err(|e| CommandError::engine(format!("/Info 对象读取失败：{e}")))?;
        let dict = info
            .as_dict_mut()
            .map_err(|_| CommandError::engine("/Info 非字典"))?;
        for key in INFO_IDENTITY_KEYS {
            let _ = dict.remove(key.as_bytes());
        }
    }
    if has_metadata {
        inc.opt_clone_object_to_new_document(catalog_id)
            .map_err(|e| CommandError::engine(format!("catalog 克隆失败：{e}")))?;
        let catalog = inc
            .new_document
            .get_object_mut(catalog_id)
            .map_err(|e| CommandError::engine(format!("catalog 读取失败：{e}")))?;
        let dict = catalog
            .as_dict_mut()
            .map_err(|_| CommandError::engine("catalog 非字典"))?;
        // XMP 整面下线（Acrobat 回落读 /Info——已被上方匿名化）。
        let _ = dict.remove(b"Metadata");
    }

    let mut out = Vec::new();
    inc.save_to(&mut out)
        .map_err(|e| CommandError::engine(format!("lopdf 增量保存失败：{e}")))?;
    // 前缀逐位保持断言（增量语义核心——标注写器同纪律）。
    let prefix_ok = out.len() >= pdf.len() && &out[..pdf.len()] == pdf;
    if !prefix_ok {
        return Err(CommandError::engine(
            "增量前缀违约：产物未逐位保持输入字节前缀",
        ));
    }
    // 产物重载自检（页树可达——「写入成功但阅读器看不到」防线）。
    let reloaded =
        Document::load_mem(&out).map_err(|e| CommandError::engine(format!("产物重载失败：{e}")))?;
    if reloaded.get_pages().is_empty() {
        return Err(CommandError::engine(
            "产物自检失败：重载后页树为空（xref 链断裂）",
        ));
    }
    Ok(out)
}
