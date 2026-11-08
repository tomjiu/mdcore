//! 外壳根视图 —— #63 Lead 布局的组合点 + #68 阶段 1 编辑区接线。
//!
//! ```text
//! ┌──────────────────────────────────────────────┐
//! │ 标题栏 36px（自绘：拖拽 + 窗口控制 + 文档名）    │
//! ├────┬─────────┬──────────────────┬────────────┤
//! │图标│ 左栏     │ 编辑区           │ 右栏        │
//! │条  │ 缺省260  │ Editor / 空态     │ 缺省280     │
//! │52px│ (150-500)│ (#68 md 编辑面)  │ (150-600)  │
//! │    │ 可拖拽   │                  │ 可拖拽      │
//! ├────┴─────────┴──────────────────┴────────────┤
//! │ 状态栏 24px（文件 / 修改标记 / 行,列 / 字数）    │
//! └──────────────────────────────────────────────┘
//! ```
//!
//! 折叠状态（`left_open` / `right_open`）与 `editor`（`Option<Entity<Editor>>`，
//! 由 main 的 CLI 文档分派构造）是本视图全部自持状态；F1 起 `editor` 可经
//! [`ShellApp::open_document`] 运行时替换（Ctrl+O 对话框 / 文件树 / 最近
//! 列表共用通道），失败/不支持轻提示横幅（[`NoticeBoard`]）；F2 起
//! `workspace`（`Option<Entity<FileTree>>`，Ctrl+K Ctrl+O 打开文件夹）——
//! 记忆落盘随 settings（重启恢复）。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use std::sync::Arc;

use gpui::{
    App, Bounds, Context, CursorStyle, DispatchPhase, DragMoveEvent, Entity, FocusHandle,
    Focusable as _, KeyDownEvent, MouseButton, MouseDownEvent, MouseUpEvent, PathPromptOptions,
    Pixels, Render, Window, canvas, px,
};
use theme::Theme;
use ui::prelude::*;

use crate::activity_bar;
use crate::context_menu::{self, MenuItem};
use crate::context_menu_view;
use crate::document::{self, OpenOutcome};
use crate::editor::{Editor, SaveOutcome};
// #182：设置面板的缩进单位状态（editor::buffer 的纯枚举——面板/标签共用）。
use crate::editor::buffer::IndentUnit;
use crate::file_tree::FileTree;
use crate::git_panel::GitPanel;
use crate::menu_bar;
use crate::menu_bar_view;
use crate::outline_panel::OutlinePanel;
// #217：键位表（shell 键钩子的单一事实源——默认表 = 原分支精确镜像）。
use crate::keymap::{self, ShellAction};
use crate::panels;
use crate::pdf_pages::PdfPagesView;
use crate::recent::RecentStore;
use crate::right_dock;
use crate::settings_panel;
use crate::status_bar;
use crate::tab_bar::{self, TabItem};
use crate::theme_switch;
use crate::title_bar;
use crate::xlsx_grid_view::XlsxGridView;

/// Ctrl+K 和弦窗口（第二键有效期；过期自动作废——惰性判定无需定时器）。
const CHORD_WINDOW: Duration = Duration::from_secs(2);

/// F4：确认框选项序（0 保存 / 1 不保存 / 2 取消）。键盘 Enter 激活
/// 选中项、Tab/←/→ 循环、Esc 直接取消。
const CONFIRM_BUTTONS: [&str; 3] = ["保存", "不保存", "取消"];

/// #166：删除确认按钮（0 删除 / 1 取消）——与 CONFIRM_BUTTONS 分列：
/// 按钮集属于动作语义（保存三选 / 删除二选），不是全局常量。
const DELETE_BUTTONS: [&str; 2] = ["删除", "取消"];

/// F4：待确认动作（脏标签关闭 / 退出 / #166 文件树删除）。
#[derive(Clone, Debug)]
enum ConfirmAction {
    /// 关闭 `idx` 标签（它有未保存修改）。
    CloseTab { idx: usize },
    /// 退出应用（存在未保存修改的标签）。
    Quit,
    /// #166：永久删除文件树目标（确认后不可恢复——无回收站 API，
    /// gpui 闭包未暴露 trash 面）。
    DeletePath { path: PathBuf },
}

/// F4：确认框状态。
/// G4：面板拖拽侧（左栏/右栏——resize 手柄归属）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PanelSide {
    Left,
    Right,
}

/// G4：面板 resize 手柄的 drag 载荷类型（`on_drag` / `on_drag_move`
/// 配对标识；GPUI 按【类型】路由 drag 会话——侧别由监听器闭包捕获，
/// 载荷本身不携带数据）。
#[derive(Clone, Copy)]
struct PanelResizeDrag;

/// G4：面板拖拽会话态（mousedown 落点起 → drag_move 步进 → mouseup
/// 收尾落盘）。
#[derive(Clone, Copy, Debug)]
struct PanelDragState {
    side: PanelSide,
    /// 上一步的窗口 x（步进位移基准）。
    last_x: f32,
}

/// G4：resize 手柄跟随视图（`on_drag` 契约需要一个 Render 实体——
/// 空渲染不可见；与 editor::DragGhost 同模式）。
struct PanelDragGhost;

impl Render for PanelDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().id("panel-drag-ghost").size_0()
    }
}

struct ConfirmState {
    message: String,
    detail: String,
    action: ConfirmAction,
    /// 选中按钮序（键盘循环 / 鼠标点击）。语义随动作集不同：
    /// 保存三选（0 保存 1 不保存 2 取消）/ 删除二选（0 删除 1 取消）。
    selected: usize,
    /// #166：按钮文案集（按动作携带——不再全局固定三钮）。
    buttons: Vec<&'static str>,
}

pub struct ShellApp {
    left_open: bool,
    right_open: bool,
    /// G4：左右栏宽度（拖拽热区调整；settings `panel_widths` 持久化，
    /// 缺省 [`panels::LEFT_PANEL_WIDTH`] / [`panels::RIGHT_PANEL_WIDTH`]）。
    left_width: Pixels,
    right_width: Pixels,
    /// G4：面板 resize 拖拽会话（`None` = 非拖拽中）。
    panel_drag: Option<PanelDragState>,
    /// F3：标签集（每标签 = 一个打开文档）；`active` 指向当前标签
    ///（空集 = 空态）。原 `Option<Entity<Editor>>` 单例面由此取代。
    tabs: Vec<TabItem>,
    active: usize,
    /// F2：工作区文件树（打开文件夹后呈现；记忆落盘随 settings）。
    workspace: Option<Entity<FileTree>>,
    /// F7：git 面板（工作区打开即建——非 git 目录走面板内提示态）。
    git_panel: Option<Entity<GitPanel>>,
    /// #571：大纲面板实体（批 7——激活标签的键盘导航结构树；
    /// 换绑判据见 [`Self::sync_outline`]）。
    outline: Option<Entity<OutlinePanel>>,
    /// F7：左栏视图（文件树 / git——图标条切换）。
    git_view: bool,
    /// #289-⑤：设置弹窗开关（模态——并替代旧 `settings_view` 右栏视图；
    /// 左条设置钮/菜单「设置」打开，Esc/点遮罩/关闭钮关闭）。
    settings_open: bool,
    /// #289-④：pdf 页面视图（当前 pdf 标签的位图渲染实体 + 归属路径
    /// ——切到不同 pdf 标签时重建；关闭该标签时丢弃）。
    pdf_pages: Option<(PathBuf, Entity<PdfPagesView>)>,
    /// #289-④：pdf 显示模式（true = 页面位图视图——缺省；false =
    /// 文本行模型。状态栏「页面/文本」钮切换；两视图各有查找面
    /// （#422：页面视图内查找条，不再强制切文本）。
    pdf_view_mode: bool,
    /// #389 阶段 1：xlsx 网格视图（当前 xlsx 标签的真网格渲染实体 +
    /// 归属路径——生命周期与 pdf_pages 同模式：切标签重建/关标签丢弃）。
    xlsx_grid: Option<(PathBuf, Entity<XlsxGridView>)>,
    /// #389 阶段 2：xlsx 编辑会话银行（路径 → 会话 + 脏标记）。切标签
    /// 重建网格实体时，旧实体会话先行入行（`stash_xlsx_session`）
    /// ——已提交未落盘编辑跨实体存活零丢失；回切时经
    /// `new_with_session` 注回；关标签丢弃（关闭确认在前保证 dirty
    /// 先保存/放弃）。
    xlsx_sessions: std::collections::HashMap<PathBuf, (mdc_office_io::xlsx_edit::XlsxEditor, bool)>,
    /// #389 阶段 1：xlsx 显示模式（true = 网格视图——缺省；false =
    /// 文本行模型 fallback。状态栏「网格/文本」钮切换——pdf 页面/文本
    /// 先例同构）。
    xlsx_grid_view_mode: bool,
    /// #164 1c：首帧计时基点（ShellApp 构造时置 = perf::boot()，首个
    /// render 呈现时消费——`perf: first-frame` 证据行；MANG_PERF 未开
    /// 恒 None，零开销）。
    perf_first_frame: Option<Instant>,
    /// F2：Ctrl+K 和弦待续期（None = 无和弦进行中）。
    chord: Option<(keymap::KeyQuery, Instant)>,
    /// F4：确认框（脏标签关闭/退出前确认；None = 无进行中）。
    confirm: Option<ConfirmState>,
    /// F4：确认框焦点域（模态开启时接管焦点——编辑器不再收键；
    /// 键事件沿遮罩节点→shell 根冒泡，由确认拦截分支统一处理）。
    confirm_focus: FocusHandle,
    /// shell 根焦点域（空态回退——无标签时键事件仍走 root→shell 根
    /// 冒泡路径；无此前提派发路径止于合成根节点，shell 级键位全聋）。
    root_focus: FocusHandle,
    /// F5：最近打开（文件 20 / 工作区 10——空态引导页呈现 + 写穿透落盘）。
    recent: RecentStore,
    /// F6：主题模式（system/light/dark——Ctrl+Alt+T 循环；持久化）。
    theme_mode: &'static str,
    /// F6：主题蓝本（启动时的全局主题 = One Dark——浅色派生与切回的
    /// 基准；运行期不追改）。
    base_theme: Arc<Theme>,
    /// F6：编辑器字号（px，8-32——Ctrl+=/-/0 缩放；应用到全部标签；
    /// 持久化）。
    font_size: f32,
    /// #182：缩进单位运行时值（设置面板展示/写入的单一状态源——
    /// 新建标签仍从 settings 读，两处同源不漂移；持久化走
    /// `settings::indent_unit` 既有键，零新增存储）。
    indent_unit: IndentUnit,
    /// F1：提示横幅（打开失败 / 类型不支持 / 对话框不可用等面向用户消息）。
    notice_board: NoticeBoard,
    /// #63 打磨（#385 批 1）：右键菜单实体（`ui::ContextMenu` 组件，
    /// None = 未打开）。键盘导航/焦点/ARIA/dismiss 由组件自带；动作
    /// 派发回 shell 的 activate_context_menu_item（id 派发面不变）。
    context_menu: Option<Entity<ui::ContextMenu>>,
    /// 右键菜单展开点（触发时的窗口坐标，`anchored` 锚定用——溢出
    /// 翻转/吸附由引擎按实际尺寸处理，不再需要 shell 侧几何）。
    context_menu_pos: gpui::Point<Pixels>,
    /// #63 打磨：右键菜单来源（决定动作 id 派发到哪套处理器）。
    context_menu_origin: MenuOrigin,
    /// #166：文件树右键目标（动作执行的路径面——新建/打开/重命名/删除/
    /// 资源管理器揭示都需要知道右键的是哪一行）。
    context_menu_target: Option<PathBuf>,
    /// #209（#385 批 4）：当前展开的顶部菜单序号（None = 全关）。
    /// 选中项/子菜单开合/键盘导航已由 `ui::ContextMenu` 组件自带，
    /// shell 只编排「哪个菜单开」与浮层实体生命周期。
    menu_bar_open: Option<usize>,
    /// #385 批 4：菜单栏下拉实体（`ui::ContextMenu`——打开时构建，
    /// 动作派发回 `activate_menu_action`；DismissEvent → close 清态）。
    menu_bar_entity: Option<Entity<ui::ContextMenu>>,
    /// 下拉展开锚点（标题点击 x + 标题栏下缘 y——`anchored` 定位用，
    /// 溢出翻转/吸附由引擎按实际渲染尺寸处理）。
    menu_bar_anchor: gpui::Point<Pixels>,
    /// 菜单实体的 DismissEvent 订阅（打开下一个菜单时 drop 旧的——
    /// 防旧实体的迟到 Dismiss 误关新菜单；与实体的生命周期同步）。
    menu_bar_subscription: Option<gpui::Subscription>,
}

/// 右键菜单来源（决定同一动作 id 派发到哪套处理器）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MenuOrigin {
    /// 编辑器区右键（剪切/复制/粘贴/查找…）。
    #[default]
    Editor,
    /// 文件树右键（新建/重命名/删除/在资源管理器显示）。
    FileTree,
}

/// F1：提示横幅状态机（纯逻辑——代际/清除语义单测钉定）。
///
/// 代际（`generation`）防「旧定时误清新消息」：每条消息取得代际号，
/// 自动清除任务只在当前代际匹配时生效——后到的消息使先前的定时器失效。
#[derive(Default)]
struct NoticeBoard {
    notice: Option<Notice>,
    generation: u32,
}

struct Notice {
    generation: u32,
    message: String,
}

impl NoticeBoard {
    /// 展示消息，返回其代际号。
    fn show(&mut self, message: impl Into<String>) -> u32 {
        self.generation += 1;
        self.notice = Some(Notice {
            generation: self.generation,
            message: message.into(),
        });
        self.generation
    }

    /// 按代际清除（仅当仍是当前消息）；返回是否实际清除。
    fn dismiss(&mut self, generation: u32) -> bool {
        if self
            .notice
            .as_ref()
            .is_some_and(|n| n.generation == generation)
        {
            self.notice = None;
            true
        } else {
            false
        }
    }

    /// 立即清空（成功打开文档时）。
    fn clear(&mut self) {
        self.notice = None;
    }

    fn message(&self) -> Option<&str> {
        self.notice.as_ref().map(|n| n.message.as_str())
    }
}

