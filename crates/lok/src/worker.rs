//! worker 侧运行时：LOK 内核绑定 + 请求分发 + 回调转发。
//!
//! ## 线程模型
//!
//! ### Linux：unipoll 主循环（B1）
//!
//! ```text
//! 宿主 pipe ──stdin──▶ reader 线程（只读字节 → channel）
//!                                          │ Note::Ask/Woken/Closed
//! LO 内核 ──runLoop──▶ 主线程（poll 回调：同步分发请求 = 主循环上下文）
//!                        │
//! LO 内核线程 ──回调──▶ 事件队列 ──▶ 写线程（独占 stdout：应答+事件同串行化）
//! 宿主 pipe ◀──stdout── 写线程
//! ```
//!
//! ### Windows：message-only window 消息驱动（#104，#69 终局）
//!
//! ```text
//! 宿主 pipe ──stdin──▶ reader 线程（只读字节 → PostMessageW）
//!                                          │ ASK(帧指针) / CLOSED(退出码)
//! LO 内核 ──runLoop(空钩子)──▶ 主线程消息泵 ──▶ WndProc：ASK=handle_frame
//!                        │
//! LO 内核线程 ──回调──▶ 事件队列 ──▶ 写线程（独占 stdout：应答+事件同串行化）
//! 宿主 pipe ◀──stdout── 写线程
//! ```
//!
//! **为什么 Windows 换消息驱动**（#69 Windows 实测 + ElyOffice
//! `worker/messages.rs` 实证）：LO Windows 版 `run_loop` 要求调用线程
//! **先持有 message-only window**——无窗口时 runLoop 立即返回
//! （`bye(run-loop-returned)`，worker 随即退出，请求帧无应答）；先
//! `CreateWindowExW(HWND_MESSAGE)` 再进 runLoop，则 runLoop 阻塞运行
//! 成为本线程的消息泵，`PostMessageW` 的请求经 `WndProc` 在主线程
//! 同步分发——与 unipoll 的「所有 LOK 调用串行于主循环」同构。
//! unipoll 契约（`SAL_LOK_OPTIONS=unipoll`）仍要求传 hooks，Windows
//! 消息循环从不调用它们——空实现满足契约（ElyOffice 同款：
//! "Unipoll needs hooks; the Windows loop never calls them"）。
//!
//! **为什么 unipoll**（异步事件黑洞实录，gates/633.md）：LO 的
//! `postKeyEvent/postMouseEvent/postUnoCommand` 均为 `postKeyEventAsync`
//! → `Application::PostUserEvent` 入队（unotxdoc.cxx:4015-4018），消费依赖
//! `lo_startmain` 线程泵主循环；外部宿主场景实测该线程可整段沉睡
//! （wchan=futex，CPU 0.05s/6s 非忙等）→ 排队键鼠/UNO 事件黑洞 →
//! **编辑静默丢失**（smoke 复跑 4 连败：超时/像素不变/保存产物缺文本）。
//! 修复 = `SAL_LOK_OPTIONS=unipoll`（hook_2 前生效）+ 主线程进
//! `runLoop`（poll 回调里消费请求通道 = 内核主循环上下文同步分发；
//! unipoll 下 `postEventAsync` 直调 `LOKPostAsyncEvent`——lokhelper.cxx
//! :1283-1290，同步确定；ElyOffice Linux 同款模型）。只设 env 不进
//! runLoop 会 SIGSEGV（实测 rc=-11）——两者必须配套，缺一不可。
//!
//! 回调在 LO 内核线程触发（ElyOffice 实证）→ 只入队（持锁极短），由写线程
//! 单点写帧，避免应答/事件交错。写线程管道断裂（宿主已死）→ worker 硬退出
//! （孤儿清理）。
//!
//! **fd 0 抢救**（`steal_stdin_fd`）：LO 内嵌组件（python 等）可能直接读
//! fd 0 与 reader 线程竞争读者、撕碎帧流 → dup 保管道、fd 0 换空设备
//! （本仓 Linux 实测复现：连续 post_key 中途偶发挂起；ElyOffice Windows
//! 同款教训）。
//!
//! ## 环境卫生（ElyOffice 实证教训，spawn 前由宿主与 worker 双侧设置）
//!
//! - `SAL_NOLOCK_PROFILE=1`（被杀 worker 的 profile 锁会挂死下一个）
//! - `SAL_DISABLE_OPENCL=1`（驱动自测挂死）
//! - Linux `SAL_DISABLESKIA=1`（cairo tile = BGRA 保序；Skia 是 RGBA）
//! - Windows `SAL_SKIA=raster`（同因）
//!
//! ## 退出纪律
//!
//! 一律 `_exit()`（unix）/ `ExitProcess`（Windows）：跳过 LO 的 atexit
//! 处理器（ElyOffice 实证：正常 exit 会走 LO 清理，慢且可能死锁）。

use crate::abi::{
    self, callbacks, DocType, LokDocClass, LokDocument, LokHook2, LokKit, LokKitClass,
};
use crate::bmp;
use crate::ffi::{cstr_take, Library};
use crate::profile::SessionProfile;
use crate::protocol::{self, codes};
use crate::runtime::LokRuntime;
use crate::sha256::sha256_hex;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::ffi::CString;
use std::io::{BufReader, Write};
use std::os::raw::{c_char, c_int, c_long, c_void};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

/// worker 内部错误（分发层用，转协议错误码）。
type DispatchResult = Result<Value, (&'static str, String)>;

/// 已绑定内核 + 当前文档（单文档 worker：issue「单文档 worker」）。
pub struct WorkerState {
    runtime: LokRuntime,
    _lib: Library, // 保活：hook 指针指向其内（进程级唯一，永不卸载）
    kit: *mut LokKit,
    kit_class: *const LokKitClass,
    doc: *mut LokDocument,
    doc_class: *const LokDocClass,
    profile: SessionProfile,
    session_dir: PathBuf,
    tile_counter: u64,
    loaded_url: String,
    // #69：getVersionInfo 首调结果缓存。LO 26.x 实测：getVersionInfo 返回
    // 内部串指针，freeError 释放后二调返回 NULL 且内核状态受损（后续
    // documentLoad 时 worker 静默死亡）。缓存 = 只调一次，规避二调。
    version_cache: String,
}

unsafe impl Send for WorkerState {}
unsafe impl Sync for WorkerState {}

/// 写线程消息（应答与事件同通道单点串行化）。
pub enum ToWriter {
    Frame(Value),
    /// 写失败（宿主管道断裂）→ worker 硬退出。
    Quit,
}

/// worker 启动环境参数（宿主传入）。
pub struct WorkerBoot {
    pub runtime: LokRuntime,
    pub session_dir: PathBuf,
}

impl WorkerBoot {
    pub fn new(runtime: LokRuntime, session_dir: &Path) -> WorkerBoot {
        WorkerBoot {
            runtime,
            session_dir: session_dir.to_path_buf(),
        }
    }
}

/// worker 主入口（bin 调用）。
///
/// `fake` = Some(mode) 时走协议自测分发（无 LOK；mode ∈ echo|hang|die），
/// 供 client/protocol 集成测试在无运行时环境恒跑。
pub fn worker_main(boot: Option<WorkerBoot>, fake: Option<&str>) -> ! {
    // 环境卫生（双侧：宿主 spawn 时也设过；幂等）
    env_hygiene();
    let protocol_fd = steal_stdin_fd();
    #[cfg(windows)]
    windows_worker_preamble();

    let (tx, rx) = std::sync::mpsc::channel::<ToWriter>();
    let writer = std::thread::Builder::new()
        .name("lok-writer".into())
        .spawn(move || writer_loop(rx))
        .expect("spawn writer");
    let _ = writer;

    let hello = protocol::life_event_frame(
        "hello",
        json!({
            "pid": std::process::id(),
            "fake": fake,
            "max_frame_bytes": crate::MAX_FRAME_BYTES,
        }),
    );
    let _ = tx.send(ToWriter::Frame(hello));

    match fake {
        Some(mode) => fake_dispatch_loop(mode, &tx, protocol_fd),
        None => {
            let boot = boot.expect("真实 worker 需要 WorkerBoot（--session-dir）");
            let state = match bind_kernel(&boot, &tx) {
                Ok(st) => st,
                Err((code, msg)) => {
                    // 绑定失败：发 bye 事件 + 保留请求循环（每个 LOK 请求回
                    // 明确错误，不静默死——issue「明确错误」）
                    let _ = tx.send(ToWriter::Frame(protocol::life_event_frame(
                        "boot-failed",
                        json!({"code": code, "message": msg}),
                    )));
                    no_kernel_loop(protocol_fd, &tx, code, msg);
                }
            };
            // 平台路由（模块头「线程模型」两节）：Windows 走 message-only
            // window 消息驱动（#104），其余平台走 unipoll 主循环（B1）。
            #[cfg(windows)]
            {
                messages_loop(state, &tx, protocol_fd)
            }
            #[cfg(not(windows))]
            {
                unipoll_loop(state, &tx, protocol_fd)
            }
        }
    }
}

/// 写线程：独占 stdout，帧写管道断裂 → 硬退出（宿主已死）。
fn writer_loop(rx: Receiver<ToWriter>) -> ! {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    loop {
        match rx.recv() {
            Ok(ToWriter::Frame(v)) => {
                if protocol::write_frame(&mut out, &v).is_err() {
                    hard_exit(4); // 宿主管道断：孤儿 worker 清理
                }
            }
            Ok(ToWriter::Quit) | Err(_) => hard_exit(0),
        }
    }
}

/// 硬退出：跳过 LO atexit（模块头「退出纪律」）。
pub fn hard_exit(code: i32) -> ! {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    #[cfg(unix)]
    {
        extern "C" {
            fn _exit(status: c_int) -> !;
        }
        unsafe { _exit(code) }
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn ExitProcess(exit_code: u32);
        }
        unsafe { ExitProcess(code as u32) }
        unreachable!("ExitProcess 不会返回")
    }
}

