//! 命令面板的分组渲染（#63 打磨项；#385 批 3b：列表迁 zed 组件族）。
//!
//! 数据与顺序在 [`crate::command_groups`]（纯逻辑、有单测）；本模块只负责
//! 把 [`Section`] 画成「段标题 + 条目 + 右侧快捷键」。
//!
//! 面板本体（输入框、键处理）在 `editor/mod.rs`——那里持有 `PaletteState`
//! 与浮层容器（#385 批 3b：浮层视觉壳已换 `ui::Popover`）。
//! 本模块提供 [`palette_sections`]，让编辑器在渲染时按分组取数据。

use gpui::{App, SharedString, px};
use ui::prelude::*;
use ui::{ListHeader, ListItem};

use crate::command_groups::{Entry, Group, RecentRing, Section, build_sections, flatten};
use crate::editor::commands::{self, Cmd};

/// 最近使用容量（面板顶部固定区长度）。
pub const RECENT_CAPACITY: usize = 4;

/// 计算面板当前应显示的段（查询 → 分组）。
pub fn palette_sections(query: &str, recent: &RecentRing) -> Vec<Section> {
    let results = commands::search(query);
    build_sections(&results, recent)
}

/// 扁平命令序（键盘上下移动的遍历域）。
pub fn palette_flat(sections: &[Section]) -> Vec<Cmd> {
    flatten(sections)
}

/// 渲染面板列表区（#385 批 3b：段标题 → `ListHeader`、条目 →
/// `ListItem`——选中态/排版/间距主题推导）。
///
/// 行不挂 `on_click`（行为守恒：面板选中是键盘 ↑↓ 驱动，
/// `PaletteState.selected` 单一状态源；ListItem 只消费选中态）。
///
/// `selected` = 扁平序里的选中下标；`max_rows` 限制总行数（面板不无限长）。
pub fn palette_list(
    sections: &[Section],
    selected: usize,
    max_rows: usize,
    cx: &App,
) -> impl IntoElement {
    let colors = cx.theme().colors().clone();
    let mut col = div().id("palette-sections").flex().flex_col().gap_1();

    let mut drawn = 0usize;
    for section in sections {
        if drawn >= max_rows {
            break;
        }
        // 段标题（最近使用 / 文件 / 编辑 / 导航 / 视图 / 帮助）。
        col = col.child(ListHeader::new(section.group.title()));
        for Entry { cmd, index } in &section.entries {
            if drawn >= max_rows {
                break;
            }
            drawn += 1;
            let is_sel = *index == selected;
            col = col.child(
                ListItem::new(SharedString::from(format!("pal-row-{index}")))
                    .inset(true)
                    .toggle_state(is_sel)
                    .child(div().text_color(colors.text).child(cmd.title()))
                    .end_slot(
                        div()
                            .text_size(px(11.))
                            .text_color(colors.text_muted)
                            .child(cmd.default_key().unwrap_or("")),
                    ),
            );
        }
    }

    if drawn == 0 {
        col = col.child(
            div()
                .px_2()
                .py_2()
                .text_size(px(12.))
                .text_color(colors.text_muted)
                .child("无匹配命令"),
        );
    }
    col
}

/// 段标题是否应显示（最近组为空时不显示"最近使用"标题）。
pub fn section_visible(group: Group, entries: &[Entry]) -> bool {
    if group == Group::Recent {
        return !entries.is_empty();
    }
    !entries.is_empty()
}
