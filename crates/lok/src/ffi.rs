//! LibreOfficeKit FFI（#633）。
//!
//! ABI 依据：钉定头
//! `include/LibreOfficeKit/{LibreOfficeKit,LibreOfficeKitInit,LibreOfficeKitEnums,LibreOfficeKitTypes}.h`
//! @ LibreOffice/core `0d51fdc57aef6707466320821b8c2931c00297c0`。
//! 两个 class 结构体均为「`size_t nSize` 首成员 + 顺序函数指针槽」，
//! LO 侧只追加不重排（append-only）；本模块按钉定头声明全部槽位，
//! 调用前做 **nSize 运行时成员检查**（比 C 宏 `LIBREOFFICEKIT_HAS`
//! 的 `offsetof < nSize` 更严一档：要求 `offset + 8 <= nSize` 整槽
//! 在界），运行时过旧给 `RuntimeTooOld` 明确错误。
//!
//! 加载序（钉定 LibreOfficeKitInit.h `lok_dlopen`）：
//! `<program>/libsofficeapp.so`（存在且 >1000 字节）否则
//! `<program>/libmergedlo.so`；Windows 同理 `sofficeapp.dll` /
//! `mergedlo.dll`，且用 `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR |
//! LOAD_LIBRARY_SEARCH_DEFAULT_DIRS` 让依赖 DLL 从同目录解析
//! （ElyOffice 同款做法）。
//!
//! hook 语义（钉定 desktop/source/lib/init.cxx `lo_initialize` 实证）：
//! - `install_path` = **program 目录**（init.cxx:8208 以
//!   `aAppURL + "/.."` 推导 BRAND_BASE_DIR——传安装根目录会找错
//!   share/registry，Linux 实测见 gates/633.md）；
//! - `user_profile_url` 接受 `vnd.sun.star.pathname:<系统路径>`，
//!   每会话独立 profile（不复用平台共享 profile，防多会话互锁）。
//!
//! 内存纪律：LO 返回的 `char*` 一律经 `freeError`（钉定头注释：跨
//! CRT 边界禁止自己 free）。

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::os::raw::{c_uchar, c_ulonglong};
use std::path::{Path, PathBuf};
use std::ptr;

use libloading::{Library, Symbol};

use crate::error::{ErrorCode, LokError, LokResult};

/// 钉定 LibreOfficeKitEnums.h @0d51fdc 的枚举值（只列本 crate 用到的）。
pub mod lok_enums {
    /// LibreOfficeKitKeyEventType：LOK_KEYEVENT_KEYINPUT。
    pub const KEYEVENT_KEYINPUT: i32 = 0;
    /// LOK_KEYEVENT_KEYUP。
    pub const KEYEVENT_KEYUP: i32 = 1;

    /// LibreOfficeKitMouseEventType。
    pub const MOUSEEVENT_BUTTON_DOWN: i32 = 0;
    pub const MOUSEEVENT_BUTTON_UP: i32 = 1;
    pub const MOUSEEVENT_MOVE: i32 = 2;

    /// LibreOfficeKitCallback（节选）。
    pub const CALLBACK_INVALIDATE_TILES: i32 = 0;
    pub const CALLBACK_INVALIDATE_VISIBLE_CURSOR: i32 = 1;
    pub const CALLBACK_TEXT_SELECTION: i32 = 2;
    pub const CALLBACK_STATE_CHANGED: i32 = 8;
    pub const CALLBACK_DOCUMENT_SIZE_CHANGED: i32 = 13;
    pub const CALLBACK_UNO_COMMAND_RESULT: i32 = 16;
    pub const CALLBACK_ERROR: i32 = 22;

    /// LibreOfficeKitTileMode。
    pub const TILEMODE_RGBA: i32 = 0;
    pub const TILEMODE_BGRA: i32 = 1;
}

/// 回调原型：`void (*LibreOfficeKitCallback)(int, const char*, void*)`。
pub type LokCallback =
    unsafe extern "C" fn(n_type: c_int, payload: *const c_char, data: *mut c_void);

/// `LibreOfficeKitPollCallback`（runLoop 用，钉定 Types 头）：
/// 返回非 0 = 处理了事件。
pub type LokPollCallback = unsafe extern "C" fn(p_data: *mut c_void, timeout_us: i32) -> i32;

