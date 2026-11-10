//! 平台动态加载（零三方依赖：unix dlopen / win32 LoadLibraryExW）。
//!
//! 与 ElyOffice `ely-lok/src/ffi.rs` 同源结论：LOK 内核库在 Linux 为
//! `program/libmergedlo.so`，Windows 为 `program/mergedlo.dll`（若发行版
//! 走 `--enable-mergelibs`）；官方 `LibreOfficeKitInit.h` 的 `lok_dlopen`
//! 引导序为「先试 `program/sofficeapp.{dll,so}`，存在且 >1000 字节则用之，
//! 否则用 merged 库」——本 crate 不重复该引导序（那是**内核库文件**的
//! 选择），本模块加载的就是 [`crate::runtime`] 按引导序选出的那一个库文件。

use std::ffi::{c_void, CStr};
use std::os::raw::c_char;

#[cfg(unix)]
mod imp {
    use super::{c_char, c_void};
    use std::ffi::CStr;
    use std::os::raw::c_int;

    /// dlopen 选项：Unix `RTLD_LAZY | RTLD_GLOBAL`（LO 符号跨 so 解析需要
    /// GLOBAL；ElyOffice 同参数）。
    const RTLD_LAZY: c_int = 0x1;
    const RTLD_GLOBAL: c_int = 0x100;

    // 自写 FFI：不引 libc（仓库零依赖纪律；glibc>=2.34 起 dlopen 在 libc
    // 内，老系统经 libdl 兜底——两个链接名都声明，由链接器择一）。
    extern "C" {
        fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlclose(handle: *mut c_void) -> c_int;
        fn dlerror() -> *mut c_char;
    }

    pub struct RawLib(*mut c_void);

    pub fn open(path: &std::path::Path) -> Result<RawLib, String> {
        let c = std::ffi::CString::new(path.to_string_lossy().as_bytes())
            .map_err(|e| format!("路径含 NUL: {e}"))?;
        unsafe {
            // 清除本线程上一次的 dlerror 残留
            dlerror();
            let h = dlopen(c.as_ptr(), RTLD_LAZY | RTLD_GLOBAL);
            if h.is_null() {
                let err = dlerror();
                let msg = if err.is_null() {
                    "dlopen 失败（dlerror 无信息）".to_string()
                } else {
                    CStr::from_ptr(err).to_string_lossy().into_owned()
                };
                return Err(msg);
            }
            Ok(RawLib(h))
        }
    }

    impl RawLib {
        pub fn sym(&self, name: &str) -> Result<*mut c_void, String> {
            let c = std::ffi::CString::new(name).map_err(|e| format!("符号名含 NUL: {e}"))?;
            unsafe {
                dlerror();
                let s = dlsym(self.0, c.as_ptr());
                if s.is_null() {
                    let err = dlerror();
                    let msg = if err.is_null() {
                        format!("dlsym({name}) 返回 NULL")
                    } else {
                        CStr::from_ptr(err).to_string_lossy().into_owned()
                    };
                    return Err(msg);
                }
                Ok(s)
            }
        }
    }

    impl Drop for RawLib {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { dlclose(self.0) };
            }
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::c_void;
    use std::os::raw::c_int;

    // win32 FFI（零依赖自写；winapi 不引）。
    type HMODULE = *mut c_void;
    type DWORD = u32;
    type LPCSTR = *const u8;
    type BOOL = c_int;

    const LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR: DWORD = 0x100;
    const LOAD_LIBRARY_SEARCH_DEFAULT_DIRS: DWORD = 0x1000;

    #[allow(non_snake_case)]
    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryExA(lpLibFileName: LPCSTR, hFile: *mut c_void, dwFlags: DWORD) -> HMODULE;
        fn SetDllDirectoryA(lpPathName: LPCSTR) -> BOOL;
        fn GetProcAddress(hModule: HMODULE, lpProcName: LPCSTR) -> *mut c_void;
        fn FreeLibrary(hModule: HMODULE) -> BOOL;
        fn GetLastError() -> DWORD;
    }

    pub struct RawLib(HMODULE);

    pub fn open(path: &std::path::Path) -> Result<RawLib, String> {
        // LO 的 DLL 全在 program/ 同目录：LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR
        // 让依赖 DLL 解析落在同目录（ElyOffice 同参数）。
        //
        // #69 Windows 实测补充：仅 DLL_LOAD_DIR 不够——mergedlo.dll 的依赖
        // 解析报 GetLastError=126（ERROR_MOD_NOT_FOUND）。原因是 LO 26.x 的
        // 内核还会延迟加载同目录的其他组件 DLL，而标准搜索路径不含 program/。
        // 修复：加载前 SetDllDirectoryA(program 目录)（加入搜索序），
        // 加载后还原空（不污染进程级搜索路径）。SetDllDirectory 同时隐式
        // 启用 LOAD_LIBRARY_SEARCH_USER_DIRS 语义，双保险。
        let dir = path.parent().unwrap_or(path);
        let dir_c = std::ffi::CString::new(dir.to_string_lossy().as_bytes())
            .map_err(|e| format!("目录路径含 NUL: {e}"))?;
        let c = std::ffi::CString::new(path.to_string_lossy().as_bytes())
            .map_err(|e| format!("路径含 NUL: {e}"))?;
        unsafe {
            SetDllDirectoryA(dir_c.as_ptr().cast());
            let h = LoadLibraryExA(
                c.as_ptr().cast(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            );
            // 还原搜索路径（NULL = 恢复默认），失败也继续用句柄
            SetDllDirectoryA(std::ptr::null());
            if h.is_null() {
                return Err(format!(
                    "LoadLibraryExA 失败：GetLastError={}（dir={:?}）",
                    GetLastError(),
                    dir
                ));
            }
            Ok(RawLib(h))
        }
    }

    impl RawLib {
        pub fn sym(&self, name: &str) -> Result<*mut c_void, String> {
            let mut buf: Vec<u8> = name.as_bytes().to_vec();
            buf.push(0);
            unsafe {
                let s = GetProcAddress(self.0, buf.as_ptr());
                if s.is_null() {
                    return Err(format!(
                        "GetProcAddress({name}) 返回 NULL：GetLastError={}",
                        GetLastError()
                    ));
                }
                Ok(s)
            }
        }
    }

    impl Drop for RawLib {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { FreeLibrary(self.0) };
            }
        }
    }
}

/// 已加载的动态库句柄（包装平台实现）。
pub struct Library(imp::RawLib);

impl Library {
    /// 打开动态库。`path` 必须是绝对路径（运行时发现序已保证）。
    pub fn open(path: &std::path::Path) -> Result<Library, String> {
        imp::open(path).map(Library)
    }

    /// 取符号地址（dlsym / GetProcAddress）。
    pub fn sym(&self, name: &str) -> Result<*mut c_void, String> {
        self.0.sym(name)
    }
}

/// C 字符串工具：从 LOK 返回的 `char*`（UTF-8）安全取回。
///
/// # Safety
///
/// `ptr` 必须指向一个以 NUL 结尾的合法 UTF-8 C 字符串（LOK 约定 UTF-8），
/// 且在函数返回前保持有效；仅做拷贝，不释放（释放必须走 kit 的
/// `freeError`，见 worker 模块的跨 CRT 纪律）。
pub unsafe fn cstr_take(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let s = CStr::from_ptr(ptr).to_string_lossy().into_owned();
    Some(s)
}
