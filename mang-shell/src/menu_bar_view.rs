//! 菜单栏渲染桥接层（#209 → #385 批 4：手写浮层 → `ui::ContextMenu`）。
//!
//! # 分层（迁移后——与批 1 `context_menu_view` 同构）
//!
//! - 结构/灰显/勾选数据在 [`crate::menu_bar`]（纯函数，单测钉定——不变）；
//! - 本模块三件桥接：
//!   ① [`menu_titles`]——标题行**自绘保留**（按钮族视觉语系，对照表
//!     结论 2：IconButton/Button 无固定尺寸 API，36×36 硬规格同因）；
//!     点击 = 开/关/切换顶部菜单（`open_menu_bar` / `close_menu_bar`）；
//!   ② [`build_menu_bar_menu`]——`TopMenu` 数据 → `ui::ContextMenu` 实体。
//!     浮层渲染 / 行渲染 / **一层子菜单**（组件 `.submenu()` 原生——主题/
//!     字号/侧栏/最近打开）/ 键盘导航（↑↓/Enter/Esc/←→ 进出子菜单）/
//!     点外关闭（`on_mouse_down_out` → `DismissEvent`）全部由组件自带；
//!   ③ [`menu_bar_anchor`]——`deferred + anchored` 锚定挂载（溢出翻转/
//!     吸附按**实际渲染尺寸**处理——自研 `place_menu`/`place_submenu`/
//!     `estimate_anchor_x` 几何消费面随之退役）。
//!
//! # 为什么 shortcut 走 `custom_entry` 自绘行
//!
//! 与批 1 同因：zed entry 的快捷键显示绑定 gpui Action 注册体系
//! （`KeyBinding::for_action`）；mang-shell 的键位标签来自自研 keymap 表
//! （用户可换绑，非 gpui action）。`custom_entry` 是行级公开扩展点——
//! 行内自绘 label + shortcut，容器级导航/hover/选中态仍由组件承载。
//!
//! # 灰显与勾选
//!
//! - 灰显（enabled=false）：`custom_entry` + `.selectable(false)`（键盘
//!   导航跳过 + 点击无 handler——「点不到不算死按钮」，与旧实现
//!   「enabled 才挂 on_click」同语义）；
//! - 勾选（主题/侧栏子菜单的当前态）：`toggleable_entry`（组件原生
//!   toggle 渲染——勾选列语义替代旧「✓」文本列）。

use gpui::{App, Context, Entity, Pixels, Point, WeakEntity, Window, anchored, deferred, px};
use ui::ContextMenu as UiContextMenu;
use ui::prelude::*;

use crate::menu_bar::{MenuEntry, MenuItemSpec, TopMenu};
use crate::shell::ShellApp;
use crate::title_bar::TITLE_BAR_HEIGHT;

/// 标题栏里的菜单标题行（文件/编辑/查看/帮助——title_bar 左侧标题之后
/// 挂载；`open_idx` = 当前展开的顶部菜单序，None = 全关）。
pub fn menu_titles(
    menus: &[TopMenu],
    open_idx: Option<usize>,
    cx: &mut Context<ShellApp>,
) -> impl IntoElement {
    let colors = cx.theme().colors().clone();
    let hover = colors.element_hover;
    let selected_bg = colors.element_selected;
    h_flex()
        .id("menu-bar-titles")
        .debug_selector(|| "menu-bar-titles".into())
        .h_full()
        .flex_none()
        .items_center()
        .gap(px(crate::menu_bar::TITLES_GAP))
        .children(menus.iter().enumerate().map(|(idx, menu)| {
            let is_open = open_idx == Some(idx);
            div()
                .id(SharedString::from(format!("menu-title-{}", menu.id)))
                .h_full()
                .flex()
                .items_center()
                .px(px(crate::menu_bar::TITLE_BTN_PAD_X))
                .rounded_sm()
                .text_size(px(13.))
                .when(is_open, move |el| el.bg(selected_bg))
                .hover(move |el| el.bg(hover))
                .child(div().text_color(colors.text).child(menu.label))
                // on_mouse_down（非 on_click）：按下瞬间捕获 event.position.x
                // 作下拉锚点（anchored 定位用——真实点击位，非估宽）。
                // 已开同菜单 = 关闭，未开/别的菜单 = 一键直达切换：
                // 浮层由组件 `on_mouse_down_out` 关闭（事件正向派发先于
                // out 补发——标题 handler 先执行，toggle 判定取的是点击
                // 瞬间状态，无竞态）。
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                        let x = f32::from(event.position.x);
                        if this.is_menu_bar_open_at(idx) {
                            this.close_menu_bar(window, cx);
                        } else {
                            this.open_menu_bar(idx, x, window, cx);
                        }
                    }),
                )
        }))
}

