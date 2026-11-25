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

#[cfg(test)]
#[path = "home_actions_tests.rs"]
mod home_actions_tests;
#[path = "home_shell.rs"]
mod home_shell;
#[path = "home_view.rs"]
mod home_view;
#[cfg(test)]
#[path = "shell_tests/notice_tests.rs"]
mod notice_tests;
#[cfg(test)]
#[path = "shell_tests/statusbar_labels_runtime_tests.rs"]
mod statusbar_labels_runtime_tests;
#[cfg(test)]
#[path = "shell_tests/statusbar_view_toggle_tests.rs"]
mod statusbar_view_toggle_tests;
#[cfg(test)]
#[path = "shell_tests/tab_workflow_tests.rs"]
mod tab_workflow_tests;

#[cfg(test)]
#[path = "shell_tests/confirm_lifecycle_tests.rs"]
mod confirm_lifecycle_tests;
#[cfg(test)]
#[path = "shell_tests/context_menu_lifecycle_tests.rs"]
mod context_menu_lifecycle_tests;
#[cfg(test)]
#[path = "shell_tests/g4_custom_theme_tests.rs"]
mod g4_custom_theme_tests;
#[cfg(test)]
#[path = "shell_tests/goto_symbol_108_tests.rs"]
mod goto_symbol_108_tests;
#[cfg(test)]
#[path = "shell_tests/menu_bar_lifecycle_tests.rs"]
mod menu_bar_lifecycle_tests;
#[cfg(test)]
#[path = "shell_tests/mtime_poll_tests.rs"]
mod mtime_poll_tests;
#[cfg(test)]
#[path = "shell_tests/page_view_action_honesty_tests.rs"]
mod page_view_action_honesty_tests;
#[cfg(test)]
#[path = "shell_tests/pdf_annot_save_tests.rs"]
mod pdf_annot_save_tests;
#[cfg(test)]
#[path = "shell_tests/pdf_search_wiring_tests.rs"]
mod pdf_search_wiring_tests;
#[cfg(test)]
#[path = "shell_tests/settings_lifecycle_tests.rs"]
mod settings_lifecycle_tests;
#[cfg(test)]
#[path = "shell_tests/xlsx_save_close_tests.rs"]
mod xlsx_save_close_tests;

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
// #109 批②③：项目搜索面板（左栏 dock 均面板——工作区打开时物化）。
use crate::menu_bar;
use crate::menu_bar_view;
use crate::outline_panel::OutlinePanel;
use crate::search_panel::{SearchPanel, SearchPanelHooks};
// #217：键位表（shell 键钩子的单一事实源——默认表 = 原分支精确镜像）。
use crate::keymap::{self, ShellAction};
use crate::panels;
use crate::pdf_pages::PdfPagesView;
use crate::pdf_thumbs::{PdfThumbnailsView, PdfThumbsHooks};
// #50 C1：docx 页面视图（LOK 内核真源渲染——会话层在
// editor::docx_lok_session）。
use crate::docx_pages::DocxPagesView;
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

/// #306 ①：OCR 识别线程 → 前台回流消息（16ms 轮询 drain——
/// pdf_pages::SearchIndexMsg 同款纪律）。
#[derive(Debug)]
enum OcrProgressMsg {
    /// 单页完成（done/total 1 基；words = 累计词数）。
    Page {
        done: usize,
        total: usize,
        words: usize,
    },
    /// 全量完成（error = 环境性失败[首页即败，结果不应用]；failed_pages
    /// = 逐页失败清单——空词占位，完成横幅诚实计数）。
    Done {
        error: Option<String>,
        pages: Vec<Vec<mdc_pdf::OcrWord>>,
        word_count: usize,
        failed_pages: Vec<usize>,
        lang: String,
    },
}

/// #306 ①：OCR 进度轮询间隔（pdf_pages::POLL_INTERVAL 同款 16ms）。
const OCR_POLL_INTERVAL: Duration = Duration::from_millis(16);

