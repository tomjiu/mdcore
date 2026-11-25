//! 设置弹窗（#182 立项；#289-⑤ 改形态为模态弹窗）。
//!
//! ## 定位
//!
//! `settings.rs` 8 类存取早已全实现、`theme_switch.rs` 主题三态全套
//! 已实现——缺的只是一个 UI 壳。本模块就是这个壳：**只消费现有 API，
//! 零新增存储格式字段**（铁律 1）：
//!
//! | 项 | 控件 | 数据来源 / 生效链 |
//! |---|---|---|
//! | 主题 | 三态钮（跟随系统/浅色/深色） | `theme_switch::apply_mode` + `save_theme_mode`（`ShellApp::set_theme_mode`） |
//! | 字号 | −/值/+（8-32 步进 1） | `ShellApp::apply_font_size`（与 Ctrl+=/- 同一条链——立即应用到全部标签 + 持久化） |
//! | 缩进单位 | 三态钮（4 空格/2 空格/Tab） | `save_indent_unit` + 全部标签 `Editor::set_indent_unit`（`ShellApp::set_indent_unit_all`） |
//! | 面板宽度 | 「重置默认」钮 | `save_panel_widths` + 运行时宽度恢复（`ShellApp::reset_panel_widths`） |
//!
//! ## 形态裁决（#289-⑤ 重裁）
//!
//! **模态弹窗**（zed「设置是独立界面不是 dock 视图」/ vscode「设置
//! 打开为独立窗格」的同款语义）：
//! - 旧版（#182）：占右栏、与属性/来源互斥——用户实测反馈「左侧边栏
//!   成了右侧按钮的详情菜单」（左图标条设置钮驱动右栏视图，语义
//!   错位）；
//! - 新版：`ShellApp::settings_open` 为真时渲染居中卡片 + 半透明遮罩
//!   （确认框 confirm_overlay 同款模态机制）；入口 = 左图标条底部
//!   设置钮 / 菜单栏「查看 → 设置」；Esc 或点遮罩关闭。
//!
//! ## 写法参照
//!
//! `panels.rs` 的面板骨架（v_flex + 分组标题 + hover/selected 主题色）
//! 与 `activity_bar.rs` 的 `cx.listener` 接线模式。每个可点控件都有
//! 真实行为（铁律 4：不留空闭包——`wiring_integrity` 的空闭包扫描
//! 对本文件同样适用）。

use gpui::{App, ClickEvent, Context, Hsla, Pixels, Window, px};
use ui::prelude::*;
use ui::{
    Color, Icon, IconName, IconSize, Label, LabelSize, Modal, ModalHeader, Section, SectionHeader,
    Tooltip,
};

use crate::editor::buffer::IndentUnit;
use crate::shell::ShellApp;
use crate::theme_switch;

/// 弹窗卡片宽度（px）。
const MODAL_W: Pixels = px(400.);
/// 控件钮纵向高度（px）。
const CONTROL_H: Pixels = px(26.);
/// 「标签 + 控件」行的标签列宽（px）。
const LABEL_W: Pixels = px(64.);

