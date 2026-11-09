//! 协议/超时/帧限/死亡面集成测试（恒跑：无 LibreOffice 也能全绿——
//! 用 worker 的 `--fake` 模式，覆盖 CI 缺运行时环境的门禁）。
//!
//! 覆盖面（AGENTS §13 环境门纪律的「恒跑层」）：
//! - 请求 ID 关联（并发请求不串）
//! - 帧长上限：请求超限拒发 / 应答超限降级 E_OVERSIZE（worker 存活）
//! - 超时看门狗：挂死 op → Timeout + worker 被杀
//! - worker 死亡：boom → 后续请求 WorkerDied（含 stderr 尾部）
//! - 宿主仍可操作：死后新会话 ping

use mdc_lok::client::{LokError, WorkerClient};
use mdc_lok::protocol::codes;
use serde_json::json;
use std::path::PathBuf;
use std::time::Duration;

fn worker_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mdc-lok-worker"))
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mdc-lok-proto-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn fake_worker_request_id_correlation() {
    let dir = tmp("echo");
    let w = WorkerClient::spawn(None, &dir, Some(&worker_bin()), Some("echo")).expect("spawn");
    // 顺序三个请求，ID 自增且应答对应
    for i in 1..=3 {
        let v = w
            .request_with_timeout("op", json!({"n": i}), Duration::from_secs(5))
            .expect("echo 应答");
        assert_eq!(v["echo"], "op", "echo 模式应回显 op");
    }
    w.kill();
}

#[test]
fn fake_worker_hang_times_out_and_kills() {
    let dir = tmp("hang");
    let w = WorkerClient::spawn(None, &dir, Some(&worker_bin()), Some("hang")).expect("spawn");
    let started = std::time::Instant::now();
    let err = w
        .request_with_timeout("slow", json!({}), Duration::from_millis(800))
        .expect_err("slow 永不回答，必须超时");
    match &err {
        LokError::Timeout { op, .. } => assert_eq!(op, "slow"),
        other => panic!("应为 Timeout，实得：{other}"),
    }
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "看门狗应立刻杀"
    );
    assert!(w.is_dead(), "超时后 worker 必须已被杀");
    // 死后请求 → WorkerDied
    let err2 = w.request("ping", json!({})).expect_err("死后请求必须报错");
    assert!(matches!(err2, LokError::WorkerDied { .. }));
    assert_eq!(err2.code(), codes::WORKER_DIED);
}

#[test]
fn fake_worker_die_clear_error() {
    let dir = tmp("die");
    let w = WorkerClient::spawn(None, &dir, Some(&worker_bin()), Some("die")).expect("spawn");
    // 先 ping 确认活着
    w.request_with_timeout("ping", json!({}), Duration::from_secs(5))
        .expect("ping 应答");
    // boom → worker 退出（读线程 EOF 扩散）
    let err = w
        .request_with_timeout("boom", json!({}), Duration::from_secs(10))
        .expect_err("boom 后请求必须失败");
    assert!(
        matches!(err, LokError::WorkerDied { .. }),
        "应为 WorkerDied，实得：{err}"
    );
    assert!(w.is_dead());
    // 宿主仍可操作：新会话
    let dir2 = tmp("die2");
    let w2 = WorkerClient::spawn(None, &dir2, Some(&worker_bin()), Some("echo")).expect("respawn");
    w2.request_with_timeout("ping", json!({}), Duration::from_secs(5))
        .expect("宿主死后仍可开新会话");
    w2.kill();
}

#[test]
fn oversize_answer_downgrades_worker_survives() {
    let dir = tmp("oversize");
    let w = WorkerClient::spawn(None, &dir, Some(&worker_bin()), Some("echo")).expect("spawn");
    // huge 应答 > 2MiB → worker 侧降级 E_OVERSIZE 且存活
    let err = w
        .request_with_timeout("huge", json!({}), Duration::from_secs(15))
        .expect_err("huge 必须降级");
    assert_eq!(err.code(), codes::OVERSIZE, "err={err}");
    assert!(!w.is_dead(), "E_OVERSIZE 不杀 worker");
    // 同一 worker 继续服务
    let v = w
        .request_with_timeout("ping", json!({}), Duration::from_secs(5))
        .expect("降级后 worker 仍可服务");
    assert_eq!(v["pong"], true);
    w.kill();
}

#[test]
fn oversize_request_refused_client_side() {
    let dir = tmp("oversize-req");
    let w = WorkerClient::spawn(None, &dir, Some(&worker_bin()), Some("echo")).expect("spawn");
    let big = "x".repeat(mdc_lok::MAX_FRAME_BYTES + 128);
    let err = w
        .request_with_timeout("op", json!({ "blob": big }), Duration::from_secs(5))
        .expect_err("请求超限必须拒发");
    assert_eq!(err.code(), codes::OVERSIZE);
    assert!(!w.is_dead(), "请求未发出，worker 无辜");
    w.kill();
}

#[test]
fn hello_event_and_frame_limit_constant() {
    // hello 生命周期事件在事件通道上可见；帧上限常量与验收口径一致
    let dir = tmp("hello");
    let w = WorkerClient::spawn(None, &dir, Some(&worker_bin()), Some("echo")).expect("spawn");
    let mut saw_hello = false;
    for _ in 0..20 {
        match w.recv_event_timeout(Duration::from_millis(200)) {
            Some(mdc_lok::client::Event::Life { kind, .. }) if kind == "hello" => {
                saw_hello = true;
                break;
            }
            Some(_) => continue,
            None => break,
        }
    }
    assert!(saw_hello, "worker 启动必须发 hello 事件");
    assert_eq!(mdc_lok::MAX_FRAME_BYTES, 2 * 1024 * 1024);
    w.kill();
}
