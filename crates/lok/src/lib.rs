//! mdc-lok — LibreOfficeKit 隔离 worker 原型（#633，2026-11-09）
//!
//! 规格 = issue #633 正文（派单人 Agent-n8q7）一手裁决 + LibreOffice 官方
//! `include/LibreOfficeKit/LibreOfficeKit.h`（pin commit
//! `0d51fdc57aef6707466320821b8c2931c00297c0`）与 ElyOffice
//! `crates/ely-lok`/`ely-ipc`（commit `dc32744d`）最少运行时审查。
//!
//! ## 职责边界（一个原型 PR 只做 worker 与 smoke）
//!
//! | 面 | 本 crate | 不做（归属） |
//! |---|---|---|
//! | LOK 动态加载 | ✅ [`runtime`]（发现序 + hook_2 引导） | — |
//! | 单文档 worker | ✅ [`worker`] + `mdc-lok-worker` bin | GPUI 产品输入/菜单/IME（#616） |
//! | 宿主客户端 | ✅ [`client`]（请求ID/超时/帧限/脏位） | 标签保存/关闭闭环（#616） |
//! | 真实运行时 smoke | ✅ `mdc-lok-smoke` bin + [`tests/lok_smoke.rs`] | 保存损坏修复（#629） |
//! | PDF | — | 既有 PDFium 管线 |
//!
//! ## 真实门执行状态（诚实声明，绿 CI 不能替代实机验收）
//!
//! - ✅ **Linux 实测**（本沙盒）：LibreOffice 25.2.3.2（Debian 13
//!   `libreoffice-core 4:25.2.3-2+deb13u8`），内核库
//!   `/usr/lib/libreoffice/program/libmergedlo.so` 导出
//!   `libreofficekit_hook_2`（`objdump -T` 实证；Debian 构建不导出
//!   `lok_init_2`——正确入口就是 hook，与官方 LibreOfficeKitInit.h 的
//!   `lok_dlopen`+`dlsym("libreofficekit_hook_2")` 引导序一致）。
//! - ⛔ **Windows 实测门未执行**（无实机）：运行时发现序 / `LoadLibraryExW`
//!   加载 / stdin 私有化（LO 内核会读 fd 0，ElyOffice `worker/stdin.rs`
//!   实证） / DPI unaware（tile 假定 96 DPI）代码已实现并纳入
//!   `cargo check --target x86_64-pc-windows-msvc` 编译面，但**未经
//!   Windows 实机验证**。复现清单见 [`runtime`] 模块文档。
//! - ⛔ macOS 实测门未执行（同上）。
//!
//! ## ABI 防漂移（issue「nSize/member 检查」要求）
//!
//! [`abi`] 按 pin 头文件成员序手写 `#[repr(C)]` 类结构，**每次调用前**
//! `has_member`（`std::mem::offset_of` + 运行时 `nSize`）逐成员守卫。
//! 实证：Doc 类成员 0-18 与 Kit 类成员 0-12 在 pin 头与 25.2.3.2 头
//! 完全同序（`paste` 成员两版同在 #19 位），本 crate 绑定面即止于此。
//! LO 26.x 曾发生成员中段插入（ElyOffice slot 注释实证），中段之后的
//! 成员一律不绑——需要时先扩 `has_member` 守卫再扩成员。
//!
//! ## 脏位语义（issue 第 4 条）
//!
//! 宿主侧 [`client::DocHandle`] 维护 `dirty`：任何成功的输入/UNO 变更
//! 置位；**只有** `save_as` 成功才清位；超时 / worker 异常 / 保存失败
//! 一律不清位（issue 明文）。宿主绝不在进程内 fallback 重跑 LOK 内核
//! （worker 崩溃 = 明确错误 + 脏位保留 + 可重开会话）。
//!
//! ## 三域纪律
//!
//! 对 mdcore / mdc-office 零 path 依赖；LOK 是 Office 域的**外部运行时**，
//! 本 crate 只做协议与隔离，不引入第二套文档模型（#309 自研排版后置）。

pub mod abi;
pub mod bmp;
pub mod client;
pub mod ffi;
pub mod profile;
pub mod protocol;
pub mod runtime;
pub mod sha256;
pub mod smoke;
pub mod worker;

/// 帧长上限（字节）。响应与事件行超过即拒：worker 回 `E_OVERSIZE`
/// 错误帧并存活；宿主对请求行同样拒发。tile 像素走 side file 不走帧。
pub const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;

/// smoke / 测试用的 fixture 文档（中文锚文本 + 表格 + 内嵌图片）。
pub fn smoke_fixture_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/smoke-chars-img-table.docx")
}