/// `LibreOfficeKitWakeCallback`（runLoop 用）。
pub type LokWakeCallback = unsafe extern "C" fn(p_data: *mut c_void);

// ---------------------------------------------------------------- vtable

/// Office 级 vtable（钉定 LibreOfficeKit.h，顺序即 ABI，26 槽）。
#[repr(C)]
pub struct LibreOfficeKitClass {
    pub n_size: usize,
    pub destroy: Option<unsafe extern "C" fn(*mut LibreOfficeKit)>,
    pub document_load: Option<
        unsafe extern "C" fn(*mut LibreOfficeKit, *const c_char) -> *mut LibreOfficeKitDocument,
    >,
    pub get_error: Option<unsafe extern "C" fn(*mut LibreOfficeKit) -> *mut c_char>,
    pub document_load_with_options: Option<
        unsafe extern "C" fn(
            *mut LibreOfficeKit,
            *const c_char,
            *const c_char,
        ) -> *mut LibreOfficeKitDocument,
    >,
    pub free_error: Option<unsafe extern "C" fn(*mut c_char)>,
    pub register_callback:
        Option<unsafe extern "C" fn(*mut LibreOfficeKit, LokCallback, *mut c_void)>,
    pub get_filter_types: Option<unsafe extern "C" fn(*mut LibreOfficeKit) -> *mut c_char>,
    pub set_optional_features: Option<unsafe extern "C" fn(*mut LibreOfficeKit, c_ulonglong)>,
    pub set_document_password:
        Option<unsafe extern "C" fn(*mut LibreOfficeKit, *const c_char, *const c_char)>,
    pub get_version_info: Option<unsafe extern "C" fn(*mut LibreOfficeKit) -> *mut c_char>,
    pub run_macro: Option<unsafe extern "C" fn(*mut LibreOfficeKit, *const c_char) -> c_int>,
    pub sign_document: Option<
        unsafe extern "C" fn(
            *mut LibreOfficeKit,
            *const c_char,
            *const c_uchar,
            c_int,
            *const c_uchar,
            c_int,
        ) -> bool,
    >,
    pub run_loop: Option<
        unsafe extern "C" fn(*mut LibreOfficeKit, LokPollCallback, LokWakeCallback, *mut c_void),
    >,
    pub set_option: Option<unsafe extern "C" fn(*mut LibreOfficeKit, *const c_char, *const c_char)>,
    pub dump_state:
        Option<unsafe extern "C" fn(*mut LibreOfficeKit, *const c_char, *mut *mut c_char)>,
    pub extract_request:
        Option<unsafe extern "C" fn(*mut LibreOfficeKit, *const c_char) -> *mut c_char>,
    pub trim_memory: Option<unsafe extern "C" fn(*mut LibreOfficeKit, c_int)>,
    pub start_urp: Option<
        unsafe extern "C" fn(
            *mut LibreOfficeKit,
            *mut c_void,
            *mut c_void,
            *mut c_void,
        ) -> *mut c_void,
    >,
    pub stop_urp: Option<unsafe extern "C" fn(*mut LibreOfficeKit, *mut c_void)>,
    pub join_threads: Option<unsafe extern "C" fn(*mut LibreOfficeKit) -> c_int>,
    pub start_threads: Option<unsafe extern "C" fn(*mut LibreOfficeKit)>,
    pub set_forked_child: Option<unsafe extern "C" fn(*mut LibreOfficeKit, bool)>,
    pub extract_document_structure_request: Option<
        unsafe extern "C" fn(*mut LibreOfficeKit, *const c_char, *const c_char) -> *mut c_char,
    >,
    pub register_any_input_callback:
        Option<unsafe extern "C" fn(*mut LibreOfficeKit, *mut c_void, *mut c_void)>,
    pub get_docs_count: Option<unsafe extern "C" fn(*mut LibreOfficeKit) -> c_int>,
    pub register_file_save_dialog_callback:
        Option<unsafe extern "C" fn(*mut LibreOfficeKit, *mut c_void)>,
}

