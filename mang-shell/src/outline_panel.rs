//! #571：大纲面板键盘导航（#385 批 5a 登记遗留件——批 7）。
//!
//! 批 5a（PR #482）交付说明：「键盘导航（Navigable）坦白未接——
//! panels 无状态渲染架构需先立焦点/选中状态面，挂后续批次」。本模块
//! 兑现该挂账：heading_tree 从 panels.rs 的无状态内联渲染改为
//! [`OutlinePanel`] 实体承载（循 FileTree/GitPanel「一面板一实体」
//! 先例），行焦点句柄按**条目序号**缓存复用，跳转目标行单独保留。
//! 同页多个PDF书签也有独立焦点与UI身份，键盘面经 [`ui::Navigable`] 接线：
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
//! 行身份按条目序号保留：编辑增加/删除前项时焦点留在原序号位置；
//! 不把跳转目标行当作条目身份，同目标的不同书签不会共享焦点。

use crate::button_ext::icon_button_36;
use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    App, Context, Entity, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, ScrollAnchor,
    ScrollHandle, Window,
};
use ui::prelude::*;
use ui::{
    Icon, IconName, IconSize, Label, LabelSize, List, ListHeader, ListItem, Navigable,
    NavigableEntry,
};

use crate::editor::Editor;

/// 激活钩子（#289-④ pdf 页滚动等 bin 面副作用注入位）。
/// 入参 = 激活行号；`jump_to_line` 已由面板本体先行执行。
pub type ActivateHook = Arc<dyn Fn(usize, &mut Window, &mut App)>;

/// Esc 宿主回调：焦点还给实际挂载的 PDF 页面 / XLSX 网格 / 编辑器 / 空态根。
pub type EscapeHook = Arc<dyn Fn(&mut Window, &mut App)>;

/// 钩子的借用形态（clippy type_complexity 收口）。
type ActivateHookRef<'a> = Option<&'a dyn Fn(usize, &mut Window, &mut App)>;

/// 大纲行（渲染数据 + 行焦点句柄）。句柄按条目序号缓存于
/// [`OutlinePanel::handles`]，line仅用于真实文档跳转。
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
/// 订阅编辑器通知：buffer 编辑 → 行集差量刷新（句柄按条目序号保留）。
/// #121 批③b：大纲面板 dock 键（布局序列化身份——发布后不可改）。
pub const OUTLINE_PANEL_KEY: &str = "outline";

pub struct OutlinePanel {
    /// #121 批③b：面板位置（拖拽换侧——Right 起，可换 Left）。
    position: mdc_dock::DockPosition,
    editor: Entity<Editor>,
    rows: Vec<OutlineRow>,
    /// 条目序号 → 焦点句柄缓存（不依赖可能重复的jump目标line）。
    handles: HashMap<usize, FocusHandle>,
    /// 大纲体滚动句柄（NavigableEntry 滚动锚点——键盘导航越视口滚动）。
    scroll: ScrollHandle,
    /// #121 批③b：面板焦点句柄（dock 激活协议落点——树行句柄之外
    /// 的面板根焦点面，FileTree tree_focus 同构）。
    panel_focus: FocusHandle,
    /// bin 面副作用注入（见模块文档「lib 树纪律」）。
    activate_hook: Option<ActivateHook>,
    /// 独立宿主可省略；Shell 注入当前活动视图的焦点路由。
    escape_hook: Option<EscapeHook>,
    /// #108 D1：过滤查询（大小写不敏感子串——纯前端行集过滤，
    /// 不动数据面；报告 §3 P1 口径）。空 = 全行。
    filter: String,
    /// #108 D1：过滤框焦点（面板独立实体——不借 Editor 表单域，
    /// 自持输入焦点与键盘消费）。
    filter_focus: FocusHandle,
    /// #108 批②：分区折叠集（行索引 = 数据面身份——不随渲染漂移；
    /// 报告 §3 提示「行焦点按序号协议与折叠的行序号漂移交互需设计
    /// 先行」：折叠集按全量 headings 的行索引记，句柄缓存不清退
    /// （retain 按全量 len——解折叠即恢复，与编辑差量刷新同语义）。
    collapsed: std::collections::HashSet<usize>,
}

