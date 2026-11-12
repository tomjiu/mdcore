//! 宿主侧客户端：worker 进程生命周期 + 请求/超时/事件消费 + 脏位语义。
//!
//! ## 隔离保证（issue 第 1/4 条）
//!
//! - 宿主进程**永不加载 LOK 内核**（见 [`crate::runtime`]「隔离铁律」）；
//!   所有内核调用都发生在 worker 进程，由本客户端代理。
//! - 超时：每个请求带截止时间；超时 → **杀 worker**（挂死的内核不可信，
//!   不等待、不重试同进程）→ 脏位保留 → 宿主可重开会话。
//! - worker 异常退出：读线程 EOF → 所有挂起请求立刻失败（`E_WORKER_DIED`，
//!   附 stderr 尾部），宿主继续可用。
//! - 脏位：输入/UNO 变更成功置位；**只有 save_as 成功才清位**；超时 /
//!   worker 死亡 / 保存失败一律保留（issue 明文）。

use crate::protocol::{self, codes};
use crate::runtime::{LokRuntime, LOK_HOME_ENV, LOK_LIB_ENV};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 宿主侧错误（含线上错误帧的错误码 + 本地超时/死亡错误）。
#[derive(Debug, Clone)]
pub enum LokError {
    /// 线上错误帧（code 见 [`crate::protocol::codes`]）。
    Lok { code: String, message: String },
    /// 请求超时（worker 已被看门狗杀掉）。
    Timeout { op: String, waited_ms: u128 },
    /// worker 异常退出（附 stderr 尾部与退出信息）。
    WorkerDied {
        detail: String,
        stderr_tail: Vec<String>,
    },
    /// 协议/IO 层错误。
    Protocol(String),
}

impl std::fmt::Display for LokError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LokError::Lok { code, message } => write!(f, "[{code}] {message}"),
            LokError::Timeout { op, waited_ms } => {
                write!(
                    f,
                    "[{}] op={op} 超时（{waited_ms}ms，worker 已被看门狗终止）",
                    codes::TIMEOUT
                )
            }
            LokError::WorkerDied {
                detail,
                stderr_tail,
            } => {
                write!(f, "[{}] {detail}", codes::WORKER_DIED)?;
                if !stderr_tail.is_empty() {
                    write!(f, "\nstderr 尾部：\n  {}", stderr_tail.join("\n  "))?;
                }
                Ok(())
            }
            LokError::Protocol(m) => write!(f, "[E_PROTOCOL] {m}"),
        }
    }
}

impl std::error::Error for LokError {}

impl LokError {
    /// 线上错误码。
    pub fn code(&self) -> &str {
        match self {
            LokError::Lok { code, .. } => code,
            LokError::Timeout { .. } => codes::TIMEOUT,
            LokError::WorkerDied { .. } => codes::WORKER_DIED,
            LokError::Protocol(_) => "E_PROTOCOL",
        }
    }
}

/// worker 事件（回调 + 生命周期）。
#[derive(Debug, Clone)]
pub enum Event {
    /// LOK 回调（type 值见 [`crate::abi::callbacks`]，payload 原文）。
    Callback {
        type_id: i32,
        name: String,
        payload: String,
        truncated: bool,
    },
    /// 生命周期（hello / ready / boot-failed / bye）。
    Life { kind: String, detail: Value },
}

/// 页矩形（twips，**画布坐标系**；`page_geometry` 应答单元）。
///
/// 来源 LOK `getPartPageRectangles`（25.2.3.2 实测，2026-11-11）：
/// Writer 分页模式下每页一条、顺序即页序，坐标与 paintTile 的文档
/// 坐标同源（跳页滚动锚定可直接用）。画布 = 页 + 四周 284 twips 边距，
/// 页间隙同为 284：单页为 `(284, 284, w, h)`，第 i 页 `y =
/// 284 + (页高 + 284) × i`，画布总高 = 末页底 + 284（与
/// getDocumentSize 自洽）。页数 = `pages.len()`（派生，不单测内核
/// `getPagesCount`——#19+ 未实证槽不扩绑，见 abi.rs 防漂移纪律）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRect {
    /// 页左上角 x（twips；画布坐标，单栏排版恒等于左边距 284）。
    pub x: i64,
    /// 页顶 y（twips；画布坐标：首页 284，后续页 = 前页底 + 284 页隙）。
    pub y: i64,
    /// 页宽（twips；A4 纵向 11906，不含画布边距）。
    pub w: i64,
    /// 页高（twips；含页边距，A4 纵向 16838）。
    pub h: i64,
}

/// `page_geometry` 应答 → 页矩形列表（两面共用：DocHandle / DocSession）。
fn parse_page_geometry(v: &Value) -> Result<Vec<PageRect>, LokError> {
    let arr = v
        .get("pages")
        .and_then(Value::as_array)
        .ok_or_else(|| LokError::Protocol("page_geometry 应答缺 pages 数组".into()))?;
    let mut out = Vec::with_capacity(arr.len());
    for p in arr {
        let g = |k: &str| p.get(k).and_then(Value::as_i64).unwrap_or(0);
        out.push(PageRect {
            x: g("x"),
            y: g("y"),
            w: g("w"),
            h: g("h"),
        });
    }
    Ok(out)
}

