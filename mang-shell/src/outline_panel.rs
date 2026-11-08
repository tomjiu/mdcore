//! #571：大纲面板键盘导航（#385 批 5a 登记遗留件——批 7）。
//!
//! 批 5a（PR #482）交付说明：「键盘导航（Navigable）坦白未接——
//! panels 无状态渲染架构需先立焦点/选中状态面，挂后续批次」。本模块
//! 兑现该挂账：heading_tree 从 panels.rs 的无状态内联渲染改为
//! [`OutlinePanel`] 实体承载（循 FileTree/GitPanel「一面板一实体」
//! 先例），行焦点句柄按**行号**缓存复用（编辑后标题增删只重建差量
//! 行），键盘面经 [`ui::Navigable`] 接线：
//!
//! - `↑/↓`（menu::SelectPrevious/SelectNext——main.rs 既有 menu 键位族）
//!   在行间移动焦点（`NavigableEntry` 序 = 行序）；
//! - `Enter`（menu::Confirm）激活聚焦行 = 现点击同链
//!   （[`Editor::jump_to_line`] + #289-④ pdf 页滚钩子）；
//! - `Esc`（menu::Cancel）焦点还给编辑区（面板键盘域退出）；
//! - 行视觉 = 批 5a 组件面原样（List/ListHeader/ListItem + 12px 缩进），
//!   键盘焦点行经 `ListItem::focused` 呈现。
//!
//! # lib 树纪律与依赖方向
//!
//! 本模块进 lib 树（`#[gpui::test]` 无头测试挂 bin 目标 = CI 永不运行
//! ——假绿，document.rs 同纪律先例）。因此**不引用 [`crate::shell`]
//! （bin 树私有）**：#289-④ 的 pdf 页滚动副作用经 `activate_hook`
//! 回调注入（shell.rs 构造实体时捕获 `WeakEntity<ShellApp>` 闭包）——
//! 依赖倒置，palette_view（lib 供 bin 消费）同型。
//!
//! 已知边界：两标题同行的 md 边缘（`# A ## B` 同行双标题）共享焦点
//! 句柄（行号即键）——跳转目标相同，无行为歧义。

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, Context, Entity, FocusHandle, ScrollAnchor, ScrollHandle, Window};
use ui::prelude::*;
use ui::{
    Icon, IconName, IconSize, Label, LabelSize, List, ListHeader, ListItem, Navigable,
    NavigableEntry,
};

use crate::editor::Editor;

/// 激活钩子（#289-④ pdf 页滚动等 bin 面副作用注入位）。
/// 入参 = 激活行号；`jump_to_line` 已由面板本体先行执行。
pub type ActivateHook = Arc<dyn Fn(usize, &mut Window, &mut App)>;

/// 钩子的借用形态（clippy type_complexity 收口）。
type ActivateHookRef<'a> = Option<&'a dyn Fn(usize, &mut Window, &mut App)>;

/// 大纲行（渲染数据 + 行焦点句柄）。句柄按行号缓存于
/// [`OutlinePanel::handles`]，跨编辑重建复用（焦点身份稳定）。
#[derive(Clone)]
struct OutlineRow {
    label: String,
    level: usize,
    line: usize,
    focus_handle: FocusHandle,
}

/// 大纲面板实体（结构树——md 标题 / docx 标题树 / pdf 书签树的
/// 既有 [`Editor::headings`] 数据面）。
///
/// 订阅编辑器通知：buffer 编辑 → 行集差量刷新（句柄按行号保留）。
pub struct OutlinePanel {
    editor: Entity<Editor>,
    rows: Vec<OutlineRow>,
    /// 行号 → 焦点句柄缓存（差量复用键）。
    handles: HashMap<usize, FocusHandle>,
    /// 大纲体滚动句柄（NavigableEntry 滚动锚点——键盘导航越视口滚动）。
    scroll: ScrollHandle,
    /// bin 面副作用注入（见模块文档「lib 树纪律」）。
    activate_hook: Option<ActivateHook>,
}

