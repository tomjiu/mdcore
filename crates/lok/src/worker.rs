//! worker 进程主循环（#633）。
//!
//! 进程模型：宿主 spawn 本进程（stdin/stdout 帧协议，stderr 供崩溃
//! 取证）。
//!
//! **真实模式 = unipoll 主循环集成**：调用 hook_2 的本线程直接进入
//! LOK 的 runLoop，宿主请求在 poll 回调里消费——每次请求都在内核
//! 主循环上下文同步执行。这是修复「异步事件黑洞」的关键：非 unipoll
//! 下键鼠/UNO 走 PostUserEvent 入队、依赖 lo_startmain 线程消费；
//! 外部宿主场景实测该线程可整段沉睡（futex），排队事件永不被处理
//! ——编辑静默丢失（gates/633.md 实录）。unipoll 下 postEventAsync
//! 直调 LOKPostAsyncEvent，同步确定（ElyOffice Linux 同款模型
//! worker/unipoll.rs）。宿主侧须在 spawn 时设 `SAL_LOK_OPTIONS=unipoll`
//! （client.rs 已内置；手动运行见 mdc_lok_worker 入口兜底）。
//!
//! mock 模式无内核，走简单分发循环（协议/边界测试面）。
//! 退出恒走 `_exit`（绕开 LO 注册的 C atexit——ElyOffice 同款教训，
//! 避免 teardown 崩溃污染退出码）。

use std::ffi::c_void;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};

use crate::backend::{DocSession, LoadInfo, RealSession};
use crate::error::{ErrorCode, LokError, LokResult};
use crate::ffi::lok_enums;
use crate::mock::MockSession;
use crate::proto::{write_frame, FrameBuf, FrameKind, UNASKED};

/// 单文档状态（worker 侧）；revision 由变更操作自增，随事件/响应
/// 回传（宿主用它丢弃过期 tile，语义参考 ElyOffice generation）。
struct WorkerState {
    session: SessionHandle,
    loaded_url: Option<String>,
    revision: u64,
    hello_meta: Value,
}

enum SessionHandle {
    Real(Box<RealSession>),
    Mock(Box<MockSession>),
}

macro_rules! with_session {
    ($self:expr, |$s:ident| $body:expr) => {
        match &mut $self.session {
            SessionHandle::Real($s) => $body,
            SessionHandle::Mock($s) => $body,
        }
    };
}

impl WorkerState {
    fn bump_revision(&mut self) -> u64 {
        self.revision += 1;
        self.revision
    }
}

/// worker 侧守卫：save_as 目标不得等于当前加载的源 URL（源文件
/// 永不直接覆盖——宿主必须走「临时产物 + 校验 + 决定替换」链）。
fn guard_save_target(target_url: &str, loaded_url: &Option<String>) -> LokResult<()> {
    if let Some(src) = loaded_url {
        if src == target_url {
            return Err(LokError::new(
                ErrorCode::InvalidRequest,
                "save_as 目标与当前加载的源文件相同：原型禁止 worker 直接\
                 覆盖源文件（宿主先存临时产物、校验后再决定替换）",
            ));
        }
    }
    Ok(())
}

/// 把 stdin 从 fd 0 上“偷走”：dup 到新 fd，fd 0 换成空设备。
/// 返回接管协议流的文件句柄（失败时退回普通 stdin——LO 未抢时不影响）。
///
/// 动机：LO 内嵌组件（python 等）可能直接读 fd 0，与协议读线程形成
/// 竞争读者，把帧流撕碎 → 宿主超时但 worker 未死（ElyOffice Windows
/// 同款教训 DuplicateHandle+dup2 NUL；本仓 Linux 实测复现：连续
/// post_key 中途偶发挂起）。
fn steal_stdin_fd() -> std::fs::File {
    use std::os::fd::{FromRawFd, IntoRawFd, RawFd};

    #[cfg(unix)]
    unsafe extern "C" {
        fn dup(fd: RawFd) -> RawFd;
        fn dup2(old: RawFd, new: RawFd) -> RawFd;
        fn close(fd: RawFd) -> RawFd;
    }
    #[cfg(windows)]
    unsafe extern "C" {
        fn _dup(fd: RawFd) -> RawFd;
        fn _dup2(old: RawFd, new: RawFd) -> RawFd;
    }

    let devnull_path = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let ok = (|| -> Option<std::fs::File> {
        unsafe {
            #[cfg(unix)]
            let saved = dup(0);
            #[cfg(windows)]
            let saved = _dup(0);
            if saved < 0 {
                return None;
            }
            // 换 fd 0 前先把 saved 交给 File 管理（防提前返回泄漏）。
            let protocol = std::fs::File::from_raw_fd(saved);
            let nul = std::fs::File::open(devnull_path).ok()?;
            let nul_fd = nul.into_raw_fd();
            #[cfg(unix)]
            let rc = dup2(nul_fd, 0);
            #[cfg(windows)]
            let rc = _dup2(nul_fd, 0);
            // nul_fd 已复制到 0，关闭原句柄（unix dup2 后原 fd 仍开）。
            #[cfg(unix)]
            close(nul_fd);
            if rc < 0 {
                return None;
            }
            Some(protocol)
        }
    })();
    ok.unwrap_or_else(|| {
        // 兜底：dup 失败（极老平台）时用回标准 stdin。
        unsafe { std::fs::File::from_raw_fd(0) }
    })
}