/// 环境卫生（见模块头）。
fn env_hygiene() {
    unsafe {
        std::env::set_var("SAL_NOLOCK_PROFILE", "1");
        std::env::set_var("SAL_DISABLE_OPENCL", "1");
        // unipoll 主循环契约（模块头「为什么 unipoll」）：必须在 hook_2
        // 之前生效；宿主 spawn 侧也设（client.rs，幂等双保险）。
        std::env::set_var("SAL_LOK_OPTIONS", "unipoll");
        if cfg!(windows) {
            std::env::set_var("SAL_SKIA", "raster");
        } else {
            std::env::set_var("SAL_DISABLESKIA", "1");
        }
    }
}

/// Windows 专用前置（**实测门未执行**——如实注明；ElyOffice 实证教训）：
/// ① stdin 私有化（LO 内核的 Lightproof.py 会 seek/读 fd 0，吞掉 IPC 流；
///    本原型把标准输入句柄重定向到 NUL 设备，IPC 走 dup 保留下来的管道）；
/// ② 线程 DPI unaware（tile 假定 96 DPI，per-monitor 缩放会画歪）。
#[cfg(windows)]
fn windows_worker_preamble() {
    use std::os::raw::c_void;
    type HANDLE = *mut c_void;
    type DWORD = u32;
    type BOOL = i32;
    #[link(name = "kernel32")]
    extern "system" {
        fn SetStdHandle(nStdHandle: DWORD, hHandle: HANDLE) -> BOOL;
        fn CreateFileA(
            lpFileName: *const u8,
            dwDesiredAccess: DWORD,
            dwShareMode: DWORD,
            lpSecurityAttributes: *mut c_void,
            dwCreationDisposition: DWORD,
            dwFlagsAndAttributes: DWORD,
            hTemplateFile: HANDLE,
        ) -> HANDLE;
    }
    const STD_INPUT_HANDLE: DWORD = 0xFFFF_FFF6u32;
    const GENERIC_READ: DWORD = 0x8000_0000;
    const OPEN_EXISTING: DWORD = 3;
    unsafe {
        // ① 标准输入句柄 → NUL（IPC 从本函数之前已就绪的管道句柄读取；
        //    完整 DuplicateHandle 私有化在 Windows 实测门补全，见 gates/633.md）
        let nul = CreateFileA(
            b"NUL\0".as_ptr(),
            GENERIC_READ,
            0x1 | 0x2,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            0x80,
            std::ptr::null_mut(),
        );
        if !nul.is_null() {
            SetStdHandle(STD_INPUT_HANDLE, nul);
        }
    }
    // ② DPI unaware（DPI_AWARENESS_CONTEXT_UNAWARE = -1）
    #[link(name = "user32")]
    extern "system" {
        fn SetThreadDpiAwarenessContext(value: isize) -> isize;
    }
    unsafe {
        SetThreadDpiAwarenessContext(-1);
    }
}

/// kit 级回调（LO 内核线程调用；只入队）。
unsafe extern "C" fn kit_callback(n_type: c_int, payload: *const c_char, data: *mut c_void) {
    let tx = &*(data as *const Sender<ToWriter>);
    let name = callbacks::name(n_type).unwrap_or("OTHER");
    let payload = if payload.is_null() {
        String::new()
    } else {
        std::ffi::CStr::from_ptr(payload)
            .to_string_lossy()
            .into_owned()
    };
    let _ = tx.send(ToWriter::Frame(protocol::cb_event_frame(
        n_type, name, &payload,
    )));
}

/// 绑定内核（worker 进程内；宿主永不执行——见 runtime 模块「隔离铁律」）。
fn bind_kernel(
    boot: &WorkerBoot,
    _tx: &Sender<ToWriter>,
) -> Result<WorkerState, (&'static str, String)> {
    // profile 先建（hook_2 第二参契约）
    let profile = SessionProfile::new(&boot.session_dir)
        .map_err(|e| (codes::IO, format!("建会话 profile 目录失败：{e}")))?;

    let lib = Library::open(&boot.runtime.kernel_lib)
        .map_err(|e| (codes::LOK_LOAD, format!("dlopen 内核库失败：{e}")))?;
    let hook_ptr = lib.sym("libreofficekit_hook_2").map_err(|e| {
        (
            codes::LOK_LOAD,
            format!("内核库缺 libreofficekit_hook_2：{e}"),
        )
    })?;
    let hook: LokHook2 = unsafe { std::mem::transmute(hook_ptr) };

    let install_c = CString::new(boot.runtime.program_dir.to_string_lossy().as_bytes())
        .map_err(|e| (codes::LOK_LOAD, format!("program 路径含 NUL：{e}")))?;
    let profile_url = profile.file_url();
    let profile_c = CString::new(profile_url.as_bytes())
        .map_err(|e| (codes::LOK_LOAD, format!("profile URL 含 NUL：{e}")))?;

    // hook_2：LO 静态初始化全在这里（数秒级，正常）。install_path 契约
    // = **program 目录**（传安装根会 bootstrap 失败——见 runtime 模块头
    // 「install_path 契约」的实测链）。
    let kit = unsafe { hook(install_c.as_ptr(), profile_c.as_ptr()) };
    if kit.is_null() {
        return Err((
            codes::LOK_INIT,
            format!(
                "libreofficekit_hook_2 返回 NULL（LO 初始化失败）。program={:?} \
                 profile={profile_url}；常见原因：install_path 不是 program 目录（BRAND_BASE_DIR \
                 错指上一级→theDefaultProvider bootstrap 异常）/ 运行时不完整 / profile 目录不可写。",
                boot.runtime.program_dir
            ),
        ));
    }
    let kit_class = unsafe { (*kit).p_class };
    if kit_class.is_null() {
        return Err((codes::LOK_INIT, "LOK Kit 类结构指针为空".into()));
    }

    let mut state = WorkerState {
        runtime: boot.runtime.clone(),
        _lib: lib,
        kit,
        kit_class,
        doc: std::ptr::null_mut(),
        doc_class: std::ptr::null_mut(),
        profile,
        session_dir: boot.session_dir.clone(),
        tile_counter: 0,
        loaded_url: String::new(),
        version_cache: String::new(),
    };

    // nSize 成员守卫（issue「nSize/member 检查」）：绑定面全量检查
    let n = unsafe { (*kit_class).n_size };
    let need = [
        (abi::kit_offsets::DESTROY, "destroy"),
        (
            abi::kit_offsets::DOCUMENT_LOAD_WITH_OPTIONS,
            "documentLoadWithOptions",
        ),
        (abi::kit_offsets::GET_ERROR, "getError"),
        (abi::kit_offsets::FREE_ERROR, "freeError"),
        (abi::kit_offsets::REGISTER_CALLBACK, "registerCallback"),
        (
            abi::kit_offsets::SET_OPTIONAL_FEATURES,
            "setOptionalFeatures",
        ),
        (abi::kit_offsets::GET_VERSION_INFO, "getVersionInfo"),
        // B1：unipoll 主循环的硬依赖（缺槽 = 本 worker 不支持该运行时，
        // boot-failed 明确报错——不回退直调模式：直调有异步事件黑洞，
        // 产品不可用，宁可明确失败）。
        (abi::kit_offsets::RUN_LOOP, "runLoop"),
    ];
    for (off, name) in need {
        if !abi::has_member(n, off, std::mem::size_of::<usize>()) {
            return Err((
                codes::MEMBER,
                format!("Kit 类 nSize={n} 缺成员 {name}（offset={off}）——运行时过旧，不支持 LOK 原型所需面"),
            ));
        }
    }

    // 版本透传（25.2 文本 / 26.x JSON，原文返回）
    let ver = state.kit_version_info();
    let _ = _tx.send(ToWriter::Frame(protocol::life_event_frame(
        "ready",
        json!({
            "version_info": ver,
            "kernel_lib": state.runtime.kernel_lib.to_string_lossy(),
            "kernel_sha256": state.runtime.kernel_sha256,
            "profile_dir": state.profile.dir().to_string_lossy(),
        }),
    )));

    // kit 级回调（密码请求 / 内核级错误事件）
    // Sender 泄漏进原始指针：回调可能活到进程退出（ElyOffice 同款「故意泄漏」）
    let tx_boxed: &'static Sender<ToWriter> = Box::leak(Box::new(_tx.clone()));
    unsafe {
        if let Some(f) = (*kit_class).register_callback {
            f(kit, kit_callback, tx_boxed as *const _ as *mut c_void);
        }
        if let Some(f) = (*kit_class).set_optional_features {
            f(kit, abi::FEATURE_PART_IN_INVALIDATION_CALLBACK);
        }
    }
    Ok(state)
}

impl WorkerState {
    /// kit getError 原文（用 freeError 释放——绝不用 Rust free，跨 CRT 释放
    /// 会崩，官方头文件明文警告）。
    unsafe fn kit_error_text(&self) -> Option<String> {
        let get = (*self.kit_class).get_error?;
        let free = (*self.kit_class).free_error?;
        let e = get(self.kit);
        if e.is_null() {
            return None;
        }
        let s = cstr_take(e).unwrap_or_default();
        free(e);
        Some(s)
    }

