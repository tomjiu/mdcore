//! 拖拽停靠数据面（#121 批③a）——纯逻辑 + 协议 API。
//!
//! ## 蓝本（PROVENANCE）
//!
//! - Q-IDE `app/dock.rs:484-524`（侧区几何）/`:550-567`（挤掉原槽位）
//! - gpui-base `drag.rs:269`（四向 + 中心合并区）
//! - Ely 情报（#139，`layout/dock/view.rs`）：`DraggedPanel` 携带源 dock
//!   `EntityId`，drop 时校验防跨实例误落。
//!
//! ## 本批范围（批③a：数据面）
//!
//! - [`DraggedPanel`]（拖拽 payload）+ [`DockDropZone`]（三分区 + 中心合并）
//! - [`drop_zone_at`]（窗口几何 → 落位分区纯函数）
//! - [`placement_for`]（落位裁决：分区 × 面板合法边 × 偏好位）
//! - [`Dock::take_panel`]（拔出——订阅随行不退订）/[`Dock::receive_panel`]
//!   （落位即激活 = 挤掉原槽位语义）
//!
//! 宿主渲染面（覆盖层渲染/drag 手势挂接/真实鼠标事件链）归批③b，
//! 经本文件 pub API 接线。
//!
//! ## 几何口径（Q-IDE delta 登记）
//!
//! Q-IDE 二分按目标容器中线；本批为**全窗口三分区**口径（#121 正文
//! 「全窗口拖放覆盖层」）：
//!
//! ```text
//! +----+----------+----+
//! | L  |          | R  |   底条带（y > 2/3）全宽优先 → Bottom
//! |    |  Center  |    |   左右条带（x < 1/3 / x > 2/3）次之
//! |    |  合并区  |    |   其余 = Center（取消拖拽回原位）
//! +----+----------+----+
//! |      Bottom      |
//! +-------------------+
//! ```
//!
//! 底全宽优先：拖到窗口下沿任意水平位置都落底（拖拽手感自然——
//! 底部是终值边）；左右只在底条带之上的两翼生效。

use gpui::{Bounds, EntityId, Pixels, Point};

use super::dock::DockPosition;
use super::handle::PanelHandle;

/// 拖拽中的面板 payload（Ely `DraggedPanel` 同构）。
///
/// `source_dock`：拖起面板时所在 dock 实体 id——drop 侧校验
/// [`Dock::receive_panel`] 的目标一致性（防跨实例误落：同一窗口
/// 多个 Shell 实验中拖起的面板不能落进另一个 Shell 的 dock）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DraggedPanel {
    /// 拖起时的源 dock 实体 id。
    pub source_dock: EntityId,
    /// 面板键（`panel_key()` 口径——dock 列内的唯一性面）。
    pub key: &'static str,
}

/// 拖放落位分区（三分区 + 中心合并区）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockDropZone {
    Left,
    Right,
    Bottom,
    /// 中心合并区：取消拖拽，面板回原位（无换侧动作）。
    Center,
}

impl DockDropZone {
    /// 分区对应的 dock 边（中心区 = None，无落位目标）。
    pub fn dock_position(self) -> Option<DockPosition> {
        match self {
            Self::Left => Some(DockPosition::Left),
            Self::Right => Some(DockPosition::Right),
            Self::Bottom => Some(DockPosition::Bottom),
            Self::Center => None,
        }
    }

    /// 分区文案（宿主覆盖层 tooltip/日志面）。
    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "左",
            Self::Right => "右",
            Self::Bottom => "底",
            Self::Center => "取消",
        }
    }
}

/// 光标 → 落位分区（纯几何；窗口 `bounds` 相对坐标外的点按边界
/// clamp 后判定——drop 只发生在窗口内）。
pub fn drop_zone_at(bounds: Bounds<Pixels>, cursor: Point<Pixels>) -> DockDropZone {
    // 相对坐标（0..1；越界 clamp——drop 不会发生在窗口外，防御口径）。
    let width = f32::from(bounds.size.width).max(1.);
    let height = f32::from(bounds.size.height).max(1.);
    let rel_x = (f32::from(cursor.x - bounds.origin.x) / width).clamp(0., 1.);
    let rel_y = (f32::from(cursor.y - bounds.origin.y) / height).clamp(0., 1.);
    if rel_y > 2. / 3. {
        DockDropZone::Bottom
    } else if rel_x < 1. / 3. {
        DockDropZone::Left
    } else if rel_x > 2. / 3. {
        DockDropZone::Right
    } else {
        DockDropZone::Center
    }
}

/// 落位裁决（纯函数——宿主 drop 收尾时的一次性决策）。
///
/// 输入：drop 分区 + 面板偏好位（`position()` 声明）+ 合法边判定
/// （`position_is_valid`——面板声明自己可停靠的边）。输出 [`Placement`]：
///
/// - 中心区 → [`Placement::Cancel`]（回原位，无换侧）
/// - 边分区且面板声明合法 → [`Placement::Dock`]（落位该侧——已占用
///   则挤掉原槽位：落位即激活，原激活面板让位留列）
/// - 边分区但面板不合法（如 outline 只许 Left）→ 回偏好位（偏好位
///   合则落偏好侧，偏好位也非法则取消——面板自相矛盾时保守不动）
pub fn placement_for(
    zone: DockDropZone,
    preferred: DockPosition,
    is_valid: impl Fn(DockPosition) -> bool,
) -> Placement {
    match zone.dock_position() {
        None => Placement::Cancel,
        Some(position) if is_valid(position) => Placement::Dock(position),
        Some(_) if is_valid(preferred) => Placement::Dock(preferred),
        Some(_) => Placement::Cancel,
    }
}

/// 落位裁决结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// 换侧落位（宿主：`take_panel`（源）→ `receive_panel`（目标）+
    /// `set_position` + 持久化）。
    Dock(DockPosition),
    /// 取消（无动作——面板留源 dock 原位）。
    Cancel,
}

/// 拔出的面板（拖拽中转态——订阅随行不退订，落位/回位原样带回）。
pub struct PanelDetachment {
    /// 面板句柄（源 dock `entries` 原件转移）。
    pub panel: std::sync::Arc<dyn PanelHandle>,
    /// 随行订阅（面板事件订阅——drop 拖拽中途不退订，落位侧接续）。
    pub subscriptions: Vec<gpui::Subscription>,
    /// 随行记忆尺寸（面板的 per-panel 记忆随面板走，落位侧继承——
    /// 验收「拖拽后布局持久化 round-trip」的尺寸面）。
    pub size: Option<f32>,
}

/// 跨实例误落校验（Ely 情报第 3 点：`DraggedPanel` 携带源 dock
/// `EntityId`，drop 时与目标 dock 比对）。
///
/// 源 ≠ 目标 = 跨侧移动（正常落位）；源 == 目标 = 拖回源侧
///（面板本来就在这边——按取消处理，宿主不动面板）。
pub fn drop_is_intended(dragged: DraggedPanel, target_dock: EntityId) -> bool {
    dragged.source_dock != target_dock
}