/// 事件/响应写出（stdout 锁：分发线程独占写，读线程只读协议 fd）。
fn emit(frame: FrameBuf) {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    if write_frame(&mut lock, &frame).is_err() {
        // 宿主已退出：静默终止（下一步 recv 会失败退出循环）。
    }
}

fn err_response(id: u64, err: &LokError) -> FrameBuf {
    FrameBuf::from_json_value(
        FrameKind::Response,
        id,
        &json!({
            "ok": false,
            "error": {"code": err.code.as_str(), "message": err.message}
        }),
        Vec::new(),
    )
}

/// worker 主循环。`program_dir` 真实模式必填。
pub fn serve(mock: bool, program_dir: Option<PathBuf>, profile_dir: PathBuf) -> LokResult<()> {
    // ---- fd 0 抢救（必须先于任何 LOK 初始化）----
    let protocol_stdin = steal_stdin_fd();

    // ---- 后端初始化 ----
    let state: WorkerState = if mock {
        WorkerState {
            session: SessionHandle::Mock(Box::default()),
            loaded_url: None,
            revision: 0,
            hello_meta: json!({"mock": true, "profile_dir": profile_dir.to_string_lossy(), "proto": "MDCLK1"}),
        }
    } else {
        let dir = program_dir.ok_or_else(|| {
            LokError::new(ErrorCode::InvalidRequest, "真实模式需要 program 目录参数")
        })?;
        let lib = crate::ffi::pick_library(&dir);
        let real = RealSession::new(&dir, &profile_dir)?;
        let version = real.version_info().ok().flatten();
        let meta = json!({
            "mock": false,
            "program_dir": dir.to_string_lossy(),
            "library": lib.as_ref().ok().map(|(p, _)| p.to_string_lossy()),
            "version_info": version,
            "kit_nsize": real.kit_nsize(),
            "profile_dir": profile_dir.to_string_lossy(),
            "proto": "MDCLK1",
        });
        WorkerState {
            session: SessionHandle::Real(Box::new(real)),
            loaded_url: None,
            revision: 0,
            hello_meta: meta,
        }
    };

    // ---- IO 线程（读 dup 后的协议 fd，不再碰 fd 0）----
    let (tx, rx) = mpsc::channel::<Note>();
    let reader_tx = tx.clone();
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(protocol_stdin);
        loop {
            match crate::proto::read_frame(&mut reader) {
                Ok(Some(f)) => {
                    if reader_tx.send(Note::Ask(f)).is_err() {
                        break;
                    }
                }
                Ok(None) => {
                    let _ = reader_tx.send(Note::Closed);
                    break;
                }
                Err(e) => {
                    eprintln!("mdc-lok-worker: 帧读取失败：{e}");
                    let _ = reader_tx.send(Note::Closed);
                    break;
                }
            }
        }
    });

    let mut ctx = LoopCtx { state, rx };
    if mock {
        // ---- mock：简单分发循环（无内核主循环面）----
        loop {
            match ctx.rx.recv() {
                Ok(Note::Ask(f)) => {
                    if handle_request(&mut ctx, f) {
                        break;
                    }
                }
                Ok(Note::Woken) => {}
                Ok(Note::Closed) | Err(_) => break,
            }
        }
        worker_exit(&mut ctx);
    } else {
        // ---- 真实模式：unipoll——本线程进入内核主循环（runLoop），
        // 请求在 poll 回调里消费 = 主循环上下文同步分发（模块头注释
        // 与 gates/633.md「异步事件黑洞」实录）。----
        let kit = match &ctx.state.session {
            SessionHandle::Real(real) => real.kit_ptr(),
            SessionHandle::Mock(_) => unreachable!("真实路径不该有 mock 会话"),
        };
        let _ = WAKE_TX.set(Mutex::new(tx));
        let ctx_ptr = Box::into_raw(Box::new(ctx));
        // lok_run_loop 为安全封装（内部自行守卫 nSize 与空槽）。
        let loop_r =
            crate::ffi::lok_run_loop(kit, unipoll_poll, unipoll_wake, ctx_ptr.cast::<c_void>());
        if let Err(e) = loop_r {
            eprintln!("mdc-lok-worker: runLoop 异常退出：{e}");
        }
        let mut ctx = unsafe { Box::from_raw(ctx_ptr) };
        worker_exit(&mut ctx);
    }
}

