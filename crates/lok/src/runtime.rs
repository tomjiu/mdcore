//! LOK 运行时发现（issue 第 2 条：明确发现方式、精确版本/来源/hash）。
//!
//! ## 隔离铁律
//!
//! **宿主进程只做路径发现 + 文件哈希，永不 dlopen 内核库**——dlopen 会执行
//! 库静态构造器，LO 内核崩溃面必须全部圈在 worker 进程内（issue「不因
//! fallback 在宿主再执行崩溃的内核」的结构性保证）。内核库加载与
//! hook 符号解析只发生在 worker 进程（[`crate::worker`]），版本信息由
//! worker 经 `version` 请求回报宿主。
//!
//! ## install_path 契约（实测实证，2026-11-09）
//!
//! `libreofficekit_hook_2(install_path, profile_url)` 的 **install_path 是
//! `program` 目录**（`lo_initialize` 内部：`BRAND_BASE_DIR = install/..`、
//! `setIniFilename(install + "/sofficerc")`、Debian `CONFIGURATION_LAYERS`
//! 逐层引用 fundamentalrc/bootstraprc——传安装根会让 BRAND_BASE_DIR 错指
//! 上一级、sofficerc 落空，bootstrap 抛
//! `component context fails to supply singleton ...theDefaultProvider`
//! 后段错误；官方头文件注释「含 program/share 的安装目录」与实现不符，
//! 以实现为准）。本机实证：`/usr/lib/libreoffice/program` → Kit 有效、
//! `nSize=208`、`getVersionInfo` = JSON `LibreOffice 25.2(.3.2)`。
//!
//! ## 发现序（对齐 mdc-pdf `engine_pdfium.rs` 定位序先例：env → 部署位 →
//! 系统常见位 → 失败列全部已试路径）
//!
//! 1. `MDC_LOK_HOME`（LibreOffice 安装根：含 `program/`、`share/` 的目录）
//! 2. `MDC_LOK_LIB`（直接指定内核库文件，program 目录取其父目录）
//! 3. 部署位：exe 同目录 `LOK/`、`libreoffice/` 与仓内 `bin/`、`lib/`
//! 4. 平台常见安装位（见 [`platform_roots`]）
//!
//! ## 内核库文件选择序（官方 `LibreOfficeKitInit.h` 的 `lok_dlopen` 引导）
//!
//! `<HOME>/program/sofficeapp.{so,dll}` 存在且 >1000 字节 → 用之；
//! 否则 `<HOME>/program/libmergedlo.{so,dll}`（Windows：ElyOffice 自建
//! `--enable-mergelibs` 版为 `mergedlo.dll`，官方 TDF 包为 `sofficeapp.dll`
//! 优先）。macOS：`<HOME>/Contents/Frameworks/libmergedlo.dylib`、
//! program 目录 `<HOME>/Contents/program`（.app 包）。
//!
//! ## 入口符号（worker 内解析）
//!
//! `libreofficekit_hook_2`（官方引导头 `dlsym` 的符号；Debian 构建的
//! `libmergedlo.so` **不导出** `lok_init_2`，`objdump -T` 实证）。老运行时
//! 无 hook_2 只有 `libreofficekit_hook`（不支持独立 user profile → 多会话
//! 互锁）——issue 明文「共享平台 profile 不可复用」，**直接拒绝**并给
//! 明确错误。
//!
//! ## Windows 复现清单（实测门未执行——如实注明）
//!
//! ```text
//! 1. 安装官方 TDF LibreOffice（如 25.2.x）默认目录，或设 MDC_LOK_HOME。
//! 2. 确认 <HOME>\program\sofficeapp.dll 为真实 DLL（官方包 >1000 字节）；
//!    若发行版只有 libmergedlo 形态，用 dumpbin /exports 或
//!    objdump -T 确认 libreofficekit_hook_2 导出后再用 MDC_LOK_LIB 指定。
//! 3. cargo test -p mdc-lok --test lok_smoke -- --nocapture
//! 4. 已知未实测风险面（ElyOffice 实证，本 crate 已内置防御但未经实机）：
//!    - LO 内核读 fd 0 → worker 启动即私有化 stdin（worker 模块 win 面）
//!    - 系统按 per-monitor DPI 启动 → tile 偏移 → SetThreadDpiAwareness
//!    - liblangtag LT_LOCK 句柄泄漏（ElyOffice 补丁 0060；官方包未带，
//!      长会话反复开关文档可能句柄泄漏——冒烟一轮不受影响）
//! ```

