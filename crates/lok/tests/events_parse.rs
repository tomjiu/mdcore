//! #115 缺口 #11 集成面：真内核事件 payload 的 typed 解析实证。
//!
//! 单测（events.rs）锁解析器对**蓝本形态**的覆盖；本文件用真 LO 25.2.3.2
//! 的实际回调 payload 验证形态假设（版本间可变的外部输入——解析器必须
//! 吃得下真内核吐的串，而非只吃单测夹具）。
//!
//! 链路：spawn 真 worker → load 夹具 → 真实编辑/选区 → `recv_event_timeout`
//! 收 `INVALIDATE_VISIBLE_CURSOR` / `TEXT_SELECTION` / `STATE_CHANGED` →
//! `events::` 解析器吃真 payload。环境门：无 LibreOffice 运行时 SKIP。

use mdc_lok::abi::callbacks;
use mdc_lok::client::WorkerClient;
use mdc_lok::events;
use serde_json::json;
use std::path::PathBuf;
use std::time::Duration;

fn runtime_available() -> bool {
    mdc_lok::runtime::discover().is_ok()
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke-chars-img-table.docx")
}

/// 夹具复制到 tmp（**既有约定**：全部 LO 测试先复制再加载——LO 打开
/// 会写 `.~lock` 锁；worker 硬杀（SIGKILL）后锁残留 → 同路径重开 =
/// **只读模式**，postKeyEvent 回 ok 但编辑静默丢弃（本文件实测踩坑：
/// 0 回调、无 ModifiedStatus——正是复制约定存在的原因）。
/// **第二坑**：spawn() 清空 session_root——work 文件必须在 session_root
/// **之外**（session 一律 `dir.join("s")` 子目录，work 在 `dir` 下）。
fn work_fixture(dir: &std::path::Path) -> PathBuf {
    let work = dir.join("work.docx");
    std::fs::copy(fixture(), &work).expect("复制夹具");
    work
}

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mdc-lok-115ev-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn file_url(p: &std::path::Path) -> String {
    let canon = std::fs::canonicalize(p).unwrap();
    format!("file://{}", canon.to_string_lossy())
}

fn worker_bin_env() {
    if let Some(w) = option_env!("CARGO_BIN_EXE_mdc-lok-worker") {
        std::env::set_var("MDC_LOK_WORKER_BIN", w);
    }
}

/// 在事件流里等一个目标回调（限时），返回其 payload。
fn await_callback(w: &WorkerClient, type_id: i32, timeout: Duration) -> Option<String> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        let remain = deadline.saturating_duration_since(std::time::Instant::now());
        match w.recv_event_timeout(remain.min(Duration::from_millis(300))) {
            Some(mdc_lok::client::Event::Callback {
                type_id: t,
                payload,
                ..
            }) if t == type_id => return Some(payload),
            Some(_) => continue,
            None => continue,
        }
    }
    None
}