/// 文档级 vtable（钉定头，78 槽；只命名本 crate 调用面，尾部槽计数
/// 以 `_tail` 占位保证 size_of 与钉定头一致，nSize 检查依赖此布局）。
#[repr(C)]
pub struct LibreOfficeKitDocumentClass {
    pub n_size: usize,
    pub destroy: Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument)>,
    pub save_as: Option<
        unsafe extern "C" fn(
            *mut LibreOfficeKitDocument,
            *const c_char,
            *const c_char,
            *const c_char,
        ) -> c_int,
    >,
    pub get_document_type: Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument) -> c_int>,
    pub get_parts: Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument) -> c_int>,
    pub get_part_page_rectangles:
        Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument) -> *mut c_char>,
    pub get_part: Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument) -> c_int>,
    pub set_part: Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument, c_int)>,
    pub get_part_name:
        Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument, c_int) -> *mut c_char>,
    pub set_part_mode: Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument, c_int)>,
    pub paint_tile: Option<
        unsafe extern "C" fn(
            *mut LibreOfficeKitDocument,
            *mut c_uchar,
            c_int,
            c_int,
            c_int,
            c_int,
            c_int,
            c_int,
        ),
    >,
    pub get_tile_mode: Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument) -> c_int>,
    pub get_document_size:
        Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument, *mut i64, *mut i64)>,
    pub initialize_for_rendering:
        Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument, *const c_char)>,
    pub register_callback:
        Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument, LokCallback, *mut c_void)>,
    pub post_key_event:
        Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument, c_int, c_int, c_int)>,
    pub post_mouse_event: Option<
        unsafe extern "C" fn(*mut LibreOfficeKitDocument, c_int, c_int, c_int, c_int, c_int, c_int),
    >,
    pub post_uno_command: Option<
        unsafe extern "C" fn(*mut LibreOfficeKitDocument, *const c_char, *const c_char, bool),
    >,
    pub set_text_selection:
        Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument, c_int, c_int, c_int)>,
    pub get_text_selection: Option<
        unsafe extern "C" fn(
            *mut LibreOfficeKitDocument,
            *const c_char,
            *mut *mut c_char,
        ) -> *mut c_char,
    >,
    pub paste: Option<
        unsafe extern "C" fn(
            *mut LibreOfficeKitDocument,
            *const c_char,
            *const c_char,
            usize,
        ) -> bool,
    >,
    pub set_graphic_selection:
        Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument, c_int, c_int, c_int)>,
    pub reset_selection: Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument)>,
    pub get_command_values:
        Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument, *const c_char) -> *mut c_char>,
    pub set_client_zoom:
        Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument, c_int, c_int, c_int, c_int)>,
    pub set_client_visible_area:
        Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument, c_int, c_int, c_int, c_int)>,
    pub create_view: Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument) -> c_int>,
    pub destroy_view: Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument, c_int)>,
    pub set_view: Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument, c_int)>,
    pub get_view: Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument) -> c_int>,
    pub get_views_count: Option<unsafe extern "C" fn(*mut LibreOfficeKitDocument) -> c_int>,
    /// 尾部未命名槽（renderFont…setColorPreviewState，钉定头共 78 槽）。
    /// 用原始指针数组占位：size_of 保证 nSize 检查的偏移语义正确。
    pub _tail: [*mut c_void; 48],
}

#[repr(C)]
pub struct LibreOfficeKit {
    pub class: *mut LibreOfficeKitClass,
}

#[repr(C)]
pub struct LibreOfficeKitDocument {
    pub class: *mut LibreOfficeKitDocumentClass,
}

const SLOT: usize = std::mem::size_of::<*mut c_void>();

/// 整槽在界检查：`offset + 8 <= n_size`（严于 C 宏 `offsetof < nSize`）。
fn slot_available(n_size: usize, offset: usize, member: &str) -> LokResult<()> {
    if offset + SLOT <= n_size {
        Ok(())
    } else {
        Err(LokError::too_old(member, n_size, offset + SLOT))
    }
}

macro_rules! kit_member {
    ($kit:expr, $field:ident) => {{
        let class = unsafe { &*$kit.class };
        slot_available(
            class.n_size,
            std::mem::offset_of!(LibreOfficeKitClass, $field),
            stringify!($field),
        )?;
        class.$field.ok_or_else(|| {
            LokError::new(
                ErrorCode::InitFailed,
                concat!("运行时 vtable 槽位为空：", stringify!($field)),
            )
        })?
    }};
}