impl OutlinePanel {
    /// 构造（订阅编辑器 + 首轮行集刷新）。
    pub fn new(
        editor: Entity<Editor>,
        activate_hook: Option<ActivateHook>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            position: mdc_dock::DockPosition::Right,
            panel_focus: cx.focus_handle(),
            editor,
            rows: Vec::new(),
            handles: HashMap::new(),
            scroll: ScrollHandle::new(),
            activate_hook,
            escape_hook: None,
            filter: String::new(),
            filter_focus: cx.focus_handle(),
            collapsed: crate::settings::load_outline_collapsed()
                .into_iter()
                .collect(),
        };
        this.refresh(cx);
        // 编辑器通知（buffer 编辑/标题增删）→ 行集差量刷新。
        // 派生实体自行通知，不能依赖编辑器视图的旁路重绘。
        cx.observe(&this.editor, |this, _, cx| {
            this.refresh(cx);
            cx.notify();
        })
        .detach();
        this
    }

    /// 注入 Esc 焦点路由，不改变构造签名；未注入时沿用独立编辑器行为。
    pub fn with_escape_hook(mut self, hook: EscapeHook) -> Self {
        self.escape_hook = Some(hook);
        self
    }

    /// 宿主编辑器（shell 侧实体换绑判据）。
    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    /// 行标签只读序（#47 A1：shell 级标签切换测试的断言面——右栏
    /// 内容随当前视图的运行时证据；同 `editor()` 先例：面板只读面）。
    pub fn row_labels(&self) -> Vec<&str> {
        self.rows.iter().map(|r| r.label.as_str()).collect()
    }

    /// 行集是否为空（left_panel 沿用「无标题不渲染分组」的批 5a 前语义）。
    /// #108 P2③：过滤框聚焦句柄（goto 符号链——开右栏后聚焦此处，
    /// 用户输入即符号过滤、Enter 跳行——「符号对话框」承接面）。
    pub fn filter_focus_handle(&self) -> &FocusHandle {
        &self.filter_focus
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// #108 D1：设置过滤查询（实时——每键入即时生效；空恢复全行）。
    pub fn set_filter(&mut self, query: String) {
        self.filter = query;
    }

    /// #108 D1：过滤框键盘编辑（最小文本语义——单字符追加 / 退格 /
    /// Esc 清空；IME 长文本组合走 GPUI 平台输入通道时以 marked
    /// 段提交形态扩展，此处起步语义为 ASCII/直接输入面）。字符
    /// 追加含 CJK（`key` 为单字符判定——`to_lowercase` 过滤域不变）。
    fn dispatch_filter_key(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ks = &event.keystroke;
        let key = ks.key.to_lowercase();
        // 修饰键形态不进过滤编辑（Ctrl/Alt/Shift 组合留给动作面）。
        let plain = !ks.modifiers.control && !ks.modifiers.alt && !ks.modifiers.platform;
        if plain && key == "escape" {
            self.filter.clear();
            cx.notify();
            return;
        }
        if plain && key == "backspace" {
            self.filter.pop();
            cx.notify();
            return;
        }
        if plain && key == "enter" {
            // Enter 落在过滤框：无编辑语义（行激活 Enter 在行级
            // menu::Confirm——过滤框消费防误触行跳）。
            cx.stop_propagation();
            return;
        }
        if plain && key.chars().count() == 1 && !ks.modifiers.shift {
            // 单字符键（GPUI key 面：字母/数字/符号单字符形态）。
            let ch = key.chars().next().unwrap();
            self.filter.push(ch);
            cx.notify();
            return;
        }
        if plain && key.chars().count() == 1 && ks.modifiers.shift {
            // Shift+字母：GPUI key 已小写化——shift 语义查英文大写
            //（中文输入法下 key 为原始形态，Shift 组合不追加——
            // 简化面：仅 ASCII 大写近似，非英文布局可能不完全准确，
            // 真机 IME 走平台输入通道不进此分支）。
            let ch = key.chars().next().unwrap();
            if ch.is_ascii_alphabetic() {
                self.filter.push(ch.to_ascii_uppercase());
            }
            cx.notify();
            return;
        }
    }

    /// #108 D1：过滤后可见行标签（测试断言面——渲染同源 visible_rows）。
    /// #108 D1：过滤后行集（无过滤 = 全行；大小写不敏感子串——
    /// `to_lowercase` 两侧同域比较，CJK 无大小写变换恒等）。
    fn visible_rows(&self) -> Vec<&OutlineRow> {
        if self.filter.is_empty() {
            return self.rows.iter().collect();
        }
        let needle = self.filter.to_lowercase();
        self.rows
            .iter()
            .filter(|r| r.label.to_lowercase().contains(&needle))
            .collect()
    }

    /// #108 D1：过滤空集提示文案（有标题但无命中——可见语义非白屏；
    /// 无过滤/有命中 = None 不渲染）。
    fn filter_empty_hint(&self) -> &'static str {
        if !self.filter.is_empty() && self.visible_rows().is_empty() {
            "无匹配标题"
        } else {
            ""
        }
    }

    /// #108 批②：折叠/展开分区头（toggle——渲染集即时重算）。
    /// #108 批③：折叠集变更同步落盘（跨重启恢复——settings
    /// `outline_collapsed` 键沿 indent_unit 先例）。
    pub fn toggle_fold(&mut self, index: usize) {
        if index < self.rows.len() {
            if self.collapsed.contains(&index) {
                self.collapsed.remove(&index);
            } else {
                self.collapsed.insert(index);
            }
            crate::settings::save_outline_collapsed(self.collapsed.iter().copied().collect());
        }
    }

    /// #108 P2 批④：全部折叠（全部可折叠头入折叠集——候选判定与
    /// 渲染 foldable 同源：行 i 有更深子级即头；瞬时动作但落盘沿
    /// 批③语义——折叠集是持久态）。
    pub fn fold_all(&mut self) {
        for i in 0..self.rows.len() {
            if self
                .rows
                .get(i + 1)
                .is_some_and(|next| next.level > self.rows[i].level)
            {
                self.collapsed.insert(i);
            }
        }
        crate::settings::save_outline_collapsed(self.collapsed.iter().copied().collect());
    }

    /// #108 P2 批④：全部展开（折叠集清空——可见行集回全集）。
    pub fn unfold_all(&mut self) {
        self.collapsed.clear();
        crate::settings::save_outline_collapsed(self.collapsed.iter().copied().collect());
    }

    /// #108 批②：可见行（分区折叠后——折叠头保留，其子行到下一
    /// 同级或更高级标题前隐藏；行索引按全量数据面身份——渲染
    /// 序号协议稳定）。
    pub fn visible_rows_indexed(&self) -> Vec<(usize, &OutlineRow)> {
        let mut out = Vec::new();
        let mut hiding_until_level: Option<usize> = None;
        // #108 D1：过滤门前置于折叠视图（无过滤 = 全行；过滤与折叠
        // 两层正交——被折叠隐藏的行不进过滤比较，命中行仍受折叠门）。
        let needle = if self.filter.is_empty() {
            None
        } else {
            Some(self.filter.to_lowercase())
        };
        for (index, row) in self.rows.iter().enumerate() {
            if let Some(n) = &needle {
                if !row.label.to_lowercase().contains(n.as_str()) {
                    continue;
                }
            }
            if let Some(min_level) = hiding_until_level {
                if row.level <= min_level {
                    // 到同级/更高级标题——恢复可见。
                    hiding_until_level = None;
                } else {
                    continue; // 深层子行隐藏
                }
            }
            out.push((index, row));
            if self.collapsed.contains(&index) {
                // 折叠头：其后的更深层行隐藏（到 level ≤ 头级止）。
                hiding_until_level = Some(row.level);
            }
        }
        out
    }

    /// #108 批②：可见行标签（测试断言面——渲染同源 visible_rows_indexed）。
    /// #108 P2①：光标跟随高亮——光标所属 heading 的全量行号。
    /// 语义（Q-IDE outline.rs 参照）：光标行 >= 某 heading 行的
    /// 「最后一个 heading」= 当前节；光标在首个 heading 前 → None。
    pub fn current_row_for_cursor(&self, cx: &App) -> Option<usize> {
        let cursor_line = self.editor.read(cx).cursor_point().line;
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.line <= cursor_line)
            .map(|(i, _)| i)
            .next_back()
    }

    pub fn visible_labels(&self) -> Vec<&str> {
        self.visible_rows_indexed()
            .iter()
            .map(|(_, r)| r.label.as_str())
            .collect()
    }

    /// 行集差量刷新：重建行数据，句柄按条目序号保留、清退删除项、
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
        self.handles.retain(|index, _| *index < headings.len());
        self.rows = headings
            .into_iter()
            .enumerate()
            .map(|(index, (label, level, line))| {
                let focus_handle = self
                    .handles
                    .entry(index)
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

impl gpui::EventEmitter<mdc_dock::PanelEvent> for OutlinePanel {}

impl gpui::Focusable for OutlinePanel {
    fn focus_handle(&self, _cx: &gpui::App) -> gpui::FocusHandle {
        self.panel_focus.clone()
    }
}

impl mdc_dock::Panel for OutlinePanel {
    fn persistent_name() -> &'static str {
        OUTLINE_PANEL_KEY
    }

    fn panel_key() -> &'static str {
        OUTLINE_PANEL_KEY
    }

    fn position(&self) -> mdc_dock::DockPosition {
        self.position
    }

    fn position_is_valid(&self, position: mdc_dock::DockPosition) -> bool {
        // #121 批③b：大纲面板可停靠右/左。
        matches!(
            position,
            mdc_dock::DockPosition::Left | mdc_dock::DockPosition::Right
        )
    }

    fn set_position(&mut self, position: mdc_dock::DockPosition, _cx: &mut Context<Self>) {
        self.position = position;
    }
}