impl ShellApp {
    /// `editor`：成功打开的启动文档（`None` = 空态）；`workspace`：
    /// 启动恢复的工作区根（F2 记忆——`None` = 无记忆/不存在）。
    pub fn new(
        editor: Option<Entity<Editor>>,
        workspace_root: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        if let Some(root) = &workspace_root {
            // 证据通道（与 open_workspace 的 "workspace: <路径>" 区分）。
            eprintln!("workspace(restored): {}", root.display());
        }
        let workspace = workspace_root.map(|root| {
            let shell = cx.entity().downgrade();
            cx.new(|cx| FileTree::new(root, Self::file_tree_hooks(shell), cx))
        });
        // F3：启动文档 → 首个标签（未命名草稿取合成身份——CLI 无参
        // 不构造实体，实际不可达；防御完整）。
        let tabs = editor
            .map(|editor| {
                let path = editor
                    .read(cx)
                    .file_path()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| PathBuf::from("未命名"));
                vec![TabItem { path, editor }]
            })
            .unwrap_or_default();
        // #179（#188 接线）：启动标签注入右键菜单回调（editor 侧不引
        // ShellApp 类型——lib 目标不含 shell 模块树；弱句柄在此捕获，
        // #166 file_tree 同款模式。与下述 open_document_tab /
        // open_readonly_tab 同一注入面，三入口全覆盖）。
        let shell_weak = cx.entity().downgrade();
        for tab in &tabs {
            tab.editor.update(cx, |ed, _| {
                ed.set_menu_opener(menu_opener_for(shell_weak.clone()));
            });
        }
        // F4：窗口关闭前确认钩子（平台 X 钮/OS 关闭——四后端均接线
        // on_should_close；cx.quit 不经此路）。脏标签存在 → 拦截本次
        // 关闭 + 弹确认框；确认后保存/放弃再 cx.quit。
        let confirm_focus = cx.focus_handle();
        let root_focus = cx.focus_handle();
        // F5：最近打开记忆（空态引导页数据源）。CLI 启动文件不走
        // open_document（main 直构）——此处补记。
        let mut recent = RecentStore::load();
        for tab in &tabs {
            recent.record_file(&tab.path);
        }
        // F6：主题模式（settings 记忆 → 应用）+ 蓝本捕获 + 字号。
        let base_theme = theme::GlobalTheme::theme(cx).clone();
        let theme_mode = theme_switch::parse_mode(crate::settings::load_theme_mode().as_deref());
        theme_switch::apply_mode(theme_mode, &base_theme, cx);
        let font_size = crate::settings::load_font_size().unwrap_or(15.);
        if !recent.recent_files().is_empty() || !recent.recent_workspaces().is_empty() {
            eprintln!(
                "recent: {} 文件 / {} 工作区",
                recent.recent_files().len(),
                recent.recent_workspaces().len()
            );
        }
        let root_focus_for_init = root_focus.clone();
        {
            let shell = cx.entity().clone();
            let confirm_focus = confirm_focus.clone();
            window.on_window_should_close(cx, move |window, cx| {
                let dirty: Vec<usize> = shell
                    .read(cx)
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(i, tab)| {
                        // #386：pdf 批注脏计入退出前确认（tab_is_modified
                        // 同源——含页视图未落盘批注）。
                        tab.editor.read(cx).is_modified()
                            || shell.read(cx).pdf_view_dirty(cx, &tab.path)
                    })
                    .map(|(i, _)| i)
                    .collect();
                if dirty.is_empty() {
                    return true;
                }
                shell.update(cx, |shell, cx| {
                    eprintln!("confirm: 退出前确认（{} 个未保存）", dirty.len());
                    shell.confirm = Some(ConfirmState {
                        message: format!("有 {} 个未保存的文档", dirty.len()),
                        detail: "退出前保存修改？".to_string(),
                        action: ConfirmAction::Quit,
                        selected: 0,
                        buttons: CONFIRM_BUTTONS.to_vec(),
                    });
                    cx.notify();
                });
                // 模态接管焦点（编辑器隔离）。
                window.focus(&confirm_focus, cx);
                false
            });
        }
        let mut this = Self {
            left_open: true,
            right_open: true,
            // G4：面板宽度记忆恢复（无记忆 = 缺省宽；手改配置越界时
            // 载入即钳制到拖拽域——不信任磁盘值）。
            left_width: crate::settings::load_panel_widths()
                .map(|(l, _)| px(panels::clamp_panel_width(true, l)))
                .unwrap_or(panels::LEFT_PANEL_WIDTH),
            right_width: crate::settings::load_panel_widths()
                .map(|(_, r)| px(panels::clamp_panel_width(false, r)))
                .unwrap_or(panels::RIGHT_PANEL_WIDTH),
            panel_drag: None,
            tabs,
            active: 0,
            workspace,
            git_panel: None,
            outline: None,
            git_view: false,
            settings_open: false,
            pdf_pages: None,
            pdf_view_mode: true,
            xlsx_grid: None,
            xlsx_sessions: std::collections::HashMap::new(),
            xlsx_grid_view_mode: true,
            perf_first_frame: crate::perf::on().then(crate::perf::boot),
            chord: None,
            confirm: None,
            confirm_focus,
            root_focus,
            recent,
            theme_mode,
            base_theme,
            font_size,
            indent_unit: crate::settings::load_indent_unit()
                .as_deref()
                .map(IndentUnit::from_setting)
                .unwrap_or_default(),
            notice_board: NoticeBoard::default(),
            context_menu: None,
            context_menu_pos: gpui::Point::new(Pixels::ZERO, Pixels::ZERO),
            context_menu_origin: MenuOrigin::default(),
            context_menu_target: None,
            menu_bar_open: None,
            menu_bar_entity: None,
            menu_bar_anchor: gpui::Point::new(Pixels::ZERO, Pixels::ZERO),
            menu_bar_subscription: None,
        };
        // 初始焦点：有文档 → 编辑器（键入/编辑即时可用）；空态 →
        // shell 根域（Ctrl+O 等空态键位经 root→shell 根路径可达——
        // 无焦点时派发路径止于合成根节点，shell 监听器不触发）。
        match this.editor() {
            Some(tab) => {
                let focus = tab.read(cx).focus_handle().clone();
                window.focus(&focus, cx);
            }
            None => window.focus(&root_focus_for_init, cx),
        }
        // F6：记忆字号应用到启动标签。
        this.apply_font_size(px(this.font_size), cx);
        // #571：启动标签的大纲面板落位。
        this.sync_outline(cx);
        this
    }

    /// 当前标签的编辑器（无标签 = 空态 None）。
    pub fn editor(&self) -> Option<&Entity<Editor>> {
        self.tabs.get(self.active).map(|tab| &tab.editor)
    }

    // ------------------------------------------------------------------
    // RPC 桥公开面（#104 任务 2）：标签清单/定位/无确认关闭——控制
    // 通道（get_state/close_file/switch_tab）的数据与操作面。全部薄
    // 方法（既有私有面的直读），不引入新状态。
    // ------------------------------------------------------------------

    /// 标签数。
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// 活动标签索引（无标签时为遗留值——调用方以 tab_count 先判空）。
    pub fn active_index(&self) -> usize {
        self.active
    }

    /// 指定标签的路径（越界 `None`；未命名草稿为「未命名」占位路径）。
    pub fn tab_path(&self, idx: usize) -> Option<PathBuf> {
        self.tabs.get(idx).map(|tab| tab.path.clone())
    }

    /// 指定标签是否有未保存修改（越界 `None`）。#389 阶段 2：xlsx
    /// 标签追加网格编辑会话脏态（活实体或银行会话——两路同查）。
    /// #386：pdf 标签的批注脏（页视图未落盘）计入——标题栏脏标/
    /// 关闭确认/RPC 拦截同源。
    pub fn tab_is_modified(&self, idx: usize, cx: &App) -> Option<bool> {
        self.tabs.get(idx).map(|tab| {
            tab.editor.read(cx).is_modified()
                || self.xlsx_dirty_at(&tab.path, cx)
                || self.pdf_view_dirty(cx, &tab.path)
        })
    }

    /// #386：指定路径的 pdf 页视图是否有未落盘批注（Shell 持有的
    /// `pdf_pages` 槽按路径匹配——无页视图/路径不匹配 = 不脏）。
    fn pdf_view_dirty(&self, cx: &App, path: &Path) -> bool {
        self.pdf_pages
            .as_ref()
            .is_some_and(|(owned, entity)| *owned == path && entity.read(cx).is_dirty())
    }

    /// #386：标签级保存分流（「谁负责保存」的决策点——按 issue 铁律
    /// 放 Shell，Editor 不知 PDF 存在）：
    ///
    /// - pdf 标签且页视图脏 → [`PdfPagesView::save_annotations`] 增量
    ///   落盘（路径 = Shell 槽持有的 `PathBuf`，单一真源）；失败
    ///   toast 显式示警（不静默）。
    /// - 其余（含 pdf 无页视图/页视图干净）→ [`Editor::save`] 三态；
    ///   `ReadOnlyRejected`/`Failed` toast 示警。
    ///
    /// 返回三态供调用方（Ctrl+S 反馈 / 确认框决策：`Saved`/
    /// `Unmodified` 才可关标签）。
    pub fn save_tab(&mut self, idx: usize, cx: &mut Context<Self>) -> SaveOutcome {
        let is_pdf = self
            .tabs
            .get(idx)
            .is_some_and(|tab| tab.editor.read(cx).doc_kind() == "pdf");
        if is_pdf {
            let tab_path = self.tabs.get(idx).map(|tab| tab.path.clone());
            if let Some(path) = tab_path {
                if let Some((owned, entity)) = self.pdf_pages.clone() {
                    if owned == path {
                        return match entity.update(cx, |v, _| v.save_annotations(&owned)) {
                            Ok(Some(_)) => SaveOutcome::Saved,
                            // 页视图干净 → 文本面只读态（编辑器三态给
                            // Unmodified——只读文本面无未存修改）。
                            Ok(None) => self.editor_save_outcome(idx, cx),
                            Err(e) => {
                                self.show_notice(
                                    format!("PDF 批注保存失败（{}）：{e}", path.display()),
                                    cx,
                                );
                                SaveOutcome::Failed
                            }
                        };
                    }
                }
            }
        }
        self.editor_save_outcome(idx, cx)
    }

    /// [`Self::save_tab`] 的编辑器分支（三态消费 + 拒绝/失败示警）。
    fn editor_save_outcome(&mut self, idx: usize, cx: &mut Context<Self>) -> SaveOutcome {
        let Some(tab) = self.tabs.get(idx) else {
            return SaveOutcome::Failed;
        };
        let editor = tab.editor.clone();
        let outcome = editor.update(cx, |ed, _| ed.save());
        match outcome {
            SaveOutcome::ReadOnlyRejected => {
                self.show_notice("此文档为只读，改动无法保存".into(), cx);
            }
            SaveOutcome::Failed => {
                let name = editor.read(cx).display_name().to_string();
                self.show_notice(format!("保存失败（{name}）——详见应用日志"), cx);
            }
            SaveOutcome::Saved | SaveOutcome::Unmodified => {}
        }
        outcome
    }

    /// 指定标签是否只读渲染态（越界 `None`）。
    pub fn tab_is_readonly(&self, idx: usize, cx: &App) -> Option<bool> {
        self.tabs
            .get(idx)
            .map(|tab| tab.editor.read(cx).is_read_only())
    }

    /// 按路径定位标签索引（与 open_document_tab 同口径绝对化——
    /// CLI 相对入参 vs 对话框绝对入参不歧义）。未命中 `None`。
    pub fn tab_index_by_path(&self, path: &Path) -> Option<usize> {
        let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        self.tabs.iter().position(|tab| tab.path == absolute)
    }

    /// 无确认关闭指定标签（控制面通道——确认框是用户交互面，不适合
    /// 机器通道；**脏拦截在调用方**（RPC 层拒 dirty，先保存再关）。
    /// 越界 `false`，关闭成功 `true`。
    pub fn close_tab_no_confirm(
        &mut self,
        idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if idx >= self.tabs.len() {
            return false;
        }
        self.close_tab_at(idx, window, cx);
        true
    }

    /// 折叠/展开左栏（文件树）。图标条 `act-file-tree` 触发——已开且
    /// 在 git 视图时切回文件树视图（F7）。
    pub fn toggle_left(&mut self, cx: &mut Context<Self>) {
        if self.left_open && self.git_view {
            self.git_view = false;
        } else {
            self.left_open = !self.left_open;
        }
        cx.notify();
    }

    /// F7：切到 git 视图（图标条 `act-git` 触发——已开且在 git 视图时
    /// 折叠左栏，再开切回）。
    pub fn toggle_git(&mut self, cx: &mut Context<Self>) {
        if self.left_open && self.git_view {
            self.left_open = false;
        } else {
            self.left_open = true;
            self.git_view = true;
        }
        cx.notify();
    }

    /// F7：git 面板实体（烟雾钩子/内部访问）。
    /// #571：大纲面板换绑——激活编辑器变更时重建（同步行集 + 句柄
    /// 差量在实体内部）；同编辑器（含光标/编辑中）复用既有实体（焦点
    /// 身份稳定）。#289-④ pdf 页滚经 ActivateHook 注入（实体在 lib 树
    /// 不引 shell——依赖倒置）。
    fn sync_outline(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor().cloned() else {
            self.outline = None;
            return;
        };
        let stale = match &self.outline {
            Some(panel) => panel.read(cx).editor() != &editor,
            None => true,
        };
        if stale {
            let shell = cx.entity().downgrade();
            let escape_shell = shell.clone();
            let ed = editor.clone();
            // #289-④：pdf 页面视图下激活行随页滚动（书签链同款）。
            let hook = move |line: usize, window: &mut Window, cx: &mut App| {
                if let Some(page) = ed.read(cx).pdf_page_for_line(line) {
                    if let Some(shell) = shell.upgrade() {
                        let _ = shell.update(cx, |shell, cx| shell.scroll_pdf_to_page(page, cx));
                    }
                }
            };
            let escape = move |window: &mut Window, cx: &mut App| {
                let _ = escape_shell.update(cx, |shell, cx| shell.focus_active(window, cx));
            };
            self.outline = Some(cx.new(|cx| {
                OutlinePanel::new(editor, Some(std::sync::Arc::new(hook)), cx)
                    .with_escape_hook(std::sync::Arc::new(escape))
            }));
        }
    }

    /// #575：FileTree 的 ShellApp 面收口（lib 化钩子注入——三闭包
    /// 调既有方法，行为零变化）。
    fn file_tree_hooks(shell: gpui::WeakEntity<Self>) -> crate::file_tree::FileTreeHooks {
        use std::sync::Arc;
        let open_shell = shell.clone();
        let menu_shell = shell.clone();
        crate::file_tree::FileTreeHooks {
            // 文件点击 → open_document_tab + 焦点落位（原链原样）。
            open_file: Arc::new(move |path, window, cx| {
                let focus = open_shell
                    .update(cx, |shell, cx| shell.open_document_tab(path.clone(), cx))
                    .ok()
                    .flatten();
                if let Some(focus) = focus {
                    window.focus(&focus, cx);
                }
            }),
            // 右键 → 文件树上下文菜单（#166 原链——坐标转换在此收口）。
            context_menu: Arc::new(move |pos, path, is_dir, window, cx| {
                let _ = menu_shell.update(cx, |shell, cx| {
                    shell.open_file_tree_context_menu(pos, path, is_dir, window, cx)
                });
            }),
            // Esc → 焦点还激活编辑器（#571 大纲面板同语义）。
            escape: Arc::new(move |window, cx| {
                let _ = shell.update(cx, |shell, cx| shell.focus_active(window, cx));
            }),
        }
    }

    /// #577：GitPanel 的 ShellApp 面收口（lib 化钩子注入——行为零变化：
    /// open_readonly = 原 open_readonly_tab + 焦点落位原链）。
    fn git_panel_hooks(shell: gpui::WeakEntity<Self>) -> crate::git_panel::GitPanelHooks {
        use std::sync::Arc;
        let open_shell = shell.clone();
        crate::git_panel::GitPanelHooks {
            open_readonly: Arc::new(move |title, text, window, cx| {
                let focus = open_shell
                    .update(cx, |shell, cx| shell.open_readonly_tab(title, text, cx))
                    .ok()
                    .flatten();
                if let Some(focus) = focus {
                    window.focus(&focus, cx);
                }
            }),
            escape: Arc::new(move |window, cx| {
                let _ = shell.update(cx, |shell, cx| shell.focus_active(window, cx));
            }),
        }
    }

    pub fn git_panel(&self) -> Option<Entity<GitPanel>> {
        self.git_panel.clone()
    }

    // ------------------------------------------------------------------
    // G4：面板宽度拖拽（边缘 4px 热区；col-resize 光标；域钳制；
    // settings 持久化）
    // ------------------------------------------------------------------

    /// G4：手柄 mousedown——落点入会话态（`last_x` 为步进位移基准）。
    fn begin_panel_drag(&mut self, side: PanelSide, x: f32) {
        self.panel_drag = Some(PanelDragState { side, last_x: x });
    }

    /// G4：手柄 drag_move——步进位移按侧别换算宽度（左栏右移增宽 /
    /// 右栏左移增宽），域钳制后重绘。选区/文本不受影响：drag 会话
    /// 捕获全部 move 事件（payload 异于编辑器拖选——不同 drag 通道，
    /// 编辑器 mousedown 未发生，无选区可言）。
    fn dispatch_panel_resize(
        &mut self,
        side: PanelSide,
        event: &DragMoveEvent<PanelResizeDrag>,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.panel_drag.as_mut() else {
            return;
        };
        if state.side != side {
            return;
        }
        let now_x = f32::from(event.event.position.x);
        let dx = now_x - state.last_x;
        state.last_x = now_x;
        let is_left = side == PanelSide::Left;
        let current = if is_left {
            f32::from(self.left_width)
        } else {
            f32::from(self.right_width)
        };
        let clamped =
            panels::clamp_panel_width(is_left, current + panels::width_delta(is_left, dx));
        if is_left {
            self.left_width = px(clamped);
        } else {
            self.right_width = px(clamped);
        }
        cx.notify();
    }

    /// G4：拖拽收尾（窗口级 mouseup——任意位置抬起；与 editor 同模式
    /// 每帧注册）。宽度落 settings（持久化）+ 证据行；非拖拽中为空操作。
    fn finish_panel_drag(&mut self, cx: &mut Context<Self>) {
        if self.panel_drag.take().is_none() {
            return;
        }
        let (left, right) = (f32::from(self.left_width), f32::from(self.right_width));
        crate::settings::save_panel_widths(left, right);
        eprintln!("panel-resize: left {left} right {right} (persist)");
        cx.notify();
    }

    /// G4：手柄元素（边缘 4px 热区——col-resize 光标 + 拖拽会话）。
    fn panel_resize_handle(&self, side: PanelSide, cx: &Context<Self>) -> impl IntoElement {
        let hover_bg = cx.theme().colors().element_hover;
        div()
            .id(SharedString::from(match side {
                PanelSide::Left => "panel-resize-left",
                PanelSide::Right => "panel-resize-right",
            }))
            .w(panels::PANEL_RESIZE_HOTZONE)
            .h_full()
            .flex_none()
            .cursor(CursorStyle::ResizeColumn)
            .hover(|el| el.bg(hover_bg))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                    this.begin_panel_drag(side, f32::from(event.position.x));
                    cx.notify();
                }),
            )
            .on_drag(PanelResizeDrag, |_, _, _, cx| cx.new(|_| PanelDragGhost))
            .on_drag_move::<PanelResizeDrag>(cx.listener(
                move |this, event: &DragMoveEvent<PanelResizeDrag>, _window, cx| {
                    this.dispatch_panel_resize(side, event, cx);
                },
            ))
    }

    /// F7：打开只读文本标签（diff 视图 / 提交清单——Editor 只读态：
    /// 导航/查找/复制全复用，编辑被门禁拦截）。同路径已开 → 激活。
    pub fn open_readonly_tab(
        &mut self,
        title: String,
        content: String,
        cx: &mut Context<Self>,
    ) -> Option<FocusHandle> {
        let path = PathBuf::from(format!("git-view:{title}"));
        if let Some(idx) = self.tabs.iter().position(|tab| tab.path == path) {
            return self.activate_tab_focus(idx, cx);
        }
        let editor = cx.new(|cx| Editor::new_readonly(Some(path.clone()), content, cx));
        // #179（#188 接线）：只读标签同样注入右键菜单回调（菜单对只读
        // 态有灰显语义——剪切/粘贴/替换灰显，复制/查找/跳行可用）。
        let shell_weak = cx.entity().downgrade();
        editor.update(cx, |ed, _| {
            ed.set_menu_opener(menu_opener_for(shell_weak));
        });
        let focus = editor.read(cx).focus_handle().clone();
        self.tabs.push(TabItem { path, editor });
        self.active = self.tabs.len() - 1;
        // #571：激活编辑器变更 → 大纲面板换绑。
        self.sync_outline(cx);
        eprintln!("git-view: {title}（tabs: {} 开）", self.tabs.len());
        cx.notify();
        Some(focus)
    }

    /// 折叠/展开右栏（属性/来源）。右缘 dock 条 `dock-toggle-right`
    /// 触发（#289-⑤：右栏开关归右缘——不再占用左图标条）。
    pub fn toggle_right(&mut self, cx: &mut Context<Self>) {
        self.right_open = !self.right_open;
        cx.notify();
    }

    /// #289-⑤：打开设置弹窗（左条设置钮 / 菜单「查看 → 设置」入口；
    /// 弹窗形态——zed「设置是独立界面不是 dock 视图」同款语义）。
    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_open = true;
        cx.notify();
    }

    /// #289-⑤：关闭设置弹窗（Esc / 点遮罩 / 卡片关闭钮）。
    pub fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_open = false;
        cx.notify();
    }

    /// #289-⑤：设置弹窗开合切换（左条设置钮——再点收起；与
    /// toggle_git 同构的「一键直达」语义）。
    pub fn toggle_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_open = !self.settings_open;
        cx.notify();
    }

    // ── #289-④：pdf 页面视图（位图渲染面） ──────────────────────

    /// 当前活动标签是否 pdf 态（页视图的挂载判据）。
    fn active_is_pdf(&self, cx: &Context<Self>) -> bool {
        self.editor()
            .is_some_and(|ed| ed.read(cx).doc_kind() == "pdf")
    }

    /// 确保当前 pdf 标签的页视图实体在场（非 pdf 态 / 已在同路径实体
    /// 上 = 无操作）。切到不同 pdf 标签时重建（旧实体 drop——页缓存
    /// 随之释放，单实体生命周期与单 pdf 标签对齐）。
    fn ensure_pdf_pages(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if tab.editor.read(cx).doc_kind() != "pdf" {
            return;
        }
        let path = tab.path.clone();
        if let Some((owned, _)) = &self.pdf_pages {
            if *owned == path {
                return;
            }
        }
        // 读盘（pdf_doc::load 同源 100MB 前置门——超限不入页视图，
        // 文本模型不受影响）。
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) => {
                let message = format!("页面视图装载失败（{}）：{err}", path.display());
                self.show_notice(message, cx);
                self.pdf_pages = None;
                return;
            }
        };
        if bytes.len() as u64 > 100 * 1024 * 1024 {
            self.show_notice("页面视图仅支持 ≤100MB 文件（文本视图不受限）".into(), cx);
            self.pdf_pages = None;
            return;
        }
        let entity = cx.new(|cx| PdfPagesView::new(bytes, cx));
        self.pdf_pages = Some((path, entity));
        cx.notify();
    }

    /// #289-④：切换 pdf 显示模式（状态栏「页面/文本」钮）。
    pub fn toggle_pdf_view_mode(&mut self, cx: &mut Context<Self>) {
        if !self.active_is_pdf(cx) {
            return;
        }
        self.pdf_view_mode = !self.pdf_view_mode;
        if self.pdf_view_mode {
            self.ensure_pdf_pages(cx);
        }
        cx.notify();
    }

    /// #289-④：页视图滚到指定页（书签跳转锚；1 基页号，非 pdf 态/
    /// 文本模式/无实体 = 无操作）。
    pub fn scroll_pdf_to_page(&mut self, page_1based: usize, cx: &mut Context<Self>) {
        if !self.pdf_view_mode {
            return;
        }
        if let Some((_, entity)) = &self.pdf_pages {
            entity
                .read(cx)
                .scroll_to_page(page_1based.saturating_sub(1));
            cx.notify();
        }
    }

    // ── #389 阶段 1：xlsx 网格视图（真网格渲染面） ──────────────

    /// 当前活动标签是否 xlsx 态（网格视图的挂载判据）。
    fn active_is_xlsx(&self, cx: &Context<Self>) -> bool {
        self.editor()
            .is_some_and(|ed| ed.read(cx).doc_kind() == "xlsx")
    }

    /// 确保当前 xlsx 标签的网格视图实体在场（生命周期与 ensure_pdf_pages
    /// 同模式：非 xlsx 态 / 已在同路径实体上 = 无操作；切标签重建）。
    /// #389 阶段 2：重建前旧实体会话入行（`stash_xlsx_session`——脏
    /// 编辑跨实体存活）；银行命中时经 `new_with_session` 回接。
    fn ensure_xlsx_grid(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if tab.editor.read(cx).doc_kind() != "xlsx" {
            return;
        }
        let path = tab.path.clone();
        if let Some((owned, _)) = &self.xlsx_grid {
            if *owned == path {
                return;
            }
        }
        // #389 阶段 2：旧实体退役——会话（含未收尾缓冲的提交）入行。
        self.stash_xlsx_session(cx);
        // 网格模型装载（office-io 读面同源——xlsx_doc::load 同一入口；
        // 失败 → error 态实体 + 提示，文本模型不受影响）。
        let entity = match crate::xlsx_grid_view::XlsxGridModel::load(&path) {
            Ok(model) => {
                // #389 阶段 2：银行命中 → 会话回接（脏标记随行）；未
                // 命中 → 现开（失败不阻渲染，双击时明示拒因）。
                if let Some((session, dirty)) = self.xlsx_sessions.remove(&path) {
                    cx.new(|cx| {
                        let mut view =
                            XlsxGridView::new_with_session(model, path.clone(), session, cx);
                        view.set_modified(dirty);
                        view
                    })
                } else {
                    cx.new(|cx| XlsxGridView::new(model, path.clone(), cx))
                }
            }
            Err(err) => {
                let message = format!("网格视图装载失败（{}）：{err}", path.display());
                self.show_notice(message, cx);
                cx.new(|cx| XlsxGridView::error_state(err, cx))
            }
        };
        self.xlsx_grid = Some((path, entity));
        cx.notify();
    }

    /// #389 阶段 2：旧网格实体会话入行（实体退役前调用——`take_session`
    /// 内部先提交未收尾的编辑缓冲，再交还会话；脏标记随行保存）。
    fn stash_xlsx_session(&mut self, cx: &mut Context<Self>) {
        let Some((path, view)) = self.xlsx_grid.take() else {
            return;
        };
        let taken = view.update(cx, |v, _| {
            let dirty = v.is_modified();
            v.take_session().map(|session| (session, dirty))
        });
        if let Some((session, dirty)) = taken {
            self.xlsx_sessions.insert(path, (session, dirty));
        }
    }

    /// #389 阶段 2：指定路径的 xlsx 脏态（活实体或银行会话任一）。
    fn xlsx_dirty_at(&self, path: &Path, cx: &App) -> bool {
        if let Some((owned, view)) = &self.xlsx_grid {
            if owned == path {
                return view.read(cx).is_modified();
            }
        }
        self.xlsx_sessions
            .get(path)
            .is_some_and(|(_, dirty)| *dirty)
    }

    /// #389 阶段 2：指定路径的 xlsx 保存（统一收口——Ctrl+S 键位/
    /// 菜单「文件 → 保存」/关闭确认「保存」/退出确认三路共用）。
    /// 活实体在场 → 实体保存（会话在实体里）；否则银行会话直接落盘。
    fn save_xlsx_at(&mut self, path: &Path, cx: &mut Context<Self>) -> bool {
        if let Some((owned, view)) = &self.xlsx_grid {
            if owned == path {
                return view.update(cx, |v, _| v.save());
            }
        }
        let Some((session, dirty)) = self.xlsx_sessions.get_mut(path) else {
            // 无会话 = 只读浏览态（未编辑过）——无可保存，安静成功
            //（与「保存了但没变化」同语义；假成功风险零——本就无编辑）。
            return true;
        };
        if !*dirty {
            return true;
        }
        // 借用域内完成序列化 + 落盘（错误统一字符串面）。
        let saved: Result<(), String> =
            session
                .save_to_vec()
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    crate::xlsx_grid_view::write_xlsx_atomically(path, &bytes)
                        .map_err(|e| e.to_string())
                });
        // session/dirty 借用已尽（NLL）——dirty 回写与 notice 面安全。
        match saved {
            Ok(()) => {
                if let Some((_, dirty)) = self.xlsx_sessions.get_mut(path) {
                    *dirty = false;
                }
                eprintln!("saved (xlsx session): {}", path.display());
                true
            }
            Err(e) => {
                self.show_notice(format!("xlsx 保存失败（{}）：{e}", path.display()), cx);
                false
            }
        }
    }

    /// #389 阶段 1：切换 xlsx 显示模式（状态栏「网格/文本」钮——pdf
    /// 页面/文本先例同构）。
    pub fn toggle_xlsx_view_mode(&mut self, cx: &mut Context<Self>) {
        if !self.active_is_xlsx(cx) {
            return;
        }
        self.xlsx_grid_view_mode = !self.xlsx_grid_view_mode;
        if self.xlsx_grid_view_mode {
            self.ensure_xlsx_grid(cx);
        }
        cx.notify();
    }

    // ── #63 打磨：右键菜单 ─────────────────────────────────────

    /// 在窗口坐标 `pos` 处弹出编辑器右键菜单（#385 批 1：构建
    /// `ui::ContextMenu` 实体，键盘导航/焦点/dismiss 由组件自带）。
    ///
    /// `has_selection` / `read_only` 决定各项可用性（灰显而非消失）。
    /// 溢出翻转/吸附由 `anchored` 按实际渲染尺寸处理。
    pub fn open_editor_context_menu(
        &mut self,
        pos: gpui::Point<Pixels>,
        has_selection: bool,
        read_only: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.spawn_context_menu(
            context_menu::editor_menu(has_selection, read_only),
            pos,
            MenuOrigin::Editor,
            None,
            window,
            cx,
        );
    }

    /// 在窗口坐标 `pos` 处弹出文件树右键菜单（#166：`target` = 右键的
    /// 行路径——动作执行面；目录本身 / 文件均可作目标）。
    pub fn open_file_tree_context_menu(
        &mut self,
        pos: gpui::Point<Pixels>,
        target: PathBuf,
        is_dir: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.spawn_context_menu(
            context_menu::file_tree_menu(is_dir),
            pos,
            MenuOrigin::FileTree,
            Some(target),
            window,
            cx,
        );
    }

    /// 右键菜单实体构建/接线单点（两个 open_* 共用）。
    ///
    /// - 组件 `DismissEvent`（点外部/Esc/选完动作）→ 状态清理 + 焦点
    ///   归位（编辑器）——原遮罩点击关闭面由组件 on_blur 承担；
    /// - 焦点移入菜单（键盘导航的前提——`RightClickMenu` 同款两帧后
    ///   聚焦，避免 deferred 渲染树未链入时的闪焦）；
    /// - 打开菜单栏时互斥关闭右键菜单的旧链路保留在各自入口。
    fn spawn_context_menu(
        &mut self,
        items: Vec<MenuItem>,
        pos: gpui::Point<Pixels>,
        origin: MenuOrigin,
        target: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shell = cx.entity().downgrade();
        let menu = context_menu_view::build_context_menu(&items, shell, window, cx);
        self.context_menu = Some(menu.clone());
        self.context_menu_pos = pos;
        self.context_menu_origin = origin;
        self.context_menu_target = target;
        // DismissEvent → 状态清理（点选动作后组件自动 emit；Esc/点外部
        // 经 on_blur → Cancel → emit——两条路都归一到这里）。
        cx.subscribe(&menu, |this, _, _: &gpui::DismissEvent, cx| {
            this.close_context_menu(cx);
        })
        .detach();
        // 键盘导航需要焦点在菜单（RightClickMenu 同款：等 deferred 渲染
        // 树链入焦点域后再聚焦，避免 contains_focused 误判闪烁）。
        let focus = menu.read(cx).focus_handle(cx).clone();
        window.on_next_frame(move |window, _cx| {
            window.on_next_frame(move |window, cx| {
                window.focus(&focus, cx);
            });
        });
        cx.notify();
    }

    /// 关闭右键菜单（Esc / 点空白 / 执行完动作）。目标随菜单一起清
    ///（#166——菜单关了，目标路径不再有效）。
    pub fn close_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.context_menu.take().is_some() {
            self.context_menu_target = None;
            cx.notify();
        }
    }

    /// 执行一个菜单动作（按 id 派发）。#166：带窗口——文件树「打开」
    /// 要聚焦编辑区（与 #165 编辑器命令面的窗口诉求同源）。
    ///
    /// 未接线的 id 只写证据通道并关菜单——**不 panic 也不静默**：
    /// 灰显项本来就点不到，能点到却没接线的项说明菜单配错了，
    /// stderr 留痕比默默吞掉好。
    pub fn activate_context_menu_item(
        &mut self,
        id: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let origin = self.context_menu_origin;
        let target = self.context_menu_target.clone();
        self.context_menu = None;
        self.context_menu_target = None;
        match (origin, id) {
            // 编辑器命令动作：统一走 run_command（键位 / 命令面板 / 图标条
            // 搜索钮同一分派路径）。#165 前经 exec_menu_command 的
            // `_ => {}` 静默吞掉 OpenFind/OpenReplace/GotoLine——菜单
            // 「查找/替换/跳转到行」是死项；现在窗口引用随点击 listener
            // 传入，全命令可达（exec_menu_command 已删——见其原址注释）。
            (MenuOrigin::Editor, "ed-select-all") => {
                self.dispatch_editor_cmd(crate::editor::commands::Cmd::SelectAll, window, cx);
            }
            (MenuOrigin::Editor, "ed-find") => {
                self.dispatch_editor_cmd(crate::editor::commands::Cmd::OpenFind, window, cx);
            }
            (MenuOrigin::Editor, "ed-replace") => {
                self.dispatch_editor_cmd(crate::editor::commands::Cmd::OpenReplace, window, cx);
            }
            (MenuOrigin::Editor, "ed-goto") => {
                self.dispatch_editor_cmd(crate::editor::commands::Cmd::GotoLine, window, cx);
            }
            (MenuOrigin::Editor, "ed-fold") => {
                self.dispatch_editor_cmd(crate::editor::commands::Cmd::ToggleFold, window, cx);
            }
            (MenuOrigin::Editor, "ed-cut") => {
                // 剪贴板动作走 Editor 的菜单方法（Cmd 枚举无这三项——
                // 它们是剪贴板动作，不是可登记的编辑器命令）。
                if let Some(editor) = self.editor().cloned() {
                    editor.update(cx, |ed, cx| ed.menu_cut(cx));
                }
            }
            (MenuOrigin::Editor, "ed-copy") => {
                if let Some(editor) = self.editor().cloned() {
                    editor.update(cx, |ed, cx| ed.menu_copy(cx));
                }
            }
            (MenuOrigin::Editor, "ed-paste") => {
                if let Some(editor) = self.editor().cloned() {
                    editor.update(cx, |ed, cx| ed.menu_paste(cx));
                }
            }
            // ----------------------------------------------------------
            // #166：文件树动作（全部接线——不允许「点了只打日志」）。
            // ----------------------------------------------------------
            (MenuOrigin::FileTree, "ft-reveal") => {
                // 揭示目标本身（目录）或所在目录（文件）——原实现恒揭示
                // 工作区根，与右键目标无关，#166 修正为目标面。
                if let Some(target) = target {
                    let reveal = if target.is_dir() {
                        target
                    } else {
                        target.parent().map(|p| p.to_path_buf()).unwrap_or(target)
                    };
                    cx.reveal_path(&reveal);
                }
            }
            (MenuOrigin::FileTree, "ft-open") => {
                // 只对文件有效（菜单构建时目录项已灰显——防御面再验一道）。
                if let Some(target) = target.filter(|p| !p.is_dir()) {
                    self.open_document(target, window, cx);
                }
            }
            (MenuOrigin::FileTree, "ft-new-file") => {
                if let Some(target) = target {
                    self.prompt_create_file(&target, window, cx);
                }
            }
            (MenuOrigin::FileTree, "ft-new-folder") => {
                if let Some(target) = target {
                    self.prompt_create_folder(&target, window, cx);
                }
            }
            (MenuOrigin::FileTree, "ft-rename") => {
                if let Some(target) = target {
                    self.prompt_rename_path(&target, window, cx);
                }
            }
            (MenuOrigin::FileTree, "ft-delete") => {
                if let Some(target) = target {
                    self.confirm_delete_path(target, window, cx);
                }
            }
            (_, other) => {
                eprintln!("context-menu: 未接线的动作 {other}");
            }
        }
        cx.notify();
    }

    /// 把命令派发到当前编辑器（无编辑器时空操作）。#165：带窗口——
    /// 与键位/命令面板同链 `Editor::run_command`（开查找条要聚焦
    /// 输入域，需要 Window）。旧 exec_menu_command 无窗路径已删：
    /// 其「菜单点击拿不到窗口」的前提不成立——on_click 的 listener
    /// 本就携带 window，只是当时被 `_window` 忽略了。
    /// #386：`pub(crate)`——rpc `run_command` 同链接入（此前直调
    /// `Editor::run_command` 绕过 Shell 分流，`run_command("save")
    /// 会跳过批注落盘）。
    pub(crate) fn dispatch_editor_cmd(
        &mut self,
        cmd: crate::editor::commands::Cmd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.editor().cloned() else {
            return;
        };
        // #386：保存是「谁负责保存」的决策——归 Shell（pdf 批注分流
        // 在此，不进 Editor）。Cmd 面（命令面板/菜单）与键位面
        // （ShellAction::Save）同源走 save_tab。
        if matches!(cmd, crate::editor::commands::Cmd::Save) {
            self.save_tab(self.active, cx);
            cx.notify();
            return;
        }
        // #422：页视图下 Ctrl+F → 页内查找条（不切视图——#297 折中
        // 升级为正式能力：文本层索引 + 高亮 + 导航见 PdfPagesView）。
        // 页视图实体缺位（error 态）→ 保底旧折中：切文本视图查找。
        if matches!(cmd, crate::editor::commands::Cmd::OpenFind)
            && self.pdf_view_mode
            && editor.read(cx).doc_kind() == "pdf"
        {
            let view = self.pdf_pages.as_ref().map(|(_, e)| e.clone());
            if let Some(view) = view {
                view.update(cx, |v, cx| v.open_search(window, cx));
                return;
            }
            self.pdf_view_mode = false;
            self.show_notice(
                "已切换到文本视图（页面视图不可用，查找走文本面）".into(),
                cx,
            );
        }
        // #389 阶段 2：xlsx 标签保存 → 编辑会话统一收口（行模型只读
        // ——直达 editor.save() 是 #386 同款假成功；#422 页视图
        // Ctrl+F 拦截同模式）。活实体保存（notice 面错误明示）或银行
        // 会话直存；Ctrl+S 键位在网格视图持焦点时由视图自身 on_key_down
        // 直通——两路同收口 save_xlsx_at / view.save。
        if matches!(cmd, crate::editor::commands::Cmd::Save)
            && editor.read(cx).doc_kind() == "xlsx"
            && let Some(tab) = self.tabs.get(self.active)
        {
            let path = tab.path.clone();
            let saved = self.save_xlsx_at(&path, cx);
            if saved {
                self.show_notice("已保存（xlsx 编辑会话）".into(), cx);
            } else {
                self.show_notice("xlsx 保存失败——见网格面板状态条".into(), cx);
            }
            return;
        }
        editor.update(cx, |ed, cx| ed.run_command(cmd, window, cx));
    }

    // ── #209：顶部菜单栏 ─────────────────────────────────────────

    /// #209：菜单构建上下文快照（`menu_bar::build_menus` 入参——灰显/
    /// 勾选的全部依据；每次打开/渲染/导航现场采集，菜单模型零状态）。
    fn menu_context(&self, cx: &App) -> menu_bar::MenuContext {
        let tab = self.tabs.get(self.active);
        menu_bar::MenuContext {
            has_document: tab.is_some(),
            // 只读/选区判定与编辑器右键菜单同判定源（is_read_only /
            // selection_points 的 start < end——dispatch_context_menu 口径）。
            read_only: tab.is_some_and(|t| t.editor.read(cx).is_read_only()),
            has_selection: tab.is_some_and(|t| {
                t.editor
                    .read(cx)
                    .selection_points()
                    .is_some_and(|(s, e)| s < e)
            }),
            font_size: self.font_size,
            theme_mode: self.theme_mode,
            left_open: self.left_open,
            git_view: self.git_view,
            // #307：「导出为 PDF」门槛（仅 docx 形态提供；md/pdf/xlsx/
            // canvas 标签上灰显）。
            active_is_docx: tab.is_some_and(|t| t.editor.read(cx).doc_kind() == "docx"),
            // #307 ②：「导出为 DOCX」门槛（仅 pdf 形态提供；文本版导出
            // 读 PDF 文本层——扫描件零文本在转换内显式拒）。
            active_is_pdf: tab.is_some_and(|t| t.editor.read(cx).doc_kind() == "pdf"),
            recent_files: self.recent.recent_files().to_vec(),
        }
    }

    /// #209：顶部菜单 `idx` 是否正展开（标题行点击的开/关判定——
    /// 标题按钮自绘保留，判定面公开给 menu_bar_view）。
    pub fn is_menu_bar_open_at(&self, idx: usize) -> bool {
        self.menu_bar_open == Some(idx)
    }

    /// #209（#385 批 4）：打开顶部菜单（标题点击——`anchor_x` = 点击 x）。
    ///
    /// - 构建 `ui::ContextMenu` 实体（键盘导航/浮层/子菜单/点外关闭全
    ///   部组件自带——与批 1 右键菜单同构）；
    /// - 订阅 `DismissEvent`（激活动作/点外/Esc → close 清态）——订阅
    ///   存字段，切菜单时 drop 旧的（迟到事件不再可达，无误关窗口）；
    /// - 两帧后聚焦菜单（deferred 树链入焦点域后再聚焦——批 1 同款）；
    /// - 右键菜单互斥关闭（旧链路保留）。
    pub fn open_menu_bar(
        &mut self,
        idx: usize,
        anchor_x: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_context_menu(cx);
        let menus = menu_bar::build_menus(&self.menu_context(cx));
        let Some(menu) = menus.get(idx) else {
            return;
        };
        let shell = cx.entity().downgrade();
        let entity = menu_bar_view::build_menu_bar_menu(menu, shell, window, cx);
        // 旧订阅先退役（drop = 取消订阅）：旧实体的迟到 DismissEvent
        // 不会误关刚开的新菜单。订阅存字段（不 detach——生命周期与
        // 实体同步，drop 即取消）。
        self.menu_bar_subscription = None;
        self.menu_bar_subscription = Some(cx.subscribe(
            &entity,
            |this, _, _: &gpui::DismissEvent, cx| {
                this.close_menu_bar_internal(cx);
            },
        ));
        self.menu_bar_open = Some(idx);
        self.menu_bar_entity = Some(entity.clone());
        self.menu_bar_anchor = gpui::Point::new(px(anchor_x), px(menu_bar_view::menu_anchor_y()));
        // 键盘导航需要焦点在菜单（deferred 渲染树链入后两帧聚焦——
        // 批 1 spawn_context_menu 同款）。
        let focus = entity.read(cx).focus_handle(cx).clone();
        window.on_next_frame(move |window, _cx| {
            window.on_next_frame(move |window, cx| {
                window.focus(&focus, cx);
            });
        });
        cx.notify();
    }

    /// #209（#385 批 4）：关闭菜单栏（Esc / 点外部 / 动作执行后）——
    /// 焦点归回活动标签（空态回 shell 根域，`focus_active` 既有口径）。
    pub fn close_menu_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu_bar_open.is_none() {
            return;
        }
        self.close_menu_bar_internal(cx);
        self.focus_active(window, cx);
        cx.notify();
    }

    /// 清态单点（DismissEvent 订阅回调用——不做焦点归位，焦点在菜单
    /// 实体上，随实体摘除自然回 shell 根域；标题点击路径的关停由
    /// `close_menu_bar` 补焦点归位）。
    fn close_menu_bar_internal(&mut self, cx: &mut Context<Self>) {
        self.menu_bar_open = None;
        self.menu_bar_entity = None;
        self.menu_bar_subscription = None;
        let _ = cx;
    }

    /// #209：执行菜单动作（条目点击 / Enter——与右键菜单
    /// `activate_context_menu_item` 同型的派发单点）。**只映射既有动作**
    /// （owner 指令）：Cmd 走 [`Self::dispatch_editor_cmd`]（键位/命令
    /// 面板/右键菜单/图标条同链）；剪贴板走 `Editor::menu_*`；shell 级
    /// 走既有方法——映射表见 `menu_bar` 模块文档。
    pub fn activate_menu_action(
        &mut self,
        action: menu_bar::MenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 先关菜单 + 焦点归位（动作可能开对话框/浮层，菜单不得残留）。
        self.menu_bar_entity = None;
        self.menu_bar_subscription = None;
        self.menu_bar_open = None;
        self.focus_active(window, cx);
        use menu_bar::MenuAction as Act;
        match action {
            Act::EditorCmd(cmd) => self.dispatch_editor_cmd(cmd, window, cx),
            // 剪贴板三件：Cmd 枚举无剪贴板项——Editor 菜单方法（右键
            // 菜单 ed-cut/ed-copy/ed-paste 同款既有路径）。
            Act::Cut => {
                if let Some(editor) = self.editor().cloned() {
                    editor.update(cx, |ed, cx| ed.menu_cut(cx));
                }
            }
            Act::Copy => {
                if let Some(editor) = self.editor().cloned() {
                    editor.update(cx, |ed, cx| ed.menu_copy(cx));
                }
            }
            Act::Paste => {
                if let Some(editor) = self.editor().cloned() {
                    editor.update(cx, |ed, cx| ed.menu_paste(cx));
                }
            }
            Act::OpenFile => self.prompt_open_file(window, cx),
            Act::OpenFolder => self.prompt_open_workspace(window, cx),
            // 序 = MenuContext.recent_files 序（菜单打开期间列表不变——
            // 模态；越界/已删除防御：不崩溃，缺失给提示）。
            Act::OpenRecent(idx) => match self.recent.recent_files().get(idx).cloned() {
                Some(path) if path.exists() => self.open_document(path, window, cx),
                Some(path) => {
                    self.show_notice(format!("文件不存在：{}", path.display()), cx);
                }
                None => {}
            },
            Act::SaveAs => self.prompt_save_as(window, cx),
            // #307 阶段①：导出为 PDF（DOCX → LibreOffice headless → PDF）。
            Act::ExportPdf => self.export_active_pdf(window, cx),
            // #307 ②功能独立项：导出为 DOCX（PDF 文本层 → office-io
            // 写面——纯 Rust 链路，无外部工具依赖）。
            Act::ExportDocx => self.export_active_docx(window, cx),
            // #507 / #240 任务②：壳面系统打印流（方案 C——Windows 打印
            // 动词 / macOS open -p / CUPS lp；设置面由系统对话框或 lp
            // 参数承载）。
            Act::Print => self.print_active_document(cx),
            // 与标题栏 × 钮同一条 cx.quit 既有路径（该路径不经
            // on_should_close 脏确认——× 钮现状同此，行为对齐不超范围
            // 修补；统一走脏确认建议另开 issue）。
            Act::Quit => cx.quit(),
            // 关于：既有提示横幅承载（层 3 可发现性验收项——版本信息
            // 面向用户可见）。
            Act::About => self.show_notice(
                format!(
                    "mang-shell {}（mdcore 桌面外壳）",
                    env!("CARGO_PKG_VERSION")
                ),
                cx,
            ),
            // 命令面板：Ctrl+Shift+P 键位臂的既有逻辑提取的公开方法
            // （editor/mod.rs toggle_command_palette——两入口共用，行为
            // 零变化）。
            Act::TogglePalette => {
                if let Some(editor) = self.editor().cloned() {
                    editor.update(cx, |ed, cx| ed.toggle_command_palette(cx));
                }
            }
            // #289-⑤：设置弹窗（查看菜单「设置」项——与左条设置钮同一
            // 条 open_settings 链）。
            Act::OpenSettings => self.open_settings(cx),
            Act::SetTheme(mode) => self.set_theme_mode(mode, cx),
            // 字号：与 Ctrl+=/-/0 键位同一换算（±1 / 重置 15；域钳制在
            // apply_font_size 收口）。
            Act::FontBigger => self.apply_font_size(px(self.font_size + 1.), cx),
            Act::FontSmaller => self.apply_font_size(px(self.font_size - 1.), cx),
            Act::FontReset => self.apply_font_size(px(15.), cx),
            Act::ToggleFileTreePanel => self.toggle_left(cx),
            Act::ToggleGitPanel => self.toggle_git(cx),
            // 置灰项不绑动作（无既有命令映射）——正常不可达（灰显挂
            // 不了 on_click、Enter 有 enabled 门）；能到达说明菜单配错，
            // stderr 留痕（右键菜单未接线兜底同款口径）。
            Act::None => eprintln!("menu-bar: 置灰项不绑动作（无既有命令映射）"),
        }
        cx.notify();
    }

    /// F1/F3：打开文件（Ctrl+O 对话框 / 文件树点击 / 最近列表共用
    /// 通道）。已开 → 激活既有标签（不重复开）；新文件 → 追加标签并
    /// 激活 + 聚焦 + 文件树高亮；失败/不支持→提示横幅（不白屏不崩溃）。
    pub fn open_document(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = self.open_document_tab(path, cx) {
            window.focus(&focus, cx);
        }
    }

    /// 打开核心（无 window——跨实体调用面：FileTree/GitPanel 点击经
    /// `Entity::update` 进入（不带 window 绑定，update_in 在任务/派生
    /// 上下文解析不到窗口），焦点句柄返回由调用方在其 window 域落焦）。
    pub fn open_document_tab(
        &mut self,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) -> Option<FocusHandle> {
        // F3：标签身份用绝对路径（CLI 相对入参 vs 对话框绝对入参不歧义）。
        let path = std::path::absolute(&path).unwrap_or(path);
        eprintln!("open: {}", path.display());
        // F3：同一文件已开 → 激活既有标签。
        if let Some(idx) = self.tabs.iter().position(|tab| tab.path == path) {
            return self.activate_tab_focus(idx, cx);
        }
        match document::open_editor(&path, cx) {
            OpenOutcome::Opened(editor) => {
                // #179（#188 接线）：挂入标签前注入右键菜单回调（document.rs
                // 的 cx 是 &mut App 拿不到 shell，构造后补注）。
                let shell_weak = cx.entity().downgrade();
                editor.update(cx, |ed, _| {
                    ed.set_menu_opener(menu_opener_for(shell_weak));
                });
                let focus = editor.read(cx).focus_handle().clone();
                let is_pdf = editor.read(cx).doc_kind() == "pdf";
                self.tabs.push(TabItem {
                    path: path.clone(),
                    editor,
                });
                self.active = self.tabs.len() - 1;
                // #571：激活编辑器变更 → 大纲面板换绑。
                self.sync_outline(cx);
                self.notice_board.clear();
                self.recent.record_file(&path);
                eprintln!(
                    "tabs: open → {} 开（active {}）",
                    self.tabs.len(),
                    self.active
                );
                // F2：文件树高亮同步（工作区打开时）。
                if let Some(tree) = &self.workspace {
                    tree.update(cx, |tree, cx| {
                        tree.set_active_file(Some(path.clone()));
                        cx.notify();
                    });
                }
                // #289-④：pdf 标签 → 页视图落位（位图渲染主显示面）。
                self.ensure_pdf_pages(cx);
                // #389 阶段 1：xlsx 标签 → 网格视图落位（真网格主显示面）。
                self.ensure_xlsx_grid(cx);
                cx.notify();
                // #422：页视图态 pdf 标签——主显示面是页视图（编辑器
                // 不在树内，其焦点句柄游离）：焦点句柄改返页视图，调用
                // 方落焦后键事件可达（Ctrl+F 视图级拦截）。
                if self.pdf_view_mode
                    && is_pdf
                    && let Some((_, view)) = self.pdf_pages.as_ref()
                {
                    return Some(view.read(cx).focus_handle(cx).clone());
                }
                Some(focus)
            }
            OpenOutcome::Unsupported { path, message } => {
                self.show_notice(format!("未打开 {}：{message}", path.display()), cx);
                None
            }
            OpenOutcome::Failed { path, message } => {
                self.show_notice(format!("打开失败（{}）：{message}", path.display()), cx);
                None
            }
        }
    }

    /// F3：激活标签（点击标签 / 重复打开既有文件 / 循环切换落地）。
    pub fn activate_tab(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = self.activate_tab_focus(idx, cx) {
            window.focus(&focus, cx);
        }
    }

    /// 激活核心（无 window——焦点句柄返回，调用方落焦）。
    fn activate_tab_focus(&mut self, idx: usize, cx: &mut Context<Self>) -> Option<FocusHandle> {
        let tab = self.tabs.get(idx)?.clone();
        self.active = idx;
        // #571：激活编辑器变更 → 大纲面板换绑。
        self.sync_outline(cx);
        // #289-④：pdf 标签 → 页视图随标签落位（同路径复用，异路径重建）。
        self.ensure_pdf_pages(cx);
        // #389 阶段 1：xlsx 标签 → 网格视图随标签落位（同路径复用，异路径重建）。
        self.ensure_xlsx_grid(cx);
        let focus = tab.editor.read(cx).focus_handle().clone();
        cx.notify();
        Some(focus)
    }

    /// F3/F4：关闭当前标签（Ctrl+W）——脏标签先弹确认框（保存/不保存/
    /// 取消）；干净标签直接关。#386：pdf 批注脏（页视图未落盘）计入
    /// 脏判定（`tab_is_modified` 同源）。
    pub fn close_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        // F4：脏标签 → 确认框拦截（#389 阶段 2 xlsx 会话脏 + #386
        // pdf 批注脏——tab_is_modified 已统一收口）。
        if self.tab_is_modified(self.active, cx).unwrap_or(false) {
            let name = self.tabs[self.active]
                .editor
                .read(cx)
                .display_name()
                .to_string();
            eprintln!("confirm: 关闭未保存标签（{name}）");
            self.confirm = Some(ConfirmState {
                message: format!("{name} 有未保存的修改"),
                detail: "关闭前保存？".to_string(),
                action: ConfirmAction::CloseTab { idx: self.active },
                selected: 0,
                buttons: CONFIRM_BUTTONS.to_vec(),
            });
            // 模态接管焦点（编辑器隔离——可打印键不再入 buffer）。
            window.focus(&self.confirm_focus.clone(), cx);
            cx.notify();
            return;
        }
        self.close_tab_at(self.active, window, cx);
    }

    /// F3/F4：关闭指定标签（无确认——确认面在 [`Self::close_active_tab`]）。
    fn close_tab_at(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        let old_len = self.tabs.len();
        if idx >= old_len {
            return;
        }
        // #289-④：被关标签若持有页视图，一并丢弃（缓存随实体释放）。
        let closed_path = self.tabs[idx].path.clone();
        if let Some((owned, _)) = &self.pdf_pages {
            if *owned == closed_path {
                self.pdf_pages = None;
            }
        }
        // #389 阶段 1：网格视图同款生命周期（关标签丢弃）。
        if let Some((owned, _)) = &self.xlsx_grid {
            if *owned == closed_path {
                self.xlsx_grid = None;
            }
        }
        // #389 阶段 2：会话银行同路径丢弃（关闭确认已保证 dirty 先
        // 保存/放弃——「不保存」选择到此处即弃；无确认路径（无脏）弃
        // 会话无损失）。
        self.xlsx_sessions.remove(&closed_path);
        self.tabs.remove(idx);
        self.active = if idx < self.active {
            self.active - 1
        } else if idx == self.active {
            tab_bar::active_after_close(old_len, self.active)
        } else {
            self.active
        };
        // #571：关签后激活面可能变更 → 大纲面板换绑（同编辑器复用）。
        self.sync_outline(cx);
        eprintln!(
            "tabs: close → {} 开（active {}）",
            self.tabs.len(),
            self.active
        );
        // #289-④：新活动标签若是 pdf → 页视图随标签重建。
        self.ensure_pdf_pages(cx);
        // #389 阶段 1：新活动标签若是 xlsx → 网格视图随标签重建。
        self.ensure_xlsx_grid(cx);
        // 焦点跟到新活动标签（空态回退 shell 根域）。
        self.focus_active(window, cx);
        cx.notify();
    }

    /// F4：确认框按钮激活（键盘 Enter / 鼠标点击）。`choice`：
    /// 0 保存 / 1 不保存 / 2 取消。
    fn resolve_confirm(&mut self, choice: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(confirm) = self.confirm.take() else {
            return;
        };
        match confirm.action {
            ConfirmAction::CloseTab { idx } => match choice {
                0 => {
                    // 保存后关（保存失败不关——不静默丢内容）。#386：
                    // 走 Shell 分流（pdf 批注增量落盘在此发生）；
                    // 只 Saved/Unmodified 才关。
                    let outcome = self.save_tab(idx, cx);
                    if matches!(outcome, SaveOutcome::Saved | SaveOutcome::Unmodified) {
                        self.close_tab_at(idx, window, cx);
                    }
                }
                1 => self.close_tab_at(idx, window, cx),
                _ => {}
            },
            ConfirmAction::Quit => match choice {
                0 => {
                    // #386：逐标签走 Shell 分流（pdf 批注一并落盘；
                    // 失败已 toast 示警——与旧编辑器路径同语义：
                    // 尽力保存后退出）。
                    for i in 0..self.tabs.len() {
                        self.save_tab(i, cx);
                    }
                    cx.quit();
                }
                1 => cx.quit(),
                _ => {}
            },
            // #166：删除确认（0 删除 / 1 取消——默认停取消侧）。
            ConfirmAction::DeletePath { path } => match choice {
                0 => self.delete_path_confirmed(&path, window, cx),
                _ => {}
            },
        }
        self.focus_active(window, cx);
        cx.notify();
    }

    /// 焦点归回当前活动标签的编辑器（确认框关闭后；空态回退 shell
    /// 根域——键位不聋）。
    fn focus_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.tabs.get(self.active) {
            Some(tab) => {
                // #422：页视图态 pdf 标签——焦点落页视图（编辑器不在
                // 树内，其句柄游离则键事件死在合成根）。
                if self.pdf_view_mode
                    && tab.editor.read(cx).doc_kind() == "pdf"
                    && let Some((_, view)) = self.pdf_pages.as_ref()
                {
                    let focus = view.read(cx).focus_handle(cx).clone();
                    window.focus(&focus, cx);
                    return;
                }
                // #389 阶段 2：网格视图态 xlsx 标签——焦点落网格视图
                //（编辑键位面：双击后的 Enter/Esc/输入经视图接收）。
                if self.xlsx_grid_view_mode
                    && tab.editor.read(cx).doc_kind() == "xlsx"
                    && let Some((_, view)) = self.xlsx_grid.as_ref()
                {
                    let focus = view.read(cx).focus_handle(cx).clone();
                    window.focus(&focus, cx);
                    return;
                }
                let focus = tab.editor.read(cx).focus_handle().clone();
                window.focus(&focus, cx);
            }
            None => window.focus(&self.root_focus, cx),
        }
    }

    /// F4：另存为落位（对话框回调 / --smoke-save-as 钩子共用）——
    /// 编辑器写盘 + 身份迁移 + 标签路径更新 + 文件树高亮同步。
    /// #389 阶段 2：xlsx 标签的另存为**不在本阶段面**（会话迁移 + 双
    /// 视图重开 + 标签身份三联动——诚实拦截为明示提示，不做静默假
    /// 成功：行模型 save_as 只读返回 false）。
    pub fn save_active_as(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let path = std::path::absolute(&path).unwrap_or(path);
        let is_xlsx = self
            .tabs
            .get(self.active)
            .is_some_and(|tab| tab.editor.read(cx).doc_kind() == "xlsx");
        if is_xlsx {
            self.show_notice(
                "xlsx 另存为属后续阶段（当前可编辑保存：Ctrl+S / 文件 → 保存）".into(),
                cx,
            );
            return;
        }
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        // #386：三态消费——只读态拒绝/写盘失败显式示警（旧裸 bool
        // 的 false 静默返回 = 用户以为存上了）。
        let outcome = tab.editor.update(cx, |ed, _| ed.save_as(path.clone()));
        if !matches!(outcome, SaveOutcome::Saved | SaveOutcome::Unmodified) {
            self.show_notice(
                format!(
                    "另存失败（{}）——只读态拒绝或写盘错误，详见应用日志",
                    path.display()
                ),
                cx,
            );
            return;
        }
        self.tabs[self.active].path = path.clone();
        if let Some(tree) = &self.workspace {
            tree.update(cx, |tree, cx| {
                tree.set_active_file(Some(path.clone()));
                cx.notify();
            });
        }
        cx.notify();
    }

    // ------------------------------------------------------------------
    // #134 任务 1「RPC 文件系统命令」：工作区沙箱边界 + 标签的
    // 磁盘权威面（重载 / 改名迁移）。
    // ------------------------------------------------------------------

    /// #134：工作区根目录（RPC FS 命令的沙箱边界；无工作区 = `None`
    /// ——FS 命令一律拒绝）。返回的是 FileTree 持有的原始根路径，
    /// 符号链接摊平（canonicalize）由 rpc::resolve_sandboxed 完成。
    pub fn workspace_root(&self, cx: &Context<Self>) -> Option<PathBuf> {
        self.workspace
            .as_ref()
            .map(|tree| tree.read(cx).root().to_path_buf())
    }

    /// #134：重载指定标签（RPC `write_file` 写盘后把 Buffer 同步到磁盘
    /// 现状——`Editor::reload_from_disk` 完整编辑链）。索引越界 / 编辑器
    /// 拒绝（只读渲染态 / docx 行模型态）返回 false。
    pub fn reload_tab(&mut self, idx: usize, cx: &mut Context<Self>) -> bool {
        let Some(tab) = self.tabs.get(idx) else {
            return false;
        };
        let ok = tab.editor.update(cx, |ed, cx| ed.reload_from_disk(cx));
        if ok {
            cx.notify();
        }
        ok
    }

    /// #134：标签可否从磁盘重载（write_file 的门槛探针——只读渲染态
    /// / docx 行模型态 false）。
    pub fn tab_can_reload(&self, idx: usize, cx: &Context<Self>) -> bool {
        self.tabs
            .get(idx)
            .is_some_and(|tab| tab.editor.read(cx).can_reload_from_disk())
    }

    /// #134：指定标签的编辑器实体（batch_replace 对**非活动**标签的
    /// Buffer 操作面——`editor()` 只给活动标签）。
    pub fn tab_editor(&self, idx: usize) -> Option<&Entity<Editor>> {
        self.tabs.get(idx).map(|tab| &tab.editor)
    }

    /// #134：rename 后的标签身份迁移（磁盘已改名——按「索引 → 新路径」
    /// 计划逐一更新 `TabItem.path` + `Editor::adopt_path`；不写盘、不动
    /// Buffer）。计划由调用方在 rename **前**采集（旧路径还能 canonicalize
    /// 时）。返回迁移的标签数。
    pub fn apply_tab_renames(
        &mut self,
        plan: &[(usize, PathBuf)],
        cx: &mut Context<Self>,
    ) -> usize {
        let mut n = 0;
        for &(idx, ref new_path) in plan {
            if let Some(tab) = self.tabs.get_mut(idx) {
                tab.path = new_path.clone();
                tab.editor
                    .update(cx, |ed, _| ed.adopt_path(new_path.clone()));
                n += 1;
            }
        }
        if n > 0 {
            cx.notify();
        }
        n
    }

    // ------------------------------------------------------------------
    // #166：文件树右键动作——fs 变更面（新建 / 重命名 / 删除 + 树同步）。
    // 对话框统一走 gpui `prompt_for_new_path`（与另存为同源——闭包无
    // 原生「输入名字」轻量框，命名面借用系统保存对话框）。
    // ------------------------------------------------------------------

    /// #166 ft-new-file：新建文件对话框（落点 = 目标目录 / 文件所在目录）。
    fn prompt_create_file(&mut self, target: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let directory = dir_for_new(target);
        let rx = cx.prompt_for_new_path(&directory, None);
        cx.spawn_in(window, async move |this, cx| match rx.await {
            Ok(Ok(Some(path))) => {
                let _ = this.update_in(cx, |shell, window, cx| {
                    shell.create_file(path, window, cx);
                });
            }
            _ => {}
        })
        .detach();
    }

    /// #166 ft-new-folder：新建文件夹对话框（同源——返回路径即文件夹名，
    /// 对话框按「保存」语义呈现，落盘面按目录建）。
    fn prompt_create_folder(&mut self, target: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let directory = dir_for_new(target);
        let rx = cx.prompt_for_new_path(&directory, None);
        cx.spawn_in(window, async move |this, cx| match rx.await {
            Ok(Ok(Some(path))) => {
                let _ = this.update(cx, |shell, cx| shell.create_folder(path, cx));
            }
            _ => {}
        })
        .detach();
    }

    /// #166 ft-rename：重命名对话框（建议名 = 当前名；目标可移出原目录
    /// ——对话框返回的完整路径即新路径）。
    fn prompt_rename_path(&mut self, target: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let directory = target
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let suggested = target.file_name().map(|n| n.to_string_lossy().into_owned());
        let old = target.to_path_buf();
        let rx = cx.prompt_for_new_path(&directory, suggested.as_deref());
        cx.spawn_in(window, async move |this, cx| match rx.await {
            Ok(Ok(Some(new))) => {
                let _ = this.update(cx, |shell, cx| shell.rename_path(old, new, cx));
            }
            _ => {}
        })
        .detach();
    }

    /// 新建文件落盘（空文件）+ 树同步 + 打开（F1 通道——聚焦/高亮复用）。
    fn create_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        match std::fs::write(&path, "") {
            Ok(()) => {
                eprintln!("file-tree: 新建文件 {}", path.display());
                self.sync_tree_after_fs(&path, false, cx);
                self.open_document(path, window, cx);
            }
            Err(err) => {
                self.show_notice(format!("新建文件失败（{}）：{err}", path.display()), cx);
            }
        }
    }

    /// 新建文件夹落盘（递归建路）+ 树同步。
    fn create_folder(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match std::fs::create_dir_all(&path) {
            Ok(()) => {
                eprintln!("file-tree: 新建文件夹 {}", path.display());
                self.sync_tree_after_fs(&path, false, cx);
            }
            Err(err) => {
                self.show_notice(format!("新建文件夹失败（{}）：{err}", path.display()), cx);
            }
        }
    }

    /// 重命名落盘 + 打开标签路径迁移（apply_tab_renames 既有通道）+
    /// 树同步（旧位丢弃 / 新位展开）+ 高亮跟随。
    fn rename_path(&mut self, old: PathBuf, new: PathBuf, cx: &mut Context<Self>) {
        if new == old {
            return;
        }
        match std::fs::rename(&old, &new) {
            Ok(()) => {
                eprintln!("file-tree: 重命名 {} → {}", old.display(), new.display());
                let plan: Vec<(usize, PathBuf)> = self
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.path == old)
                    .map(|(i, _)| (i, new.clone()))
                    .collect();
                self.apply_tab_renames(&plan, cx);
                self.sync_tree_after_fs(&old, true, cx);
                self.sync_tree_after_fs(&new, false, cx);
                self.sync_tree_active(cx);
            }
            Err(err) => {
                self.show_notice(
                    format!("重命名失败（{} → {}）：{err}", old.display(), new.display()),
                    cx,
                );
            }
        }
    }

    /// #166 ft-delete：删除确认（二选一——默认停「取消」：危险操作
    /// 默认安全侧；Enter 直接确认需显式左移选中）。
    fn confirm_delete_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let kind = if path.is_dir() { "文件夹" } else { "文件" };
        eprintln!("confirm: 删除{kind}（{}）", path.display());
        self.confirm = Some(ConfirmState {
            message: format!("删除{kind}「{name}」"),
            detail: "永久删除，不可恢复（不进回收站）。".to_string(),
            action: ConfirmAction::DeletePath { path },
            selected: 1,
            buttons: DELETE_BUTTONS.to_vec(),
        });
        window.focus(&self.confirm_focus.clone(), cx);
        cx.notify();
    }

    /// 确认后的删除执行（resolve_confirm 的 DeletePath 臂调用）：
    /// 落盘删除 → 关闭指向该路径（或其下）的标签（内容已不存在，
    /// 不再弹保存确认）→ 树同步。
    fn delete_path_confirmed(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let result = if path.is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        };
        match result {
            Ok(()) => {
                eprintln!("file-tree: 删除 {}", path.display());
                // 从高往低调用避免序号漂移。
                let mut i = self.tabs.len();
                while i > 0 {
                    i -= 1;
                    if self.tabs[i].path.starts_with(path) {
                        self.close_tab_no_confirm(i, window, cx);
                    }
                }
                self.sync_tree_after_fs(path, true, cx);
                self.sync_tree_active(cx);
            }
            Err(err) => {
                self.show_notice(format!("删除失败（{}）：{err}", path.display()), cx);
            }
        }
    }

    /// fs 变更后的树同步：`changed` = 变更路径；`removed` = 已不存在
    /// （丢弃展开态/缓存）否则展开到目标使其可见。父目录清单一律重读
    /// （先刷新缓存再展开——expand_to 的 ensure_listing 不覆盖既有缓存）。
    fn sync_tree_after_fs(&mut self, changed: &Path, removed: bool, cx: &mut Context<Self>) {
        if let Some(ws) = self.workspace.as_ref().cloned() {
            ws.update(cx, |tree, cx| {
                if let Some(parent) = changed.parent() {
                    tree.reload(parent);
                }
                if removed {
                    tree.forget_subtree(changed);
                } else {
                    tree.expand_to(changed);
                }
                cx.notify();
            });
        }
    }

    /// 树高亮与当前活动标签对齐（删除/重命名后路径面可能已变）。
    fn sync_tree_active(&mut self, cx: &mut Context<Self>) {
        let active = self.tabs.get(self.active).map(|t| t.path.clone());
        if let Some(ws) = self.workspace.as_ref().cloned() {
            ws.update(cx, |tree, _| tree.set_active_file(active));
        }
    }

    /// #307 阶段①：导出活动 DOCX 为 PDF（菜单「文件 → 导出为 PDF」）。
    ///
    /// 流：docx 门槛 → 脏位拦截（导出以磁盘内容为准——先保存再导出，
    /// 不静默用旧盘面）→ LO 探测（缺席 = 原生对话框指引，#185 兜底
    /// 通道；不静默失败）→ 输出路径对话框（建议名 = 当前名去扩展
    /// 加 .pdf）→ 后台转换（读盘 → LO headless → 写盘——UI 不卡）
    /// → 成功/失败通知横幅（失败带指引文案）。
    fn export_active_pdf(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if tab.editor.read(cx).doc_kind() != "docx" {
            self.show_notice(
                "导出为 PDF 仅支持 DOCX 文档（当前标签不是 .docx）。".to_string(),
                cx,
            );
            return;
        }
        if tab.editor.read(cx).is_modified() {
            self.show_notice(
                "文档有未保存修改——请先保存（Ctrl+S）再导出（导出以磁盘内容为准）。".to_string(),
                cx,
            );
            return;
        }
        let src = tab.path.clone();
        // LO 探测：缺席走原生错误对话框（zenity/kdialog/MessageBox 族，
        // #185 同通道——比通知横幅更醒目，符合「显式指引」验收）。
        if mdc_office_io::convert::detect_libreoffice().is_none() {
            let err = mdc_office_io::convert::ExportPdfError::NoLibreOffice;
            let shown = crate::native_dialog::show_error(&crate::native_dialog::DialogSpec {
                title: "导出 PDF 需要 LibreOffice".to_string(),
                message: format!("{}\n\n{}", err, err.guidance()),
            });
            if !shown {
                // 无对话框工具（无桌面会话）——横幅兜底，不静默。
                self.show_notice(
                    format!("导出 PDF 需要 LibreOffice（未探测到）。{}", err.guidance()),
                    cx,
                );
            }
            return;
        }
        let directory = src
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let suggested = src
            .file_stem()
            .map(|n| format!("{}.pdf", n.to_string_lossy()));
        let rx = cx.prompt_for_new_path(&directory, suggested.as_deref());
        cx.spawn_in(window, async move |this, cx| match rx.await {
            Ok(Ok(Some(out_path))) => {
                // 后台转换：读盘 → LO → 写盘（IO + 外部进程全离主线程）。
                let src_clone = src.clone();
                let out_clone = out_path.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let docx = std::fs::read(&src_clone)
                            .map_err(|e| format!("读取源文件失败：{e}"))?;
                        let pdf = mdc_office_io::convert::docx_to_pdf(&docx)
                            .map_err(|e| format!("{e}\n{}", e.guidance()))?;
                        std::fs::write(&out_clone, &pdf)
                            .map_err(|e| format!("写出 PDF 失败：{e}"))?;
                        Ok::<(), String>(())
                    })
                    .await;
                let _ = this.update_in(cx, |shell, _window, cx| match result {
                    Ok(()) => shell.show_notice(format!("已导出 PDF：{}", out_path.display()), cx),
                    Err(message) => {
                        shell.show_notice(format!("导出 PDF 失败：{message}"), cx);
                    }
                });
            }
            Ok(Ok(None)) => {}
            Ok(Err(err)) => {
                let message = format!("导出对话框不可用：{err}");
                let _ = this.update_in(cx, |shell, _window, cx| {
                    shell.show_notice(message, cx);
                });
            }
            Err(_canceled) => {}
        })
        .detach();
    }

    /// #507 / #240 任务②：打印活动标签的 PDF 文档（菜单「打印…」的
    /// 分发目标）。
    ///
    /// 最小闭环（方案 C——PR #256 同层恢复）：交系统打印——Linux 走
    /// CUPS `lp`（缺省设置 = 全部页/1 份/实际大小；设置面扩展位见
    /// `print::PrintSettings`），Windows/macOS 走系统打印对话框。
    /// 非 PDF 文档给可操作提示（不静默也不崩溃）；无打印机环境
    /// （CI/沙箱）后端缺席 = typed 错误横幅——诚实报告，不伪造成功。
    fn print_active_document(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else {
            self.show_notice("无打开文档——先打开一个 PDF 再打印".into(), cx);
            return;
        };
        if tab.editor.read(cx).doc_kind() != "pdf" {
            self.show_notice("打印仅支持 PDF 文档（当前标签不是 .pdf）。".into(), cx);
            return;
        }
        let path = tab.path.clone();
        let doc_name = path
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let job = crate::print::PrintJob {
            doc_name: &doc_name,
            file: &path,
            settings: crate::print::PrintSettings::default(),
        };
        match crate::print::print_pdf(&job) {
            Ok(outcome) => self.show_notice(format!("已送打印（{}）", outcome.describe()), cx),
            Err(err) => self.show_notice(format!("打印失败：{err}"), cx),
        }
    }

    /// #307 ②功能独立项：导出为 DOCX（文本版）——PDF 标签专用。
    ///
    /// 镜像 [`Self::export_active_pdf`] 模式（#339）：磁盘内容为准
    /// （标注增量 flush 落盘后即磁盘态）→ 后台读盘 → 引擎提取 +
    /// office-io 写面 → 写盘。**无外部工具依赖**（pdfium 是产品自身
    /// 引擎——与 PDF 视图同源绑定）；扫描件零文本 / pdfium 缺席 =
    /// typed 错误显式横幅（不静默）。
    fn export_active_docx(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if tab.editor.read(cx).doc_kind() != "pdf" {
            self.show_notice(
                "导出为 DOCX 仅支持 PDF 文档（当前标签不是 .pdf）。".to_string(),
                cx,
            );
            return;
        }
        let src = tab.path.clone();
        if !src.exists() {
            self.show_notice(
                format!("源文件不在磁盘上：{}（先确保文件已保存）", src.display()),
                cx,
            );
            return;
        }
        let directory = src
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let suggested = src
            .file_stem()
            .map(|n| format!("{}.docx", n.to_string_lossy()));
        let rx = cx.prompt_for_new_path(&directory, suggested.as_deref());
        cx.spawn_in(window, async move |this, cx| match rx.await {
            Ok(Ok(Some(out_path))) => {
                // 后台转换：读盘 → 引擎提取 + 写面 → 写盘（pdfium FFI
                // 全离主线程——与渲染工人同一串行化语义）。
                let src_clone = src.clone();
                let out_clone = out_path.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let pdf = std::fs::read(&src_clone)
                            .map_err(|e| format!("读取源文件失败：{e}"))?;
                        let docx = crate::pdf_export_docx::pdf_to_docx_bytes(&pdf)
                            .map_err(|e| e.to_string())?;
                        std::fs::write(&out_clone, &docx)
                            .map_err(|e| format!("写出 DOCX 失败：{e}"))?;
                        Ok::<(), String>(())
                    })
                    .await;
                let _ = this.update_in(cx, |shell, _window, cx| match result {
                    Ok(()) => {
                        shell.show_notice(
                            format!("已导出 DOCX（文本版）：{}", out_path.display()),
                            cx,
                        );
                    }
                    Err(message) => {
                        shell.show_notice(format!("导出 DOCX 失败：{message}"), cx);
                    }
                });
            }
            Ok(Ok(None)) => {}
            Ok(Err(err)) => {
                let message = format!("导出对话框不可用：{err}");
                let _ = this.update_in(cx, |shell, _window, cx| {
                    shell.show_notice(message, cx);
                });
            }
            Err(_canceled) => {}
        })
        .detach();
    }

    /// F4：Ctrl+Shift+S 另存为对话框（prompt_for_new_path——
    /// 建议名取当前文件名；目录取当前文件所在目录，缺省工作区根/家目录）。
    fn prompt_save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (directory, suggested) = match self.tabs.get(self.active) {
            Some(tab) => (
                tab.path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default()),
                tab.path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned()),
            ),
            None => return,
        };
        let rx = cx.prompt_for_new_path(&directory, suggested.as_deref());
        cx.spawn_in(window, async move |this, cx| match rx.await {
            Ok(Ok(Some(path))) => {
                let _ = this.update(cx, |shell, cx| shell.save_active_as(path, cx));
            }
            Ok(Ok(None)) => {}
            Ok(Err(err)) => {
                let message = format!("另存为对话框不可用：{err}");
                let _ = this.update_in(cx, |shell, window, cx| {
                    shell.show_notice(message, cx);
                });
            }
            Err(_canceled) => {}
        })
        .detach();
    }

    /// #182：设置主题模式（设置面板三态钮触发——立即应用 + 持久化；
    /// 与 Ctrl+T 循环同一生效链，不重写主题逻辑）。
    pub fn set_theme_mode(&mut self, mode: &'static str, cx: &mut Context<Self>) {
        if self.theme_mode == mode {
            return;
        }
        self.theme_mode = mode;
        theme_switch::apply_mode(mode, &self.base_theme, cx);
        crate::settings::save_theme_mode(mode);
        eprintln!("theme: {}（{mode}）", theme_switch::mode_label(mode));
        cx.notify();
    }

    /// #182：设置缩进单位（设置面板三态钮触发——持久化 + 应用到全部
    /// 已开标签；新建标签从 settings 读，两处同源）。
    pub fn set_indent_unit_all(&mut self, unit: IndentUnit, cx: &mut Context<Self>) {
        if self.indent_unit == unit {
            return;
        }
        self.indent_unit = unit;
        crate::settings::save_indent_unit(unit.setting_key());
        for tab in &self.tabs {
            tab.editor.update(cx, |ed, _| ed.set_indent_unit(unit));
        }
        eprintln!("indent-unit: {}", unit.setting_key());
        cx.notify();
    }

    /// #182：重置左右栏宽度（设置面板「重置默认」钮触发——恢复缺省宽
    /// + 持久化；与拖拽收尾 finish_panel_drag 同一存储面）。
    pub fn reset_panel_widths(&mut self, cx: &mut Context<Self>) {
        self.left_width = panels::LEFT_PANEL_WIDTH;
        self.right_width = panels::RIGHT_PANEL_WIDTH;
        let (left, right) = (f32::from(self.left_width), f32::from(self.right_width));
        crate::settings::save_panel_widths(left, right);
        eprintln!("panel-reset: left {left} right {right} (persist)");
        cx.notify();
    }

    /// F6：循环主题（Ctrl+T——system → light → dark → system；
    /// 应用 + 持久化 + 通知）。
    pub fn cycle_theme(&mut self, cx: &mut Context<Self>) {
        let next = theme_switch::next_mode(self.theme_mode);
        self.theme_mode = next;
        theme_switch::apply_mode(next, &self.base_theme, cx);
        crate::settings::save_theme_mode(next);
        eprintln!("theme: {}（{next}）", theme_switch::mode_label(next));
        cx.notify();
    }

    /// F6：设置编辑器字号（应用到全部标签 + 持久化；域外 clamp 由
    /// Editor 收口）。
    pub fn apply_font_size(&mut self, size: gpui::Pixels, cx: &mut Context<Self>) {
        let size_f32 = size.as_f32().clamp(8., 32.);
        self.font_size = size_f32;
        eprintln!("font-size: {size_f32}px");
        for tab in &self.tabs {
            tab.editor
                .update(cx, |ed, cx| ed.set_font_size(px(size_f32), cx));
        }
        crate::settings::save_font_size(size_f32);
        cx.notify();
    }

    /// F3：循环切换标签（Ctrl+Tab 下一 / Ctrl+Shift+Tab 上一；
    /// wrap；单标签无操作）。
    pub fn cycle_tab(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let next = tab_bar::cycle_active(self.tabs.len(), self.active, forward);
        if next == self.active {
            return;
        }
        eprintln!(
            "tabs: cycle → active {}/{}（{}）",
            next + 1,
            self.tabs.len(),
            if forward { "下一" } else { "上一" }
        );
        self.activate_tab(next, window, cx);
    }

    /// F2：打开文件夹作为工作区（Ctrl+K Ctrl+O 和弦 / 启动记忆恢复
    /// / --smoke-workspace 钩子共用通道）——文件树实体落位 + 记忆落盘
    /// + 状态栏路径。落盘前 absolutize（相对路径记忆在 cwd 变化后失效）。
    pub fn open_workspace(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        // 相对路径 → 绝对（不解析符号链接——展示口径保留；失败原样）。
        let root = std::path::absolute(&root).unwrap_or(root);
        eprintln!("workspace: {}", root.display());
        let shell = cx.entity().downgrade();
        let tree =
            cx.new(|cx| FileTree::new(root.clone(), Self::file_tree_hooks(shell.clone()), cx));
        self.workspace = Some(tree);
        // F7：git 面板（工作区打开即建——非 git 目录/缺 git 走提示态，
        // 不崩溃不拦截）。
        let git = cx.new(|cx| GitPanel::new(root.clone(), Self::git_panel_hooks(shell), cx));
        self.git_panel = Some(git);
        crate::settings::remember_workspace(&root);
        self.recent.record_workspace(&root);
        cx.notify();
    }

    /// F2：Ctrl+K Ctrl+O 文件夹选择对话框（directories 模式；取消/
    /// portal 缺失口径同 F1）。
    fn prompt_open_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("打开文件夹（工作区）".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            match rx.await {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        let _ = this.update(cx, |shell, cx| shell.open_workspace(path, cx));
                    }
                }
                // 用户取消——无操作。
                Ok(Ok(None)) => {}
                Ok(Err(err)) => {
                    let message = format!("文件夹选择器不可用：{err}");
                    let _ = this.update(cx, |shell, cx| {
                        shell.show_notice(message, cx);
                    });
                }
                Err(_canceled) => {}
            }
        })
        .detach();
    }

    /// F1：Ctrl+O 系统文件选择对话框（gpui `prompt_for_paths`——
    /// Windows IFileOpenDialog / macOS NSOpenPanel / Linux xdg-desktop-
    /// portal；取消 = `None` 无操作，portal 缺失 = Err 提示横幅）。
    fn prompt_open_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("打开文件".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            match rx.await {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        let _ = this.update_in(cx, |shell, window, cx| {
                            shell.open_document(path, window, cx);
                        });
                    }
                }
                // 用户取消——无操作（验收：取消不崩溃）。
                Ok(Ok(None)) => {}
                Ok(Err(err)) => {
                    let message = format!("文件选择器不可用：{err}");
                    let _ = this.update(cx, |shell, cx| {
                        shell.show_notice(message, cx);
                    });
                }
                // oneshot 通道关闭（窗口已释放）——无操作。
                Err(_canceled) => {}
            }
        })
        .detach();
    }

    /// 展示提示横幅（5 秒后自动清除——代际匹配防旧定时误清新消息）。
    fn show_notice(&mut self, message: String, cx: &mut Context<Self>) {
        let generation = self.notice_board.show(message);
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            if this
                .update(cx, |shell, cx| {
                    if shell.notice_board.dismiss(generation) {
                        cx.notify();
                    }
                })
                .is_err()
            {
                // 实体已释放——定时器自然作废。
            }
        })
        .detach();
    }
}

