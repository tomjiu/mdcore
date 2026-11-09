//! 协议/边界集成测试（#633）——无 LibreOffice 运行时即可跑。
//!
//! 全部用**真实 worker 子进程**（mock 后端）：帧协议往返、请求 ID
//! 关联、kill/超时/拒写故障注入、脏位与源文件保全语义。这些门证明
//! 「协议与边界」，不证明渲染/编辑真实行为（那在 tests/lok_real.rs
//! 的环境门后）——SKIP/PASS 边界诚实（AGENTS §13.3）。

use std::path::{Path, PathBuf};
use std::time::Duration;

use mdc_lok::client::LokRuntimeConfig;
use mdc_lok::error::ErrorCode;
use mdc_lok::ffi::lok_enums;
use mdc_lok::session::DocumentSession;
use mdc_lok::verify::VerifySpec;

fn worker_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mdc-lok-worker"))
}

fn cfg() -> LokRuntimeConfig {
    LokRuntimeConfig {
        worker_bin: Some(worker_bin()),
        ..Default::default()
    }
}

fn cfg_hang(method: &str) -> LokRuntimeConfig {
    LokRuntimeConfig {
        worker_bin: Some(worker_bin()),
        mock_hang_method: Some(method.to_string()),
        ..Default::default()
    }
}

/// 测试源文件：夹具的独立副本（绝不碰仓内夹具本体）。
fn fixture_copy(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mdc-lok-bnd-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dst = dir.join("source.docx");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lok_smoke_source.docx"),
        &dst,
    )
    .unwrap();
    dst
}

#[test]
fn mock_hello_reports_runtime_meta() {
    let mut w = mdc_lok::client::LokWorker::spawn_mock(&cfg()).unwrap();
    let hello = w.hello().unwrap();
    assert_eq!(
        hello.pointer("/meta/mock").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        hello.pointer("/meta/proto").and_then(|v| v.as_str()),
        Some("MDCLK1")
    );
    w.shutdown(Duration::from_secs(5)).unwrap();
}

#[test]
fn mock_chain_paint_edit_event_dirty() {
    let src = fixture_copy("chain");
    let mut s = DocumentSession::open(&cfg(), &src, true).unwrap();
    let (w, h) = s.document_size().unwrap();
    assert!(w > 0 && h > 0, "mock 文档应有尺寸");

    let tile = s.paint_tile(64, 48, 0, 0, 11906, 16838).unwrap();
    assert_eq!((tile.width, tile.height), (64, 48));
    assert!(tile.nonwhite_pixels() > 30, "mock tile 应非空白");

    // 双击第二行（y=350 twips）选中 REPLACEME 词行 → 键入中文替换。
    s.mouse_click(200, 350, 2).unwrap();
    s.type_text("换好了").unwrap();
    assert!(s.is_dirty(), "编辑后脏位应为真");

    let events = s.drain_events();
    assert!(
        events
            .iter()
            .any(|e| e.kind == lok_enums::CALLBACK_INVALIDATE_TILES),
        "编辑后应收到失效事件（mock 注入，回调号与真实内核一致）：{events:?}"
    );
    let before = std::fs::read(&src).unwrap();
    s.close().unwrap();
    // 源文件在会话全程只读：
    assert_eq!(before, std::fs::read(&src).unwrap(), "源文件字节不得变化");
}

#[test]
fn mock_save_verify_commit_replaces_copy() {
    let src = fixture_copy("commit");
    let mut s = DocumentSession::open(&cfg(), &src, true).unwrap();
    s.mouse_click(200, 350, 2).unwrap();
    s.type_text("新词").unwrap();

    let receipt = s
        .save_edits_temp(
            &VerifySpec::new()
                .must_contain("新词")
                .must_not_contain("REPLACEME")
                .must_have_entry("word/document.xml"),
        )
        .unwrap();
    assert!(receipt.report.entry_count >= 2);
    assert!(s.is_dirty(), "保存到临时产物后脏位仍为真（未 commit）");

    s.commit_replace(&receipt).unwrap();
    assert!(!s.is_dirty(), "决定替换成功后才清脏位");
    let saved = std::fs::read(&src).unwrap();
    let saved_text = String::from_utf8_lossy(&saved);
    assert!(
        saved_text.contains("新词"),
        "副本内容应已替换为 mock 保存产物"
    );
    s.close().unwrap();
}

