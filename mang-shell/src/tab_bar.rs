//! 多标签（#68 阶段 F3；#385 批 2：渲染层迁 `ui::TabBar`/`Tab`）。
//!
//! 分层：
//! - 纯逻辑（无 gpui 依赖，单测钉定）：[`TabItem`] 载体、
//!   [`active_after_close`]（关闭后邻位规则）、[`cycle_active`]（循环
//!   切换）、[`tab_label`]（同名消歧——文件名/工作区相对路径/父目录
//!   前缀三级）；
//! - 渲染：[`tab_bar`]——zed ui 组件语系（激活态/选中凸起边框/
//!   标签栏背景/高度全部主题推导，主题切换即时跟随）。
//!
//! 快捷键（shell 级）：Ctrl+W 关闭当前（切邻位）；Ctrl+Tab /
//! Ctrl+Shift+Tab 循环切换。编辑器 `tab` 键臂已加 `!ctrl` 守卫、
//! 只读门禁放行 `tab + ctrl`（导航组合）——Ctrl+Tab 从编辑态冒泡。

use std::cmp::Ordering;
use std::path::Path;

use gpui::{Context, Entity, MouseButton, SharedString, px};
use ui::prelude::*;
use ui::{Label, LabelSize, Tab, TabBar, TabPosition};

use crate::editor::Editor;
use crate::shell::ShellApp;

/// 一个标签（打开的文档）。
#[derive(Clone)]
pub struct TabItem {
    pub path: std::path::PathBuf,
    pub editor: Entity<Editor>,
}

/// 关闭 `active` 后的新 active 序（关闭前长度 `old_len`）：
/// 右邻优先（被关位的后继前移补位）；关末位回退左邻；关唯一标签 → 0
/// （空态）。VS Code 同款惯例。
pub fn active_after_close(old_len: usize, active: usize) -> usize {
    if old_len <= 1 {
        0
    } else if active + 1 < old_len {
        active
    } else {
        active - 1
    }
}

/// 循环切换（wrap）：forward = Ctrl+Tab 下一；backward = Ctrl+Shift+Tab
/// 上一。空集/单标签返回原值（调用方以此短路）。
pub fn cycle_active(len: usize, active: usize, forward: bool) -> usize {
    if len < 2 {
        return active;
    }
    (active + if forward { 1 } else { len - 1 }) % len
}

/// 标签文案（同名消歧三级）：
/// 1. 无同名 → 文件名；
/// 2. 同名且在工作区内 → 工作区相对路径（`sub/a.md`——相对路径含父
///    信息即消歧）；
/// 3. 其余（同名且不在工作区/相对路径无父信息）→ `父目录/文件名`。
pub fn tab_label(path: &Path, all: &[&Path], workspace: Option<&Path>) -> String {
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return path.display().to_string();
    };
    let duplicated = all
        .iter()
        .any(|p| p.as_os_str() != path.as_os_str() && p.file_name() == path.file_name());
    if !duplicated {
        return name;
    }
    if let Some(ws) = workspace
        && let Ok(rel) = path.strip_prefix(ws)
        && rel.parent().is_some_and(|p| !p.as_os_str().is_empty())
    {
        return rel.display().to_string();
    }
    let parent = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    if parent.is_empty() {
        name
    } else {
        format!("{parent}/{name}")
    }
}