impl ShellApp {
    /// 标题栏文案：`mang — 文件名`（无文档仅 `mang`）。
    fn title_with(&self, cx: &gpui::App) -> String {
        match self.editor() {
            Some(ed) => format!("mang — {}", ed.read(cx).display_name()),
            None => "mang".to_string(),
        }
    }
}

impl Render for ShellApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // #164 1c：首帧计时（首个 render = 首帧呈现；消费后恒 None——
        // 后续 render 零开销）。红线 <2s（#68 设计口径）。
        if let Some(since) = self.perf_first_frame.take() {
            crate::perf::stage("first-frame", since);
        }
        let colors = cx.theme().colors().clone();
        let title = self.title_with(cx);
        // #209：菜单栏数据（每次渲染重建——结构随文档/主题/最近列表快照
        // 变化；标题行与浮层共用一份，构建成本 = 小字符串拼装）。
        let menus = menu_bar::build_menus(&self.menu_context(cx));
        // G4：面板拖拽收尾监听用的实体句柄（canvas paint 闭包捕获——
        // on_mouse_event 只能在 paint 期调用，render 期直调会 panic，
        // 与 editor 的 ime-hook canvas 同模式接线）。
        let shell_entity = cx.entity().clone();
        v_flex()
            .id("shell-root")
            .size_full()
            .track_focus(&self.root_focus)
            .bg(colors.background)
            .text_color(colors.text)
            .font_family("Noto Sans SC")
            // F1/F2/F3：shell 级键钩子。空态（无编辑区焦点）时键事件
            // 派发到 root 节点；编辑态编辑器不消费的组合冒泡至此
            // （dispatch_key 对未知 ctrl 组合零处理零拦截；tab 键臂
            // F3 起带 !ctrl 守卫、只读门禁放行 tab+ctrl）。
            //
            // 命令集：Ctrl+O（F1 文件打开）；Ctrl+K Ctrl+O 和弦（F2
            // 打开文件夹工作区——2s 窗口）；Ctrl+W（F3 关标签）；
            // Ctrl+Tab / Ctrl+Shift+Tab（F3 循环切换）。
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let ks = &event.keystroke;
                let ctrl = ks.modifiers.control || ks.modifiers.platform;
                let key = ks.key.to_lowercase();

                // #209（#385 批 4）：菜单栏 Esc 兑底关闭。菜单焦点在
                // `ui::ContextMenu` 时组件自身的 Cancel 已处理（→
                // DismissEvent → close_menu_bar_internal）；此处保留作为
                // 焦点不在菜单时的兑底（防御：键盘焦点被别处夺走时 Esc
                // 仍可关——批 1 右键菜单同款口径）。其余导航键（↑↓/
                // Enter/←→）由组件内建，shell 不再拦截。
                if key == "escape" && this.menu_bar_open.is_some() {
                    this.close_menu_bar(window, cx);
                    return;
                }

                // #166（#385 批 1）：右键菜单 Esc 关闭。菜单焦点在
                // `ui::ContextMenu` 时组件自身的 Cancel 已处理（→
                // DismissEvent → close_context_menu）；此处保留作为
                // 焦点不在菜单时的兜底（防御：键盘焦点被别处夺走时
                // Esc 仍可关）。菜单与确认框互斥——ft-delete 开确认时
                // 菜单已关。
                if key == "escape" && this.context_menu.is_some() {
                    this.close_context_menu(cx);
                    return;
                }

                // #289-⑤：设置弹窗 Esc 关闭（与点遮罩/关闭钮同一条
                // close_settings 链；与确认框互斥——两弹窗不同时开）。
                if key == "escape" && this.settings_open {
                    this.close_settings(cx);
                    return;
                }

                // F4：确认框模态拦截（最高优先——开启时其余键位全部隔离）。
                if this.confirm.is_some() {
                    match key.as_str() {
                        "escape" => {
                            this.confirm = None;
                            this.focus_active(window, cx);
                            cx.notify();
                        }
                        "enter" => {
                            let selected = this.confirm.as_ref().map(|c| c.selected).unwrap_or(2);
                            this.resolve_confirm(selected, window, cx);
                        }
                        "tab" => {
                            if let Some(confirm) = this.confirm.as_mut() {
                                confirm.selected = (confirm.selected + 1) % confirm.buttons.len();
                                cx.notify();
                            }
                        }
                        "left" => {
                            if let Some(confirm) = this.confirm.as_mut() {
                                confirm.selected = (confirm.selected + confirm.buttons.len() - 1)
                                    % confirm.buttons.len();
                                cx.notify();
                            }
                        }
                        "right" => {
                            if let Some(confirm) = this.confirm.as_mut() {
                                confirm.selected = (confirm.selected + 1) % confirm.buttons.len();
                                cx.notify();
                            }
                        }
                        _ => {}
                    }
                    return;
                }

                // #421：pdf 页面视图键域——缩放命令（canvas 态键域同构
                // 先例：editor::dispatch_canvas_key 在 keymap 表之前分流
                // 视口命令）。pdf 页面视图是替换渲染面（editor_area 分流
                // 编辑器不在场），shell 单点接键：Ctrl+0 适应页面 /
                // Ctrl+1 实际大小 / Ctrl+Shift+0 适应宽度 / Ctrl+=·Ctrl+-
                // 步进缩放。消费即 return——防 Ctrl+0/=/- 落 keymap 字号
                // 键（S6-S9）二次响应；非 pdf 态/文本视图不进本分支，
                // 键位行为零变化。Ctrl+滚轮在视图内 capture 期拦截
                //（pdf_pages paint 注册），不经本分支。
                if this.pdf_view_mode && this.active_is_pdf(cx) {
                    if let Some((_, entity)) = &this.pdf_pages {
                        let mut zoom_consumed = false;
                        entity.update(cx, |view, cx| {
                            zoom_consumed =
                                view.dispatch_zoom_key(&key, ctrl, ks.modifiers.shift, cx);
                        });
                        if zoom_consumed {
                            return;
                        }
                    }
                }

                // #217：shell 键位改查 keymap 表（默认表 = 下方原分支的
                // 精确镜像，行为零变化；用户换绑后跟随）。模态守卫（菜单
                // /右键菜单/确认框）在表之前——模态语义不入表。
                let kq = keymap::KeyQuery {
                    key,
                    ctrl,
                    shift: ks.modifiers.shift,
                    alt: ks.modifiers.alt,
                };
                let outcome =
                    keymap::store().resolve_shell(&kq, this.chord.as_ref(), Instant::now());
                // 挂起态随本次派发终结（Pending 分支随即重置——原
                // `chord_until.take()` 语义：过期/续接落空 = 作废不吞键，
                // 本键照常走单键匹配）。
                this.chord = None;
                match outcome {
                    // F2 和弦第一键：Ctrl+K（进入待续期，重复按刷新窗口）。
                    keymap::ShellOutcome::Pending => {
                        this.chord = Some((kq, Instant::now() + CHORD_WINDOW));
                        return;
                    }
                    keymap::ShellOutcome::None => {}
                    keymap::ShellOutcome::Matched(action) => match action {
                        // F2 和弦第二键：窗口期内 Ctrl+O → 工作区（吞掉，
                        // 不落 F1 文件打开）；过期/非 O 由引擎按 deadline
                        // 归一为单键匹配。
                        ShellAction::PromptOpenWorkspace => {
                            this.prompt_open_workspace(window, cx);
                            return;
                        }
                        // F3：Ctrl+Tab 下一标签（原单分支 !shift 参数化拆分）。
                        ShellAction::NextTab => {
                            this.cycle_tab(true, window, cx);
                            return;
                        }
                        // F3：Ctrl+Shift+Tab 上一标签。
                        ShellAction::PrevTab => {
                            this.cycle_tab(false, window, cx);
                            return;
                        }
                        // F3：Ctrl+W 关闭当前标签（F4：脏标签经确认框）。
                        ShellAction::CloseTab => {
                            this.close_active_tab(window, cx);
                            return;
                        }
                        // #386：Ctrl+S 保存（编辑器 E30 移除后冒泡至此
                        // ——保存分流在 Shell：pdf 批注增量落盘/编辑器
                        // 三态；失败 toast 示警不静默）。
                        ShellAction::Save => {
                            this.save_tab(this.active, cx);
                            cx.notify();
                            return;
                        }
                        // F4：Ctrl+Shift+S 另存为（编辑器 s 臂带 !shift 守卫
                        // 冒泡至此）。
                        ShellAction::PromptSaveAs => {
                            this.prompt_save_as(window, cx);
                            return;
                        }
                        // F6：Ctrl+= / Ctrl++ 放大（shift 兼容——"+" 多为
                        // shift+= 产生）。
                        ShellAction::FontBigger => {
                            let next = this.font_size + 1.;
                            this.apply_font_size(px(next), cx);
                            return;
                        }
                        // F6：Ctrl+- 缩小。
                        ShellAction::FontSmaller => {
                            let next = this.font_size - 1.;
                            this.apply_font_size(px(next), cx);
                            return;
                        }
                        // F6：Ctrl+0 重置 15px。
                        ShellAction::FontReset => {
                            this.apply_font_size(px(15.), cx);
                            return;
                        }
                        // F6：Ctrl+T 循环主题（system → light → dark → system）。
                        ShellAction::CycleTheme => {
                            this.cycle_theme(cx);
                            return;
                        }
                        // F1：Ctrl+O 文件打开。
                        ShellAction::PromptOpenFile => {
                            this.prompt_open_file(window, cx);
                        }
                    },
                }

                // #422：pdf 页面视图内查找——Ctrl+F（或用户换绑的
                // open_find）不切视图，直接开页内查找条。页视图态编辑
                // 器不在树内，键落到本根节点（Ctrl+F 是编辑器动作，
                // resolve_shell 不解析——查 keymap 编辑表保持换绑跟随；
                // 命中即吞，未命中照常下沉不拦）。
                if this.pdf_view_mode
                    && this
                        .editor()
                        .is_some_and(|ed| ed.read(cx).doc_kind() == "pdf")
                    && let Some(view) = this.pdf_pages.as_ref().map(|(_, e)| e.clone())
                {
                    let editor_outcome = keymap::store()
                        .resolve_editor_main(&kq, this.chord.as_ref().map(|(q, _)| q));
                    if let keymap::EditorOutcome::Matched(keymap::EditorAction::OpenFind) =
                        editor_outcome
                    {
                        this.chord = None;
                        view.update(cx, |v, cx| v.open_search(window, cx));
                        return;
                    }
                }
            }))
            .child(title_bar::title_bar(&title, &menus, self.menu_bar_open, cx))
            // F3：标签栏（有标签才渲染；标题栏与主体之间）。
            .when(!self.tabs.is_empty(), |el| {
                let ws_root = self
                    .workspace
                    .as_ref()
                    .map(|tree| tree.read(cx).root().to_path_buf());
                el.child(tab_bar::tab_bar(
                    &self.tabs,
                    self.active,
                    ws_root.as_deref(),
                    cx,
                ))
            })
            .child(
                h_flex()
                    .id("shell-body")
                    .flex_1()
                    .min_h_0()
                    .child(activity_bar::activity_bar(
                        self.left_open,
                        self.git_view,
                        self.settings_open,
                        cx,
                    ))
                    .when(self.left_open, |el| {
                        el.child(panels::left_panel(
                            self.editor(),
                            self.workspace.as_ref(),
                            self.git_panel.as_ref(),
                            self.git_view,
                            // G4：可拖拽宽度（域钳制 + settings 持久化）
                            self.left_width,
                            // #571：大纲面板实体（键盘导航 + #289-④
                            // 页滚钩子在实体构造时注入）。
                            self.outline.as_ref(),
                            cx,
                        ))
                        // G4：左栏右缘 4px 拖拽热区
                        .child(self.panel_resize_handle(PanelSide::Left, cx))
                    })
                    .child(editor_area(self, cx))
                    .when(self.right_open, |el| {
                        // G4：右栏左缘 4px 拖拽热区
                        el.child(self.panel_resize_handle(PanelSide::Right, cx))
                            // #289-⑤：右栏单一视图——属性/来源（设置已改
                            // 弹窗，不再占右栏位）。
                            .child(panels::right_panel(
                                self.editor().cloned().as_ref(),
                                self.right_width,
                                cx,
                            ))
                    })
                    // #289-⑤：右缘 dock 条（属性/来源开关归右缘——
                    // 恒在，右栏收起时是唯一入口；zed 折叠 dock 同构）。
                    .child(right_dock::right_dock_strip(self.right_open, cx)),
            )
            // F1：提示横幅（状态栏上方一条——有消息时才渲染）。
            .map(|el| match self.notice_board.message() {
                Some(message) => el.child(notice_bar(message, &colors)),
                None => el,
            })
            // F4：确认框模态（半透明遮罩全屏拦截点按）。
            .map(|el| match &self.confirm {
                Some(confirm) => el.child(confirm_overlay(confirm, &self.confirm_focus, cx)),
                None => el,
            })
            // #289-⑤：设置弹窗模态（链尾最上层——后画盖前画；与确认框
            // 互斥，两弹窗不同时开）。
            .when(self.settings_open, |el| {
                el.child(settings_panel::settings_modal(
                    self.theme_mode,
                    self.font_size,
                    self.indent_unit,
                    cx,
                ))
            })
            .child(status_bar::status_bar(
                self.editor().cloned().as_ref(),
                self.workspace
                    .as_ref()
                    .map(|tree| tree.read(cx).root().to_path_buf()),
                self.pdf_view_mode,
                self.xlsx_grid_view_mode,
                cx,
            ))
            // G4：面板拖拽收尾钩（窗口级 mouseup——canvas paint 期每帧
            // 注册，与 editor end_drag 同模式；非拖拽中为空操作）。
            .child(
                div()
                    .id("panel-mouseup-hook")
                    .absolute()
                    .size_0()
                    .child(canvas(
                        move |bounds: Bounds<Pixels>, _window, _cx| bounds,
                        move |_bounds, _t, window: &mut Window, _cx| {
                            window.on_mouse_event(
                                move |_: &MouseUpEvent, phase: DispatchPhase, _window, cx| {
                                    if phase == DispatchPhase::Bubble {
                                        // #89 坑3 变体：drag 会话的 mouseup 派发在
                                        // drag 源（shell）的 update 内——直接再
                                        // update 即重入 panic；延迟到下一事件
                                        // 拍收尾（落盘 + 证据行）。
                                        let shell = shell_entity.clone();
                                        cx.spawn(async move |cx| {
                                            cx.update(|cx| {
                                                shell.update(cx, |shell, cx| {
                                                    shell.finish_panel_drag(cx)
                                                })
                                            });
                                        })
                                        .detach();
                                    }
                                },
                            );
                        },
                    )),
            )
            // #166（#385 批 1）：右键菜单浮层——**链尾最上层**。`deferred
            // + anchored` 引擎级浮层（溢出翻转/吸附按实际尺寸处理），
            // 点外部关闭由组件 on_blur→DismissEvent 承担（透明遮罩退役）。
            .map(|el| match &self.context_menu {
                Some(menu) => el.child(context_menu_view::context_menu_anchor(
                    menu,
                    self.context_menu_pos,
                )),
                None => el,
            })
            // #209（#385 批 4）：菜单栏下拉浮层——链尾最上层（与右键菜单
            // 浮层互斥——打开时已收口）。`deferred + anchored` 引擎级
            // 浮层（溢出翻转/吸附按实际渲染尺寸），点外关闭由组件
            // on_mouse_down_out → DismissEvent 承担（全屏遮罩退役）。
            .map(|el| match (&self.menu_bar_open, &self.menu_bar_entity) {
                (Some(_), Some(entity)) => {
                    el.child(menu_bar_view::menu_bar_anchor(entity, self.menu_bar_anchor))
                }
                _ => el,
            })
    }
}