/// 默认每操作超时（LO 首启慢：hook_2 一次性数秒属正常）。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// worker 客户端。
pub struct WorkerClient {
    child: std::sync::Mutex<Child>,
    child_pid: u32,
    stdin: std::sync::Mutex<Option<ChildStdin>>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, Sender<Value>>>>,
    // events Sender 保活：读线程持克隆，本字段防止「唯一接收端被取走前
    // 通道被拆」（take_events 一次性取走接收端；此后由接收端持有连接）
    _events_keepalive: Sender<Event>,
    event_rx: Mutex<Option<Receiver<Event>>>,
    dead: Arc<AtomicBool>,
    death_detail: Arc<Mutex<String>>,
    stderr_tail: crate::worker::StderrTail,
    session_dir: PathBuf,
}

impl WorkerClient {
    /// spawn worker。
    ///
    /// `worker_bin`：worker 可执行文件（None = 与本进程同目录的
    /// `mdc-lok-worker`，或测试经 `CARGO_BIN_EXE_mdc-lok-worker` 注入）。
    /// `fake`：协议自测模式（echo/hang/die；无 LOK）。
    pub fn spawn(
        runtime: Option<&LokRuntime>,
        session_root: &Path,
        worker_bin: Option<&Path>,
        fake: Option<&str>,
    ) -> Result<WorkerClient, LokError> {
        let exe = worker_bin
            .map(PathBuf::from)
            // 运行时覆盖（部署时 worker 不在宿主同目录——如安装器
            // resources/ 布局；集成测试经此注入 cargo 构建产物）。
            .or_else(|| std::env::var_os("MDC_LOK_WORKER_BIN").map(PathBuf::from))
            .or_else(|| {
                std::env::current_exe().ok().and_then(|p| {
                    p.parent()
                        .map(|d| d.join(format!("mdc-lok-worker{}", std::env::consts::EXE_SUFFIX)))
                })
            })
            .ok_or_else(|| LokError::Protocol("无法定位 mdc-lok-worker 可执行文件".into()))?;
        if !exe.is_file() {
            return Err(LokError::Protocol(format!(
                "worker 可执行文件不存在：{exe:?}（部署时应与本进程同目录）"
            )));
        }
        if fake.is_none() && runtime.is_none() {
            return Err(LokError::Protocol(
                "真实 worker 需要 runtime（fake 模式才可不传）".into(),
            ));
        }

        std::fs::create_dir_all(session_root)
            .map_err(|e| LokError::Protocol(format!("建会话目录失败：{e}")))?;
        // 干净会话目录（残留 profile 可能带锁语义混乱；SAL_NOLOCK_PROFILE=1
        // 下无锁，但清空最稳）
        if let Ok(entries) = std::fs::read_dir(session_root) {
            for e in entries.flatten() {
                if e.path().is_dir() {
                    let _ = std::fs::remove_dir_all(e.path());
                } else {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }

        let mut cmd = Command::new(&exe);
        cmd.arg("--session-dir").arg(session_root);
        if let Some(mode) = fake {
            cmd.arg("--fake").arg(mode);
        }
        if let Some(rt) = runtime {
            cmd.arg("--kernel-lib").arg(&rt.kernel_lib);
            cmd.arg("--program-dir").arg(&rt.program_dir);
            cmd.arg("--kernel-sha256").arg(&rt.kernel_sha256);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // 环境卫生（worker 启动时也会自设；双侧幂等）
        cmd.env("SAL_NOLOCK_PROFILE", "1");
        cmd.env("SAL_DISABLE_OPENCL", "1");
        // 【Windows 实测 2026-11-12】unipoll 在 Windows 上会让 LO 跳过内核
        // 主线程启动（init.cxx 仅在非 unipoll 分支 osl_createThread
        // (lo_startmain)），user installation 因此不建立，documentLoad 实测
        // 访问违例（0xC0000005，stock 26.8 同症）。故 Windows 侧清掉该变量
        // （含宿主环境继承值），交由 worker 的 serial_loop 串行直调；
        // 非 Windows 仍走 unipoll 主循环。证据链见 worker.rs `serial_loop`。
        if cfg!(windows) {
            cmd.env_remove("SAL_LOK_OPTIONS");
        } else {
            cmd.env("SAL_LOK_OPTIONS", "unipoll");
        }
        if cfg!(windows) {
            cmd.env("SAL_SKIA", "raster");
        } else {
            cmd.env("SAL_DISABLESKIA", "1");
        }
        // 发现序环境透传（worker 内 bind 需要同序发现）
        for k in [LOK_HOME_ENV, LOK_LIB_ENV] {
            if let Some(v) = std::env::var_os(k) {
                cmd.env(k, v);
            }
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| LokError::Protocol(format!("spawn worker 失败（{exe:?}）：{e}")))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        let pending: Arc<Mutex<HashMap<u64, Sender<Value>>>> = Arc::new(Mutex::new(HashMap::new()));
        let (ev_tx, ev_rx) = std::sync::mpsc::channel::<Event>();
        let dead = Arc::new(AtomicBool::new(false));
        let death_detail = Arc::new(Mutex::new(String::new()));

        // 读线程：应答路由 + 事件转发 + EOF 死亡扩散
        std::thread::Builder::new()
            .name("lok-reader".into())
            .spawn({
                let pending = pending.clone();
                let dead = dead.clone();
                let death_detail = death_detail.clone();
                let ev_tx_reader = ev_tx.clone();
                move || reader_loop(stdout, pending, ev_tx_reader, dead, death_detail)
            })
            .map_err(|e| LokError::Protocol(format!("spawn reader 线程失败：{e}")))?;

        // stderr 线程：尾部环形保留（诊断；绝不吞掉）
        let stderr_tail = crate::worker::StderrTail::new(200);
        let tail_buf = stderr_tail.lines.clone();
        std::thread::Builder::new()
            .name("lok-stderr".into())
            .spawn(move || {
                let mut r = BufReader::new(stderr);
                let mut line = String::new();
                loop {
                    line.clear();
                    match r.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            if let Ok(mut q) = tail_buf.lock() {
                                if q.len() >= 200 {
                                    q.pop_front();
                                }
                                q.push_back(line.trim_end().to_string());
                            }
                        }
                    }
                }
            })
            .map_err(|e| LokError::Protocol(format!("spawn stderr 线程失败：{e}")))?;

        Ok(WorkerClient {
            child_pid: child.id(),
            child: std::sync::Mutex::new(child),
            stdin: std::sync::Mutex::new(stdin),
            next_id: AtomicU64::new(1),
            pending,
            _events_keepalive: ev_tx,
            event_rx: Mutex::new(Some(ev_rx)),
            dead,
            death_detail,
            stderr_tail,
            session_dir: session_root.to_path_buf(),
        })
    }

    /// 子进程 PID（诊断/被杀步骤记录用）。
    pub fn session_pid(&self) -> u32 {
        self.child_pid
    }

    /// 会话目录（tile side file 落在这里）。
    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }

    /// worker 是否已死。
    pub fn is_dead(&self) -> bool {
        self.dead.load(Ordering::SeqCst)
    }

    /// 死亡细节（诊断）。
    pub fn death_detail(&self) -> String {
        self.death_detail
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    /// stderr 尾部（最近的在后）。
    pub fn stderr_tail(&self, n: usize) -> Vec<String> {
        self.stderr_tail.tail(n)
    }

    /// 取事件接收端（一次性；多订阅者自建 channel 转发）。
    pub fn take_events(&self) -> Option<Receiver<Event>> {
        self.event_rx.lock().ok().and_then(|mut g| g.take())
    }

    /// 非阻塞取一条事件。
    pub fn try_recv_event(&self) -> Option<Event> {
        self.event_rx
            .lock()
            .ok()
            .and_then(|mut g| g.as_mut().and_then(|rx| rx.try_recv().ok()))
    }

    /// 带超时取一条事件（回调断言用）。
    pub fn recv_event_timeout(&self, dur: Duration) -> Option<Event> {
        self.event_rx
            .lock()
            .ok()
            .and_then(|mut g| g.as_mut().and_then(|rx| rx.recv_timeout(dur).ok()))
    }

    /// 发请求（默认超时 [`DEFAULT_TIMEOUT`]）。
    pub fn request(&self, op: &str, args: Value) -> Result<Value, LokError> {
        self.request_with_timeout(op, args, DEFAULT_TIMEOUT)
    }

    /// 发请求（带截止超时；超时 → 杀 worker → `Timeout`）。
    pub fn request_with_timeout(
        &self,
        op: &str,
        args: Value,
        timeout: Duration,
    ) -> Result<Value, LokError> {
        if self.is_dead() {
            return Err(LokError::WorkerDied {
                detail: format!(
                    "worker 已死：{}（本请求 {} 未发出；脏位语义不受影响）",
                    self.death_detail(),
                    op
                ),
                stderr_tail: self.stderr_tail(10),
            });
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = std::sync::mpsc::channel::<Value>();
        if let Ok(mut p) = self.pending.lock() {
            p.insert(id, tx);
        }
        let mut v = json!({"id": id, "op": op});
        if let (Some(obj), Some(extra)) = (v.as_object_mut(), args.as_object()) {
            for (k, val) in extra {
                obj.insert(k.clone(), val.clone());
            }
        }
        // 请求帧长上限（写侧拒发）
        let line = serde_json::to_vec(&v)
            .map_err(|e| LokError::Protocol(format!("请求序列化失败：{e}")))?;
        if line.len() > crate::MAX_FRAME_BYTES {
            if let Ok(mut p) = self.pending.lock() {
                p.remove(&id);
            }
            return Err(LokError::Lok {
                code: codes::OVERSIZE.to_string(),
                message: format!(
                    "请求帧 {} 字节超上限 {}（op={op}）",
                    line.len(),
                    crate::MAX_FRAME_BYTES
                ),
            });
        }
        {
            let guard = self
                .stdin
                .lock()
                .map_err(|e| LokError::Protocol(format!("stdin 锁失败（worker 已死？）：{e}")))?;
            let mut w = guard
                .as_ref()
                .ok_or_else(|| LokError::Protocol("stdin 已关闭".into()))?;
            if let Err(e) = w
                .write_all(&line)
                .and_then(|_| w.write_all(b"\n"))
                .and_then(|_| w.flush())
            {
                if let Ok(mut p) = self.pending.lock() {
                    p.remove(&id);
                }
                return Err(LokError::WorkerDied {
                    detail: format!("写 worker stdin 失败（op={op}）：{e}"),
                    stderr_tail: self.stderr_tail(10),
                });
            }
        }
        let started = Instant::now();
        match rx.recv_timeout(timeout) {
            Ok(answer) => {
                if answer.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                    Ok(answer)
                } else {
                    let e = answer.get("error").cloned().unwrap_or(json!({}));
                    Err(LokError::Lok {
                        code: e
                            .get("code")
                            .and_then(Value::as_str)
                            .unwrap_or("E_INTERNAL")
                            .to_string(),
                        message: e
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("worker 应答缺错误信息")
                            .to_string(),
                    })
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                // 看门狗：挂死内核不可信 → 杀；脏位语义在 DocHandle 层保留
                self.kill();
                Err(LokError::Timeout {
                    op: op.to_string(),
                    waited_ms: started.elapsed().as_millis(),
                })
            }
            Err(RecvTimeoutError::Disconnected) => Err(LokError::WorkerDied {
                detail: format!("worker 死亡扩散到达（op={op}）：{}", self.death_detail()),
                stderr_tail: self.stderr_tail(10),
            }),
        }
    }

    /// 硬杀 worker（超时看门狗 / 宿主主动放弃）。
    pub fn kill(&self) {
        if !self.dead.swap(true, Ordering::SeqCst) {
            if let Ok(mut child) = self.child.lock() {
                let _ = child.kill();
                let _ = child.wait();
            }
            if let Ok(mut d) = self.death_detail.lock() {
                if d.is_empty() {
                    *d = "宿主看门狗终止（超时/放弃）".into();
                }
            }
        }
        if let Ok(mut s) = self.stdin.lock() {
            *s = None;
        }
        self.fail_all_pending();
    }

    /// 关机（先友好 shutdown 请求，1s 后兜底杀）。
    pub fn shutdown(&self) {
        if self.is_dead() {
            return;
        }
        let _ = self.request_with_timeout("shutdown", json!({}), Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(300));
        if !self.is_dead() {
            self.kill();
        }
    }

    fn fail_all_pending(&self) {
        if let Ok(mut p) = self.pending.lock() {
            let map: HashMap<u64, Sender<Value>> = std::mem::take(&mut *p);
            for (_, tx) in map {
                // 值不会到达：挂起方经 Disconnected 得到 WorkerDied
                drop(tx);
            }
        }
    }
}

impl Drop for WorkerClient {
    fn drop(&mut self) {
        if !self.is_dead() {
            self.kill();
        }
    }
}

/// 读线程主体。
fn reader_loop<R: Read>(
    stdout: R,
    pending: Arc<Mutex<HashMap<u64, Sender<Value>>>>,
    events: Sender<Event>,
    dead: Arc<AtomicBool>,
    death_detail: Arc<Mutex<String>>,
) {
    let mut reader = BufReader::new(stdout);
    loop {
        let frame = match protocol::read_frame(&mut reader) {
            Ok(Some(f)) => f,
            Ok(None) => {
                mark_dead(
                    &dead,
                    &death_detail,
                    "worker stdout 干净 EOF（正常退出或被杀）",
                );
                break;
            }
            Err(e) => {
                mark_dead(&dead, &death_detail, &format!("worker stdout 帧超限：{e}"));
                break;
            }
        };
        let v: Value = match serde_json::from_slice(&frame) {
            Ok(v) => v,
            Err(_) => continue, // 坏行跳过（应答缺失由超时兜底）
        };
        if let Some(ev) = v.get("ev").and_then(Value::as_str) {
            if ev == "cb" {
                let e = Event::Callback {
                    type_id: v.get("type").and_then(Value::as_i64).unwrap_or(-1) as i32,
                    name: v
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                        .to_string(),
                    payload: v
                        .get("payload")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    truncated: v
                        .get("payload_truncated")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                };
                let _ = events.send(e);
            } else {
                let e = Event::Life {
                    kind: ev.to_string(),
                    detail: v.get("detail").cloned().unwrap_or(json!({})),
                };
                let _ = events.send(e);
            }
            continue;
        }
        if let Some(id) = v.get("id").and_then(Value::as_u64) {
            if let Ok(mut p) = pending.lock() {
                if let Some(tx) = p.remove(&id) {
                    let _ = tx.send(v);
                }
            }
        }
    }
    // EOF：清空所有挂起（发送端 drop → 调用方得 WorkerDied）
    if let Ok(mut p) = pending.lock() {
        let map: HashMap<u64, Sender<Value>> = std::mem::take(&mut *p);
        for (_, tx) in map {
            drop(tx);
        }
    }
}

fn mark_dead(dead: &AtomicBool, detail: &Mutex<String>, why: &str) {
    dead.store(true, Ordering::SeqCst);
    if let Ok(mut d) = detail.lock() {
        if d.is_empty() {
            *d = why.to_string();
        }
    }
}

/// 打开的文档句柄（脏位宿主侧语义）。
pub struct DocHandle<'a> {
    client: &'a WorkerClient,
    dirty: std::cell::Cell<bool>,
    loaded_url: String,
}

impl<'a> DocHandle<'a> {
    /// load + 建句柄（单文档 worker：成功后本 worker 上不能再 load）。
    pub fn load(
        client: &'a WorkerClient,
        url: &str,
        options: Option<&str>,
    ) -> Result<DocHandle<'a>, LokError> {
        let mut a = json!({"url": url});
        if let Some(o) = options {
            a["options"] = json!(o);
        }
        client.request("load", a)?;
        Ok(DocHandle::new(client, url))
    }