/// 标签栏（标题栏下方一条，仅在有标签时渲染；#385 批 2：zed 组件语系）。
///
/// 迁移面（原 152 行手写 → 组件）：
/// - 激活/非激活背景、文字色、选中凸起边框（`TabPosition` 位置语义
///   ——首/中/末与相对选中位）全部由 `Tab` 主题推导，主题切换即时
///   跟随（原手写 element_selected/element_hover 固定搭配退役）；
/// - 标签栏背景/高度/底部边框线/溢出横滚由 `TabBar` 承担
///   （原 h(30px) + title_bar_background 硬编码退役）；
/// - 保留：max_w(220) 长名限宽、脏标 ●（Accent，zed 习惯放 start_slot
///   图标位）、同名消歧文案、点击切换。
///
/// 每个标签的关闭钮经 Shell 的指定标签确认路径，未保存时不直接丢弃。
pub fn tab_bar(
    tabs: &[TabItem],
    active: usize,
    workspace: Option<&Path>,
    cx: &mut Context<ShellApp>,
) -> impl IntoElement {
    let paths: Vec<&Path> = tabs.iter().map(|t| t.path.as_path()).collect();
    let rows: Vec<(String, bool)> = tabs
        .iter()
        .map(|t| {
            (
                tab_label(&t.path, &paths, workspace),
                t.editor.read(cx).is_modified(),
            )
        })
        .collect();
    let dirty_dot_color = ui::Color::Accent.color(cx);

    TabBar::new("tab-bar").children(rows.iter().enumerate().map(|(idx, (label, dirty))| {
        let is_active = idx == active;
        let last = idx + 1 == rows.len();
        // 位置语义（Tab 边框策略的输入）：首/末固定；中间按相对
        // 选中位（Equal = 选中自身——完整边框 + 无底线「凸起」）。
        let position = if !last && idx == 0 {
            TabPosition::First
        } else if last && idx != 0 {
            TabPosition::Last
        } else if is_active {
            TabPosition::Middle(Ordering::Equal)
        } else if idx < active {
            TabPosition::Middle(Ordering::Less)
        } else {
            TabPosition::Middle(Ordering::Greater)
        };
        Tab::new(SharedString::from(format!("tab-{idx}")))
            .toggle_state(is_active)
            .position(position)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.activate_tab(idx, window, cx);
                }),
            )
            .when(*dirty, |tab| {
                tab.start_slot(div().text_color(dirty_dot_color).child("●"))
            })
            .end_slot(
                div()
                    .id(SharedString::from(format!("tab-close-{idx}")))
                    .debug_selector(move || format!("tab-close-{idx}"))
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.request_close_tab(idx, window, cx);
                    }))
                    .child(ui::Icon::new(ui::IconName::Close).size(ui::IconSize::XSmall)),
            )
            .child(
                // 长名限宽（Tab 无宽度 API——限在 label 面并中段截断
                // `…`，VS Code 同款；溢出横滚由 TabBar 承担）。
                div().max_w(px(220.)).child(
                    Label::new(label.clone())
                        .size(LabelSize::XSmall)
                        .map(|l| {
                            if is_active {
                                l
                            } else {
                                l.color(ui::Color::Muted)
                            }
                        })
                        .truncate_middle(),
                ),
            )
    }))
}

// ---------------------------------------------------------------------------
// 纯逻辑单测。
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    /// 关闭后邻位规则：右邻优先；末位回左邻；唯一标签归零。
    #[test]
    fn close_picks_right_neighbor_then_left() {
        // [A,B,C] 关 B(active 1) → C 前移补位，active 仍 1。
        assert_eq!(active_after_close(3, 1), 1);
        // 关末位 C(active 2) → 回退左邻 B → active 1。
        assert_eq!(active_after_close(3, 2), 1);
        // 关首位 A(active 0) → B 补位，active 0。
        assert_eq!(active_after_close(3, 0), 0);
        // 唯一标签 → 空态。
        assert_eq!(active_after_close(1, 0), 0);
        // 空集防御。
        assert_eq!(active_after_close(0, 0), 0);
    }

    /// 循环切换 wrap 语义。
    #[test]
    fn cycle_wraps_both_directions() {
        // 3 标签：0→1→2→0（forward）；0→2→1→0（backward）。
        assert_eq!(cycle_active(3, 0, true), 1);
        assert_eq!(cycle_active(3, 1, true), 2);
        assert_eq!(cycle_active(3, 2, true), 0, "末位 wrap 回首");
        assert_eq!(cycle_active(3, 0, false), 2, "首位 wrap 回末");
        assert_eq!(cycle_active(3, 2, false), 1);
        // 单标签/空集不变。
        assert_eq!(cycle_active(1, 0, true), 0);
        assert_eq!(cycle_active(0, 0, true), 0);
    }

    /// 同名消歧三级：无同名文件名 / 工作区相对 / 父目录前缀。
    #[test]
    fn label_disambiguates_duplicates() {
        let a = Path::new("/ws/sub/a.md");
        let b = Path::new("/ws/other/a.md");
        let c = Path::new("/elsewhere/a.md");
        let ws = Path::new("/ws");

        // 无同名 → 文件名。
        let solo = [a, Path::new("/ws/b.md")];
        assert_eq!(tab_label(a, &solo, Some(ws)), "a.md");

        // 同名且都在工作区内 → 相对路径。
        let pair = [a, b];
        assert_eq!(tab_label(a, &pair, Some(ws)), "sub/a.md");
        assert_eq!(tab_label(b, &pair, Some(ws)), "other/a.md");

        // 同名但不在工作区 → 父目录/文件名。
        let outside = [a, c];
        assert_eq!(tab_label(c, &outside, Some(ws)), "elsewhere/a.md");

        // 无工作区 → 父目录/文件名。
        assert_eq!(tab_label(a, &pair, None), "sub/a.md");
        // 工作区直下同名（rel 无父信息）→ 父目录兜底。
        let root_a = Path::new("/ws/a.md");
        let root_pair = [root_a, a];
        assert_eq!(tab_label(root_a, &root_pair, Some(ws)), "ws/a.md");
    }
}
