//! DockAreaState 结构化持久化——数据面（#120 批②）。
//!
//! 参照：Q-IDE `layout.rs:20-22/:525-662`（版本号 + 全量快照 + 坏值
//! 容忍）与 zed `dock.rs:1043-1060`（PanelSizeState 尺寸面）。serde
//! 结构形状（字段名/tag）发布后冻结——布局文件兼容性依赖（Q-IDE
//! layout.rs 头注释同款纪律）。dump/restore 的实体协议面在
//! [`super::dock`]（私有字段同文件访问）。

use serde::{Deserialize, Serialize};

/// 布局版本号（Q-IDE layout.rs:20 `LAYOUT_VERSION` 口径）。
///
/// **升级迁移语义**：`Dock::restore` 遇版本不符整份丢弃回默认（不崩
/// ——蓝本 dock.rs:518-521 `if layout.layout_version < LAYOUT_VERSION
/// { dock_state.clear() }` 同款裁决）。升版 = 显式改此常量 + 宿主侧
/// 复跑持久化测试面。
pub const DOCK_LAYOUT_VERSION: u32 = 1;

/// 单面板持久化条目：身份键 + 记忆尺寸（px；`None` = 无记忆——
/// 面板注册后未拖拽过宽度）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelState {
    pub key: String,
    pub size: Option<f32>,
}

/// 单侧 dock 持久化快照。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DockAreaState {
    /// 布局版本（写入时恒 [`DOCK_LAYOUT_VERSION`]；恢复时校验）。
    pub version: u32,
    /// 开合态（蓝本 dock.rs:283 `is_open` 字段直存）。
    pub open: bool,
    /// 激活面板键（仅当下面板恢复后经 `Dock::restore` 真激活——
    /// 未物化键静默丢弃，坏键不崩）。
    pub active: Option<String>,
    /// 面板条目（按注册序——dump 时逐条快照）。
    pub panels: Vec<PanelState>,
}

impl DockAreaState {
    /// 默认布局：开、无激活、空面板集（版本不符丢弃后的重建基线）。
    pub fn default_layout() -> Self {
        Self {
            version: DOCK_LAYOUT_VERSION,
            open: true,
            active: None,
            panels: Vec::new(),
        }
    }
}

/// 版本不符（携带实得/期望两值——宿主诊断面）。
#[derive(Clone, Debug, PartialEq)]
pub struct DockLayoutVersionMismatch {
    pub found: u32,
    pub expected: u32,
}

impl std::fmt::Display for DockLayoutVersionMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "dock 布局版本不符：实得 {}（期望 {}）——整份丢弃回默认",
            self.found, self.expected
        )
    }
}

impl std::error::Error for DockLayoutVersionMismatch {}