    fn new(client: &'a WorkerClient, url: &str) -> DocHandle<'a> {
        DocHandle {
            client,
            dirty: std::cell::Cell::new(false),
            loaded_url: url.to_owned(),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    fn mark_dirty(&self) {
        self.dirty.set(true);
    }

    /// 任意请求的脏位包装：成功回调收到 result 后置位（编辑类 op）。
    fn edit_request(&self, op: &str, args: Value) -> Result<Value, LokError> {
        self.mark_dirty();
        self.client.request(op, args)
    }

    pub fn ping(&self) -> Result<Value, LokError> {
        self.client.request("ping", json!({}))
    }

    pub fn version(&self) -> Result<String, LokError> {
        let v = self.client.request("version", json!({}))?;
        Ok(v.get("version_info")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string())
    }

    pub fn init_rendering(&self, args: Option<&str>) -> Result<Value, LokError> {
        let mut a = json!({});
        if let Some(s) = args {
            a["args"] = json!(s);
        }
        self.client.request("init_rendering", a)
    }

    pub fn size(&self) -> Result<(i64, i64), LokError> {
        let v = self.client.request("size", json!({}))?;
        Ok((
            v.get("width_twips").and_then(Value::as_i64).unwrap_or(0),
            v.get("height_twips").and_then(Value::as_i64).unwrap_or(0),
        ))
    }

    /// 页几何（getPartPageRectangles → 按页序的页矩形，twips）。
    /// 跳页/页码/页间分隔的数据源（#50 C 线 2026-11-10 协议缺口登记）。
    pub fn page_geometry(&self) -> Result<Vec<PageRect>, LokError> {
        let v = self.client.request("page_geometry", json!({}))?;
        parse_page_geometry(&v)
    }

    /// paintTile（返回 side file 路径与统计）。
    pub fn paint(
        &self,
        canvas_w: usize,
        canvas_h: usize,
        tile_x: i32,
        tile_y: i32,
        tile_w: i32,
        tile_h: i32,
    ) -> Result<Value, LokError> {
        self.client.request(
            "paint",
            json!({
                "canvas_w": canvas_w, "canvas_h": canvas_h,
                "tile_x": tile_x, "tile_y": tile_y,
                "tile_w": tile_w, "tile_h": tile_h,
            }),
        )
    }

    /// 输入文本（逐字符 KEYINPUT+KEYUP；选区被替换——标准编辑语义）。
    /// 成功即置脏位。
    pub fn type_text(&self, text: &str) -> Result<(), LokError> {
        for ch in text.chars() {
            self.edit_request("post_key", json!({"char": ch as i32, "key_code": 0}))?;
        }
        Ok(())
    }

    /// IME 文本（LOK 槽 #40 postWindowExtTextInputEvent）。
    /// `commit=false` = 组合期（LOK_EXT_TEXTINPUT）；`commit=true` =
    /// 提交（LOK_EXT_TEXTINPUT_END）。
    ///
    /// **⚠️ stock 25.2.3.2 语义不可靠（契约文档 §10 实证登记）**：首个
    /// 会话组合串被当定稿插入、后续会话事件被静默丢弃——产品 IME
    /// 通道用 [`Self::type_text`]（preedit 宿主渲染，定稿逐字符下发，
    /// #50 C3 / A 线批次 12 架构）；本方法保留为内核 preedit 通道的
    /// 机械面，待 LO 升级/补丁运行时重实证后解锁。
    pub fn post_text(&self, commit: bool, text: &str) -> Result<(), LokError> {
        self.edit_request("post_text", json!({"commit": commit, "text": text}))?;
        Ok(())
    }

    /// postUnoCommand（args = JSON 字符串）。编辑类命令成功置脏位
    /// （查询类命令由调用方用 [`Self::uno_query`]）。
    pub fn uno(&self, command: &str, args: Option<&str>, notify: bool) -> Result<Value, LokError> {
        let mut a = json!({"command": command, "notify": notify});
        if let Some(s) = args {
            a["args"] = json!(s);
        }
        self.edit_request("uno", a)
    }

    /// 非变更类 UNO（SelectAll / ExecuteSearch 查找等；不置脏位）。
    pub fn uno_query(&self, command: &str, args: Option<&str>) -> Result<Value, LokError> {
        let mut a = json!({"command": command, "notify": false});
        if let Some(s) = args {
            a["args"] = json!(s);
        }
        self.client.request("uno", a)
    }

    /// getTextSelection。
    pub fn text_selection(&self, mime: &str) -> Result<(String, String), LokError> {
        let v = self
            .client
            .request("text_selection", json!({"mime": mime}))?;
        Ok((
            v.get("mime")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            v.get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ))
    }

    /// saveAs：**成功才清脏位**（issue 明文：超时/异常/失败不清）。
    pub fn save_as(&self, url: &str, format: &str) -> Result<Value, LokError> {
        crate::save_guard::check(&self.loaded_url, url).map_err(LokError::Protocol)?;
        let r = self
            .client
            .request("save_as", json!({"url": url, "format": format}));
        if r.is_ok() {
            self.dirty.set(false);
        } // Err 路径脏位保留
        r
    }

    /// 导出 PDF（LOK saveAs(url, "pdf", typed options)；详见 B2 契约文档
    /// §7。`page_range = Some((from, to))` 1 起含端，None=全文；
    /// `notes_in_margin` 批注边栏；`tracked_changes` 修订标记。
    /// 非变更类：dirty 不动（产物是新文件，不改文档状态）。
    pub fn export_pdf(
        &self,
        url: &str,
        page_range: Option<(u32, u32)>,
        notes_in_margin: bool,
        tracked_changes: bool,
    ) -> Result<Value, LokError> {
        crate::save_guard::check(&self.loaded_url, url).map_err(LokError::Protocol)?;
        let mut a = json!({
            "url": url,
            "notes_in_margin": notes_in_margin,
            "tracked_changes": tracked_changes,
        });
        if let Some((from, to)) = page_range {
            a["page_from"] = json!(from);
            a["page_to"] = json!(to);
        }
        self.client.request("export_pdf", a)
    }

    pub fn close(&self) -> Result<Value, LokError> {
        self.client.request("close", json!({}))
    }
}

// ============================================================================
// B2：可拥有的产品会话（#49 工作线 B → #50 C 线消费契约）
// ============================================================================

/// 可拥有的文档会话——GPUI 宿主（#50 C 线）的产品接入形态。
///
/// 与 [`DocHandle`]（借用 `&WorkerClient`，smoke/内部测试用）不同：本类型
/// **拥有** worker 子进程与全部通道，无生命周期参数——可直接放进 GPUI
/// Entity / 状态树并在线程间 move（`Send`）。所有方法 `&self`/`&mut self`，
/// 不自引用、不捕获借用。
///
/// ## 线程与阻塞契约（C 线必读）
///
/// - 方法是**同步阻塞**的（等 worker 应答，多数毫秒级；`open` 首启含
///   LO hook_2 可达数秒）。**不要在 UI 线程上直接调用**——GPUI 宿主应
///   在后台任务线程调用，UI 线程只消费 [`Self::take_events`] 的回调流。
/// - 会话与文档一一对应（单文档 worker 契约）：一文档一会话一进程；
///   多文档 = 多会话（`open` 多次，独立 profile 不互锁——smoke 实测）。
/// - 事件通道一次性：[`Self::take_events`] 第一次调用返回 `Receiver`，
///   由调用方持有轮询；之后可用 [`Self::try_recv_event`] /
///   [`Self::recv_event_timeout`]（通道被取走后这些方法内部经共享
///   暂存转发，语义不丢）。
///
/// ## dirty / revision 契约
///
/// - `dirty`：编辑类操作（`type_text` / `uno(notify=true)`）成功置位；
///   **仅 `save_as` 成功清位**（超时/worker 死亡/保存失败一律保留——
///   `LokError` 返回即未清）。
/// - `revision`：宿主侧单调计数，**每次成功编辑 +1、`save_as` 成功 +1**；
///   初始 0。C 线用它丢弃过期 tile（编辑后旧 tile 作废）。worker 侧
///   回调流尚不携带 revision（B4 计划下沉到 worker 并随事件回传——
///   已登记 #50 缺口）。
pub struct DocSession {
    client: WorkerClient,
    dirty: bool,
    revision: u64,
    loaded_url: String,
}

impl std::fmt::Debug for DocSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocSession")
            .field("pid", &self.client.session_pid())
            .field("dirty", &self.dirty)
            .field("revision", &self.revision)
            .field("url", &self.loaded_url)
            .field("dead", &self.client.is_dead())
            .finish()
    }
}

impl DocSession {
    /// 打开文档一步到位：spawn worker（独立 profile）→ load →
    /// initializeForRendering（`DeviceFormFactor=desktop`）。
    ///
    /// `runtime`：None = 发现序自动定位（`MDC_LOK_HOME`/`MDC_LOK_LIB` →
    /// 部署位 → 平台常见位；见 [`crate::runtime`]）。`session_root`：
    /// 会话目录根（profile/诊断产物落这里，建议每会话独立子目录——
    /// 本方法自动追加 `session-<pid>` 语义由调用方保证，直接传唯一
    /// 路径即可）。
    pub fn open(
        runtime: Option<&crate::runtime::LokRuntime>,
        session_root: &Path,
        url: &str,
        options: Option<&str>,
    ) -> Result<DocSession, LokError> {
        // runtime=None = 发现序自动定位（MDC_LOK_HOME / MDC_LOK_LIB →
        // 部署位 → 平台常见位）；显式传入用于测试与多运行时选择。
        let discovered;
        let rt = match runtime {
            Some(r) => r,
            None => {
                discovered = crate::runtime::discover()
                    .map_err(|e| LokError::Protocol(format!("LOK 运行时发现失败：{e}")))?;
                &discovered
            }
        };
        let client = WorkerClient::spawn(Some(rt), session_root, None, None)?;
        let mut a = json!({"url": url});
        if let Some(o) = options {
            a["options"] = json!(o);
        }
        client.request("load", a)?;
        // 与 smoke 全链一致：init_rendering 空 args（25.2 实测
        // initializeForRendering("DeviceFormFactor=desktop") 触发内核
        // Application Error——参数格式按版本有分叉，产品默认不传，见
        // docs/lok-session-contract.md「不稳定参数」节）。
        client.request("init_rendering", json!({}))?;
        Ok(DocSession {
            client,
            dirty: false,
            revision: 0,
            loaded_url: url.to_string(),
        })
    }