/// 渲染设置弹窗（#385 批 3：卡片骨架迁 `ui::Modal`；`ShellApp::settings_open`
/// 为真时由 shell 渲染链尾挂载）。
///
/// 迁移面（原 378 行手写外壳 → 组件）：
/// - 卡片骨架/标题行/关闭钮 → `Modal` + `ModalHeader`（headline 排版、
///   dismiss 钮、分组间距全组件）；
/// - 分组 → `Section` + `SectionHeader`（原 group_header 手写退役）；
/// - 控件行容器 → `ModalRow`；
/// - 保留：遮罩/居中（ui crate 无遮罩组件——模态容器是 zed 主仓
///   闭包外能力）、按钮族（choice/step/action——bar_button 同款自绘
///   视觉语系，对照表不迁清单）。
///
/// 关闭链：组件 dismiss 钮 dispatch `menu::Cancel` → 本层 `on_action`
/// 接住 → `close_settings`（与 Esc / 点遮罩同一条链）。
///
/// `theme_mode` / `font_size` / `indent_unit` 均为 ShellApp 运行时状态
/// （调用处传入），点击经 `cx.listener` 回写 ShellApp（单一状态源，
/// 弹窗自身零状态）。
pub fn settings_modal(
    theme_mode: &str,
    font_size: f32,
    indent_unit: IndentUnit,
    soft_wrap: bool,
    custom_theme_draft: [String; 31],
    theme_name_input: String,
    color_slot_focus: Vec<gpui::FocusHandle>,
    cx: &mut Context<ShellApp>,
) -> impl IntoElement {
    let colors = cx.theme().colors().clone();
    div()
        .id("settings-modal-layer")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        // 半透明遮罩：点外关闭（弹窗内卡片是后画兄弟，点击不穿透）。
        .bg(gpui::black().opacity(0.35))
        .on_mouse_down(gpui::MouseButton::Left, {
            cx.listener(|this, _, _, cx| this.close_settings(cx))
        })
        // ModalHeader 的 dismiss 钮 dispatch menu::Cancel——在这里
        // 接住（与 Esc / 点遮罩同一条 close_settings 链）。
        .on_action(cx.listener(|this, _: &menu::Cancel, _, cx| {
            this.close_settings(cx);
        }))
        .child(ui_modal(
            theme_mode,
            font_size,
            indent_unit,
            soft_wrap,
            custom_theme_draft,
            theme_name_input.clone(),
            color_slot_focus,
            &colors,
            cx,
        ))
}

/// 弹窗卡片（#385 批 3：骨架迁 `ui::Modal`——header/section 排版全
/// 组件；卡片尺寸/浮层视觉容器由本层提供，`Modal` 是填充式骨架
/// （size_full——zed 用法即「放进预设尺寸的卡片容器」）。
fn ui_modal(
    theme_mode: &str,
    font_size: f32,
    indent_unit: IndentUnit,
    soft_wrap: bool,
    custom_theme_draft: [String; 31],
    theme_name_input: String,
    color_slot_focus: Vec<gpui::FocusHandle>,
    colors: &theme::ThemeColors,
    cx: &mut Context<ShellApp>,
) -> impl IntoElement {
    div()
        .id("settings-card")
        .w(MODAL_W)
        .max_h(px(520.))
        .min_h_0()
        .bg(colors.elevated_surface_background)
        .border_1()
        .border_color(colors.border)
        .rounded_md()
        .shadow_lg()
        .overflow_y_scroll()
        .child(modal_body(
            theme_mode,
            font_size,
            indent_unit,
            soft_wrap,
            custom_theme_draft,
            theme_name_input.clone(),
            color_slot_focus,
            colors,
            cx,
        ))
}

/// `ui::Modal` 骨架（标题行 + 分区——排版/间距/主题色全组件）。
fn modal_body(
    theme_mode: &str,
    font_size: f32,
    indent_unit: IndentUnit,
    soft_wrap: bool,
    custom_theme_draft: [String; 31],
    theme_name_input: String,
    color_slot_focus: Vec<gpui::FocusHandle>,
    colors: &theme::ThemeColors,
    cx: &mut Context<ShellApp>,
) -> Modal {
    Modal::new("settings", None)
        .header(
            ModalHeader::new()
                .headline("设置")
                .show_dismiss_button(true),
        )
        // ── 外观 ───────────────────────────────────────────
        .section(
            Section::new()
                .header(SectionHeader::new("外观"))
                .child(control_row(
                    "sg-theme-row",
                    "主题",
                    theme_choices(theme_mode, colors, cx),
                ))
                // #110 G1 批B：具名主题行（下拉 + 删除钮——四态轴第四面）。
                .child(named_theme_row(theme_mode, theme_name_input.as_str(), colors, cx)),
        )
        // ── #110 G4：自定义主题色（外观组内子区——D3 一期）─────────
        .section(
            Section::new()
                .header(SectionHeader::new("自定义主题色"))
                .child(custom_theme_editor(
                    custom_theme_draft,
                    theme_name_input,
                    color_slot_focus,
                    colors,
                    cx,
                )),
        )
        // ── 编辑器 ─────────────────────────────────────────
        .section(
            Section::new()
                .header(SectionHeader::new("编辑器"))
                .child(control_row(
                    "sg-font-row",
                    "字号",
                    font_stepper(font_size, colors, cx),
                ))
                .child(control_row(
                    "sg-indent-row",
                    "缩进单位",
                    indent_choices(indent_unit, colors, cx),
                ))
                // #111 P2：软换行开关（双态钮——关 = 长行不折断横向滚动）。
                .child(control_row(
                    "sg-soft-wrap-row",
                    "软换行",
                    soft_wrap_choices(soft_wrap, colors, cx),
                )),
        )
        // ── 布局 ───────────────────────────────────────────
        .section(
            Section::new()
                .header(SectionHeader::new("布局"))
                .child(control_row(
                    "sg-width-row",
                    "面板宽度",
                    action_button(
                        "sg-width-reset",
                        "重置默认",
                        colors.element_hover,
                        cx.listener(|this, _, _, cx| this.reset_panel_widths(cx)),
                    ),
                )),
        )
}