impl OutlinePanel {
    /// 构造（订阅编辑器 + 首轮行集刷新）。
    pub fn new(
        editor: Entity<Editor>,
        activate_hook: Option<ActivateHook>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            editor,
            rows: Vec::new(),
            handles: HashMap::new(),
            scroll: ScrollHandle::new(),
            activate_hook,
        };
        this.refresh(cx);
        // 编辑器通知（buffer 编辑/标题增删）→ 行集差量刷新。
        // focus 句柄按行号保留：编辑只影响差量行的焦点身份。
        cx.observe(&this.editor, |this, _, cx| this.refresh(cx))
            .detach();
        this
    }

    /// 宿主编辑器（shell 侧实体换绑判据）。
    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    /// 行集是否为空（left_panel 沿用「无标题不渲染分组」的批 5a 前语义）。
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// 行集差量刷新：重建行数据，句柄按行号保留存续行、清退删除行、
    /// 补建新增行。
    ///
    /// 数据面统一：md 走 [`Editor::md_outline`]（原仅 RPC 消费——面板
    /// 从未呈现 md 大纲，本批补齐：主格式首次有结构树）；渲染态
    /// （docx/pdf/xlsx）走 [`Editor::headings`]（批 5a 前同源）。标签
    /// 同 `Hn · 文本` 语系（`heading_label` 同构）。
    fn refresh(&mut self, cx: &App) {
        let e = self.editor.read(cx);
        let headings: Vec<(String, usize, usize)> = match e.md_outline() {
            Some(md) => md
                .into_iter()
                .map(|(level, line, title)| (format!("H{level} · {title}"), level as usize, line))
                .collect(),
            None => e
                .headings()
                .iter()
                .map(|h| (e.heading_label(h), h.level, h.line))
                .collect(),
        };
        let live: std::collections::HashSet<usize> =
            headings.iter().map(|(_, _, line)| *line).collect();
        self.handles.retain(|line, _| live.contains(line));
        self.rows = headings
            .into_iter()
            .map(|(label, level, line)| {
                let focus_handle = self
                    .handles
                    .entry(line)
                    .or_insert_with(|| cx.focus_handle())
                    .clone();
                OutlineRow {
                    label,
                    level,
                    line,
                    focus_handle,
                }
            })
            .collect();
    }

    /// 激活行（Enter/点击同链）：编辑器跳行 + bin 钩子（#289-④）。
    fn activate(
        editor: &Entity<Editor>,
        line: usize,
        hook: ActivateHookRef<'_>,
        window: &mut Window,
        cx: &mut App,
    ) {
        editor.update(cx, |ed, _| ed.jump_to_line(line));
        if let Some(hook) = hook {
            hook(line, window, cx);
        }
    }
}