    /// getVersionInfo 原文（首调缓存；见 WorkerState.version_cache）。
    fn kit_version_info(&mut self) -> String {
        if !self.version_cache.is_empty() {
            return self.version_cache.clone();
        }
        let s = unsafe {
            let f = match (*self.kit_class).get_version_info {
                Some(f) => f,
                None => return String::new(),
            };
            let p = f(self.kit);
            if p.is_null() {
                return String::new();
            }
            let s = cstr_take(p).unwrap_or_default();
            if let Some(free) = (*self.kit_class).free_error {
                free(p);
            }
            s
        };
        self.version_cache = s;
        self.version_cache.clone()
    }

    /// doc 成员守卫。
    fn doc_guard(&self, offset: usize, name: &str) -> Result<(), (&'static str, String)> {
        if self.doc.is_null() {
            return Err((codes::NO_DOCUMENT, "无打开文档（先 load）".into()));
        }
        let n = unsafe { (*self.doc_class).n_size };
        if !abi::has_member(n, offset, std::mem::size_of::<usize>()) {
            return Err((
                codes::MEMBER,
                format!("Doc 类 nSize={n} 缺成员 {name}（offset={offset}）——运行时过旧"),
            ));
        }
        Ok(())
    }

    /// documentLoadWithOptions。
    fn op_load(&mut self, url: &str, options: Option<&str>) -> DispatchResult {
        if !self.doc.is_null() {
            return Err((
                codes::INTERNAL,
                "单文档 worker：已有打开文档（先 close）".into(),
            ));
        }
        unsafe {
            // kit 级成员守卫（不是 doc_guard：此处文档尚未打开）
            let n = (*self.kit_class).n_size;
            if !abi::has_member(
                n,
                abi::kit_offsets::DOCUMENT_LOAD_WITH_OPTIONS,
                std::mem::size_of::<usize>(),
            ) {
                return Err((
                    codes::MEMBER,
                    format!("Kit 类 nSize={n} 缺 documentLoadWithOptions——运行时过旧"),
                ));
            }
            let f = (*self.kit_class)
                .document_load_with_options
                .ok_or((codes::MEMBER, "缺 documentLoadWithOptions".to_string()))?;
            let url_c = CString::new(url).map_err(|_| (codes::BAD_REQUEST, "url 含 NUL".into()))?;
            // CString 必须活到调用结束：绝不能写 `opt.map(|c| c.as_ptr())`
            // （map 表达式结束 CString 即 drop，传给 LOK 的是悬垂指针——
            // 2026-11-09 实测事故：垃圾 JSON 让 boost ptree 拗 C++ 异常，
            // 穿透 Rust 帧直接 abort）。
            let opt_c: Option<CString> = match options {
                Some(o) => Some(
                    CString::new(o).map_err(|_| (codes::BAD_REQUEST, "options 含 NUL".into()))?,
                ),
                None => None,
            };
            let opt_ptr = opt_c
                .as_ref()
                .map(|c| c.as_ptr())
                .unwrap_or(std::ptr::null());
            let doc = f(self.kit, url_c.as_ptr(), opt_ptr);
            if doc.is_null() {
                let why = self.kit_error_text().unwrap_or_default();
                return Err((
                    codes::LOAD,
                    format!("documentLoad 失败：{why}（url={url}）"),
                ));
            }
            self.doc = doc;
            self.doc_class = (*doc).p_class;
            self.loaded_url = url.to_owned();
        }
        // 加载即注册 doc 级回调（失效/搜索结果/UNO 结果事件持续透传）
        unsafe {
            self.doc_guard(abi::doc_offsets::REGISTER_CALLBACK, "registerCallback")?;
            let tx_boxed: &'static Sender<ToWriter> =
                Box::leak(Box::new(WRITER_TX.with(|t| t.borrow().clone()).unwrap()));
            if let Some(f) = (*self.doc_class).register_callback {
                f(self.doc, kit_callback, tx_boxed as *const _ as *mut c_void);
            }
        }
        let doc_type = self.doc_type_raw();
        let parts = self.doc_parts_raw();
        Ok(json!({
            "url": url,
            "doc_type": doc_type,
            "doc_type_name": format!("{:?}", DocType::from_raw(doc_type)),
            "parts": parts,
        }))
    }

    fn doc_type_raw(&self) -> c_int {
        unsafe {
            if self.doc.is_null() {
                return -1;
            }
            if !abi::has_member(
                (*self.doc_class).n_size,
                abi::doc_offsets::GET_DOCUMENT_TYPE,
                std::mem::size_of::<usize>(),
            ) {
                return -1;
            }
            (*self.doc_class)
                .get_document_type
                .map(|f| f(self.doc))
                .unwrap_or(-1)
        }
    }

    fn doc_parts_raw(&self) -> c_int {
        unsafe {
            if self.doc.is_null() {
                return -1;
            }
            if !abi::has_member(
                (*self.doc_class).n_size,
                abi::doc_offsets::GET_PARTS,
                std::mem::size_of::<usize>(),
            ) {
                return -1;
            }
            (*self.doc_class)
                .get_parts
                .map(|f| f(self.doc))
                .unwrap_or(-1)
        }
    }

    /// initializeForRendering（paint 前必须；ElyOffice 顺序：init → 回调 → paint）。
    fn op_init_rendering(&mut self, args: Option<&str>) -> DispatchResult {
        unsafe {
            self.doc_guard(
                abi::doc_offsets::INITIALIZE_FOR_RENDERING,
                "initializeForRendering",
            )?;
            let f = (*self.doc_class)
                .initialize_for_rendering
                .ok_or((codes::MEMBER, "缺 initializeForRendering".to_string()))?;
            let a_c: Option<CString> = match args {
                Some(s) => {
                    Some(CString::new(s).map_err(|_| (codes::BAD_REQUEST, "args 含 NUL".into()))?)
                }
                None => None,
            };
            let a_ptr = a_c.as_ref().map(|c| c.as_ptr()).unwrap_or(std::ptr::null());
            f(self.doc, a_ptr);
        }
        Ok(json!({"initialized": true}))
    }

    /// getDocumentSize（twips）。
    fn op_size(&mut self) -> DispatchResult {
        let (mut w, mut h): (c_long, c_long) = (0, 0);
        unsafe {
            self.doc_guard(abi::doc_offsets::GET_DOCUMENT_SIZE, "getDocumentSize")?;
            let f = (*self.doc_class)
                .get_document_size
                .ok_or((codes::MEMBER, "缺 getDocumentSize".to_string()))?;
            f(self.doc, &mut w as *mut c_long, &mut h as *mut c_long);
        }
        Ok(json!({"width_twips": w, "height_twips": h}))
    }

    /// getPartPageRectangles → 页矩形列表（twips；跳页/页码/页间分隔
    /// 数据源——#50 C 线 2026-11-10 协议缺口登记的应答）。
    /// 返回串按 kit freeError 释放（跨 CRT 纪律，同 text_selection）；
    /// 格式解析见 [`parse_page_rectangles`]（25.2.3.2 实测）。
    fn op_page_geometry(&mut self) -> DispatchResult {
        let raw: *mut c_char;
        unsafe {
            self.doc_guard(
                abi::doc_offsets::GET_PART_PAGE_RECTANGLES,
                "getPartPageRectangles",
            )?;
            let f = (*self.doc_class)
                .get_part_page_rectangles
                .ok_or((codes::MEMBER, "缺 getPartPageRectangles".to_string()))?;
            raw = f(self.doc);
        }
        let s = unsafe { cstr_take(raw) }.unwrap_or_default();
        // LOK 返回内存必须 kit freeError 释放（跨 CRT 纪律，同 text_selection）
        unsafe {
            if !raw.is_null() {
                if let Some(free) = (*self.kit_class).free_error {
                    free(raw);
                }
            }
        }
        if s.is_empty() {
            return Err((
                codes::INTERNAL,
                "内核返回空页矩形串（getPartPageRectangles；非分页文档或内核异常）".to_string(),
            ));
        }
        let rects = parse_page_rectangles(&s).map_err(|e| {
            let head: String = s.chars().take(120).collect();
            (
                codes::INTERNAL,
                format!("页矩形串解析失败：{e}（原文前 120 字节：{head}）"),
            )
        })?;
        Ok(json!({
            "pages": rects.iter().map(|r| json!({
                "x": r[0], "y": r[1], "w": r[2], "h": r[3],
            })).collect::<Vec<_>>(),
            "page_count": rects.len(),
            "units": "twips",
        }))
    }

    /// paintTile → side file（BMP）+ 统计（帧内不传像素——帧长纪律）。
    fn op_paint(
        &mut self,
        canvas_w: usize,
        canvas_h: usize,
        tile_x: i32,
        tile_y: i32,
        tile_w: i32,
        tile_h: i32,
    ) -> DispatchResult {
        const MAX_CANVAS: usize = 2048;
        if canvas_w == 0 || canvas_h == 0 || canvas_w > MAX_CANVAS || canvas_h > MAX_CANVAS {
            return Err((
                codes::BAD_REQUEST,
                format!("canvas 越界（1..={MAX_CANVAS}）：{canvas_w}x{canvas_h}"),
            ));
        }
        if tile_w <= 0 || tile_h <= 0 {
            return Err((
                codes::BAD_REQUEST,
                format!("tile 尺寸必须为正：{tile_w}x{tile_h}"),
            ));
        }
        // 哨兵填充：0xAB——paint 后残留哨兵即内核未写满（诊断用）
        let mut buf = vec![0xABu8; canvas_w * canvas_h * 4];
        unsafe {
            self.doc_guard(abi::doc_offsets::PAINT_TILE, "paintTile")?;
            let f = (*self.doc_class)
                .paint_tile
                .ok_or((codes::MEMBER, "缺 paintTile".to_string()))?;
            f(
                self.doc,
                buf.as_mut_ptr(),
                canvas_w as c_int,
                canvas_h as c_int,
                tile_x,
                tile_y,
                tile_w,
                tile_h,
            );
        }
        let stats = bmp::stats_bgra(&buf, canvas_w, canvas_h);
        self.tile_counter += 1;
        let path = self
            .session_dir
            .join(format!("tile-{:03}.bmp", self.tile_counter));
        bmp::write_bmp(&path, &buf, canvas_w, canvas_h)
            .map_err(|e| (codes::IO, format!("tile side file 写失败：{e}")))?;
        let sha = sha256_hex(&buf);
        Ok(json!({
            "tile_path": path.to_string_lossy(),
            "canvas": [canvas_w, canvas_h],
            "tile_twips": [tile_x, tile_y, tile_w, tile_h],
            "sha256_bgra": sha,
            "stats": {
                "unique_colors": stats.unique_colors,
                "non_white_ratio": stats.non_white_ratio,
                "colored_ratio": stats.colored_ratio,
            },
        }))
    }