/// F4：待确认动作（脏标签关闭 / 退出 / #166 文件树删除）。
#[derive(Clone, Debug)]
enum ConfirmAction {
    /// `target` is authoritative; `idx` only records the original prompt position.
    CloseTab { idx: usize, target: gpui::EntityId },
    /// 退出应用（存在未保存修改的标签）。
    Quit,
    /// #166：永久删除文件树目标（确认后不可恢复——无回收站 API，
    /// gpui 闭包未暴露 trash 面）。
    DeletePath { path: PathBuf },
    /// #107 P1②：拖拽移动文件树目标（确认后 fs::rename + 前缀重写
    /// 打开标签路径）。
    MovePath { old: PathBuf, new: PathBuf },
    /// #109 批③：项目搜索全部替换（预览确认后执行——replace_all +
    /// 打开标签 mtime 重载善后）。
    SearchReplaceAll {
        pattern: String,
        replacement: String,
        options: crate::project_search::SearchOptions,
    },
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
    right_open: bool,
    document_info_open: bool,
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
    /// Snapshot of target identities + clean-only intent; cancel/failure drops the tail.
    close_queue: std::collections::VecDeque<(gpui::EntityId, bool)>,
    /// Session-local (20 max), file paths only; reopening always reads disk.
    closed_tabs: Vec<PathBuf>,
    /// F2：工作区文件树（打开文件夹后呈现；记忆落盘随 settings）。
    workspace: Option<Entity<FileTree>>,
    /// F7：git 面板（工作区打开即建——非 git 目录走面板内提示态）。
    git_panel: Option<Entity<GitPanel>>,
    /// #109 批②③：项目搜索面板实体（工作区打开时物化；无工作区 = None）。
    search_panel: Option<Entity<SearchPanel>>,
    /// #119（mdc-dock 批①）：左栏 dock 实体——文件树/git 视图切换经
    /// 统一激活协议（`git_view` 布尔退役）。面板未物化（无工作区）时
    /// `activate_key` 记录预物化意图，渲染面据 `active_key` 呈占位视图
    /// ——与旧 git_view=true + git_panel=None 的占位语义一比一等价。
    /// #120（批②）：左栏开合态（原 `left_open` 布尔退役）与 per-panel
    /// 记忆尺寸随 [`mdc_dock::DockAreaState`] 持久化（settings `dock` 节点）。
    left_dock: Entity<mdc_dock::Dock>,
    /// #121（批③b）：右栏 dock 实体（Right 位——拖拽换侧的真实终点；
    /// 批③a 数据面 take/receive 协议的目标宿主）。
    right_dock: Entity<mdc_dock::Dock>,
    /// #121（批③b）：拖拽会话当前分区（`on_drag_move` 全窗口追踪——
    /// 覆盖层三分区高亮的数据源；`None` = 非拖拽中）。
    drag_zone: Option<mdc_dock::DockDropZone>,
    /// #571：大纲面板实体（批 7——激活标签的键盘导航结构树；
    /// 换绑判据见 [`Self::sync_outline`]）。
    outline: Option<Entity<OutlinePanel>>,
    /// #110 G4：自定义主题色草稿（6 高频槽 hex 文本——设置弹窗
    /// 「自定义主题色」子区状态；空串 = 未编辑（渲染时取当前主题
    /// 实际色为初值））。
    custom_theme_draft: [String; 31],
    /// #110 G1 批B：主题名输入框草稿（「保存为主题」用——空 = my-theme
    /// 旧行为兼容；输入名即 themes/<name>.json 文件名与 registry 名）。
    theme_name_input: String,
    /// #110 G4：六槽输入焦点句柄池（键捕获范式——构造时一槽一句柄）。
    color_slot_focus: Vec<gpui::FocusHandle>,
    /// #289-⑤：设置弹窗开关（模态——并替代旧 `settings_view` 右栏视图；
    /// 左条设置钮/菜单「设置」打开，Esc/点遮罩/关闭钮关闭）。
    settings_open: bool,
    /// #51 D4b：pdf 页视图状态快照（路径 → （缩放档, 顶部页）——切
    /// 标签重建实体时保留/回放：「当前页与缩放状态一致」——回来还在
    /// 原页原档，不从首页 FitWidth 重来）。
    pdf_view_snapshots: std::collections::HashMap<PathBuf, (crate::pdf_pages::ZoomMode, usize)>,
    /// #289-④：pdf 页面视图（当前 pdf 标签的位图渲染实体 + 归属路径
    /// ——切到不同 pdf 标签时重建；关闭该标签时丢弃）。
    pdf_pages: Option<(PathBuf, Entity<PdfPagesView>)>,
    /// #420：pdf 缩略图侧栏（与 pdf_pages 实体同生命周期：同路径重建/
    /// 关标签/装载失败一并丢弃；主视图 error 态不挂——缺 pdfium 环境
    /// 无空壳侧栏）。联动经 PdfThumbsHooks 双向钩子（点击跳页/高亮
    /// 跟随——见 ensure_pdf_pages 接线）。
    pdf_thumbs: Option<Entity<PdfThumbnailsView>>,
    /// #289-④：pdf 显示模式（true = 页面位图视图——缺省；false =
    /// 文本行模型。状态栏「页面/文本」钮切换；两视图各有查找面
    /// （#422：页面视图内查找条，不再强制切文本）。
    pdf_view_mode: bool,
    /// #50 C1：docx 页面视图（当前 docx 标签的 LOK 内核渲染实体 +
    /// 归属路径——生命周期与 pdf_pages 同模式：切标签重建/关标签
    /// 丢弃；会话随实体 Drop 拆除）。
    docx_pages: Option<(PathBuf, Entity<DocxPagesView>)>,
    /// #50 C1：docx 显示模式（true = LOK 页面视图——缺省；false =
    /// 行投影文本模型。状态栏「页面/文本」钮切换；文本态未保存编辑
    /// 拒切页面态（toggle 卫——单一编辑真源）。
    docx_view_mode: bool,
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
    /// #306 ①：OCR 识别回流通道（专用线程 → 逐页进度；Done 携全量词
    /// 表——pdf_pages 搜索索引同款纪律：通道 + 16ms 轮询 drain）。
    ocr_rx: Option<std::sync::mpsc::Receiver<OcrProgressMsg>>,
    /// #306 ①：OCR 轮询循环在跑（Done 消费即停——防重复起循环）。
    ocr_polling: bool,
    /// #107 P1①：mtime 轮询在跑标记（tabs 非空期启动；空则停——
    /// 外部变更感知面 D1 后半）。
    mtime_polling: bool,
    /// #107 P1①：各标签磁盘 mtime 基线（轮询比对——打开/重载/保存后
    /// 刷新；None = 不可得（跳过该标签比对））。
    mtime_baselines: Vec<Option<std::time::SystemTime>>,
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
    /// Home action identities survive renders and picker cancellation.
    home_focus: [FocusHandle; 3],
    /// F5：最近打开（文件 20 / 工作区 10——空态引导页呈现 + 写穿透落盘）。
    recent: RecentStore,
    /// F6：主题模式（system/light/dark——Ctrl+Alt+T 循环；持久化）。
    /// #110 G1 批B：四态轴（"system"/"light"/"dark"/"user:<name>"——String）。
    theme_mode: String,
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
    /// #111 P2：软换行开关（settings `soft_wrap` 键——set_soft_wrap_all
    /// 传播全部标签；沿 set_indent_unit_all 先例）。
    soft_wrap: bool,
    /// F1：提示横幅（打开失败 / 类型不支持 / 对话框不可用等面向用户消息）。
    notice_board: NoticeBoard,
    /// #63 打磨（#385 批 1）：右键菜单实体（`ui::ContextMenu` 组件，
    /// None = 未打开）。键盘导航/焦点/ARIA/dismiss 由组件自带；动作
    /// 派发回 shell 的 activate_context_menu_item（id 派发面不变）。
    context_menu: Option<Entity<ui::ContextMenu>>,
    context_menu_generation: u64,
    context_menu_previous_focus: Option<FocusHandle>,
    context_menu_focus: Option<FocusHandle>,
    context_tab_target: Option<(gpui::EntityId, Vec<gpui::EntityId>)>,
    context_menu_subscription: Option<gpui::Subscription>,
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
    Tab,
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
        let workspace = workspace_root.clone().map(|root| {
            let shell = cx.entity().downgrade();
            cx.new(|cx| FileTree::new(root.clone(), Self::file_tree_hooks(shell.clone()), cx))
        });
        // #109 批②③：启动恢复的工作区同步建搜索面板（root 换绑在内）。
        let search_panel = workspace_root.map(|root| {
            let shell = cx.entity().downgrade();
            cx.new(|cx| {
                let mut panel = SearchPanel::new(Self::search_panel_hooks(shell), cx);
                panel.set_root(Some(root), cx);
                panel
            })
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
        // #110 G1 批B：四态持久化（三态键或 user:<name>——坏值回 system）。
        let theme_mode_raw = crate::settings::load_theme_mode()
            .unwrap_or_else(|| theme_switch::MODE_SYSTEM.to_string());
        let theme_mode = if theme_switch::is_user_mode(&theme_mode_raw)
            || theme_mode_raw == theme_switch::MODE_SYSTEM
            || theme_mode_raw == theme_switch::MODE_LIGHT
            || theme_mode_raw == theme_switch::MODE_DARK
        {
            theme_mode_raw
        } else {
            theme_switch::MODE_SYSTEM.to_string()
        };
        theme_switch::apply_mode(&theme_mode, &base_theme, cx);
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
                    .filter(|(i, _tab)| {
                        // #604：退出前脏收集与 tab_is_modified 同源（单一
                        // 口径——xlsx 会话脏「活实体/银行」、#386 pdf 批注
                        // 脏一并计入；此前散写两谓词漏 xlsx，会话脏静默
                        // 丢）。#389 阶段 2 的标签脏判定即此函数。
                        shell.read(cx).tab_is_modified(*i, cx).unwrap_or(false)
                    })
                    .map(|(i, _)| i)
                    .collect();
                if dirty.is_empty() {
                    return true;
                }
                shell.update(cx, |shell, cx| {
                    shell.close_context_menu(cx);
                    shell.close_menu_bar_internal(cx);
                    shell.close_queue.clear();
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
            right_open: crate::settings::load_dock_layout()
                .map(|(_, r)| r)
                .unwrap_or(true),
            document_info_open: false,
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
            close_queue: Default::default(),
            closed_tabs: Vec::new(),
            workspace,
            git_panel: None,
            search_panel: search_panel.clone(),
            outline: None,
            custom_theme_draft: Default::default(),
            theme_name_input: String::new(),
            color_slot_focus: (0..31).map(|_| cx.focus_handle()).collect(),
            // #119（mdc-dock 批①）：左栏激活协议实体；#120（批②）：
            // 布局恢复在构造尾（面板注册之后——见下）。
            left_dock: cx.new(|cx| mdc_dock::Dock::new(mdc_dock::DockPosition::Left, cx)),
            // #121 批③b：右栏 dock（Right 位）——拖拽换侧终点。
            right_dock: cx.new(|cx| mdc_dock::Dock::new(mdc_dock::DockPosition::Right, cx)),
            drag_zone: None,
            settings_open: false,
            pdf_pages: None,
            pdf_thumbs: None,
            ocr_rx: None,
            ocr_polling: false,
            mtime_polling: false,
            mtime_baselines: Vec::new(),
            pdf_view_mode: true,
            pdf_view_snapshots: std::collections::HashMap::new(),
            docx_pages: None,
            docx_view_mode: true,
            xlsx_grid: None,
            xlsx_sessions: std::collections::HashMap::new(),
            xlsx_grid_view_mode: true,
            perf_first_frame: crate::perf::on().then(crate::perf::boot),
            chord: None,
            confirm: None,
            confirm_focus,
            root_focus,
            home_focus: [
                cx.focus_handle().tab_stop(true),
                cx.focus_handle().tab_stop(true),
                cx.focus_handle().tab_stop(true),
            ],
            recent,
            theme_mode: theme_mode.clone(),
            base_theme,
            font_size,
            indent_unit: crate::settings::load_indent_unit()
                .as_deref()
                .map(IndentUnit::from_setting)
                .unwrap_or_default(),
            soft_wrap: crate::settings::load_soft_wrap().unwrap_or(true),
            notice_board: NoticeBoard::default(),
            context_menu: None,
            context_menu_generation: 0,
            context_menu_previous_focus: None,
            context_menu_focus: None,
            context_tab_target: None,
            context_menu_subscription: None,
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
        // #119（mdc-dock 批①）：构造期工作区（F2 记忆恢复/CLI 参数）的
        // 树面板注册进左栏 dock（不激活——active 无即文件树视图缺省渲染
        // 语义，与旧 git_view=false 起始一比一等价；git 面板沿旧例不在
        // 构造期建——预物化意图承接其占位语义）。
        if let Some(tree) = this.workspace.clone() {
            this.left_dock.update(cx, |dock, cx| {
                dock.add_panel(&tree, cx);
            });
        }
        // #109 批②③：启动工作区的搜索面板同批注册（不激活——视图
        // 归用户记忆/缺省文件树；激活键激活时面板可聚焦查询框）。
        if let Some(search) = this.search_panel.clone() {
            this.left_dock.update(cx, |dock, cx| {
                dock.add_panel(&search, cx);
            });
        }
        // #120（批②）：布局恢复——**面板注册之后**（记忆对已物化面板
        // 生效：开合态/激活键/per-panel 尺寸；未物化键静默丢弃——
        // 与旧「重启回默认全开」相比仅新增能力，零行为回退）。版本
        // 不符 → 整份丢弃回默认（Q-IDE layout.rs:518-521 口径）。
        if let Some((state, _)) = crate::settings::load_dock_layout() {
            let dock = this.left_dock.clone();
            let _ = dock.update(cx, |dock, cx| dock.restore(state, cx));
        }
        // #121 批③b：右 dock 布局恢复（additive "right" 键——旧文件缺键
        // 走 None 默认，兼容）。
        if let Some(right_state) = crate::settings::load_right_dock_layout() {
            let dock = this.right_dock.clone();
            let _ = dock.update(cx, |dock, cx| dock.restore(right_state, cx));
        }
        // #120：左栏渲染宽同步到恢复后的激活面板记忆宽。
        this.sync_left_width_to_active(cx);
        // #175：启动 FILE 路径的页面视图接线——三 ensure 与
        // `open_document_tab` 尾部同源同序（此前仅标签路径调用：双击
        // 文件/命令行直给文件时 docx/pdf/xlsx 主显示面静默缺失，用户
        // 拿到文本投影态 = 「打开没渲染」真机症状；产品意图「打开即
        // 页面视图」——`docx_view_mode` 等缺省 true——在启动路径兑现）。
        this.ensure_pdf_pages(cx);
        this.ensure_docx_pages(cx);
        this.ensure_xlsx_grid(cx);
        // 主显示面为页视图/网格时焦点改落视图实体（焦点路由单源
        // `focus_active`——编辑器不在渲染树，句柄游离则键事件死在
        // 合成根，#422/#604 同源病灶）。
        this.focus_active(window, cx);
        // #175 证据面：启动路径接线结果（真机双击验收的 stderr 断言
        // 行——与标签路径的 "docx-pages: ready" 互补，区分两条打开
        // 通道；空启动不打印）。
        if let Some(tab) = this.tabs.first() {
            eprintln!(
                "startup: page-view wiring kind={} docx_pages={} pdf_pages={} xlsx_grid={}",
                tab.editor.read(cx).doc_kind(),
                this.docx_pages.is_some(),
                this.pdf_pages.is_some(),
                this.xlsx_grid.is_some(),
            );
        }
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
                // #50 C3/C4：docx 页面视图脏（EditDone 回执驱动的真实
                // 脏位——关闭确认/退出确认从此对 LOK 编辑生效）。
                || self.docx_page_dirty(cx, &tab.path)
        })
    }

    /// #50 C3/C4：指定路径的 docx 页面视图是否脏（Shell 持有的
    /// `docx_pages` 槽按路径匹配——无页视图/路径不匹配 = 不脏）。
    fn docx_page_dirty(&self, cx: &App, path: &Path) -> bool {
        self.docx_pages
            .as_ref()
            .is_some_and(|(owned, entity)| *owned == path && entity.read(cx).is_dirty())
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
    /// - **#604：xlsx 标签 → [`Self::save_xlsx_at`] 编辑会话统一收口**
    ///   （网格活实体 / 银行会话——Ctrl+S 键位/菜单「文件 → 保存」/
    ///   关闭确认「保存」/退出确认四路全在此分流）。文本面只读，
    ///   `Editor::save` 对 xlsx 恒 `Unmodified`（#386 同款假成功——
    ///   确认框会带着未落盘的网格编辑直接关标签，静默丢数据）。
    /// - 其余（含 pdf 无页视图/页视图干净）→ [`Editor::save`] 三态；
    ///   `ReadOnlyRejected`/`Failed` toast 示警。
    ///
    /// 返回三态供调用方（Ctrl+S 反馈 / 确认框决策：`Saved`/
    /// `Unmodified` 才可关标签——`Failed` 拒关，脏态保留）。
    pub fn save_tab(&mut self, idx: usize, cx: &mut Context<Self>) -> SaveOutcome {
        // #604：xlsx 分流前置（键位/命令/双确认四路收口点——活跃与
        // 非活跃标签同径：活实体在场走实体保存，否则银行会话直存）。
        // 脏在保存前采样（保存成功即清——区分 Saved/Unmodified 两态，
        // 确认框只认这两态关标签）。
        let xlsx_path = self
            .tabs
            .get(idx)
            .filter(|tab| tab.editor.read(cx).doc_kind() == "xlsx")
            .map(|tab| tab.path.clone());
        if let Some(path) = xlsx_path {
            let was_dirty = self.xlsx_dirty_at(&path, cx);
            let active_grid = self
                .xlsx_grid
                .as_ref()
                .is_some_and(|(owned, _)| *owned == path);
            return if self.save_xlsx_at(&path, cx) {
                if was_dirty {
                    self.show_notice("已保存（xlsx 编辑会话）".into(), cx);
                    SaveOutcome::Saved
                } else {
                    // 无编辑（只读浏览/已存过）——与「保存了但没变化」
                    // 同语义，允许关标签。
                    SaveOutcome::Unmodified
                }
            } else if active_grid {
                // 活实体路径：具体错误文案在网格面板状态条（view.save
                // 的 notice 面）——shell 面补同源 toast；银行路径
                // save_xlsx_at 已 toast 具体错误，不重复弹。
                self.show_notice("xlsx 保存失败——见网格面板状态条".into(), cx);
                SaveOutcome::Failed
            } else {
                SaveOutcome::Failed
            };
        }
        let is_pdf = self
            .tabs
            .get(idx)
            .is_some_and(|tab| tab.editor.read(cx).doc_kind() == "pdf");
        // #50 C1：docx 页面态卫——LOK 页面视图 C1 只读渲染（保存面 C4
        // 接线），且切换卫保证行投影恒干净 → Unmodified。**不走**
        // Editor::save：docx 引擎保存无 dirty 卫，会在 LOK 会话脚下盲
        // 目重写盘上文件（破坏单一编辑真源）。
        let docx_page_owned = self
            .tabs
            .get(idx)
            .filter(|tab| tab.editor.read(cx).doc_kind() == "docx")
            .filter(|_| self.docx_view_mode)
            .and_then(|tab| {
                self.docx_pages
                    .as_ref()
                    .filter(|(owned, _)| *owned == tab.path)
                    .map(|_| tab.path.clone())
            });
        if let Some(owned) = docx_page_owned {
            // #50 C3/C4：页面态保存改走 LOK 会话（saveAs 回源路径 +
            // 修订号对账清 dirty——C1 时期的只读提示退役：编辑面已接）。
            // 干净态短路（Unmodified——关闭/退出三态语义保持）；脏态发
            // `request_save`（异步回执 SaveDone——对账后自动清 dirty +
            // toast；本调用返回 Saved 表「保存已发起」——失败面经回执
            // toast 显式示警，不静默不假绿）。
            let dirty = self
                .docx_pages
                .as_ref()
                .is_some_and(|(o, e)| *o == owned && e.read(cx).is_dirty());
            if !dirty {
                // #247②：干净态也提示（#176 不静默精神——页面视图下按
                // 保存应给用户可感知反馈，而非无声 no-op。文案同时含
                //「页面视图」与「只读渲染」锚词：save_tab 断言面与
                // Ctrl+S 键管线断言面共用一条）。
                self.show_notice("页面视图无修改（只读渲染——切文本视图后可编辑）".into(), cx);
                return SaveOutcome::Unmodified;
            }
            let sent = self
                .docx_pages
                .as_ref()
                .map(|(_, e)| e.clone())
                .map(|entity| entity.update(cx, |view, cx| view.request_save(cx)))
                .unwrap_or(false);
            if !sent {
                self.show_notice("保存未发起——LOK 会话不可用（详见应用日志）".into(), cx);
                return SaveOutcome::Unmodified;
            }
            self.show_notice("保存中（LOK 内核写盘——完成后自动提示）".into(), cx);
            return SaveOutcome::Saved;
        }
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
    /// 在 git 视图时切回文件树视图（F7）。#120（批②）：开合态归
    /// dock（`set_open` 协议）+ 持久化。
    pub fn toggle_left(&mut self, cx: &mut Context<Self>) {
        if self.left_open(cx) && self.git_active(cx) {
            // #119（mdc-dock 批①）：回切文件树视图经统一激活协议
            //（原 git_view 布尔置位退役）。
            self.activate_left_view(crate::file_tree::FILE_TREE_PANEL_KEY, cx);
        } else {
            let dock = self.left_dock.clone();
            dock.update(cx, |dock, cx| {
                let next = !dock.is_open();
                dock.set_open(next, cx);
            });
        }
        self.persist_dock_layout(cx);
        cx.notify();
    }

    /// F7：切到 git 视图（图标条 `act-git` 触发——已开且在 git 视图时
    /// 折叠左栏，再开切回）。
    pub fn toggle_git(&mut self, cx: &mut Context<Self>) {
        if self.left_open(cx) && self.git_active(cx) {
            let dock = self.left_dock.clone();
            dock.update(cx, |dock, cx| dock.set_open(false, cx));
        } else {
            let dock = self.left_dock.clone();
            dock.update(cx, |dock, cx| dock.set_open(true, cx));
            // #119：直达 git 视图经统一激活协议——git 面板未物化（无
            // 工作区）时 dock 记预物化意图，渲染面呈占位视图（旧
            // git_view=true + git_panel=None 语义一比一等价）。
            self.activate_left_view(crate::git_panel::GIT_PANEL_KEY, cx);
        }
        self.persist_dock_layout(cx);
        cx.notify();
    }

    /// #120（批②）：左栏开合态（dock `is_open`——原 `left_open` 布尔
    /// 退役；菜单勾选/图标条选中态/渲染门/测试共用）。
    pub fn left_open(&self, cx: &App) -> bool {
        self.left_dock.read(cx).is_open()
    }

    /// #109 批②③：项目搜索面板视图切换（图标条搜索钮——与文件树/git
    /// 钮同构的统一激活协议入口；已开且在搜索视图时折叠左栏，再开切回）。
    pub fn toggle_search(&mut self, cx: &mut Context<Self>) {
        if self.left_open(cx) && self.search_active(cx) {
            let dock = self.left_dock.clone();
            dock.update(cx, |dock, cx| dock.set_open(false, cx));
        } else {
            let dock = self.left_dock.clone();
            dock.update(cx, |dock, cx| dock.set_open(true, cx));
            self.activate_left_view(crate::search_panel::SEARCH_PANEL_KEY, cx);
        }
        self.persist_dock_layout(cx);
        cx.notify();
    }

    /// #110 G4：六槽焦点句柄池访问（渲染面 track_focus 捕获用）。
    pub fn color_slot_focus_handles(&self) -> &[gpui::FocusHandle] {
        &self.color_slot_focus
    }

    /// #110 G4：设置面板 toast 收口（show_notice 跨模块公开面——
    /// 保存结果提示）。
    pub fn notice_message(&mut self, message: String, cx: &mut Context<Self>) {
        self.show_notice(message, cx);
    }

    /// #110 G4：草稿访问（渲染面初值与校验共用）。
    pub fn custom_theme_draft(&self) -> &[String; 31] {
        &self.custom_theme_draft
    }

    /// #110 G1 批B：主题名输入框取值。
    pub fn theme_name_input(&self) -> &str {
        &self.theme_name_input
    }

    /// #110 G1 批B：主题名键入（键捕获范式——panel 名框；非法字符
    /// 拒（路径安全：/ 与控制字符）+ 长度界 40）。
    pub fn set_theme_name_input(&mut self, value: String) {
        let clean: String = value
            .chars()
            .filter(|c| !c.is_control() && *c != '/' && *c != '\\')
            .take(40)
            .collect();
        self.theme_name_input = clean;
    }

    /// #110 G4：草稿槽写入（hex 文本输入 on_change——行内校验在渲染
    /// 面（parse_hsla 复用），本面只存原文；坏色不炸不写盘）。
    /// #110 G1 批A：31 槽位表（(JSON 键, 中文标签, 当前色取自 ThemeColors 的
    /// 闭包)——编辑器 UI 行序与 effective_slots 键序的唯一事实源；G2 六键
    /// 前置（老主题文件兼容 + 高频易用），后续按组排）。
    pub fn custom_theme_slot_table() -> [(&'static str, &'static str); 31] {
        [
            ("background", "背景"),
            ("surface", "面板"),
            ("editor", "编辑器"),
            ("foreground", "前景"),
            ("accent", "强调"),
            ("border", "边框"),
            ("border_variant", "边框弱"),
            ("border_focused", "边框聚焦"),
            ("border_selected", "边框选中"),
            ("border_transparent", "边框透明"),
            ("border_disabled", "边框禁用"),
            ("element_background", "元素底"),
            ("element_hover", "元素悬停"),
            ("element_active", "元素激活"),
            ("element_selected", "元素选中"),
            ("element_selection_background", "选区底"),
            ("element_disabled", "元素禁用"),
            ("ghost_element_background", "幽灵底"),
            ("ghost_element_hover", "幽灵悬停"),
            ("ghost_element_active", "幽灵激活"),
            ("ghost_element_selected", "幽灵选中"),
            ("ghost_element_disabled", "幽灵禁用"),
            ("surface_background", "面板平"),
            ("text_muted", "文字弱"),
            ("text_placeholder", "文字占位"),
            ("text_disabled", "文字禁用"),
            ("text_accent", "文字强调"),
            ("icon", "图标"),
            ("icon_muted", "图标弱"),
            ("icon_disabled", "图标禁用"),
            // text 主色与 text_muted 同源（G2 级联）——表尾补齐 31 位。
            ("text", "文字"),
        ]
    }

    /// #110 G1 批A：槽位当前实际色（空槽初值/预览底——ThemeColors 槽取值）。
    pub fn custom_theme_slot_current(slot: usize, colors: &theme::ThemeColors) -> gpui::Hsla {
        let c = &colors;
        match Self::custom_theme_slot_table().get(slot).map(|t| t.0) {
            Some("background") => c.background,
            Some("surface") => c.elevated_surface_background,
            Some("editor") => c.editor_background,
            Some("foreground") | Some("text") => c.text,
            Some("accent") => c.element_active,
            Some("border") => c.border,
            Some("border_variant") => c.border_variant,
            Some("border_focused") => c.border_focused,
            Some("border_selected") => c.border_selected,
            Some("border_transparent") => c.border_transparent,
            Some("border_disabled") => c.border_disabled,
            Some("element_background") => c.element_background,
            Some("element_hover") => c.element_hover,
            Some("element_active") => c.element_active,
            Some("element_selected") => c.element_selected,
            Some("element_selection_background") => c.element_selection_background,
            Some("element_disabled") => c.element_disabled,
            Some("ghost_element_background") => c.ghost_element_background,
            Some("ghost_element_hover") => c.ghost_element_hover,
            Some("ghost_element_active") => c.ghost_element_active,
            Some("ghost_element_selected") => c.ghost_element_selected,
            Some("ghost_element_disabled") => c.ghost_element_disabled,
            Some("surface_background") => c.surface_background,
            Some("text_muted") => c.text_muted,
            Some("text_placeholder") => c.text_placeholder,
            Some("text_disabled") => c.text_disabled,
            Some("text_accent") => c.text_accent,
            Some("icon") => c.icon,
            Some("icon_muted") => c.icon_muted,
            Some("icon_disabled") => c.icon_disabled,
            _ => c.background,
        }
    }

    pub fn set_custom_theme_slot(&mut self, slot: usize, value: String) {
        if let Some(cell) = self.custom_theme_draft.get_mut(slot) {
            *cell = value;
        }
    }

    /// #110 G4：草稿有效槽集（**hex 原文**——保真落盘：不经 Hsla
    /// 往返；parse_hsla 是 G2 装载的灰度近似语义，色值往返会漂移
    /// （#1a2b3c → #2b2b2b 教训），保存链只做**合法性校验**+原文
    /// 透传）。空槽 → 当前主题实际色 hex（所见即所改初值）；坏槽 →
    /// None（任一槽坏整体拒保存——行内报错另有面）。
    fn custom_theme_effective_slots(&self, cx: &App) -> Option<Vec<(String, String)>> {
        let colors = cx.theme().colors();
        let table = Self::custom_theme_slot_table();
        let mut out = Vec::new();
        for (i, (key, _)) in table.iter().enumerate() {
            let raw = self.custom_theme_draft[i].trim().to_string();
            if raw.is_empty() {
                // 空槽：当前主题实际色（hex 化——Rgba 通道取整；
                // #110 G1 批A 31 槽位表驱动）。
                let cur = Self::custom_theme_slot_current(i, &colors);
                let hex = format!(
                    "#{:02x}{:02x}{:02x}",
                    (gpui::Rgba::from(cur).r * 255.0).round() as u8,
                    (gpui::Rgba::from(cur).g * 255.0).round() as u8,
                    (gpui::Rgba::from(cur).b * 255.0).round() as u8
                );
                out.push((key.to_string(), hex));
            } else {
                // 校验（坏色拒）+ 原文透传（保真）。
                crate::settings::parse_hsla(&raw)?;
                out.push((key.to_string(), raw));
            }
        }
        Some(out)
    }

    /// #110 G4：保存自定义主题（D3 一期收口）：
    /// 草稿 6 槽 → `themes_dir/my-theme.json`（G2 格式）→ **立即装载**
    /// （load_user_themes 入 registry）→ **立即切换**（GlobalTheme::
    /// update_theme——不走 base_theme/mode 三态链，自定义主题对三态
    /// 钮的语义=「新基准」；mode 保持，保存后切回三态钮可再切回派生）。
    /// 返回写盘路径（调用方 toast 提示）；None = 槽全坏或写盘失败。
    pub fn save_custom_theme(&mut self, cx: &mut App) -> Option<std::path::PathBuf> {
        // #110 G1 批B：名泛化——输入名优先（空 = my-theme 旧行为）；
        // 保存成功即切「user:<name>」四态轴（三态钮与具名下拉联动）。
        let name = {
            let n = self.theme_name_input.trim();
            if n.is_empty() {
                "my-theme".to_string()
            } else {
                n.to_string()
            }
        };
        let slots: Vec<(String, String)> = self.custom_theme_effective_slots(cx)?;
        let path = crate::settings::save_user_theme_file(&name, &slots, cx)?;
        // 即时装载（registry insert——下次启动 load_user_themes 同源）。
        crate::settings::load_user_themes(cx);
        // 即时切换：从 registry 取回装载后的实体 + 四态轴落位。
        let registry = theme::ThemeRegistry::default_global(cx);
        let loaded = registry.get(&name).ok()?;
        theme::GlobalTheme::update_theme(cx, loaded);
        self.theme_mode = theme_switch::user_mode(&name);
        crate::settings::save_theme_mode(&self.theme_mode);
        cx.refresh_windows();
        Some(path)
    }

    /// #110 G1 批B：删除用户主题（下拉选中项的「删除」链）：
    /// 磁盘文件删 + registry 移除 + 若当前正用该主题 → 回退 dark 三态。
    /// 返回 true = 删了；false = 无该主题（键入名不存在/无磁盘文件）。
    pub fn delete_user_theme(&mut self, name: &str, cx: &mut App) -> bool {
        let registry = theme::ThemeRegistry::default_global(cx);
        if registry.get(name).is_err() {
            return false;
        }
        // 磁盘文件（themes_dir/<name>.json）。
        let mut disk_removed = false;
        if let Some(dir) = crate::settings::themes_dir() {
            let path = dir.join(format!("{name}.json"));
            if path.exists() {
                disk_removed = std::fs::remove_file(&path).is_ok();
                if !disk_removed {
                    eprintln!("theme-delete: 删盘失败 {}", path.display());
                }
            }
        }
        // registry 侧（ThemeMeta 名集）。
        registry.remove_user_themes(&[name.into()]);
        // 当前正用 → 回退 dark（三态轴回位 + 全窗重绘）。
        if theme_switch::parse_user_mode(&self.theme_mode) == Some(name) {
            self.theme_mode = theme_switch::MODE_DARK.to_string();
            theme_switch::apply_mode(theme_switch::MODE_DARK, &self.base_theme, cx);
            crate::settings::save_theme_mode(theme_switch::MODE_DARK);
        }
        cx.refresh_windows();
        disk_removed || true
    }

    /// #110 G1 批B：registry 用户主题名集（下拉数据源——
    /// themes_dir 装载的 user-* 名；内置主题不列，下拉只做用户面）。
    pub fn user_theme_names(&self, _cx: &mut App) -> Vec<String> {
        // #110 G1 批B：磁盘直读（themes_dir/*.json 文件名——registry 枚举
        // 要 &mut App，渲染期 ShellApp 已在 update 内嵌套炸（gpui
        // entity_map 164——settings_lifecycle 两测试实证）；磁盘读零
        // gpui 依赖，渲染与测试双安全。名字 = 文件名去 .json（与
        // save_user_theme_file/load_user_themes 同一命名面）。
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

    /// #108 P2③：转到符号——开右栏 + 聚焦大纲过滤框（过滤即符号
    /// 查找、Enter 跳行——「符号对话框」承接面；参照 Q-IDE outline.rs
    /// goto 对话框语义，我方以大纲面板过滤框承接，无新 UI）。
    /// 无大纲（无激活文档）时提示不静默（#176 精神同款）。
    pub fn goto_symbol(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.right_open {
            self.right_open = true;
            self.persist_dock_layout(cx);
        }
        match self.outline.as_ref() {
            Some(panel) => {
                let focus = panel.read(cx).filter_focus_handle().clone();
                window.focus(&focus, cx);
            }
            None => {
                self.show_notice(
                    "转到符号：当前无文档大纲（先打开 Markdown 文档）".into(),
                    cx,
                );
            }
        }
        cx.notify();
    }

    /// #109：左栏当前是否搜索视图（图标条选中态/菜单勾选共用真源）。
    pub fn search_active(&self, cx: &App) -> bool {
        self.left_dock
            .read(cx)
            .active_is(crate::search_panel::SEARCH_PANEL_KEY)
            || self
                .right_dock
                .read(cx)
                .active_is(crate::search_panel::SEARCH_PANEL_KEY)
    }

    /// #119（mdc-dock 批①）：左栏当前是否 git 视图（dock 激活键裁决
    /// ——含预物化意图口径；菜单勾选/图标条选中态/渲染面共用）。
    pub fn git_active(&self, cx: &App) -> bool {
        // #121（批④）：git 面板可能落在任一 dock（批③b 拖拽换侧）——
        // 两 dock 合查激活态（菜单勾选/图标条选中态的统一真源）。
        self.left_dock
            .read(cx)
            .active_is(crate::git_panel::GIT_PANEL_KEY)
            || self
                .right_dock
                .read(cx)
                .active_is(crate::git_panel::GIT_PANEL_KEY)
    }

    /// #119：左栏视图激活单点（统一激活协议入口——图标条/菜单/快捷
    /// 键三入口共用；后续批多面板扩展不再扩布尔）。#120（批②）：
    /// 激活后同步该面板的记忆宽到渲染宽 + 持久化。
    fn activate_left_view(&mut self, key: &'static str, cx: &mut Context<Self>) {
        let dock = self.left_dock.clone();
        dock.update(cx, |dock, cx| dock.activate_key(key, cx));
        self.sync_left_width_to_active(cx);
        self.persist_dock_layout(cx);
    }

    /// #120（批②）：左栏渲染宽同步激活面板记忆宽（无记忆 = 不动；
    /// 越界值钳制到拖拽域——与启动载入同口径，不信任磁盘值）。
    fn sync_left_width_to_active(&mut self, cx: &App) {
        let dock = self.left_dock.read(cx);
        let Some(key) = dock.active_key() else {
            return;
        };
        if let Some(width) = dock.panel_size(key) {
            self.left_width = px(panels::clamp_panel_width(true, width));
        }
    }

    /// #120（批②）：dock 布局落盘单点（左 dock 快照 + 右栏开合态——
    /// 开/切/拖宽/换工作区四类变更点统一走此）。
    fn persist_dock_layout(&self, cx: &App) {
        let state = self.left_dock.read(cx).dump();
        crate::settings::save_dock_layout(&state, self.right_open);
        // #121 批③b：右 dock 快照同点落盘（additive 键——见 settings）。
        let right = self.right_dock.read(cx).dump();
        crate::settings::save_right_dock_layout(&right);
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
        let move_shell = shell.clone();
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
            // #107 P1②：拖拽移动（树内拖→目录行放——确认→前缀重写链）。
            move_file: Arc::new(move |old, new_dir, window, cx| {
                let _ = move_shell.update(cx, |shell, cx| {
                    shell.confirm_move_path(old, new_dir, window, cx);
                });
            }),
            // Esc → 焦点还激活编辑器（#571 大纲面板同语义）。
            escape: Arc::new(move |window, cx| {
                let _ = shell.update(cx, |shell, cx| shell.focus_active(window, cx));
            }),
        }
    }

    /// #109 批②③：SearchPanel 的 ShellApp 面收口（lib 化钩子注入——
    /// open_hit = open_document_tab + 编辑器跳行；replace_all = 预览 +
    /// 确认 + 执行 + 善后整链；escape = 焦点还编辑器）。
    fn search_panel_hooks(shell: gpui::WeakEntity<Self>) -> SearchPanelHooks {
        use std::sync::Arc;
        let open_shell = shell.clone();
        let replace_shell = shell.clone();
        SearchPanelHooks {
            // 命中点击 → 打开文件 + 跳行（文件树 open_file 同链 + 行定位）。
            open_hit: Arc::new(move |path, line, window, cx| {
                if let Some(focus) = open_shell
                    .update(cx, |shell, cx| {
                        shell.open_document_tab(path.to_path_buf(), cx)
                    })
                    .ok()
                    .flatten()
                {
                    window.focus(&focus, cx);
                    // 跳行（open 后才有效——新开或既有标签都跳）。
                    if let Some(shell) = open_shell.upgrade() {
                        shell.update(cx, |shell, cx| {
                            if let Some(tab) = shell.tabs.get(shell.active) {
                                tab.editor
                                    .update(cx, |ed, _| ed.jump_to_line(line.saturating_sub(1)));
                            }
                        });
                    }
                }
            }),
            // 全部替换（批③整链：预览计数 → 确认框 → 执行 → 善后 → toast）。
            replace_all: Arc::new(move |pattern, replacement, options, window, cx| {
                if let Some(shell) = replace_shell.upgrade() {
                    shell.update(cx, |shell, cx| {
                        shell.request_search_replace(
                            pattern.to_string(),
                            replacement.to_string(),
                            options.clone(),
                            window,
                            cx,
                        );
                    });
                }
            }),
            // Esc → 焦点还激活编辑器（面板键盘域退出——同语义）。
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
    /// #120（批②）：左栏宽同步写激活面板的 per-panel 记忆宽（dock
    /// 快照随 persist_dock_layout 落盘——文件树/git 面各记各宽）。
    fn finish_panel_drag(&mut self, cx: &mut Context<Self>) {
        if self.panel_drag.take().is_none() {
            return;
        }
        let (left, right) = (f32::from(self.left_width), f32::from(self.right_width));
        crate::settings::save_panel_widths(left, right);
        let dock = self.left_dock.clone();
        dock.update(cx, |dock, cx| {
            if let Some(key) = dock.active_key() {
                dock.set_panel_size(key, left, cx);
            }
        });
        self.persist_dock_layout(cx);
        eprintln!("panel-resize: left {left} right {right} (persist)");
        cx.notify();
    }

    /// G4：手柄元素（边缘 4px 热区——col-resize 光标 + 拖拽会话）。
    /// #121（批③b）：面板标题拖拽手柄——拖拽源（`DraggedPanel` 载荷
    /// 携带源 dock `EntityId` + 面板键；跨实例误落校验见
    /// [`mdc_dock::drop_is_intended`]）。挂左/右栏标题区（Ely
    /// 「面板头部长按拖动」同构；redistributable_columns.rs 同构蓝本）。
    fn panel_title_drag_handle(
        &self,
        dock: &Entity<mdc_dock::Dock>,
        key: &'static str,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let payload = mdc_dock::DraggedPanel {
            source_dock: dock.entity_id(),
            key,
        };
        div()
            .id(SharedString::from(format!("panel-drag-{key}")))
            .w_full()
            .h(px(24.))
            .flex_none()
            .cursor(CursorStyle::Arrow)
            .debug_selector(|| format!("panel-drag-{key}").into())
            .on_drag(payload, move |_, _, _, cx| cx.new(|_| PanelDragGhost))
    }

    /// #121（批③b）：拖放收尾——`placement_for` 裁决 → take/receive
    /// 协议 → 面板 `set_position` → 持久化。Center/同侧/非法 = 取消
    /// 无动作（面板留源 dock 原位——批③a Cancel 语义）。
    fn finish_panel_drop(
        &mut self,
        dragged: mdc_dock::DraggedPanel,
        zone: mdc_dock::DockDropZone,
        cx: &mut Context<Self>,
    ) {
        self.drag_zone = None;
        // 源 dock：面板当前所在（dragged.source_dock 对应侧实体）。
        let (source_dock, target_dock, target_pos) = (
            if dragged.source_dock == self.left_dock.entity_id() {
                self.left_dock.clone()
            } else {
                self.right_dock.clone()
            },
            match zone.dock_position() {
                Some(mdc_dock::DockPosition::Left) => self.left_dock.clone(),
                Some(mdc_dock::DockPosition::Right) => self.right_dock.clone(),
                _ => self.right_dock.clone(),
            },
            zone.dock_position(),
        );
        // 面板偏好位 + 合法边（经 PanelHandle 动态查询——面板实体上）。
        let panel_pos = source_dock.read(cx).panel_position(dragged.key, cx);
        let preferred = panel_pos.unwrap_or(mdc_dock::DockPosition::Left);
        let is_valid = |pos: mdc_dock::DockPosition| {
            panel_pos.is_some_and(|_| {
                source_dock
                    .read(cx)
                    .panel_position_is_valid(dragged.key, pos, cx)
            })
        };
        let placement = mdc_dock::placement_for(zone, preferred, is_valid);
        let mdc_dock::Placement::Dock(pos) = placement else {
            cx.notify();
            return; // Cancel：面板留原位（Center/非法侧/同侧）。
        };
        if !mdc_dock::drop_is_intended(dragged, target_dock.entity_id()) {
            cx.notify();
            return; // 拖回源侧 = 取消（面板本来在这边）。
        }
        // take → receive（落位即激活 = 挤掉原槽位语义，批③a 协议）。
        if let Some(detachment) =
            source_dock.update(cx, |dock, cx| dock.take_panel(dragged.key, cx))
        {
            target_dock.update(cx, |dock, cx| dock.receive_panel(detachment, cx));
            // 面板换位（宿主 Panel::set_position 覆写生效）。
            let _ = self.update_panel_position(dragged.key, pos, cx);
            // 目标侧右栏开合态联动（Left→Right 落右栏时开右栏）。
            if pos == mdc_dock::DockPosition::Right {
                self.right_open = true;
            }
            self.persist_dock_layout(cx);
        }
        cx.notify();
    }

    /// #121（批③b）：面板实体 `set_position` 转发（FileTree/GitPanel/
    /// OutlinePanel 的 Panel impl 覆写——经 Dock 实体更新）。
    fn update_panel_position(
        &mut self,
        key: &str,
        pos: mdc_dock::DockPosition,
        cx: &mut Context<Self>,
    ) -> bool {
        let applied = self
            .left_dock
            .update(cx, |dock, cx| dock.set_panel_position(key, pos, cx))
            || self
                .right_dock
                .update(cx, |dock, cx| dock.set_panel_position(key, pos, cx));
        applied
    }

    /// #121（批④）：右栏呈现面——right_dock 激活面板驱动。
    ///
    /// 激活 git（批③b 拖入）→ GitPanel 本体 + 标题拖拽手柄（可再拖走）；
    /// 其余/无激活 → 旧 right_panel 兜底（outline/文档大纲——#289-⑤
    /// 语义不变，零回归口径）。
    fn right_dock_content(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let active_key = self.right_dock.read(cx).active_key();
        if active_key == Some(crate::git_panel::GIT_PANEL_KEY)
            && let Some(git) = self.git_panel.as_ref()
        {
            let title_drag = self.right_dock.read(cx).active_key().map(|key| {
                self.panel_title_drag_handle(&self.right_dock.clone(), key, cx)
                    .into_any_element()
            });
            return v_flex()
                .id("right-dock-content")
                .debug_selector(|| "right-dock-content".into())
                .w(self.right_width)
                .flex_none()
                .h_full()
                .min_h_0()
                .min_w_0()
                .bg(colors.panel_background)
                .p(panels::PANEL_PADDING)
                .when_some(title_drag, |el, handle| el.child(handle))
                .child(
                    Label::new("源代码管理")
                        .size(LabelSize::Small)
                        .color(ui::Color::Muted),
                )
                .child(git.clone())
                .into_any_element();
        }
        panels::right_panel(self.outline.as_ref(), self.right_width, cx).into_any_element()
    }

    /// #121（批③b）：拖放**感应面**——常驻透明全窗（`size_full`
    /// absolute）。承载 `on_drag_move`（分区追踪——drag 起于
    /// `DraggedPanel` 时 move 元素内外都回调）与 `on_drop`（MouseUp
    /// 命中管线）。无视觉。
    fn drag_drop_sensor(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .size_full()
            .top_0()
            .left_0()
            .on_drop::<mdc_dock::DraggedPanel>(cx.listener(
                |this, dragged: &mdc_dock::DraggedPanel, window, cx| {
                    let zone = this.drag_zone.unwrap_or(mdc_dock::DockDropZone::Center);
                    this.finish_panel_drop(*dragged, zone, cx);
                    window.refresh();
                },
            ))
            .on_drag_move::<mdc_dock::DraggedPanel>(cx.listener(
                |this, event: &DragMoveEvent<mdc_dock::DraggedPanel>, window, cx| {
                    // 窗口局部坐标域（`window.bounds()` 是屏幕全域——OS
                    // 窗口位置非局部原点，会造成分区错位；局部口径 =
                    // origin(0,0) + viewport_size）。
                    let size = window.viewport_size();
                    let bounds = gpui::Bounds::from_corners(
                        gpui::point(gpui::px(0.), gpui::px(0.)),
                        gpui::point(size.width, size.height),
                    );
                    let zone = mdc_dock::drop_zone_at(bounds, event.event.position);
                    if this.drag_zone != Some(zone) {
                        this.drag_zone = Some(zone);
                        cx.notify();
                    }
                },
            ))
    }

    /// #121（批③b）：拖放高亮层——三分区视觉（Q-IDE 模式；仅拖拽
    /// 中渲染）。Center 无高亮（取消拖拽视觉）。
    fn drag_drop_overlay(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let zone = self.drag_zone;
        let highlight = colors.element_active;
        let left_w = px(140.);
        let bottom_h = px(120.);
        div()
            .absolute()
            .size_full()
            .top_0()
            .left_0()
            // 分区高亮（仅拖拽中——外层 .when(drag_zone.is_some())）。
            .when(matches!(zone, Some(mdc_dock::DockDropZone::Left)), |el| {
                el.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .h_full()
                        .w(left_w)
                        .bg(highlight.opacity(0.25))
                        .border_r_1()
                        .border_color(highlight),
                )
            })
            .when(matches!(zone, Some(mdc_dock::DockDropZone::Right)), |el| {
                el.child(
                    div()
                        .absolute()
                        .top_0()
                        .right_0()
                        .h_full()
                        .w(left_w)
                        .bg(highlight.opacity(0.25))
                        .border_l_1()
                        .border_color(highlight),
                )
            })
            .when(matches!(zone, Some(mdc_dock::DockDropZone::Bottom)), |el| {
                el.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .w_full()
                        .h(bottom_h)
                        .bg(highlight.opacity(0.25))
                        .border_t_1()
                        .border_color(highlight),
                )
            })
    }

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
        // #120（批②）：右栏开合态随 dock 节点持久化（#106 报告 §6 顺手缺陷）。
        self.persist_dock_layout(cx);
        cx.notify();
    }

    pub fn toggle_document_info(&mut self, cx: &mut Context<Self>) {
        self.document_info_open = !self.document_info_open;
        cx.notify();
    }

    /// #289-⑤：打开设置弹窗（左条设置钮 / 菜单「查看 → 设置」入口；
    /// 弹窗形态——zed「设置是独立界面不是 dock 视图」同款语义）。
    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.close_context_menu(cx);
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
        self.close_context_menu(cx);
        self.settings_open = !self.settings_open;
        cx.notify();
    }

