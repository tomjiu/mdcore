//! 右键菜单渲染层（#385 批 1：手写浮层 → `ui::ContextMenu` 组件；
//! 批 1×4 收口：几何层已退役）。
//!
//! # 分层（迁移后）
//!
//! - 数据/预置菜单在 [`crate::context_menu`]（零 gpui 依赖，lib/bin 共用
//!   ——不变）；自研几何（`place_menu`/`menu_size`/`estimate_width` 一族
//!   + Point/Rect）已随批 4 迁完整体退役——摆放全部交 gpui
//!   `anchored` 按实际渲染尺寸处理。
//! - 本模块只做两件桥接：
//!   ① [`build_context_menu`]——`MenuItem` 数据 → `ui::ContextMenu`
//!     实体（键盘导航 / 焦点 / ARIA / dismiss 全部由组件自带）；
//!   ② [`context_menu_anchor`]——`gpui::anchored` 锚定渲染（SwitchAnchor
//!     溢出翻转 + 边缘吸附，与原 `place_menu` 语义一致，但按**实际渲染
//!     尺寸**而非文本估宽判定）。
//!
//! # 为什么 shortcut 走 `custom_entry` 而不是 `ContextMenuEntry::action`
//!
//! zed entry 的快捷键显示绑定 gpui Action 注册体系
//! （`KeyBinding::for_action`）；mang-shell 的键位标签来自自研 keymap
//! 表（用户可换绑，非 gpui action）。`custom_entry` 是组件公开的行级
//! 扩展点——行内自绘 label + shortcut，容器级导航/hover/选中态仍由
//! 组件的 `ListItem` + `selected_index` 承载（零内部逻辑拷贝）。
//!
//! 灰显项：`custom_entry` 无 `disabled` 槽（那是 `ContextMenuEntry` 的），
//! 等价组合 = 行内 `Color::Muted` 灰显 + `.selectable(false)`（键盘
//! 导航跳过、点击无 handler——与原实现的「enabled 才挂 on_click」
//! 同语义）。

use gpui::{App, Entity, Pixels, Point, WeakEntity, Window, anchored, deferred, px};
use ui::ContextMenu as UiContextMenu;
use ui::prelude::*;

use crate::context_menu::MenuItem;
use crate::shell::ShellApp;

/// 单个动作项的行渲染（label 左 / shortcut 右）。
///
/// 颜色用语义色 `ui::Color`（渲染时从主题解析——主题切换即时跟随，
/// zed entry 的 `Color::Default`/`Color::Disabled` 同口径：灰显项用
/// `Muted` 与组件 entry 的 disabled 视觉语系一致）。
fn action_row(
    label: &str,
    shortcut: Option<&str>,
    label_color: ui::Color,
) -> impl Fn(&mut Window, &mut App) -> gpui::AnyElement + 'static {
    let label = label.to_string();
    let shortcut = shortcut.map(str::to_string);
    move |_, cx| {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .w_full()
            .text_color(label_color.color(cx))
            .child(label.clone())
            .children(shortcut.clone().map(|s| {
                div()
                    .text_size(px(11.))
                    .text_color(ui::Color::Muted.color(cx))
                    .child(s)
            }))
            .into_any_element()
    }
}

/// [`crate::context_menu::MenuItem`] 列表 → `ui::ContextMenu` 实体。
///
/// 动作执行统一回 `ShellApp::activate_context_menu_item`（id 派发单点
/// ——本批不改派发面）。子菜单项本批仍不支持（原实现即只做一层，
/// 数据层 `MenuItem::Submenu` 变体保留，渲染为灰显提示行——诚实登记
/// 不假装支持）。
pub fn build_context_menu(
    items: &[MenuItem],
    shell: WeakEntity<ShellApp>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<UiContextMenu> {
    UiContextMenu::build(window, cx, |menu, _window, _cx| {
        let mut menu = menu;
        for item in items {
            menu = match item {
                MenuItem::Action {
                    id,
                    label,
                    shortcut,
                    enabled,
                } => {
                    let label_color = if *enabled {
                        ui::Color::Default
                    } else {
                        ui::Color::Muted
                    };
                    let action_id = *id;
                    let shell = shell.clone();
                    menu.custom_entry(
                        action_row(label, shortcut.as_deref(), label_color),
                        move |window, cx| {
                            let _ = shell.update(cx, |shell, cx| {
                                shell.activate_context_menu_item(action_id, window, cx)
                            });
                        },
                    )
                    // 灰显项不可选（键盘导航跳过 + 点击无 handler）。
                    .selectable(*enabled)
                }
                MenuItem::Separator => menu.separator(),
                MenuItem::Submenu { label, .. } => {
                    // 一层菜单：子菜单渲染为不可选的灰显提示行（与原
                    // context_menu_view 的「带箭头但不可点」同语义）。
                    let label = label.clone();
                    menu.custom_row(move |_, cx| {
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .w_full()
                            .text_color(ui::Color::Muted.color(cx))
                            .child(label.clone())
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(ui::Color::Muted.color(cx))
                                    .child("›"),
                            )
                            .into_any_element()
                    })
                }
            };
        }
        menu
    })
}

/// 锚定渲染：`deferred` + `anchored`（SwitchAnchor）+ `occlude` 遮挡。
///
/// - **点外部关闭**：组件 on_blur → `menu::Cancel` → `DismissEvent`
///   （shell 侧订阅清状态），不再需要全屏透明遮罩；
/// - **边缘翻转**：`anchored` 默认 SwitchAnchor（翻得动就翻转、翻不动
///   吸附窗口边——原 `place_menu` 语义的引擎级实现，按实际尺寸判定）；
/// - `with_priority(1)`：浮层置顶（右键菜单遮罩退役后点击命中的次序
///   由 hitbox/occlude 决定，优先级沿用 `RightClickMenu` 同款值）。
pub fn context_menu_anchor(menu: &Entity<UiContextMenu>, pos: Point<Pixels>) -> impl IntoElement {
    deferred(
        anchored()
            .anchor(gpui::Anchor::TopLeft)
            .position(pos)
            .child(gpui::div().occlude().child(menu.clone())),
    )
    .with_priority(1)
}