/// #110 G1 批B：主题名输入框（hex 槽同款键捕获——plain 守卫 + Enter 保存快捷）。
fn theme_name_field(
    theme_name_input: &str,
    colors: &theme::ThemeColors,
    cx: &mut Context<ShellApp>,
) -> impl IntoElement {
    let hover = colors.element_hover;
    h_flex()
        .id("sg-theme-name-field")
        .gap_2()
        .items_center()
        .child(
            div().w(px(44.)).child(
                Label::new("主题名")
                    .size(LabelSize::XSmall)
                    .color(ui::Color::Default),
            ),
        )
        .child(
            div()
                .id("sg-theme-name-input")
                .flex_1()
                .min_w_0()
                .h(px(22.))
                .px_1()
                .rounded_sm()
                .border_1()
                .border_color(colors.border)
                .hover(|el| el.bg(hover))
                .on_key_down(cx.listener(
                    move |this, event: &gpui::KeyDownEvent, _, cx| {
                        let ks = &event.keystroke;
                        let key = ks.key.to_lowercase();
                        let plain = !ks.modifiers.control
                            && !ks.modifiers.alt
                            && !ks.modifiers.platform;
                        let mut text = this.theme_name_input().to_string();
                        if plain && (key == "backspace" || key == "escape") {
                            if key == "backspace" {
                                text.pop();
                            } else {
                                text.clear();
                            }
                        } else if plain && key == "enter" {
                            let _ = this.save_custom_theme(cx);
                            cx.notify();
                            return;
                        } else if plain && key.chars().count() == 1 {
                            // 名域单字符（非法字符 set_theme_name_input 过滤）。
                            let mut ch = key.chars().next().unwrap();
                            if ks.modifiers.shift && ch.is_ascii_alphabetic() {
                                ch = ch.to_ascii_uppercase();
                            }
                            text.push(ch);
                        }
                        this.set_theme_name_input(text);
                        cx.notify();
                    },
                ))
                .child(
                    Label::new(if theme_name_input.is_empty() {
                        // 空名占位：my-theme（保存旧行为提示）。
                        "my-theme"
                    } else {
                        theme_name_input
                    })
                    .size(LabelSize::XSmall)
                    .color(ui::Color::Default),
                ),
        )
}

/// #110 G1 批B：磁盘枚举用户主题名（渲染期安全版——settings 静态面）。
fn user_theme_names_from_disk() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    if let Some(dir) = crate::settings::themes_dir() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) == Some("json") {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        names.push(stem.to_string());
                    }
                }
            }
        }
    }
    names.sort();
    names
}