impl gpui::Render for OutlinePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let editor = self.editor.clone();
        let hook = self.activate_hook.clone();
        let rows = self.rows.clone();
        let scroll = self.scroll.clone();

        // 行体（批 5a 组件面原样 + 焦点挂载）：外层 stateful div 持
        // track_focus（ListItem 无焦点 API——ui 组件面单证），点击聚焦；
        // ListItem 承载视觉与跳行（批 5a 点击链不动）。
        let body = div()
            .id("outline-body")
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .child(
                List::new()
                    .header(
                        ListHeader::new("文档结构").toggle(true).start_slot(
                            Icon::new(IconName::ListTree)
                                .size(IconSize::XSmall)
                                .color(ui::Color::Muted),
                        ),
                    )
                    .children(rows.iter().map(|row| {
                        let handle = row.focus_handle.clone();
                        let editor = editor.clone();
                        let hook = hook.clone();
                        let line = row.line;
                        let focused = row.focus_handle.contains_focused(window, cx);
                        div()
                            .id(SharedString::from(format!("outline-row-{line}")))
                            .track_focus(&handle)
                            // 点击 = 聚焦该行（键盘导航起点）。
                            .on_click({
                                let handle = handle.clone();
                                move |_, window, cx| handle.focus(window, cx)
                            })
                            .child(
                                ListItem::new(SharedString::from(format!("outline-item-{line}")))
                                    .inset(true)
                                    // 层级缩进：每级 12px（1 级不缩进——批 5a 同款）。
                                    .indent_level(row.level.saturating_sub(1))
                                    .focused(focused)
                                    .child(Label::new(row.label.as_str()).size(LabelSize::Small))
                                    .on_click(move |_, window, cx| {
                                        Self::activate(&editor, line, hook.as_deref(), window, cx);
                                    }),
                            )
                    })),
            );

        // Navigable 实体集（键序 = 行序——SelectNext/Previous 遍历语义；
        // 滚动锚点 = 大纲体滚动句柄：焦点行越视口时滚入）。
        let mut navigable = Navigable::new(body.into_any_element());
        for row in &rows {
            navigable = navigable.entry(NavigableEntry {
                focus_handle: row.focus_handle.clone(),
                scroll_anchor: Some(ScrollAnchor::for_handle(scroll.clone())),
            });
        }

        div()
            .id("outline-panel")
            // ↑↓/Enter/Esc 走 main.rs 既有 menu 键位族（批 4 注册面——
            // 键上下文匹配聚焦元素祖先链，本根即 "menu"）。
            .key_context("menu")
            // Enter（menu::Confirm）：激活聚焦行。
            .on_action({
                let rows = rows.clone();
                let editor = editor.clone();
                let hook = hook.clone();
                move |_: &menu::Confirm, window, cx| {
                    if let Some(row) = rows
                        .iter()
                        .find(|r| r.focus_handle.contains_focused(window, cx))
                    {
                        Self::activate(&editor, row.line, hook.as_deref(), window, cx);
                    }
                }
            })
            // Esc（menu::Cancel）：焦点还给编辑区（面板键盘域退出；
            // 不消费时该 action 会冒泡进窗口级关闭链——显式收口）。
            .on_action({
                let editor = editor.clone();
                move |_: &menu::Cancel, window, cx| {
                    let handle = editor.read(cx).focus_handle().clone();
                    handle.focus(window, cx);
                }
            })
            .child(navigable)
    }
}

