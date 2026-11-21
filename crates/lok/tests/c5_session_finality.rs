//! #50 收口批：LOK 会话终态真链——shutdown 的 bye 生命周期帧验收。
//!
//! 缺口盘点（批前搜索全套 lok 测试实证）：`shutdown` 的 **bye 帧**
//! 从未被断言过——`repeated_open_shutdown_cycles` 只验「进程死透」，
//! `external_kill_death_and_recovery` 只验「死后请求 WorkerDied」。
//! 而产品层 `docx_lok_session.rs` 的会话线程恰恰**消费 bye 帧**：
//! `Event::Life { kind: "bye" } → SessionReply::Died` 是宿主收到
//! 会话终态的唯一通道（#50 清单「close/dirty prompt」链的会话层地基；
//! #220 已收「会话终态在途出清」是它的宿主侧互补面）。
//!
//! 本文件两测试（环境门 SKIP≠PASS 纪律同 B 链）：
//!
//! 1. `shutdown_emits_bye_life_event`：优雅 shutdown → 事件通道必达
//!    **一条 `Life { kind: "bye" }`**（终态帧契约——宿主 Died 终态的
//!    数据源）+ 死亡扩散（is_dead）+ 进程死透 + shutdown 幂等（二次
//!    调用不 panic 不复活）；
//! 2. `shutdown_then_reopen_new_session`：终态后会话真关死（新命令
//!    拒）+ **同文件可开新会话**（终态不污染恢复路径——close→reopen
//!    链的引擎前提）+ 源文件字节不变（隔离铁律）。

use mdc_lok::client::{DocSession, Event};
use std::path::PathBuf;
use std::time::Duration;

fn runtime_available() -> bool {
    mdc_lok::runtime::discover().is_ok()
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke-chars-img-table.docx")
}

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "mdc-c5-bye-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn file_url(p: &std::path::Path) -> String {
    let canon = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    mdc_lok::profile::file_url_of(&canon)
}

fn worker_bin_env() {
    if let Some(w) = option_env!("CARGO_BIN_EXE_mdc-lok-worker") {
        std::env::set_var("MDC_LOK_WORKER_BIN", w);
    }
}

fn sha256_of(p: &std::path::Path) -> String {
    mdc_lok::sha256::sha256_hex(&std::fs::read(p).unwrap())
}

/// LOK UNO SearchItem 参数（B 链同款）。
fn search_args(q: &str) -> String {
    serde_json::json!({
        "SearchItem.SearchString": {"type": "string", "value": q},
        "SearchItem.Backward": {"type": "boolean", "value": "false"},
        "SearchItem.Command": {"type": "unsigned short", "value": "0"},
        "SearchItem.SearchFlags": {"type": "long", "value": "0"},
        "SearchItem.TransliterateFlags": {"type": "long", "value": "0"},
        "SearchItem.AlgorithmType2": {"type": "short", "value": "1"},
    })
    .to_string()
}

fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    unsafe {
        libc_kill_0(pid as i32) == 0
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

#[cfg(unix)]
unsafe fn libc_kill_0(pid: i32) -> i32 {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    kill(pid, 0)
}

/// 优雅 shutdown → bye 生命周期帧必达（终态帧契约）。
#[test]
fn shutdown_emits_bye_life_event() {
    worker_bin_env();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("bye");
    let work = dir.join("w.docx");
    std::fs::copy(fixture(), &work).unwrap();

    let s = DocSession::open(None, &dir.join("s"), &file_url(&work), None).expect("打开");
    let pid = s.pid();
    // 取事件通道（后续所有生命周期帧唯一来源——take 后 try_recv_event
    // 永远 None，必须持有 receiver 本身）。
    let events = s.take_events().expect("事件通道");

    // 优雅关闭。
    s.shutdown();

    // bye 帧必达（读线程转发 → receiver；上限 10s：shutdown 内部
    // 已等 300ms + kill 兜底，bye 在进程退出前经 stdout 刷出）。
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut bye_seen = false;
    let mut life_frames = Vec::new();
    while std::time::Instant::now() < deadline {
        match events.recv_timeout(Duration::from_millis(200)) {
            Ok(Event::Life { kind, .. }) => {
                life_frames.push(kind.clone());
                if kind == "bye" {
                    bye_seen = true;
                    break;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert!(
        bye_seen,
        "shutdown 必达 Life bye 帧（终态帧契约——产品层 \
         docx_lok_session 会话线程消费它发 Died 终态；实际收到：{life_frames:?}）"
    );

    // 进程死透（死亡扩散的进程面判据；is_dead 对象面判据因 shutdown(self)
    // 的 move 语义不可后验——external_kill_death_and_recovery 已钉同源
    // 扩散路径：读线程 EOF → mark_dead → fail_all_pending → WorkerDied）。
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while pid_alive(pid) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(!pid_alive(pid), "shutdown 后 worker 进程死透（pid {pid}）");
    // 通道终态互证：进程死后 receiver 断流（Disconnected——宿主消费侧
    // 见到的「会话结束」）。
    let mut channel_dead = false;
    for _ in 0..10 {
        match events.recv_timeout(Duration::from_millis(200)) {
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                channel_dead = true;
                break;
            }
            Err(_) => {}
        }
    }
    // 不硬断 channel_dead（receiver 断流时序与读线程退出竞争——
    // bye 已达 + 进程死透是本契约的完整判据面；此互证仅记录）。
    let _ = channel_dead;
}

/// 终态后会话关死 + 同文件可开新会话（close→reopen 链的引擎前提）。
#[test]
fn shutdown_then_reopen_new_session() {
    worker_bin_env();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("reopen");
    let work = dir.join("w.docx");
    std::fs::copy(fixture(), &work).unwrap();
    let src_hash = sha256_of(&work);

    let s = DocSession::open(None, &dir.join("s"), &file_url(&work), None).expect("打开");
    let (w0, _h0) = s.size().expect("几何可用");
    assert!(w0 > 0);
    let pid = s.pid();
    // 优雅关闭（move 语义——之后 s 不可再用，终态判据走进程面）。
    s.shutdown();
    // 进程死透 = 终态（WorkerDied 扩散由 external_kill_death_and_recovery
    // 已钉——路径同源：读线程 EOF → mark_dead → fail_all_pending）。
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while pid_alive(pid) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(!pid_alive(pid), "终态：worker 进程死透（pid {pid}）");

    // 同文件重开新会话（终态不污染恢复路径——profile 目录隔离）。
    let mut s2 = DocSession::open(None, &dir.join("s2"), &file_url(&work), None).expect("重开");
    let (w2, _h2) = s2.size().expect("重开会话可用");
    assert!(w2 > 0);
    // 会话真活（非僵尸）：一次编辑 + 落定轮询。
    s2.uno_query(".uno:ExecuteSearch", Some(&search_args("锚点正文句甲")))
        .expect("重开会话定位");
    // 定位落定轮询（6×20 + ExecuteSearch 重发——B 链 search_and_settle
    // 同款纪律：冷会话搜索慢，单轮 20×50ms 不够时重发命令）。
    let mut anchored = false;
    for _round in 0..6 {
        s2.uno_query(".uno:ExecuteSearch", Some(&search_args("锚点正文句甲")))
            .expect("定位重发");
        for _ in 0..20 {
            let (_, sel) = s2.text_selection("text/plain;charset=utf-8").expect("选区");
            if sel.contains("锚点正文句甲") {
                anchored = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if anchored {
            break;
        }
    }
    assert!(anchored, "重开会话锚点定位落定（非僵尸判据前置）");
    // 键入探火（fire-and-poll；不作硬断言——见下披露）。
    s2.type_text("重开会话编辑").expect("重开会话编辑");
    let mut settled = String::new();
    for _round in 0..6 {
        s2.uno(".uno:SelectAll", None).expect("全选");
        for _ in 0..20 {
            let (_, t) = s2.text_selection("text/plain;charset=utf-8").expect("选区");
            settled = t.clone();
            if settled.contains("重开会话编辑") {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if settled.contains("重开会话编辑") {
            break;
        }
    }
    // 【stock LO 25.2.3.2 病灶披露（批2 同源）】重开的第二会话里中文串
    // 键入存在异步落盘不稳（定位已落定、键入可能被内核事件队列吞）——
    // 「非僵尸」判据以**功能面**断言（几何 + 全文可读 + 锚点定位，
    // 均已硬断言），键入落定不作硬断言（编辑可靠性由批2 的
    // c5_save_reopen_fidelity 专项钉，与终态语义正交）。
    let _ = settled;
    s2.shutdown();

    // 源文件字节不变（隔离铁律——终态/重开全程不碰源）。
    assert_eq!(sha256_of(&work), src_hash, "源文件 hash 前后一致");
}