    // ── #289-④：pdf 页面视图（位图渲染面） ──────────────────────

    /// 当前活动标签是否 pdf 态（页视图的挂载判据）。
    fn active_is_pdf(&self, cx: &Context<Self>) -> bool {
        self.editor()
            .is_some_and(|ed| ed.read(cx).doc_kind() == "pdf")
    }

    /// 当前活动标签是否 docx 态（#50 页面视图挂载判据）。
    fn active_is_docx(&self, cx: &Context<Self>) -> bool {
        self.editor()
            .is_some_and(|ed| ed.read(cx).doc_kind() == "docx")
    }

    /// #176：活动 docx 标签处于 LOK 页面视图（菜单灰显/键守卫/图标条
    /// 共用判定）——与 `editor_area` 渲染分流同源三条件：docx 标签 +
    /// 页面模式开关 + 页视图实体在场（实体缺位时 `editor_area` 实际
    /// 呈现文本编辑器，非页面态——不误伤）。取 `&App`（菜单快照
    /// `menu_context` 与键/渲染面 `Context` 双消费——Context 解引用
    /// 到 App，双面零适配）。
    pub(crate) fn docx_page_view_active(&self, cx: &App) -> bool {
        self.editor()
            .is_some_and(|ed| ed.read(cx).doc_kind() == "docx")
            && self.docx_view_mode
            && self.docx_pages.is_some()
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
        match self.pdf_pages.as_ref().map(|(owned, _)| *owned == path) {
            Some(true) => return, // 同路径实体已在位——无操作。
            Some(false) => {
                // #51 D4b：让位前快照（缩放档 + 顶部页）——回来时回放，
                // 「当前页与缩放状态一致」（不从首页 FitWidth 重来）。
                let (old_path, old_view) = self.pdf_pages.take().expect("在位已判");
                let snap = old_view.read(cx).snapshot();
                self.pdf_view_snapshots.insert(old_path, snap);
            }
            None => {}
        }
        // 读盘（pdf_doc::load 同源 100MB 前置门——超限不入页视图，
        // 文本模型不受影响）。
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) => {
                let message = format!("页面视图装载失败（{}）：{err}", path.display());
                self.show_notice(message, cx);
                self.pdf_pages = None;
                self.pdf_thumbs = None;
                return;
            }
        };
        if bytes.len() as u64 > 100 * 1024 * 1024 {
            self.show_notice("页面视图仅支持 ≤100MB 文件（文本视图不受限）".into(), cx);
            self.pdf_pages = None;
            self.pdf_thumbs = None;
            return;
        }
        let entity = cx.new(|cx| PdfPagesView::new(bytes, cx));
        // #51 D4b：快照回放（同路径此前打开过——缩放档 + 跳回原页）。
        if let Some((mode, page)) = self.pdf_view_snapshots.get(&path).copied() {
            entity.update(cx, |v, _| v.restore_state(mode, page));
        }
        // #51 D4b：页码读数活更新订阅（页变化 emit → Shell 重渲——
        // 状态栏「第 N / M 页」跟随滚动/跳页/缩放重排）。
        cx.subscribe(&entity, |_, _, _: &crate::pdf_pages::PdfPagesEvent, cx| {
            cx.notify();
        })
        .detach();
        self.pdf_pages = Some((path, entity.clone()));
        // #420：缩略图侧栏——与页视图同源字节/几何；error 态（缺
        // pdfium/引擎失败）不挂空壳。联动钩子：点击 → scroll_to_page；
        // 高亮 ← top_visible_page（#420 新读数面）。
        let engine_ok = entity.read(cx).page_count() > 0;
        self.pdf_thumbs = engine_ok.then(|| {
            let pages_jump = entity.clone();
            let pages_read = entity.clone();
            cx.new(|cx| {
                PdfThumbnailsView::new(
                    entity.read(cx).bytes(),
                    entity.read(cx).page_aspect(),
                    PdfThumbsHooks {
                        jump_to_page: Arc::new(move |page, _window, cx| {
                            pages_jump.update(cx, |v, _| v.scroll_to_page(page));
                        }),
                        current_page: Arc::new(move |app| pages_read.read(app).top_visible_page()),
                    },
                    cx,
                )
            })
        });
        cx.notify();
    }

    /// #289-④：切换 pdf 显示模式（状态栏「页面/文本」钮）。
    pub fn toggle_pdf_view_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.active_is_pdf(cx) {
            return;
        }
        self.pdf_view_mode = !self.pdf_view_mode;
        if self.pdf_view_mode {
            self.ensure_pdf_pages(cx);
        }
        // #604：切换后重聚焦当前主显示面（页视图 ↔ 文本编辑器）。
        // 视图实体的焦点节点随渲染树增删——不重聚焦则句柄游离，键
        // 事件死在合成根（#422 同款；xlsx 网格/文本切换对称）。
        self.focus_active(window, cx);
        cx.notify();
    }

    /// #50 C1：确保当前 docx 标签的页面视图实体在场（生命周期与
    /// ensure_pdf_pages 同模式：非 docx 态 / 已在同路径实体上 = 无操
    /// 作；切标签重建）。会话在视图实体内自管（Drop 即拆 worker——
    /// 会话层线程演员收尾）。
    /// #47 A2：**文本态不建隐藏会话**——`toggle_docx_view_mode` 切文本
    /// 态时拆 LOK 会话（行投影成为唯一编辑面）；激活/重开路径若在此
    /// 无视模式重建实体，会在文本态背后重新拉起一个 LOK worker（不渲
    /// 染、只占资源，且违背拆会话语义）——与 pdf/xlsx 不同，docx 会话
    /// 无银行（dirty/编辑面全在行投影），文本态重建无任何状态可恢复。
    fn ensure_docx_pages(&mut self, cx: &mut Context<Self>) {
        // #47 A2：文本态直接让位（模式卫先于一切——含同路径命中）。
        if !self.docx_view_mode {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if tab.editor.read(cx).doc_kind() != "docx" {
            return;
        }
        let path = tab.path.clone();
        if let Some((owned, _)) = &self.docx_pages {
            if *owned == path {
                return;
            }
        }
        // #50 C1：传路径（LOK 从盘装载——标签路径绑定单一真源）；打开
        // 链/装载失败在视图内以 Boot::Failed 呈现（错误面板自带文本
        // 视图后备提示——不白屏、不假绿）。
        let entity = cx.new(|cx| DocxPagesView::new(path.clone(), cx));
        self.docx_pages = Some((path, entity));
        cx.notify();
    }

    /// #50 C1：切换 docx 显示模式（状态栏「页面/文本」钮）。
    /// 互斥卫：文本态有未保存编辑拒切页面态——LOK 会话从盘装载，
    /// 双编辑真源会静默分叉（先保存再切换）。
    pub fn toggle_docx_view_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.active_is_docx(cx) {
            return;
        }
        if !self.docx_view_mode && self.editor().is_some_and(|ed| ed.read(cx).is_modified()) {
            self.show_notice(
                "文本视图有未保存编辑——先保存（Ctrl+S）再切换到页面视图（单一编辑真源）".into(),
                cx,
            );
            return;
        }
        self.docx_view_mode = !self.docx_view_mode;
        if self.docx_view_mode {
            self.ensure_docx_pages(cx);
        } else {
            // 切文本态：拆 LOK 会话（实体 Drop → worker 关停）——行
            // 投影重新成为唯一编辑面。
            self.docx_pages = None;
        }
        self.focus_active(window, cx);
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
    pub fn toggle_xlsx_view_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.active_is_xlsx(cx) {
            return;
        }
        self.xlsx_grid_view_mode = !self.xlsx_grid_view_mode;
        if self.xlsx_grid_view_mode {
            self.ensure_xlsx_grid(cx);
        }
        // #604：切换后重聚焦当前主显示面（网格 ↔ 文本编辑器）——
        // 不重聚焦则网格句柄在文本态游离（Ctrl+S/键入失灵 =
        // #604 漏存复现路径之二），pdf 页视图切换对称同修。
        self.focus_active(window, cx);
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
        if self.confirm.is_some() || self.settings_open {
            return;
        }
        self.close_menu_bar_internal(cx);
        let owns_focus = self
            .context_menu_focus
            .as_ref()
            .is_some_and(|focus| focus.contains_focused(window, cx));
        let previous = if owns_focus {
            self.context_menu_previous_focus.take()
        } else {
            window.focused(cx)
        };
        if owns_focus {
            if let Some(previous) = previous.as_ref() {
                window.focus(previous, cx);
            }
        }
        self.close_context_menu(cx);
        self.context_menu_generation = self.context_menu_generation.wrapping_add(1);
        let generation = self.context_menu_generation;
        self.context_menu_previous_focus = previous.clone();
        let shell = cx.entity().downgrade();
        let menu =
            context_menu_view::build_context_menu(&items, shell.clone(), generation, window, cx);
        self.context_menu = Some(menu.clone());
        self.context_menu_pos = pos;
        self.context_menu_origin = origin;
        self.context_menu_target = target;
        // DismissEvent → 状态清理（点选动作后组件自动 emit；Esc/点外部
        // 经 on_blur → Cancel → emit——两条路都归一到这里）。
        let dismiss_shell = shell.clone();
        self.context_menu_subscription = Some(window.subscribe(
            &menu,
            cx,
            move |menu, _: &gpui::DismissEvent, window, cx| {
                let _ = dismiss_shell.update(cx, |this, cx| {
                    if this.context_menu_generation != generation
                        || this.context_menu.as_ref() != Some(&menu)
                    {
                        return;
                    }
                    if this.confirm.is_none()
                        && !this.settings_open
                        && menu.read(cx).focus_handle(cx).contains_focused(window, cx)
                    {
                        if let Some(previous) = previous.as_ref() {
                            window.focus(previous, cx);
                        }
                    }
                    this.close_context_menu(cx);
                });
            },
        ));
        // 键盘导航需要焦点在菜单（RightClickMenu 同款：等 deferred 渲染
        // 树链入焦点域后再聚焦，避免 contains_focused 误判闪烁）。
        let focus = menu.read(cx).focus_handle(cx).clone();
        self.context_menu_focus = Some(focus.clone());
        window.on_next_frame(move |window, _cx| {
            window.on_next_frame(move |window, cx| {
                let _ = shell.update(cx, |this, cx| {
                    if this.context_menu_generation == generation
                        && this.context_menu.is_some()
                        && this.confirm.is_none()
                        && !this.settings_open
                        && this.menu_bar_open.is_none()
                        && window.focused(cx) == this.context_menu_previous_focus
                    {
                        window.focus(&focus, cx);
                    }
                });
            });
        });
        cx.notify();
    }

    /// 关闭右键菜单（Esc / 点空白 / 执行完动作）。目标随菜单一起清
    ///（#166——菜单关了，目标路径不再有效）。
    pub fn close_context_menu(&mut self, cx: &mut Context<Self>) {
        self.context_menu_subscription = None;
        self.context_menu_generation = self.context_menu_generation.wrapping_add(1);
        self.context_menu_target = None;
        self.context_tab_target = None;
        self.context_menu_previous_focus = None;
        self.context_menu_focus = None;
        if self.context_menu.take().is_some() {
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
        if self.context_menu.is_none() || self.confirm.is_some() || self.settings_open {
            return;
        }
        let origin = self.context_menu_origin;
        let target = self.context_menu_target.clone();
        let tab_target = self.context_tab_target.clone();
        let previous = self.context_menu_previous_focus.clone();
        let menu_owned_focus = self
            .context_menu_focus
            .as_ref()
            .is_some_and(|focus| focus.contains_focused(window, cx));
        self.close_context_menu(cx);
        if menu_owned_focus {
            if let Some(previous) = previous.as_ref() {
                window.focus(previous, cx);
            }
        }
        if origin == MenuOrigin::Tab {
            self.activate_tab_menu_item(id, tab_target, window, cx);
            return;
        }
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
            (MenuOrigin::FileTree, "ft-refresh") => {
                // #107 D1①：外部改动后的手动同步——重读目标目录（目录）
                // 或父目录（文件）清单。复用 #166 既有 reload（只刷已缓存
                // 目录；未展开目录本无清单，展开时现读）；根展开态常驻
                // （TreeState::new 即读根），根级刷新必然命中缓存重读。
                if let Some(target) = target {
                    let dir = if target.is_dir() {
                        target
                    } else {
                        target.parent().map(|p| p.to_path_buf()).unwrap_or(target)
                    };
                    if let Some(ws) = self.workspace.as_ref().cloned() {
                        ws.update(cx, |tree, cx| {
                            tree.reload(&dir);
                            cx.notify();
                        });
                    }
                }
            }
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
            // #107 P2：复制路径（右键目标绝对路径 → 系统剪贴板——
            // form_input.rs 剪贴板先例同型；无目标空操作）。
            (MenuOrigin::FileTree, "ft-copy-path") => {
                if let Some(target) = target {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                        target.display().to_string(),
                    ));
                }
            }
            // #107 P1 批⑦：树面手动刷新（报告 P1① 后半——mtime 轮询
            // 未立项前的手动面；工作区根整树重读——#595 光标路径重绑定
            // 既有语义背书零新语义；无工作区时空操作）。
            (MenuOrigin::FileTree, "ft-refresh") => {
                if let Some(tree) = &self.workspace {
                    let root = tree.read(cx).root().to_path_buf();
                    tree.update(cx, |tree, cx| {
                        tree.reload(&root);
                        cx.notify();
                    });
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
        // #604：xlsx 标签保存的分流已上收 [`Self::save_tab`]（上方
        // Cmd::Save 首臂）。此前此处另有一支 xlsx 判定，因首臂提前
        // return 永不可达（死代码——#604 复现的遗漏点：菜单保存从未
        // 走到会话路径）；删除归一——菜单/命令面板/键位/双确认四路
        // 同收口，三态语义（Saved/Unmodified/Failed）由 save_tab 统一
        // 供确认框消费。
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
            theme_mode: self.theme_mode.clone(),
            left_open: self.left_open(cx),
            git_view: self.git_active(cx),
            // #109：搜索视图态（侧栏菜单勾选——dock 激活键真源）。
            search_view: self.search_active(cx),
            // #307：「导出为 PDF」门槛（仅 docx 形态提供；md/pdf/xlsx/
            // canvas 标签上灰显）。
            active_is_docx: tab.is_some_and(|t| t.editor.read(cx).doc_kind() == "docx"),
            // #307 ②：「导出为 DOCX」门槛（仅 pdf 形态提供；文本版导出
            // 读 PDF 文本层——扫描件零文本在转换内显式拒）。
            active_is_pdf: tab.is_some_and(|t| t.editor.read(cx).doc_kind() == "pdf"),
            // #306 ①：「OCR 识别扫描件」门槛（pdf 态 + 扫描件 + 未识别）。
            pdf_ocr_pending: tab.is_some_and(|t| t.editor.read(cx).pdf_ocr_pending()),
            // #306 ②：「另存为可搜索 PDF」门槛（pdf 态 + OCR 词表在位）。
            pdf_ocr_ready: tab.is_some_and(|t| t.editor.read(cx).pdf_ocr_ready()),
            // #176：docx 页面视图在位（作用于不可见编辑器的动作置灰
            // 依据——见 menu_bar::REASON_PAGE_*）。
            docx_page_view: self.docx_page_view_active(cx),
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
        if self.confirm.is_some() || self.settings_open {
            return;
        }
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
        let previous = window.focused(cx);
        let shell = cx.entity().downgrade();
        let menu_id = entity.entity_id();
        window.on_next_frame(move |window, _cx| {
            window.on_next_frame(move |window, cx| {
                let _ = shell.update(cx, |this, cx| {
                    if this
                        .menu_bar_entity
                        .as_ref()
                        .is_some_and(|menu| menu.entity_id() == menu_id)
                        && this.context_menu.is_none()
                        && this.confirm.is_none()
                        && !this.settings_open
                        && window.focused(cx) == previous
                    {
                        window.focus(&focus, cx);
                    }
                });
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
        // #50 C3（#176 翻新）：docx 页面态——剪贴板/Cmd 编辑动作分流到
        // 页视图编辑通道（同键位域 dispatch_edit_key 一条链）。undo/
        // redo/cut/copy/paste 页面已可编辑（PR #217 后置灰原因退役；
        // 查找/替换/面板是编辑器浮层仍置灰——menu_bar 门控区分对待）。
        if self.docx_page_view_active(cx) {
            if let Some((_, entity)) = self.docx_pages.clone() {
                let mut consumed = false;
                entity.update(cx, |view, cx| {
                    consumed = match action {
                        Act::EditorCmd(crate::editor::commands::Cmd::Undo) => {
                            view.dispatch_edit_key("z", true, false, window, cx);
                            true
                        }
                        Act::EditorCmd(crate::editor::commands::Cmd::Redo) => {
                            view.dispatch_edit_key("y", true, false, window, cx);
                            true
                        }
                        Act::Cut => view.dispatch_edit_key("x", true, false, window, cx),
                        Act::Copy => view.dispatch_edit_key("c", true, false, window, cx),
                        Act::Paste => view.dispatch_edit_key("v", true, false, window, cx),
                        _ => false,
                    };
                });
                if consumed {
                    return;
                }
            }
        }
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
            // #306 ①：OCR 识别扫描件（后台逐页 tesseract 词级识别）。
            Act::OcrPdf => self.ocr_active_pdf(window, cx),
            // #306 ②：另存为可搜索 PDF（OCR 词 → 隐形文本层写回）。
            Act::SaveSearchablePdf => self.save_searchable_pdf(window, cx),
            // #507 / #240 任务②：壳面系统打印流（方案 C——Windows 打印
            // 动词 / macOS open -p / CUPS lp；设置面由系统对话框或 lp
            // 参数承载）。
            Act::Print => self.print_active_document(cx),
            // 与标题栏 × 钮同一条 cx.quit 既有路径（该路径不经
            // on_should_close 脏确认——× 钮现状同此，行为对齐不超范围
            // 修补；统一走脏确认建议另开 issue）。
            Act::Quit => {
                // #114 批②：退出前窗口矩形先存（跨重启恢复——关窗
                // 无条件先存语义的退出链面；非优雅退出不保，边界
                // 如实——窗口 WM 摩擦场景下次启动回默认）。
                let b = window.bounds();
                crate::settings::save_window_bounds(
                    b.origin.x.as_f32(),
                    b.origin.y.as_f32(),
                    b.size.width.as_f32(),
                    b.size.height.as_f32(),
                );
                cx.quit();
            }
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
            // #109 批②③：项目搜索面板（图标条/菜单同链——Ctrl+Shift+F
            // 键位同接此处）。
            Act::ToggleSearchPanel => self.toggle_search(cx),
            // #108 P2③：转到符号（Ctrl+Shift+O——开右栏 + 聚焦大纲
            // 过滤框；无大纲不静默）。
            Act::GotoSymbol => self.goto_symbol(window, cx),
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
                // #50 C1：docx 页面态焦点路由同源（is_docx）。
                let is_docx = editor.read(cx).doc_kind() == "docx";
                self.tabs.push(TabItem {
                    path: path.clone(),
                    editor,
                });
                self.active = self.tabs.len() - 1;
                // #107 P1①：mtime 轮询启动（tabs 非空——外部变更感知；
                // 首步建基线，2s 周期比对）。
                self.spawn_mtime_poll_loop(cx);
                // #571：激活编辑器变更 → 大纲面板换绑。
                self.sync_outline(cx);
                self.notice_board.clear();
                self.recent.record_file(&path);
                eprintln!(
                    "tabs: open → {} 开（active {}）",
                    self.tabs.len(),
                    self.active
                );
                // #114 批①：开标签落盘（会话恢复数据源）。
                self.persist_session_tabs();
                // F2：文件树高亮同步（工作区打开时）。
                if let Some(tree) = &self.workspace {
                    tree.update(cx, |tree, cx| {
                        tree.set_active_file(Some(path.clone()));
                        cx.notify();
                    });
                }
                // #289-④：pdf 标签 → 页视图落位（位图渲染主显示面）。
                self.ensure_pdf_pages(cx);
                // #50 C1：docx 标签 → LOK 页面视图落位（内核真源渲染
                // 主显示面；缺内核时错误面板明示 + 文本视图后备）。
                self.ensure_docx_pages(cx);
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
                // #50 C1：docx 页面态——主显示面是 LOK 页视图（编辑器
                // 不在树内）：焦点改返页视图（#422 同源病灶预防）。
                if self.docx_view_mode
                    && is_docx
                    && let Some((_, view)) = self.docx_pages.as_ref()
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
    /// #114 批①：启动会话恢复（settings 读 → 逐路径 open_document_tab
    /// 既有链——报告 D1 约束：不动 docx/pdf/xlsx 编辑器构造链）。
    /// 上限 20（防炸开）；已删文件静默跳过（Q-IDE 同语义）+ 恢复
    /// 证据行；激活标签末位恢复（索引越界钳到尾）。返回恢复数。
    pub fn restore_session_tabs(&mut self, cx: &mut Context<Self>) -> usize {
        let (paths, active) = crate::settings::load_session_tabs();
        if paths.is_empty() {
            return 0;
        }
        let mut restored = 0;
        for path in paths.into_iter().take(20) {
            if !path.is_file() {
                eprintln!("session-restore: 已删跳过 {}", path.display());
                continue;
            }
            if self.tabs.iter().any(|t| t.path == path) {
                continue; // 启动初始标签去重（CLI 直开同文件）
            }
            if self.open_document_tab(path, cx).is_some() {
                restored += 1;
            }
        }
        // 激活恢复（存档索引越界/文件被跳 → 钳到有效尾）。
        let idx = active.min(self.tabs.len().saturating_sub(1));
        let _ = self.activate_tab_focus(idx, cx);
        eprintln!("session-restore: 恢复 {restored} 标签（active {idx}）");
        restored
    }

    /// #114 P1④：关窗先存统一面（on_window_should_close 钩内调用——
    /// 窗口矩形（#230）+ 会话标签（批①）两键无条件落存；非优雅退出
    /// 物理边界不保）。
    pub(crate) fn persist_on_close(&self, window: &mut Window) {
        let b = window.bounds();
        crate::settings::save_window_bounds(
            b.origin.x.as_f32(),
            b.origin.y.as_f32(),
            b.size.width.as_f32(),
            b.size.height.as_f32(),
        );
        self.persist_session_tabs();
    }

    /// #114 P1④：关窗钩读面（main 钩内 shell.update 用——快照
    /// 只读，不引 Context）。
    pub(crate) fn tabs_paths_snapshot(&self) -> Vec<std::path::PathBuf> {
        self.tabs.iter().map(|t| t.path.clone()).collect()
    }

    /// #114 批①：会话标签列表落盘（open/close/activate 三时机统一面
    /// ——跨重启恢复的数据源；光标不恢复——Q-IDE 亦不立）。
    fn persist_session_tabs(&self) {
        let paths: Vec<std::path::PathBuf> = self.tabs.iter().map(|t| t.path.clone()).collect();
        crate::settings::save_session_tabs(&paths, self.active);
    }

    pub fn activate_tab(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = self.activate_tab_focus(idx, cx) {
            window.focus(&focus, cx);
        }
    }

    /// 激活核心（无 window——焦点句柄返回，调用方落焦）。
    fn activate_tab_focus(&mut self, idx: usize, cx: &mut Context<Self>) -> Option<FocusHandle> {
        let tab = self.tabs.get(idx)?.clone();
        self.active = idx;
        // #114 批①：激活切换落盘（会话恢复——激活索引随变更即存）。
        self.persist_session_tabs();
        // #571：激活编辑器变更 → 大纲面板换绑。
        self.sync_outline(cx);
        // #289-④：pdf 标签 → 页视图随标签落位（同路径复用，异路径重建）。
        self.ensure_pdf_pages(cx);
        // #50 C1：docx 标签 → LOK 页面视图随标签落位（同路径复用，异
        // 路径重建——旧实体 Drop 拆会话）。
        self.ensure_docx_pages(cx);
        // #389 阶段 1：xlsx 标签 → 网格视图随标签落位（同路径复用，异路径重建）。
        self.ensure_xlsx_grid(cx);
        // #604：焦点句柄与 [`Self::focus_active`] 同源——视图模式态
        //（pdf 页视图 / xlsx 网格）主显示面是视图实体，文本编辑器不
        // 在渲染树，其句柄游离则键事件死在合成根（#422 同款病灶；
        // 切走再切回后 Ctrl+S/键入全部失灵 = #604 漏存复现路径之一）。
        if self.pdf_view_mode
            && tab.editor.read(cx).doc_kind() == "pdf"
            && let Some((_, view)) = self.pdf_pages.as_ref()
        {
            let focus = view.read(cx).focus_handle(cx).clone();
            cx.notify();
            return Some(focus);
        }
        if self.xlsx_grid_view_mode
            && tab.editor.read(cx).doc_kind() == "xlsx"
            && let Some((_, view)) = self.xlsx_grid.as_ref()
        {
            let focus = view.read(cx).focus_handle(cx).clone();
            cx.notify();
            return Some(focus);
        }
        // #50 C1：docx 页面态焦点路由（主显示面是 LOK 页视图）。
        if self.docx_view_mode
            && tab.editor.read(cx).doc_kind() == "docx"
            && let Some((_, view)) = self.docx_pages.as_ref()
        {
            let focus = view.read(cx).focus_handle(cx).clone();
            cx.notify();
            return Some(focus);
        }
        let focus = tab.editor.read(cx).focus_handle().clone();
        cx.notify();
        Some(focus)
    }

    /// F3/F4：关闭当前标签（Ctrl+W）——脏标签先弹确认框（保存/不保存/
    /// 取消）；干净标签直接关。#386：pdf 批注脏（页视图未落盘）计入
    /// 脏判定（`tab_is_modified` 同源）。
    pub fn close_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.request_close_tab(self.active, window, cx);
    }

    pub fn request_close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirm.is_some() || self.settings_open {
            return;
        }
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        let target = tab.editor.entity_id();
        self.close_context_menu(cx);
        self.close_queue.clear();
        self.close_queue.push_back((target, false));
        self.drain_close_queue(window, cx);
    }

    fn request_close_tab_internal(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.tabs.len() {
            return;
        }
        // F4：脏标签 → 确认框拦截（#389 阶段 2 xlsx 会话脏 + #386
        // pdf 批注脏——tab_is_modified 已统一收口）。
        if self.tab_is_modified(index, cx).unwrap_or(false) {
            let name = self.tabs[index].editor.read(cx).display_name().to_string();
            eprintln!("confirm: 关闭未保存标签（{name}）");
            self.confirm = Some(ConfirmState {
                message: format!("{name} 有未保存的修改"),
                detail: "关闭前保存？".to_string(),
                action: ConfirmAction::CloseTab {
                    idx: index,
                    target: self.tabs[index].editor.entity_id(),
                },
                selected: 0,
                buttons: CONFIRM_BUTTONS.to_vec(),
            });
            // 模态接管焦点（编辑器隔离——可打印键不再入 buffer）。
            window.focus(&self.confirm_focus.clone(), cx);
            cx.notify();
            return;
        }
        self.close_tab_at(index, window, cx);
    }

    /// F3/F4：关闭指定标签（无确认——确认面在 [`Self::close_active_tab`]）。
    fn close_tab_at(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        let old_len = self.tabs.len();
        if idx >= old_len {
            return;
        }
        // #289-④：被关标签若持有页视图，一并丢弃（缓存随实体释放）。
        // #51 D4b：丢弃前快照（重开同文件回到原页原档——关标签不丢
        // 阅读位置）。
        let closed_path = self.tabs[idx].path.clone();
        // Session-local history contains file-backed tabs only, never git previews.
        if closed_path.is_absolute() {
            self.closed_tabs.retain(|path| path != &closed_path);
            self.closed_tabs.push(closed_path.clone());
            if self.closed_tabs.len() > 20 {
                self.closed_tabs.remove(0);
            }
        }
        if let Some((owned, view)) = self.pdf_pages.take() {
            if owned == closed_path {
                let snap = view.read(cx).snapshot();
                self.pdf_view_snapshots.insert(owned, snap);
                // #420：侧栏与页视图同生命周期（同路径丢弃）。
                self.pdf_thumbs = None;
            } else {
                self.pdf_pages = Some((owned, view)); // 非本标签——放回。
            }
        }
        // #389 阶段 1：网格视图同款生命周期（关标签丢弃）。
        if let Some((owned, _)) = &self.xlsx_grid {
            if *owned == closed_path {
                self.xlsx_grid = None;
            }
        }
        // #50 C1：docx 页面视图同款生命周期（关标签丢弃——会话随实
        // 体 Drop 拆除，worker 关停）。
        if let Some((owned, _)) = &self.docx_pages {
            if *owned == closed_path {
                self.docx_pages = None;
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
        // #114 批①：关标签落盘（会话恢复——标签集即变更即存）。
        self.persist_session_tabs();
        // #289-④：新活动标签若是 pdf → 页视图随标签重建。
        self.ensure_pdf_pages(cx);
        // #389 阶段 1：新活动标签若是 xlsx → 网格视图随标签重建。
        self.ensure_xlsx_grid(cx);
        // #50 C1：新活动标签若是 docx → LOK 页面视图随标签重建。
        self.ensure_docx_pages(cx);
        // 焦点跟到新活动标签（空态回退 shell 根域）。
        self.focus_active(window, cx);
        cx.notify();
    }

    /// F4：确认框按钮激活（键盘 Enter / 鼠标点击）。`choice`：
    /// #109 批③：全部替换请求（面板钩子入口）：预览计数 → 确认框
    /// （复用 #166 删除确认双选形态：0 全部替换 / 1 取消）。
    fn request_search_replace(
        &mut self,
        pattern: String,
        replacement: String,
        options: crate::project_search::SearchOptions,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.workspace_root(cx) else {
            self.show_notice("未打开工作区——先打开文件夹（Ctrl+K Ctrl+O）".into(), cx);
            return;
        };
        // 预览数据面（批① preview_replace：文件清单 + 处数——后台跑，
        // 主线程只弹确认）。
        let root_clone = root.clone();
        let pattern_clone = pattern.clone();
        let options_clone = options.clone();
        cx.spawn(async move |this, cx| {
            let preview = cx
                .background_executor()
                .spawn(async move {
                    crate::project_search::preview_replace(
                        &root_clone,
                        &pattern_clone,
                        &options_clone,
                    )
                })
                .await;
            let _ = this.update(cx, |shell, cx| {
                match preview {
                    Ok(list) if list.is_empty() => {
                        shell.show_notice("无匹配可替换".into(), cx);
                    }
                    Ok(list) => {
                        let files = list.len();
                        let count: usize = list.iter().map(|(_, n)| n).sum();
                        // 确认框（detail = 文件数/处数概览——首批文件清单
                        // 摘要，防超长）。
                        let mut detail = list
                            .iter()
                            .take(8)
                            .map(|(p, n)| {
                                let rel = p
                                    .strip_prefix(&root)
                                    .map(|r| r.display().to_string())
                                    .unwrap_or_else(|_| p.display().to_string());
                                format!("{rel}（{n} 处）")
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        if files > 8 {
                            detail.push_str(&format!("\n…等 {files} 个文件"));
                        }
                        shell.confirm = Some(ConfirmState {
                            message: "在打开的工作区全部替换？".into(),
                            detail: format!("共 {files} 个文件 / {count} 处将替换\n{detail}"),
                            action: ConfirmAction::SearchReplaceAll {
                                pattern,
                                replacement,
                                options,
                            },
                            selected: 1,
                            buttons: vec!["全部替换", "取消"],
                        });
                        // 模态焦点接管由既有确认框渲染层承担（confirm_focus）。
                        cx.notify();
                    }
                    Err(message) => {
                        shell.show_notice(format!("替换预览失败：{message}"), cx);
                    }
                }
            });
        })
        .detach();
    }

    /// #109 批③：确认后的执行（写盘 + 善后 + toast 报数）。
    fn search_replace_confirmed(
        &mut self,
        pattern: &str,
        replacement: &str,
        options: &crate::project_search::SearchOptions,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.workspace_root(cx) else {
            return;
        };
        // 执行前快照打开标签的 mtime（替换写盘后比对——精确善后判定）。
        let before: Vec<(usize, Option<std::time::SystemTime>)> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                (
                    i,
                    std::fs::metadata(&tab.path).and_then(|m| m.modified()).ok(),
                )
            })
            .collect();
        let root_c = root.clone();
        let pattern_c = pattern.to_string();
        let replacement_c = replacement.to_string();
        let options_c = options.clone();
        cx.spawn(async move |this, cx| {
            let report = cx
                .background_executor()
                .spawn(async move {
                    crate::project_search::replace_all(
                        &root_c,
                        &pattern_c,
                        &replacement_c,
                        &options_c,
                    )
                })
                .await;
            let _ = this.update(cx, |shell, cx| {
                match report {
                    Ok(rep) => {
                        // 善后：打开标签 mtime 变化 → 重载（#134 reload_tab
                        // 完整编辑链；未变/mtime 不可得跳过——写失败另 toast）。
                        let mut reloaded = 0;
                        for (i, before_m) in &before {
                            let Some(tab) = shell.tabs.get(*i) else {
                                continue;
                            };
                            let now_m =
                                std::fs::metadata(&tab.path).and_then(|m| m.modified()).ok();
                            if now_m.is_some() && *before_m != now_m {
                                if shell.reload_tab(*i, cx) {
                                    reloaded += 1;
                                }
                            }
                        }
                        shell.show_notice(
                            format!(
                                "已替换 {} 处（{} 个文件）· 重载 {reloaded} 个打开标签",
                                rep.replacements, rep.files_changed
                            ),
                            cx,
                        );
                        if rep.failed > 0 {
                            shell.show_notice(
                                format!("{} 个文件写盘失败——详见应用日志", rep.failed),
                                cx,
                            );
                        }
                        // 搜索面板结果重扫（内容已变——面板自刷新）。
                        if let Some(panel) = shell.search_panel.clone() {
                            panel.update(cx, |p, cx| p.rerun_after_replace(cx));
                        }
                    }
                    Err(message) => {
                        shell.show_notice(format!("替换失败：{message}"), cx);
                    }
                }
            });
        })
        .detach();
    }

    /// 0 保存 / 1 不保存 / 2 取消。
    fn resolve_confirm(&mut self, choice: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(confirm) = self.confirm.take() else {
            return;
        };
        match confirm.action {
            ConfirmAction::CloseTab { target, .. } => {
                let Some(idx) = self.tab_index_by_id(target) else {
                    self.close_queue.clear();
                    self.focus_active(window, cx);
                    cx.notify();
                    return;
                };
                match choice {
                    0 => {
                        // 保存后关（保存失败不关——不静默丢内容）。#386：
                        // 走 Shell 分流（pdf 批注增量落盘在此发生）；
                        // 只 Saved/Unmodified 才关。
                        let outcome = self.save_tab(idx, cx);
                        if matches!(outcome, SaveOutcome::Saved | SaveOutcome::Unmodified) {
                            self.close_tab_at(idx, window, cx);
                        } else {
                            self.close_queue.clear();
                        }
                    }
                    1 => self.close_tab_at(idx, window, cx),
                    _ => {
                        self.close_queue.clear();
                    }
                }
                if choice < 2 {
                    self.drain_close_queue(window, cx);
                }
            }
            ConfirmAction::Quit => match choice {
                0 => {
                    // #386：逐标签走 Shell 分流（pdf 批注一并落盘；
                    // 失败已 toast 示警——与旧编辑器路径同语义：
                    // 尽力保存后退出）。
                    for i in 0..self.tabs.len() {
                        let outcome = self.save_tab(i, cx);
                        if !matches!(outcome, SaveOutcome::Saved | SaveOutcome::Unmodified) {
                            // A failed write must not turn "save and quit" into
                            // an implicit discard. Keep the dialog for retry.
                            self.confirm = Some(confirm);
                            window.focus(&self.confirm_focus, cx);
                            cx.notify();
                            return;
                        }
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
            // #107 P1②：移动确认（0 移动 / 1 取消——默认取消侧）。
            ConfirmAction::MovePath { old, new } => match choice {
                0 => self.move_path_confirmed(&old, &new, cx),
                _ => {}
            },
            // #109 批③：全部替换确认（0 全部替换 / 1 取消）。
            ConfirmAction::SearchReplaceAll {
                pattern,
                replacement,
                options,
            } => match choice {
                0 => self.search_replace_confirmed(&pattern, &replacement, &options, cx),
                _ => {}
            },
        }
        if self.confirm.is_some() {
            window.focus(&self.confirm_focus, cx);
        } else {
            self.focus_active(window, cx);
        }
        cx.notify();
    }

    /// 焦点归回当前活动标签的编辑器（确认框关闭后；空态回退 shell
    /// 根域——键位不聋）。
    fn focus_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirm.is_some() {
            window.focus(&self.confirm_focus, cx);
            return;
        }
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
                // #50 C1 / #47 A4：docx 页面态——焦点落 LOK 页视图（编辑
                // 器不在树内，其句柄游离则键事件死在合成根——#422 同款
                // 病灶；toggle 切页面态/关标签落 docx/菜单面板关闭归焦
                // 全经本函数，此前漏 docx 分支一律落游离句柄）。
                if self.docx_view_mode
                    && tab.editor.read(cx).doc_kind() == "docx"
                    && let Some((_, view)) = self.docx_pages.as_ref()
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
    /// #107 D1①批②：标题栏「刷新」钮动作——文件树全树重读（外部
    /// 改动范围未知时不猜目标：整树已缓存清单过一遍，根必命中——
    /// TreeState::new 即读根且展开态常驻；展开态不动，不折叠用户
    /// 已展开目录）。无工作区时无操作（无树可刷）。
    pub fn refresh_file_tree(&mut self, cx: &mut Context<Self>) {
        if let Some(ws) = self.workspace.as_ref().cloned() {
            ws.update(cx, |tree, cx| {
                tree.reload_root();
                cx.notify();
            });
        }
    }

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

    /// #107 P1②：拖拽移动确认（二选一——默认「取消」危险侧；
    /// 守卫：目标已存在/移进自身拒前置 toast 不弹窗）。
    pub fn confirm_move_path(
        &mut self,
        old: PathBuf,
        new: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if new == old {
            return; // 同路径无操作。
        }
        if new.starts_with(&old) {
            self.show_notice("移动无效：目标在被移动目标内部".into(), cx);
            return;
        }
        if new.exists() {
            self.show_notice(format!("移动无效：目标已存在（{}）", new.display()), cx);
            return;
        }
        let name = old
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| old.display().to_string());
        let dir_name = new
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        self.confirm = Some(ConfirmState {
            message: format!("移动「{name}」"),
            detail: format!("到 {}（标签路径随之改写）", dir_name),
            action: ConfirmAction::MovePath { old, new },
            selected: 1,
            buttons: vec!["移动", "取消"],
        });
        window.focus(&self.confirm_focus.clone(), cx);
        cx.notify();
    }

    /// #107 P1②：移动执行（fs::rename + **前缀重写**打开标签路径——
    /// 目录移动时 starts_with(old) 全量 plan（D2：前缀重写面牵连多
    /// 标签）；文件移动精确匹配）+ 树同步 + mtime 基线刷新（防轮询
    /// 误报）+ toast。
    fn move_path_confirmed(&mut self, old: &Path, new: &Path, cx: &mut Context<Self>) {
        match std::fs::rename(old, new) {
            Ok(()) => {
                eprintln!("file-tree: 移动 {} → {}", old.display(), new.display());
                // 前缀重写 plan：目录 → starts_with 全量；文件 → 精确。
                let plan: Vec<(usize, PathBuf)> = self
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.path.starts_with(old))
                    .map(|(i, t)| {
                        let rel = t.path.strip_prefix(old).unwrap_or(std::path::Path::new(""));
                        (i, new.join(rel))
                    })
                    .collect();
                let n = plan.len();
                self.apply_tab_renames(&plan, cx);
                // 标签路径已变 → mtime 基线同步重置（防轮询误报）。
                self.mtime_baselines.clear();
                self.sync_tree_after_fs(old, true, cx);
                self.sync_tree_after_fs(new, false, cx);
                self.sync_tree_active(cx);
                self.show_notice(
                    format!(
                        "已移动（{} 个打开标签路径改写）",
                        if old.is_dir() { n } else { n.max(1) }
                    ),
                    cx,
                );
            }
            Err(err) => {
                self.show_notice(
                    format!("移动失败（{} → {}）：{err}", old.display(), new.display()),
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

    /// #306 ①：OCR 识别扫描件（菜单「文件 → OCR 识别扫描件…」）。
    ///
    /// 流：pdf+扫描件+未识别门槛（双保险——菜单灰显外方法再验）→
    /// 读盘 → tesseract 环境探针（二进制缺席 = 原生错误对话框 + 指引；
    /// 语言面：chi_sim+eng > chi_sim > eng，全缺 = 对话框指引装包）→
    /// **专用线程逐页词级识别**（`ocr_page_words`——引擎面每页独立
    /// 装载/渲染/识别；mpsc 回流进度，前台 16ms 轮询 drain 更新横幅
    /// ——UI 零卡顿面，大文档逐页推进）→ Done 携全量词表 →
    /// `PdfPayload::with_ocr` 落 Editor（文本视图即读面）→ 完成横幅
    /// （词数 + 失败页诚实计数 + 可搜索/另存指引）。
    ///
    /// 失败面（诚实语义）：第 1 页即失败 = 环境性失败（语言包/引擎）→
    /// 不应用任何结果，错误横幅；后续页失败 = 单页识别失败 → 该页
    /// 记空（行模型占位「本页识别无词」），继续后续页，完成横幅带
    /// 失败页清单。
    fn ocr_active_pdf(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let editor = tab.editor.clone();
        if !editor.read(cx).pdf_ocr_pending() {
            self.show_notice(
                "OCR 仅支持未识别的扫描件 PDF（当前标签不适用）。".to_string(),
                cx,
            );
            return;
        }
        let path = tab.path.clone();
        if !path.exists() {
            self.show_notice(
                format!("源文件不在磁盘上：{}（先确保文件已保存）", path.display()),
                cx,
            );
            return;
        }
        let Some(page_count) = editor.read(cx).pdf_props().map(|p| p.pages) else {
            return;
        };
        if page_count == 0 {
            self.show_notice("PDF 页数为 0——无 OCR 对象。".to_string(), cx);
            return;
        }
        // tesseract 环境探针（引擎面单一真源——ocr_available）。
        let env = mdc_pdf::ocr::ocr_available();
        if !env.binary_ok {
            let message = "OCR 识别需要 tesseract（未探测到）。安装示例（Debian/Ubuntu）：\n\
                 sudo apt-get install tesseract-ocr tesseract-ocr-chi-sim\n\
                 或经 MDC_OCR_BIN 指定可执行路径。";
            let shown = crate::native_dialog::show_error(&crate::native_dialog::DialogSpec {
                title: "OCR 需要 tesseract".to_string(),
                message: message.to_string(),
            });
            if !shown {
                self.show_notice(message.to_string(), cx);
            }
            return;
        }
        let lang: &'static str = if env.has_lang("chi_sim") && env.has_lang("eng") {
            "chi_sim+eng"
        } else if env.has_lang("chi_sim") {
            "chi_sim"
        } else if env.has_lang("eng") {
            "eng"
        } else {
            let message = format!(
                "tesseract 无可用语言包（探测：{:?}）。中文识别请取件：\n\
                 test-artifacts/ocr-tessdata/README.md（chi_sim 取件脚本）",
                env.languages
            );
            let shown = crate::native_dialog::show_error(&crate::native_dialog::DialogSpec {
                title: "OCR 缺语言包".to_string(),
                message: message.clone(),
            });
            if !shown {
                self.show_notice(message, cx);
            }
            return;
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.show_notice(format!("读取源文件失败（{}）：{e}", path.display()), cx);
                return;
            }
        };
        if self.ocr_rx.is_some() {
            self.show_notice("OCR 识别正在进行中——请等待完成。".to_string(), cx);
            return;
        }
        // 专用线程（pdf_pages 搜索索引同款纪律：线程 + mpsc + 轮询 drain）。
        let (tx, rx) = std::sync::mpsc::channel::<OcrProgressMsg>();
        let lang_owned = lang.to_string();
        std::thread::spawn(move || {
            let mut pages: Vec<Vec<mdc_pdf::OcrWord>> = Vec::with_capacity(page_count);
            let mut word_count = 0usize;
            let mut failed_pages: Vec<usize> = Vec::new();
            for page in 0..page_count {
                match mdc_pdf::ocr::ocr_page_words(&bytes, page, &lang_owned) {
                    Ok(words) => {
                        word_count += words.len();
                        pages.push(words);
                    }
                    Err(e) if page == 0 => {
                        // 首页即失败 = 环境性（语言包/引擎）——不应用任何
                        // 结果，错误直报（Done.error 面）。
                        let _ = tx.send(OcrProgressMsg::Done {
                            error: Some(format!("{e:?}")),
                            pages: Vec::new(),
                            word_count: 0,
                            failed_pages: Vec::new(),
                            lang: lang_owned,
                        });
                        return;
                    }
                    Err(e) => {
                        eprintln!("ocr: 第 {} 页识别失败：{e:?}", page + 1);
                        failed_pages.push(page);
                        pages.push(Vec::new()); // 失败页占位（行模型空页标记）
                    }
                }
                let _ = tx.send(OcrProgressMsg::Page {
                    done: page + 1,
                    total: page_count,
                    words: word_count,
                });
            }
            let _ = tx.send(OcrProgressMsg::Done {
                error: None,
                pages,
                word_count,
                failed_pages,
                lang: lang_owned,
            });
        });
        self.ocr_rx = Some(rx);
        self.show_notice(format!("OCR 识别中（{lang}）：0/{page_count} 页…"), cx);
        if !self.ocr_polling {
            self.ocr_polling = true;
            Self::spawn_ocr_progress_loop(window, cx);
        }
    }

    /// #107 P1①：mtime 单步轮询（2s 周期的循环体——pub 测试面）：
    /// 比对各标签磁盘 mtime 与基线 → 干净标签自动重载（#134 reload_tab
    /// 链）+ toast；脏标签**不自动覆盖**（脏缓冲冲突策略——防丢用户
    /// 未保存编辑，toast 警示交还决策权）；不可重载态（只读渲染/
    /// docx 行模型）跳过。返回变化的标签数（循环日志用）。
    pub fn poll_mtimes_once(&mut self, cx: &mut Context<Self>) -> usize {
        if self.tabs.is_empty() {
            return 0;
        }
        // 基线尺寸对齐（打开/关闭标签后首步补齐）。
        if self.mtime_baselines.len() != self.tabs.len() {
            self.mtime_baselines = self
                .tabs
                .iter()
                .map(|tab| std::fs::metadata(&tab.path).and_then(|m| m.modified()).ok())
                .collect();
            return 0; // 首步只建基线（不比对——防打开即误报）。
        }
        let mut changed = 0;
        let mut notices: Vec<String> = Vec::new();
        for i in 0..self.tabs.len() {
            let now_m = std::fs::metadata(&self.tabs[i].path)
                .and_then(|m| m.modified())
                .ok();
            let baseline = self.mtime_baselines.get(i).copied().flatten();
            if now_m.is_none()
                || now_m == baseline
                || Some(now_m) == Some(baseline) && baseline.is_some()
            {
                continue;
            }
            changed += 1;
            // 基线即时推进（本轮已感知——重载/警示后的新基线）。
            self.mtime_baselines[i] = now_m;
            if self.tab_is_modified(i, cx).unwrap_or(false) {
                // 脏标签：不自动覆盖——警示交还决策权。
                notices.push(format!(
                    "{} 在磁盘已被外部修改——未保存编辑在前（保存将覆盖外部改动）",
                    self.tabs[i].path.display()
                ));
            } else if self.tab_can_reload(i, cx) {
                if self.reload_tab(i, cx) {
                    notices.push(format!(
                        "检测到外部修改并已重载：{}",
                        self.tabs[i].path.display()
                    ));
                }
            }
            // 不可重载态（只读渲染/docx 行模型）：静默跳过——
            // 重载确认链在该态无语义（页视图保存另有 C4 面）。
        }
        for n in notices {
            self.show_notice(n, cx);
        }
        if changed > 0 {
            cx.notify();
        }
        changed
    }

    /// #107 P1①：mtime 轮询循环（2s 周期——tabs 非空期常驻；空则停）。
    fn spawn_mtime_poll_loop(&mut self, cx: &mut Context<Self>) {
        if self.mtime_polling {
            return;
        }
        self.mtime_polling = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
                let mut stop = false;
                let _ = this.update(cx, |shell, cx| {
                    shell.poll_mtimes_once(cx);
                    if shell.tabs.is_empty() {
                        shell.mtime_polling = false;
                        stop = true;
                    }
                });
                if stop {
                    break;
                }
            }
        })
        .detach();
    }

    /// #306 ①：OCR 进度轮询循环（16ms 定时 drain——pdf_pages 搜索索引
    /// 同款；Done 消费即止。`spawn_in`/`update_in` 携窗口句柄——Done
    /// 应用面的视图切换需要 window [#604 后 toggle_pdf_view_mode
    /// 重聚焦需窗口]）。
    fn spawn_ocr_progress_loop(window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(OCR_POLL_INTERVAL).await;
                let mut stop = false;
                let _ = this.update_in(cx, |shell, window, cx| {
                    shell.drain_ocr_progress(window, cx);
                    if shell.ocr_rx.is_none() {
                        shell.ocr_polling = false;
                        stop = true;
                    }
                });
                if stop {
                    break;
                }
            }
        })
        .detach();
    }

    /// #306 ①：OCR 回流消费（进度横幅更新 / Done 应用 + 终态横幅）。
    fn drain_ocr_progress(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rx) = self.ocr_rx.take() else {
            return;
        };
        let mut done: Option<OcrProgressMsg> = None;
        while let Ok(msg) = rx.try_recv() {
            if matches!(msg, OcrProgressMsg::Done { .. }) {
                done = Some(msg);
            } else if let OcrProgressMsg::Page {
                done: d,
                total,
                words,
            } = msg
            {
                self.show_notice(format!("OCR 识别中：{d}/{total} 页（{words} 词）…"), cx);
            }
        }
        self.ocr_rx = if done.is_some() { None } else { Some(rx) };
        let Some(OcrProgressMsg::Done {
            error,
            pages,
            word_count,
            failed_pages,
            lang,
        }) = done
        else {
            return;
        };
        if let Some(err) = error {
            self.show_notice(format!("OCR 识别失败（环境性）：{err}"), cx);
            return;
        }
        // 应用面：活动标签仍是识别发起的 pdf（防御——识别期间可能切
        // 标签；标签不在/形态变 = 丢弃结果 + 诚实横幅）。
        let Some(tab) = self.tabs.get(self.active) else {
            self.show_notice(
                "OCR 完成，但发起标签已关闭——识别结果未应用（重开文件可重识别）。".to_string(),
                cx,
            );
            return;
        };
        let editor = tab.editor.clone();
        if !editor.read(cx).pdf_ocr_pending() {
            self.show_notice(
                "OCR 完成，但活动标签已不是未识别扫描件——结果未应用。".to_string(),
                cx,
            );
            return;
        }
        let page_total = pages.len();
        editor.update(cx, |ed, cx| {
            let Some(payload) = ed.pdf_payload().cloned() else {
                return;
            };
            let outcome = crate::editor::pdf_doc::PdfOcrOutcome {
                lang,
                pages,
                word_count,
            };
            ed.apply_pdf_ocr(payload.with_ocr(outcome), cx);
        });
        // 文本视图是读面宿主——页视图态切到文本（识别文本即读即搜；
        // #604 后 toggle 需 window 重聚焦——update_in 携窗口句柄）。
        if self.pdf_view_mode {
            self.toggle_pdf_view_mode(window, cx);
        }
        let failed_note = if failed_pages.is_empty() {
            String::new()
        } else {
            format!(
                "；{} 页识别失败（{}——空页处理）",
                failed_pages.len(),
                failed_pages
                    .iter()
                    .map(|p| (p + 1).to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        self.show_notice(
            format!(
                "OCR 完成：{} 页 {} 词{failed_note}——文本可读可搜（Ctrl+F）；\
                 如需任何阅读器可搜，用「文件 → 另存为可搜索 PDF…」。",
                page_total, word_count
            ),
            cx,
        );
    }

    /// #306 ②：另存为可搜索 PDF（菜单「文件 → 另存为可搜索 PDF…」）。
    ///
    /// 流：pdf+已识别门槛（双保险）→ 输出路径对话框（建议名
    /// `<原名>_可搜索.pdf`）→ 后台写回（读盘 → `write_searchable_pdf`
    /// [3 Tr 隐形文本层] → 写盘 → **重开验证**[pdf_doc::load_from_bytes
    /// ——文本层非空即通过；写回产物自证]）→ 成功横幅（词数 + 验证
    /// 结论）/ 失败横幅（typed 错误不静默）。
    fn save_searchable_pdf(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let editor = tab.editor.clone();
        if !editor.read(cx).pdf_ocr_ready() {
            self.show_notice(
                "另存为可搜索 PDF 需先识别（文件 → OCR 识别扫描件…）。".to_string(),
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
        // 词表随 payload 携带（Editor 单一真源——克隆即快照）。
        let Some(outcome) = editor.read(cx).pdf_payload().and_then(|p| p.ocr.clone()) else {
            self.show_notice("OCR 词表不在位（请重新识别）。".to_string(), cx);
            return;
        };
        let directory = src
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let suggested = src
            .file_stem()
            .map(|n| format!("{}_可搜索.pdf", n.to_string_lossy()));
        let rx = cx.prompt_for_new_path(&directory, suggested.as_deref());
        cx.spawn_in(window, async move |this, cx| match rx.await {
            Ok(Ok(Some(out_path))) => {
                // 后台写回 + 重开验证（IO + 引擎全离主线程——核心链路
                // = [`Self::write_searchable_to`]，对话框闭包外独立函数
                // = 手势级 e2e 可测面）。
                let src_clone = src.clone();
                let out_clone = out_path.clone();
                let result = cx
                    .background_executor()
                    .spawn(
                        async move { Self::write_searchable_to(&src_clone, &out_clone, &outcome) },
                    )
                    .await;
                let _ = this.update_in(cx, |shell, _window, cx| match result {
                    Ok((words, chars)) => {
                        shell.show_notice(
                            format!(
                                "已另存为可搜索 PDF（注入 {} 词，重开验证文本层 {} 字）：{}",
                                words,
                                chars,
                                out_path.display()
                            ),
                            cx,
                        );
                    }
                    Err(message) => {
                        shell.show_notice(format!("另存可搜索 PDF 失败：{message}"), cx);
                    }
                });
            }
            Ok(Ok(None)) => {}
            Ok(Err(err)) => {
                let message = format!("另存对话框不可用：{err}");
                let _ = this.update_in(cx, |shell, _window, cx| {
                    shell.show_notice(message, cx);
                });
            }
            Err(_canceled) => {}
        })
        .detach();
    }

    /// #306 ②：另存核心链路（读盘 → 写回[3 Tr 隐形文本层] → 落盘 →
    /// **重开验证**[pdf_doc::load_from_bytes——文本层非空即闭环；引擎面
    /// 双引擎提取已由 pdff4_ocr 锁 7 背书]）。
    ///
    /// 对话框闭包外的独立函数 = 手势级 e2e 可测面（`prompt_for_new_path`
    /// 在 TestAppContext 不可用——核心链路经此直接验证，对话框层是薄壳）。
    fn write_searchable_to(
        src: &Path,
        out_path: &Path,
        outcome: &crate::editor::pdf_doc::PdfOcrOutcome,
    ) -> Result<(usize, usize), String> {
        let pdf = std::fs::read(src).map_err(|e| format!("读取源文件失败：{e}"))?;
        let (out, report) = mdc_pdf::ocr_writeback::write_searchable_pdf(&pdf, &outcome.pages)
            .map_err(|e| format!("写回失败：{e:?}"))?;
        std::fs::write(out_path, &out).map_err(|e| format!("写出 PDF 失败：{e}"))?;
        let payload = crate::editor::pdf_doc::load_from_bytes(out)
            .map_err(|e| format!("重开验证失败：{e}"))?;
        if payload.props.scanned {
            return Err("重开验证失败：写出文件仍无文本层".to_string());
        }
        Ok((report.words_written, payload.props.chars))
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
    /// #110 G1 批B：四态泛化（&str——三态键或 user:<name>；写盘 String）。
    pub fn set_theme_mode(&mut self, mode: &str, cx: &mut Context<Self>) {
        if self.theme_mode == mode {
            return;
        }
        self.theme_mode = mode.to_string();
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

    /// #111 P2：软换行开关传播（存盘 + 全标签 Editor::set_soft_wrap——
    /// 关 = 已有断点清空显示行回落；沿 set_indent_unit_all 同型）。
    pub fn set_soft_wrap_all(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.soft_wrap == on {
            return;
        }
        self.soft_wrap = on;
        crate::settings::save_soft_wrap(on);
        for tab in &self.tabs {
            tab.editor.update(cx, |ed, _| ed.set_soft_wrap(on));
        }
        eprintln!("soft-wrap: {}", if on { "on" } else { "off" });
        cx.notify();
    }

    /// #182：重置左右栏宽度（设置面板「重置默认」钮触发——恢复缺省宽
    /// + 持久化；与拖拽收尾 finish_panel_drag 同一存储面）。
    pub fn reset_panel_widths(&mut self, cx: &mut Context<Self>) {
        self.left_width = panels::LEFT_PANEL_WIDTH;
        self.right_width = panels::RIGHT_PANEL_WIDTH;
        let (left, right) = (f32::from(self.left_width), f32::from(self.right_width));
        crate::settings::save_panel_widths(left, right);
        // #120（批②）：per-panel 记忆宽一并清（重置语义 = 全部回缺省宽）。
        let dock = self.left_dock.clone();
        dock.update(cx, |dock, cx| dock.clear_panel_sizes(cx));
        self.persist_dock_layout(cx);
        eprintln!("panel-reset: left {left} right {right} (persist)");
        cx.notify();
    }

    /// F6：循环主题（Ctrl+T——system → light → dark → system；
    /// 应用 + 持久化 + 通知）。
    pub fn cycle_theme(&mut self, cx: &mut Context<Self>) {
        let next = theme_switch::next_mode(&self.theme_mode);
        self.theme_mode = next.to_string();
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
        self.workspace = Some(tree.clone());
        // F7：git 面板（工作区打开即建——非 git 目录/缺 git 走提示态，
        // 不崩溃不拦截）。
        let git =
            cx.new(|cx| GitPanel::new(root.clone(), Self::git_panel_hooks(shell.clone()), cx));
        self.git_panel = Some(git.clone());
        // #109 批②③：项目搜索面板（工作区打开即建——面板实体承载
        // 输入/后台扫描/结果/替换链；无工作区引导文案在面板内）。
        let search = cx.new(|cx| SearchPanel::new(Self::search_panel_hooks(shell), cx));
        search.update(cx, |panel, cx| panel.set_root(Some(root.clone()), cx));
        self.search_panel = Some(search.clone());
        // #119（mdc-dock 批①）：面板注册进左栏 dock——旧面板撤注、新面
        // 板入列、激活键承接（无历史激活 → 文件树缺省；旧 git 态/预物化
        // 意图经 add_panel 与 activate_key 自动兑现——与旧 git_view 保
        // 语义：换工作区时视图不跳变）。#120（批②）：开合态跨撤注保持
        //（撤注激活面板会收起 dock——蓝本 ：911 语义，换工作区不是用户
        // 关栏意图，保开）；per-panel 尺寸与激活键随后同步 + 落盘。
        let (prev_view, prev_open) = {
            let dock = self.left_dock.read(cx);
            (dock.active_key(), dock.is_open())
        };
        let dock = self.left_dock.clone();
        dock.update(cx, |dock, cx| {
            dock.remove_panel_by_key(crate::file_tree::FILE_TREE_PANEL_KEY, cx);
            dock.remove_panel_by_key(crate::git_panel::GIT_PANEL_KEY, cx);
            dock.remove_panel_by_key(crate::search_panel::SEARCH_PANEL_KEY, cx);
            dock.add_panel(&tree, cx);
            dock.add_panel(&git, cx);
            dock.add_panel(&search, cx);
            dock.set_open(prev_open, cx);
            match prev_view {
                Some(key) => {
                    dock.activate_key(key, cx);
                }
                None => {
                    dock.activate_key(crate::file_tree::FILE_TREE_PANEL_KEY, cx);
                }
            }
        });
        self.sync_left_width_to_active(cx);
        self.persist_dock_layout(cx);
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
                        let _ = this.update(cx, |shell, cx| {
                            if path.is_dir() {
                                shell.open_workspace(path, cx);
                            } else {
                                shell.show_notice(
                                    format!("文件夹不存在或不可访问：{}", path.display()),
                                    cx,
                                );
                            }
                        });
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

/// A1（#47 批次 2）：右 dock 让位临界宽——窗口宽低于此值时右栏在渲染期
/// 让位（编辑区保 ≥240px 可用宽：三固定栏 52+260+280=592px，760px
/// 最小窗宽下编辑区实测仅 108px）。**不改写 `right_open` 用户开关态**：
/// 宽度恢复后右栏按原开关态自动回归。900px 取值使临界处编辑区恰保
/// ≥240px（width-652=248，probe 实测口径）。
const RIGHT_DOCK_YIELD_WIDTH: Pixels = px(900.);

impl Render for ShellApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // #164 1c：首帧计时（首个 render = 首帧呈现；消费后恒 None——
        // 后续 render 零开销）。红线 <2s（#68 设计口径）。
        if let Some(since) = self.perf_first_frame.take() {
            crate::perf::stage("first-frame", since);
        }
        let colors = cx.theme().colors().clone();
        // A1（#47 批次 2）：窄窗右栏让位（渲染期判定——viewport 宽低于
        // RIGHT_DOCK_YIELD_WIDTH 时右栏不占位，编辑区保可用宽；right_open
        // 用户开关态不被改写，宽度恢复后按原开关态回归）。
        let right_open_effective =
            self.right_open && window.viewport_size().width >= RIGHT_DOCK_YIELD_WIDTH;
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

                if key == "escape" && this.document_info_open {
                    this.document_info_open = false;
                    this.focus_active(window, cx);
                    cx.notify();
                    return;
                }

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
                            this.resolve_confirm(2, window, cx);
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

                // Home-only Tab traversal; Enter/Space are native Button clicks.
                if this.editor().is_none() && !this.settings_open
                    && this.menu_bar_open.is_none() && this.context_menu.is_none()
                    && (this.root_focus.is_focused(window)
                        || this.home_focus.iter().any(|focus| focus.is_focused(window)))
                    && key == "tab" && !ctrl && !ks.modifiers.alt
                {
                    if this.root_focus.is_focused(window) {
                        let index = if ks.modifiers.shift { 2 } else { 0 };
                        window.focus(&this.home_focus[index], cx);
                    } else if ks.modifiers.shift { window.focus_prev(cx); }
                    else { window.focus_next(cx); }
                    cx.stop_propagation();
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

                // #50 C2：docx 页面视图键域——缩放命令（pdf #421 分支同构
                // 先例：视口命令在 keymap 表之前分流，消费即 return 防
                // Ctrl+0/=/- 落 keymap 字号键二次响应）。键位：Ctrl+0
                // 适应宽度 / Ctrl+1 实际大小 / Ctrl+= · Ctrl+- 步进缩放
                //（连续画布无「适应页面」档——页边界依赖 B 线页面几何
                // op，#49 已登记；Ctrl+滚轮在视图内 capture 期拦截，
                // docx_pages 探针注册，不经本分支）。非 docx 页面态/文本
                // 视图不进本分支，键位行为零变化。
                if this.docx_view_mode && this.active_is_docx(cx) {
                    if let Some((_, entity)) = &this.docx_pages {
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
                        ShellAction::ReopenClosedTab => {
                            this.reopen_closed_tab(window, cx);
                            return;
                        }
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
                        // #109 批②③：Ctrl+Shift+F 项目搜索面板（图标条/
                        // 菜单同一 toggle_search 链）。
                        ShellAction::ToggleSearchPanel => {
                            this.toggle_search(cx);
                            return;
                        }
                        // #108 P2③：Ctrl+Shift+O 转到符号（菜单同一
                        // goto_symbol 链——开右栏 + 聚焦大纲过滤框）。
                        ShellAction::GotoSymbol => {
                            this.goto_symbol(window, cx);
                            return;
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
                    // #51 D4：pdf 页面视图内复制——Ctrl+C（或用户换绑的
                    // copy）把当前文本选择送剪贴板（左键拖选产物）。
                    // 编辑器不在树内同上——查 keymap 编辑表换绑跟随；
                    // 视图吞键与否自报（无选择不吞照常下沉，编辑器
                    // Copy 语义零冲突——页面视图态编辑器不在场）。
                    if let keymap::EditorOutcome::Matched(keymap::EditorAction::Copy) =
                        editor_outcome
                    {
                        let mut copied = false;
                        view.update(cx, |v, cx| {
                            copied = v.copy_selection(cx);
                        });
                        if copied {
                            return;
                        }
                    }
                }

                // #176：docx 页面视图键守卫——查找（Ctrl+F）/命令面板
                // （Ctrl+Shift+P）是编辑器动作，页面态编辑器不在树内，
                // 键派发不到（静默无反应 = 用户判定「假按钮」的病灶之一）。
                // 此处接住给明确提示（pdf #422 分支同构先例——查 keymap
                // 编辑表保持换绑跟随；命中即吞，未命中照常下沉不拦）。
                // chord 已在上方 resolve_shell 前终结，编辑器动作无和弦
                // 续接，pending 传 None。
                if this.docx_page_view_active(cx) {
                    let editor_outcome = keymap::store().resolve_editor_main(&kq, None);
                    match editor_outcome {
                        keymap::EditorOutcome::Matched(keymap::EditorAction::OpenFind) => {
                            this.show_notice(
                                "页面视图暂不支持查找——切换到文本视图后使用（状态栏「切换到文本视图」）"
                                    .into(),
                                cx,
                            );
                        }
                        keymap::EditorOutcome::Matched(keymap::EditorAction::PaletteToggle) => {
                            this.show_notice(
                                "页面视图暂不支持命令面板——切换到文本视图后使用".into(),
                                cx,
                            );
                        }
                        _ => {}
                    }
                }
            }))
            .child(title_bar::title_bar(&title, &menus, self.menu_bar_open, cx))
            .child(
                h_flex()
                    .id("shell-body")
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .w_full()
                    .overflow_hidden()
                    .child(activity_bar::activity_bar(
                        // #119/#120：视图归属与开合态均从 dock 读。
                        self.left_open(cx),
                        self.git_active(cx),
                        self.settings_open,
                        // #109 批②③：搜索钮改接项目搜索面板（R8 口径）——
                        // 面板不依赖编辑器浮层，docx 页面视图/无文档均可用
                        //（置灰口径退役；Ctrl+F 编辑器内查找不受影响）。
                        // 搜索视图态在此读（render 持 &self——activity_bar
                        // 内读 ShellApp 会双租约 panic，见其函数注释）。
                        self.search_active(cx),
                        cx,
                    ))
                    .when(self.left_open(cx), |el| {
                        el.child(panels::left_panel(
                            self.editor(),
                            self.workspace.as_ref(),
                            self.git_panel.as_ref(),
                            self.git_active(cx),
                            // G4：可拖拽宽度（域钳制 + settings 持久化）
                            self.left_width,
                            // #121 批③b：标题拖拽手柄（激活面板键上
                            // 挂——FileTree/GitPanel 拖拽源）。
                            self.left_dock.read(cx).active_key().map(|key| {
                                self.panel_title_drag_handle(&self.left_dock.clone(), key, cx)
                                    .into_any_element()
                            }),
                            // #571：大纲面板实体（键盘导航 + #289-④
                            // 页滚钩子在实体构造时注入）。
                            None,
                            cx,
                        ))
                        // G4：左栏右缘 4px 拖拽热区
                        .child(self.panel_resize_handle(PanelSide::Left, cx))
                    })
                    .child(editor_column(self, cx))
                    // A1（#47 批次 2）：窄窗右栏让位（渲染期判定；right_open
                    // 用户开关态不被改写——宽度恢复后按原开关态回归）。
                    .when(right_open_effective, |el| {
                        // G4：右栏左缘 4px 拖拽热区
                        el.child(self.panel_resize_handle(PanelSide::Right, cx))
                            // #121（批④）：右栏呈现面统一——right_dock
                            // 激活面板本体渲染（批③b 拖入的面板真实可见）；
                            // 无激活/未物化 → 旧 right_panel 兜底（#289-⑤
                            // 属性/来源视图）。
                            .child(self.right_dock_content(cx))
                    })
                    // #289-⑤：右缘 dock 条（属性/来源开关归右缘——
                    // 恒在，右栏收起时是唯一入口；zed 折叠 dock 同构）。
                    // A1（#47 批次 2）：入参用有效态——窄窗让位期间呈
                    // 收起观感，点击仍切换 right_open 逻辑态。
                    .child(right_dock::right_dock_strip(right_open_effective, cx))
                    // #121（批③b）：拖放**感应面**（常驻透明全窗——
                    // on_drag_move 分区追踪 + on_drop 收尾；drag 激活即可
                    // 回调（鸡生蛋修正：条件渲染会把回调连着隐藏））。
                    .child(self.drag_drop_sensor(cx))
                    // #121（批③b）：三分区**高亮层**（仅拖拽中——感应面
                    // 已常驻，此处纯视觉）。
                    .when(self.drag_zone.is_some(), |el| {
                        el.child(self.drag_drop_overlay(cx))
                    }),
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
                    &self.theme_mode,
                    self.font_size,
                    self.indent_unit,
                    self.soft_wrap,
                    self.custom_theme_draft.clone(),
                    self.theme_name_input.clone(),
                    self.color_slot_focus.clone(),
                    cx,
                ))
            })
            .when(self.document_info_open, |el| {
                el.child(document_info_overlay(self, cx))
            })
            .child(status_bar::status_bar(
                self.editor().cloned().as_ref(),
                self.workspace
                    .as_ref()
                    .map(|tree| tree.read(cx).root().to_path_buf()),
                self.pdf_view_mode,
                self.xlsx_grid_view_mode,
                self.docx_view_mode,
                // #51 D4b：页面视图态的页码读数（页视图 top_visible_page
                // 驱动——emit 订阅活更新；文本态 None 沿用编辑器光标
                // 页表映射，零变化）。
                self.pdf_view_mode
                    .then(|| {
                        self.pdf_pages.as_ref().and_then(|(_, e)| {
                            let v = e.read(cx);
                            (v.page_count() > 0).then(|| {
                                format!("第 {} / {} 页", v.top_visible_page() + 1, v.page_count())
                            })
                        })
                    })
                    .flatten(),
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
fn editor_column(shell: &mut ShellApp, cx: &mut Context<ShellApp>) -> impl IntoElement {
    let workspace = shell
        .workspace
        .as_ref()
        .map(|tree| tree.read(cx).root().to_path_buf());
    v_flex()
        .id("editor-column")
        .debug_selector(|| "editor-column".into())
        .flex_1()
        .min_w_0()
        .min_h_0()
        .h_full()
        .overflow_hidden()
        .when(!shell.tabs.is_empty(), |el| {
            el.child(
                div()
                    .id("editor-tabs")
                    .debug_selector(|| "editor-tabs".into())
                    .w_full()
                    .flex_none()
                    .child(tab_bar::tab_bar(
                        &shell.tabs,
                        shell.active,
                        workspace.as_deref(),
                        cx,
                    )),
            )
        })
        .child(
            div()
                .id("editor-content")
                .debug_selector(|| "editor-content".into())
                .relative()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .w_full()
                .overflow_hidden()
                .child(editor_area(shell, cx)),
        )
}

fn document_info_overlay(shell: &ShellApp, cx: &mut Context<ShellApp>) -> impl IntoElement {
    let colors = cx.theme().colors().clone();
    div()
        .id("document-info-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui::black().opacity(0.25))
        .child(
            v_flex()
                .id("document-info")
                .debug_selector(|| "document-info".into())
                .w(px(440.))
                .h(px(540.))
                .rounded_md()
                .overflow_hidden()
                .bg(colors.elevated_surface_background)
                .child(
                    h_flex().w_full().justify_end().p_2().child(
                        div()
                            .id("document-info-close")
                            .debug_selector(|| "document-info-close".into())
                            .px_3()
                            .py_1()
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_document_info(cx)))
                            .child(ui::Label::new("关闭")),
                    ),
                )
                .child(panels::document_information(shell.editor(), px(440.), cx)),
        )
}

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
    // #50 C1：docx 标签的 LOK 页面视图（内核真源渲染主显示面）——模
    // 式开且有实体时替代文本行模型呈现（状态栏「页面/文本」钮切换
    // 回退；错误/启动面板在视图内自呈现）。
    let docx_pages_entity = shell
        .editor()
        .filter(|ed| ed.read(cx).doc_kind() == "docx")
        .and_then(|_| shell.docx_view_mode.then(|| ()))
        .and_then(|_| shell.docx_pages.as_ref().map(|(_, e)| e.clone()));
    let active_editor = shell.editor().cloned();
    // #420：缩略图侧栏（与页视图同生命周期；页视图在位才挂——文本
    // 模式/无 pdfium 不出空壳）。
    let pdf_thumbs_entity = pdf_pages_entity
        .as_ref()
        .and_then(|_| shell.pdf_thumbs.clone());
    match (
        pdf_pages_entity,
        docx_pages_entity,
        xlsx_grid_entity,
        active_editor,
    ) {
        (Some(pages), _, _, _) => match pdf_thumbs_entity {
            Some(thumbs) => h_flex()
                .id("pdf-view-with-thumbs")
                .size_full()
                .child(thumbs)
                // 主视图占满余幅（min_w_0 防溢出挤压；size_full 的页根
                // 在此容器内重定尺寸）。
                .child(div().flex_1().min_w_0().h_full().child(pages))
                .into_any_element(),
            None => pages.into_any_element(),
        },
        // #50 C1：docx 页面态——LOK 页视图替代行模型（互斥：pdf 臂在前
        // 优先级不变，xlsx/editor 臂顺延）。
        (None, Some(docx), _, _) => docx.into_any_element(),
        (None, None, Some(grid), _) => grid.into_any_element(),
        (None, None, None, Some(editor)) => editor.into_any_element(),
        (None, None, None, None) => {
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
                .child({
                    let listener =
                        cx.listener(|shell, action: &home_view::HomeAction, window, cx| {
                            shell.activate_home_action(*action, window, cx);
                        });
                    home_view::HomeView::new(&shell.home_focus, move |action, _, window, cx| {
                        listener(&action, window, cx);
                    })
                })
                .child(
                    ui::Label::new("Ctrl+O 打开文档 · Ctrl+K Ctrl+O 打开文件夹")
                        .color(ui::Color::Muted),
                )
                .when(
                    !recent_files.is_empty() || !recent_workspaces.is_empty(),
                    |el| {
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
                                .children(
                                    recent_files
                                        .iter()
                                        .map(|path| recent_row(path, false, hover, cx)),
                                )
                                .when(!recent_workspaces.is_empty(), |el| {
                                    el.child(
                                        ui::Label::new("最近的工作区")
                                            .size(ui::LabelSize::Small)
                                            .color(ui::Color::Muted)
                                            .mt_2(),
                                    )
                                    .children(
                                        recent_workspaces
                                            .iter()
                                            .map(|path| recent_row(path, true, hover, cx)),
                                    )
                                }),
                        )
                    },
                )
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
#[path = "tab_menu.rs"]
mod tab_menu;

#[cfg(test)]
#[path = "tab_menu_tests.rs"]
mod tab_menu_tests;

// ---------------------------------------------------------------------------
// #422：页内查找 Shell 接线——Ctrl+F 不切视图（页面视图内查找条）
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// #176 方案 A：docx 页面视图动作诚实面交互测试（TestAppContext 真实键
// 管线——验收④「断言可见结果（提示/不派发），非仅不 panic」）。
//
// 宿主：WiringHost 模式同 pdf_search_wiring_tests（ShellApp 必须进渲染
// 树——Ctrl+F/Ctrl+S 走 shell-root on_key_down 真实派发路径）。
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// #385 批 4：菜单栏状态桥交互测试（TestAppContext 无头面——对照表
// §六 质量门 1：「点击 → 状态变化」级，非仅函数返回值断言）。
//
// 宿主：ShellHost 模式同 tab_workflow_tests（ShellApp 不作根视图——
// render 路径与本测试无关，空 div 宿主开窗只取真实 Window）。
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// #385 批 1×4 收口：右键菜单状态桥交互测试（TestAppContext 无头面——
// 对照表 §六 质量门 1：「手势 → 状态变化」级）。右键菜单面此前只有
// 数据层单测（context_menu.rs）+ wiring 静态锚，此处补运行时面：
// 开（实体/位置/来源/目标四态）→ 切换（重建实体）→ 关（全清）→
// 组件 Cancel → DismissEvent → shell 清态链 → 执行动作先行清菜单态。
// ---------------------------------------------------------------------------

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
// #585 残余①（文案）：钮文案 = 状态的 1:1 纯函数
// （`status_info::pdf_toggle_label` / `xlsx_toggle_label`——渲染消费
// 同一函数）。三面闭环：映射穷尽单测（status_info，CI 可见）+
// 接线源码锚点（ui_wiring_faces face 5，CI 可见）+ 本模块真实点击
// 翻转后断言纯函数输出随翻（运行时联动）。像素级文本呈现归 #554。
//
// #585 残余②（零占位）：结构性证据 = ui_wiring_faces face 5 锚点
//（钮构造整体位于 `.when(is_pdf/is_xlsx)` 臂内且全文件无臂外构造点
// ——GPUI `when(false, f)` 不应用 f，元素/命中盒/占位不存在）；运行
// 时证据 = 本模块 md 态全扫零翻转（用户可见行为）。像素级截图归 #554。
//
// #585 残余③（PDFium 真跑）：开档成功即打印「PDFIUM 真跑（非
// SKIP）」横幅——CI/本地输出可辨真跑与跳过；无库环境保持 SKIP 横幅
// 不伪装通过（AGENTS 13.1）。
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// #385 批 10（#582）：settings 模态与面板开合族生命周期交互测试
//（§六 质量门 1 扩面 Ⅱ——五面齐〔menu_bar/context_menu/palette/goto/
// tab〕后的遗留面）。宿主与派发模式同 menu_bar_lifecycle_tests /
// tab_workflow_tests：shell_window 宿主 + 真实 PlatformInput::KeyDown
// 派发管线（draw 一帧 → dispatch_event，键事件按焦点路径进
// shell-root on_key_down 监听链）。
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// #385 批 12（#590）：确认框模态交互测试——F4 拦截分支的键盘语义面
//（§六 质量门 1 扩面 Ⅲ；stacked on 批 10 的 settings_lifecycle_tests
// ——宿主/派发模式同款）。批 10 已锁 Esc 分派优先序；本模块锁确认框
// 自身：Tab/←/→ 选择循环（三钮 + 二钮形）+ Enter 取消路径（no-op
// 零副作用）+ Esc 单独在场的完整关闭链（焦点回落 root）。
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "workspace_layout_tests.rs"]
mod workspace_layout_tests;

/// #107 P1② 测试（内联域）：拖拽移动全链（rename + 前缀重写 + 守卫）。
#[cfg(test)]
mod drag_move_tests {

    fn temp_ws(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mang-move-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.md"), "# A\n").unwrap();
        std::fs::write(dir.join("sub").join("b.md"), "# B\n").unwrap();
        dir
    }

    /// 文件移动：rename + 打开标签路径精确重写 + toast。
    #[gpui::test]
    fn move_file_rewrites_open_tab_path(cx: &mut gpui::TestAppContext) {
        let ws = temp_ws("file");
        let (shell, mut visual) = super::workspace_layout_tests::host(cx);
        let a = ws.join("a.md");
        let shell2 = shell.clone();
        let p = a.clone();
        visual.update(|_window, app| {
            shell2.update(app, |shell, cx| {
                let _ = shell.open_document_tab(p, cx);
            });
        });
        // 直测 move_path_confirmed（confirm 面另测）。
        let new = ws.join("sub").join("a.md");
        let shell3 = shell.clone();
        let (o, n) = (a.clone(), new.clone());
        visual.update(|_window, app| {
            shell3.update(app, |shell, cx| {
                shell.move_path_confirmed(&o, &n, cx);
            });
        });
        assert!(!a.exists(), "旧路径消失（已 rename）");
        assert!(new.exists(), "新路径在场");
        shell.read_with(&visual, |shell, _| {
            let idx = shell.tabs.iter().position(|t| t.path == new).unwrap();
            assert_eq!(shell.tabs[idx].path, new, "标签路径重写");
        });
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// 目录移动：其下打开标签**前缀重写**（D2 语义）。
    #[gpui::test]
    fn move_dir_rewrites_tab_path_prefixes(cx: &mut gpui::TestAppContext) {
        let ws = temp_ws("dir");
        let (shell, mut visual) = super::workspace_layout_tests::host(cx);
        let b = ws.join("sub").join("b.md");
        let shell2 = shell.clone();
        let p = b.clone();
        visual.update(|_window, app| {
            shell2.update(app, |shell, cx| {
                let _ = shell.open_document_tab(p, cx);
            });
        });
        // 移动 sub → sub2（其下 b.md 标签路径前缀重写）。
        let old = ws.join("sub");
        let new = ws.join("sub2");
        let shell3 = shell.clone();
        let (o, n) = (old.clone(), new.clone());
        visual.update(|_window, app| {
            shell3.update(app, |shell, cx| {
                shell.move_path_confirmed(&o, &n, cx);
            });
        });
        assert!(new.join("b.md").exists(), "目录内容随迁");
        shell.read_with(&visual, |shell, _| {
            assert!(
                shell.tabs.iter().any(|t| t.path == new.join("b.md")),
                "前缀重写：标签路径 = sub2/b.md（实得 {:?}）",
                shell
                    .tabs
                    .iter()
                    .map(|t| t.path.clone())
                    .collect::<Vec<_>>()
            );
        });
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// confirm 守卫：目标已存在拒（不弹确认不 rename）。
    #[gpui::test]
    fn move_guard_rejects_existing_target(cx: &mut gpui::TestAppContext) {
        let ws = temp_ws("guard");
        let (shell, mut visual) = super::workspace_layout_tests::host(cx);
        let a = ws.join("a.md");
        let new = ws.join("sub"); // 已存在目录。
        let shell2 = shell.clone();
        let (o, n) = (a.clone(), new.clone());
        visual.update(|window, app| {
            shell2.update(app, |shell, cx| {
                shell.confirm_move_path(o, n, window, cx);
            });
        });
        shell.read_with(&visual, |shell, _| {
            assert!(shell.confirm.is_none(), "守卫拒——不弹确认");
            assert!(a.exists(), "未 rename（原位保持）");
        });
        let _ = std::fs::remove_dir_all(&ws);
    }
}

// ---------------------------------------------------------------------------
// #604：XLSX 统一保存与退出脏检查（#389 阶段 2 收口）——Shell 分流面。
//
// 与既有面的分工（不重复钉定）：
// - `xlsx_grid_view` 视图级测试（#584/#500）：网格编辑/合并格/视图自身
//   Ctrl+S（持焦直通 view.save）；
// - 本模块：**Shell 收口面**——菜单/命令面板保存（dispatch_editor_cmd
//   → save_tab）、关闭确认「保存」（resolve_confirm → save_tab）、退出
//   确认（on_window_should_close 脏收集 + Quit 保存）、键位 Ctrl+S
//   （ShellAction::Save → save_tab，焦点不在网格时）与失败拒关。
//
// 复现登记（修复前，main 7deddcd）：
// - save_tab 无 xlsx 分支 → 文本面只读 Editor::save 恒 Unmodified（假
//   成功）——关闭确认「保存」带着未落盘的网格编辑直接关标签；
// - dispatch_editor_cmd 的 xlsx 判定在 Cmd::Save 首臂 return 之后（死
//   代码）——菜单保存从未走到会话路径；
// - on_window_should_close 脏收集散写两谓词漏 xlsx_dirty_at——退出
//   关闭静默丢会话编辑（确认框都不弹）。
//
// 交互纪律：编辑置脏走**真实事件**（双击命中网格体 + 真实键入 + Enter
// 提交——坐标不硬编码：候选点扫描，网格持焦才继续键入，脏标记为命中
// 判据；#584 真实事件 e2e 同构）。保存/关闭/退出走生产入口（菜单面
// dispatch_editor_cmd、真实 Ctrl+W 键位、simulate_close 真实回调链）。
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// #47 A5 剩余缺口：状态栏真实文案**运行时证据**（statusbar_labels_runtime_tests）
//
// 分工（与既有面的关系）：
// - ui_wiring_faces face5（静态锚点）：锁「status_bar 渲染真的消费
//   status_info 数据源」——源码级 contains 断言，**非运行时证据**；
// - statusbar_view_toggle_tests（#585）：右侧切换钮的真实点击行为；
// - 本模块（A5 收口）：**文案计算面的运行时证据**——真 md 文件（UTF-8
//   + CRLF + 中文正文）+ 真 .git/HEAD（find_git_root/git_branch_at 的
//   实际解析对象）+ 编辑器真实状态（光标/选区/脏位），断言
//   `status_bar::status_bar_labels`（渲染的单一事实源——渲染零逻辑）
//   各段产出真实文案。row_labels 先例同构：内容证据不依赖像素断言
//   （像素级文本呈现归 #554）。
// ---------------------------------------------------------------------------