    /// postKeyEvent（KEYINPUT+KEYUP 成对，自动替换选区——标准编辑语义）。
    fn op_post_key(&mut self, ch: i32, key_code: i32) -> DispatchResult {
        unsafe {
            self.doc_guard(abi::doc_offsets::POST_KEY_EVENT, "postKeyEvent")?;
            let f = (*self.doc_class)
                .post_key_event
                .ok_or((codes::MEMBER, "缺 postKeyEvent".to_string()))?;
            f(self.doc, abi::KEYEVENT_KEYINPUT, ch, key_code);
            f(self.doc, abi::KEYEVENT_KEYUP, ch, key_code);
        }
        Ok(json!({"key": ch}))
    }

    /// postWindowExtTextInputEvent（槽 #40；IME 组合/提交流——
    /// #50 C3 中文输入数据源，与 A 线批次 12 GPUI 侧 IME 卫对接）。
    /// commit=false → LOK_EXT_TEXTINPUT(0) 预编辑串；commit=true →
    /// LOK_EXT_TEXTINPUT_END(2) 定稿串。
    fn op_post_text(&mut self, commit: bool, text: &str) -> DispatchResult {
        unsafe {
            self.doc_guard(
                abi::doc_offsets::POST_WINDOW_EXT_TEXT_INPUT_EVENT,
                "postWindowExtTextInputEvent",
            )?;
            let f = (*self.doc_class)
                .post_window_ext_text_input_event
                .ok_or((codes::MEMBER, "缺 postWindowExtTextInputEvent".to_string()))?;
            let text_c =
                CString::new(text).map_err(|_| (codes::BAD_REQUEST, "text 含 NUL".into()))?;
            // CString 生命周期纪律：具名绑定活到调用结束（悬垂指针事故教训）
            let n_type = if commit {
                abi::EXT_TEXTINPUT_END
            } else {
                abi::EXT_TEXTINPUT
            };
            f(self.doc, abi::MAIN_WINDOW_ID, n_type, text_c.as_ptr());
        }
        Ok(json!({"commit": commit, "text": text}))
    }

    /// postMouseEvent。
    #[allow(clippy::too_many_arguments)]
    fn op_post_mouse(
        &mut self,
        n_type: i32,
        x: i32,
        y: i32,
        count: i32,
        buttons: i32,
        modifier: i32,
    ) -> DispatchResult {
        unsafe {
            self.doc_guard(abi::doc_offsets::POST_MOUSE_EVENT, "postMouseEvent")?;
            let f = (*self.doc_class)
                .post_mouse_event
                .ok_or((codes::MEMBER, "缺 postMouseEvent".to_string()))?;
            f(self.doc, n_type, x, y, count, buttons, modifier);
        }
        Ok(json!({"mouse": [n_type, x, y, count, buttons, modifier]}))
    }

    /// postUnoCommand（args 为 JSON 字符串——LO UNO 属性包格式）。
    fn op_uno(&mut self, command: &str, args: Option<&str>, notify: bool) -> DispatchResult {
        unsafe {
            self.doc_guard(abi::doc_offsets::POST_UNO_COMMAND, "postUnoCommand")?;
            let f = (*self.doc_class)
                .post_uno_command
                .ok_or((codes::MEMBER, "缺 postUnoCommand".to_string()))?;
            let cmd_c =
                CString::new(command).map_err(|_| (codes::BAD_REQUEST, "command 含 NUL".into()))?;
            // CString 生命周期同上（悬垂指针事故教训）
            let a_c: Option<CString> = match args {
                Some(s) => {
                    Some(CString::new(s).map_err(|_| (codes::BAD_REQUEST, "args 含 NUL".into()))?)
                }
                None => None,
            };
            let a_ptr = a_c.as_ref().map(|c| c.as_ptr()).unwrap_or(std::ptr::null());
            f(self.doc, cmd_c.as_ptr(), a_ptr, notify);
        }
        Ok(json!({"command": command}))
    }

    /// getTextSelection（应答超帧限 → E_OVERSIZE，worker 存活）。
    fn op_text_selection(&mut self, mime: &str) -> DispatchResult {
        let mut used: *mut c_char = std::ptr::null_mut();
        let text: *mut c_char;
        unsafe {
            self.doc_guard(abi::doc_offsets::GET_TEXT_SELECTION, "getTextSelection")?;
            let f = (*self.doc_class)
                .get_text_selection
                .ok_or((codes::MEMBER, "缺 getTextSelection".to_string()))?;
            let mime_c =
                CString::new(mime).map_err(|_| (codes::BAD_REQUEST, "mime 含 NUL".into()))?;
            text = f(self.doc, mime_c.as_ptr(), &mut used);
        }
        let body = unsafe { cstr_take(text) }.unwrap_or_default();
        let used_mime = unsafe { cstr_take(used) }.unwrap_or_else(|| mime.to_string());
        // LOK 返回的内存必须 kit freeError 释放（跨 CRT 纪律）
        unsafe {
            if !text.is_null() {
                if let Some(free) = (*self.kit_class).free_error {
                    free(text);
                }
            }
            if !used.is_null() {
                if let Some(free) = (*self.kit_class).free_error {
                    free(used);
                }
            }
        }
        let v = json!({"mime": used_mime, "text": body});
        if serde_json::to_vec(&v)
            .map(|b| b.len())
            .unwrap_or(usize::MAX)
            > crate::MAX_FRAME_BYTES
        {
            return Err((
                codes::OVERSIZE,
                format!(
                    "getTextSelection({mime}) 应答超 {} 字节（缩小选区后重试）",
                    crate::MAX_FRAME_BYTES
                ),
            ));
        }
        Ok(v)
    }

    /// saveAs（格式如 "docx"；失败附 kit getError 原文，**不清宿主脏位**——
    /// 脏位语义在宿主侧，本 op 只如实报成败）。
    fn op_save_as(
        &mut self,
        url: &str,
        format: &str,
        filter_options: Option<&str>,
    ) -> DispatchResult {
        unsafe {
            self.doc_guard(abi::doc_offsets::SAVE_AS, "saveAs")?;
            crate::save_guard::check(&self.loaded_url, url)
                .map_err(|message| (codes::SAVE, message))?;
            let f = (*self.doc_class)
                .save_as
                .ok_or((codes::MEMBER, "缺 saveAs".to_string()))?;
            let url_c = CString::new(url).map_err(|_| (codes::BAD_REQUEST, "url 含 NUL".into()))?;
            let fmt_c =
                CString::new(format).map_err(|_| (codes::BAD_REQUEST, "format 含 NUL".into()))?;
            // CString 生命周期同上（悬垂指针事故教训）
            let fo_c: Option<CString> = match filter_options {
                Some(s) => Some(
                    CString::new(s).map_err(|_| (codes::BAD_REQUEST, "options 含 NUL".into()))?,
                ),
                None => None,
            };
            let fo_ptr = fo_c
                .as_ref()
                .map(|c| c.as_ptr())
                .unwrap_or(std::ptr::null());
            let ok = f(self.doc, url_c.as_ptr(), fmt_c.as_ptr(), fo_ptr);
            if ok != 1 {
                let why = self.kit_error_text().unwrap_or_default();
                return Err((
                    codes::SAVE,
                    format!("saveAs 失败（format={format}）：{why}；url={url}"),
                ));
            }
        }
        Ok(json!({"saved": true, "url": url, "format": format}))
    }

    /// setPart。
    fn op_set_part(&mut self, part: i32) -> DispatchResult {
        unsafe {
            self.doc_guard(abi::doc_offsets::SET_PART, "setPart")?;
            let f = (*self.doc_class)
                .set_part
                .ok_or((codes::MEMBER, "缺 setPart".to_string()))?;
            f(self.doc, part);
        }
        Ok(json!({"part": part}))
    }

