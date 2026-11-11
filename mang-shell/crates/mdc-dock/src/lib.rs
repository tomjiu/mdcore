//! mdc-dock —— dock 体系（#119 批①）。
//!
//! ## 蓝本（PROVENANCE）
//!
//! zed `crates/workspace/src/dock.rs`（rev `1663af4`，1767 行）经
//! mang-shell 铅锤移植——非物理搬运（`client::proto`/`db::kvp`/
//! `Workspace` 纠缠面解绑，见同目录 [`PROVENANCE.md`]）。Lead 裁决：
//! #105 评论（2026-11-11 方案 C）。
//!
//! ## 本批范围（#119）
//!
//! - [`Panel`]（行为层 trait）+ [`PanelEvent`]：蓝本 dock.rs:36
//! - [`PanelHandle`]（对象安全镜像）+ `PanelId`：蓝本 dock.rs:106
//! - [`DockPosition`] + [`Dock`] 实体（每侧 `Vec` 面板 + 激活协议）：
//!   蓝本 dock.rs:283/:322/:936/:550
//! - [`PanelRegistry`]：按 persistent_name 重建实体（布局恢复预留——
//!   批② 结构化持久化的消费面，Q-IDE app/dock.rs:95 模式）
//!
//! ## 批次边界
//!
//! 持久化/版本迁移（#120 批②）、拖拽停靠/drop zone（#121 批③）、
//! zoom、RTL 不在本批。[`Dock`] 的 `set_open`/`is_open` 协议面先行
//! 就位（蓝本 dock.rs:576），持久化接线归批②。
//!
//! ## 纪律
//!
//! 纯逻辑（`ActiveTracker`/键查询/注册表）无 gpui 实体依赖可单测；
//! 涉实体的激活协议用 `#[gpui::test]`（test-support 特性只进
//! dev-deps）。本 crate 不引用 mang-shell 任何模块——依赖方向单向
//! （mang-shell → mdc-dock），mang-shell 测试树不被反向传染。

mod dock;
mod handle;
mod panel;
mod registry;

#[cfg(test)]
mod tests;

pub use dock::{ActiveTracker, Dock, DockPosition, PanelEntry};
pub use handle::{PanelHandle, PanelId};
pub use panel::{Panel, PanelEvent};
pub use registry::{PanelBuilder, PanelRegistry};
