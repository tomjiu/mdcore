//! dark_scan —— 暗标导出后扫描器 wasm 出口（#502 / #161 P0-2）。
//!
//! 自由函数 [`dark_scan`]（t16 `docx_import::import_docx` / t17
//! `xlsx_import::import_xlsx` 先例——不碰 WasmCore 状态，类面零
//! 变化）：任意产物字节 + 文档种类 → 身份残留 JSON 数组（空数组 =
//! 干净）。错误面 typed JSON `{"code":"dark_scan","message":"..."}`
//! （非 zip 容器拒扫——fail-closed）。
//!
//! 分层（t16/t17 同构）：纯内核 [`dark_scan_core`]（无 JsValue——
//! 原生 cargo test 直测面）+ wasm 薄壳 [`dark_scan`]（JsValue 仅此层）。
//!
//! ## 宿主消费面
//! 暗标导出链收口：导出（`docx_export.anonymize` /
//! `export_with_edits_anonymized`）→ **扫描验证**（本函数）→ 残留
//! 非空即告警/阻断。扫描对任意来源文件开放（外部工具产物、旧版本
//! 产物、手工处理产物——验证面不预设产物出自本引擎）。

use wasm_bindgen::prelude::*;

use mdc_office_io::dark_scan::{
    scan_identity_residuals_docx, scan_identity_residuals_xlsx, IdentityResidual,
};

/// 桥错误（typed 码——宿主可 JSON.parse；纯内核面）。
#[derive(Debug)]
pub struct DarkScanBridgeError {
    pub code: &'static str,
    pub message: String,
}

/// 残留 → JSON（字段面与 `IdentityResidual` 1:1——手工构造，serde
/// derive 不入 office-io 依赖面（canvas `object_json` 先例））。
fn residual_json(r: &IdentityResidual) -> String {
    format!(
        "{{\"part\":{},\"field\":{},\"value\":{},\"hint\":{}}}",
        serde_json::to_string(&r.part).unwrap(),
        serde_json::to_string(&r.field).unwrap(),
        serde_json::to_string(&r.value).unwrap(),
        serde_json::to_string(r.hint).unwrap(),
    )
}

/// 扫描内核（字节 + kind → 残留 JSON 数组串；无 JsValue——原生
/// cargo test 直测面）。
pub fn dark_scan_core(bytes: &[u8], kind: &str) -> Result<String, DarkScanBridgeError> {
    let err = |message: String| DarkScanBridgeError {
        code: "dark_scan",
        message,
    };
    let residuals = match kind {
        "docx" => scan_identity_residuals_docx(bytes),
        "xlsx" => scan_identity_residuals_xlsx(bytes),
        other => return Err(err(format!("unknown kind `{other}` (docx|xlsx)"))),
    }
    .map_err(|e| err(format!("{e}")))?;
    let items: Vec<String> = residuals.iter().map(residual_json).collect();
    Ok(format!("[{}]", items.join(",")))
}

/// 暗标身份残留扫描（kind = `"docx"` | `"xlsx"`）。
///
/// 返回 JSON 数组：`[{part,field,value,hint},…]`（确定性序 part/
/// field/value 字典序）；**空数组 = 干净**。非 zip 容器 / 坏 kind →
/// Err typed JSON（`dark_scan` 码）。
#[wasm_bindgen]
pub fn dark_scan(bytes: &[u8], kind: &str) -> Result<String, JsValue> {
    dark_scan_core(bytes, kind).map_err(|e| {
        JsValue::from_str(&serde_json::json!({ "code": e.code, "message": e.message }).to_string())
    })
}