    /// 已加载文档的 URL（load 参数原样）。
    pub fn loaded_url(&self) -> &str {
        &self.loaded_url
    }

    /// 会话 pid（诊断/进程管理）。
    pub fn pid(&self) -> u32 {
        self.client.session_pid()
    }

    /// 会话目录（profile 与诊断产物所在）。
    pub fn session_dir(&self) -> &Path {
        self.client.session_dir()
    }

    /// 文档脏位（见类型文档「dirty / revision 契约」）。
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// 宿主侧 revision（见类型文档「dirty / revision 契约」）。
    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn edit_ok(&mut self) {
        self.dirty = true;
        self.revision += 1;
    }

    /// 事件通道（一次性取走；见类型文档「线程与阻塞契约」）。
    pub fn take_events(&self) -> Option<Receiver<Event>> {
        self.client.take_events()
    }

    pub fn try_recv_event(&self) -> Option<Event> {
        self.client.try_recv_event()
    }

    pub fn recv_event_timeout(&self, dur: Duration) -> Option<Event> {
        self.client.recv_event_timeout(dur)
    }

    /// worker 是否已死（死后所有请求明确 `LokError::WorkerDied`）。
    pub fn is_dead(&self) -> bool {
        self.client.is_dead()
    }

    /// 死亡详情 + stderr 尾部（诊断）。
    pub fn death_detail(&self) -> String {
        self.client.death_detail()
    }