macro_rules! doc_member {
    ($doc:expr, $field:ident) => {{
        let class = unsafe { &*(*$doc.doc).class };
        slot_available(
            class.n_size,
            std::mem::offset_of!(LibreOfficeKitDocumentClass, $field),
            stringify!($field),
        )?;
        class.$field.ok_or_else(|| {
            LokError::new(
                ErrorCode::InitFailed,
                concat!("运行时 vtable 槽位为空：", stringify!($field)),
            )
        })?
    }};
}

// ------------------------------------------------------------ 运行时发现

/// 平台库文件名（钉定 LibreOfficeKitInit.h 加载序）。
fn library_candidates(program_dir: &Path) -> Vec<(PathBuf, &'static str)> {
    #[cfg(target_os = "macos")]
    let names = ["libsofficeapp.dylib", "libmergedlo.dylib"];
    #[cfg(all(unix, not(target_os = "macos")))]
    let names = ["libsofficeapp.so", "libmergedlo.so"];
    #[cfg(windows)]
    let names = ["sofficeapp.dll", "mergedlo.dll"];
    names.iter().map(|n| (program_dir.join(n), *n)).collect()
}

/// 按钉定 Init.h 规则挑库：sofficeapp 存在且 >1000 字节则用之，
/// 否则用 merged 库。
pub fn pick_library(program_dir: &Path) -> LokResult<(PathBuf, &'static str)> {
    let candidates = library_candidates(program_dir);
    // 第一候选：sofficeapp 且非 stub（merged 构建里它是单行文本 stub）。
    if let Some((path, name)) = candidates.first() {
        if let Ok(meta) = std::fs::metadata(path) {
            if meta.is_file() && meta.len() > 1000 {
                return Ok((path.clone(), name));
            }
        }
    }
    if let Some((path, name)) = candidates.get(1) {
        if path.exists() {
            return Ok((path.clone(), name));
        }
    }
    Err(LokError::runtime_not_found(format!(
        "在 {program_dir:?} 未找到 LibreOffice 库（尝试 libsofficeapp/libmergedlo）；\
         可用 MDC_LOK_DIR 显式指定 program 目录，运行时发现方式见 docs/lok-runtime.md"
    )))
}

/// 宿主平台候选 program 目录（无显式配置时的发现序）。
pub fn default_program_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(dir) = std::env::var("MDC_LOK_DIR") {
        out.push(PathBuf::from(dir));
    }
    #[cfg(unix)]
    {
        for p in [
            "/usr/lib/libreoffice/program",
            "/usr/local/lib/libreoffice/program",
            "/opt/libreoffice/program",
        ] {
            out.push(PathBuf::from(p));
        }
    }
    #[cfg(windows)]
    {
        if let Ok(pf) = std::env::var("ProgramFiles") {
            out.push(PathBuf::from(pf).join("LibreOffice").join("program"));
        }
        if let Ok(pf86) = std::env::var("ProgramFiles(x86)") {
            out.push(PathBuf::from(pf86).join("LibreOffice").join("program"));
        }
    }
    out
}

// ---------------------------------------------------------------- Kit 封装

/// 已初始化的 LOK Office 句柄。
///
/// 线程纪律：LOK 非线程安全——本句柄的全部调用必须来自创建它的
/// 同一线程（worker 的分发线程，见 worker.rs）。
pub struct Lok {
    _lib: Library,
    kit: *mut LibreOfficeKit,
    class: *mut LibreOfficeKitClass,
}