/// 右键目标作为「新建」落点目录：目录本身；文件则取所在目录
///（#166 ft-new-file / ft-new-folder 共用）。
fn dir_for_new(target: &Path) -> PathBuf {
    if target.is_dir() {
        target.to_path_buf()
    } else {
        target
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| target.to_path_buf())
    }
}

/// #179（#188 接线）：编辑器右键菜单上抛回调的工厂——ShellApp 弱句柄
/// 在**这里**捕获（editor 是 lib+bin 共用源码，lib 目标不含 shell
/// 模块树，类型上不许直引；闭包做类型擦除的跨实体通道，#166
/// file_tree 的弱句柄模式同款）。三个挂入口共用：
/// ShellApp::new / open_document_tab / open_readonly_tab。
fn menu_opener_for(shell: gpui::WeakEntity<ShellApp>) -> crate::editor::MenuOpener {
    Box::new(move |pos, has_selection, read_only, window, cx| {
        let _ = shell.update(cx, |shell, cx| {
            shell.open_editor_context_menu(pos, has_selection, read_only, window, cx)
        });
    })
}

/// F4：确认框模态遮罩（半透明背景全屏 + 居中卡片 + 三钮）。遮罩
/// 拦截一切点按（背景无事件——点击不穿透到底下 UI）；焦点域在遮罩
/// 节点（track_focus）——编辑器隔离，键事件沿遮罩→shell 根冒泡。
fn confirm_overlay(
    confirm: &ConfirmState,
    focus: &FocusHandle,
    cx: &mut Context<ShellApp>,
) -> impl IntoElement {
    let colors = cx.theme().colors().clone();
    div()
        .id("confirm-backdrop")
        .absolute()
        .size_full()
        .top_0()
        .left_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui::black().opacity(0.5))
        .track_focus(focus)
        .child(
            v_flex()
                .id("confirm-box")
                .w(px(380.))
                .p_4()
                .gap_3()
                .bg(colors.elevated_surface_background)
                .border_1()
                .border_color(colors.border)
                .rounded_md()
                .child(ui::Label::new(confirm.message.as_str()).size(ui::LabelSize::Default))
                .child(
                    ui::Label::new(confirm.detail.as_str())
                        .size(ui::LabelSize::Small)
                        .color(ui::Color::Muted),
                )
                .child(
                    h_flex()
                        .id("confirm-buttons")
                        .gap_2()
                        .justify_end()
                        .children(confirm.buttons.iter().enumerate().map(|(i, label)| {
                            let selected = confirm.selected == i;
                            let selected_bg = colors.element_selected;
                            let hover_bg = colors.element_hover;
                            h_flex()
                                .id(gpui::SharedString::from(format!("confirm-btn-{i}")))
                                .px_3()
                                .py_1()
                                .rounded_sm()
                                .border_1()
                                .border_color(colors.border)
                                .when(selected, move |el| el.bg(selected_bg))
                                .hover(move |el| el.bg(hover_bg))
                                .on_mouse_down(
                                    gpui::MouseButton::Left,
                                    cx.listener(move |this, _, window, cx| {
                                        this.resolve_confirm(i, window, cx);
                                    }),
                                )
                                .child(ui::Label::new(*label).size(ui::LabelSize::Small))
                        })),
                ),
        )
}