    /// getDocumentSize → (宽, 高) twips。
    pub fn size(&self) -> Result<(i64, i64), LokError> {
        let v = self.client.request("size", json!({}))?;
        Ok((
            v.get("width_twips").and_then(Value::as_i64).unwrap_or(0),
            v.get("height_twips").and_then(Value::as_i64).unwrap_or(0),
        ))
    }

    /// 页几何（getPartPageRectangles → 按页序的页矩形，twips；
    /// B2 契约面——C 线跳页/页码/页间分隔的数据源）。页数 = 返回长度。
    pub fn page_geometry(&self) -> Result<Vec<PageRect>, LokError> {
        let v = self.client.request("page_geometry", json!({}))?;
        parse_page_geometry(&v)
    }

    /// paintTile：tile 像素经 side file（BMP）返回，JSON 含
    /// `tile`（BMP 路径）/`tile_mode`/统计（见 worker op_paint）。
    pub fn paint(
        &self,
        canvas_w: usize,
        canvas_h: usize,
        tile_x: i32,
        tile_y: i32,
        tile_w: i32,
        tile_h: i32,
    ) -> Result<Value, LokError> {
        self.client.request(
            "paint",
            json!({
                "canvas_w": canvas_w, "canvas_h": canvas_h,
                "tile_x": tile_x, "tile_y": tile_y,
                "tile_w": tile_w, "tile_h": tile_h,
            }),
        )
    }