/// #110 G1 批B：具名主题行（下拉 = registry 用户主题选择钮排 + 删除钮）。
/// 选中态 = theme_mode 是 user:<name>；点击 → set_theme_mode(user:<name>)
/// （apply_mode 具名分支 registry 命中即切）。删除钮只对已保存主题有效
/// （键入名与枚举集都不含时禁用语义——点删无效果提示）。
fn named_theme_row(
    theme_mode: &str,
    theme_name_input: &str,
    colors: &theme::ThemeColors,
    cx: &mut Context<ShellApp>,
) -> impl IntoElement {
    let hover = colors.element_hover;
    let selected = colors.element_selected;
    // 磁盘直读（与 ShellApp::user_theme_names 同逻辑——渲染期零
    // entity.update 嵌套；themes_dir 是 settings 静态函数）。
    let names = user_theme_names_from_disk();
    let current_name = theme_switch::parse_user_mode(theme_mode);
    let delete_target = {
        // 删除目标：选中名优先，否则名框输入名（都无 → None 禁删）。
        current_name
            .map(|n| n.to_string())
            .or_else(|| {
                let t = theme_name_input.trim();
                if t.is_empty() {
                    None
                } else {
                    Some(t.to_string())
                }
            })
    };
    v_flex()
        .id("sg-named-theme-row")
        .gap_1()
        .when(names.is_empty(), |el| {
            el.child(
                Label::new("（无自定义主题——下方编辑后「保存为主题」）")
                    .size(LabelSize::XSmall)
                    .color(ui::Color::Muted),
            )
        })
        .children(names.iter().map(|name| {
            let is_selected = current_name == Some(name.as_str());
            h_flex()
                .id(SharedString::from(format!("sg-user-theme-{name}")))
                .gap_2()
                .items_center()
                .child(
                    div()
                        .id(SharedString::from(format!("sg-user-theme-pick-{name}")))
                        .h(px(20.))
                        .px_2()
                        .rounded_sm()
                        .when(is_selected, |el| el.bg(selected))
                        .hover(|el| el.bg(hover))
                        .child(
                            Label::new(name.as_str())
                                .size(LabelSize::XSmall)
                                .color(ui::Color::Default),
                        )
                        .on_click({
                            let name = name.clone();
                            cx.listener(move |this, _, _, cx| {
                                let mode = theme_switch::user_mode(&name);
                                this.set_theme_mode(&mode, cx);
                                cx.notify();
                            })
                        }),
                )
                .when(delete_target.as_deref() == Some(name.as_str()), |el| {
                    el.child(
                        div()
                            .id(SharedString::from(format!("sg-user-theme-del-{name}")))
                            .h(px(20.))
                            .px_2()
                            .rounded_sm()
                            .hover(|el| el.bg(hover))
                            .child(
                                Label::new("删除")
                                    .size(LabelSize::XSmall)
                                    .color(ui::Color::Error),
                            )
                            .on_click({
                                let name = name.clone();
                                cx.listener(move |this, _, _, cx| {
                                    if this.delete_user_theme(&name, cx) {
                                        this.notice_message(
                                            format!("主题 {name} 已删除（回退深色）"),
                                            cx,
                                        );
                                    } else {
                                        this.notice_message(
                                            format!("主题 {name} 不存在（未删）"),
                                            cx,
                                        );
                                    }
                                    cx.notify();
                                })
                            }),
                    )
                })
        }))
}

/// 主题三选钮（跟随系统/浅色/深色）。
fn theme_choices(
    theme_mode: &str,
    colors: &theme::ThemeColors,
    cx: &mut Context<ShellApp>,
) -> impl IntoElement {
    let hover = colors.element_hover;
    let selected = colors.element_selected;
    h_flex()
        .id("sg-theme-choices")
        .gap_1()
        .child(choice_button(
            "sg-theme-system",
            "跟随系统",
            theme_mode == theme_switch::MODE_SYSTEM,
            hover,
            selected,
            cx.listener(|this, _, _, cx| this.set_theme_mode(theme_switch::MODE_SYSTEM, cx)),
        ))
        .child(choice_button(
            "sg-theme-light",
            "浅色",
            theme_mode == theme_switch::MODE_LIGHT,
            hover,
            selected,
            cx.listener(|this, _, _, cx| this.set_theme_mode(theme_switch::MODE_LIGHT, cx)),
        ))
        .child(choice_button(
            "sg-theme-dark",
            "深色",
            theme_mode == theme_switch::MODE_DARK,
            hover,
            selected,
            cx.listener(|this, _, _, cx| this.set_theme_mode(theme_switch::MODE_DARK, cx)),
        ))
}

