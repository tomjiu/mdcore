//! B5 故障注入：坏输入面（#49 工作线 B）。
//!
//! 会话级坏输入：load 不存在的文件 / 非文档垃圾字节——必须明确错误
//! （可操作信息），不挂死、不崩溃、不静默成功。帧级坏输入（非 JSON/
//! 超限帧）已在 tests/protocol.rs 恒跑覆盖；kill/超时/拒写面在 B1
//! smoke + B3 lifecycle + B4 reliability 覆盖。环境门：缺运行时 SKIP。

use mdc_lok::client::DocSession;
use std::path::PathBuf;

fn runtime_available() -> bool {
    mdc_lok::runtime::discover().is_ok()
}

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mdc-lok-b5-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn worker_bin_env() {
    if let Some(w) = option_env!("CARGO_BIN_EXE_mdc-lok-worker") {
        std::env::set_var("MDC_LOK_WORKER_BIN", w);
    }
}

/// load 不存在的文件：明确失败（LO getError 或加载错误码），worker
/// 存活、会话可明确报死。
#[test]
fn load_missing_file_clear_error() {
    worker_bin_env();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("missing");
    let missing = dir.join("不存在文档.docx");
    let url = format!(
        "file://{}",
        std::fs::canonicalize(&dir)
            .unwrap()
            .join("不存在文档.docx")
            .to_string_lossy()
    );
    let _ = missing;

    let r = DocSession::open(None, &dir.join("s"), &url, None);
    let err = match r {
        Ok(s) => {
            s.shutdown();
            panic!("load 不存在的文件必须失败（url={url}）")
        }
        Err(e) => e,
    };
    let msg = format!("{err}");
    assert!(!msg.is_empty(), "错误信息非空");
    eprintln!("缺文件错误（可操作样本）：{msg}");
}

/// load 垃圾字节（非文档）：明确失败，不静默成功。
#[test]
fn load_garbage_bytes_clear_error() {
    worker_bin_env();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("garbage");
    let junk = dir.join("garbage.docx");
    // PK 头但结构损坏（模拟半下载/截断产物——比纯随机字节更毒）
    let mut bytes = b"PK\x03\x04".to_vec();
    bytes.extend_from_slice(&[0xA5; 512]);
    std::fs::write(&junk, &bytes).unwrap();
    let url = format!(
        "file://{}",
        std::fs::canonicalize(&junk).unwrap().to_string_lossy()
    );

    let r = DocSession::open(None, &dir.join("s"), &url, None);
    let err = match r {
        Ok(s) => {
            s.shutdown();
            panic!("load 垃圾字节必须失败（不能静默成功开空文档）")
        }
        Err(e) => e,
    };
    let msg = format!("{err}");
    assert!(!msg.is_empty());
    eprintln!("垃圾文档错误（可操作样本）：{msg}");
}