use crate::sha256::sha256_hex_file;
use std::path::{Path, PathBuf};

/// 运行时环境变量：LibreOffice 安装根（含 program/ 子目录）。
pub const LOK_HOME_ENV: &str = "MDC_LOK_HOME";
/// 运行时环境变量：直接指定内核库文件。
pub const LOK_LIB_ENV: &str = "MDC_LOK_LIB";

/// 已发现的 LOK 运行时（仅路径与指纹；内核加载在 worker 进程内）。
#[derive(Debug, Clone)]
pub struct LokRuntime {
    /// hook_2 的 install_path：**program 目录**（见模块头「install_path 契约」）。
    pub program_dir: PathBuf,
    /// 将要加载的内核库文件（worker 内 dlopen）。
    pub kernel_lib: PathBuf,
    /// 内核库文件 SHA-256（发现期即可计算，issue「精确来源/hash」验收项）。
    pub kernel_sha256: String,
}

/// 平台常见安装根候选（无 env 时的兜底发现序）。
fn platform_roots() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if cfg!(windows) {
        for env in ["PROGRAMFILES", "PROGRAMFILES(X86)", "PROGRAMW6432"] {
            if let Some(base) = std::env::var_os(env) {
                v.push(PathBuf::from(base).join("LibreOffice"));
            }
        }
    } else if cfg!(target_os = "macos") {
        v.push(PathBuf::from("/Applications/LibreOffice.app"));
    } else {
        v.push(PathBuf::from("/usr/lib/libreoffice"));
        v.push(PathBuf::from("/opt/libreoffice"));
        v.push(PathBuf::from("/usr/lib64/libreoffice"));
    }
    v
}

/// exe 同目录与仓内部署位候选（照 pdfium 候选序：部署位最前）。
fn deploy_roots() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // 部署形态：<bin>/LOK/ 或 <bin>/libreoffice/（产品把 LO 摆在旁边）
            v.push(dir.join("LOK"));
            v.push(dir.join("libreoffice"));
        }
    }
    // 仓内开发取件位（CARGO_MANIFEST_DIR 的上两级 = 仓根）
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        if let Some(root) = Path::new(&manifest).ancestors().nth(2) {
            v.push(root.join("bin"));
            v.push(root.join("lib"));
        }
    }
    v
}

/// 安装根 →（program 目录， 内核库候选）。内核库选择序 = 官方
/// `lok_dlopen` 引导（sofficeapp 存在且 >1000 字节优先，否则 merged）。
fn root_layout(root: &Path) -> Option<(PathBuf, PathBuf)> {
    let (program_dir, kernel_dir): (PathBuf, PathBuf) = if cfg!(target_os = "macos") {
        let contents = root.join("Contents");
        (contents.join("program"), contents.join("Frameworks"))
    } else {
        let program = root.join("program");
        (program.clone(), program)
    };
    let (sofficeapp, merged) = if cfg!(windows) {
        ("sofficeapp.dll", "mergedlo.dll")
    } else {
        ("sofficeapp.so", "libmergedlo.so")
    };
    // 官方引导序：sofficeapp 存在且 >1000 字节优先（小桩文件回退 merged）
    let a = kernel_dir.join(sofficeapp);
    let ok_size = std::fs::metadata(&a)
        .map(|m| m.len() > 1000)
        .unwrap_or(false);
    let kernel = if ok_size { a } else { kernel_dir.join(merged) };
    if kernel.is_file() && program_dir.is_dir() {
        return Some((program_dir, kernel));
    }
    None
}

