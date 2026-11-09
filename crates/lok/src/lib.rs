//! mdc-lok —— LibreOfficeKit 隔离 worker 原型（#633）。
//!
//! # 定位（AGENTS 纯引擎面声明）
//!
//! 本 crate 只做「可运行 worker 与 smoke」（#633 验收原文）；GPUI
//! 产品输入、菜单、剪贴板、IME、标签保存/关闭及图片视觉验收由
//! **#616** 接续。本 crate 对 mdcore 零依赖、不进 wasm 闭包。
//!
//! # 组成
//!
//! - [`ffi`]：钉定头（LibreOffice/core@0d51fdc）vtable + nSize 运行时
//!   成员检查 + 动态加载序 + freeError 跨 CRT 纪律 + runLoop。
//! - [`proto`]：stdio 帧协议（magic/kind/req_id/长度上限/二进制尾）。
//! - [`backend`]/[`mock`]：DocSession trait，真实 LOK 与 mock 同构。
//! - [`worker`]（+ `src/bin/mdc_lok_worker.rs`）：单文档 worker 进程
//!   （unipoll：请求在内核主循环上下文同步分发）。
//! - [`client`]：宿主句柄（spawn/请求 ID/超时/事件/kill/重拉）。
//! - [`session`]：DocumentSession——脏位语义 + 保存安全链
//!   （临时产物 → 校验 → 决定替换；源文件只读）。
//! - [`verify`]：保存产物结构校验（最小 zip 读取器）。
//!
//! # 测试面诚实边界
//!
//! - 协议/边界测试（`tests/boundary.rs`）：**无运行时可跑**——mock
//!   worker 是真实子进程，kill/超时/拒写/脏位全部真测；
//! - 真实运行时门（`tests/lok_real.rs` + `examples/smoke.rs`）：需要
//!   LibreOffice（发现序 `MDC_LOK_DIR` → 平台候选）；缺运行时 = SKIP
//!   横幅（不是 PASS）。Windows 实机清单与未执行门见 `docs/lok-runtime.md`。
//!
//! # 运行时发现（摘）
//!
//! - Linux（Debian 系实证）：`<安装根>/program/libmergedlo.so` 导出
//!   `libreofficekit_hook_2`；install_path 传 **program 目录**。
//! - Windows：`%ProgramFiles%\LibreOffice\program\mergedlo.dll`
//!   （LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR 同目录解析依赖）。

pub mod backend;
pub mod client;
pub mod error;
pub mod ffi;
pub mod mock;
pub mod proto;
pub mod session;
pub mod verify;
pub mod worker;

pub use client::{LokEventMsg, LokRuntimeConfig, LokWorker, DEFAULT_TIMEOUT};
pub use error::{ErrorCode, LokError, LokResult};
pub use session::{DocumentSession, SaveReceipt, Tile};
pub use verify::{verify_saved_docx, VerifyReport, VerifySpec};
