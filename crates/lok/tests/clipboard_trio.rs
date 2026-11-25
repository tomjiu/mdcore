//! #115 比对缺口 #5：LOK 剪贴板三件套 worker 侧（ely-lok 同构，#49 B 线）。
//!
//! - **clipboard_copy**：`.uno:Copy` + `getTextSelection` 多味读回（plain + html
//!   两味键都在应答——LO 有 html 味才非空，契约面稳定可编程）；
//!   非空选区 → plain 非空。
//! - **clipboard_cut**：`.uno:Cut` + **剪后校验**（比 copy 多一层「真剪掉」
//!   的证据：剪前选区文本留档，剪后读回变化 → `removed: true`）。
//! - **clipboard_paste**：`.uno:Paste` + 前后选区对比；未生效 → `.uno:Undo`
//!   补偿 + 如实报 E_CLIPBOARD（不装成功）。
//!
//! 全选走 `.uno:SelectAll` 惯例（post_key 的 Ctrl+A 键码在 LOK 内部
//! 键码表不可靠——既有测试全用 uno 路径）。
//!
//! 环境门：无 LibreOffice 运行时 SKIP（≠PASS）。

use mdc_lok::client::DocSession;
use mdc_lok::protocol::codes;

use std::path::PathBuf;

fn runtime_available() -> bool {
    mdc_lok::runtime::discover().is_ok()
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke-chars-img-table.docx")
}

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mdc-lok-clip-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn file_url(p: &std::path::Path) -> String {
    let canon = std::fs::canonicalize(p).unwrap();
    mdc_lok::profile::file_url_of(&canon)
}

/// 复制夹具到 tmp（锁坑与 spawn session_root 坑——ely_parity.rs 同款约定）。
fn work_fixture(dir: &std::path::Path) -> PathBuf {
    let work = dir.join("work.docx");
    std::fs::copy(fixture(), &work).expect("复制夹具");
    work
}

/// worker bin 定位（cargo 集成测试惯例——CARGO_BIN_EXE 注环境变量）。
fn worker_bin_env() {
    if let Some(w) = option_env!("CARGO_BIN_EXE_mdc-lok-worker") {
        std::env::set_var("MDC_LOK_WORKER_BIN", w);
    }
}

/// DocSession 开档（load + init_rendering 全链——open 内建）。
fn open_session(dir: &std::path::Path) -> DocSession {
    let rt = mdc_lok::runtime::discover().expect("discover（环境门已过）");
    DocSession::open(
        Some(&rt),
        &dir.join("s"),
        &file_url(&work_fixture(dir)),
        None,
    )
    .expect("open 会话")
}

/// #5 三件套之 copy：多味读回（plain 非空 + html 味键在）。
#[test]
fn clipboard_copy_two_mimes() {
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    worker_bin_env();
    let dir = tmp_dir("copy");
    let mut s = open_session(&dir);

    // 全选（uno 路径惯例）→ copy。
    s.uno(".uno:SelectAll", None).expect("全选");
    let resp = s.clipboard_copy().expect("clipboard_copy");
    let plain = resp.get("plain").and_then(|v| v.as_str()).unwrap_or("");
    assert!(!plain.is_empty(), "非空选区 copy 的 plain 味非空：{resp}");
    assert!(
        resp.get("html").is_some(),
        "应答带 html 味键（多味契约）：{resp}"
    );
}

/// #5 三件套之 cut：剪后校验（removed=true = 选区内容已移除证据）。
#[test]
fn clipboard_cut_verifies_removal() {
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    worker_bin_env();
    let dir = tmp_dir("cut");
    let mut s = open_session(&dir);

    s.uno(".uno:SelectAll", None).expect("全选");
    let resp = s.clipboard_cut().expect("clipboard_cut");
    assert!(
        resp.get("removed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        "cut 应报 removed=true（选区内容已移除校验）：{resp}"
    );
}

/// #5 三件套之 paste：无剪贴板内容 → 静默失败面 → Undo 补偿 + 如实报错。
#[test]
fn clipboard_paste_failure_compensates_with_undo() {
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("paste");
    let mut s = open_session(&dir);

    // 无剪贴板内容的 paste（新 worker 会话剪贴板空）。
    let err = s
        .clipboard_paste()
        .expect_err("无剪贴板内容 paste 应报错（不装成功）");
    assert_eq!(err.code(), codes::CLIPBOARD, "paste 失败码：{err}");
    assert!(!s.is_dead(), "补偿后 worker 存活");
}