/// 逐候选发现（纯路径逻辑，不加载任何库）。
pub fn discover_paths() -> Result<(PathBuf, PathBuf), Vec<String>> {
    let mut tried: Vec<String> = Vec::new();

    // 1) MDC_LOK_HOME（安装根）
    if let Some(home) = std::env::var_os(LOK_HOME_ENV) {
        let home = PathBuf::from(home);
        match root_layout(&home) {
            Some(x) => return Ok(x),
            None => tried.push(format!("{home:?}（{LOK_HOME_ENV} 无 program/内核库）")),
        }
    }

    // 2) MDC_LOK_LIB（直接内核库文件；program 目录取其父目录，
    //    macOS 特例 = 祖父/Contents/program——.app 包布局）
    if let Some(lib) = std::env::var_os(LOK_LIB_ENV) {
        let lib = PathBuf::from(lib);
        if lib.is_file() {
            let program_dir = if cfg!(target_os = "macos") {
                lib.parent()
                    .and_then(Path::parent)
                    .map(|p| p.join("program"))
                    .unwrap_or_else(|| lib.parent().map(Path::to_path_buf).unwrap_or_default())
            } else {
                lib.parent().map(Path::to_path_buf).unwrap_or_default()
            };
            if program_dir.is_dir() {
                return Ok((program_dir, lib));
            }
            tried.push(format!("{lib:?}（{LOK_LIB_ENV} 父目录不是 program 目录）"));
        } else {
            tried.push(format!("{lib:?}（{LOK_LIB_ENV} 不是文件）"));
        }
    }

    // 3+4) 部署位 + 平台常见位
    for root in deploy_roots().into_iter().chain(platform_roots()) {
        match root_layout(&root) {
            Some(x) => return Ok(x),
            None => tried.push(format!("{root:?}（无 program/内核库）")),
        }
    }
    Err(tried)
}

/// 发现运行时（路径 + 指纹；不加载内核——见模块头「隔离铁律」）。
pub fn discover() -> Result<LokRuntime, String> {
    let (program_dir, kernel_lib) = discover_paths().map_err(|tried| {
        format!(
            "未发现 LibreOffice 运行时。已尝试：{}\n\
             补救：安装 LibreOffice 或设 {LOK_HOME_ENV}=<安装根> / {LOK_LIB_ENV}=<内核库>。",
            tried.join("；")
        )
    })?;
    let sha =
        sha256_hex_file(&kernel_lib).map_err(|e| format!("读取 {kernel_lib:?} 哈希失败：{e}"))?;
    Ok(LokRuntime {
        program_dir,
        kernel_lib,
        kernel_sha256: sha,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_in_sandbox_or_skip() {
        // 环境门（AGENTS §13：缺运行时 → SKIP 横幅；恒跑逻辑层在 protocol 测试）
        if std::env::var_os(LOK_HOME_ENV).is_none()
            && std::env::var_os(LOK_LIB_ENV).is_none()
            && !Path::new("/usr/lib/libreoffice/program/libmergedlo.so").is_file()
        {
            eprintln!(
                "[lok] SKIP runtime::discover_in_sandbox_or_skip —— 本机无 LibreOffice \
                 运行时；补救：安装或设 {LOK_HOME_ENV}"
            );
            return;
        }
        let rt = discover().expect("沙盒内应能发现 LibreOffice 25.2.3.2 运行时");
        assert!(rt.kernel_lib.is_file());
        assert_eq!(rt.kernel_sha256.len(), 64);
        // install_path 契约：hook_2 收到的是 program 目录（sofficerc 所在）
        assert!(
            rt.program_dir.join("sofficerc").is_file()
                || rt.program_dir.join("soffice.rc").is_file()
                || rt.program_dir.join("soffice.ini").is_file(),
            "program 目录必须有平台对应的 soffice bootstrap 入口：{:?}",
            rt.program_dir
        );
    }

    #[test]
    fn env_lib_overrides_and_program_dir_is_parent() {
        // 纯逻辑：MDC_LOK_LIB 指向任意存在文件 → 直接采用，program 目录取父目录
        let tmp = std::env::temp_dir().join("mdc-lok-ut-fake");
        std::fs::create_dir_all(tmp.join("program")).unwrap();
        let fake = tmp.join("program").join("libmergedlo.so");
        std::fs::write(&fake, b"x").unwrap();
        // 隔离调用者的实际安装根，测试显式库路径分支；不改变生产发现优先级。
        let previous_home = std::env::var_os(LOK_HOME_ENV);
        let previous_lib = std::env::var_os(LOK_LIB_ENV);
        unsafe { std::env::remove_var(LOK_HOME_ENV) };
        unsafe { std::env::set_var(LOK_LIB_ENV, &fake) };
        let rt = discover().expect("env 指定应直接采用");
        assert_eq!(rt.kernel_lib, fake);
        assert_eq!(rt.program_dir, tmp.join("program"));
        assert_eq!(rt.kernel_sha256.len(), 64);
        unsafe {
            match previous_lib {
                Some(value) => std::env::set_var(LOK_LIB_ENV, value),
                None => std::env::remove_var(LOK_LIB_ENV),
            }
            if let Some(value) = previous_home {
                std::env::set_var(LOK_HOME_ENV, value);
            }
        };
        let _ = std::fs::remove_dir_all(tmp);
    }
}