// ---------------------------------------------------------------------------
// #571 无头测试三面（实体差量 / 导航面 / 激活面——对照表 §六 质量门 1
// 「点击 → 状态变化」级）。
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::buffer::Point;
    use gpui::{TestAppContext, VisualTestContext, size};

    /// md 编辑器构造（editor/mod.rs md_editor 同款装载面）。
    fn md_editor(cx: &mut TestAppContext, text: &str) -> gpui::Entity<Editor> {
        cx.update(|cx| cx.new(|cx| Editor::new(Some("notes.md".into()), text.to_string(), cx)))
    }

    /// 面板宿主（Host 模式——menu_bar_lifecycle_tests 同款：独立小宿主
    /// 开真窗口，面板实体经 Entity<Render> 挂载）。
    struct Host(gpui::Entity<OutlinePanel>);

    impl gpui::Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
            div().size_full().child(self.0.clone())
        }
    }

    /// 面板 + 宿主窗口（开窗 → draw 一帧 → VisualTestContext；
    /// 主题初始化序列镜像 main.rs——ButtonLike/ListItem 渲染需
    /// GlobalThemeSettingsProvider）。
    fn panel_window(
        cx: &mut TestAppContext,
        text: &str,
    ) -> (
        gpui::Entity<Editor>,
        gpui::Entity<OutlinePanel>,
        VisualTestContext,
    ) {
        cx.update(|app| {
            theme::init(theme::LoadThemes::JustBase, app);
            theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
        });
        let editor = md_editor(cx, text);
        let panel = cx.update(|cx| {
            let editor = editor.clone();
            cx.new(|cx| OutlinePanel::new(editor, None, cx))
        });
        let window = cx.open_window(size(gpui::px(260.), gpui::px(400.)), {
            let panel = panel.clone();
            move |_, _| Host(panel)
        });
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        (editor, panel, visual)
    }

    /// 实体面：行集随编辑差量刷新 + 存续行焦点句柄跨重建复用
    /// （焦点身份稳定——键盘导航不因编辑断链）。
    #[gpui::test]
    fn rows_refresh_and_handles_stable_across_edits(cx: &mut TestAppContext) {
        let editor = md_editor(cx, "# A\n正文\n## B\n尾段");
        let panel = cx.update(|cx| {
            let editor = editor.clone();
            cx.new(|cx| OutlinePanel::new(editor, None, cx))
        });

        // 初始两行（H1/H2）。
        panel.read_with(cx, |p, _| {
            assert_eq!(p.rows.len(), 2, "初始标题行集");
            assert_eq!(p.rows[0].label, "H1 · A");
            assert_eq!(p.rows[1].level, 2, "H2 层级");
        });

        // 存续行句柄身份（编辑前后同一把）。
        let a_before = panel.read_with(cx, |p, _| p.rows[0].focus_handle.clone());

        // 编辑：尾段行插入 # C（Editor 编辑链 → notify → observe 差量刷新）。
        editor.update(cx, |ed, cx| {
            // 公开编辑面（buffer 私有）：行 3 首插入 # C——insert 内
            // 部自 notify（真实编辑链同一通知源）。
            ed.insert_text_at(3, 0, "# C\n", cx);
        });
        cx.run_until_parked();

        panel.read_with(cx, |p, _| {
            assert_eq!(p.rows.len(), 3, "编辑后行集差量 +1（# C 入列）");
            assert_eq!(
                p.rows[0].focus_handle, a_before,
                "存续行（# A）句柄跨重建复用——焦点身份稳定"
            );
            assert_eq!(p.rows[2].label, "H1 · C");
        });
    }

    /// 导航面：首行聚焦 → SelectNext → 次行含焦点（Navigable 遍历语义）。
    #[gpui::test]
    fn select_next_moves_focus_to_next_row(cx: &mut TestAppContext) {
        let (_editor, panel, mut visual) = panel_window(cx, "# A\n正文\n## B\n尾段");

        let (first, second) = panel.read_with(&visual, |p, _| {
            (
                p.rows[0].focus_handle.clone(),
                p.rows[1].focus_handle.clone(),
            )
        });
        // 键盘导航起点：聚焦首行。
        visual.update(|window, cx| first.focus(window, cx));
        // ↑/↓ 的 action 直达（menu 键位族 = 真机同一入口）。
        visual.dispatch_action(menu::SelectNext);
        visual.update(|window, cx| {
            assert!(
                second.contains_focused(window, cx),
                "SelectNext → 次行含焦点（Navigable 遍历语义）"
            );
        });
    }

    /// 激活面：聚焦行 → Confirm → 编辑器光标跳行（jump_to_line 状态桥）。
    #[gpui::test]
    fn confirm_jumps_editor_cursor_to_row_line(cx: &mut TestAppContext) {
        let (editor, panel, mut visual) = panel_window(cx, "# A\n正文\n## B\n尾段");

        // 聚焦第二行（## B，行 2）。
        let second = panel.read_with(&visual, |p, _| p.rows[1].focus_handle.clone());
        visual.update(|window, cx| second.focus(window, cx));

        visual.dispatch_action(menu::Confirm);
        editor.read_with(&visual, |ed, _| {
            assert_eq!(
                ed.cursor_point(),
                Point::new(2, 0),
                "Confirm 激活聚焦行 → 光标跳行 2 首列（jump_to_line 状态桥）"
            );
        });
    }
}
