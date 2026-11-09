//! worker 侧运行时：LOK 内核绑定 + 请求分发 + 回调转发。
//!
//! ## 线程模型（原型）
//!
//! ```text
//! 宿主 pipe ──stdin──▶ 主线程（请求循环：所有 LOK 调用串行在此执行）
//!                        │
//! LO 内核线程 ──回调──▶ 事件队列 ──▶ 写线程（独占 stdout：应答+事件同串行化）
//! 宿主 pipe ◀──stdout── 写线程
//! ```
//!
//! 回调在 LO 内核线程触发（ElyOffice 实证）→ 只入队（持锁极短），由写线程
//! 单点写帧，避免应答/事件交错。写线程管道断裂（宿主已死）→ worker 硬退出
//! （孤儿清理）。
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
        Some(mode) => fake_dispatch_loop(mode, &tx),
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
                    no_kernel_loop(&tx, code, msg);
                }
            };
            request_loop(state, &tx)
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

    /// getVersionInfo 原文（freeError 释放）。
    fn kit_version_info(&mut self) -> String {
        unsafe {
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
        }
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
// request_loop 启动时注入）。
thread_local! {
    static WRITER_TX: std::cell::RefCell<Option<Sender<ToWriter>>> =
        const { std::cell::RefCell::new(None) };
}

/// 真实请求循环（主线程串行执行所有 LOK 调用）。
fn request_loop(mut state: WorkerState, tx: &Sender<ToWriter>) -> ! {
    WRITER_TX.with(|t| *t.borrow_mut() = Some(tx.clone()));
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    loop {
        let frame = match protocol::read_frame(&mut reader) {
            Ok(Some(f)) => f,
            Ok(None) => {
                // 宿主关了 stdin：干净关机
                let _ = state.op_close();
                let _ = tx.send(ToWriter::Frame(protocol::life_event_frame(
                    "bye",
                    json!({"reason": "stdin-eof"}),
                )));
                let _ = tx.send(ToWriter::Quit);
                std::thread::sleep(std::time::Duration::from_millis(200));
                hard_exit(0);
            }
            Err(e) => {
                let _ = tx.send(ToWriter::Frame(protocol::life_event_frame(
                    "bye",
                    json!({"reason": "oversize", "message": e.to_string()}),
                )));
                let _ = tx.send(ToWriter::Quit);
                std::thread::sleep(std::time::Duration::from_millis(200));
                hard_exit(2);
            }
        };
        let v: Value = match serde_json::from_slice(&frame) {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(ToWriter::Frame(protocol::err_frame(
                    0,
                    codes::BAD_REQUEST,
                    &format!("帧不是合法 JSON：{e}"),
                )));
                continue;
            }
        };
        let id = protocol::req_u64(&v, "id").unwrap_or(0);
        let op = protocol::req_str(&v, "op").unwrap_or_default();
        let result = dispatch(&mut state, &op, &v);
        let frame = match result {
            Ok(val) => protocol::answer_or_oversize(id, protocol::ok_frame(id, val)),
            Err((code, msg)) => protocol::err_frame(id, code, &msg),
        };
        if tx.send(ToWriter::Frame(frame)).is_err() {
            hard_exit(4);
        }
    }
}

/// 内核绑定失败后的请求循环（协议活、LOK 请求全部明确报错）。
fn no_kernel_loop(tx: &Sender<ToWriter>, code: &'static str, msg: String) -> ! {
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
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
fn fake_dispatch_loop(mode: &str, tx: &Sender<ToWriter>) -> ! {
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
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