/// dlopen 并 hook_2 初始化（profile 隔离：每 worker 独立目录）。
///
/// `program_dir` = LibreOffice program 目录（钉定 init.cxx 语义）；
/// `profile_dir` = 本会话专属用户 profile 目录（worker 负责创建，
/// 不复用平台共享 profile）。
pub fn lok_init(program_dir: &Path, profile_dir: &Path) -> LokResult<Lok> {
    let (lib_path, lib_name) = pick_library(program_dir)?;
    std::fs::create_dir_all(profile_dir).map_err(|e| {
        LokError::new(
            ErrorCode::InitFailed,
            format!("profile 目录创建失败 {profile_dir:?}：{e}"),
        )
    })?;

    unsafe {
        // Windows：依赖 DLL 与 mergedlo 同目录，需搜索标志加载
        // （libloading 0.9 os::windows）；Unix：Library::new = dlopen
        // RTLD_LAZY|RTLD_LOCAL（与钉定 Init.h 默认一致）。
        #[cfg(windows)]
        let lib = {
            use libloading::os::windows::{
                Library as WinLibrary, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
            };
            WinLibrary::load_with_flags(
                &lib_path,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            )
            .map_err(|e| {
                LokError::new(
                    ErrorCode::LibraryLoadFailed,
                    format!("LoadLibraryExW {lib_path:?} 失败：{e}"),
                )
            })?
        };
        #[cfg(unix)]
        let lib = Library::new(&lib_path).map_err(|e| {
            LokError::new(
                ErrorCode::LibraryLoadFailed,
                format!("dlopen {lib_path:?} 失败：{e}"),
            )
        })?;

        let hook2: Symbol<
            unsafe extern "C" fn(*const c_char, *const c_char) -> *mut LibreOfficeKit,
        > = lib.get(b"libreofficekit_hook_2\0").map_err(|e| {
            LokError::new(
                ErrorCode::LibraryLoadFailed,
                format!(
                    "{lib_name:?} 缺 libreofficekit_hook_2 导出（{e}）；\
                         实际命令：`nm -D <lib> | grep hook`（Windows：\
                         dumpbin /exports）核对发行版符号面"
                ),
            )
        })?;

        // profile 走 vnd.sun.star.pathname: 形式（钉定 init.cxx:8196
        // 会转成 URL；系统路径含空格/中文时无需手工百分号编码）。
        let install_c = CString::new(program_dir.as_os_str().to_string_lossy().as_bytes())
            .map_err(|_| LokError::new(ErrorCode::InvalidRequest, "install 路径含内部 NUL"))?;
        let profile_specifier = format!("vnd.sun.star.pathname:{}", profile_dir.to_string_lossy());
        let profile_c = CString::new(profile_specifier.as_bytes())
            .map_err(|_| LokError::new(ErrorCode::InvalidRequest, "profile 路径含内部 NUL"))?;

        let kit = hook2(install_c.as_ptr(), profile_c.as_ptr());
        if kit.is_null() {
            return Err(LokError::new(
                ErrorCode::InitFailed,
                format!(
                    "libreofficekit_hook_2 返回 NULL（program_dir={program_dir:?}，\
                     profile={profile_dir:?}）——常见原因：install 路径应为 program 目录\
                     而非安装根；或运行时缺 share/registry"
                ),
            ));
        }
        let class = (*kit).class;
        if class.is_null() || (*class).n_size < SLOT * 4 {
            return Err(LokError::new(
                ErrorCode::InitFailed,
                "LOK kit vtable 异常（nSize 过小）",
            ));
        }
        Ok(Lok {
            _lib: lib,
            kit,
            class,
        })
    }
}

impl Lok {
    /// LO 返回的 char* → String，并按钉定头纪律经 freeError 释放。
    unsafe fn take_string(&self, raw: *mut c_char) -> Option<String> {
        if raw.is_null() {
            return None;
        }
        let s = CStr::from_ptr(raw).to_string_lossy().into_owned();
        if let Some(free_error) = (*self.class).free_error {
            free_error(raw);
        }
        Some(s)
    }

    /// 运行时版本信息 JSON（getVersionInfo：ProductName/Version/BuildId）。
    pub fn version_info(&self) -> LokResult<Option<String>> {
        let f = kit_member!(self, get_version_info);
        unsafe { Ok(self.take_string(f(self.kit))) }
    }

    /// kit 级最近错误文本（documentLoad 失败时随附）。
    pub fn last_error(&self) -> LokResult<Option<String>> {
        let f = kit_member!(self, get_error);
        unsafe { Ok(self.take_string(f(self.kit))) }
    }

    pub fn kit_nsize(&self) -> usize {
        unsafe { (*self.class).n_size }
    }

