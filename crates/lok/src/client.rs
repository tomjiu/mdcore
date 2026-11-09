//! 宿主端 worker 句柄（#633）。
//!
//! 职责（issue #633 任务 1）：spawn worker 子进程、请求 ID 关联、
//! 每请求超时、帧长上限（proto 层）、事件通道、崩溃检测（EOF →
//! 在途请求全部失败并带 stderr 尾部）、kill + 重新拉起（宿主始终
//! 可恢复，不因一个 worker 死亡停摆）。
//!
//! profile 隔离：每次 spawn 生成独立 profile 目录（不复用平台共享
//! profile；被杀 worker 不留 stale .lock 影响下一个会话——多会话
//! 互不锁定）。TMPDIR/TMP/TEMP 重定向进该 profile，LO 临时文件不
//! 逃逸。真实模式强制 `SAL_LOK_OPTIONS=unipoll`（worker.rs 模块头
//! 注释：异步事件黑洞修复的必要条件）。

use std::collections::HashMap;
use std::io::{BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::error::{ErrorCode, LokError, LokResult};
use crate::proto::{read_frame, write_frame, FrameBuf, FrameKind};

/// 默认请求超时（可按方法覆盖；paint/save 在会话层给更长档）。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const STDERR_TAIL_LINES: usize = 40;

/// worker 运行时配置。
#[derive(Debug, Clone)]
pub struct LokRuntimeConfig {
    /// LibreOffice program 目录（真实模式）；None = 自动发现序
    /// （MDC_LOK_DIR → 平台候选，见 ffi::default_program_dirs）。
    pub program_dir: Option<PathBuf>,
    /// worker 二进制；None = 当前可执行文件同目录找
    /// `mdc-lok-worker(-.exe)`。
    pub worker_bin: Option<PathBuf>,
    /// 单请求默认超时。
    pub timeout: Duration,
    /// mock 后端故障注入：在该方法上永久睡眠（边界测试超时门专用；
    /// **仅 mock 模式生效**，真实后端忽略。per-worker 设置，不污染
    /// 测试进程全局环境——并行测试互不影响）。
    pub mock_hang_method: Option<String>,
}

impl Default for LokRuntimeConfig {
    fn default() -> Self {
        LokRuntimeConfig {
            program_dir: None,
            worker_bin: None,
            timeout: DEFAULT_TIMEOUT,
            mock_hang_method: None,
        }
    }
}

/// 一次已收到的 LOK 事件（worker 回调转发）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct LokEventMsg {
    #[serde(rename = "type")]
    pub kind: i32,
    pub name: String,
    pub payload: String,
    pub rev: u64,
}

struct Shared {
    pending: Mutex<HashMap<u64, std::sync::mpsc::Sender<FrameBuf>>>,
    alive: AtomicBool,
    stderr_tail: Mutex<Vec<String>>,
}

/// 活着的 worker 句柄。
pub struct LokWorker {
    child: Child,
    stdin: std::process::ChildStdin,
    next_id: AtomicU64,
    shared: Arc<Shared>,
    events_rx: std::sync::mpsc::Receiver<LokEventMsg>,
}

impl LokWorker {
    /// spawn 真实 worker（自动发现运行时；发现失败给出可行动错误）。
    pub fn spawn(cfg: &LokRuntimeConfig) -> LokResult<Self> {
        let program_dir = match &cfg.program_dir {
            Some(d) => Some(d.clone()),
            None => crate::ffi::default_program_dirs()
                .into_iter()
                .find(|d| crate::ffi::pick_library(d).is_ok()),
        };
        let program_dir = program_dir.ok_or_else(|| {
            LokError::runtime_not_found(
                "未发现 LibreOffice 运行时：先装 LibreOffice（≥6.0 完整安装）\
                 或设 MDC_LOK_DIR=<program 目录>；Windows 发现方式与 hash 记录\
                 见 docs/lok-runtime.md",
            )
        })?;
        Self::spawn_inner(cfg, Some(program_dir), false)
    }

    /// spawn mock worker（协议/边界测试与 #616 宿主开发用）。
    pub fn spawn_mock(cfg: &LokRuntimeConfig) -> LokResult<Self> {
        Self::spawn_inner(cfg, None, true)
    }

