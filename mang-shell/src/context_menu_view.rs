//! 右键菜单构建/部署层（#63 打磨项 → #385 批 1：迁 `ui::ContextMenu`）。
//!
//! # 职责（批 1 迁移后）
//!
//! - [`build_ui_menu`]：把 [`crate::context_menu::MenuItem`] 数据集构建为
//!   `ui::ContextMenu` **实体**。动作项用 `CustomEntry`：行内容自渲染
//!   （标签 + 快捷键标签——快捷键文本继续取 keymap 单一事实源 #217，
//!   用户换绑后随下次打开跟随），点击 handler 回派
//!   `ShellApp::activate_context_menu_item`；灰显项 `selectable(false)`
//!   （无 hover/无点击/键盘导航跳过）。
//! - [`context_menu_layer`]：部署元素 = `deferred(anchored().position(点击点))`
//!   ——**摆放/边缘翻转/夹紧全部由 gpui `Anchored` 承担**
//!   （`AnchoredFitMode::SwitchAnchor`：右/下越界翻锚角，仍越界贴窗口边
//!   夹紧——旧 `place_menu` 的语义由框架同构接管）。宽度走真实文本测量
//!   （旧 `estimate_width` 粗估在右键菜单路径退役）。
//!
//! 「点空白处关闭」由 `ui::ContextMenu` 自带 `on_mouse_down_out` →
//! `DismissEvent` 承担（shell 订阅该事件清态）——旧的全屏手写遮罩退役。
//! 键盘导航（↑/↓/Enter/Esc）在 `shell.rs` 根 `on_key_down` 调 menu 实体
//! 方法（select_next/select_previous/confirm）。
//!
//! # 旧实现退役清单（#385 批 1 真去重）
//!
//! ```text
//! 旧 context_menu_view.rs（146 行手写浮层）：
//!   全屏透明遮罩 div ×2 按钮        → ui::ContextMenu on_mouse_down_out
//!   手写菜单容器/圆角/边框/阴影      → ui::ContextMenu render（elevation_2）
//!   手写行渲染（hover/灰显/快捷键）  → ui::ContextMenu CustomEntry
//!   手写布局几何（menu_size/layout） → gpui Anchored + 真实文本测量
//!   ContextMenu::action_ids()        → handler 闭包直接携带动作 id
//! ```

use gpui::{App, Entity, IntoElement, Pixels, Point, WeakEntity, Window, anchored, deferred, px};
use ui::ContextMenu as UiContextMenu;
use ui::prelude::*;

use crate::context_menu::MenuItem;
use crate::shell::ShellApp;

/// 把 [`MenuItem`] 集构建为 `ui::ContextMenu` 实体。
///
/// 动作项点击 → `ShellApp::activate_context_menu_item(id, ..)`（与键位 /
/// 命令面板同一派发体）。灰显（`enabled == false`）→ `selectable(false)`：
/// 无 hover 高亮、无点击、键盘导航自动跳过（`ui::ContextMenu` 的
/// `is_selectable` 语义——「点不到不算死按钮」）。
pub fn build_ui_menu(
    items: Vec<MenuItem>,
    shell: WeakEntity<ShellApp>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<UiContextMenu> {
    UiContextMenu::build(window, cx, move |mut menu, _window, _cx| {
        for item in items {
            menu = match item {
                MenuItem::Action {
                    id,
                    label,
                    shortcut,
                    enabled,
                } => {
                    let shell = shell.clone();
                    // CustomEntry 的行内容闭包：标签 + 快捷键标签（keymap
                    // 单一事实源——菜单打开时取当前键位，换绑即跟随）。
                    let label_text = label.clone();
                    let shortcut_text = shortcut.clone().unwrap_or_default();
                    let row_enabled = enabled;
                    menu.custom_entry(
                        move |_window, cx| {
                            let colors = cx.theme().colors().clone();
                            let text_color = if row_enabled {
                                colors.text
                            } else {
                                colors.text_muted
                            };
                            h_flex()
                                .w_full()
                                .justify_between()
                                .gap_3()
                                .child(div().text_color(text_color).child(label_text.clone()))
                                .when(!shortcut_text.is_empty(), |row| {
                                    row.child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(colors.text_muted)
                                            .child(shortcut_text.clone()),
                                    )
                                })
                                .into_any_element()
                        },
                        move |window, cx| {
                            let _ = shell.update(cx, |shell, cx| {
                                shell.activate_context_menu_item(id, window, cx);
                            });
                        },
                    )
                    .selectable(enabled)
                }
                MenuItem::Separator => menu.separator(),
                MenuItem::Submenu { label, .. } => {
                    // 现役预设（editor_menu/file_tree_menu）不含子菜单；保守
                    // 渲染为不可选提示行（与旧渲染层同口径，不假装支持）。
                    // 真子菜单支持 = #385 后续批次（ui::ContextMenu 自带
                    // submenu builder，数据面届时再开口）。
                    let label_text = label.clone();
                    menu.custom_entry(
                        move |_window, cx| {
                            let colors = cx.theme().colors().clone();
                            h_flex()
                                .w_full()
                                .justify_between()
                                .child(
                                    div()
                                        .text_color(colors.text_muted)
                                        .child(label_text.clone()),
                                )
                                .child(div().text_size(px(11.)).child("›"))
                                .into_any_element()
                        },
                        |_window, _cx| {
                            // 不可选项本就挂不上 on_click（selectable=false）；
                            // 防御位：若数据面未来加子菜单而构建层未跟进，
                            // stderr 留痕（仓库纪律：不静默吞掉）。
                            eprintln!("context_menu: submenu reached handler (not yet supported)");
                        },
                    )
                    .selectable(false)
                }
            };
        }
        menu
    })
}

/// 部署元素：`deferred(anchored().position(点击点))`。
///
/// - `deferred`：浮层最后画（链尾最上层，同旧实现的层序语义）；
/// - `anchored`（默认 `Anchor::TopLeft` + `SwitchAnchor` + Window 坐标）：
///   点击点右下展开，右/下越界翻锚角，翻不动贴窗口边——旧
///   `place_menu` 翻转/夹紧语义由框架同构接管；
/// - `occlude`：菜单区域吞掉下层的命中测试（点菜单不穿透）。
pub fn context_menu_layer(menu: &Entity<UiContextMenu>, pos: Point<Pixels>) -> impl IntoElement {
    deferred(
        anchored()
            .position(pos)
            .child(div().occlude().child(menu.clone())),
    )
}