    /// 内核 kit 原始指针（unipoll runLoop 集成用）。
    pub fn kit_ptr(&self) -> *mut LibreOfficeKit {
        self.kit
    }

    /// 加载文档（file: URL）。返回文档句柄。
    pub fn document_load(&self, url: &str) -> LokResult<LokDocument> {
        let f = kit_member!(self, document_load);
        let url_c = CString::new(url)
            .map_err(|_| LokError::new(ErrorCode::InvalidRequest, "文档 URL 含内部 NUL"))?;
        let doc = unsafe { f(self.kit, url_c.as_ptr()) };
        if doc.is_null() {
            let detail = self.last_error().ok().flatten().unwrap_or_default();
            return Err(LokError::new(
                ErrorCode::LoadFailed,
                format!("documentLoad 失败：{url}（LO 错误：{detail}）"),
            ));
        }
        Ok(LokDocument {
            doc,
            kit_class: self.class,
        })
    }

    /// kit 级回调（LOK_CALLBACK_ERROR / DOCUMENT_PASSWORD 等无文档期事件）。
    /// crate 内部面（backend 蹦床用；宿主走 session/client 层——公开面
    /// 不暴露裸指针 data）。
    pub(crate) fn register_kit_callback(
        &self,
        cb: LokCallback,
        data: *mut c_void,
    ) -> LokResult<()> {
        let f = kit_member!(self, register_callback);
        unsafe { f(self.kit, cb, data) };
        Ok(())
    }
}

/// 进入内核主循环（unipoll）。调用线程成为内核主循环线程；空闲时
/// poll 回调被调（在此消费宿主请求 = 主循环上下文同步分发），内核
/// 其他线程经 wake 叫醒。真实模式 worker 的 serve 尾部调用。
/// crate 内部面（worker 集成用；宿主走 session/client 层——公开面
/// 不暴露裸指针 kit）。
pub(crate) fn lok_run_loop(
    kit: *mut LibreOfficeKit,
    poll: LokPollCallback,
    wake: LokWakeCallback,
    data: *mut c_void,
) -> LokResult<()> {
    let class = unsafe { (*kit).class };
    let n_size = unsafe { (*class).n_size };
    let need = std::mem::offset_of!(LibreOfficeKitClass, run_loop) + SLOT;
    if need > n_size {
        return Err(LokError::too_old("run_loop", n_size, need));
    }
    let f = unsafe { (*class).run_loop }
        .ok_or_else(|| LokError::new(ErrorCode::InitFailed, "run_loop 槽位为空"))?;
    unsafe { f(kit, poll, wake, data) };
    Ok(())
}

/// 文档句柄（携带 kit vtable 指针：doc 级字符串释放需 kit 的
/// freeError，钉定头跨 CRT 纪律）。
pub struct LokDocument {
    doc: *mut LibreOfficeKitDocument,
    kit_class: *mut LibreOfficeKitClass,
}

impl LokDocument {
    pub fn doc_nsize(&self) -> usize {
        unsafe { (*(*self.doc).class).n_size }
    }

    pub fn document_type(&self) -> LokResult<i32> {
        let f = doc_member!(self, get_document_type);
        Ok(unsafe { f(self.doc) })
    }

    pub fn parts(&self) -> LokResult<i32> {
        let f = doc_member!(self, get_parts);
        Ok(unsafe { f(self.doc) })
    }

    pub fn set_part(&self, part: i32) -> LokResult<()> {
        let f = doc_member!(self, set_part);
        unsafe { f(self.doc, part) };
        Ok(())
    }

    /// 初始化渲染面（tile 前置；args 为 JSON 串，空串即可）。
    pub fn initialize_for_rendering(&self, args: &str) -> LokResult<()> {
        let f = doc_member!(self, initialize_for_rendering);
        let args_c = CString::new(args)
            .map_err(|_| LokError::new(ErrorCode::InvalidRequest, "render args 含 NUL"))?;
        unsafe { f(self.doc, args_c.as_ptr()) };
        Ok(())
    }