/// 真内核三事件面：光标（JSON/裸两形态实测）+ 选区（SelectAll 真链）+
/// 状态（编辑置脏 STATE_CHANGED）。
#[test]
fn real_kernel_events_parse_typed() {
    worker_bin_env();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("real");
    let rt = mdc_lok::runtime::discover().expect("discover（环境门已过）");
    let w = WorkerClient::spawn(Some(&rt), &dir.join("s"), None, None).expect("spawn");
    w.request_with_timeout(
        "load",
        json!({"url": file_url(&work_fixture(&dir))}),
        Duration::from_secs(30),
    )
    .expect("load 夹具");
    // init_rendering 空 args（DocSession::open 同链——postKeyEvent 需要
    // 渲染初始化；实测无此步键入被内核丢弃，UNO 命令不受影响）。
    w.request_with_timeout("init_rendering", json!({}), Duration::from_secs(15))
        .expect("init_rendering");

    // ① 光标：load 后内核自发 INVALIDATE_VISIBLE_CURSOR（真 payload）。
    // 实测形态（25.2.3.2）：裸串 `"1418, 1418, 0, 345"`——插入符是竖线，
    // **w 可为 0**（真内核实测发现；蓝本/单测无此形态）。
    let cursor_payload = await_callback(
        &w,
        callbacks::INVALIDATE_VISIBLE_CURSOR,
        Duration::from_secs(15),
    )
    .expect("load 后应有光标回调（真内核 25.2.3.2 实测行为）");
    let (rect, _anchor) = events::cursor_of(&cursor_payload)
        .unwrap_or_else(|e| panic!("真光标 payload 解析失败（{e:?}）：{cursor_payload:?}"));
    let rect = rect.unwrap_or_else(|| panic!("真光标 payload 无矩形：{cursor_payload:?}"));
    assert!(
        rect.h > 0,
        "插入符有高（宽可为 0——竖线）：{rect:?}（payload={cursor_payload:?}）"
    );
    eprintln!("115ev ✓ 真光标解析：{rect:?} ← {cursor_payload:?}");

    // ② 选区：SelectAll 真链 → TEXT_SELECTION（多矩形/锚点按真 payload 实测）。
    // 实测形态（25.2.3.2）：SelectAll 后内核先发**空 payload**（清除信号），
    // 真矩形串在后续事件——轮询到非空（空也是合法解析输入：空集不 Err）。
    w.request_with_timeout(
        "uno",
        json!({"command": ".uno:SelectAll", "notify": false}),
        Duration::from_secs(10),
    )
    .expect("SelectAll");
    let mut sel_payload = String::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        let remain = deadline.saturating_duration_since(std::time::Instant::now());
        match w.recv_event_timeout(remain.min(Duration::from_millis(300))) {
            Some(mdc_lok::client::Event::Callback {
                type_id: callbacks::TEXT_SELECTION,
                payload,
                ..
            }) if !payload.is_empty() => {
                sel_payload = payload;
                break;
            }
            Some(_) => continue,
            None => continue,
        }
    }
    assert!(
        !sel_payload.is_empty(),
        "SelectAll 后应等到非空选区回调（15s 内只见到空 payload）"
    );
    let (rects, _anchor) = events::selection_of(&sel_payload)
        .unwrap_or_else(|e| panic!("真选区 payload 解析失败（{e:?}）：{sel_payload:?}"));
    assert!(
        !rects.is_empty(),
        "全文档选区至少一矩形（payload={sel_payload:?}）"
    );
    let area: i64 = rects.iter().map(|r| r.w as i64 * r.h as i64).sum();
    assert!(area > 0, "选区面积非零（rects={rects:?}）");
    eprintln!(
        "115ev ✓ 真选区解析：{} 矩形 ← {:?}",
        rects.len(),
        sel_payload
    );

    // ③ 状态：真实键入置脏 → STATE_CHANGED（DocModified）。
    w.request_with_timeout(
        "post_key",
        json!({"char": '增' as i32, "key_code": 0}),
        Duration::from_secs(10),
    )
    .expect("键入置脏");
    let mut got_doc_modified = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        let remain = deadline.saturating_duration_since(std::time::Instant::now());
        match w.recv_event_timeout(remain.min(Duration::from_millis(300))) {
            Some(mdc_lok::client::Event::Callback {
                type_id: callbacks::STATE_CHANGED,
                payload,
                ..
            }) => {
                if let Some((name, value)) = events::state_of(&payload) {
                    // 真事件名 = .uno:ModifiedStatus（25.2.3.2 探针实测；
                    // 非蓝本文档中的 DocModified——裸 ".uno:X=v" 形态）。
                    if name == ".uno:ModifiedStatus" && value == "true" {
                        got_doc_modified = true;
                        eprintln!("115ev ✓ 真状态解析：{name}={value}");
                        break;
                    }
                }
            }
            Some(_) => continue,
            None => continue,
        }
    }
    assert!(
        got_doc_modified,
        "键入后应有 .uno:ModifiedStatus=true 状态回调（可解析形态）"
    );

    w.kill();
}