/// 单个动作项的行渲染（label 左 / shortcut 右——`custom_entry` 的
/// entry_render 闭包，keymap 表标签非 gpui action，见模块文档）。
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

/// 一个条目层的构建（顶部菜单与子菜单共用——子菜单内不再嵌套子菜单，
/// `build_menus` 结构保证，`MenuEntry::Submenu` 分支只出现在顶层）。
fn build_entries(
    menu: UiContextMenu,
    entries: &[MenuEntry],
    shell: &WeakEntity<ShellApp>,
) -> UiContextMenu {
    let mut menu = menu;
    for entry in entries {
        match entry {
            MenuEntry::Separator => menu = menu.separator(),
            MenuEntry::Submenu { label, items } => {
                // 组件 builder 签名要求 'static——子菜单条目 clone 进闭包
                //（与 build_menu_bar_menu 顶层的 entries 同口径）。
                let sub_items = items.clone();
                let shell = shell.clone();
                // 组件原生子菜单（键盘 → 展开 / ← 收起 / 悬停跟随——
                // 旧实现自绘右侧「›」+ place_submenu 摆放的退役面）。
                menu = menu.submenu(label.clone(), move |sub, _window, _cx| {
                    build_entries(sub, &sub_items, &shell)
                });
            }
            MenuEntry::Item(spec) => {
                menu = build_item(menu, spec, shell);
            }
        }
    }
    menu
}

/// 动作项构建：勾选项 → `toggleable_entry`（组件原生 toggle 渲染）；
/// 普通项 → `custom_entry`（自绘 label + shortcut 行）。灰显统一
/// `.selectable(false)`（toggleable 路仅主题/侧栏项——恒启用，灰显
/// 面全在 custom 路上）。
fn build_item(
    menu: UiContextMenu,
    spec: &MenuItemSpec,
    shell: &WeakEntity<ShellApp>,
) -> UiContextMenu {
    if spec.checked {
        // 勾选项（主题/侧栏子菜单——无 shortcut 标签，keymap 无绑定）。
        let action = spec.action;
        let shell = shell.clone();
        return menu.toggleable_entry(
            spec.label.clone(),
            true,
            IconPosition::Start,
            None,
            move |window, cx| {
                let _ = shell.update(cx, |shell, cx| {
                    shell.activate_menu_action(action, window, cx)
                });
            },
        );
    }
    let label_color = if spec.enabled {
        ui::Color::Default
    } else {
        ui::Color::Muted
    };
    let action = spec.action;
    let shell = shell.clone();
    menu.custom_entry(
        action_row(&spec.label, spec.shortcut.as_deref(), label_color),
        move |window, cx| {
            let _ = shell.update(cx, |shell, cx| {
                shell.activate_menu_action(action, window, cx)
            });
        },
    )
    // 灰显项不可选（键盘导航跳过 + 点击无 handler——组件语义）。
    .selectable(spec.enabled)
}

/// [`crate::menu_bar::TopMenu`] 数据 → `ui::ContextMenu` 实体（打开菜单
/// 时构建一次——菜单内容随 `MenuContext` 快照定死，模态期间不变）。
///
/// 动作执行统一回 `ShellApp::activate_menu_action`（派发单点——本批
/// 不改派发面）；激活/点外/Esc → 组件 `DismissEvent` → shell 订阅
/// `close_menu_bar` 清态（旧全屏遮罩的关闭链退役）。
///
/// `entries` 走 clone 进闭包：组件 builder 签名要求 `'static`（菜单
/// 数据为小字符串集合，每次打开一次 clone，成本可忽略——与每次打开
/// 重建实体的节奏同量级）。
pub fn build_menu_bar_menu(
    menu: &TopMenu,
    shell: WeakEntity<ShellApp>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<UiContextMenu> {
    let entries = menu.entries.clone();
    UiContextMenu::build(window, cx, move |menu, _window, _cx| {
        build_entries(menu, &entries, &shell)
    })
}

/// 锚定挂载：`deferred + anchored`（溢出翻转/吸附按实际渲染尺寸）+
/// `occlude` 遮挡。锚点 = 标题点击 x + 标题栏下缘 y（`open_menu_bar`
/// 采集）。
pub fn menu_bar_anchor(menu: &Entity<UiContextMenu>, anchor: Point<Pixels>) -> impl IntoElement {
    deferred(
        anchored()
            .anchor(gpui::Anchor::TopLeft)
            .position(anchor)
            .child(gpui::div().occlude().child(menu.clone())),
    )
    .with_priority(1)
}

/// 下拉锚点 y（标题栏下缘——`TITLE_BAR_HEIGHT` 与旧版遮罩同源）。
pub fn menu_anchor_y() -> f32 {
    f32::from(TITLE_BAR_HEIGHT)
}