    /// 导出 PDF（LOK saveAs(url, "pdf", typed options)——暗标导出线 /
    /// #52 分发验收的参数化面；options 形态循 ely-lok `save.rs` 同源
    /// typed JSON：`{type, value}` 包裹，LO PDF 过滤器契约）。
    /// 源文件守卫与 save_as 同拍（导出目标不得别名源文件）。
    fn op_export_pdf(
        &mut self,
        url: &str,
        from: Option<u32>,
        to: Option<u32>,
        notes_in_margin: bool,
        tracked_changes: bool,
    ) -> DispatchResult {
        unsafe {
            self.doc_guard(abi::doc_offsets::SAVE_AS, "saveAs")?;
            crate::save_guard::check(&self.loaded_url, url)
                .map_err(|message| (codes::SAVE, message))?;
            let f = (*self.doc_class)
                .save_as
                .ok_or((codes::MEMBER, "缺 saveAs".to_string()))?;
            let url_c = CString::new(url).map_err(|_| (codes::BAD_REQUEST, "url 含 NUL".into()))?;
            let fmt_c =
                CString::new("pdf").map_err(|_| (codes::BAD_REQUEST, "format 含 NUL".into()))?;
            let flag = |on: bool| json!({"type": "boolean", "value": on.to_string()});
            let mut options = json!({
                "ExportNotes": flag(false),
                "ExportNotesInMargin": flag(notes_in_margin),
                "ExportTrackedChanges": flag(tracked_changes),
            });
            if let (Some(from), Some(to)) = (from, to) {
                if from == 0 || to == 0 || from > to {
                    return Err((
                        codes::BAD_REQUEST,
                        format!("页范围必须 1 起且 from ≤ to：{from}-{to}"),
                    ));
                }
                options["PageRange"] = json!({"type": "string", "value": format!("{from}-{to}")});
            }
            let opts_c = CString::new(options.to_string())
                .map_err(|_| (codes::BAD_REQUEST, "options 含 NUL".into()))?;
            let ok = f(self.doc, url_c.as_ptr(), fmt_c.as_ptr(), opts_c.as_ptr());
            if ok != 1 {
                let why = self.kit_error_text().unwrap_or_default();
                return Err((
                    codes::SAVE,
                    format!("导出 PDF 失败：{why}；url={url}（range={from:?}-{to:?}）"),
                ));
            }
        }
        Ok(json!({
            "exported": true,
            "url": url,
            "format": "pdf",
            "page_from": from,
            "page_to": to,
            "notes_in_margin": notes_in_margin,
            "tracked_changes": tracked_changes,
        }))
    }

    /// close（单文档 worker：**不调用 LO destroy**——LO 25.2 实测：销毁带
    /// doc 级回调的文档在内核 teardown 路径 SalAbort 直退（C++ 对照实验
    /// 2026-11-09：无 doc 回调时 destroy 干净，有则必 SalAbort；setTextSelection
    /// RESET 预清理也无效）。本 worker 是一次性单文档进程：close = 逻辑上
    /// 关闭（后续 doc 请求拒 E_NO_DOCUMENT），内存由进程退出回收，
    /// 落盘只依赖 saveAs，不依赖 destroy）。
    fn op_close(&mut self) -> DispatchResult {
        if !self.doc.is_null() {
            self.doc = std::ptr::null_mut();
            self.doc_class = std::ptr::null_mut();
        }
        Ok(json!({"closed": true, "destroy_skipped": true,
                  "note": "LO 25.2 teardown abort 规避；一次性 worker 由进程退出回收"}))
    }
}

// 写线程 Sender 的线程局部（doc 回调注册时需要从 op_load 内拿到；
// unipoll_loop 启动时注入）。
thread_local! {
    static WRITER_TX: std::cell::RefCell<Option<Sender<ToWriter>>> =
        const { std::cell::RefCell::new(None) };
}

/// 把 stdin 从 fd 0 上「偷走」：dup 到新 fd，fd 0 换成空设备。
/// 返回接管协议流的文件句柄（失败时退回普通 stdin——LO 未抢时不影响）。
///
/// 动机见模块头「fd 0 抢救」：LO 内嵌组件（python 等）直接读 fd 0 会与
/// 协议读线程形成竞争读者，把帧流撕碎 → 宿主超时但 worker 未死。
#[cfg(unix)]
fn steal_stdin_fd() -> std::fs::File {
    use std::os::fd::{AsRawFd, FromRawFd};
    unsafe extern "C" {
        fn dup(fd: c_int) -> c_int;
        fn dup2(old: c_int, new: c_int) -> c_int;
    }
    let saved = unsafe { dup(0) };
    if saved < 0 {
        eprintln!("LOK protocol stdin duplication failed");
        hard_exit(2);
    }
    let protocol = unsafe { std::fs::File::from_raw_fd(saved) };
    let nul = std::fs::File::open("/dev/null").unwrap_or_else(|e| {
        eprintln!("LOK stdin isolation failed: {e}");
        hard_exit(2);
    });
    if unsafe { dup2(nul.as_raw_fd(), 0) } < 0 {
        eprintln!("LOK stdin isolation failed");
        hard_exit(2);
    }
    protocol
}

#[cfg(windows)]
fn steal_stdin_fd() -> std::fs::File {
    use std::os::windows::io::FromRawHandle;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(kind: u32) -> *mut c_void;
        fn GetCurrentProcess() -> *mut c_void;
        fn DuplicateHandle(
            source_process: *mut c_void,
            source: *mut c_void,
            target_process: *mut c_void,
            target: *mut *mut c_void,
            access: u32,
            inherit: i32,
            options: u32,
        ) -> i32;
    }
    #[link(name = "msvcrt")]
    unsafe extern "C" {
        fn _open(path: *const c_char, flags: c_int, ...) -> c_int;
        fn _dup2(old: c_int, new: c_int) -> c_int;
        fn _close(fd: c_int) -> c_int;
    }
    unsafe {
        // Capture the Win32 pipe before the preamble redirects GetStdHandle to NUL.
        // A CRT descriptor is not a Win32 HANDLE and cannot use FromRawFd.
        let input = GetStdHandle(0xfffffff6);
        let mut private = std::ptr::null_mut();
        let process = GetCurrentProcess();
        if input.is_null()
            || input as isize == -1
            || DuplicateHandle(process, input, process, &mut private, 0, 0, 2) == 0
        {
            eprintln!("LOK protocol stdin handle duplication failed");
            hard_exit(2);
        }
        let protocol = std::fs::File::from_raw_handle(private);
        // The private HANDLE stays readable when the CRT's fd 0 is replaced.
        let nul = _open(c"NUL".as_ptr(), 0x8000);
        if nul < 0 {
            eprintln!("LOK CRT stdin isolation failed");
            hard_exit(2);
        }
        let result = _dup2(nul, 0);
        _close(nul);
        if result != 0 {
            eprintln!("LOK CRT stdin redirection failed");
            hard_exit(2);
        }
        protocol
    }
}

/// 真实主循环（B1：unipoll 集成，非 Windows 平台）：主线程进 LOK
/// `runLoop`，宿主请求在 poll 回调里同步分发（内核主循环上下文）；
/// reader 线程只管 stdin→ channel 并 wake（模块头「线程模型」与
/// 「为什么 unipoll」）。Windows 见下方 [`messages_loop`]（#104）。
#[cfg(not(windows))]
fn unipoll_loop(state: WorkerState, tx: &Sender<ToWriter>, protocol_fd: std::fs::File) -> ! {
    WRITER_TX.with(|t| *t.borrow_mut() = Some(tx.clone()));

    // reader 线程：协议流（抢救后的 fd）→ channel；只读字节，JSON 解析
    // 与分发都在主线程（poll 回调）——保证所有 LOK 调用串行于主循环。
    let (note_tx, note_rx) = std::sync::mpsc::channel::<Note>();
    let reader_tx = note_tx.clone();
    let reader = std::thread::Builder::new()
        .name("lok-reader".into())
        .spawn(move || {
            let mut reader = BufReader::new(protocol_fd);
            loop {
                match protocol::read_frame(&mut reader) {
                    Ok(Some(f)) => {
                        if reader_tx.send(Note::Ask(f)).is_err() {
                            break; // 主循环已退出
                        }
                    }
                    Ok(None) => {
                        // 宿主关了 stdin：干净关机（退出码 0）
                        let _ = reader_tx.send(Note::Closed(0));
                        break;
                    }
                    Err(e) => {
                        eprintln!("mdc-lok-worker: 帧读取失败：{e}");
                        let _ = reader_tx.send(Note::Closed(2));
                        break;
                    }
                }
            }
        })
        .expect("spawn lok-reader");
    let _ = reader;

    // wake 通道注册：内核其他线程要主循环干活时叫醒 poll（runLoop 契约）。
    let _ = WAKE_TX.set(std::sync::Mutex::new(note_tx));

    let ctx = LoopCtx {
        state,
        tx: tx.clone(),
        rx: note_rx,
    };
    // runLoop 槽位已在 bind_kernel 的 nSize 守卫查过（缺槽走 boot-failed）。
    let run_loop = unsafe { (*ctx.state.kit_class).run_loop }
        .expect("runLoop 槽位（bind_kernel nSize 守卫已查）");
    let kit = ctx.state.kit;
    let ctx_ptr = Box::into_raw(Box::new(ctx)) as *mut c_void;
    unsafe { run_loop(kit, Some(unipoll_poll), Some(unipoll_wake), ctx_ptr) };
    // runLoop 返回 = 内核主循环正常退出：干净收尾。
    let mut ctx = unsafe { Box::from_raw(ctx_ptr.cast::<LoopCtx>()) };
    loop_exit(&mut ctx, 0, "run-loop-returned");
}

/// 主循环待办：宿主请求帧 / 内核唤醒 / 对端关闭（携退出码）。
/// （unipoll 机件，非 Windows 平台——Windows 走 Win32 消息，见
/// [`messages_loop`]。）
#[cfg(not(windows))]
enum Note {
    Ask(Vec<u8>),
    Woken,
    /// 0 = stdin EOF（干净），2 = 帧读取失败（对齐旧 request_loop 语义）。
    Closed(i32),
}

/// 主循环上下文：unipoll 经 runLoop `data` 指针回到这里；Windows
/// messages_loop 经 [`WND_CTX`] 静态指针回到这里（两平台共用，
/// `handle_frame`/`loop_exit` 的分发/收尾逻辑平台无关）。
struct LoopCtx {
    state: WorkerState,
    tx: Sender<ToWriter>,
    /// unipoll 的 reader 通道（非 Windows；Windows 用 PostMessageW）。
    #[cfg(not(windows))]
    rx: Receiver<Note>,
}

/// wake 通道（内核任意线程 → 主循环 poll；unipoll 机件，非 Windows）。
#[cfg(not(windows))]
static WAKE_TX: std::sync::OnceLock<std::sync::Mutex<Sender<Note>>> = std::sync::OnceLock::new();