/// F1：提示横幅（状态栏上方一条——警告性信息面：打开失败/类型不支持/
/// 对话框不可用）。色板无 warning 语义色（JustBase 基础主题），用
/// element_background + border + text_accent 组合呈现。
fn notice_bar(message: &str, colors: &theme::ThemeColors) -> impl IntoElement {
    h_flex()
        .id("shell-notice")
        .w_full()
        .h(px(28.))
        .px_3()
        .flex_none()
        .items_center()
        .gap_2()
        .bg(colors.element_background)
        .border_t_1()
        .border_color(colors.border)
        .child(
            ui::Label::new(message)
                .color(ui::Color::Accent)
                .size(ui::LabelSize::Small),
        )
}

/// 编辑区：当前标签的 `Editor` 实体；无标签 → 引导空态 + 最近打开
/// 列表（F5——文件点击打开 / 工作区点击打开；已删除文件置灰不可点，
/// 点击提示「文件不存在」）。
fn editor_area(shell: &mut ShellApp, cx: &mut Context<ShellApp>) -> impl IntoElement {
    let colors = cx.theme().colors().clone();
    // #289-④：pdf 标签的页面视图（位图渲染主显示面）——模式开且有
    // 实体时替代文本编辑器呈现（查找面：#422 页内查找条随视图在位，
    // 书签锚仍在文本模型上）。
    let pdf_pages_entity = shell
        .editor()
        .filter(|ed| ed.read(cx).doc_kind() == "pdf")
        .and_then(|_| shell.pdf_view_mode.then(|| ()))
        .and_then(|_| shell.pdf_pages.as_ref().map(|(_, e)| e.clone()));
    // #389 阶段 1：xlsx 标签的网格视图（真网格主显示面）——模式开且有
    // 实体时替代文本行模型呈现（状态栏「网格/文本」钮切换回退）。
    let xlsx_grid_entity = shell
        .editor()
        .filter(|ed| ed.read(cx).doc_kind() == "xlsx")
        .and_then(|_| shell.xlsx_grid_view_mode.then(|| ()))
        .and_then(|_| shell.xlsx_grid.as_ref().map(|(_, e)| e.clone()));
    let active_editor = shell.editor().cloned();
    match (pdf_pages_entity, xlsx_grid_entity, active_editor) {
        (Some(pages), _, _) => pages.into_any_element(),
        (None, Some(grid), _) => grid.into_any_element(),
        (None, None, Some(editor)) => editor.into_any_element(),
        (None, None, None) => {
            // F5：最近列表数据（文件至多 8 / 工作区至多 3 展示面——
            // 记忆上限仍为 20/10，展示截断防长列表挤爆空态页）。
            let recent_files: Vec<PathBuf> = shell
                .recent
                .recent_files()
                .iter()
                .take(8)
                .cloned()
                .collect();
            let recent_workspaces: Vec<PathBuf> = shell
                .recent
                .recent_workspaces()
                .iter()
                .take(3)
                .cloned()
                .collect();
            let hover = colors.element_hover;
            v_flex()
                .id("editor-area-empty")
                .flex_1()
                .min_w_0()
                .h_full()
                .bg(colors.background)
                .items_center()
                .justify_center()
                .gap_3()
                .child(ui::Headline::new("mang").size(ui::HeadlineSize::Large))
                .child(
                    ui::Label::new(
                        "打开 .md 可编辑；.docx/.pdf 只读渲染——Ctrl+O 打开文件 / Ctrl+K Ctrl+O 打开文件夹（多标签已接线：Ctrl+Tab 切换）",
                    )
                    .color(ui::Color::Muted),
                )
                .when(!recent_files.is_empty() || !recent_workspaces.is_empty(), |el| {
                    el.child(
                        v_flex()
                            .id("recent-list")
                            .gap_1()
                            .mt_4()
                            .w(px(420.))
                            .child(
                                ui::Label::new("最近打开")
                                    .size(ui::LabelSize::Small)
                                    .color(ui::Color::Muted),
                            )
                            .children(recent_files.iter().map(|path| {
                                recent_row(path, false, hover, cx)
                            }))
                            .when(!recent_workspaces.is_empty(), |el| {
                                el.child(
                                    ui::Label::new("最近的工作区")
                                        .size(ui::LabelSize::Small)
                                        .color(ui::Color::Muted)
                                        .mt_2(),
                                )
                                .children(recent_workspaces.iter().map(|path| {
                                    recent_row(path, true, hover, cx)
                                }))
                            }),
                    )
                })
                .into_any_element()
        }
    }
}

/// F5：最近列表行（文件/工作区）。已删除 → 置灰（Hidden 色 + 不可点
/// hover），点击仍给「不存在」提示（可感知而非无反应）。
fn recent_row(
    path: &Path,
    is_workspace: bool,
    hover: gpui::Hsla,
    cx: &mut Context<ShellApp>,
) -> gpui::AnyElement {
    let exists = path.exists();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let detail = path.display().to_string();
    let path_for_click = path.to_path_buf();

    h_flex()
        .id(gpui::SharedString::from(format!("recent-{detail}")))
        .w_full()
        .px_2()
        .py_1()
        .gap_2()
        .items_center()
        .rounded_sm()
        .when(exists, move |el| el.hover(move |el| el.bg(hover)))
        .on_mouse_down(
            gpui::MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                if is_workspace {
                    this.open_workspace(path_for_click.clone(), cx);
                } else if path_for_click.exists() {
                    this.open_document(path_for_click.clone(), window, cx);
                } else {
                    let message = format!("文件不存在：{}", path_for_click.display());
                    this.show_notice(message, cx);
                }
            }),
        )
        .child(
            ui::Icon::new(if is_workspace {
                ui::IconName::Folder
            } else {
                ui::IconName::File
            })
            .size(ui::IconSize::XSmall)
            .color(if exists {
                ui::Color::Muted
            } else {
                ui::Color::Hidden
            }),
        )
        .child(
            ui::Label::new(name)
                .size(ui::LabelSize::Small)
                .color(if exists {
                    ui::Color::Default
                } else {
                    ui::Color::Hidden
                }),
        )
        .child(
            ui::Label::new(detail)
                .size(ui::LabelSize::XSmall)
                .color(ui::Color::Hidden),
        )
        .into_any_element()
}

//-----------------------------------------------------------------------
// F1 单测：提示横幅状态机（纯逻辑——无需 gpui 上下文）。
//-----------------------------------------------------------------------
#[cfg(test)]
mod notice_tests {
    use super::NoticeBoard;

    #[test]
    fn show_returns_incrementing_generations() {
        let mut board = NoticeBoard::default();
        assert_eq!(board.message(), None, "初始无消息");
        let g1 = board.show("第一条");
        assert_eq!(g1, 1);
        assert_eq!(board.message(), Some("第一条"));
        let g2 = board.show("第二条");
        assert_eq!(g2, 2, "代际递增");
        assert_eq!(board.message(), Some("第二条"), "新消息覆盖旧消息");
    }

    #[test]
    fn dismiss_only_matches_current_generation() {
        let mut board = NoticeBoard::default();
        let g1 = board.show("第一条");
        let g2 = board.show("第二条");
        // 旧代际定时器到达——不得误清新消息。
        assert!(!board.dismiss(g1), "过时代际不清除");
        assert_eq!(board.message(), Some("第二条"));
        // 当前代际定时器到达——正常清除。
        assert!(board.dismiss(g2));
        assert_eq!(board.message(), None);
        // 空板再 dismiss 恒 false。
        assert!(!board.dismiss(g2));
    }

    #[test]
    fn clear_wipes_unconditionally() {
        let mut board = NoticeBoard::default();
        let g = board.show("消息");
        board.clear();
        assert_eq!(board.message(), None);
        // clear 后旧代际定时到达仍无效（无消息可清）。
        assert!(!board.dismiss(g));
    }

    #[test]
    fn generation_survives_clear() {
        let mut board = NoticeBoard::default();
        let _g1 = board.show("第一条");
        board.clear();
        let g2 = board.show("第二条");
        // clear 不重置代际——防 clear 前后同号定时器误配。
        assert_eq!(g2, 2);
    }
}