/// #110 G4：自定义主题色编辑子区（D3 一期降级——色块预览 + hex
/// 文本输入 + 坏色行内报错；outline 过滤框同款键捕获范式）。
/// 6 高频槽与 G2 装载器同源（background/surface/editor/foreground/
/// accent/border）；空槽初值 = 当前主题实际色（所见即所改）。
fn custom_theme_editor(
    custom_theme_draft: [String; 31],
    theme_name_input: String,
    color_slot_focus: Vec<gpui::FocusHandle>,
    colors: &theme::ThemeColors,
    cx: &mut Context<ShellApp>,
) -> impl IntoElement {
    let hover = colors.element_hover;
    let slot_labels = [
        ("background", "背景"),
        ("surface", "面板"),
        ("editor", "编辑器"),
        ("foreground", "前景"),
        ("accent", "强调"),
        ("border", "边框"),
    ];
    let draft: Vec<String> = custom_theme_draft
        .iter()
        .map(|s| s.trim().to_string())
        .collect();
    let slot_handles: Vec<gpui::FocusHandle> = color_slot_focus;
    let current = [
        colors.background,
        colors.elevated_surface_background,
        colors.editor_background,
        colors.text,
        colors.element_active,
        colors.border,
    ];
    v_flex()
        .id("sg-custom-theme")
        .gap_1()
        // #110 G1 批B：主题名输入框（保存用——空 = my-theme 旧行为；
        // 键捕获范式同 hex 槽：plain 守卫 + 非法字符拒 + 40 界）。
        .child(theme_name_field(theme_name_input.as_str(), colors, cx))
        .children(slot_labels.iter().enumerate().map(|(i, (key, label))| {
            let raw = &draft[i];
            let cur = ShellApp::custom_theme_slot_current(i, colors);
            let parsed = if raw.is_empty() {
                Some(cur)
            } else {
                crate::settings::parse_hsla(raw)
            };
            let bad = parsed.is_none();
            let swatch = parsed.unwrap_or(gpui::hsla(0., 0., 0.5, 1.));
            let error_color = colors.border;
            h_flex()
                // #110 G1 批A：组界组头（0/6/11/17/22/27 六组——行首组
                // 标签，31 槽分组的视觉锚）。
                .when(matches!(i, 0 | 6 | 11 | 17 | 22 | 27), |el| {
                    let group = match i {
                        0 => "高频六槽（G2 键）",
                        6 => "边框族",
                        11 => "元素族",
                        17 => "幽灵族（透明系）",
                        22 => "文本与面板",
                        _ => "图标族与主色",
                    };
                    el.mt_1().child(
                        div()
                            .id(SharedString::from(format!("sg-color-group-{i}")))
                            .w_full()
                            .child(
                                Label::new(group)
                                    .size(LabelSize::XSmall)
                                    .color(ui::Color::Muted),
                            ),
                    )
                })
                .id(SharedString::from(format!("sg-color-slot-{key}")))
                .gap_2()
                .items_center()
                .child(
                    div().w(px(44.)).child(
                        Label::new(*label)
                            .size(LabelSize::XSmall)
                            .color(ui::Color::Default),
                    ),
                )
                // 色块预览（坏色灰显）。
                .child(
                    div()
                        .id(SharedString::from(format!("sg-color-swatch-{key}")))
                        .size(px(18.))
                        .rounded_sm()
                        .border_1()
                        .border_color(colors.border)
                        .bg(bad.then_some(colors.border).unwrap_or(swatch)),
                )
                // hex 文本输入（键捕获范式——一期单行编辑语义）。
                .child(
                    div()
                        .id(SharedString::from(format!("sg-color-input-{key}")))
                        .flex_1()
                        .min_w_0()
                        .h(px(22.))
                        .px_1()
                        .rounded_sm()
                        .border_1()
                        .border_color(if bad { error_color } else { colors.border })
                        .track_focus(&slot_handles[i])
                        .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                            // 键读法照 outline 过滤框先例（keystroke.key
                            // 小写化 + plain 守卫——修饰组合不进编辑）。
                            let ks = &event.keystroke;
                            let key = ks.key.to_lowercase();
                            let plain = !ks.modifiers.control
                                && !ks.modifiers.alt
                                && !ks.modifiers.platform;
                            let slot = i;
                            let mut text = this.custom_theme_draft()[slot].clone();
                            if plain && (key == "backspace" || key == "escape") {
                                if key == "backspace" {
                                    text.pop();
                                } else {
                                    text.clear();
                                }
                            } else if plain && key == "enter" {
                                // Enter = 保存快捷（草稿链收口）。
                                let _ = this.save_custom_theme(cx);
                                cx.notify();
                                return;
                            } else if plain && key.chars().count() == 1 {
                                // hex 域单字符（# 与 0-9a-f；超 7 字符拒收——#rrggbb）。
                                let mut ch = key.chars().next().unwrap();
                                if ks.modifiers.shift && ch.is_ascii_alphabetic() {
                                    ch = ch.to_ascii_uppercase();
                                }
                                if (ch.is_ascii_hexdigit() || ch == '#')
                                    && text.chars().count() < 7
                                {
                                    text.push(ch);
                                }
                            }
                            this.set_custom_theme_slot(slot, text);
                            cx.notify();
                        }))
                        .child(
                            Label::new(if raw.is_empty() {
                                // 空槽显示当前主题实际色 hex（初值语义）。
                                format!(
                                    "#{:02x}{:02x}{:02x}",
                                    ((gpui::Rgba::from(cur).r * 255.0).round() as u8),
                                    (gpui::Rgba::from(cur).g * 255.0).round() as u8,
                                    (gpui::Rgba::from(cur).b * 255.0).round() as u8
                                )
                            } else {
                                raw.clone()
                            })
                            .size(LabelSize::XSmall)
                            .color(if bad { ui::Color::Error } else { ui::Color::Default }),
                        ),
                )
                // 坏色行内报错。
                .when(bad, |el| {
                    el.child(
                        Label::new("无效颜色")
                            .size(LabelSize::XSmall)
                            .color(ui::Color::Error),
                    )
                })
        }))
        // 「保存为主题」钮（choice_button 同视觉语系——div+Label+on_click）。
        .child(
            h_flex()
                .id("sg-custom-theme-save")
                .justify_end()
                .child(
                    div()
                        .id("sg-theme-save")
                        .h(CONTROL_H)
                        .px_2()
                        .flex()
                        .items_center()
                        .rounded_sm()
                        .hover(|el| el.bg(hover))
                        .child(
                            Label::new("保存为主题")
                                .size(LabelSize::Small)
                                .color(Color::Default),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            match this.save_custom_theme(cx) {
                                Some(path) => {
                                    this.notice_message(format!(
                                        "自定义主题已保存并应用（{}）",
                                        path.display()
                                    ), cx);
                                }
                                None => {
                                    this.notice_message(
                                        "自定义主题未保存——槽色无效（见行内报错）".into(),
                                        cx,
                                    );
                                }
                            }
                        })),
                ),
        )
}