/// unipoll poll 回调（非 Windows）：内核空闲时在此等待；请求到达即在本
/// （主）线程同步分发。timeout 语义（微秒，LO runLoop 契约）：0=非阻塞，
/// 负=无限等，正=上限。返回非零 = 有事件已处理（LO 随后泵内部队列）。
#[cfg(not(windows))]
unsafe extern "C" fn unipoll_poll(data: *mut c_void, timeout_us: c_int) -> c_int {
    let ctx = &mut *data.cast::<LoopCtx>();
    let note = match timeout_us {
        0 => match ctx.rx.try_recv() {
            Ok(n) => n,
            Err(_) => return 0,
        },
        t if t < 0 => match ctx.rx.recv() {
            Ok(n) => n,
            Err(_) => loop_exit(ctx, 0, "reader-gone"),
        },
        t => match ctx.rx.recv_timeout(std::time::Duration::from_micros(
            t.clamp(0, 10_000_000) as u64
        )) {
            Ok(n) => n,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return 0,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                loop_exit(ctx, 0, "reader-gone")
            }
        },
    };
    match note {
        Note::Woken => 1,
        Note::Closed(code) => {
            loop_exit(ctx, code, if code == 0 { "stdin-eof" } else { "oversize" })
        }
        Note::Ask(frame) => {
            handle_frame(ctx, frame);
            1
        }
    }
}

/// unipoll wake 回调（非 Windows）：内核其他线程要主循环干活时叫醒 poll。
#[cfg(not(windows))]
unsafe extern "C" fn unipoll_wake(_data: *mut c_void) {
    if let Some(tx) = WAKE_TX.get() {
        let _ = tx.lock().expect("wake 通道锁中毒").send(Note::Woken);
    }
}

// ============================================================================
// Windows：message-only window 消息驱动主循环（#104，#69 终局）
// ============================================================================
//
// 架构与参照：ELY-Open-Source/ElyOffice `crates/ely-lok/src/worker/messages.rs`
// （Apache-2.0；本仓按自身 FFI 风格手写零新依赖）。LO Windows 版
// run_loop 的硬前提：调用线程先持有 message-only window（无窗口
// runLoop 立即返回——#69 Windows 实测：worker ready 后请求帧无应答、
// bye(run-loop-returned) 退出）。顺序：建窗口 → reader 线程
// PostMessageW → runLoop(空钩子) 阻塞泵消息 → WndProc 在主线程同步
// 分发（与 unipoll 同构：所有 LOK 调用串行于内核主循环上下文）。

/// 请求帧到达（lparam = `Box<Vec<u8>>` 指针，WndProc 接管后回收）。
#[cfg(windows)]
const WM_ASK: u32 = 0x8000 + 1; // WM_APP + 1

/// stdin 关闭/读失败（wparam = 退出码：0=EOF 干净，2=帧读失败）。
#[cfg(windows)]
const WM_CLOSED: u32 = 0x8000 + 2; // WM_APP + 2

/// WndProc → 主循环上下文（单窗口单上下文；runLoop 泵消息前必已就位）。
#[cfg(windows)]
static WND_CTX: std::sync::atomic::AtomicPtr<LoopCtx> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// Win32 `WNDCLASSW`（手写 repr(C)，对齐 x64 MSVC 布局；零新依赖）。
#[cfg(windows)]
#[repr(C)]
struct WndClassW {
    style: u32,
    wnd_proc: Option<unsafe extern "system" fn(*mut c_void, u32, usize, isize) -> isize>,
    cls_extra: i32,
    wnd_extra: i32,
    instance: *mut c_void,
    icon: *mut c_void,
    cursor: *mut c_void,
    background: *mut c_void,
    menu_name: *const u16,
    class_name: *const u16,
}

/// Windows 真实主循环（#104）：先建 message-only window，reader 线程把
/// 帧投递成窗口消息，主线程 runLoop 泵消息，WndProc 里同步分发。
#[cfg(windows)]
fn messages_loop(state: WorkerState, tx: &Sender<ToWriter>, protocol_fd: std::fs::File) -> ! {
    use std::sync::atomic::Ordering;

    // WRITER_TX 注入（op_load 的 doc 回调注册面；与 unipoll_loop 同构）。
    WRITER_TX.with(|t| *t.borrow_mut() = Some(tx.clone()));

    // ① 先建窗口（LO Windows run_loop 硬前提——见本节模块注释）。
    //    失败 = 无法进入内核主循环：bye 如实报因 + 硬退（码 3 = 窗口
    //    创建失败，见模块头「退出纪律」的码表扩展）。
    let hwnd = match window_create() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("mdc-lok-worker: message-only window 创建失败：{e}");
            let _ = tx.send(ToWriter::Frame(protocol::life_event_frame(
                "bye",
                json!({"reason": "window-create-failed", "message": e}),
            )));
            let _ = tx.send(ToWriter::Quit);
            std::thread::sleep(std::time::Duration::from_millis(200));
            hard_exit(3);
        }
    };

    // ② 主循环上下文就位（runLoop 泵消息前必已设置——消息只会在泵期
    //    间分发，静态指针无竞态）。
    let ctx = Box::into_raw(Box::new(LoopCtx {
        state,
        tx: tx.clone(),
    }));
    WND_CTX.store(ctx, Ordering::SeqCst);

    // ③ reader 线程：协议流（私有化句柄）→ PostMessageW。帧指针经
    //    lparam 移交 WndProc（Box 独占所有权，投递失败即回收防泄漏）。
    //    HWND 以 usize 携带跨线程（句柄非解引用指针，整数语义安全）。
    let hwnd_usize = hwnd as usize;
    let reader = std::thread::Builder::new()
        .name("lok-reader".into())
        .spawn(move || {
            let hwnd = hwnd_usize as *mut c_void;
            let mut reader = BufReader::new(protocol_fd);
            loop {
                match protocol::read_frame(&mut reader) {
                    Ok(Some(f)) => {
                        let p = Box::into_raw(Box::new(f));
                        if post_message(hwnd, WM_ASK, 0, p as isize) == 0 {
                            // 窗口已不可达（进程退出竞态）：帧丢失，如实硬退。
                            drop(unsafe { Box::from_raw(p) });
                            eprintln!("mdc-lok-worker: 请求帧投递失败");
                            hard_exit(2);
                        }
                    }
                    Ok(None) => {
                        // 宿主关了 stdin：干净关机（码 0；语义对齐
                        // unipoll Note::Closed(0)）。
                        let _ = post_message(hwnd, WM_CLOSED, 0, 0);
                        break;
                    }
                    Err(e) => {
                        eprintln!("mdc-lok-worker: 帧读取失败：{e}");
                        let _ = post_message(hwnd, WM_CLOSED, 2, 0);
                        break;
                    }
                }
            }
        })
        .expect("spawn lok-reader");
    let _ = reader;

    // ④ runLoop（空钩子）：unipoll 契约（env 已设 SAL_LOK_OPTIONS=unipoll）
    //    要求传 hooks，Windows 消息循环从不调用——空实现满足契约
    //    （ElyOffice messages.rs 同款）。窗口就绪后 runLoop 阻塞泵消息
    //    直到内核主循环退出。
    let run_loop = unsafe { (*(*ctx).state.kit_class).run_loop }
        .expect("runLoop 槽位（bind_kernel nSize 守卫已查）");
    let kit = unsafe { (*ctx).state.kit };
    unsafe {
        run_loop(
            kit,
            Some(messages_poll),
            Some(messages_wake),
            std::ptr::null_mut(),
        )
    };

    // ⑤ runLoop 返回 = 内核主循环正常退出：干净收尾（与 unipoll 同义）。
    let ctx = WND_CTX.swap(std::ptr::null_mut(), Ordering::SeqCst);
    loop_exit(unsafe { &mut *ctx }, 0, "run-loop-returned");
}