    /// 文档级回调注册（payload 归 LOK 所有，回调内只读不存；crate 内部面）。
    pub(crate) fn register_callback(&self, cb: LokCallback, data: *mut c_void) -> LokResult<()> {
        let f = doc_member!(self, register_callback);
        unsafe { f(self.doc, cb, data) };
        Ok(())
    }

    /// 文档全幅（twips）。
    pub fn document_size(&self) -> LokResult<(i64, i64)> {
        let f = doc_member!(self, get_document_size);
        let (mut w, mut h) = (0i64, 0i64);
        unsafe { f(self.doc, &mut w, &mut h) };
        Ok((w, h))
    }

    pub fn tile_mode(&self) -> LokResult<i32> {
        let f = doc_member!(self, get_tile_mode);
        Ok(unsafe { f(self.doc) })
    }

    /// 画 tile 到 `buffer`（RGBA/BGRA 由 tile_mode() 决定，调用方负责
    /// 解释；canvas 尺寸像素、tile 矩形 twips）。
    #[allow(clippy::too_many_arguments)] // 签名钉定 LibreOfficeKit.h
    pub fn paint_tile(
        &self,
        buffer: &mut [u8],
        canvas_w: i32,
        canvas_h: i32,
        tile_x: i32,
        tile_y: i32,
        tile_w: i32,
        tile_h: i32,
    ) -> LokResult<()> {
        let f = doc_member!(self, paint_tile);
        let need = canvas_w as usize * canvas_h as usize * 4;
        if buffer.len() < need {
            return Err(LokError::new(
                ErrorCode::InvalidRequest,
                format!("tile 缓冲不足：{} < {need}", buffer.len()),
            ));
        }
        unsafe {
            f(
                self.doc,
                buffer.as_mut_ptr(),
                canvas_w,
                canvas_h,
                tile_x,
                tile_y,
                tile_w,
                tile_h,
            )
        };
        Ok(())
    }

    pub fn set_client_zoom(
        &self,
        tile_px_w: i32,
        tile_px_h: i32,
        tile_tw_w: i32,
        tile_tw_h: i32,
    ) -> LokResult<()> {
        let f = doc_member!(self, set_client_zoom);
        unsafe { f(self.doc, tile_px_w, tile_px_h, tile_tw_w, tile_tw_h) };
        Ok(())
    }

    /// 键事件（type 见 lok_enums::KEYEVENT_*；char 为 Unicode 码点）。
    pub fn post_key_event(&self, kind: i32, char_code: i32, key_code: i32) -> LokResult<()> {
        let f = doc_member!(self, post_key_event);
        unsafe { f(self.doc, kind, char_code, key_code) };
        Ok(())
    }

    /// 鼠标事件（坐标 twips；count=2 即双击）。
    #[allow(clippy::too_many_arguments)] // 签名钉定 LibreOfficeKit.h
    pub fn post_mouse_event(
        &self,
        kind: i32,
        x: i32,
        y: i32,
        count: i32,
        buttons: i32,
        modifiers: i32,
    ) -> LokResult<()> {
        let f = doc_member!(self, post_mouse_event);
        unsafe { f(self.doc, kind, x, y, count, buttons, modifiers) };
        Ok(())
    }

    /// UNO 命令（args 为 JSON 串可选；notify=true 时结果经
    /// LOK_CALLBACK_UNO_COMMAND_RESULT 回调返回）。
    pub fn post_uno_command(
        &self,
        command: &str,
        args: Option<&str>,
        notify: bool,
    ) -> LokResult<()> {
        let f = doc_member!(self, post_uno_command);
        let cmd_c = CString::new(command)
            .map_err(|_| LokError::new(ErrorCode::InvalidRequest, "UNO 命令含 NUL"))?;
        let args_c = args
            .map(CString::new)
            .transpose()
            .map_err(|_| LokError::new(ErrorCode::InvalidRequest, "UNO 参数含 NUL"))?;
        unsafe {
            f(
                self.doc,
                cmd_c.as_ptr(),
                args_c.map(|c| c.as_ptr()).unwrap_or(ptr::null()),
                notify,
            )
        };
        Ok(())
    }