/// 字号步进（−/值/+；域界 8-32 灰显）。
fn font_stepper(
    font_size: f32,
    colors: &theme::ThemeColors,
    cx: &mut Context<ShellApp>,
) -> impl IntoElement {
    let hover = colors.element_hover;
    h_flex()
        .id("sg-font-stepper")
        .gap_1()
        .child(step_button(
            "sg-font-dec",
            IconName::SquareMinus,
            "减小字号",
            font_size > 8.,
            hover,
            cx.listener(move |this, _, _, cx| {
                this.apply_font_size(px((font_size - 1.).max(8.)), cx);
            }),
        ))
        .child(
            div()
                .id("sg-font-value")
                .h(CONTROL_H)
                .flex()
                .items_center()
                .px_2()
                .child(
                    Label::new(format!("{font_size:.0} px"))
                        .size(LabelSize::Small)
                        .color(Color::Default),
                ),
        )
        .child(step_button(
            "sg-font-inc",
            IconName::Plus,
            "增大字号",
            font_size < 32.,
            hover,
            cx.listener(move |this, _, _, cx| {
                this.apply_font_size(px((font_size + 1.).min(32.)), cx);
            }),
        ))
}

/// 缩进单位三选钮（4 空格/2 空格/Tab）。
/// #111 P2：软换行双态钮（开/关——沿 indent_choices 语系）。
fn soft_wrap_choices(
    soft_wrap: bool,
    colors: &theme::ThemeColors,
    cx: &mut Context<ShellApp>,
) -> impl IntoElement {
    let hover = colors.element_hover;
    let selected = colors.element_selected;
    h_flex()
        .id("sg-soft-wrap-choices")
        .gap_1()
        .child(choice_button(
            "sg-soft-wrap-on",
            "开",
            soft_wrap,
            hover,
            selected,
            cx.listener(|this, _, _, cx| this.set_soft_wrap_all(true, cx)),
        ))
        .child(choice_button(
            "sg-soft-wrap-off",
            "关",
            !soft_wrap,
            hover,
            selected,
            cx.listener(|this, _, _, cx| this.set_soft_wrap_all(false, cx)),
        ))
}

