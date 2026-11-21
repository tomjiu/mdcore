//! 左侧图标条（#63 Lead：图标条 48px 宽 + 间距修正「每个按钮 36×36、
//! 间距 4px、外边距 8px」）。
//!
//! 实宽取 52px = 8 + 36 + 8——间距要求是 Lead 对「图标条内边距太挤」的
//! 修正项，优先于布局草图的 48px 字面值（36 + 2×8 > 48，两者不可同时
//! 满足；按钮本体 36×36 与 8px 边距为硬规格，宽度取其和）。
//!
//! #289-⑤ 语义收口（用户原话「左侧边栏成了右侧按钮的详情菜单了这个
//! 很不好」）：**左图标条只管左栏**——
//! - 主面板钮（文件树/git）在上，切换左栏视图（vscode 资源管理器/
//!   源代码管理同构）；
//! - 搜索钮接项目搜索面板（#109 批②③——`ShellApp::toggle_search`，
//!   与文件树/git 钮同构的左栏视图钮；编辑器内 Ctrl+F 独立并存）；
//! - 设置钮沉底（vscode 同款位置）：#289-⑤ 起改为打开**设置弹窗**
//!   （`ShellApp::open_settings`——不再占用右栏视图；zed/vscode 的
//!   「设置不是任何 dock 的视图」语义）；
//! - 右栏（属性/来源）开关**不在左条**——移至右缘 dock 条
//!   （[`crate::right_dock`]，「右侧面板的事归右侧」）。
//! - 无 AI 面板——agent 能力全部走 MCP 暴露（#63 Lead 裁决）。
//!
//! 按钮为自绘单元而非 `IconButton`：后者方形尺寸由 IconSize 的 rem 机器
//! 推导（icon + 2×padding），无法钉定 36×36 字面规格；自绘单元同时掌控
//! hover/selected 主题色与精确几何。a11y（键盘焦点/读屏）记为后续 PR。

use gpui::{App, ClickEvent, Context, Hsla, Pixels, Window, px};
use ui::prelude::*;
use ui::{Color, Icon, IconName, IconSize, Tooltip};

use crate::shell::ShellApp;

/// 图标条实宽（px）：8 边距 + 36 按钮 + 8 边距。
pub const ACTIVITY_BAR_WIDTH: Pixels = px(52.);
/// 按钮边长（px，Lead 硬规格）。
pub const BUTTON_SIZE: Pixels = px(36.);
/// 按钮纵向间距（px）。
const BUTTON_GAP: Pixels = px(4.);
/// 图标条四周内边距（px）。
///
/// #289-⑤ 起 [`crate::right_dock`] 复用（右缘条同规格）。
pub(crate) const BAR_PADDING: Pixels = px(8.);

/// 渲染图标条。`left_open`/`git_view` 决定文件树/git 钮的选中态；
/// `settings_open` 决定设置钮的选中态（弹窗开 = 选中视觉）。
/// `search_disabled`（#176）：docx 页面视图下搜索钮置灰 + 原因
/// tooltip（查找是编辑器浮层，页面态编辑器不在树内——不进死路）。
pub fn activity_bar(
    left_open: bool,
    git_view: bool,
    settings_open: bool,
    search_disabled: bool,
    cx: &mut Context<ShellApp>,
) -> impl IntoElement {
    let colors = cx.theme().colors().clone();
    // #109：search_disabled 参数保留签名兼容（恒 false——置灰口径退役）。
    let _ = search_disabled;
    // #109：搜索视图态（图标条选中——与 git_view 同源 dock 读）。
    let search_view = cx.entity().read(cx).search_active(cx);
    v_flex()
        .id("activity-bar")
        .debug_selector(|| "activity-bar".into())
        .w(ACTIVITY_BAR_WIDTH)
        .flex_none()
        .h_full()
        .bg(colors.panel_background)
        .p(BAR_PADDING)
        .gap(BUTTON_GAP)
        .child(bar_button(
            "act-file-tree",
            IconName::FileTree,
            left_open && !git_view,
            "文件树",
            colors.element_hover,
            colors.element_selected,
            cx.listener(|this, _, _, cx| this.toggle_left(cx)),
        ))
        // F7：git 改动/历史视图（文件树钮与 git 钮互切视图）。
        .child(bar_button(
            "act-git",
            IconName::GitBranch,
            left_open && git_view,
            "改动 / 历史（git）",
            colors.element_hover,
            colors.element_selected,
            cx.listener(|this, _, _, cx| this.toggle_git(cx)),
        ))
        // #109 批②③：搜索钮 → 项目搜索面板（toggle_search——面板激活
        // 协议，与文件树/git 钮同构；#165 的编辑器内 Ctrl+F 键位不受影响
        // ——两域并存）。#176 置灰口径退役：项目搜索不依赖编辑器浮层
        // （docx 页面视图/无文档均可搜工作区）。
        .child(
            bar_button(
                "act-search",
                IconName::MagnifyingGlass,
                left_open && search_view,
                "项目搜索（Ctrl+K Ctrl+F）",
                colors.element_hover,
                colors.element_selected,
                cx.listener(|this, _, _, cx| this.toggle_search(cx)),
            )
            .into_any_element(),
        )
        .child(div().flex_1())
        // #289-⑤：设置钮沉底（vscode 同款位置）——打开设置**弹窗**
        // （不再驱动右栏视图；右栏只归属性/来源，右缘 dock 条自管）。
        .child(bar_button(
            "act-settings",
            IconName::Settings,
            settings_open,
            "设置",
            colors.element_hover,
            colors.element_selected,
            cx.listener(|this, _, _, cx| this.toggle_settings(cx)),
        ))
}

/// 36×36 图标按钮单元：hover/selected 主题色（色值由调用方从当前主题注入），
/// 图标 16px（IconSize::Medium），tooltip 走 ui::Tooltip。
///
/// #289-⑤ 起 [`crate::right_dock`] 复用本单元（左右两条按钮视觉同语系）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn bar_button(
    id: &'static str,
    icon: IconName,
    selected: bool,
    tooltip: &'static str,
    hover_bg: Hsla,
    selected_bg: Hsla,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .size(BUTTON_SIZE)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_md()
        .when(selected, |el| el.bg(selected_bg))
        .hover(|el| el.bg(hover_bg))
        .tooltip(Tooltip::text(tooltip))
        .child(
            Icon::new(icon)
                .size(IconSize::Medium)
                .color(Color::Default)
                .when(selected, |el| el.color(Color::Selected)),
        )
        .on_click(on_click)
}

/// #176：置灰图标按钮单元（bar_button 的禁用态——同 36×36 几何）：
/// Muted 图标色、无 hover 反馈、无 on_click；tooltip 说明原因
/// （「明确禁用态与说明」，非静默无反应）。a11y 面与 bar_button
/// 同纪录（模块文档：后续 PR）。
pub(crate) fn disabled_bar_button(
    id: &'static str,
    icon: IconName,
    reason: &'static str,
) -> impl IntoElement {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .size(BUTTON_SIZE)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_md()
        .tooltip(Tooltip::text(reason))
        .child(Icon::new(icon).size(IconSize::Medium).color(Color::Muted))
}