#[cfg(test)]
mod tests {
    /// 纯状态机面：无需 gpui 上下文即可钉定折叠语义。
    /// （Entity 构造需要 App 上下文，此处以最小桩验证状态翻转逻辑——
    /// 真值由 Xvfb 烟雾与编辑器纯逻辑单测背书。）
    #[test]
    fn toggle_semantics_flip_independently() {
        let mut left = true;
        let mut right = true;
        left = !left;
        assert!(!left, "左栏翻转：开 → 关");
        left = !left;
        assert!(left, "左栏再翻转：关 → 开");
        right = !right;
        assert!(!right);
        assert!(left, "右栏翻转不影响左栏");
    }
}

// ---------------------------------------------------------------------------
// #219：多标签文档工作流——运行时测试（真实 gpui::TestAppContext：
// 实体/窗口/焦点全真，无头宿主窗口，与 editor 运行时测试同模式；
// 经 bin 单测面随 `cargo test --tests` 执行）。
//
// 分工（三层互证，不重复钉定）：
// - 纯逻辑三件（邻位规则/循环 wrap/同名消歧）→ tab_bar.rs 既有单测；
// - 键位可配置面（S2/S3/S4 默认绑定+守卫+换绑）→ keymap.rs
//   `shell_single_key_branches` 等既有单测；
// - 本模块：**标签生命周期与 dirty 状态的运行时面**——多文档并存
//   切换状态保持 / 同文件重复打开去重（相对+绝对两形态）/ 关闭
//   邻位规则与末标签空态 / 脏标签关闭确认三选（取消-保存-放弃）。
//
// 宿主：ShellApp 不作根视图（render 路径的菜单/主题/资产面与本模块
// 无关），空 div 宿主开窗只取真实 `Window`——EditorHost 同款口径。
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tab_workflow_tests {
    use super::*;
    use crate::editor::buffer::Point;
    use crate::editor::commands::Cmd;

    /// 测试宿主（见模块注释）。
    struct ShellHost(Entity<ShellApp>);

    impl gpui::Render for ShellHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
            gpui::div()
        }
    }

    /// 临时夹具目录（OS 临时区，进程号隔离；测试首尾各清一次——
    /// 崩溃残留也只是临时区垃圾，不污染工作树）。
    fn fixture_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mdc-tab-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 写一个 md 夹具（返回绝对路径——对话框/文件树入参形态）。
    fn write_md(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    /// 空态 ShellApp + 宿主窗口（返回实体与窗口句柄）。
    fn shell_window(
        cx: &mut gpui::TestAppContext,
    ) -> (Entity<ShellApp>, gpui::WindowHandle<ShellHost>) {
        // 主题全局：真实应用在 main.rs 开窗前 theme::init(JustBase)——
        // 测试镜像同一序列（ShellApp::new 的 GlobalTheme::theme /
        // theme_switch::apply_mode 首访即需；TestPlatform 的
        // window_appearance 为固定 Light 桩，fallback 主题集不依赖
        // 资产源——JustBase 与 main 同款可无头初始化）。
        cx.update(|app| theme::init(theme::LoadThemes::JustBase, app));
        // ShellApp::new 需要 &mut Window（on_window_should_close 等），
        // 实体只能在 add_window 闭包内构造；槽位带出（FnOnce 一次性）。
        let mut slot: Option<Entity<ShellApp>> = None;
        let window = cx.add_window(|window, cx| {
            let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
            slot = Some(shell.clone());
            ShellHost(shell)
        });
        (slot.expect("宿主构造即置"), window)
    }

    /// 打开一个文档标签（open_document_tab 的薄包装——无 window 面）。
    fn open(shell: &Entity<ShellApp>, cx: &mut Context<ShellHost>, path: PathBuf) {
        shell.update(cx, |shell, cx| {
            shell.open_document_tab(path, cx);
        });
    }

    /// #219 验收①：至少 3 个文档并存，切换后各自内容、光标/选区与
    /// dirty 状态保持正确（tab0 脏+光标、tab1 全选区、tab2 干净基线；
    /// 循环切换双向 wrap 后逐标签回读）。
    #[gpui::test]
    fn three_tabs_switch_preserves_state(cx: &mut gpui::TestAppContext) {
        let dir = fixture_dir("preserve");
        let a = write_md(&dir, "a.md", "A0\nA1\nA2\n");
        let b = write_md(&dir, "b.md", "B0\nB1\n");
        let c = write_md(&dir, "c.md", "C0\nC1\n");

        let (shell, window) = shell_window(cx);
        window.update(cx, |_host, _window, cx| {
            open(&shell, cx, a.clone());
            open(&shell, cx, b.clone());
            open(&shell, cx, c.clone());
            shell.update(cx, |shell, cx| {
                assert_eq!(shell.tab_count(), 3, "三文档并存");
                assert_eq!(shell.active, 2, "新开即激活");
            });
        });

        // 各标签置入待保持的状态：tab0 脏 + 光标在插入点；tab1 全选；
        // tab2 干净。tab0 的编辑不得波及 tab1/tab2 的脏位。（闭包内赋值
        // 走 default 起始 + expect 收口——闭包捕获未初始化局部不可编译。）
        let mut cursor_in_tab0 = Point::default();
        let mut selection_in_tab1: Option<(Point, Point)> = None;
        window.update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                let ed0 = shell.tab_editor(0).unwrap().clone();
                cursor_in_tab0 = ed0
                    .update(cx, |ed, cx| ed.insert_text_at(1, 2, "X", cx))
                    .expect("可编辑文档插入返回光标点");
                assert!(shell.tab_is_modified(0, cx).unwrap(), "tab0 插入后脏");

                shell.activate_tab(1, window, cx);
                let ed1 = shell.tab_editor(1).unwrap().clone();
                ed1.update(cx, |ed, cx| ed.run_command(Cmd::SelectAll, window, cx));
                selection_in_tab1 = ed1.read(cx).selection_points();

                shell.activate_tab(2, window, cx);
                assert!(!shell.tab_is_modified(2, cx).unwrap(), "tab2 干净基线");
            });
        });
        let selection_in_tab1 = selection_in_tab1.expect("tab1 全选有选区");

        // 循环切换 2→0（wrap）→1→0（backward）——全程逐项回读保持。
        window.update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.cycle_tab(true, window, cx);
                assert_eq!(shell.active, 0, "末位 wrap 回首（方向）");
                let ed0 = shell.tab_editor(0).unwrap().clone();
                let ed = ed0.read(cx);
                assert_eq!(ed.buffer_line(1), "A1X", "tab0 内容保持（编辑在）");
                assert_eq!(ed.cursor_point(), cursor_in_tab0, "tab0 光标保持");
                assert!(ed.is_modified(), "tab0 脏位跨切换保持");

                shell.cycle_tab(true, window, cx);
                assert_eq!(shell.active, 1);
                let ed1 = shell.tab_editor(1).unwrap().clone();
                assert_eq!(
                    ed1.read(cx).selection_points(),
                    Some(selection_in_tab1),
                    "tab1 选区跨切换保持"
                );
                assert!(!ed1.read(cx).is_modified(), "tab1 不脏");

                shell.cycle_tab(false, window, cx);
                assert_eq!(shell.active, 0, "backward 方向");

                // 脏位隔离：只有 tab0 脏。
                assert!(shell.tab_is_modified(0, cx).unwrap());
                assert!(!shell.tab_is_modified(1, cx).unwrap());
                assert!(!shell.tab_is_modified(2, cx).unwrap());
            });
        });

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #219 验收②/边界：同一文件重复打开（CLI 相对形态 + 对话框绝对
    /// 形态）激活既有标签，不产生重复会话。
    ///
    /// 相对形态夹具必须落在 CWD 内（cargo test 的 CWD = mang-shell
    /// workspace 根；Windows 上跨盘符构造不出相对路径）——目录名进
    /// .gitignore，测试首尾自清。
    #[gpui::test]
    fn same_file_reopen_dedupes_to_existing_tab(cx: &mut gpui::TestAppContext) {
        let tag_dir = std::env::current_dir()
            .unwrap()
            .join(format!("mdc-tab-rel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tag_dir);
        std::fs::create_dir_all(&tag_dir).unwrap();
        // 相对形态（原生分隔符：Windows 下 join 产反斜杠，与
        // std::path::absolute 的 GetFullPathNameW 归一结果一致）。
        let rel_dir = PathBuf::from(tag_dir.file_name().unwrap());
        let rel_a = rel_dir.join("a.md");
        let abs_a = tag_dir.join("a.md");
        std::fs::write(&abs_a, "A\n").unwrap();
        let abs_b = {
            let p = tag_dir.join("b.md");
            std::fs::write(&p, "B\n").unwrap();
            p
        };

        let (shell, window) = shell_window(cx);
        window.update(cx, |_host, _window, cx| {
            // 相对形态首开：落一个标签，身份 = 绝对化结果。
            open(&shell, cx, rel_a.clone());
            shell.update(cx, |shell, cx| {
                assert_eq!(shell.tab_count(), 1);
                assert_eq!(shell.tab_path(0), Some(abs_a.clone()), "相对入参绝对化");
            });

            // 绝对形态开 b → 两标签。
            open(&shell, cx, abs_b.clone());
            shell.update(cx, |shell, cx| {
                assert_eq!(shell.tab_count(), 2);
                assert_eq!(shell.active, 1);
            });

            // 同文件绝对形态重开 → 激活既有标签，不增。
            open(&shell, cx, abs_a.clone());
            shell.update(cx, |shell, cx| {
                assert_eq!(shell.tab_count(), 2, "绝对形态重开不重复开会话");
                assert_eq!(shell.active, 0, "重开 = 激活既有");
                assert_eq!(shell.tab_index_by_path(&abs_a), Some(0));
            });

            // 同文件相对形态重开 → 同样去重（两形态同身份）。
            open(&shell, cx, rel_a.clone());
            shell.update(cx, |shell, cx| {
                assert_eq!(shell.tab_count(), 2, "相对形态重开不重复开会话");
                assert_eq!(shell.active, 0);
                assert_eq!(shell.tab_index_by_path(&rel_a), Some(0));
            });
        });

        let _ = std::fs::remove_dir_all(&tag_dir);
    }

    /// #219 验收③/边界：关闭邻位规则（右邻优先/末位回左）、关非活动
    /// 标签的 active 校正、关最后一个标签 → 空态不 panic、空态再关
    /// 幂等。
    #[gpui::test]
    fn close_neighbor_rule_and_last_tab_empty_state(cx: &mut gpui::TestAppContext) {
        let dir = fixture_dir("close");
        let a = write_md(&dir, "a.md", "A\n");
        let b = write_md(&dir, "b.md", "B\n");
        let c = write_md(&dir, "c.md", "C\n");

        let (shell, window) = shell_window(cx);
        window.update(cx, |_host, _window, cx| {
            open(&shell, cx, a.clone());
            open(&shell, cx, b.clone());
            open(&shell, cx, c.clone());
        });

        window.update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                // [A,B,C] 激活 1，关 B → C 前移补位，active 仍 1。
                shell.activate_tab(1, window, cx);
                shell.close_active_tab(window, cx);
                assert_eq!(shell.tab_count(), 2);
                assert_eq!(shell.active, 1, "右邻前移补位");
                assert_eq!(shell.tab_path(1), Some(c.clone()), "C 补位到 1");

                // [A,C] active 1，关非活动左邻 A（idx 0）→ active 校正 0。
                shell.close_tab_at(0, window, cx);
                assert_eq!(shell.tab_count(), 1);
                assert_eq!(shell.active, 0, "关左侧非活动标签 active 随减");

                // 关最后一个标签 → 空态（editor() None，active 归零）。
                shell.close_active_tab(window, cx);
                assert_eq!(shell.tab_count(), 0, "关最后一个标签 → 空态");
                assert_eq!(shell.active, 0);
                assert!(shell.editor().is_none());

                // 空态再关：幂等不 panic。
                shell.close_active_tab(window, cx);
                assert_eq!(shell.tab_count(), 0);
            });
        });

        // 空态后可继续开新文档（生命周期闭环）。
        window.update(cx, |_host, _window, cx| {
            open(&shell, cx, a.clone());
            shell.update(cx, |shell, cx| {
                assert_eq!(shell.tab_count(), 1, "空态后再开正常");
            });
        });

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #219 验收④：脏标签关闭确认三选——取消（原标签与内容不变）、
    /// 保存（落盘后关）、不保存（直接关、盘上内容不动）。干净标签
    /// 关闭无确认框。
    #[gpui::test]
    fn dirty_close_confirm_cancel_save_discard(cx: &mut gpui::TestAppContext) {
        let dir = fixture_dir("confirm");
        let a = write_md(&dir, "a.md", "L0\n");

        let (shell, window) = shell_window(cx);
        window.update(cx, |_host, _window, cx| {
            open(&shell, cx, a.clone());
        });

        // 干净标签：直接关，无确认框。
        window.update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.close_active_tab(window, cx);
                assert!(shell.confirm.is_none(), "干净标签关闭不弹确认");
                assert_eq!(shell.tab_count(), 0);
            });
        });

        // 重开并置脏（L0 → L0X）。
        window.update(cx, |_host, _window, cx| {
            open(&shell, cx, a.clone());
            shell.update(cx, |shell, cx| {
                shell.tab_editor(0).unwrap().update(cx, |ed, cx| {
                    ed.insert_text_at(0, 2, "X", cx);
                });
                assert!(shell.tab_is_modified(0, cx).unwrap());
            });
        });

        // 关脏标签 → 确认框（文案/按钮/动作/默认选中）。
        window.update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.close_active_tab(window, cx);
                let confirm = shell.confirm.as_ref().expect("脏标签关闭弹确认");
                assert!(confirm.message.contains("a.md"), "文案含文件名");
                assert_eq!(
                    confirm.buttons,
                    CONFIRM_BUTTONS.to_vec(),
                    "保存/不保存/取消三选"
                );
                assert!(matches!(confirm.action, ConfirmAction::CloseTab { idx: 0 }));
                assert_eq!(confirm.selected, 0, "默认停在保存侧");
            });
        });

        // 取消：原标签与内容原样保留，确认框清除。
        window.update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.resolve_confirm(2, window, cx);
                assert!(shell.confirm.is_none(), "取消清确认框");
                assert_eq!(shell.tab_count(), 1, "取消不关标签");
                let ed = shell.tab_editor(0).unwrap().clone();
                assert_eq!(ed.read(cx).buffer_line(0), "L0X", "取消内容不变");
                assert!(ed.read(cx).is_modified(), "取消后仍脏");
            });
        });

        // 保存（choice 0）：落盘 + 关标签；盘上带 X。
        window.update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.close_active_tab(window, cx);
                assert!(shell.confirm.is_some());
                shell.resolve_confirm(0, window, cx);
                assert_eq!(shell.tab_count(), 0, "保存后关");
                assert_eq!(
                    std::fs::read_to_string(&a).unwrap(),
                    "L0X\n",
                    "保存 = 编辑落盘"
                );
            });
        });

        // 重开：内容带 X 且干净；再置脏（L0X → L0XY）后不保存（choice 1）：
        // 直接关、盘上仍是无 Y 的版本。
        window.update(cx, |_host, _window, cx| {
            open(&shell, cx, a.clone());
            shell.update(cx, |shell, cx| {
                let ed = shell.tab_editor(0).unwrap().clone();
                assert_eq!(ed.read(cx).buffer_line(0), "L0X", "重开 = 盘上内容");
                assert!(!ed.read(cx).is_modified(), "重开干净");
                ed.update(cx, |ed, cx| {
                    ed.insert_text_at(0, 3, "Y", cx);
                });
            });
        });
        window.update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.close_active_tab(window, cx);
                shell.resolve_confirm(1, window, cx);
                assert_eq!(shell.tab_count(), 0, "不保存直接关");
                assert_eq!(
                    std::fs::read_to_string(&a).unwrap(),
                    "L0X\n",
                    "放弃 = 盘上内容不动（Y 未落盘）"
                );
            });
        });

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #219 边界：循环切换在空态/单标签为 no-op；越界激活安全不改
    /// 状态（标签点击竞态/程序化调用的防御面）。
    #[gpui::test]
    fn cycle_noop_edges_and_out_of_range_activate(cx: &mut gpui::TestAppContext) {
        let dir = fixture_dir("edges");
        let a = write_md(&dir, "a.md", "A\n");
        let b = write_md(&dir, "b.md", "B\n");

        let (shell, window) = shell_window(cx);

        // 空态循环：no-op 不 panic。
        window.update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.cycle_tab(true, window, cx);
                shell.cycle_tab(false, window, cx);
                assert_eq!(shell.tab_count(), 0);
                assert_eq!(shell.active, 0);
            });
        });

        window.update(cx, |_host, window, cx| {
            open(&shell, cx, a.clone());
            // 单标签循环：不变（len < 2 短路——shell 级方法验证短路
            // 真的生效，焦点面不炸）。
            shell.update(cx, |shell, cx| {
                shell.cycle_tab(true, window, cx);
                assert_eq!(shell.tab_count(), 1);
                assert_eq!(shell.active, 0, "单标签循环不变");
            });

            // 两标签（active 1 = b）：循环两周回原位（1→0→1——cycle 的
            // eprintln 为 1 基展示，与本处 0 基状态勿混）；越界激活安全
            // 不改状态（标签点击竞态/程序化调用的防御面）。
            open(&shell, cx, b.clone());
            shell.update(cx, |shell, cx| {
                shell.cycle_tab(true, window, cx);
                shell.cycle_tab(true, window, cx);
                assert_eq!(shell.active, 1, "两标签循环两周回原位（b→a→b）");

                shell.activate_tab(7, window, cx);
                assert_eq!(shell.active, 1, "越界激活不改 active");
                shell.activate_tab(0, window, cx);
                assert_eq!(shell.active, 0);
                shell.activate_tab(2, window, cx);
                assert_eq!(shell.active, 0, "越界激活不改 active（再证）");
            });
        });

        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------------------
// #422：页内查找 Shell 接线——Ctrl+F 不切视图（页面视图内查找条）
// ---------------------------------------------------------------------------
#[cfg(test)]
mod pdf_search_wiring_tests {
    use super::*;

    /// 测试宿主（ShellApp 不作根视图但**必须进渲染树**——#422 的键
    /// 派发走真实管线：焦点路径经 shell-root → editor_area → 页视图
    /// 的 track_focus 节点；空 div 宿主会把整棵树挡在派发树外）。
    struct WiringHost(Entity<ShellApp>);

    impl gpui::Render for WiringHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
            gpui::div().size_full().child(self.0.clone())
        }
    }

    /// smoke.pdf 夹具字节（两页——pdf_pages/#422 视图级测试同源夹具）。
    fn smoke_pdf_bytes() -> Vec<u8> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.pdf");
        std::fs::read(&p).unwrap_or_else(|e| panic!("夹具不可读（{}）：{e}", p.display()))
    }

    /// #422 接线面：页面视图下 Ctrl+F（真实键击管线）→ 开页内查找条
    /// （**不切视图**——#297 折中升级为正式能力）；Esc 关条清态；菜单
    /// 路径（dispatch_editor_cmd）同源。引擎门控：无 libpdfium 时打开
    /// 本身失败（错误进 notice）→ SKIP 横幅（AGENTS 13.1——不伪装通过）。
    #[gpui::test]
    fn pdf_view_ctrl_f_opens_in_page_search(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("mdc-422-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pdf = dir.join("s.pdf");
        std::fs::write(&pdf, smoke_pdf_bytes()).unwrap();

        // 渲染面全局件（main.rs 生产同构——空 div 宿主时代不渲染整树
        // 用不到；#422 键派发需要整树，theme provider 必须在位）。
        cx.update(|app| {
            theme::init(theme::LoadThemes::JustBase, app);
            theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
        });
        let mut slot: Option<Entity<ShellApp>> = None;
        let window = cx.add_window(|window, cx| {
            let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
            slot = Some(shell.clone());
            WiringHost(shell)
        });
        let shell = slot.expect("宿主构造即置");

        // 生产同构链：open_document（open_document_tab + 落焦——#422
        // 后页视图态 pdf 标签落焦页视图，键事件可达）。
        window.update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_document(pdf.clone(), window, cx);
            });
        });
        // 引擎门控：无 libpdfium → pdf_doc::load 即失败（open 走 Failed
        // 分支，无标签无槽，错误文案进 notice）→ SKIP 横幅 + return；
        // 槽在位 = 引擎健康（tab 开成功 ⇔ 引擎开成功——同一定位序）。
        let open_notice = window
            .update(cx, |_host, _window, cx| {
                shell.update(cx, |shell, _| {
                    shell.notice_board.message().map(str::to_owned)
                })
            })
            .expect("窗口更新");
        let view = cx.read(|app| shell.read(app).pdf_pages.as_ref().map(|(_, e)| e.clone()));
        let view = match (view, open_notice) {
            (None, Some(msg)) if msg.contains("libpdfium") => {
                eprintln!(
                    "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                     本机无 libpdfium——#422 Ctrl+F 接线 e2e 未真跑\n  \
                     （pdf 打开面即失败：引擎绑定缺库）。\n  \
                     有 libpdfium 的环境（MDC_PDFIUM_LIB=<库路径> \
                     cargo test）全量执行。\n═══════════════════════════\n"
                );
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
            (None, other) => {
                panic!("pdf 标签未开（夹具/真缺陷，非环境缺件）：{other:?}")
            }
            (Some(view), _) => view,
        };
        // 前置：页面视图态（缺省开）。
        window
            .update(cx, |_host, _window, cx| {
                shell.update(cx, |shell, _| {
                    assert!(shell.pdf_view_mode, "前置：pdf 标签缺省页面视图");
                });
            })
            .expect("前置");
        // ① 键盘 Ctrl+F（真实派发管线——main.rs 烟雾同构的
        // update_window + dispatch_event）：不切视图，开页内查找条
        // （#297 的「自动切文本视图」在此必须不再发生）。焦点已落页
        // 视图；先强制一帧绘制（派发树在位——键事件按焦点路径派发）。
        let any: gpui::AnyWindowHandle = window.clone().into();
        cx.update_window(any, |_, window, cx| {
            window.draw(cx).clear(cx);
        })
        .expect("预绘一帧");
        cx.update_window(any, |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                    keystroke: gpui::Keystroke {
                        modifiers: gpui::Modifiers {
                            control: true,
                            ..Default::default()
                        },
                        key: "f".into(),
                        key_char: None,
                    },
                    is_held: false,
                    prefer_character_input: false,
                }),
                cx,
            );
        })
        .expect("派发 Ctrl+F");
        window
            .update(cx, |_host, _window, cx| {
                shell.update(cx, |shell, cx| {
                    assert!(
                        shell.pdf_view_mode,
                        "#422：Ctrl+F 不切文本视图（#297 折中已升级）"
                    );
                    assert!(view.read(cx).search_open(), "#422：页内查找条应开");
                });
            })
            .expect("断言");
        // ② Esc（查找输入行焦点面）：关条清态。重绘一帧让焦点迁移
        // 落进派发树（open_search 聚焦 search_focus）。
        cx.update_window(any, |_, window, cx| {
            window.draw(cx).clear(cx);
        })
        .expect("预绘一帧");
        cx.update_window(any, |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                    keystroke: gpui::Keystroke {
                        modifiers: Default::default(),
                        key: "escape".into(),
                        key_char: None,
                    },
                    is_held: false,
                    prefer_character_input: false,
                }),
                cx,
            );
        })
        .expect("派发 Esc");
        window
            .update(cx, |_host, _window, cx| {
                shell.update(cx, |shell, cx| {
                    assert!(!view.read(cx).search_open(), "Esc 关条清态");
                    assert!(shell.pdf_view_mode, "关条不动视图模式");
                });
            })
            .expect("断言");
        // ③ 菜单路径（「编辑 → 查找」同源分流——dispatch_editor_cmd）：
        // 同样开页内查找条不切视图。
        window
            .update(cx, |_host, window, cx| {
                shell.update(cx, |shell, cx| {
                    shell.dispatch_editor_cmd(crate::editor::commands::Cmd::OpenFind, window, cx);
                });
            })
            .expect("菜单路径");
        window
            .update(cx, |_host, _window, cx| {
                shell.update(cx, |shell, cx| {
                    assert!(shell.pdf_view_mode, "菜单查找同源不切视图");
                    assert!(view.read(cx).search_open(), "菜单查找开页内查找条");
                });
            })
            .expect("断言");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------------------
// #385 批 4：菜单栏状态桥交互测试（TestAppContext 无头面——对照表
// §六 质量门 1：「点击 → 状态变化」级，非仅函数返回值断言）。
//
// 宿主：ShellHost 模式同 tab_workflow_tests（ShellApp 不作根视图——
// render 路径与本测试无关，空 div 宿主开窗只取真实 Window）。
// ---------------------------------------------------------------------------
#[cfg(test)]
mod menu_bar_lifecycle_tests {
    use super::*;
    // menu crate（action 定义——批 3 起依赖）：本地 `menu` 变量名会
    // 遮蔽 crate 名，测试内以别名引用。
    use menu as menu_crate;

    /// 测试宿主（ShellHost 同款——模块间不共享私有设施，各 10 行）。
    struct Host(Entity<ShellApp>);