impl gpui::Render for OutlinePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let editor = self.editor.clone();
        let hook = self.activate_hook.clone();
        // #108 P2①：光标跟随行（渲染时读编辑器光标——独立实体读合法；
        // 光标动 → 编辑器 notify 重绘，本面板随 shell 全树 notify 链重渲染）。
        let current_row = self.current_row_for_cursor(cx);
        // #108 批②：渲染行集走折叠视图（visible_rows_indexed——折叠头
        // 保留、子行隐藏；行身份 = 全量索引（句柄协议不变——解折叠
        // 即恢复），折叠箭头 toggle 按全量索引落点）。
        let rows: Vec<(usize, OutlineRow)> = self
            .visible_rows_indexed()
            .into_iter()
            .map(|(i, r)| (i, r.clone()))
            .collect();
        // 折叠候选：紧邻全量行有更深子级（箭头只渲染在可折叠头上）。
        let rows_snapshot = self.rows.clone();
        let foldable = move |i: usize| {
            rows_snapshot
                .get(i + 1)
                .is_some_and(|next| next.level > rows_snapshot[i].level)
        };
        let collapsed = self.collapsed.clone();
        // #108 D1：过滤面渲染变量（与折叠视图共存——过滤是前端
        // 行集门，折叠是层级门；两层各自独立生效）。
        let filter = self.filter.clone();
        let filter_focus = self.filter_focus.clone();
        let filter_hint = self.filter_empty_hint().to_string();
        let scroll = self.scroll.clone();
        let mut entries = Vec::with_capacity(rows.len());

        // 行体（批 5a 组件面原样 + 焦点挂载）：外层 stateful div 持
        // track_focus（ListItem 无焦点 API——ui 组件面单证），点击聚焦；
        // ListItem 承载视觉与跳行（批 5a 点击链不动）。
        // #108 D1：过滤框（面板顶条——List 外独立装配；List.header 只
        // 接 ListHeader 不可嵌任意元素）。键盘编辑自持：on_key_down 进
        // dispatch_filter_key（字符追加/退格/Esc 清空）。
        let filter_bar = h_flex()
            .id("outline-filter")
            .flex_none()
            .px_2()
            .py_1()
            .gap_1()
            .items_center()
            .child(
                Icon::new(IconName::Filter)
                    .size(IconSize::XSmall)
                    .color(ui::Color::Muted),
            )
            .child(
                div()
                    .id("outline-filter-input")
                    .flex_1()
                    .min_w_0()
                    .h(px(22.))
                    .overflow_hidden()
                    .track_focus(&filter_focus)
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        this.dispatch_filter_key(event, window, cx);
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    .child(
                        Label::new(if filter.is_empty() {
                            "过滤…"
                        } else {
                            filter.as_str()
                        })
                        .size(LabelSize::XSmall)
                        .color(if filter.is_empty() {
                            ui::Color::Muted
                        } else {
                            ui::Color::Default
                        }),
                    ),
            );

        // #108 D1：空集提示（有标题无命中——可见非白屏；根列承载）。
        let empty_hint = (!filter_hint.is_empty()).then(|| {
            h_flex()
                .id("outline-filter-empty")
                .flex_none()
                .px_2()
                .pb_1()
                .child(
                    Label::new(filter_hint.as_str())
                        .size(LabelSize::XSmall)
                        .color(ui::Color::Muted),
                )
        });

        let body = div()
            .id("outline-body")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .child(
                List::new()
                    .header(
                        ListHeader::new("文档结构")
                            .toggle(true)
                            .start_slot(
                                Icon::new(IconName::ListTree)
                                    .size(IconSize::XSmall)
                                    .color(ui::Color::Muted),
                            )
                            // #108 P2 批④：全部展开/全部折叠（面板头
                            // 工具区——超越参照项，我方定义语义；作用折叠
                            // 集整体——与批②折叠视图同数据源）。
                            .end_slot(
                                h_flex()
                                    .gap_1()
                                    .child(
                                        icon_button_36("outline-unfold-all", IconName::Plus)
                                            .tooltip_text("全部展开")
                                            .on_click({
                                                let weak = cx.entity().downgrade();
                                                move |_, _, cx| {
                                                    if let Some(panel) = weak.upgrade() {
                                                        panel.update(cx, |panel, _| {
                                                            panel.unfold_all();
                                                        });
                                                    }
                                                }
                                            })
                                            .build(window, cx),
                                    )
                                    .child(
                                        icon_button_36("outline-fold-all", IconName::SquareMinus)
                                            .tooltip_text("全部折叠")
                                            .on_click({
                                                let weak = cx.entity().downgrade();
                                                move |_, _, cx| {
                                                    if let Some(panel) = weak.upgrade() {
                                                        panel.update(cx, |panel, _| {
                                                            panel.fold_all();
                                                        });
                                                    }
                                                }
                                            })
                                            .build(window, cx),
                                    ),
                            ),
                    )
                    .children(rows.iter().map(|(gindex, row)| {
                        let handle = row.focus_handle.clone();
                        let editor = editor.clone();
                        let hook = hook.clone();
                        let line = row.line;
                        let focused = row.focus_handle.contains_focused(window, cx);
                        let anchor = ScrollAnchor::for_handle(scroll.clone());
                        entries.push(NavigableEntry {
                            focus_handle: handle.clone(),
                            scroll_anchor: Some(anchor.clone()),
                        });
                        let is_current = Some(*gindex) == current_row;
                        div()
                            .id(SharedString::from(format!("outline-row-{gindex}")))
                            .debug_selector(move || {
                                format!("outline-row-{gindex}{}", if is_current { "-cur" } else { "" })
                            })
                            // #108 P2①：光标跟随高亮（selected 视觉——
                            // 光标所属节行 bg；与键盘焦点 focused 并列呈现）。
                            .when(is_current, |el| {
                                el.bg(cx.theme().colors().element_selected)
                            })
                            .track_focus(&handle)
                            .anchor_scroll(Some(anchor))
                            // 点击 = 聚焦该行（键盘导航起点）。
                            .on_click({
                                let handle = handle.clone();
                                move |_, window, cx| handle.focus(window, cx)
                            })
                            .child(
                                ListItem::new(SharedString::from(format!("outline-item-{gindex}")))
                                    .inset(true)
                                    // 层级缩进：每级 12px（1 级不缩进——批 5a 同款）。
                                    .indent_level(row.level.saturating_sub(1))
                                    .focused(focused)
                                    // #108 批②：折叠箭头（可折叠头才有——点击
                                    // toggle 折叠/展开；stop_propagation 防行跳）。
                                    .when(foldable(*gindex), |li| {
                                        let g = *gindex;
                                        let is_collapsed = collapsed.contains(&g);
                                        li.start_slot(
                                            div()
                                                .id(SharedString::from(format!("outline-fold-{g}")))
                                                .cursor_pointer()
                                                .on_click(cx.listener(
                                                    move |this, _: &gpui::ClickEvent, _, cx| {
                                                        this.toggle_fold(g);
                                                        cx.stop_propagation();
                                                        cx.notify();
                                                    },
                                                ))
                                                .child(
                                                    Icon::new(if is_collapsed {
                                                        IconName::ChevronRight
                                                    } else {
                                                        IconName::ChevronDown
                                                    })
                                                    .size(IconSize::XSmall)
                                                    .color(ui::Color::Muted),
                                                ),
                                        )
                                    })
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
        for entry in entries {
            navigable = navigable.entry(entry);
        }

        div()
            .id("outline-panel")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            // ↑↓/Enter/Esc 走 main.rs 既有 menu 键位族（批 4 注册面——
            // 键上下文匹配聚焦元素祖先链，本根即 "menu"）。
            .key_context("menu")
            // Enter（menu::Confirm）：激活聚焦行。
            .on_action({
                let rows = rows.clone();
                let editor = editor.clone();
                let hook = hook.clone();
                move |_: &menu::Confirm, window, cx| {
                    if let Some((_, row)) = rows
                        .iter()
                        .find(|(_, r)| r.focus_handle.contains_focused(window, cx))
                    {
                        Self::activate(&editor, row.line, hook.as_deref(), window, cx);
                    }
                }
            })
            // Esc（menu::Cancel）：焦点还给编辑区（面板键盘域退出；
            // 不消费时该 action 会冒泡进窗口级关闭链——显式收口）。
            .on_action({
                let editor = editor.clone();
                let escape = self.escape_hook.clone();
                move |_: &menu::Cancel, window, cx| {
                    if let Some(escape) = &escape {
                        escape(window, cx);
                    } else {
                        let handle = editor.read(cx).focus_handle().clone();
                        handle.focus(window, cx);
                    }
                }
            })
            // 根列向承载：过滤条 + 空集提示 +（滚动 body 的）navigable
            //（body 形态零变化——List 虚拟化有界容器保持，长文档滚动
            // 测试语义不变）。
            .child(filter_bar)
            .when_some(empty_hint, |el, hint| el.child(hint))
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
            v_flex().size_full().child(self.0.clone())
        }
    }

    /// 面板 + 宿主窗口（开窗 → draw 一帧 → VisualTestContext；
    /// 主题初始化序列镜像 main.rs——ButtonLike/ListItem 渲染需
    /// GlobalThemeSettingsProvider）。
    /// #108 P2 批④：全部折叠/全部展开——折叠候选全集入集/清空
    /// （候选 = 有更深子级的头行——与渲染 foldable 同源判定）。
    #[gpui::test]
    fn fold_all_and_unfold_all_toggle_full_set(cx: &mut TestAppContext) {
        let tmp = std::env::temp_dir().join(format!("m108-foldall-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let prev = std::env::var_os("XDG_CONFIG_HOME");
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &tmp) };
        let (_editor, panel, mut visual) =
            panel_window(cx, "# A\nbody\n## A1\ntext\n# B\nmore\n## B1\ntail");
        // 两 H1 头（行 0 与行 4）+ 一 H2 头（行 2 与行 6——B1）。
        // 全部折叠：四个可折叠头全入集（H1×2 + H2×2）。
        visual.update(|_window, cx| {
            panel.update(cx, |panel, _| {
                panel.fold_all();
                // 大纲行集 = 标题行（A/A1/B/B1 四头）；可折叠候选 =
                // 有更深子级的头：A 与 B（H1——A1/B1 是叶子 H2）。
                assert_eq!(
                    panel.collapsed.len(),
                    2,
                    "两个 H1 头全折叠（A1/B1 是叶子——实得 {}）",
                    panel.collapsed.len()
                );
                // 全部折叠后可见行 = 全部标题行（H1 折叠只隐 H2——
                // A1/B1 藏在 A/B 折叠区内）。
                assert_eq!(
                    panel.visible_rows_indexed().len(),
                    2,
                    "折叠后仅两 H1 头可见"
                );
            });
        });
        // 全部展开：清空回全集。
        visual.update(|_window, cx| {
            panel.update(cx, |panel, _| {
                panel.unfold_all();
                assert!(panel.collapsed.is_empty(), "折叠集清空");
                assert_eq!(
                    panel.visible_rows_indexed().len(),
                    panel.rows.len(),
                    "展开后全集可见"
                );
            });
        });
        match prev {
            Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },
            None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

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
        use std::sync::atomic::{AtomicUsize, Ordering};

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
        let notices = Arc::new(AtomicUsize::new(0));
        let observed = notices.clone();
        let _observer = cx.update(|app| {
            app.observe(&panel, move |_, _| {
                observed.fetch_add(1, Ordering::SeqCst);
            })
        });

        // 编辑：尾段行插入 # C（Editor 编辑链 → notify → observe 差量刷新）。
        editor.update(cx, |ed, cx| {
            // 公开编辑面（buffer 私有）：行 3 首插入 # C——insert 内
            // 部自 notify（真实编辑链同一通知源）。
            ed.insert_text_at(3, 0, "# C\n", cx);
        });
        cx.run_until_parked();
        assert!(
            notices.load(Ordering::SeqCst) > 0,
            "派生面板须通知自己的视图订阅者"
        );

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
    /// #108 D1：过滤框——实时忽略大小写子串匹配（报告 §3 P1 原文
    /// 口径），纯前端过滤行集不动数据面。先红：方法不存在 = 编译红。
    #[gpui::test]
    fn filter_narrows_rows_case_insensitive(cx: &mut TestAppContext) {
        let (_editor, panel, _visual) = panel_window(cx, "# Alpha\n## Beta\n# Gamma\n### Delta");
        panel.update(cx, |p, _| {
            // 初始全行（4 标题）。
            assert_eq!(p.visible_labels().len(), 4, "无过滤全行");

            // 键入 "beta"：小写查询命中大写 Beta 行——大小写不敏感。
            p.set_filter("beta".into());
            assert_eq!(p.visible_labels().len(), 1, "过滤后 1 行");
            assert!(
                p.visible_labels()[0].contains("Beta"),
                "命中的是 Beta 行（大小写不敏感）"
            );

            // 清空：恢复全行。
            p.set_filter(String::new());
            assert_eq!(p.visible_labels().len(), 4, "清空恢复全行");
        });
    }

    /// #108 D1：过滤空集提示（有标题文档过滤无命中 =「无匹配」可见
    /// 语义——不是白屏）。
    #[gpui::test]
    fn filter_no_match_is_visible_not_blank(cx: &mut TestAppContext) {
        let (_editor, panel, _visual) = panel_window(cx, "# Alpha\n## Beta");
        panel.update(cx, |p, _| {
            p.set_filter("zzz".into());
            assert_eq!(p.visible_labels().len(), 0, "无命中空集");
            assert_eq!(p.filter_empty_hint(), "无匹配标题", "空集提示文案");
        });
    }

    /// #108 批②：分区折叠——H1 分区折叠后子行不出渲染集、折叠头
    /// 保留；展开恢复全行。先红：方法不存在 = 编译红。
    /// #108 批③：XDG 临时区 guard（dock_open_states_persist_across_restart
    /// 同型——toggle_fold 落盘写临时区、构造装载读临时区默认空，
    /// 不污染真实 config——批③落盘面接入后的测试隔离窗）。
    #[gpui::test]
    fn section_fold_hides_children_keeps_head(cx: &mut TestAppContext) {
        let xdg = std::env::temp_dir().join(format!("moutline-fold-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&xdg);
        let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &xdg) };
        let text = "# A\n## A1\n## A2\n# B\n## B1";
        let (_editor, panel, _visual) = panel_window(cx, text);
        panel.update(cx, |p, _| {
            // 初始 5 行（A/A1/A2/B/B1）。
            assert_eq!(p.visible_labels().len(), 5, "初始全行");

            // 折叠 H1 A（行 0）：A1/A2 隐藏，B/B1 保留。
            p.toggle_fold(0);
            assert_eq!(p.visible_labels().len(), 3, "折叠 A 后 3 行");
            assert!(p.visible_labels()[0].contains("H1 · A"), "折叠头 A 保留");
            assert!(
                !p.visible_labels().iter().any(|l| l.contains("A1")),
                "A1 子行隐藏"
            );

            // 展开：恢复 5 行（句柄不清退——解折叠即恢复）。
            p.toggle_fold(0);
            assert_eq!(p.visible_labels().len(), 5, "展开恢复全行");
        });
        // XDG 还原（先例同型收口）。
        match prev_xdg {
            Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },
            None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
        }
        let _ = std::fs::remove_dir_all(&xdg);
    }

    /// #108 批②：折叠 H1 下嵌套 H2 子分区独立折叠（内层折叠不动
    /// 外层；两级语义正交）。#108 批③：XDG 临时区 guard 同型。
    #[gpui::test]
    fn section_fold_nested_h2_independent(cx: &mut TestAppContext) {
        let xdg = std::env::temp_dir().join(format!("moutline-nested-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&xdg);
        let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &xdg) };
        let text = "# A\n## A1\n### A1a\n## A2";
        let (_editor, panel, _visual) = panel_window(cx, text);
        panel.update(cx, |p, _| {
            // 折叠 H2 A1（行 1）：A1a 隐藏，A/A2 保留。
            p.toggle_fold(1);
            assert_eq!(p.visible_labels().len(), 3, "折叠 A1 后 3 行");
            assert!(
                !p.visible_labels().iter().any(|l| l.contains("A1a")),
                "A1a 深层子行隐藏"
            );
            assert!(
                p.visible_labels().iter().any(|l| l.contains("A2")),
                "同级 A2 不受影响"
            );
        });
        // XDG 还原（先例同型收口）。
        match prev_xdg {
            Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },
            None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
        }
        let _ = std::fs::remove_dir_all(&xdg);
    }

    #[gpui::test]
    fn cursor_follow_highlights_current_section_row(cx: &mut TestAppContext) {
        // #108 P2①：光标跟随高亮——光标落在「## B」段内（行 3 尾段）
        // → 大纲第二行（行 2 的 heading）呈 selected（outline-row-1-cur 锚）。
        let (editor, panel, mut visual) = panel_window(cx, "# A
正文
## B
尾段");

        // 前置：光标先在第 0 行（首 heading 行——current_row=0 呈 selected）。
        let cur = editor.read_with(&visual, |ed, _| ed.cursor_point().line);
        assert_eq!(cur, 0, "前置：光标在第 0 行");
        // 光标移到「尾段」（行 3——## B 之后的段内）。
        editor.update(&mut visual, |ed, cx| {
            ed.jump_to_line(3);
        });
        visual.update(|window, app| window.draw(app).clear(app));
        // 渲染面：第二行（gindex=1）= selected 锚在场；首行无 -cur。
        let second_cur = visual.debug_bounds("outline-row-1-cur");
        let first_cur = visual.debug_bounds("outline-row-0-cur");
        assert!(second_cur.is_some(), "光标在 ## B 段内 → 第二行 selected");
        assert!(first_cur.is_none(), "首行不在光标所属段");

        // 数据面 API 直证：current_row_for_cursor = Some(1)。
        panel.read_with(&visual, |p, app| {
            assert_eq!(p.current_row_for_cursor(app), Some(1), "所属节 = 第二 heading");
        });
    }

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

    /// 真实PDF转换链允许同页多个书签：[0,0,6]，不可按jump line共享焦点。
    #[gpui::test]
    fn pdf_same_page_bookmarks_have_distinct_focus_and_real_targets(cx: &mut TestAppContext) {
        use crate::editor::pdf_doc::{PdfOutlineEntry, PdfRaw, PdfTextPage, convert};
        use std::sync::Mutex;

        cx.update(|app| {
            theme::init(theme::LoadThemes::JustBase, app);
            theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
        });
        let payload = convert(&PdfRaw {
            pages: vec![
                PdfTextPage {
                    index: 0,
                    text: "Page one\nFirst content\n\nLast content".into(),
                },
                PdfTextPage {
                    index: 1,
                    text: "Page two\nSecond content".into(),
                },
            ],
            outline: vec![
                PdfOutlineEntry {
                    level: 1,
                    title: "First chapter".into(),
                    page: 0,
                },
                PdfOutlineEntry {
                    level: 2,
                    title: "Same page child".into(),
                    page: 0,
                },
                PdfOutlineEntry {
                    level: 1,
                    title: "Second chapter".into(),
                    page: 1,
                },
            ],
            ..Default::default()
        });
        assert_eq!(
            payload.headings.iter().map(|h| h.line).collect::<Vec<_>>(),
            vec![0, 0, 6]
        );
        let editor =
            cx.update(|cx| cx.new(|cx| Editor::new_pdf("shared-page.pdf".into(), payload, cx)));
        let activated_lines = Arc::new(Mutex::new(Vec::new()));
        let recorded = activated_lines.clone();
        let hook: ActivateHook = Arc::new(move |line, _, _| {
            recorded.lock().unwrap().push(line);
        });
        let panel = cx.update(|cx| {
            let editor = editor.clone();
            cx.new(|cx| OutlinePanel::new(editor, Some(hook), cx))
        });
        let (first, second, third) = panel.read_with(cx, |p, _| {
            assert_eq!(
                p.rows.iter().map(|r| r.line).collect::<Vec<_>>(),
                vec![0, 0, 6]
            );
            assert_ne!(
                p.rows[0].focus_handle, p.rows[1].focus_handle,
                "同页书签仍是独立行"
            );
            (
                p.rows[0].focus_handle.clone(),
                p.rows[1].focus_handle.clone(),
                p.rows[2].focus_handle.clone(),
            )
        });
        let window = cx.open_window(size(gpui::px(260.), gpui::px(400.)), {
            let panel = panel.clone();
            move |_, _| Host(panel)
        });
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.update(|window, cx| first.focus(window, cx));
        visual.dispatch_action(menu::SelectNext);
        visual.update(|window, cx| {
            assert!(second.contains_focused(window, cx), "第一下↓到同行第二书签");
            assert!(!first.contains_focused(window, cx));
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        visual.dispatch_action(menu::SelectNext);
        visual.update(|window, cx| {
            assert!(
                third.contains_focused(window, cx),
                "第二下↓不能卡在同页书签"
            );
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        visual.dispatch_action(menu::Confirm);
        editor.read_with(&visual, |ed, _| {
            assert_eq!(
                ed.cursor_point(),
                Point::new(6, 0),
                "激活按真实line，不用条目index"
            );
            assert_eq!(ed.pdf_page_for_line(ed.cursor_point().line), Some(2));
        });
        visual.dispatch_action(menu::SelectPrevious);
        visual.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        let bounds = visual
            .debug_bounds("outline-row-1")
            .expect("第二书签实际渲染行");
        visual.simulate_click(bounds.center(), gpui::Modifiers::default());
        visual.update(|window, cx| assert!(second.contains_focused(window, cx)));
        editor.read_with(&visual, |ed, _| {
            assert_eq!(
                ed.cursor_point(),
                Point::new(0, 0),
                "第二书签点击仍跳首页line0"
            );
            assert_eq!(ed.pdf_page_for_line(ed.cursor_point().line), Some(1));
        });
        visual.dispatch_action(menu::Confirm);
        assert_eq!(
            *activated_lines.lock().unwrap(),
            vec![6, 0, 0],
            "Enter与点击保留真实跳转line钩子"
        );
    }

    /// 真实布局/下一帧滚动回归：焦点变化还必须让目标行进入有界viewport。
    #[gpui::test]
    fn long_outline_navigation_scrolls_target_into_view(cx: &mut TestAppContext) {
        let text = (0..64)
            .map(|i| format!("# Heading {i}\nBody {i}\n"))
            .collect::<String>();
        let (_editor, panel, mut visual) = panel_window(cx, &text);
        let first = panel.read_with(&visual, |p, _| p.rows[0].focus_handle.clone());
        let target = panel.read_with(&visual, |p, _| p.rows[35].focus_handle.clone());
        let viewport = panel.read_with(&visual, |p, _| p.scroll.bounds());
        if let Some(before) = visual.debug_bounds("outline-row-35") {
            assert!(
                !viewport.contains(&before.origin),
                "前置：第36条超过当前viewport"
            );
        }
        visual.update(|window, cx| first.focus(window, cx));
        for _ in 0..35 {
            visual.dispatch_action(menu::SelectNext);
            visual.update(|window, cx| {
                window.simulate_next_frame(cx);
                window.draw(cx).clear(cx);
            });
        }
        visual.update(|window, cx| assert!(target.contains_focused(window, cx)));
        let viewport = panel.read_with(&visual, |p, _| p.scroll.bounds());
        let offset = panel.read_with(&visual, |p, _| p.scroll.offset());
        let after = visual
            .debug_bounds("outline-row-35")
            .expect("滚后目标行真实布局");
        assert!(
            offset.y < gpui::px(0.),
            "实际滚动offset必须改变，不能只有焦点变化"
        );
        assert!(
            viewport.contains(&after.origin),
            "目标行起点须滚入viewport：{after:?} / {viewport:?}"
        );
        assert!(after.bottom() <= viewport.bottom(), "目标行完整高度可见");
    }

    /// Hook 模式只挂载替代视图目标，隐藏模型 Editor；默认模式挂真实 Editor。
    struct EscapeHost {
        panel: Entity<OutlinePanel>,
        editor: Option<Entity<Editor>>,
        host_target: Option<FocusHandle>,
    }

    impl gpui::Render for EscapeHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            v_flex()
                .size_full()
                .child(self.panel.clone())
                .when_some(self.editor.clone(), |el, editor| {
                    el.child(v_flex().w_full().h(gpui::px(100.)).child(editor))
                })
                .when_some(self.host_target.clone(), |el, target| {
                    el.child(
                        div()
                            .id("outline-host-focus-target")
                            .debug_selector(|| "outline-host-focus-target".to_string())
                            .w_full()
                            .h(gpui::px(1.))
                            .flex_shrink_0()
                            .track_focus(&target),
                    )
                })
        }
    }

    fn escape_window(
        cx: &mut TestAppContext,
        inject_host_hook: bool,
    ) -> (
        Entity<Editor>,
        Entity<OutlinePanel>,
        VisualTestContext,
        FocusHandle,
    ) {
        cx.update(|app| {
            theme::init(theme::LoadThemes::JustBase, app);
            theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
        });
        let editor = md_editor(cx, "# Heading\nBody");
        let target = cx.update(|app| app.focus_handle());
        let panel = cx.update(|app| {
            let editor = editor.clone();
            let target = target.clone();
            app.new(|cx| {
                let panel = OutlinePanel::new(editor, None, cx);
                if inject_host_hook {
                    panel.with_escape_hook(Arc::new(move |window, cx| target.focus(window, cx)))
                } else {
                    panel
                }
            })
        });
        let window = cx.open_window(size(gpui::px(260.), gpui::px(400.)), {
            let panel = panel.clone();
            let editor = (!inject_host_hook).then_some(editor.clone());
            let host_target = inject_host_hook.then_some(target.clone());
            move |_, _| EscapeHost {
                panel,
                editor,
                host_target,
            }
        });
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        (editor, panel, visual, target)
    }

    #[gpui::test]
    fn cancel_with_host_hook_focuses_mounted_target_instead_of_hidden_editor(
        cx: &mut TestAppContext,
    ) {
        let (editor, panel, mut visual, target) = escape_window(cx, true);
        assert!(
            visual.debug_bounds("outline-host-focus-target").is_some(),
            "宿主焦点目标必须真实渲染挂载"
        );
        let row = panel.read_with(&visual, |p, _| p.rows[0].focus_handle.clone());
        let model_focus = editor.read_with(&visual, |ed, _| ed.focus_handle().clone());
        visual.update(|window, cx| row.focus(window, cx));
        visual.dispatch_action(menu::Cancel);
        visual.update(|window, cx| {
            assert!(
                target.contains_focused(window, cx),
                "Cancel 走宿主的实际视图目标"
            );
            assert!(
                !model_focus.contains_focused(window, cx),
                "不能落到未挂载模型 Editor"
            );
            assert!(
                !row.contains_focused(window, cx),
                "Cancel 必须离开大纲行焦点"
            );
        });
    }

    #[gpui::test]
    fn cancel_without_host_hook_preserves_editor_focus_fallback(cx: &mut TestAppContext) {
        let (editor, panel, mut visual, _) = escape_window(cx, false);
        panel.read_with(&visual, |p, _| assert!(p.escape_hook.is_none()));
        let row = panel.read_with(&visual, |p, _| p.rows[0].focus_handle.clone());
        let editor_focus = editor.read_with(&visual, |ed, _| ed.focus_handle().clone());
        visual.update(|window, cx| row.focus(window, cx));
        visual.dispatch_action(menu::Cancel);
        visual.update(|window, cx| {
            assert!(
                editor_focus.contains_focused(window, cx),
                "未注入hook时仍回独立Editor"
            );
            assert!(!row.contains_focused(window, cx));
        });
    }
}
