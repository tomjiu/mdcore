//! 面板对象安全镜像（#119 批①）——蓝本 zed `dock.rs:106-152` `PanelHandle`
//! + blanket 实现 `dock.rs:154-281`。
//!
//! dock 实体持有 `Arc<dyn PanelHandle>`（蓝本 dock.rs:285 的
//! `Vec<PanelEntry>` 同构），经本镜像调用具体面板——`Entity<T: Panel>`
//! 自动获得实现（蓝本 dock.rs:154 模式）。

use gpui::{AnyView, App, Entity, EntityId, FocusHandle};

use super::dock::DockPosition;
use super::panel::Panel;

/// 面板标识（蓝本 `pub use proto::PanelId`——协作 protobuf 解绑为
/// EntityId 派生新类型，仿 gpui-base `layout/node.rs:33` `From<EntityId>`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PanelId(EntityId);

impl From<EntityId> for PanelId {
    fn from(id: EntityId) -> Self {
        Self(id)
    }
}

impl PanelId {
    /// 面板所属实体 id（批② 持久化映射/批③ 拖拽命中用）。
    pub fn as_entity_id(self) -> EntityId {
        self.0
    }
}

/// 对象安全镜像：Dock 侧调用面板的唯一通道。
pub trait PanelHandle: Send + Sync {
    fn panel_id(&self) -> PanelId;
    fn persistent_name(&self) -> &'static str;
    fn panel_key(&self) -> &'static str;
    fn position(&self, cx: &App) -> DockPosition;
    fn position_is_valid(&self, position: DockPosition, cx: &App) -> bool;
    fn set_position(&self, position: DockPosition, cx: &mut App);
    fn enabled(&self, cx: &App) -> bool;
    fn visible(&self, cx: &App) -> bool;
    fn closable(&self, cx: &App) -> bool;
    fn zoomable(&self, cx: &App) -> bool;
    fn starts_open(&self, cx: &App) -> bool;
    fn activation_priority(&self, cx: &App) -> u32;
    fn activation_focus_handle(&self, cx: &App) -> FocusHandle;
    /// 激活通知（协议去重后经此下发——蓝本 dock.rs:115）。
    fn set_active(&self, active: bool, cx: &mut App);
    /// 任意视图化（downcast 取回具体面板实体的通道——蓝本 dock.rs:132）。
    fn to_any(&self) -> AnyView;
}

impl<T> PanelHandle for Entity<T>
where
    T: Panel,
{
    fn panel_id(&self) -> PanelId {
        PanelId::from(self.entity_id())
    }

    fn persistent_name(&self) -> &'static str {
        T::persistent_name()
    }

    fn panel_key(&self) -> &'static str {
        T::panel_key()
    }

    fn position(&self, cx: &App) -> DockPosition {
        self.read(cx).position()
    }

    fn position_is_valid(&self, position: DockPosition, cx: &App) -> bool {
        self.read(cx).position_is_valid(position)
    }

    fn set_position(&self, position: DockPosition, cx: &mut App) {
        self.update(cx, |this, cx| this.set_position(position, cx));
    }

    fn enabled(&self, cx: &App) -> bool {
        self.read(cx).enabled(cx)
    }

    fn visible(&self, cx: &App) -> bool {
        self.read(cx).visible(cx)
    }

    fn closable(&self, cx: &App) -> bool {
        self.read(cx).closable(cx)
    }

    fn zoomable(&self, cx: &App) -> bool {
        self.read(cx).zoomable(cx)
    }

    fn starts_open(&self, cx: &App) -> bool {
        self.read(cx).starts_open()
    }

    fn activation_priority(&self, cx: &App) -> u32 {
        self.read(cx).activation_priority()
    }

    fn activation_focus_handle(&self, cx: &App) -> FocusHandle {
        self.read(cx).activation_focus_handle(cx)
    }

    fn set_active(&self, active: bool, cx: &mut App) {
        self.update(cx, |this, cx| this.set_active(active, cx));
    }

    fn to_any(&self) -> AnyView {
        self.clone().into()
    }
}