fn indent_choices(
    indent_unit: IndentUnit,
    colors: &theme::ThemeColors,
    cx: &mut Context<ShellApp>,
) -> impl IntoElement {
    let hover = colors.element_hover;
    let selected = colors.element_selected;
    h_flex()
        .id("sg-indent-choices")
        .gap_1()
        .child(choice_button(
            "sg-indent-4",
            "4 空格",
            indent_unit == IndentUnit::Spaces4,
            hover,
            selected,
            cx.listener(|this, _, _, cx| this.set_indent_unit_all(IndentUnit::Spaces4, cx)),
        ))
        .child(choice_button(
            "sg-indent-2",
            "2 空格",
            indent_unit == IndentUnit::Spaces2,
            hover,
            selected,
            cx.listener(|this, _, _, cx| this.set_indent_unit_all(IndentUnit::Spaces2, cx)),
        ))
        .child(choice_button(
            "sg-indent-tab",
            "Tab",
            indent_unit == IndentUnit::Tab,
            hover,
            selected,
            cx.listener(|this, _, _, cx| this.set_indent_unit_all(IndentUnit::Tab, cx)),
        ))
}

/// 「标签 + 控件」行（标签定宽对齐，控件靠左跟随）。
///
/// 容器不换 `ModalRow`（组件仅是 py_1 行包装，不 impl
/// `InteractiveElement`——无 id/无事件面，包装价值低于 id 损失；
/// 迁移重心在 Modal/ModalHeader/Section 骨架层）。
fn control_row(id: &'static str, label: &str, controls: impl IntoElement) -> impl IntoElement {
    h_flex()
        .id(id)
        .gap_2()
        .py_1()
        .child(
            div()
                .id(SharedString::from(format!("{id}-label")))
                .w(LABEL_W)
                .flex_none()
                .h(CONTROL_H)
                .flex()
                .items_center()
                .child(
                    Label::new(label.to_string())
                        .size(LabelSize::Small)
                        .color(Color::Default),
                ),
        )
        .child(controls)
}

/// 单选钮（选中 = element_selected 实底 + 文字 Selected 色；未选中
/// hover 浮起——`activity_bar.rs` 的 bar_button 同款视觉语系）。
fn choice_button(
    id: &'static str,
    label: &'static str,
    is_selected: bool,
    hover_bg: Hsla,
    selected_bg: Hsla,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .h(CONTROL_H)
        .px_2()
        .flex()
        .items_center()
        .rounded_sm()
        .when(is_selected, |el| el.bg(selected_bg))
        .hover(|el| el.bg(hover_bg))
        .child(
            Label::new(label)
                .size(LabelSize::Small)
                .color(if is_selected {
                    Color::Selected
                } else {
                    Color::Default
                }),
        )
        .on_click(on_click)
}

/// 步进钮（−/+——`enabled = false`（域界）时灰显且不挂 on_click：
/// 点不到不算死按钮，与 context_menu_view 灰显项同口径）。
fn step_button(
    id: &'static str,
    icon: IconName,
    tooltip: &'static str,
    enabled: bool,
    hover_bg: Hsla,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .size(CONTROL_H)
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .when(enabled, |el| {
            el.hover(|el| el.bg(hover_bg))
                .on_click(on_click)
                .child(Icon::new(icon).size(IconSize::XSmall))
                .tooltip(Tooltip::text(tooltip))
        })
        .when(!enabled, |el| {
            el.child(
                Icon::new(icon)
                    .size(IconSize::XSmall)
                    .color(Color::Disabled),
            )
        })
}

/// 动作钮（面板宽度重置等——choice_button 的单发形态：普通底、
/// hover 浮起、按下有真实行为）。
fn action_button(
    id: &'static str,
    label: &'static str,
    hover_bg: Hsla,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .h(CONTROL_H)
        .px_2()
        .flex()
        .items_center()
        .rounded_sm()
        .hover(|el| el.bg(hover_bg))
        .child(
            Label::new(label.to_string())
                .size(LabelSize::Small)
                .color(Color::Default),
        )
        .on_click(on_click)
}