/// 主循环里的一条待办：宿主请求 / 内核唤醒 / 对端关闭。
enum Note {
    Ask(FrameBuf),
    Woken,
    Closed,
}

/// unipoll 上下文（进入 runLoop 后经 data 指针回到这里）。
struct LoopCtx {
    state: WorkerState,
    rx: Receiver<Note>,
}

/// 唤醒通道（内核其他线程 → 主循环 poll）。
static WAKE_TX: std::sync::OnceLock<Mutex<mpsc::Sender<Note>>> = std::sync::OnceLock::new();

/// 处理一条请求帧：分发 → 响应 → 转发事件。返回 true = shutdown。
fn handle_request(ctx: &mut LoopCtx, frame: FrameBuf) -> bool {
    if frame.kind != FrameKind::Request {
        eprintln!("mdc-lok-worker: 忽略非请求帧 kind={:?}", frame.kind);
        return false;
    }
    let id = frame.id;
    let params: Value = serde_json::from_slice(&frame.json).unwrap_or(Value::Null);
    let (resp, bin, exit_after) = dispatch(&mut ctx.state, &params, frame.bin);
    let resp_frame = match resp {
        Ok(mut result) => {
            result
                .as_object_mut()
                .map(|m| m.insert("rev".into(), json!(ctx.state.revision)));
            let obj = match result {
                Value::Object(mut map) => {
                    map.insert("ok".into(), Value::Bool(true));
                    Value::Object(map)
                }
                other => other,
            };
            FrameBuf::from_json_value(FrameKind::Response, id, &obj, bin)
        }
        Err(ref e) => err_response(id, e),
    };
    emit(resp_frame);
    forward_events(&mut ctx.state);
    exit_after
}

/// 收尾：销毁文档 + `_exit`（绕开 LO atexit 崩溃面）。
fn worker_exit(ctx: &mut LoopCtx) -> ! {
    let _ = with_session!(ctx.state, |s| s.destroy());
    extern "C" {
        fn _exit(code: i32) -> !;
    }
    unsafe { _exit(0) }
}

/// unipoll poll 回调：内核空闲时在此等待；请求到达即在本（主）线程
/// 同步分发。timeout 语义（微秒）：0=非阻塞，负=无限等，正=上限。
unsafe extern "C" fn unipoll_poll(data: *mut c_void, timeout_us: i32) -> i32 {
    let ctx = &mut *data.cast::<LoopCtx>();
    let note = match timeout_us {
        0 => match ctx.rx.try_recv() {
            Ok(n) => n,
            Err(_) => return 0,
        },
        t if t < 0 => match ctx.rx.recv() {
            Ok(n) => n,
            Err(_) => worker_exit(ctx),
        },
        t => match ctx
            .rx
            .recv_timeout(Duration::from_micros(t.clamp(0, 10_000_000) as u64))
        {
            Ok(n) => n,
            Err(RecvTimeoutError::Timeout) => return 0,
            Err(RecvTimeoutError::Disconnected) => worker_exit(ctx),
        },
    };
    match note {
        Note::Woken => 1,
        Note::Closed => worker_exit(ctx),
        Note::Ask(frame) => {
            if handle_request(ctx, frame) {
                worker_exit(ctx);
            }
            1
        }
    }
}

