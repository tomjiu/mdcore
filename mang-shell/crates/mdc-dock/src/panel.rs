//! 面板行为层 trait（#119 批①）——蓝本 zed `dock.rs:36-104` `Panel`。
//!
//! 纪律：本 trait 只承载 dock 协议需要的最小行为面（持久化身份 /
//! 位置合法性 / 激活可见性裁决），呈现层钩子（标题/工具栏）经
//! [`super::PanelHandle`] 由宿主组合——与蓝本「行为/呈现两缝」的
//! gpui-base 设计（其 base `panel.rs:21` + component `panel.rs:71`）
//! 同构。批次②（持久化）与批次③（拖拽）到需要时再扩，不预铺。

use gpui::{App, Context, FocusHandle};

use super::dock::DockPosition;

/// 面板事件（蓝本 dock.rs:27-32）。`Activate` 由面板上行请求激活；
/// `Close` 请求移除（`closable` 门在 Dock 侧二次裁决——蓝本同款）；
/// `ZoomIn`/`ZoomOut` 本批仅定义（zoom 归后续批次）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelEvent {
    Activate,
    Close,
    ZoomIn,
    ZoomOut,
}

/// 面板行为层。蓝本 supertrait 面：`Focusable` + `EventEmitter<
/// PanelEvent>` + `Render` + `Sized`（dock.rs:36）——本批一比一保留：
/// 面板必须可聚焦（激活焦点协议的落点）且可独立渲染。
pub trait Panel: gpui::Focusable + gpui::EventEmitter<PanelEvent> + gpui::Render + Sized {
    /// 持久化身份名（布局序列化键——发布后不可改，蓝本 dock.rs:37
    /// 注释同款警告）。
    fn persistent_name() -> &'static str;

    /// 面板键（蓝本 dock.rs:38；zed 中与 persistent_name 分立——
    /// persistent_name 面向序列化、panel_key 面向设置锚点。本批两键
    /// 等值实现由具体面板自定，接口面保留分立）。
    fn panel_key() -> &'static str;

    /// 面板激活时应聚焦的句柄（蓝本 dock.rs:44-46：缺省 = 面板根
    /// focus_handle；面板内另有优先焦点面（过滤框等）时覆写）。
    fn activation_focus_handle(&self, cx: &App) -> FocusHandle {
        self.focus_handle(cx)
    }

    /// 面板位置（本批为固定值；设置驱动换位归后续批次——蓝本带
    /// `window`/`settings` 参，见 PROVENANCE delta）。
    fn position(&self) -> DockPosition;

    /// 位置合法性（面板声明自己可停靠的边）。
    fn position_is_valid(&self, position: DockPosition) -> bool;

    /// 换位（本批缺省无操作——固定位置面板不覆写）。
    fn set_position(&mut self, _position: DockPosition, _cx: &mut Context<Self>) {}

    /// 启用裁决（禁用面板被 `first_enabled_panel_idx` 跳过——蓝本
    /// dock.rs:550/:92）。
    fn enabled(&self, _cx: &App) -> bool {
        true
    }

    /// 可见性（批② 持久化面消费；本批协议面先行）。
    fn visible(&self, _cx: &App) -> bool {
        true
    }

    /// 可关性（`Close` 事件移除门——蓝本 dock.rs:34）。
    fn closable(&self, _cx: &App) -> bool {
        false
    }

    /// 可缩放性（zoom 归后续批次；本批协议面先行）。
    fn zoomable(&self, _cx: &App) -> bool {
        false
    }

    /// 初始即开（注册进空 dock 时是否立即激活的裁决输入之一）。
    fn starts_open(&self) -> bool {
        false
    }

    /// 激活优先级（并列面板的次序裁决——蓝本 dock.rs:91）。
    fn activation_priority(&self) -> u32 {
        0
    }

    /// 激活/去激活通知（蓝本 dock.rs:84）。幂等语义由 [`Dock`] 的
    /// 激活协议保证（去重——见 [`super::ActiveTracker`]）。
    fn set_active(&mut self, _active: bool, _cx: &mut Context<Self>) {}
}