#[test]
fn worker_kill_keeps_dirty_source_and_host_recovers() {
    let src = fixture_copy("kill");
    let before = std::fs::read(&src).unwrap();
    let mut s = DocumentSession::open(&cfg(), &src, true).unwrap();
    s.mouse_click(200, 350, 2).unwrap();
    s.type_text("编辑中").unwrap();

    // 故障注入：杀 worker。
    s.kill_worker();
    assert!(!s.worker_alive());

    // 后续请求 → WorkerDied 明确错误；脏位保留。
    let err = s.paint_tile(32, 32, 0, 0, 1000, 1000).unwrap_err();
    assert_eq!(
        err.code,
        ErrorCode::WorkerDied,
        "错误应带 worker 死亡码：{err}"
    );
    assert!(s.is_dirty(), "worker 死亡不得清脏位");

    // 宿主仍可操作：重开新 worker 会话（源文件只读）。
    let mut s2 = DocumentSession::open(&cfg(), &src, true).unwrap();
    let tile = s2.paint_tile(32, 32, 0, 0, 1000, 1000).unwrap();
    assert!(tile.nonwhite_pixels() > 10);
    s2.close().unwrap();

    assert_eq!(
        before,
        std::fs::read(&src).unwrap(),
        "kill 全程源文件字节不变"
    );
}

#[test]
fn request_timeout_keeps_dirty_and_allows_recovery() {
    // 注入：worker 在 save_as 上永久睡眠（per-worker 配置，不污染并行测试）。
    let src = fixture_copy("timeout");
    let mut s = DocumentSession::open(&cfg_hang("save_as"), &src, true).unwrap();
    s.mouse_click(200, 350, 2).unwrap();
    s.type_text("等待超时").unwrap();

    // 直接以 2s 截止时间调 save_as（会话档 180s 不适合本测试）。
    let url = mdc_lok::ffi::path_to_file_url(&std::env::temp_dir().join("hang-target.docx"));
    let err = s
        .call_worker(
            "save_as",
            &serde_json::json!({"url": url, "format": "docx", "options": ""}),
            Duration::from_secs(2),
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::RequestTimeout, "应报请求超时：{err}");
    assert!(s.is_dirty(), "超时不得清脏位");

    // 恢复：杀挂死的 worker + 新会话可继续工作。
    s.kill_worker();
    let mut s2 = DocumentSession::open(&cfg(), &src, true).unwrap();
    s2.paint_tile(32, 32, 0, 0, 1000, 1000).unwrap();
    s2.close().unwrap();
}

#[test]
fn save_refusal_clear_error_keeps_dirty() {
    // 只读目录作为保存目标（POSIX 0555）。
    let ro = std::env::temp_dir().join(format!("mdc-lok-ro-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ro);
    std::fs::create_dir_all(&ro).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
    }
    let src = fixture_copy("refuse");
    let mut s = DocumentSession::open(&cfg(), &src, true).unwrap();
    s.mouse_click(200, 350, 2).unwrap();
    s.type_text("拒写场景").unwrap();

    let url = mdc_lok::ffi::path_to_file_url(&ro.join("out.docx"));
    let err = s
        .call_worker(
            "save_as",
            &serde_json::json!({"url": url, "format": "docx", "options": ""}),
            Duration::from_secs(10),
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::SaveFailed, "拒写应报保存失败：{err}");
    assert!(s.is_dirty(), "保存失败不得清脏位");
    s.close().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let _ = std::fs::remove_dir_all(&ro);
}

#[test]
fn save_as_to_loaded_source_is_rejected_by_worker() {
    let src = fixture_copy("guard");
    let mut s = DocumentSession::open(&cfg(), &src, true).unwrap();
    let url = mdc_lok::ffi::path_to_file_url(&src);
    let err = s
        .call_worker(
            "save_as",
            &serde_json::json!({"url": url, "format": "docx", "options": ""}),
            Duration::from_secs(10),
        )
        .unwrap_err();
    assert_eq!(
        err.code,
        ErrorCode::InvalidRequest,
        "worker 必须拒绝直接覆盖源文件：{err}"
    );
    s.close().unwrap();
}

#[test]
fn oversized_canvas_rejected() {
    let src = fixture_copy("bigcanvas");
    let mut s = DocumentSession::open(&cfg(), &src, true).unwrap();
    let err = s.paint_tile(9000, 9000, 0, 0, 1000, 1000).unwrap_err();
    assert_eq!(
        err.code,
        ErrorCode::InvalidRequest,
        "canvas 越界应明确拒绝：{err}"
    );
    // 帧上限内的正常请求不受影响：
    s.paint_tile(64, 64, 0, 0, 1000, 1000).unwrap();
    s.close().unwrap();
}