/// unipoll wake 回调：内核其他线程要主循环干活时叫醒 poll。
unsafe extern "C" fn unipoll_wake(_data: *mut c_void) {
    if let Some(tx) = WAKE_TX.get() {
        let _ = tx.lock().expect("wake 通道锁中毒").send(Note::Woken);
    }
}

/// 请求分发。返回 (结果 JSON, 二进制尾, 是否退出循环)。
fn dispatch(
    state: &mut WorkerState,
    params: &Value,
    _bin_in: Vec<u8>,
) -> (LokResult<Value>, Vec<u8>, bool) {
    let method = params
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let p = params.get("params").cloned().unwrap_or(Value::Null);
    let result = match method.as_str() {
        "hello" => Ok(json!({
            "meta": hello_meta_of(state),
        })),
        "load" => {
            let url = p
                .get("url")
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_string();
            if url.is_empty() {
                Err(LokError::new(ErrorCode::InvalidRequest, "load 缺 url"))
            } else if state.loaded_url.is_some() {
                Err(LokError::new(
                    ErrorCode::DocumentAlreadyOpen,
                    "已有文档打开（原型单文档：先 close）",
                ))
            } else {
                let outcome = match &mut state.session {
                    SessionHandle::Real(real) => real
                        .load(&url)
                        .map(|info: LoadInfo| serde_json::to_value(info).unwrap_or(Value::Null)),
                    SessionHandle::Mock(mock) => {
                        mock.loaded_url = Some(url.clone());
                        Ok(json!({"doc_type": 0, "parts": 1, "doc_nsize": 0}))
                    }
                };
                outcome.inspect(|_| {
                    state.loaded_url = Some(url);
                })
            }
        }
        "close" => {
            state.loaded_url = None;
            with_session!(state, |s| s.destroy()).map(|_| json!({"closed": true}))
        }
        "doc_size" => {
            let (w, h) = with_session!(state, |s| s.doc_size());
            Ok(json!({"width_twips": w, "height_twips": h}))
        }
        "tile_mode" => Ok(json!({"tile_mode": with_session!(state, |s| s.tile_mode())})),
        "paint_tile" => {
            let cw = p.get("canvas_w").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ch = p.get("canvas_h").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let tx = p.get("tile_x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ty = p.get("tile_y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let tw = p.get("tile_w").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let th = p.get("tile_h").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            if cw <= 0 || ch <= 0 || cw > 8192 || ch > 8192 {
                return (
                    Err(LokError::new(ErrorCode::InvalidRequest, "canvas 尺寸越界")),
                    Vec::new(),
                    false,
                );
            }
            if (cw as usize) * (ch as usize) * 4 > crate::proto::MAX_BIN_LEN as usize {
                return (
                    Err(LokError::new(ErrorCode::BadFrame, "tile 像素超过帧上限")),
                    Vec::new(),
                    false,
                );
            }
            let mut buf = vec![0u8; cw as usize * ch as usize * 4];
            let painted = with_session!(state, |s| s.paint_tile(&mut buf, cw, ch, tx, ty, tw, th));
            match painted {
                Ok(()) => {
                    let mode = with_session!(state, |s| s.tile_mode());
                    let meta = json!({
                        "width": cw, "height": ch,
                        "tile_x": tx, "tile_y": ty, "tile_w": tw, "tile_h": th,
                        "tile_mode": mode,
                    });
                    return (Ok(meta), buf, false);
                }
                Err(e) => return (Err(e), Vec::new(), false),
            }
        }
        "post_key" => {
            let kind = p.get("kind").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let char_code = p.get("char").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let key_code = p.get("keycode").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            with_session!(state, |s| s.post_key(kind, char_code, key_code)).map(|_| {
                state.bump_revision();
                json!({})
            })
        }
        "post_mouse" => {
            let kind = p.get("kind").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let x = p.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let y = p.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let count = p.get("count").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
            let buttons = p.get("buttons").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
            let modifiers = p.get("modifiers").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            with_session!(state, |s| s
                .post_mouse(kind, x, y, count, buttons, modifiers))
            .map(|_| {
                if kind != 2 {
                    // 移动不改版面；按下/抬起视为交互变更。
                    state.bump_revision();
                }
                json!({})
            })
        }
        "post_uno" => {
            let command = p
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let args = p
                .get("args")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let notify = p.get("notify").and_then(|v| v.as_bool()).unwrap_or(false);
            if command.is_empty() {
                Err(LokError::new(
                    ErrorCode::InvalidRequest,
                    "post_uno 缺 command",
                ))
            } else {
                with_session!(state, |s| s.post_uno(&command, args.as_deref(), notify)).map(|_| {
                    state.bump_revision();
                    json!({})
                })
            }
        }
        "get_command_values" => {
            let command = p
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            with_session!(state, |s| s.get_command_values(&command)).map(|v| json!({"values": v}))
        }
        "get_text_selection" => {
            let mime = p
                .get("mime")
                .and_then(|v| v.as_str())
                .unwrap_or("text/plain;charset=utf-8");
            with_session!(state, |s| s.get_text_selection(mime)).map(|v| json!({"text": v}))
        }
        "set_client_zoom" => {
            let pw = p.get("px_w").and_then(|v| v.as_i64()).unwrap_or(256) as i32;
            let ph = p.get("px_h").and_then(|v| v.as_i64()).unwrap_or(256) as i32;
            let tw = p.get("tw_w").and_then(|v| v.as_i64()).unwrap_or(3840) as i32;
            let th = p.get("tw_h").and_then(|v| v.as_i64()).unwrap_or(3840) as i32;
            with_session!(state, |s| s.set_client_zoom(pw, ph, tw, th)).map(|_| json!({}))
        }
        "save_as" => {
            let url = p
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let format = p
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("docx")
                .to_string();
            let options = p
                .get("options")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if let Err(e) = guard_save_target(&url, &state.loaded_url) {
                return (Err(e), Vec::new(), false);
            }
            // 宿主侧先做过存在性检查；worker 再守一道目标目录可写。
            let dir_ok = url
                .strip_prefix("file://")
                .map(|p| {
                    let path = PathBuf::from(p);
                    path.parent().map(|d| d.exists()).unwrap_or(false)
                })
                .unwrap_or(false);
            if !dir_ok {
                return (
                    Err(LokError::new(
                        ErrorCode::SaveFailed,
                        format!("save_as 目标目录不存在：{url}"),
                    )),
                    Vec::new(),
                    false,
                );
            }
            with_session!(state, |s| s.save_as(&url, &format, &options)).map(|rc| {
                // 钉定头 saveAs 返回布尔语义：非 0=成功（实证探针：
                // rc=1 时产物有效落盘；详见 ffi.rs 注释）。
                if rc != 0 {
                    json!({"saved": true, "url": url})
                } else {
                    json!({"saved": false, "code": rc, "url": url})
                }
            })
        }
        "shutdown" => {
            // 响应写出后由 serve 收尾退出。
            return (Ok(json!({"bye": true})), Vec::new(), true);
        }
        _ => Err(LokError::new(
            ErrorCode::InvalidRequest,
            format!("未知方法：{method}"),
        )),
    };
    (result, Vec::new(), false)
}

fn hello_meta_of(state: &WorkerState) -> Value {
    state.hello_meta.clone()
}

fn forward_events(state: &mut WorkerState) {
    let events = with_session!(state, |s| s.drain_events());
    for ev in events {
        let name = callback_name(ev.kind);
        let payload = FrameBuf::from_json_value(
            FrameKind::Event,
            UNASKED,
            &json!({
                "type": ev.kind,
                "name": name,
                "payload": ev.payload,
                "rev": state.revision,
            }),
            Vec::new(),
        );
        emit(payload);
    }
}

/// 回调号 → 钉定头枚举名（诊断友好；未知号原样数字）。
fn callback_name(kind: i32) -> &'static str {
    match kind {
        lok_enums::CALLBACK_INVALIDATE_TILES => "invalidate_tiles",
        lok_enums::CALLBACK_INVALIDATE_VISIBLE_CURSOR => "invalidate_visible_cursor",
        lok_enums::CALLBACK_TEXT_SELECTION => "text_selection",
        lok_enums::CALLBACK_STATE_CHANGED => "state_changed",
        lok_enums::CALLBACK_DOCUMENT_SIZE_CHANGED => "document_size_changed",
        lok_enums::CALLBACK_UNO_COMMAND_RESULT => "uno_command_result",
        lok_enums::CALLBACK_ERROR => "error",
        _ => "unknown",
    }
}