    impl gpui::Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
            gpui::div()
        }
    }

    /// 空态 ShellApp + 宿主窗口（theme::init 序列镜像 main.rs）。
    fn shell_window(cx: &mut gpui::TestAppContext) -> (Entity<ShellApp>, gpui::WindowHandle<Host>) {
        cx.update(|app| theme::init(theme::LoadThemes::JustBase, app));
        let mut slot: Option<Entity<ShellApp>> = None;
        let window = cx.add_window(|window, cx| {
            let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
            slot = Some(shell.clone());
            Host(shell)
        });
        (slot.expect("宿主构造即置"), window)
    }

    /// 开合状态桥：open（实体在场 + open 序）→ 切换（重建实体）→
    /// close（全清）→ 重开；外加组件 Cancel 路径（DismissEvent →
    /// 订阅回调清态——点外关闭的等价通道）。
    #[gpui::test]
    fn menu_bar_open_switch_close_lifecycle(cx: &mut gpui::TestAppContext) {
        let (shell, window) = shell_window(cx);
        window.update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                // 初始全关。
                assert_eq!(shell.menu_bar_open, None);
                assert!(shell.menu_bar_entity.is_none());

                // 开文件菜单（标题点击路径——anchor_x = 点击位）。
                shell.open_menu_bar(0, 120.0, window, cx);
                assert_eq!(shell.menu_bar_open, Some(0), "开菜单 → open 序");
                let first = shell.menu_bar_entity.clone().expect("菜单实体构建在场");
                assert!(
                    shell.menu_bar_subscription.is_some(),
                    "DismissEvent 订阅在场"
                );

                // 一键切换到帮助菜单（idx 3）——实体重建（旧订阅退役）。
                shell.open_menu_bar(3, 400.0, window, cx);
                assert_eq!(shell.menu_bar_open, Some(3), "切换 → 新 open 序");
                assert_ne!(
                    shell.menu_bar_entity.as_ref(),
                    Some(&first),
                    "切换 = 重建实体（旧实体不残留）"
                );

                // 关闭 → 全清（Esc / 点外部路径的 shell 侧终点）。
                shell.close_menu_bar(window, cx);
                assert_eq!(shell.menu_bar_open, None, "关闭 → open 清");
                assert!(shell.menu_bar_entity.is_none(), "关闭 → 实体清");
                assert!(shell.menu_bar_subscription.is_none(), "关闭 → 订阅退役");

                // 可重开。
                shell.open_menu_bar(2, 300.0, window, cx);
                assert_eq!(shell.menu_bar_open, Some(2), "关闭后可重开");
            });
        });
    }

    /// 组件 Cancel → DismissEvent → shell 清态链（菜单实体的
    /// `menu::Cancel` action 处理器 emit DismissEvent——shell 订阅
    /// 回调收口；等效于「点菜单外部」与「Esc」两条关闭路）。
    ///
    /// emit 的派发走 effect 队列（异步）——cancel 后 `run_until_parked`
    /// 等订阅回调实际送达再断言（TestAppContext 的标准等待原语）。
    #[gpui::test]
    fn menu_bar_cancel_action_dismisses(cx: &mut gpui::TestAppContext) {
        let (shell, window) = shell_window(cx);
        let menu = window
            .update(cx, |_host, window, cx| {
                shell.update(cx, |shell, cx| {
                    shell.open_menu_bar(0, 120.0, window, cx);
                });
                // 读实体（update 返回 Result——TestWindow 签名）。
                shell
                    .read(cx)
                    .menu_bar_entity
                    .clone()
                    .expect("菜单实体在场")
            })
            .expect("宿主窗口更新");
        // 组件公有的 cancel 处理器（on_action 同一入口——键盘 Esc 经
        // KeyBinding 到达的就是它，批 4 的 bind_keys 注册面）。
        window.update(cx, |_host, window, cx| {
            menu.update(cx, |menu, cx| {
                menu.cancel(&menu_crate::Cancel, window, cx);
            });
        });
        // DismissEvent 派发 → shell 订阅回调（close_menu_bar_internal）。
        cx.run_until_parked();
        shell.update(cx, |shell, _| {
            assert_eq!(
                shell.menu_bar_open, None,
                "Cancel → DismissEvent → 订阅回调清态"
            );
            assert!(shell.menu_bar_entity.is_none(), "实体随清态退役");
        });
    }
}

// ---------------------------------------------------------------------------
// #385 批 1×4 收口：右键菜单状态桥交互测试（TestAppContext 无头面——
// 对照表 §六 质量门 1：「手势 → 状态变化」级）。右键菜单面此前只有
// 数据层单测（context_menu.rs）+ wiring 静态锚，此处补运行时面：
// 开（实体/位置/来源/目标四态）→ 切换（重建实体）→ 关（全清）→
// 组件 Cancel → DismissEvent → shell 清态链 → 执行动作先行清菜单态。
// ---------------------------------------------------------------------------
#[cfg(test)]
mod context_menu_lifecycle_tests {
    use super::*;
    // menu crate（action 定义）：cancel 处理器的入参类型来源。
    use menu as menu_crate;

    /// 测试宿主（ShellHost 同款——模块间不共享私有设施，各 10 行）。
    struct Host(Entity<ShellApp>);

    impl gpui::Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
            gpui::div()
        }
    }

    /// 空态 ShellApp + 宿主窗口（theme::init 序列镜像 main.rs）。
    fn shell_window(cx: &mut gpui::TestAppContext) -> (Entity<ShellApp>, gpui::WindowHandle<Host>) {
        cx.update(|app| theme::init(theme::LoadThemes::JustBase, app));
        let mut slot: Option<Entity<ShellApp>> = None;
        let window = cx.add_window(|window, cx| {
            let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
            slot = Some(shell.clone());
            Host(shell)
        });
        (slot.expect("宿主构造即置"), window)
    }

    /// 开合状态桥：编辑器右键开（实体/位置/来源/目标四态）→ 文件树
    /// 右键切换（重建实体 + 换来源 + 带目标）→ 关闭全清 → 可重开。
    /// 点击位直传 `gpui::Point<Pixels>`（批 1×4 收口后的签名——不再
    /// 经自研 f32 Point 往返）。
    #[gpui::test]
    fn context_menu_open_switch_close_lifecycle(cx: &mut gpui::TestAppContext) {
        let (shell, window) = shell_window(cx);
        window.update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                // 初始全关。
                assert!(shell.context_menu.is_none());
                assert!(shell.context_menu_target.is_none());

                // 编辑器右键（点击位 120,80；无选区 + 只读）。
                let pos = gpui::Point::new(px(120.), px(80.));
                shell.open_editor_context_menu(pos, false, true, window, cx);
                let first = shell.context_menu.clone().expect("菜单实体构建在场");
                assert_eq!(shell.context_menu_pos, pos, "点击位原样落状态");
                assert_eq!(shell.context_menu_origin, MenuOrigin::Editor);
                assert!(shell.context_menu_target.is_none(), "编辑器来源无目标路径");

                // 文件树右键切换（重建实体 + 来源/目标换轨）。
                let pos2 = gpui::Point::new(px(40.), px(300.));
                let target = PathBuf::from("/ws/sub/a.md");
                shell.open_file_tree_context_menu(pos2, target.clone(), true, window, cx);
                assert_ne!(
                    shell.context_menu.as_ref(),
                    Some(&first),
                    "切换 = 重建实体（旧实体不残留）"
                );
                assert_eq!(shell.context_menu_pos, pos2, "切换后位置随新点击位");
                assert_eq!(shell.context_menu_origin, MenuOrigin::FileTree);
                assert_eq!(
                    shell.context_menu_target.as_deref(),
                    Some(target.as_path()),
                    "文件树来源带目标路径"
                );

                // 关闭 → 全清（Esc / 点外部路径的 shell 侧终点）。
                shell.close_context_menu(cx);
                assert!(shell.context_menu.is_none(), "关闭 → 实体清");
                assert!(shell.context_menu_target.is_none(), "关闭 → 目标随清");

                // 可重开。
                shell.open_editor_context_menu(pos, true, false, window, cx);
                assert!(shell.context_menu.is_some(), "关闭后可重开");
            });
        });
    }

    /// 组件 Cancel → DismissEvent → shell 清态链（点外部 / Esc 的
    /// 组件内归一路径——spawn_context_menu 订阅回调收口）。emit 走
    /// effect 队列（异步），run_until_parked 等送达再断言。
    #[gpui::test]
    fn context_menu_cancel_action_dismisses(cx: &mut gpui::TestAppContext) {
        let (shell, window) = shell_window(cx);
        let menu = window
            .update(cx, |_host, window, cx| {
                shell.update(cx, |shell, cx| {
                    shell.open_file_tree_context_menu(
                        gpui::Point::new(px(60.), px(90.)),
                        PathBuf::from("/ws/b.md"),
                        false,
                        window,
                        cx,
                    );
                });
                shell.read(cx).context_menu.clone().expect("菜单实体在场")
            })
            .expect("宿主窗口更新");
        // 组件公有 cancel 处理器（on_action 同一入口——键盘 Esc 与点外
        // 关闭经 on_blur 归一到这，批 1 组件语义）。
        window.update(cx, |_host, window, cx| {
            menu.update(cx, |menu, cx| {
                menu.cancel(&menu_crate::Cancel, window, cx);
            });
        });
        cx.run_until_parked();
        shell.update(cx, |shell, _| {
            assert!(
                shell.context_menu.is_none(),
                "Cancel → DismissEvent → 订阅回调清态"
            );
            assert!(shell.context_menu_target.is_none(), "目标随清态退役");
        });
    }

    /// 执行动作 → 菜单态先行清空（「选完即关」——activate 的第一段
    /// 就是清 context_menu/context_menu_target 再派发）。ed-select-all
    /// 在无编辑器宿主下 dispatch 早退（不 panic），此处只钉「点击后
    /// 菜单不悬挂」。
    #[gpui::test]
    fn context_menu_activate_clears_menu_state(cx: &mut gpui::TestAppContext) {
        let (shell, window) = shell_window(cx);
        window.update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_editor_context_menu(
                    gpui::Point::new(px(30.), px(30.)),
                    true,
                    false,
                    window,
                    cx,
                );
                assert!(shell.context_menu.is_some(), "前置：菜单在场");
                shell.activate_context_menu_item("ed-select-all", window, cx);
                assert!(shell.context_menu.is_none(), "执行动作 → 菜单态清");
                assert!(shell.context_menu_target.is_none(), "目标随清");
            });
        });
    }
}

// ---------------------------------------------------------------------------
// #386：pdf 批注落盘——Shell 分流运行时面（手势 → 脏标签 → 关闭确认
// → 保存选择 → 盘上在档）。
//
// 三层互证分工（不重复钉定）：
// - pdf_pages.rs 测试模块：视图级闭环（commit_annotation/save_annotations
//   的手势 → 脏标 → 落盘 → 独立读回）；
// - editor/mod.rs 测试模块：Editor::save 三态诚实化（只读态不再
//   假装成功）；
// - 本模块：**Shell 决策面**——批注脏计入 tab_is_modified（标题栏
//   脏标/关闭确认/RPC 同源）、关闭确认拦截（不静默丢）、确认框
//   「保存」选择真的把批注写进用户文件（save_tab 分流）。
//
// 环境门控（render_170_pdf 同口径）：页视图构造需 pdfium——无库
// 环境 SKIP 横幅（不伪装通过），有则全量真跑。
// ---------------------------------------------------------------------------
#[cfg(test)]
mod pdf_annot_save_tests {
    use super::*;

    /// 测试宿主（与 tab_workflow_tests 同款——ShellApp 不作根视图，
    /// 空 div 宿主开窗只取真实 `Window`）。
    struct ShellHost(Entity<ShellApp>);