    /// 命令值查询（如 ".uno:DocumentSearch"）。
    pub fn get_command_values(&self, command: &str) -> LokResult<Option<String>> {
        let f = doc_member!(self, get_command_values);
        let cmd_c = CString::new(command)
            .map_err(|_| LokError::new(ErrorCode::InvalidRequest, "命令含 NUL"))?;
        let raw = unsafe { f(self.doc, cmd_c.as_ptr()) };
        if raw.is_null() {
            return Ok(None);
        }
        let s = unsafe { CStr::from_ptr(raw).to_string_lossy().into_owned() };
        // getCommandValues/getTextSelection 的返回同样必须经 kit 的
        // freeError 释放（钉定头跨 CRT 纪律）。
        unsafe {
            if let Some(free_error) = (*self.kit_class).free_error {
                free_error(raw);
            }
        }
        Ok(Some(s))
    }

    /// 当前选区文本（mime 如 "text/plain;charset=utf-8"）。
    pub fn get_text_selection(&self, mime: &str) -> LokResult<Option<String>> {
        let f = doc_member!(self, get_text_selection);
        let mime_c = CString::new(mime)
            .map_err(|_| LokError::new(ErrorCode::InvalidRequest, "mime 含 NUL"))?;
        let mut used: *mut c_char = ptr::null_mut();
        let raw = unsafe { f(self.doc, mime_c.as_ptr(), &mut used) };
        if raw.is_null() {
            return Ok(None);
        }
        let s = unsafe { CStr::from_ptr(raw).to_string_lossy().into_owned() };
        unsafe {
            if let Some(free_error) = (*self.kit_class).free_error {
                free_error(raw);
                if !used.is_null() {
                    free_error(used);
                }
            }
        }
        Ok(Some(s))
    }

    /// 另存（format 如 "docx"；options 原样透传，白名单由 worker 把守）。
    /// 返回 LO saveAs 结果：**布尔语义，非 0 = 成功**（钉定头
    /// doc_saveAs `return true`→1；Linux LO 25.2 探针实证 rc=1 时
    /// 产物有效落盘、rc=0 才是失败——勿按 POSIX 0=成功 误读）。
    pub fn save_as(&self, url: &str, format: &str, options: &str) -> LokResult<i32> {
        let f = doc_member!(self, save_as);
        let url_c = CString::new(url)
            .map_err(|_| LokError::new(ErrorCode::InvalidRequest, "保存 URL 含 NUL"))?;
        let fmt_c = CString::new(format)
            .map_err(|_| LokError::new(ErrorCode::InvalidRequest, "format 含 NUL"))?;
        let opt_c = CString::new(options)
            .map_err(|_| LokError::new(ErrorCode::InvalidRequest, "options 含 NUL"))?;
        Ok(unsafe { f(self.doc, url_c.as_ptr(), fmt_c.as_ptr(), opt_c.as_ptr()) })
    }

    pub fn destroy(&self) -> LokResult<()> {
        let f = doc_member!(self, destroy);
        unsafe { f(self.doc) };
        Ok(())
    }
}

/// 系统路径 → file: URL（documentLoad 用；百分号编码非安全字节，
/// Windows 盘符转 file:///C:/... 形式——盘符冒号不编码）。
pub fn path_to_file_url(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let bytes = s.as_bytes();
    let mut out = String::from("file://");
    let mut i = 0;
    // Windows 盘符两形态：裸 "C:/…" 与 POSIX 化的 "/C:/…"。
    let bare_drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    let slashed_drive =
        bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':';
    if bare_drive {
        out.push('/');
        out.push(bytes[0] as char);
        out.push(':');
        i = 2;
    } else if slashed_drive {
        out.push('/');
        out.push(bytes[1] as char);
        out.push(':');
        i = 3;
    }
    for &b in &bytes[i..] {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_url_percent_encodes_spaces_and_cjk() {
        let url = path_to_file_url(Path::new("/tmp/中文 文档.docx"));
        assert!(url.starts_with("file:///tmp/"), "{url}");
        assert!(!url.contains(' '), "空格必须百分号编码：{url}");
        assert!(url.contains(".docx"));
    }

    #[test]
    fn windows_drive_letter_form() {
        let url = path_to_file_url(Path::new(r"C:\Users\t\a.docx"));
        assert_eq!(url, "file:///C:/Users/t/a.docx");
    }
}