    fn spawn_inner(
        cfg: &LokRuntimeConfig,
        program_dir: Option<PathBuf>,
        mock: bool,
    ) -> LokResult<Self> {
        let worker_bin = match &cfg.worker_bin {
            Some(p) => p.clone(),
            None => Self::default_worker_bin()?,
        };
        if !worker_bin.exists() {
            return Err(LokError::new(
                ErrorCode::RuntimeNotFound,
                format!(
                    "worker 二进制不存在：{worker_bin:?}（开发期先 \
                     `cargo build -p mdc-lok`；测试经 CARGO_BIN_EXE 注入）"
                ),
            ));
        }
        // 独立 profile：每次 spawn 唯一目录，杀死的会话不留锁。
        let profile_root = std::env::temp_dir().join(format!(
            "mdc-lok-profile-{}-{}",
            std::process::id(),
            now_nanos()
        ));
        std::fs::create_dir_all(&profile_root)
            .map_err(|e| LokError::new(ErrorCode::Io, format!("profile 目录创建失败：{e}")))?;
        let tmp_dir = profile_root.join("tmp");
        std::fs::create_dir_all(&tmp_dir).ok();

        let mut cmd = Command::new(&worker_bin);
        cmd.arg(
            program_dir
                .as_ref()
                .map(|p| p.as_os_str().to_string_lossy().to_string())
                .unwrap_or_else(|| "-".into()),
        )
        .arg(&profile_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("TMPDIR", &tmp_dir)
        .env("TMP", &tmp_dir)
        .env("TEMP", &tmp_dir);
        if mock {
            cmd.arg("--mock");
            // per-worker 注入（显式清除防宿主环境污染并行会话）。
            if let Some(m) = &cfg.mock_hang_method {
                cmd.env("MDC_LOK_MOCK_HANG", m);
            } else {
                cmd.env_remove("MDC_LOK_MOCK_HANG");
            }
            cmd.env_remove("MDC_LOK_MOCK_SAVE_FAIL");
        } else {
            // unipoll：LOK 不起 lo_startmain 线程，worker 分发线程即
            // 主线程并进入 runLoop，键鼠/UNO 异步事件直连同步分发。
            // 实测不设时主循环可整段沉睡（futex），排队键事件永不被
            // 消费——编辑静默丢失（gates/633.md「异步事件黑洞」实录）。
            // ElyOffice Linux 同款（worker/unipoll.rs）。
            cmd.env("SAL_LOK_OPTIONS", "unipoll");
            cmd.env_remove("MDC_LOK_MOCK_HANG");
            cmd.env_remove("MDC_LOK_MOCK_SAVE_FAIL");
        }
        #[cfg(windows)]
        {
            // CREATE_NO_WINDOW：后台子进程不闪控制台（ElyOffice 同款）。
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().map_err(|e| {
            LokError::new(
                ErrorCode::Io,
                format!("worker spawn 失败 {worker_bin:?}：{e}"),
            )
        })?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        let shared = Arc::new(Shared {
            pending: Mutex::new(HashMap::new()),
            alive: AtomicBool::new(true),
            stderr_tail: Mutex::new(Vec::new()),
        });

        // stdout 读线程：按帧类别分流（响应→pending 通道；事件→事件
        // 通道；Log/Request 帧当前无发送方，读到即丢弃）。
        let (events_tx, events_rx) = std::sync::mpsc::channel();
        {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    match read_frame(&mut reader) {
                        Ok(Some(frame)) => match frame.kind {
                            FrameKind::Response => {
                                let mut pending = shared.pending.lock().unwrap();
                                if let Some(tx) = pending.remove(&frame.id) {
                                    let _ = tx.send(frame);
                                }
                                // 已超时被移除的响应：丢弃（宿主已报超时）。
                            }
                            FrameKind::Event => {
                                let msg: Value =
                                    serde_json::from_slice(&frame.json).unwrap_or(Value::Null);
                                let _ = events_tx.send(LokEventMsg {
                                    kind: msg.get("type").and_then(|v| v.as_i64()).unwrap_or(-1)
                                        as i32,
                                    name: msg
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("unknown")
                                        .to_string(),
                                    payload: msg
                                        .get("payload")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string(),
                                    rev: msg.get("rev").and_then(|v| v.as_u64()).unwrap_or(0),
                                });
                            }
                            FrameKind::Log | FrameKind::Request => {}
                        },
                        Ok(None) | Err(_) => {
                            // worker 死亡/坏帧：在途请求全部失败，事件通道关闭。
                            shared.alive.store(false, Ordering::Release);
                            shared.pending.lock().unwrap().clear();
                            break;
                        }
                    }
                }
            });
        }
        // stderr 取证线程：保留尾部行（崩溃原因随 WorkerDied 报出）。
        {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                let mut stderr = stderr;
                let mut buf = String::new();
                let _ = stderr.read_to_string(&mut buf);
                let mut tail = shared.stderr_tail.lock().unwrap();
                for line in buf.lines() {
                    if tail.len() == STDERR_TAIL_LINES {
                        tail.remove(0);
                    }
                    tail.push(line.to_string());
                }
            });
        }

        Ok(LokWorker {
            child,
            stdin,
            next_id: AtomicU64::new(1),
            shared,
            events_rx,
        })
    }

    fn default_worker_bin() -> LokResult<PathBuf> {
        let exe = std::env::current_exe()
            .map_err(|e| LokError::new(ErrorCode::Io, format!("定位当前可执行文件失败：{e}")))?;
        let dir = exe
            .parent()
            .ok_or_else(|| LokError::new(ErrorCode::RuntimeNotFound, "无法定位可执行文件目录"))?;
        let name = if cfg!(windows) {
            "mdc-lok-worker.exe"
        } else {
            "mdc-lok-worker"
        };
        Ok(dir.join(name))
    }

    pub fn is_alive(&self) -> bool {
        // 读线程在 stdout EOF 时清位（进程退出必然关 fd，覆盖崩溃/被杀
        // 全路径）；不在此 try_wait（需 &mut，且在途请求语义已由
        // EOF 处理器覆盖）。
        self.shared.alive.load(Ordering::Acquire)
    }

    /// worker 初始化元数据（runtime/version/库路径/nSize）。
    pub fn hello(&mut self) -> LokResult<Value> {
        let (resp, _) = self.call("hello", &Value::Null, Vec::new(), DEFAULT_TIMEOUT)?;
        Ok(resp)
    }

    /// 发起请求并等待响应（超时→RequestTimeout；worker 死亡→WorkerDied
    /// 附 stderr 尾部）。
    pub fn call(
        &mut self,
        method: &str,
        params: &Value,
        bin: Vec<u8>,
        timeout: Duration,
    ) -> LokResult<(Value, Vec<u8>)> {
        if !self.is_alive() {
            return Err(LokError::new(
                ErrorCode::WorkerDied,
                format!("worker 已终止：{}", self.stderr_text()),
            ));
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = FrameBuf::from_json_value(
            FrameKind::Request,
            id,
            &json!({"method": method, "params": params}),
            bin,
        );
        // **先登记后写**：worker（尤其 mock）可在微秒级返回响应——
        // 若先写后登记，读线程会在 pending 里找不到 id 而把响应丢弃
        // → 假超时（并行测试负载下实测复现：post_key 15s 超时）。
        let (tx, rx) = std::sync::mpsc::channel();
        self.shared.pending.lock().unwrap().insert(id, tx);
        {
            let mut out = std::io::BufWriter::new(&mut self.stdin);
            if let Err(e) = write_frame(&mut out, &req) {
                self.shared.pending.lock().unwrap().remove(&id);
                return Err(e.into());
            }
        }

        let deadline = Instant::now() + timeout;
        let frame = loop {
            let remain = deadline.saturating_duration_since(Instant::now());
            if remain.is_zero() {
                // 超时：移除 pending（晚到响应被读线程丢弃）。
                self.shared.pending.lock().unwrap().remove(&id);
                return Err(LokError::new(
                    ErrorCode::RequestTimeout,
                    format!(
                        "请求 {method}(#{id}) 超时（{}ms）——worker 可能仍在处理；\
                         宿主应 kill 后重拉（本句柄已标嫌疑）",
                        timeout.as_millis()
                    ),
                ));
            }
            match rx.recv_timeout(remain) {
                Ok(f) => break f,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(LokError::new(
                        ErrorCode::WorkerDied,
                        format!(
                            "worker 在处理 {method}(#{id}) 期间死亡：{}",
                            self.stderr_text()
                        ),
                    ));
                }
            }
        };

        let value: Value = serde_json::from_slice(&frame.json)
            .map_err(|e| LokError::new(ErrorCode::BadFrame, format!("响应 JSON 解析失败：{e}")))?;
        if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            Ok((value, frame.bin))
        } else {
            let code = value
                .pointer("/error/code")
                .and_then(|v| v.as_str())
                .unwrap_or("io");
            let message = value
                .pointer("/error/message")
                .and_then(|v| v.as_str())
                .unwrap_or("worker 返回未知错误");
            Err(LokError::new(
                ErrorCode::from_code(code),
                message.to_string(),
            ))
        }
    }

    /// 尝试收一条事件（非阻塞）。
    pub fn try_recv_event(&self) -> Option<LokEventMsg> {
        self.events_rx.try_recv().ok()
    }

    /// 收集当前积压的全部事件（非阻塞）。
    pub fn drain_events(&self) -> Vec<LokEventMsg> {
        let mut out = Vec::new();
        while let Ok(ev) = self.events_rx.try_recv() {
            out.push(ev);
        }
        out
    }

    pub fn stderr_text(&self) -> String {
        let tail = self.shared.stderr_tail.lock().unwrap();
        let n = tail.len().saturating_sub(5);
        tail[n..].join(" | ")
    }

    /// 杀死 worker（SIGKILL/TerminateProcess 语义）。之后本句柄只能
    /// 报 WorkerDied；宿主应 spawn 新 worker 继续。
    pub fn kill(&mut self) {
        self.shared.alive.store(false, Ordering::Release);
        let _ = self.child.kill();
        self.shared.pending.lock().unwrap().clear();
    }

    /// 优雅关闭（shutdown 帧 + 等待退出；超时强杀）。
    pub fn shutdown(mut self, timeout: Duration) -> LokResult<()> {
        let r = self.call("shutdown", &Value::Null, Vec::new(), timeout);
        match r {
            Ok(_) => {
                let _ = self.child.wait();
                Ok(())
            }
            Err(e) if e.code == ErrorCode::RequestTimeout => {
                self.kill();
                Err(e)
            }
            Err(e) => Err(e),
        }
    }
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

impl Drop for LokWorker {
    fn drop(&mut self) {
        // 兜底：宿主句柄被丢弃时确保子进程不残留（正常 shutdown 已
        // wait 退出，kill 对已退出进程是 no-op；挂起中的 worker 必须
        // 在此被杀——否则测试/宿主会泄漏睡眠进程）。
        self.kill();
    }
}