    /// 键入文本（逐字符 KEYINPUT+KEYUP；选区被替换——标准编辑语义）。
    /// 成功：dirty=true、revision+1。
    pub fn type_text(&mut self, text: &str) -> Result<(), LokError> {
        if text.is_empty() {
            return Ok(());
        }
        // A failed reply can follow an applied edit. Preserve conservative dirty
        // and invalidate old tiles before sending the first mutating request.
        self.edit_ok();
        for ch in text.chars() {
            self.client
                .request("post_key", json!({"char": ch as i32, "key_code": 0}))?;
        }
        Ok(())
    }

    /// IME 文本（LOK 槽 #40 postWindowExtTextInputEvent；B2 契约面）。
    /// `commit=false` = 组合期；`commit=true` = 提交。
    /// 成功：dirty=true、revision+1。
    ///
    /// **⚠️ stock 25.2.3.2 语义不可靠（契约文档 §10 实证登记）**：产品
    /// IME 通道用 [`Self::type_text`]（GPUI preedit 宿主渲染 + 定稿逐
    /// 字符 postKeyEvent，tests/ime_text.rs 实证全链含落盘）；本方法
    /// 为内核 preedit 通道机械面，待重实证后解锁。
    pub fn post_text(&mut self, commit: bool, text: &str) -> Result<(), LokError> {
        self.edit_ok();
        self.client
            .request("post_text", json!({"commit": commit, "text": text}))?;
        Ok(())
    }