/// 建消息专用窗口：RegisterClassW + CreateWindowExW(HWND_MESSAGE)。
/// 消息只进不显示（HWND_MESSAGE 父窗口），本进程唯一窗口、唯一读者。
#[cfg(windows)]
fn window_create() -> Result<*mut c_void, String> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn RegisterClassW(class: *const WndClassW) -> u16;
        fn CreateWindowExW(
            ex_style: u32,
            class_name: *const u16,
            window_name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: *mut c_void,
            menu: *mut c_void,
            instance: *mut c_void,
            param: *mut c_void,
        ) -> *mut c_void;
    }
    const HWND_MESSAGE: *mut c_void = -3isize as *mut c_void;
    let name: Vec<u16> = "MdcLokWorker\0".encode_utf16().collect();
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        if instance.is_null() {
            return Err("GetModuleHandleW 返回 NULL".into());
        }
        let class = WndClassW {
            style: 0,
            wnd_proc: Some(messages_wndproc),
            cls_extra: 0,
            wnd_extra: 0,
            instance,
            icon: std::ptr::null_mut(),
            cursor: std::ptr::null_mut(),
            background: std::ptr::null_mut(),
            menu_name: std::ptr::null(),
            class_name: name.as_ptr(),
        };
        if RegisterClassW(&class) == 0 {
            return Err(format!(
                "RegisterClassW 失败：{}（GDI 原子 0）",
                std::io::Error::last_os_error()
            ));
        }
        let made = CreateWindowExW(
            0,
            name.as_ptr(),
            std::ptr::null(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            instance,
            std::ptr::null_mut(),
        );
        if made.is_null() {
            return Err(format!(
                "CreateWindowExW(HWND_MESSAGE) 失败：{}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(made)
    }
}

/// PostMessageW 薄封装（reader 线程投递用）。
#[cfg(windows)]
fn post_message(hwnd: *mut c_void, msg: u32, wparam: usize, lparam: isize) -> i32 {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn PostMessageW(hwnd: *mut c_void, msg: u32, wparam: usize, lparam: isize) -> i32;
    }
    unsafe { PostMessageW(hwnd, msg, wparam, lparam) }
}

/// 窗口过程：ASK=取帧分发（主线程，内核主循环上下文）；CLOSED=收尾退出；
/// 其余走 DefWindowProcW。分发复用 [`handle_frame`]（与 unipoll 同一条
/// JSON 解析→dispatch→answer_or_oversize 链，平台零分叉）。
#[cfg(windows)]
unsafe extern "system" fn messages_wndproc(
    hwnd: *mut c_void,
    msg: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn DefWindowProcW(hwnd: *mut c_void, msg: u32, wparam: usize, lparam: isize) -> isize;
    }
    match msg {
        WM_ASK => {
            // 帧所有权移交完成：Box 独占，离开作用域即释放（无泄漏路径）。
            let frame = unsafe { Box::from_raw(lparam as *mut Vec<u8>) };
            let ctx = WND_CTX.load(std::sync::atomic::Ordering::SeqCst);
            if !ctx.is_null() {
                // 主线程同步分发（runLoop 泵消息期间 = 内核主循环上下文）。
                handle_frame(unsafe { &mut *ctx }, *frame);
            }
            0
        }
        WM_CLOSED => {
            let code = wparam as i32;
            let ctx = WND_CTX.load(std::sync::atomic::Ordering::SeqCst);
            if ctx.is_null() {
                // 理论不可达（泵开始前 WND_CTX 已就位）；防御性硬退保活语义。
                hard_exit(code);
            }
            loop_exit(
                unsafe { &mut *ctx },
                code,
                if code == 0 { "stdin-eof" } else { "oversize" },
            )
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// runLoop 空钩子（unipoll 契约要求非空；Windows 消息循环从不调用）。
#[cfg(windows)]
unsafe extern "C" fn messages_poll(_data: *mut c_void, _timeout_us: c_int) -> c_int {
    0
}

/// runLoop 空 wake 钩子（同上）。
#[cfg(windows)]
unsafe extern "C" fn messages_wake(_data: *mut c_void) {}

#[cfg(all(windows, test))]
mod messages_tests {
    use super::*;

    /// Win32 `MSG`（泵用；尾部多留一个指针位兼容新 SDK 的 lPrivate）。
    #[repr(C)]
    struct Msg {
        hwnd: *mut c_void,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt_x: i32,
        pt_y: i32,
        spare: usize,
    }

    /// 窗口管线冒烟（Windows 实机跑；Linux 沙箱编译面可达但不执行）：
    /// 建窗 → 投递 ASK（WND_CTX=null 安全跳过分发、帧所有权不漏）→
    /// 投递无关消息（DefWindowProc 面）→ 手动泵消费 → 队列排空。
    /// 覆盖 RegisterClassW / CreateWindowExW(HWND_MESSAGE) / PostMessageW /
    /// WndProc 接线在无内核环境下的基本功——#104 实机门的前置冒烟。
    #[test]
    fn message_window_plumbing_smoke() {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn PeekMessageW(
                msg: *mut Msg,
                hwnd: *mut c_void,
                min: u32,
                max: u32,
                remove: u32,
            ) -> i32;
            fn DispatchMessageW(msg: *const Msg) -> isize;
        }
        const PM_REMOVE: u32 = 1;

        let hwnd = window_create().expect("message-only window 创建");
        assert!(!hwnd.is_null(), "HWND 非空");

        // ASK：帧指针经 lparam 移交（WndProc 里 Box::from_raw 回收；
        // WND_CTX 未设置 → 分发安全跳过，帧 drop 不泄漏）。
        let frame = Box::into_raw(Box::new(vec![b'{', b'}']));
        assert_ne!(post_message(hwnd, WM_ASK, 0, frame as isize), 0, "ASK 投递");
        // 无关消息：走 DefWindowProcW 面。
        assert_ne!(post_message(hwnd, 0x0400 + 7, 0, 0), 0, "WM_USER+7 投递");

        // 手动泵（不依赖内核 runLoop——本测试只验窗口接线）。
        let mut msg = Msg {
            hwnd: std::ptr::null_mut(),
            message: 0,
            wparam: 0,
            lparam: 0,
            time: 0,
            pt_x: 0,
            pt_y: 0,
            spare: 0,
        };
        let mut pumped = 0usize;
        unsafe {
            while PeekMessageW(&mut msg, hwnd, 0, 0, PM_REMOVE) != 0 {
                DispatchMessageW(&msg);
                pumped += 1;
                assert!(pumped < 16, "泵出消息数异常：{pumped}");
            }
        }
        assert!(pumped >= 2, "至少消费 ASK + WM_USER+7 两帧，实得 {pumped}");
    }
}

/// 单帧处理（JSON 解析 + 分发 + 应答；语义对齐旧 request_loop）。
fn handle_frame(ctx: &mut LoopCtx, frame: Vec<u8>) {
    let v: Value = match serde_json::from_slice(&frame) {
        Ok(v) => v,
        Err(e) => {
            let _ = ctx.tx.send(ToWriter::Frame(protocol::err_frame(
                0,
                codes::BAD_REQUEST,
                &format!("帧不是合法 JSON：{e}"),
            )));
            return;
        }
    };
    let id = protocol::req_u64(&v, "id").unwrap_or(0);
    let op = protocol::req_str(&v, "op").unwrap_or_default();
    let result = dispatch(&mut ctx.state, &op, &v);
    let frame = match result {
        Ok(val) => protocol::answer_or_oversize(id, protocol::ok_frame(id, val)),
        Err((code, msg)) => protocol::err_frame(id, code, &msg),
    };
    if ctx.tx.send(ToWriter::Frame(frame)).is_err() {
        hard_exit(4);
    }
}

/// 主循环退出（runLoop 返回 / 对端关闭 / reader 消失）：关文档 + bye +
/// 硬退出。退出码：0=干净，2=帧读取失败，4=宿主管道断。
fn loop_exit(ctx: &mut LoopCtx, code: i32, reason: &str) -> ! {
    let _ = ctx.state.op_close();
    if code != 4 {
        // 管道已断时不再可靠发 bye（写线程自己会 hard_exit(4)）。
        let _ = ctx.tx.send(ToWriter::Frame(protocol::life_event_frame(
            "bye",
            json!({"reason": reason}),
        )));
        let _ = ctx.tx.send(ToWriter::Quit);
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    hard_exit(code);
}

/// 内核绑定失败后的请求循环（协议活、LOK 请求全部明确报错）。协议流用
/// 抢救出的 fd（fd 0 已被换成空设备，stdin 不可再用）。
fn no_kernel_loop(
    protocol_fd: std::fs::File,
    tx: &Sender<ToWriter>,
    code: &'static str,
    msg: String,
) -> ! {
    let mut reader = BufReader::new(protocol_fd);
    loop {
        let frame = match protocol::read_frame(&mut reader) {
            Ok(Some(f)) => f,
            Ok(None) | Err(_) => {
                let _ = tx.send(ToWriter::Quit);
                std::thread::sleep(std::time::Duration::from_millis(200));
                hard_exit(0);
            }
        };
        let v: Value = match serde_json::from_slice(&frame) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let id = protocol::req_u64(&v, "id").unwrap_or(0);
        let op = protocol::req_str(&v, "op").unwrap_or_default();
        let frame = if op == "ping" {
            protocol::ok_frame(id, json!({"pong": true, "no_kernel": true}))
        } else {
            protocol::err_frame(id, code, &msg)
        };
        if tx.send(ToWriter::Frame(frame)).is_err() {
            hard_exit(4);
        }
    }
}

/// 协议自测分发（无 LOK；client 集成测试恒跑用）。
fn fake_dispatch_loop(mode: &str, tx: &Sender<ToWriter>, protocol_fd: std::fs::File) -> ! {
    let mut reader = BufReader::new(protocol_fd);
    let mut edits = 0;
    loop {
        let frame = match protocol::read_frame(&mut reader) {
            Ok(Some(f)) => f,
            Ok(None) | Err(_) => {
                let _ = tx.send(ToWriter::Quit);
                std::thread::sleep(std::time::Duration::from_millis(200));
                hard_exit(0);
            }
        };
        let v: Value = match serde_json::from_slice(&frame) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let id = protocol::req_u64(&v, "id").unwrap_or(0);
        let op = protocol::req_str(&v, "op").unwrap_or_default();
        let answer: Value = match (mode, op.as_str()) {
            (_, "ping") => protocol::ok_frame(id, json!({"pong": true})),
            ("edit-fails-after-first", "post_key") => {
                edits += 1;
                if edits == 1 {
                    protocol::ok_frame(id, json!({"edited": true}))
                } else {
                    protocol::err_frame(id, codes::BAD_REQUEST, "injected second edit failure")
                }
            }
            ("edit-fails-after-first", _) => protocol::ok_frame(id, json!({})),
            ("echo", _) => protocol::ok_frame(id, json!({"echo": op})),
            ("hang", "slow") => {
                // 永不回答（宿主超时看门狗测试；stdin 保活）
                continue;
            }
            ("hang", _) => protocol::ok_frame(id, json!({"echo": op})),
            ("die", "boom") => {
                let _ = tx.send(ToWriter::Frame(protocol::life_event_frame(
                    "bye",
                    json!({"reason": "boom"}),
                )));
                let _ = tx.send(ToWriter::Quit);
                std::thread::sleep(std::time::Duration::from_millis(100));
                hard_exit(1);
            }
            ("die", _) => protocol::ok_frame(id, json!({"echo": op})),
            _ => protocol::err_frame(id, codes::BAD_REQUEST, "未知 fake 模式"),
        };
        // "huge" op：任何模式都回超限应答（worker 存活，帧长降级路径恒跑）
        let answer = if op == "huge" {
            let big = "x".repeat(crate::MAX_FRAME_BYTES);
            protocol::answer_or_oversize(id, protocol::ok_frame(id, json!({ "blob": big })))
        } else {
            answer
        };
        if tx.send(ToWriter::Frame(answer)).is_err() {
            hard_exit(4);
        }
    }
}

/// 请求分发（op → state 方法；args 校验错误 = E_BAD_REQUEST）。
fn dispatch(state: &mut WorkerState, op: &str, v: &Value) -> DispatchResult {
    match op {
        "ping" => Ok(json!({"pong": true, "pid": std::process::id()})),
        "version" => Ok(json!({"version_info": state.kit_version_info()})),
        "load" => {
            let url = protocol::req_str(v, "url")
                .ok_or((codes::BAD_REQUEST, "load 需要 url".to_string()))?;
            let options = protocol::req_str(v, "options");
            state.op_load(&url, options.as_deref())
        }
        "close" => state.op_close(),
        "init_rendering" => {
            let args = protocol::req_str(v, "args");
            state.op_init_rendering(args.as_deref())
        }
        "size" => state.op_size(),
        "page_geometry" => state.op_page_geometry(),
        "paint" => {
            let cw = protocol::req_u64(v, "canvas_w")
                .ok_or((codes::BAD_REQUEST, "paint 需要 canvas_w".to_string()))?
                as usize;
            let ch = protocol::req_u64(v, "canvas_h")
                .ok_or((codes::BAD_REQUEST, "paint 需要 canvas_h".to_string()))?
                as usize;
            let tx0 = protocol::req_i32(v, "tile_x").unwrap_or(0);
            let ty0 = protocol::req_i32(v, "tile_y").unwrap_or(0);
            let tw = protocol::req_i32(v, "tile_w")
                .ok_or((codes::BAD_REQUEST, "paint 需要 tile_w".to_string()))?;
            let th = protocol::req_i32(v, "tile_h")
                .ok_or((codes::BAD_REQUEST, "paint 需要 tile_h".to_string()))?;
            state.op_paint(cw, ch, tx0, ty0, tw, th)
        }
        "post_key" => {
            let ch = protocol::req_i32(v, "char")
                .ok_or((codes::BAD_REQUEST, "post_key 需要 char".to_string()))?;
            let kc = protocol::req_i32(v, "key_code").unwrap_or(0);
            state.op_post_key(ch, kc)
        }
        "post_text" => {
            let text = protocol::req_str(v, "text")
                .ok_or((codes::BAD_REQUEST, "post_text 需要 text".to_string()))?;
            let commit = v.get("commit").and_then(Value::as_bool).unwrap_or(false);
            state.op_post_text(commit, &text)
        }
        "post_mouse" => {
            let n_type = protocol::req_i32(v, "type").unwrap_or(abi::MOUSEEVENT_BUTTON_DOWN);
            let x = protocol::req_i32(v, "x").unwrap_or(0);
            let y = protocol::req_i32(v, "y").unwrap_or(0);
            let count = protocol::req_i32(v, "count").unwrap_or(1);
            let buttons = protocol::req_i32(v, "buttons").unwrap_or(abi::MOUSE_BUTTON_LEFT);
            let modifier = protocol::req_i32(v, "modifier").unwrap_or(0);
            state.op_post_mouse(n_type, x, y, count, buttons, modifier)
        }
        "uno" => {
            let command = protocol::req_str(v, "command")
                .ok_or((codes::BAD_REQUEST, "uno 需要 command".to_string()))?;
            let args = protocol::req_str(v, "args");
            let notify = v.get("notify").and_then(Value::as_bool).unwrap_or(false);
            state.op_uno(&command, args.as_deref(), notify)
        }
        "text_selection" => {
            let mime = protocol::req_str(v, "mime")
                .ok_or((codes::BAD_REQUEST, "text_selection 需要 mime".to_string()))?;
            state.op_text_selection(&mime)
        }
        "save_as" => {
            let url = protocol::req_str(v, "url")
                .ok_or((codes::BAD_REQUEST, "save_as 需要 url".to_string()))?;
            let format = protocol::req_str(v, "format")
                .ok_or((codes::BAD_REQUEST, "save_as 需要 format".to_string()))?;
            let fo = protocol::req_str(v, "filter_options");
            state.op_save_as(&url, &format, fo.as_deref())
        }
        "set_part" => {
            let part = protocol::req_i32(v, "part")
                .ok_or((codes::BAD_REQUEST, "set_part 需要 part".to_string()))?;
            state.op_set_part(part)
        }
        "export_pdf" => {
            let url = protocol::req_str(v, "url")
                .ok_or((codes::BAD_REQUEST, "export_pdf 需要 url".to_string()))?;
            let from = protocol::req_u64(v, "page_from").and_then(|x| u32::try_from(x).ok());
            let to = protocol::req_u64(v, "page_to").and_then(|x| u32::try_from(x).ok());
            if v.get("page_from").is_some() != v.get("page_to").is_some() {
                return Err((
                    codes::BAD_REQUEST,
                    "page_from/page_to 必须成对出现".to_string(),
                ));
            }
            let notes = v
                .get("notes_in_margin")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let changes = v
                .get("tracked_changes")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            state.op_export_pdf(&url, from, to, notes, changes)
        }
        "shutdown" => {
            let _ = state.op_close();
            let _ = std::io::stdout().flush();
            hard_exit(0);
        }
        _ => Err((codes::BAD_REQUEST, format!("未知 op：{op}"))),
    }
}

/// stderr 尾部保留器（宿主诊断用；worker 自己不打日志到 stderr——LO 会打）。
pub struct StderrTail {
    pub lines: Arc<Mutex<VecDeque<String>>>,
}

impl StderrTail {
    pub fn new(cap: usize) -> StderrTail {
        StderrTail {
            lines: Arc::new(Mutex::new(VecDeque::with_capacity(cap + 1))),
        }
    }

    pub fn tail(&self, n: usize) -> Vec<String> {
        self.lines
            .lock()
            .map(|q| q.iter().rev().take(n).cloned().collect())
            .unwrap_or_default()
    }
}

/// 解析 LOK `getPartPageRectangles` 返回串 → 页矩形列表。
///
/// 实测格式（LibreOffice 25.2.3.2，Debian 13，2026-11-11，真内核链
/// 取证——smoke bin 3 页夹具）：`"x, y, width, height; "` 每页一条、
/// 顺序即页序、`; ` 分隔（尾项可带或不带分隔符）。坐标为**画布坐标
/// 系 twips**（与 paintTile 同源）：画布 = 页 + 四周 284 twips 边距，
/// 页间隙同为 284——单页为 `(284, 284, w, h)`，第 i 页 y =
/// 284 + (页高 + 284) × i。解析器对空项容错（尾/连续分隔符），对
/// 非四元组条目明确报错（不猜造语义）。
fn parse_page_rectangles(s: &str) -> Result<Vec<[i64; 4]>, String> {
    let mut out = Vec::new();
    for (idx, entry) in s.split(';').enumerate() {
        let entry = entry.trim();
        if entry.is_empty() {
            continue; // 尾分隔符 / 连续分隔符容错
        }
        let nums: Vec<&str> = entry.split(',').map(str::trim).collect();
        if nums.len() != 4 {
            return Err(format!(
                "第 {} 项应为 x, y, w, h 四元组，实得 {} 元：{:?}",
                idx + 1,
                nums.len(),
                entry
            ));
        }
        let mut r = [0i64; 4];
        for (slot, tok) in nums.iter().enumerate() {
            r[slot] = tok
                .parse::<i64>()
                .map_err(|_| format!("第 {} 项坐标 {:?} 不是整数", idx + 1, tok))?;
        }
        out.push(r);
    }
    if out.is_empty() {
        return Err("串内无有效页矩形项".to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::parse_page_rectangles;

    /// 25.2.3.2 实测格式回归（smoke bin --fixture 3page.docx 取证原文：
    /// 页矩形在画布坐标系，含 284 twips 边距与页隙，非原点首尾相接）。
    #[test]
    fn parses_three_pages_with_border_and_gap() {
        let raw = "284, 284, 11906, 16838; 284, 17406, 11906, 16838; 284, 34528, 11906, 16838";
        let r = parse_page_rectangles(raw).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0], [284, 284, 11906, 16838]);
        // 画布几何自洽：页隙 = 首页顶边距 = 284（25.2.3.2 实测不变量）
        let gap1 = r[1][1] - (r[0][1] + r[0][3]);
        let gap2 = r[2][1] - (r[1][1] + r[1][3]);
        assert_eq!(gap1, 284);
        assert_eq!(gap2, 284);
        assert_eq!(r[2][1] + r[2][3] + 284, 51650, "末页底 + 下边距 = 画布高");
    }

    #[test]
    fn tolerates_whitespace_and_missing_tail_separator() {
        let raw = "  0,0,11906,16838 ;\n 0,16838 , 11906,16838";
        let r = parse_page_rectangles(raw).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[1], [0, 16838, 11906, 16838]);
    }

    #[test]
    fn rejects_non_four_tuples_and_garbage() {
        assert!(
            parse_page_rectangles("0, 0, 11906").is_err(),
            "三元组必须拒"
        );
        assert!(
            parse_page_rectangles("abc, def, ghi, jkl").is_err(),
            "非数字必须拒"
        );
        assert!(parse_page_rectangles("").is_err(), "空串必须拒");
        assert!(parse_page_rectangles("; ;").is_err(), "仅分隔符必须拒");
    }
}