    impl gpui::Render for ShellHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
            gpui::div()
        }
    }

    /// smoke.pdf 夹具字节（两页——与 #336/#386 视图级测试同源夹具）。
    fn smoke_pdf_bytes() -> Vec<u8> {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.pdf");
        std::fs::read(&p).unwrap_or_else(|e| panic!("夹具不可读（{}）：{e}", p.display()))
    }

    /// #386 全链：手势脏 → 关闭确认 → 「保存」→ 标签关 + 盘上批注
    /// 在档（增量前缀 + 独立读回 1 条——验收 1/3/5 的 Shell 面）。
    #[gpui::test]
    fn pdf_gesture_dirty_close_confirm_save_persists(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("mdc-386-shell-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let original = smoke_pdf_bytes();
        let pdf = dir.join("x.pdf");
        std::fs::write(&pdf, &original).unwrap();

        cx.update(|app| theme::init(theme::LoadThemes::JustBase, app));
        let mut slot: Option<Entity<ShellApp>> = None;
        let window = cx.add_window(|window, cx| {
            let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
            slot = Some(shell.clone());
            ShellHost(shell)
        });
        let shell = slot.expect("宿主构造即置");

        // 开 pdf 标签（缺省页视图模式）+ 构建页视图（生产路径 = 渲染
        // 期 ensure_pdf_pages；测试直调同一函数——同一调用面）。
        window.update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_document_tab(pdf.clone(), cx);
                shell.ensure_pdf_pages(cx);
            });
        });
        // 环境门控：无 libpdfium → 页视图 error 态（SKIP 横幅）；其他
        // 失败 = 夹具/真缺陷（panic，不放过）。
        let view_err = window
            .update(cx, |_host, _window, cx| {
                shell.update(cx, |shell, cx| {
                    shell
                        .pdf_pages
                        .as_ref()
                        .map(|(_, e)| e.read(cx).engine_error().map(str::to_owned))
                })
            })
            .expect("窗口更新");
        match view_err {
            Some(Some(e)) if e.contains("pdfium") => {
                eprintln!(
                    "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                     本机无 libpdfium——#386 Shell 分流 e2e 未真跑。\n  \
                     有 libpdfium 的环境（MDC_PDFIUM_LIB=<库路径> \
                     cargo test）全量执行。\n═══════════════════════════\n"
                );
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
            Some(Some(e)) => {
                panic!("页视图构造失败（夹具/真缺陷，非环境缺件）：{e}")
            }
            Some(None) => {} // 页视图在位——全量真跑。
            // 槽缺位 = 打开本身失败（无标签可挂，open_document_tab 走
            // Failed 分支）——失败文案在 notice：libpdfium 定位/装载
            // = 环境缺件（SKIP 横幅，同上口径，AGENTS 13.1 门序）；
            // 其他 = 夹具/真缺陷（panic，不放过）。
            None => {
                let notice = window
                    .update(cx, |_host, _window, cx| {
                        shell.update(cx, |shell, _| {
                            shell.notice_board.message().map(str::to_owned)
                        })
                    })
                    .expect("窗口更新");
                match notice {
                    Some(msg) if msg.contains("libpdfium") => {
                        eprintln!(
                            "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                             本机无 libpdfium——#386 Shell 分流 e2e 未真跑\n  \
                             （pdf 打开面即失败：引擎绑定缺库）。\n  \
                             有 libpdfium 的环境（MDC_PDFIUM_LIB=<库路径> \
                             cargo test）全量执行。\n═══════════════════════════\n"
                        );
                        let _ = std::fs::remove_dir_all(&dir);
                        return;
                    }
                    other => {
                        panic!("pdf 标签未开（夹具/真缺陷，非环境缺件）：{other:?}")
                    }
                }
            }
        }

        // ① 手势驱动页视图（生产入口 = finish_drag_annotation 收尾同款
        //    commit_annotation——不跳过视图层；缺省工具 = 高亮）。
        window.update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                let (_, entity) = shell.pdf_pages.clone().expect("槽在位");
                entity.update(cx, |v, _| {
                    v.commit_annotation(0, 40., 60., 200., 120.)
                        .expect("手势提交");
                });
            });
        });
        // ② 批注脏计入标签脏判定（tab_is_modified 同源——标题栏脏标/
        //    关闭确认/RPC 同一函数）；文本面自身不脏（只读编辑器）。
        window.update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                assert!(
                    shell.tab_is_modified(0, cx).unwrap(),
                    "#386：批注脏计入标签脏判定"
                );
                let ed = shell.tab_editor(0).unwrap();
                assert!(
                    !ed.read(cx).is_modified(),
                    "只读文本面无未存修改（脏来自页视图批注——语义分层）"
                );
            });
        });
        // ③ 关脏标签 → 确认框拦截（不静默丢——验收 3）。
        window.update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.close_active_tab(window, cx);
                let confirm = shell.confirm.as_ref().expect("批注脏关闭必弹确认");
                assert!(
                    confirm.message.contains("x.pdf"),
                    "文案含文件名（得 {:?}）",
                    confirm.message
                );
                assert_eq!(confirm.buttons, CONFIRM_BUTTONS, "保存/不保存/取消三选");
            });
        });
        // ④ 确认框选「保存」→ 标签关 + 批注增量落盘（save_tab 分流）。
        window.update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.resolve_confirm(0, window, cx);
                assert_eq!(shell.tab_count(), 0, "保存后关标签");
                assert!(shell.pdf_pages.is_none(), "标签关 → 页视图槽随之清空");
            });
        });
        // ⑤ 盘上：原前缀保持（增量非重写——验收 2）+ 独立读回 1 条
        //    （mdc-pdf 读面第二意见——验收 1 的落盘形态）。
        let after = std::fs::read(&pdf).expect("盘上文件");
        assert!(
            after.starts_with(&original),
            "#386 验收 2：盘上保持原字节前缀（增量）"
        );
        assert!(after.len() > original.len(), "批注修订段非空");
        let (annots, report) =
            mdc_pdf::annotate::read_annotations(after, mdc_pdf::security::Limits::default())
                .expect("独立读回");
        assert_eq!(report.parsed, 1, "重开应见 1 条批注：{report:?}");
        assert_eq!(annots.len(), 1);
        assert_eq!(annots[0].page.0, 0, "页号往返");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #386 退出面：批注脏标签计入退出前确认（on_window_should_close
    /// 的 dirty 收集同源——含页视图未落盘批注）。仅钉「计入 + 拦截」
    /// 这一环（保存分流本体已由上一条钉定；不重复）。
    #[gpui::test]
    fn pdf_annotate_dirty_blocks_quit_confirm(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("mdc-386-quit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let original = smoke_pdf_bytes();
        let pdf = dir.join("q.pdf");
        std::fs::write(&pdf, &original).unwrap();

        cx.update(|app| theme::init(theme::LoadThemes::JustBase, app));
        let mut slot: Option<Entity<ShellApp>> = None;
        let window = cx.add_window(|window, cx| {
            let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
            slot = Some(shell.clone());
            ShellHost(shell)
        });
        let shell = slot.expect("宿主构造即置");

        window.update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_document_tab(pdf.clone(), cx);
                shell.ensure_pdf_pages(cx);
            });
        });
        let view_err = window
            .update(cx, |_host, _window, cx| {
                shell.update(cx, |shell, cx| {
                    shell
                        .pdf_pages
                        .as_ref()
                        .map(|(_, e)| e.read(cx).engine_error().map(str::to_owned))
                })
            })
            .expect("窗口更新");
        match view_err {
            Some(Some(e)) if e.contains("pdfium") => {
                eprintln!(
                    "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                     本机无 libpdfium——#386 退出面 e2e 未真跑。\n═════\
                     ════════════════════════════\n"
                );
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
            Some(Some(e)) => {
                panic!("页视图构造失败（夹具/真缺陷，非环境缺件）：{e}")
            }
            Some(None) => {}
            // 槽缺位 = 打开本身失败（同上一条口径：libpdfium 缺件 SKIP
            // 横幅；其他失败 panic 不放过——两处门文案见上）。
            None => {
                let notice = window
                    .update(cx, |_host, _window, cx| {
                        shell.update(cx, |shell, _| {
                            shell.notice_board.message().map(str::to_owned)
                        })
                    })
                    .expect("窗口更新");
                match notice {
                    Some(msg) if msg.contains("libpdfium") => {
                        eprintln!(
                            "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                             本机无 libpdfium——#386 退出面 e2e 未真跑\n  \
                             （pdf 打开面即失败：引擎绑定缺库）。\n  \
                             有 libpdfium 的环境（MDC_PDFIUM_LIB=<库路径> \
                             cargo test）全量执行。\n═══════════════════════════\n"
                        );
                        let _ = std::fs::remove_dir_all(&dir);
                        return;
                    }
                    other => {
                        panic!("pdf 标签未开（夹具/真缺陷，非环境缺件）：{other:?}")
                    }
                }
            }
        }

        // 手势置脏 → 模拟用户点窗口关闭钮（真实 should-close 回调链）
        //——确认框弹出 + 关闭被拦（批注未落盘不许静默退出）。
        let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
        vcx.update(|_window, cx| {
            shell.update(cx, |shell, cx| {
                let (_, entity) = shell.pdf_pages.clone().expect("槽在位");
                entity.update(cx, |v, _| {
                    v.commit_annotation(0, 40., 60., 200., 120.)
                        .expect("手势提交");
                });
            });
        });
        let closed = vcx.simulate_close();
        assert!(
            !closed,
            "#386：批注脏 → should-close 返回 false（窗口关闭被拦）"
        );
        vcx.update(|_window, cx| {
            shell.update(cx, |shell, cx| {
                let confirm = shell.confirm.as_ref().expect("退出前确认弹出");
                assert!(
                    matches!(confirm.action, ConfirmAction::Quit),
                    "确认动作 = Quit（得 {:?}）",
                    confirm.action
                );
                assert!(
                    confirm.message.contains("1"),
                    "文案含未保存计数（得 {:?}）",
                    confirm.message
                );
                assert_eq!(shell.tab_count(), 1, "被拦 = 标签还在（批注未静默丢）");
            });
        });

        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------------------
// #385 批 11：status_bar 视图切换钮交互测试（issue #585）——pdf/xlsx
// 双态翻转 + md 态零占位。
//
// 与既有面的分工（不重复钉定）：
// - `ui_wiring_faces` face 5：源码扫描（静态包含检查——测不出
//   「点了没反应」）；本模块补运行时面。
// - `pdf_annot_save_tests` / 批 2 PR #569：视图级；本模块钉
//   **状态栏入口**（文档形态切换钮）。
//
// 点击路径：**真实鼠标派发**（MouseDown + MouseUp 同点，完整命中
// 测试管线——与 main.rs run_panel_resize_script 烟雾同构）。按钮坐标
// 不硬编码（跨平台字体度量不可移植）：**状态反馈扫描**——从状态栏
// 右缘向左步进单击，目标态翻转即命中（右区其余成员均为无 hitbox
// 纯标签，点击零效果；pdf/xlsx 两钮按活动文档形态互斥在场，先中者
// 必为当前形态的切换钮）。
//
// 文案断言边界（如实登记）：钮标签是 `pdf_view_mode` /
// `xlsx_grid_view_mode` 的 1:1 纯函数（status_bar.rs 渲染期组串，
// 两文案等宽不位移）——本层断言状态翻转（= 文案翻转的判据源），
// 像素级文本呈现归 #554 真机走查，不冒充。
//
// 条件占位断言边界：md 态「双钮零渲染」与 toggle 守卫
// （`active_is_pdf`/`active_is_xlsx`）共享同一谓词（doc_kind 判据，
// 源码级同源）——本层运行时证据为「全扫零翻转」（用户可见行为：
// 点了没反应）；「渲染了但被守卫拦」与「未渲染」在该层不可区分，
// 属已知边界（如需像素级占位断言归 #554 截图回归）。
// ---------------------------------------------------------------------------
#[cfg(test)]
mod statusbar_view_toggle_tests {
    use super::*;
    use gpui::{
        AnyWindowHandle, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput,
        point, px,
    };

    /// 宿主：shell 进渲染树（鼠标命中测试需要整树 + 命中盒——
    /// pdf_search_wiring_tests WiringHost 同款；空 div 宿主会把状态栏
    /// 挡在派发树外，点击无法路由）。
    struct StatusHost(Entity<ShellApp>);

    impl gpui::Render for StatusHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
            gpui::div().size_full().child(self.0.clone())
        }
    }

    /// 整树宿主 + 窗口（theme provider 双注册——#422 同款：整树鼠标
    /// 派发需要 provider 在位）。
    fn host_window(
        cx: &mut gpui::TestAppContext,
    ) -> (Entity<ShellApp>, gpui::WindowHandle<StatusHost>) {
        cx.update(|app| {
            theme::init(theme::LoadThemes::JustBase, app);
            theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
        });
        let mut slot: Option<Entity<ShellApp>> = None;
        let window = cx.add_window(|window, cx| {
            let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
            slot = Some(shell.clone());
            StatusHost(shell)
        });
        (slot.expect("宿主构造即置"), window)
    }

    /// 双视图态快照（pdf_view_mode, xlsx_grid_view_mode——私有字段
    /// 同文件测试面直读）。
    fn view_modes(shell: &Entity<ShellApp>, cx: &mut gpui::TestAppContext) -> (bool, bool) {
        cx.read(|app| {
            let s = shell.read(app);
            (s.pdf_view_mode, s.xlsx_grid_view_mode)
        })
    }

    /// 重绘一帧（命中盒随最新状态重注册——update_window 口径）。
    fn redraw(window: &gpui::WindowHandle<StatusHost>, cx: &mut gpui::TestAppContext) {
        let any: AnyWindowHandle = window.clone().into();
        cx.update_window(any, |_, window, cx| {
            window.draw(cx).clear(cx);
        })
        .expect("重绘一帧");
    }

    /// 真实单击：MouseDown + MouseUp 同点派发（update_window 口径——
    /// 不包根视图 update，shell 级监听器不重入；main.rs 烟雾同构）。
    fn click_at(
        window: &gpui::WindowHandle<StatusHost>,
        cx: &mut gpui::TestAppContext,
        x: f32,
        y: f32,
    ) {
        let any: AnyWindowHandle = window.clone().into();
        let down = PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position: point(px(x), px(y)),
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        });
        let up = PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position: point(px(x), px(y)),
            modifiers: Modifiers::default(),
            click_count: 1,
        });
        cx.update_window(any, |_, window, cx| {
            window.dispatch_event(down, cx);
            window.dispatch_event(up, cx);
        })
        .expect("单击派发");
    }

    /// 状态栏右区命中扫描：从右缘（viewport 宽 - px_3 内边距）向左
    /// 步进 8px 单击至中线，直到**任一**视图态翻转。返回 (命中 x, 翻转后
    /// 双态)；扫完零翻转 = None（该形态无交互钮——md 态零占位的
    /// 运行时证据）。
    ///
    /// y = 状态栏中线（viewport 高 - 12：状态栏 = 底部 24px——
    /// 垂直方向只可能与状态栏命中盒相交，编辑区/面板/标签栏均在其上；
    /// 测试平台 display 为 1920×1080——坐标从 viewport_size 动态推
    /// 导，不硬编码）。
    fn sweep_statusbar_right(
        shell: &Entity<ShellApp>,
        window: &gpui::WindowHandle<StatusHost>,
        cx: &mut gpui::TestAppContext,
    ) -> Option<(f32, (bool, bool))> {
        let (right_edge, mid_x, status_y) = window
            .update(cx, |_host, window, _cx| {
                let size = window.viewport_size();
                (
                    f32::from(size.width) - 12.0,
                    f32::from(size.width) / 2.0,
                    f32::from(size.height) - 12.0,
                )
            })
            .expect("读 viewport 尺寸");
        redraw(window, cx);
        let mut x = right_edge;
        while x >= mid_x {
            let before = view_modes(shell, cx);
            click_at(window, cx, x, status_y);
            let after = view_modes(shell, cx);
            if after != before {
                return Some((x, after));
            }
            x -= 8.0;
        }
        None
    }

    /// smoke.xlsx 夹具路径（只读打开——本测试面不保存，零污染）。
    fn smoke_xlsx() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.xlsx")
    }

    /// 状态栏中线 y（viewport 高 - 12——底部 24px 状态栏的垂直中心；
    /// 回切重击前的坐标重取）。
    fn statusbar_y(window: &gpui::WindowHandle<StatusHost>, cx: &mut gpui::TestAppContext) -> f32 {
        window
            .update(cx, |_host, window, _cx| {
                f32::from(window.viewport_size().height) - 12.0
            })
            .expect("读 viewport 高")
    }

    /// xlsx 面：网格态起（初始 true）→ 扫描命中切换钮 → 翻转到文本态
    /// （false）→ 同点再击回切网格态（true）→ pdf 态全程不受扰
    /// （两钮互斥在场，点击不串道）。
    #[gpui::test]
    fn xlsx_view_toggle_click_flips_and_restores(cx: &mut gpui::TestAppContext) {
        let (shell, window) = host_window(cx);
        window
            .update(cx, |_host, window, cx| {
                shell.update(cx, |shell, cx| {
                    shell.open_document(smoke_xlsx(), window, cx);
                });
            })
            .expect("开 xlsx 标签");
        cx.run_until_parked();
        let (pdf0, xlsx0) = view_modes(&shell, cx);
        assert!(xlsx0, "xlsx 态初始 = 网格视图（ShellApp 默认 true）");
        assert!(pdf0, "pdf 态默认 true（未开 pdf 标签不受扰）");

        // 扫描命中（Some = 右区存在交互钮且点击真实翻转了状态）。
        let (hit_x, (pdf1, xlsx1)) =
            sweep_statusbar_right(&shell, &window, cx).expect("xlsx 标签活跃：右区应命中切换钮");
        assert!(!xlsx1, "第一击：网格 → 文本（true → false）");
        assert!(pdf1 == pdf0, "xlsx 钮点击不串扰 pdf 态");

        // 同点回切（文案两态等宽——按钮不位移；仍重绘保命中盒新鲜）。
        let status_y = statusbar_y(&window, cx);
        redraw(&window, cx);
        click_at(&window, cx, hit_x, status_y);
        let (pdf2, xlsx2) = view_modes(&shell, cx);
        assert!(xlsx2, "第二击：文本 → 网格（false → true，回切闭环）");
        assert!(pdf2 == pdf0, "回切仍不串扰 pdf 态");
    }

    /// md 面：非 pdf/xlsx 标签——右区全扫零翻转（双钮零占位的运行时
    /// 证据：用户点了没反应；占位渲染与 toggle 守卫同谓词，见模块
    /// 头「条件占位断言边界」）。
    #[gpui::test]
    fn md_tab_statusbar_right_zone_is_inert(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("mdc-b11-md-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let md = dir.join("note.md");
        std::fs::write(&md, "批 11\n状态栏零占位\n").unwrap();

        let (shell, window) = host_window(cx);
        window
            .update(cx, |_host, window, cx| {
                shell.update(cx, |shell, cx| {
                    shell.open_document(md.clone(), window, cx);
                });
            })
            .expect("开 md 标签");
        cx.run_until_parked();

        let before = view_modes(&shell, cx);
        let swept = sweep_statusbar_right(&shell, &window, cx);
        assert!(
            swept.is_none(),
            "md 标签：右区全扫零翻转（得 {:?}——存在不应在场的交互钮）",
            swept.map(|(x, _)| x)
        );
        assert_eq!(view_modes(&shell, cx), before, "双视图态纹丝不动");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// pdf 面：pdf 标签活跃 → 扫描命中 → 翻转回切（xlsx 态不串扰）。
    /// 引擎门控：无 libpdfium 时开档即失败（open 走 Failed 分支）→
    /// SKIP 横幅（AGENTS 13.1——不伪装通过）；#422 pdf_view_ctrl_f
    /// 同口径。
    #[gpui::test]
    fn pdf_view_toggle_click_flips_and_restores(cx: &mut gpui::TestAppContext) {
        let pdf_bytes = std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.pdf"),
        )
        .unwrap_or_else(|e| panic!("夹具不可读：{e}"));
        let dir = std::env::temp_dir().join(format!("mdc-b11-pdf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pdf = dir.join("s.pdf");
        std::fs::write(&pdf, pdf_bytes).unwrap();

        let (shell, window) = host_window(cx);
        window
            .update(cx, |_host, window, cx| {
                shell.update(cx, |shell, cx| {
                    shell.open_document(pdf.clone(), window, cx);
                });
            })
            .expect("开 pdf 标签");
        cx.run_until_parked();
        // 引擎门控：无 libpdfium → pdf_doc::load 失败 → 无标签 + notice。
        let open_ok = window
            .update(cx, |_host, _window, cx| {
                shell.update(cx, |shell, _| shell.tab_count() == 1)
            })
            .expect("查标签数");
        if !open_ok {
            let notice = window
                .update(cx, |_host, _window, cx| {
                    shell.update(cx, |shell, _| {
                        shell.notice_board.message().map(str::to_owned)
                    })
                })
                .expect("查 notice");
            if notice.as_deref().is_some_and(|m| m.contains("libpdfium")) {
                eprintln!(
                    "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                     本机无 libpdfium——批 11 pdf 视图切换钮 e2e 未真跑\n  \
                     （xlsx/md 两面已全量真跑——覆盖同一接线模式）。\n  \
                     有 libpdfium 的环境全量执行。\n═════════════════════════\n"
                );
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
            panic!("pdf 标签开档失败（夹具/真缺陷，非环境缺件）：{notice:?}");
        }

        let (pdf0, xlsx0) = view_modes(&shell, cx);
        assert!(pdf0, "pdf 态初始 = 页面视图（ShellApp 默认 true）");

        let (hit_x, (pdf1, xlsx1)) =
            sweep_statusbar_right(&shell, &window, cx).expect("pdf 标签活跃：右区应命中切换钮");
        assert!(!pdf1, "第一击：页面 → 文本（true → false）");
        assert!(xlsx1 == xlsx0, "pdf 钮点击不串扰 xlsx 态");

        let status_y = statusbar_y(&window, cx);
        redraw(&window, cx);
        click_at(&window, cx, hit_x, status_y);
        let (pdf2, _) = view_modes(&shell, cx);
        assert!(pdf2, "第二击：文本 → 页面（false → true，回切闭环）");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------------------
// #385 批 10（#582）：settings 模态与面板开合族生命周期交互测试
//（§六 质量门 1 扩面 Ⅱ——五面齐〔menu_bar/context_menu/palette/goto/
// tab〕后的遗留面）。宿主与派发模式同 menu_bar_lifecycle_tests /
// tab_workflow_tests：shell_window 宿主 + 真实 PlatformInput::KeyDown
// 派发管线（draw 一帧 → dispatch_event，键事件按焦点路径进
// shell-root on_key_down 监听链）。
// ---------------------------------------------------------------------------
#[cfg(test)]
mod settings_lifecycle_tests {
    use super::*;

    /// 测试宿主（**shell 必须进渲染树**——pdf_search_wiring_tests 的
    /// WiringHost 同款：空 div 宿主会把整棵树挡在派发树外，键事件
    /// 无处可派发）。
    struct Host(Entity<ShellApp>);

    impl gpui::Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
            gpui::div().size_full().child(self.0.clone())
        }
    }

    /// 空态 ShellApp + 宿主窗口（theme::init + provider 序列镜像
    /// pdf_search_wiring_tests——整树渲染需要 provider 在位；空态
    /// 焦点落 root〔shell.rs:439〕——键事件按焦点路径派发）。
    fn shell_window(cx: &mut gpui::TestAppContext) -> (Entity<ShellApp>, gpui::WindowHandle<Host>) {
        cx.update(|app| {
            theme::init(theme::LoadThemes::JustBase, app);
            theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
        });
        let mut slot: Option<Entity<ShellApp>> = None;
        let window = cx.add_window(|window, cx| {
            let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
            slot = Some(shell.clone());
            Host(shell)
        });
        (slot.expect("宿主构造即置"), window)
    }

    /// 真实 Esc 派发（tab_workflow_tests 的 Ctrl+F 烟雾同构——先强制
    /// 一帧绘制让派发树在位，再 dispatch_event；非直调方法断言）。
    fn press_escape(cx: &mut gpui::TestAppContext, window: &gpui::WindowHandle<Host>) {
        let any: gpui::AnyWindowHandle = (*window).into();
        cx.update_window(any, |_, window, cx| {
            window.draw(cx).clear(cx);
        })
        .expect("预绘一帧（派发树在位）");
        cx.update_window(any, |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                    keystroke: gpui::Keystroke {
                        modifiers: Default::default(),
                        key: "escape".into(),
                        key_char: None,
                    },
                    is_held: false,
                    prefer_character_input: false,
                }),
                cx,
            );
        })
        .expect("派发 Esc");
    }

    /// 验收 2：settings 开合循环——toggle 开 → **真实 Esc 派发**关 →
    /// 重开 → close_settings（Esc / 点遮罩 / 关闭钮同一条链的 shell
    /// 侧终点）关 → 再开再 Esc（循环可靠，非一次性巧合）。
    #[gpui::test]
    fn settings_modal_cycle_real_escape_dispatch(cx: &mut gpui::TestAppContext) {
        let (shell, window) = shell_window(cx);
        window
            .update(cx, |_host, _window, cx| {
                shell.update(cx, |shell, cx| {
                    assert!(!shell.settings_open, "前置：空态关闭");
                    shell.toggle_settings(cx);
                    assert!(shell.settings_open, "toggle 开");
                });
            })
            .expect("开");
        // 真实 Esc（焦点在 root——on_key_down 监听链收口）。
        press_escape(cx, &window);
        window
            .update(cx, |_host, _window, cx| {
                shell.update(cx, |shell, cx| {
                    assert!(
                        !shell.settings_open,
                        "真实 Esc 派发 → 关（shell-root 监听链）"
                    );
                    // 重开 → close_settings（遮罩/关闭钮同链终点）。
                    shell.toggle_settings(cx);
                    assert!(shell.settings_open, "重开");
                    shell.close_settings(cx);
                    assert!(
                        !shell.settings_open,
                        "close_settings 关（Esc/遮罩/关闭钮同链）"
                    );
                    // 再开 → 第二轮 Esc。
                    shell.toggle_settings(cx);
                });
            })
            .expect("重开");
        press_escape(cx, &window);
        window
            .update(cx, |_host, _window, cx| {
                shell.update(cx, |shell, _| {
                    assert!(!shell.settings_open, "第二轮真实 Esc 仍关（循环可靠）");
                });
            })
            .expect("断言");
    }

    /// 验收 3：**Esc 分派优先序锁定**——settings_open + confirm 并置
    /// （直接状态置位模拟「两弹窗同开」违规态；产线经 dirty 标签关闭/
    /// 退出路径产生，此处锁的是分派序不锁产径）：第一记 Esc 先关设置
    /// 且确认框保持（shell.rs on_key_down 序：菜单栏 > 右键菜单 >
    /// 设置弹窗 > 确认框 F4）；第二记 Esc 进 F4 模态拦截关确认框。
    /// 序即行为契约——倒序会让确认框「吞掉」设置关闭路径。
    #[gpui::test]
    fn escape_priority_settings_before_confirm(cx: &mut gpui::TestAppContext) {
        let (shell, window) = shell_window(cx);
        window
            .update(cx, |_host, _window, cx| {
                shell.update(cx, |shell, cx| {
                    shell.settings_open = true;
                    shell.confirm = Some(ConfirmState {
                        message: "有 1 个未保存的文档".to_string(),
                        detail: "退出前保存修改？".to_string(),
                        action: ConfirmAction::Quit,
                        selected: 0,
                        buttons: CONFIRM_BUTTONS.to_vec(),
                    });
                    cx.notify();
                });
            })
            .expect("并置态");
        press_escape(cx, &window);
        window
            .update(cx, |_host, _window, cx| {
                shell.update(cx, |shell, _| {
                    assert!(!shell.settings_open, "第一记 Esc：设置先关（分派序在前）");
                    assert!(shell.confirm.is_some(), "确认框保持（F4 拦截链未到）");
                });
            })
            .expect("断言①");
        press_escape(cx, &window);
        window
            .update(cx, |_host, _window, cx| {
                shell.update(cx, |shell, _| {
                    assert!(shell.confirm.is_none(), "第二记 Esc：F4 模态拦截关确认框");
                });
            })
            .expect("断言②");
    }

    /// 验收 4a：右缘 dock 开合——right_open 折叠/重开（状态级断言：
    /// 真实点击路径〔dock-toggle-right〕的等价状态终点；#289-⑤ 右栏
    /// 开关归右缘 dock 条）。
    #[gpui::test]
    fn right_dock_toggle_cycle(cx: &mut gpui::TestAppContext) {
        let (shell, _window) = shell_window(cx);
        shell.update(cx, |shell, cx| {
            assert!(shell.right_open, "前置：缺省开（shell.rs:387）");
            shell.toggle_right(cx);
            assert!(!shell.right_open, "折叠");
            shell.toggle_right(cx);
            assert!(shell.right_open, "重开");
        });
    }

    /// 验收 4b：左栏/git 视图开合双分支——toggle_left（git 态回切
    /// 文件树不折叠 / 非 git 态折叠展开）+ toggle_git（git 态折叠 /
    /// 非 git 态〔含栏折叠〕双置直达）。
    #[gpui::test]
    fn left_git_toggle_branches(cx: &mut gpui::TestAppContext) {
        let (shell, _window) = shell_window(cx);
        shell.update(cx, |shell, cx| {
            // 前置：left_open=true / git_view=false（shell.rs:386/401）。
            assert!(shell.left_open && !shell.git_view);

            // toggle_git 分支①：非 git 态 → 直达（开栏 + git 视图）。
            shell.toggle_git(cx);
            assert!(
                shell.left_open && shell.git_view,
                "toggle_git：非 git 态直达"
            );

            // toggle_left 分支①：git 态 → 回切文件树（栏不折叠）。
            shell.toggle_left(cx);
            assert!(
                shell.left_open && !shell.git_view,
                "toggle_left：git 态回切文件树"
            );

            // toggle_left 分支②：非 git 态 → 折叠/展开。
            shell.toggle_left(cx);
            assert!(!shell.left_open, "toggle_left：非 git 态折叠");
            shell.toggle_left(cx);
            assert!(shell.left_open, "toggle_left：重开");

            // toggle_git 分支②：git 态 → 折叠左栏（git_view 保持——
            // 重开直达 git 视图的状态记忆）。
            shell.toggle_git(cx);
            assert!(shell.left_open && shell.git_view, "再直达");
            shell.toggle_git(cx);
            assert!(
                !shell.left_open && shell.git_view,
                "toggle_git：git 态折叠左栏（git_view 保持）"
            );

            // 折叠态直达 git：双置（left_open + git_view）。
            shell.toggle_git(cx);
            assert!(
                shell.left_open && shell.git_view,
                "折叠态 toggle_git：双置直达"
            );
        });
    }
}

// ---------------------------------------------------------------------------
// #385 批 12（#590）：确认框模态交互测试——F4 拦截分支的键盘语义面
//（§六 质量门 1 扩面 Ⅲ；stacked on 批 10 的 settings_lifecycle_tests
// ——宿主/派发模式同款）。批 10 已锁 Esc 分派优先序；本模块锁确认框
// 自身：Tab/←/→ 选择循环（三钮 + 二钮形）+ Enter 取消路径（no-op
// 零副作用）+ Esc 单独在场的完整关闭链（焦点回落 root）。
// ---------------------------------------------------------------------------
#[cfg(test)]
mod confirm_lifecycle_tests {
    use super::*;

    /// 测试宿主（shell 必须进渲染树——批 10 同款）。
    struct Host(Entity<ShellApp>);

    impl gpui::Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
            gpui::div().size_full().child(self.0.clone())
        }
    }

    /// 空态 ShellApp + 宿主窗口（theme::init + provider——整树渲染）。
    fn shell_window(cx: &mut gpui::TestAppContext) -> (Entity<ShellApp>, gpui::WindowHandle<Host>) {
        cx.update(|app| {
            theme::init(theme::LoadThemes::JustBase, app);
            theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
        });
        let mut slot: Option<Entity<ShellApp>> = None;
        let window = cx.add_window(|window, cx| {
            let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
            slot = Some(shell.clone());
            Host(shell)
        });
        (slot.expect("宿主构造即置"), window)
    }

    /// 真实键派发（批 10 的 press_escape 同构——键名参数化）。
    fn press_key(cx: &mut gpui::TestAppContext, window: &gpui::WindowHandle<Host>, key: &str) {
        let any: gpui::AnyWindowHandle = (*window).into();
        cx.update_window(any, |_, window, cx| {
            window.draw(cx).clear(cx);
        })
        .expect("预绘一帧（派发树在位）");
        cx.update_window(any, |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                    keystroke: gpui::Keystroke {
                        modifiers: Default::default(),
                        key: key.into(),
                        key_char: None,
                    },
                    is_held: false,
                    prefer_character_input: false,
                }),
                cx,
            );
        })
        .expect("派发 {key}");
    }

    /// 置三钮确认态（Quit 形——selected 起始 0）。
    fn set_confirm(shell: &Entity<ShellApp>, cx: &mut gpui::TestAppContext) {
        shell.update(cx, |shell, cx| {
            shell.confirm = Some(ConfirmState {
                message: "有 1 个未保存的文档".to_string(),
                detail: "退出前保存修改？".to_string(),
                action: ConfirmAction::Quit,
                selected: 0,
                buttons: CONFIRM_BUTTONS.to_vec(),
            });
            cx.notify();
        });
    }

    /// 读当前选中序（MAX = 无确认态——TestAppContext 经 update 桥读）。
    fn selected(shell: &Entity<ShellApp>, cx: &mut gpui::TestAppContext) -> usize {
        cx.update(|app| {
            shell
                .read(app)
                .confirm
                .as_ref()
                .map(|c| c.selected)
                .unwrap_or(usize::MAX)
        })
    }

    /// 验收 2a：三钮（保存/不保存/取消）选择循环——真实 Tab ×3
    /// 前环回（0→1→2→0），Left 后环回（0→2），Right 前环回（2→0）。
    #[gpui::test]
    fn confirm_three_button_cycling_real_keys(cx: &mut gpui::TestAppContext) {
        let (shell, window) = shell_window(cx);
        set_confirm(&shell, cx);
        assert_eq!(selected(&shell, cx), 0, "前置：selected=0（保存侧）");

        // Tab 前循环 ×3：0 → 1 → 2 → 0（环回）。
        for expect in [1usize, 2, 0] {
            press_key(cx, &window, "tab");
            assert_eq!(selected(&shell, cx), expect, "Tab 前循环（含 2→0 环回）");
        }
        // Left 后循环：0 → 2（跨 0 环回到取消侧）。
        press_key(cx, &window, "left");
        assert_eq!(selected(&shell, cx), 2, "Left 后循环（0→2 环回）");
        // Right 前循环：2 → 0。
        press_key(cx, &window, "right");
        assert_eq!(selected(&shell, cx), 0, "Right 前循环（2→0）");
    }

    /// 验收 2b：二钮形（删除确认 0 删除 / 1 取消）Tab 环回。
    #[gpui::test]
    fn confirm_two_button_cycling_real_keys(cx: &mut gpui::TestAppContext) {
        let (shell, window) = shell_window(cx);
        shell.update(cx, |shell, cx| {
            shell.confirm = Some(ConfirmState {
                message: "删除最近记录？".to_string(),
                detail: "不可撤销".to_string(),
                action: ConfirmAction::DeletePath {
                    path: PathBuf::from("/tmp/不存在的夹具"),
                },
                selected: 0,
                buttons: vec!["删除", "取消"],
            });
            cx.notify();
        });
        assert_eq!(selected(&shell, cx), 0, "前置：selected=0（删除侧）");
        press_key(cx, &window, "tab");
        assert_eq!(selected(&shell, cx), 1, "二钮 Tab：0→1（取消侧）");
        press_key(cx, &window, "tab");
        assert_eq!(selected(&shell, cx), 0, "二钮 Tab 环回：1→0");
        press_key(cx, &window, "left");
        assert_eq!(selected(&shell, cx), 1, "二钮 Left：0→1（后环回）");
    }

    /// 验收 3：Enter 取消路径（selected=2 → resolve_confirm 的
    /// `_ => {}` no-op）——confirm 清空 + 焦点回落 root + 零副作用
    ///（空态 ShellApp：无标签可关、无路径可删——Quit 取消侧唯一
    /// 可观测行为就是模态消散）。Quit 与 CloseTab 两动作同面。
    #[gpui::test]
    fn confirm_enter_cancel_path_noop(cx: &mut gpui::TestAppContext) {
        let (shell, window) = shell_window(cx);
        // Quit 形 + selected=2（取消）。
        set_confirm(&shell, cx);
        shell.update(cx, |shell, cx| {
            if let Some(c) = shell.confirm.as_mut() {
                c.selected = 2;
            }
            cx.notify();
        });
        press_key(cx, &window, "enter");
        shell.update(cx, |shell, _| {
            assert!(
                shell.confirm.is_none(),
                "Enter 取消：confirm 清空（no-op 决断）"
            );
        });
        // CloseTab 形 + selected=2（取消）——idx 越界不可达（取消侧
        // 不触 close_tab_at），同样只清模态。
        shell.update(cx, |shell, cx| {
            shell.confirm = Some(ConfirmState {
                message: "标签有未保存的修改".to_string(),
                detail: "关闭前保存？".to_string(),
                action: ConfirmAction::CloseTab { idx: 0 },
                selected: 2,
                buttons: CONFIRM_BUTTONS.to_vec(),
            });
            cx.notify();
        });
        press_key(cx, &window, "enter");
        shell.update(cx, |shell, _| {
            assert!(
                shell.confirm.is_none(),
                "Enter 取消（CloseTab 形）：模态消散，idx 不被触达"
            );
            assert!(shell.tab_count() == 0, "空态零标签——取消路径零副作用的自证");
        });
    }

    /// 验收 4：Esc 单独在场时的完整关闭链——真实 Esc → confirm 清空
    ///（与批 10 优先序测试互补：那里锁的是「设置先于确认框」的分派
    /// 序，这里锁确认框自己的关闭 + 焦点回落可重复）。
    #[gpui::test]
    fn confirm_escape_full_close_chain(cx: &mut gpui::TestAppContext) {
        let (shell, window) = shell_window(cx);
        set_confirm(&shell, cx);
        press_key(cx, &window, "escape");
        shell.update(cx, |shell, _| {
            assert!(shell.confirm.is_none(), "Esc：confirm 清空");
        });
        // 可重复：再开再关。
        set_confirm(&shell, cx);
        press_key(cx, &window, "escape");
        shell.update(cx, |shell, _| {
            assert!(shell.confirm.is_none(), "第二轮 Esc 仍关（循环可靠）");
        });
        // 关后 Enter 不再被 F4 拦截（模态消散后键位回归正常分发——
        // 空态 Enter 落 keymap 兜底零副作用，只断言 confirm 保持无）。
        press_key(cx, &window, "enter");
        shell.update(cx, |shell, _| {
            assert!(shell.confirm.is_none(), "模态消散后 Enter 零副作用");
        });
    }
}