    /// postUnoCommand——编辑/变更类 UNO（如 `.uno:Undo`/`.uno:Redo`/
    /// `.uno:SelectAll`）。成功：dirty=true、revision+1。
    /// 查询类（ExecuteSearch 查找等）用 [`Self::uno_query`]。
    pub fn uno(&mut self, command: &str, args: Option<&str>) -> Result<Value, LokError> {
        let mut a = json!({"command": command, "notify": false});
        if let Some(s) = args {
            a["args"] = json!(s);
        }
        self.edit_ok();
        self.client.request("uno", a)
    }

    /// 非变更类 UNO（不置脏、不动 revision）。
    pub fn uno_query(&self, command: &str, args: Option<&str>) -> Result<Value, LokError> {
        let mut a = json!({"command": command, "notify": false});
        if let Some(s) = args {
            a["args"] = json!(s);
        }
        self.client.request("uno", a)
    }

    /// getTextSelection（mime 用 `text/plain;charset=utf-8`——裸
    /// `text/plain` 不是 transferable flavor，返回空）。
    pub fn text_selection(&self, mime: &str) -> Result<(String, String), LokError> {
        let v = self
            .client
            .request("text_selection", json!({"mime": mime}))?;
        Ok((
            v.get("mime")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            v.get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ))
    }

    /// saveAs 新文件（**不得等于源文件**——worker 硬守卫拒绝）。成功：
    /// dirty=false、revision+1（保存也是新状态，C 线据此失效旧 tile）。
    /// 失败：dirty/revision 均不动。
    pub fn save_as(&mut self, url: &str, format: &str) -> Result<Value, LokError> {
        crate::save_guard::check(&self.loaded_url, url).map_err(LokError::Protocol)?;
        let r = self
            .client
            .request("save_as", json!({"url": url, "format": format}))?;
        self.dirty = false;
        self.revision += 1;
        Ok(r)
    }

    /// 导出 PDF（LOK saveAs(url, "pdf", typed options)；B2 契约面——
    /// 暗标导出线消费面）。`page_range = Some((from, to))` 1 起含端，
    /// None=全文；`notes_in_margin` 批注边栏；`tracked_changes` 修订标记。
    /// 非变更类：dirty/revision 不动（产物是新文件，不改文档状态）。
    pub fn export_pdf(
        &self,
        url: &str,
        page_range: Option<(u32, u32)>,
        notes_in_margin: bool,
        tracked_changes: bool,
    ) -> Result<Value, LokError> {
        crate::save_guard::check(&self.loaded_url, url).map_err(LokError::Protocol)?;
        let mut a = json!({
            "url": url,
            "notes_in_margin": notes_in_margin,
            "tracked_changes": tracked_changes,
        });
        if let Some((from, to)) = page_range {
            a["page_from"] = json!(from);
            a["page_to"] = json!(to);
        }
        self.client.request("export_pdf", a)
    }

    /// 逻辑关闭（单文档 worker：close 后本会话 doc 请求明确
    /// `E_NO_DOCUMENT`；**不调 LO destroy**——teardown abort 规避，
    /// 落盘只依赖 save_as）。
    pub fn close(&self) -> Result<Value, LokError> {
        self.client.request("close", json!({}))
    }

    /// 结束会话：close + kill（SIGKILL 兜底；跳过 LO atexit——
    /// `SfxBaseModel` teardown abort 规避，产物只认 saveAs 落盘）。
    pub fn shutdown(self) {
        let _ = self.close();
        self.client.kill();
    }
}

#[cfg(test)]
mod save_safety_tests {
    use super::*;

    fn session(mode: &str, tag: &str) -> (DocSession, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "mdc-lok-save-safety-{}-{}-{tag}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("原 文.docx");
        std::fs::write(&source, b"original source bytes").unwrap();
        let worker = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(format!("mdc-lok-worker{}", std::env::consts::EXE_SUFFIX));
        let client =
            WorkerClient::spawn(None, &root.join("ipc"), Some(&worker), Some(mode)).unwrap();
        (
            DocSession {
                client,
                dirty: false,
                revision: 0,
                loaded_url: crate::profile::file_url_of(&source),
            },
            root,
        )
    }

    #[test]
    fn source_guard_rejects_original_and_hard_link_without_clearing_dirty() {
        let (mut session, root) = session("echo", "source");
        session.dirty = true;
        session.revision = 7;
        let source = root.join("原 文.docx");
        let alias = root.join("alias.docx");
        std::fs::hard_link(&source, &alias).unwrap();
        for target in [&source, &alias] {
            let result = session.save_as(&crate::profile::file_url_of(target), "docx");
            assert!(
                result.is_err(),
                "saving over the source or its hard link must fail"
            );
            assert!(session.is_dirty());
            assert_eq!(session.revision(), 7);
            assert_eq!(std::fs::read(&source).unwrap(), b"original source bytes");
        }
        session.shutdown();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn partial_text_edit_failure_preserves_dirty_and_revision() {
        let (mut session, root) = session("edit-fails-after-first", "partial");
        assert!(session.type_text("ab").is_err());
        assert!(
            session.is_dirty(),
            "the first acknowledged character was edited"
        );
        assert!(session.revision() > 0);
        session.shutdown();
        std::fs::remove_dir_all(root).unwrap();
    }
}
