//! 工作区文件树（#68 阶段 F2）——真实磁盘目录的懒加载树。
//!
//! 分层：
//! - [`TreeState`] 纯状态机（根/展开集/清单缓存/活动文件——无 gpui
//!   依赖，单测直接钉定）；
//! - [`FileTree`] gpui 实体（渲染 + 交互；宿主 [`ShellApp`]）。
//!
//! 设计要点（Lead 派单口径）：
//! - **懒加载**：目录清单只在首次展开时读取（`listings` 缓存）——
//!   打开大工作区只读根目录一层；
//! - **虚拟化**：可见行扁平化（[`TreeState::visible_rows`]）后经
//!   `uniform_list` 渲染——1000+ 文件目录只布局可视行，不卡；
//! - **排序**：目录优先 + 名称大小写不敏感字典序（[`entry_order`]）；
//! - **忽略**：隐藏项（`.` 前缀）与 `node_modules` / `target`
//!   （[`is_ignored_name`]；可配置面随 F6 设置件落位）；
//! - **符号链接不跟随**（`DirEntry::metadata` 不跟随面按文件呈现）：
//!   防循环展开（a → 父目录会在 DFS 中无限递归）；另加深度上限
//!   双保险。
//!
//! 交互：单击目录 = 展开/折叠；单击文件 = 经 `ShellApp::open_document`
//! 打开（F1 通道复用）；右键 = 上下文菜单（#166 接线——新建/打开/
//! 重命名/删除/在资源管理器中显示；菜单本体在 `context_menu.rs`，动作
//! 派发在 `shell.rs`）。
//!
//! #575（批 8）：**键盘导航**——↑↓ 键盘光标（可见行序，两端夹取 +
//! `scroll_to_item` 滚动跟随）、Enter 激活（目录 = 展开折叠 / 文件 =
//! 打开，与点击同链）、Esc 焦点还编辑区；键位走 main.rs 既有 menu
//! 键位族（#571 批 7 同源）。与批 7 的 ui::Navigable 偏离登记：其
//! 滚动锚点 ScrollAnchor::for_handle 只吃常规 ScrollHandle，本面
//! uniform_list 虚拟化（UniformListScrollHandle 不同型）——改用
//! **键盘光标模型**（单一 selected 序 + 虚拟化安全，无离屏句柄问题）。
//! ShellApp 耦合经 [`FileTreeHooks`] 钩子注入（本模块 lib 化——
//! 测试 CI 可见，outline_panel #571 先例二次应用）。
//!
//! #595 光标路径身份：所有行集变更（reload / forget_subtree /
//! expand_to / 目录 toggle——键与点击）经 `mutate_rows_keep_cursor`
//! 重绑定——序号只是显示位，**路径才是光标身份**（刷新增删行后
//! Enter 打开的仍是当初选中的文件；目标不可见则清空光标）。
//!
//! #385 批 5b：行渲染迁 `ui::ListItem`（zed ui 组件族——缩进/hover/
//! 选中/左右键接线经组件语义，非手写仿制）。**uniform_list 虚拟化保
//! 留**（ui::List 为非虚拟化容器，「1000+ 文件只布局可视行」不变量
//! 不动）；TreeViewItem 经评估不适用（非根行仅单级缩进线 + 无图标
//! 位——深度嵌套可读性回归），偏离登记在
//! `docs/ui-component-migration.md` 批 5b 落地记录。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Context, FocusHandle, IntoElement, MouseDownEvent, Pixels, Point, Render, Role,
    ScrollStrategy, UniformListScrollHandle, Window, px, uniform_list,
};
use ui::prelude::*;
use ui::{Icon, IconName, IconSize, Label, LabelSize, ListItem};

/// 文件打开钩子（bin 面 ShellApp::open_document_tab + 焦点落位注入）。
pub type OpenFileHook = Arc<dyn Fn(PathBuf, &mut Window, &mut App)>;

/// 右键菜单钩子（bin 面 ShellApp::open_file_tree_context_menu 注入；
/// 入参 = 按下点窗口坐标 / 目标路径 / 是否目录）。
pub type ContextMenuHook = Arc<dyn Fn(Point<Pixels>, PathBuf, bool, &mut Window, &mut App)>;

/// Esc 焦点归还钩子（bin 面：焦点还激活编辑器——#571 同语义）。
pub type EscapeHook = Arc<dyn Fn(&mut Window, &mut App)>;

/// ShellApp 面收口（#575 lib 化：本模块不引 shell——依赖倒置，
/// outline_panel 先例）。bin 侧构造闭包捕获 WeakEntity<ShellApp>。
pub struct FileTreeHooks {
    pub open_file: OpenFileHook,
    pub context_menu: ContextMenuHook,
    pub escape: EscapeHook,
}

/// 目录条目。
#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
}

/// 忽略名单（可配置面随 F6 设置件；当前为常量口径——`.git` 已被
/// `.` 前缀规则覆盖，列出以自文档）。
pub fn is_ignored_name(name: &str) -> bool {
    name.starts_with('.') || matches!(name, "node_modules" | "target")
}

/// 排序：目录优先 → 名称大小写不敏感字典序 → 原文序（大小写仅歧义时）。
pub fn entry_order(a: &FileEntry, b: &FileEntry) -> std::cmp::Ordering {
    b.is_dir
        .cmp(&a.is_dir)
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then_with(|| a.name.cmp(&b.name))
}

/// 读取目录一层（忽略规则过滤 + 排序；读失败返回空——权限/消失容错，
/// 空态即「该目录不可读」，不崩溃）。
pub fn list_dir(dir: &Path) -> Vec<FileEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<FileEntry> = entries
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|n| !is_ignored_name(n))
                .unwrap_or(false)
        })
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            // DirEntry::metadata 不跟随符号链接——链接按文件呈现
            //（防循环展开；见模块注释）。
            let is_dir = e.metadata().map(|m| m.is_dir()).unwrap_or(false);
            FileEntry {
                path: e.path(),
                name,
                is_dir,
            }
        })
        .collect();
    out.sort_by(entry_order);
    out
}

/// 扁平化后的可见行（uniform_list 渲染单元）。
#[derive(Clone, Debug)]
pub struct TreeRow {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
    pub is_active: bool,
}

/// 深度上限（DFS 双保险——符号链接/异常环防御）。
const MAX_DEPTH: usize = 24;

/// 文件树纯状态机（单测面——无 gpui 依赖）。
pub struct TreeState {
    root: PathBuf,
    expanded: HashSet<PathBuf>,
    listings: HashMap<PathBuf, Rc<Vec<FileEntry>>>,
    active_file: Option<PathBuf>,
}

impl TreeState {
    /// 构造：根目录展开态 + 根层清单即读（打开工作区的首次 IO）。
    pub fn new(root: PathBuf) -> Self {
        let mut this = Self {
            root,
            expanded: HashSet::new(),
            listings: HashMap::new(),
            active_file: None,
        };
        this.expanded.insert(this.root.clone());
        this.ensure_listing(&this.root.clone());
        this
    }

    /// 工作区根。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 清单缓存命中查询（懒加载语义测试面）。
    pub fn listing_loaded(&self, dir: &Path) -> bool {
        self.listings.contains_key(dir)
    }

    /// 当前打开文件高亮（ShellApp::open_document 后同步）。
    pub fn set_active_file(&mut self, path: Option<PathBuf>) {
        self.active_file = path;
    }

    /// 展开/折叠目录（展开时懒加载清单）。
    pub fn toggle(&mut self, dir: &Path) {
        if self.expanded.contains(dir) {
            self.expanded.remove(dir);
        } else {
            self.ensure_listing(dir);
            self.expanded.insert(dir.to_path_buf());
        }
    }

    /// #166：fs 变更后重读目录清单（右键新建/重命名/删除后同步）。
    /// 目录未缓存则无操作——未展开过的目录本就无清单，展开时现读。
    pub fn reload(&mut self, dir: &Path) {
        if self.listings.contains_key(dir) {
            self.listings
                .insert(dir.to_path_buf(), Rc::new(list_dir(dir)));
        }
    }

    /// #166：丢弃已删除目录的展开态与清单缓存（连同其下所有已缓存
    /// 子路径——前缀匹配）。文件路径调用无副作用（无展开态/清单命中）。
    pub fn forget_subtree(&mut self, dir: &Path) {
        self.expanded.retain(|p| !p.starts_with(dir));
        self.listings.retain(|p, _| !p.starts_with(dir));
    }

    /// #166：确保文件的各级父目录（工作区内）全部展开 + 清单就绪
    /// （新建/重命名后让目标行可见——折叠着的父目录里新文件看不见）。
    pub fn expand_to(&mut self, file: &Path) {
        let mut ancestors = Vec::new();
        let mut cur = file.parent();
        while let Some(dir) = cur {
            ancestors.push(dir.to_path_buf());
            if dir == self.root {
                break;
            }
            cur = dir.parent();
        }
        for dir in ancestors.into_iter().rev() {
            if dir.starts_with(&self.root) {
                self.ensure_listing(&dir);
                self.expanded.insert(dir);
            }
        }
    }

    fn ensure_listing(&mut self, dir: &Path) {
        if !self.listings.contains_key(dir) {
            self.listings
                .insert(dir.to_path_buf(), Rc::new(list_dir(dir)));
        }
    }

    /// 可见行扁平化（DFS：只进展开目录；行序 = 渲染序）。
    pub fn visible_rows(&self) -> Vec<TreeRow> {
        let mut rows = Vec::new();
        let root_expanded = self.expanded.contains(&self.root);
        rows.push(TreeRow {
            path: self.root.clone(),
            name: self
                .root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.root.display().to_string()),
            depth: 0,
            is_dir: true,
            expanded: root_expanded,
            is_active: false,
        });
        if root_expanded {
            let root = self.root.clone();
            self.push_children(&root, 1, &mut rows);
        }
        rows
    }

    fn push_children(&self, dir: &Path, depth: usize, rows: &mut Vec<TreeRow>) {
        if depth > MAX_DEPTH {
            return;
        }
        let Some(entries) = self.listings.get(dir) else {
            return;
        };
        for entry in entries.iter() {
            let expanded = entry.is_dir && self.expanded.contains(&entry.path);
            rows.push(TreeRow {
                path: entry.path.clone(),
                name: entry.name.clone(),
                depth,
                is_dir: entry.is_dir,
                expanded,
                is_active: self.active_file.as_deref() == Some(entry.path.as_path()),
            });
            if expanded {
                let path = entry.path.clone();
                self.push_children(&path, depth + 1, rows);
            }
        }
    }
}

/// 工作区文件树实体（左栏呈现；宿主 [`ShellApp`]——文件点击经其
/// `open_document` 打开）。
pub struct FileTree {
    state: TreeState,
    /// #575：bin 面钩子（open_file / context_menu / escape）。
    hooks: FileTreeHooks,
    /// #575：键盘光标（可见行序；None = 无光标）。
    selected: Option<usize>,
    /// #575：虚拟化滚动句柄（光标行滚动跟随）。
    scroll: UniformListScrollHandle,
    /// #575：树根焦点句柄（键位域挂载点——menu 上下文）。
    tree_focus: FocusHandle,
}

impl FileTree {
    pub fn new(root: PathBuf, hooks: FileTreeHooks, cx: &mut Context<Self>) -> Self {
        Self {
            state: TreeState::new(root),
            hooks,
            selected: None,
            scroll: UniformListScrollHandle::new(),
            tree_focus: cx.focus_handle(),
        }
    }

    /// #575：键盘光标移动（delta = ±1；两端夹取；无光标时 ↓=首行 /
    /// ↑=末行）。行集空 = 光标清空。滚动跟随 Nearest（已可视不动）。
    fn move_cursor(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.state.visible_rows().len();
        if len == 0 {
            self.selected = None;
            cx.notify();
            return;
        }
        let next = match self.selected {
            Some(i) => (i as isize + delta).clamp(0, len as isize - 1) as usize,
            None if delta > 0 => 0,
            None => len - 1,
        };
        if self.selected != Some(next) {
            self.scroll.scroll_to_item(next, ScrollStrategy::Nearest);
        }
        self.selected = Some(next);
        cx.notify();
    }

    /// #595：行集变更的光标重绑定——**路径身份优先**，防刷新后漂移误开。
    ///
    /// 变更前记下光标行的真实路径；变更后按路径在新可见行集中重定位
    ///（同一文件仍是光标——Enter 打开的就是当初选中的那个）。路径
    /// 消失/不可见（删除、折叠祖先、子树遗忘）→ 清空光标（None
    /// 语义保持，Enter 真空操作不误开其它文件）。序号位移时滚动跟随
    /// Nearest（复用 move_cursor 同策略——可视行不动，离屏行就近带回）。
    /// 无光标时零副作用（None 不被凭空点亮）。
    fn mutate_rows_keep_cursor(&mut self, mutate: impl FnOnce(&mut TreeState)) {
        let selected_path = self.selected.and_then(|ix| {
            let rows = self.state.visible_rows();
            rows.get(ix).map(|row| row.path.clone())
        });
        mutate(&mut self.state);
        self.selected = selected_path.and_then(|path| {
            let rows = self.state.visible_rows();
            rows.iter().position(|row| row.path == path)
        });
        if let Some(ix) = self.selected {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
    }

    /// #575：Enter 激活光标行（与点击同链）：目录 = 展开/折叠；
    /// 文件 = open_file 钩子（bin 面含焦点落位）。#595：目录 toggle
    /// 也经路径重绑定（光标留在被翻转的目录行，语义与身份都不漂）。
    fn activate_cursor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else { return };
        let Some(row) = self.state.visible_rows().get(ix).cloned() else {
            return;
        };
        if row.is_dir {
            let path = row.path.clone();
            self.mutate_rows_keep_cursor(|state| state.toggle(&path));
            cx.notify();
        } else {
            let hook = self.hooks.open_file.clone();
            // Opening a tab writes the active path back to this tree. Wait
            // until this listener has returned the borrowed entity to App.
            window.defer(cx, move |window, cx| hook(row.path, window, cx));
        }
    }

    /// 工作区根（状态栏显示面）。
    pub fn root(&self) -> &Path {
        self.state.root()
    }

    /// 当前打开文件高亮同步（ShellApp::open_document 后调用）。
    pub fn set_active_file(&mut self, path: Option<PathBuf>) {
        self.state.set_active_file(path);
    }

    /// #166：fs 变更后重读目录清单（转发到状态机）。#595：经路径
    /// 重绑定——刷新插入/删除行后光标仍是原文件，目标不可见则清空。
    pub fn reload(&mut self, dir: &Path) {
        self.mutate_rows_keep_cursor(|state| state.reload(dir));
    }

    /// #166：丢弃已删除目录的展开态与缓存（转发到状态机）。#595：经
    /// 路径重绑定——遗忘子树后光标下的行消失则清空，不误指其它路径。
    pub fn forget_subtree(&mut self, dir: &Path) {
        self.mutate_rows_keep_cursor(|state| state.forget_subtree(dir));
    }

    /// #166：展开到文件（转发到状态机）。#595：经路径重绑定——展开
    /// 增行位移后光标路径身份保持。
    pub fn expand_to(&mut self, file: &Path) {
        self.mutate_rows_keep_cursor(|state| state.expand_to(file));
    }
}

impl Render for FileTree {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row_count = self.state.visible_rows().len();
        let weak = cx.entity().downgrade();
        let scroll = self.scroll.clone();
        let tree_focus = self.tree_focus.clone();
        // #575：键位域根（menu 上下文 = #571 批 7 同源——↑↓/Enter/Esc
        // 走 main.rs 既有绑定；焦点经行点击落入树根句柄）。
        div()
            .id("file-tree")
            .key_context("menu")
            .track_focus(&tree_focus)
            .on_action(cx.listener(|this, _: &menu::SelectNext, _, cx| {
                this.move_cursor(1, cx);
            }))
            .on_action(cx.listener(|this, _: &menu::SelectPrevious, _, cx| {
                this.move_cursor(-1, cx);
            }))
            .on_action(cx.listener(|this, _: &menu::Confirm, window, cx| {
                this.activate_cursor(window, cx);
            }))
            .on_action(cx.listener(|this, _: &menu::Cancel, window, cx| {
                let hook = this.hooks.escape.clone();
                hook(window, cx);
            }))
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                uniform_list(
                    "file-tree-rows",
                    row_count,
                    move |visible: std::ops::Range<usize>,
                          window: &mut gpui::Window,
                          cx: &mut App| {
                        weak.update(cx, |this, cx| this.render_rows(visible, window, cx))
                            .unwrap_or_default()
                    },
                )
                .track_scroll(&scroll)
                .flex_1()
                .min_h_0(),
            )
    }
}

impl FileTree {
    fn render_rows(
        &mut self,
        range: std::ops::Range<usize>,
        _window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let rows = self.state.visible_rows();
        // #575：行序携带（光标视觉 + 点击设光标）。
        let selected = self.selected;
        range
            .filter_map(|i| rows.get(i).map(|row| self.render_row(row, i, selected, cx)))
            .collect()
    }

    fn render_row(
        &self,
        row: &TreeRow,
        index: usize,
        selected: Option<usize>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let is_dir = row.is_dir;
        let path = row.path.clone();

        // 折叠箭头 + 类型图标：目录两态动态（开/闭），文件静态右指箭头
        // （行首对齐占位——与迁移前语系一致）。#385 批 5b：箭头落
        // start_slot、类型图标与标签落 children（ListItem 内容槽）。
        let (chevron, icon) = if is_dir {
            (
                if row.expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                },
                if row.expanded {
                    IconName::FolderOpen
                } else {
                    IconName::Folder
                },
            )
        } else {
            (IconName::ChevronRight, IconName::File)
        };

        // #166：右键目标（目录本身 / 文件所在目录）与菜单位置随行携带。
        let path_for_menu = row.path.clone();

        // #385 批 5b：手写行（h_flex + 手动 pl/hover/选中/id）迁
        // ui::ListItem——非 inset 模式（缩进画行内、hover/选中底色通栏，
        // 与迁移前语系一致）；深度缩进经组件 indent_level（14px 步进 =
        // 旧 depth*14 口径）；行高钉 22px。左/右键经组件 on_click /
        // on_secondary_mouse_down（MouseButton 枚举在组件内部接线）。
        ListItem::new(gpui::SharedString::from(format!(
            "ft-{}-{}",
            row.depth,
            row.path.display()
        )))
        .indent_level(row.depth)
        .indent_step_size(px(14.))
        .height(px(22.))
        // 无障碍升级：树行语义（Role::TreeItem + 行名播报；选中态随
        // 组件 selected → aria_selected 自动播报）。
        .aria_role(Role::TreeItem)
        .aria_label(row.name.as_str())
        // 左键：目录展开/折叠；文件经 F1 通道打开（ListItem.on_click——
        // 组件内 MouseButton::Left 接线；shell 跨实体经 Entity::update
        // 进入，焦点句柄返回后在监听器 window 域落焦——update_in 在派生
        // 上下文解析不到窗口，F7 实测）。
        .on_click(cx.listener(move |this, _, window, cx| {
            // #575：点击设光标 + 焦点落树根（键位域生效——单一模型）。
            this.selected = Some(index);
            let tree_focus = this.tree_focus.clone();
            tree_focus.focus(window, cx);
            if is_dir {
                // #595：目录 toggle 也走路径重绑定（被点目录行即新光标
                // 身份——统一不变量：所有行集变更经重绑定）。
                this.mutate_rows_keep_cursor(|state| state.toggle(&path));
                cx.notify();
            } else {
                // #575：ShellApp 面 → open_file 钩子（bin 注入——lib 化）。
                let hook = this.hooks.open_file.clone();
                let path = path.clone();
                window.defer(cx, move |window, cx| hook(path, window, cx));
            }
        }))
        // 右键：上下文菜单（#166 接线保持——菜单在 shell 状态上，位置取
        // 按下点坐标；动作派发回 shell 的 activate_context_menu_item。
        // ListItem.on_secondary_mouse_down——组件内 MouseButton::Right）。
        .on_secondary_mouse_down(
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                // #575：ShellApp 面 → context_menu 钩子（坐标转换收 bin）。
                let hook = this.hooks.context_menu.clone();
                hook(event.position, path_for_menu.clone(), is_dir, window, cx);
            }),
        )
        .start_slot(
            Icon::new(chevron)
                .size(IconSize::XSmall)
                .color(ui::Color::Muted),
        )
        .child(
            Icon::new(icon)
                .size(IconSize::XSmall)
                .color(ui::Color::Muted),
        )
        .child(
            Label::new(row.name.as_str())
                .size(LabelSize::Small)
                .when(row.is_active, |l| l.color(ui::Color::Accent)),
        )
        // 当前打开文件高亮 + #575 键盘光标行（组件 selected 语系——
        // ghost_element_selected 通栏底色 + 上方 Label::Accent 文字色；
        // 两态同面：光标与激活文件共用选中底，Label::Accent 区分后者）。
        .toggle_state(row.is_active || selected == Some(index))
        .into_any_element()
    }
}

/// 状态栏工作区显示文案：`~` 缩写家目录；超长截前保尾（尾部含文件名
/// 更可辨）。纯函数（单测钉定）。
pub fn workspace_display(root: &Path) -> String {
    let full = root.display().to_string();
    let shortened = match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && full.starts_with(&home) => {
            let rest = &full[home.len()..];
            if rest.is_empty() {
                "~".to_string()
            } else {
                format!("~{rest}")
            }
        }
        _ => full.clone(),
    };
    const MAX_LEN: usize = 56;
    let count = shortened.chars().count();
    if count > MAX_LEN {
        let tail: String = shortened.chars().skip(count - MAX_LEN + 1).collect();
        format!("…{tail}")
    } else {
        shortened
    }
}

// ---------------------------------------------------------------------------
// 纯逻辑单测（临时目录夹具——真实 fs 面；无 gpui 上下文）。
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn temp_fixture(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mang-ft-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub").join("deep")).unwrap();
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("b.md"), "b").unwrap();
        std::fs::write(dir.join("A.md"), "a").unwrap();
        std::fs::write(dir.join("sub").join("c.md"), "c").unwrap();
        std::fs::write(dir.join("sub").join("deep").join("d.md"), "d").unwrap();
        dir
    }

    #[test]
    fn ignored_names_cover_hidden_and_build_dirs() {
        assert!(is_ignored_name(".git"));
        assert!(is_ignored_name(".hidden"));
        assert!(is_ignored_name("node_modules"));
        assert!(is_ignored_name("target"));
        assert!(!is_ignored_name("src"));
        assert!(!is_ignored_name("main.md"));
        assert!(!is_ignored_name("target_.md"), "整名匹配（前缀不算）");
    }

    #[test]
    fn list_dir_sorts_dirs_first_ci_alpha() {
        let dir = temp_fixture("sort");
        let names: Vec<String> = list_dir(&dir).iter().map(|e| e.name.clone()).collect();
        assert_eq!(
            names,
            vec!["sub".to_string(), "A.md".to_string(), "b.md".to_string()],
            "目录优先 + 大小写不敏感字典序"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_dir_filters_ignored() {
        let dir = temp_fixture("filter");
        let names: Vec<String> = list_dir(&dir).iter().map(|e| e.name.clone()).collect();
        assert!(
            !names.contains(&"node_modules".to_string()),
            "node_modules 过滤"
        );
        assert!(
            !names.iter().any(|n| n.starts_with('.')),
            "隐藏项（.git 等）过滤"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn visible_rows_flatten_lazily() {
        let dir = temp_fixture("flatten");
        let mut tree = TreeState::new(dir.clone());
        // 初始：仅根展开 → 根 + 一层子项（sub/A.md/b.md）。
        let rows = tree.visible_rows();
        assert_eq!(rows.len(), 4, "根 + sub + A.md + b.md");
        assert!(rows[0].is_dir && rows[0].depth == 0, "根行");
        assert_eq!(rows[1].name, "sub", "目录序首");
        assert_eq!(rows[1].depth, 1);

        // 懒加载：sub 未展开前其清单未读取。
        assert!(tree.listing_loaded(&dir), "根清单已缓存");
        assert!(
            !tree.listing_loaded(&dir.join("sub")),
            "sub 清单未加载（懒加载语义）"
        );

        // 展开 sub → c.md + deep 出现，深度 +1。
        tree.toggle(&dir.join("sub"));
        let rows = tree.visible_rows();
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"c.md"), "sub 子项出现：{names:?}");
        assert!(names.contains(&"deep"), "子目录出现：{names:?}");
        let deep_row = rows.iter().find(|r| r.name == "deep").unwrap();
        assert_eq!(deep_row.depth, 2, "深度递增");
        assert!(tree.listing_loaded(&dir.join("sub")), "展开触发清单加载");

        // 再折叠 sub → 深层行消失（清单缓存保留——再展开零 IO）。
        tree.toggle(&dir.join("sub"));
        assert_eq!(tree.visible_rows().len(), 4, "折叠恢复");
        assert!(tree.listing_loaded(&dir.join("sub")), "缓存保留");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn active_file_marks_exactly_one_row() {
        let dir = temp_fixture("active");
        let mut tree = TreeState::new(dir.clone());
        tree.set_active_file(Some(dir.join("A.md")));
        let rows = tree.visible_rows();
        let active: Vec<&TreeRow> = rows.iter().filter(|r| r.is_active).collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].name, "A.md");
        // 清除高亮。
        tree.set_active_file(None);
        assert!(tree.visible_rows().iter().all(|r| !r.is_active));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn workspace_display_abbreviates_and_truncates() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/tester".into());
        let display = workspace_display(Path::new(&format!("{home}/proj/mdcore")));
        assert!(
            display.starts_with('~') || display.contains("proj/mdcore"),
            "家目录缩写或原样保留：{display}"
        );
        let long = format!("{home}/{}", "very-long-dir-name/".repeat(12));
        let display = workspace_display(Path::new(&long));
        assert!(display.starts_with('…'), "超长截断：{display}");
        assert!(
            display.ends_with("very-long-dir-name/"),
            "截前保尾：{display}"
        );
        assert!(display.chars().count() <= 57, "长度受控：{display}");
    }

    // -----------------------------------------------------------------------
    // #166：fs 变更同步面（右键新建/重命名/删除后的树状态维护）。
    // -----------------------------------------------------------------------

    #[test]
    fn reload_picks_up_created_file() {
        let dir = temp_fixture("reload");
        let mut tree = TreeState::new(dir.clone());
        assert!(!list_dir_names(&tree, &dir).contains(&"n.md".to_string()));
        std::fs::write(dir.join("n.md"), "n").unwrap();
        // reload 前：缓存清单还是旧的（懒加载语义——磁盘变了树不知道）。
        assert!(!list_dir_names(&tree, &dir).contains(&"n.md".to_string()));
        tree.reload(&dir);
        assert!(
            list_dir_names(&tree, &dir).contains(&"n.md".to_string()),
            "reload 后新文件可见"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_on_uncached_dir_is_noop() {
        let dir = temp_fixture("reload-nocache");
        let mut tree = TreeState::new(dir.clone());
        let sub = dir.join("sub");
        assert!(!tree.listing_loaded(&sub));
        // 未展开目录无缓存——reload 不应凭空建缓存（展开时才现读）。
        tree.reload(&sub);
        assert!(!tree.listing_loaded(&sub), "无缓存目录 reload 零副作用");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn forget_subtree_drops_expansion_and_cache() {
        let dir = temp_fixture("forget");
        let mut tree = TreeState::new(dir.clone());
        let sub = dir.join("sub");
        tree.toggle(&sub); // 展开 sub → 清单缓存 + deep 可见
        assert!(tree.listing_loaded(&sub));
        assert!(
            visible_names(&tree).contains(&"deep".to_string()),
            "前置：sub 展开中"
        );
        tree.forget_subtree(&sub);
        assert!(!tree.listing_loaded(&sub), "sub 清单缓存丢弃");
        assert!(!tree.expanded.contains(&sub), "sub 展开态丢弃");
        assert!(
            !visible_names(&tree).contains(&"deep".to_string()),
            "子目录行随展开态消失"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn expand_to_reveals_file_in_collapsed_dir() {
        let dir = temp_fixture("expand-to");
        let mut tree = TreeState::new(dir.clone());
        let deep_file = dir.join("sub").join("deep").join("d.md");
        assert!(
            !visible_names(&tree).contains(&"d.md".to_string()),
            "前置：sub 折叠，深层文件不可见"
        );
        tree.expand_to(&deep_file);
        let names = visible_names(&tree);
        assert!(names.contains(&"d.md".to_string()), "各级父目录展开后可见");
        assert!(names.contains(&"deep".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn expand_to_ignores_paths_outside_root() {
        let dir = temp_fixture("expand-outside");
        let mut tree = TreeState::new(dir.clone());
        // 工作区外的路径：不得把工作区外的目录展开/缓存（前缀守卫）。
        tree.expand_to(Path::new("/elsewhere/x.md"));
        assert!(
            tree.visible_rows()
                .iter()
                .all(|r| r.depth == 0 || r.path.starts_with(&dir)),
            "工作区外路径零影响"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 当前可见行名集（断言辅助）。
    fn visible_names(tree: &TreeState) -> Vec<String> {
        tree.visible_rows().iter().map(|r| r.name.clone()).collect()
    }

    /// 目录当前缓存清单名集（断言辅助）。
    fn list_dir_names(tree: &TreeState, dir: &Path) -> Vec<String> {
        // 未缓存目录返回空（调用方仅对已缓存目录使用）。
        tree.visible_rows()
            .iter()
            .filter(|r| r.path.parent() == Some(dir))
            .map(|r| r.name.clone())
            .collect()
    }
}
// ---------------------------------------------------------------------------
// #575 键盘导航无头测试（lib 树——CI 可见；三面：光标移动夹取 /
// Enter 激活分叉 / 无光标起点语义）。
// ---------------------------------------------------------------------------
#[cfg(test)]
mod keyboard_tests {
    use super::tests::temp_fixture;
    use super::*;
    use gpui::Entity;
    use std::sync::Mutex;

    /// 测试钩子：open_file 捕获路径；其他入口触发即失败，防止测试漏接交互。
    fn test_hooks(opened: Arc<Mutex<Vec<PathBuf>>>) -> FileTreeHooks {
        FileTreeHooks {
            open_file: Arc::new(move |path, _, _| opened.lock().unwrap().push(path)),
            context_menu: Arc::new(|_, _, _, _, _| {
                panic!("unexpected context menu in keyboard test")
            }),
            escape: Arc::new(|_, _| panic!("unexpected escape in keyboard test")),
        }
    }

    /// 宿主 + 窗口（Host 模式——#571 同款）。
    struct Host(Entity<FileTree>);

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.0.clone())
        }
    }

    fn tree_window(
        cx: &mut gpui::TestAppContext,
        root: PathBuf,
        opened: Arc<Mutex<Vec<PathBuf>>>,
    ) -> (Entity<FileTree>, gpui::VisualTestContext) {
        // 主题初始化序列镜像 main.rs（ListItem/Icon 渲染依赖）。
        cx.update(|app| {
            theme::init(theme::LoadThemes::JustBase, app);
            theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
        });
        let tree = cx.update(|cx| cx.new(|cx| FileTree::new(root, test_hooks(opened), cx)));
        let window = cx.open_window(gpui::size(gpui::px(260.), gpui::px(400.)), {
            let tree = tree.clone();
            move |_, _| Host(tree)
        });
        cx.run_until_parked();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        (tree, visual)
    }

    /// 光标面：↓ 入场 = 首行；连续移动到末行；↑ 顶界夹取（不移出）。
    #[gpui::test]
    fn cursor_moves_and_clamps_at_both_ends(cx: &mut gpui::TestAppContext) {
        let root = temp_fixture("kb-move");
        let (tree, mut visual) = tree_window(cx, root, Arc::default());

        let len = tree.read_with(&visual, |t, _| t.state.visible_rows().len());
        assert!(len >= 3, "夹具行集 >= 3（实得 {len}）");

        // 无光标 ↓ → 首行。
        let focus = tree.read_with(&visual, |t, _| t.tree_focus.clone());
        visual.update(|window, cx| focus.focus(window, cx));
        visual.dispatch_action(menu::SelectNext);
        tree.read_with(&visual, |t, _| assert_eq!(t.selected, Some(0)));

        // 连续 ↓ 到末行（不越界）。
        for _ in 0..len {
            visual.dispatch_action(menu::SelectNext);
        }
        tree.read_with(&visual, |t, _| assert_eq!(t.selected, Some(len - 1)));

        // ↑ 在顶界夹取（len-1 → 0 后继续 ↑ 仍 0）。
        for _ in 0..len {
            visual.dispatch_action(menu::SelectPrevious);
        }
        tree.read_with(&visual, |t, _| assert_eq!(t.selected, Some(0)));
    }

    /// 激活面：Enter 目录 = 展开态翻转（可见行数变化）；Enter 文件 =
    /// open_file 钩子携路径触发。
    #[gpui::test]
    fn confirm_toggles_dirs_and_opens_files(cx: &mut gpui::TestAppContext) {
        let root = temp_fixture("kb-confirm");
        let opened: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
        let (tree, mut visual) = tree_window(cx, root.clone(), opened.clone());

        let (dir_ix, dir_path, file_path) = tree.read_with(&visual, |t, _| {
            let rows = t.state.visible_rows();
            // 行 0 = 根自身（默认展开态）——折叠根会清空全部子行，
            // 选**非根**子目录翻转（夹具 sub/ 在场；展开语义对称可测）。
            let dir = rows
                .iter()
                .enumerate()
                .find(|(_, r)| r.is_dir && r.path != t.state.root())
                .expect("夹具含非根目录");
            let file = rows
                .iter()
                .enumerate()
                .find(|(_, r)| !r.is_dir)
                .expect("夹具含文件");
            (dir.0, dir.1.path.clone(), file.1.path.clone())
        });
        let before = tree.read_with(&visual, |t, _| t.state.visible_rows().len());

        // 光标到目录行 → Enter → 展开翻转。
        let focus = tree.read_with(&visual, |t, _| t.tree_focus.clone());
        visual.update(|window, cx| focus.focus(window, cx));
        tree.update(&mut visual, |t, _| {
            t.selected = Some(dir_ix);
        });
        visual.dispatch_action(menu::Confirm);
        let after = tree.read_with(&visual, |t, _| t.state.visible_rows().len());
        assert_ne!(
            before, after,
            "Enter 目录 → 展开态翻转（可见行数 {before} → {after}）"
        );
        assert!(opened.lock().unwrap().is_empty(), "目录 Enter 不开文件");

        // 光标到文件行 → Enter → 钩子携路径触发（一次）。注意：目录
        // 展开后行集位移——文件行序在展开后重定位（不可用展开前序）。
        let file_path_after = tree.update(&mut visual, |t, _| {
            let rows = t.state.visible_rows();
            let (ix, row) = rows
                .iter()
                .enumerate()
                .find(|(_, r)| !r.is_dir && r.path == file_path)
                .expect("文件行在展开后行集中");
            t.selected = Some(ix);
            row.path.clone()
        });
        visual.dispatch_action(menu::Confirm);
        assert_eq!(
            opened.lock().unwrap().as_slice(),
            [file_path_after],
            "Enter 文件 → open_file 钩子携该路径触发"
        );
        let _ = (dir_ix, dir_path);
    }

    // -------------------------------------------------------------------
    // #595：刷新/行集变更后的光标路径身份（按路径重绑定，防误开）。
    // 全部走真实 GPUI 键路（SelectNext / Confirm 派发 + FileTreeHooks
    // 捕获）与真实临时目录——不测 index 补偿模拟器。
    // -------------------------------------------------------------------

    /// 光标行当前路径（断言辅助——经 selected 序号读真实行）。
    fn selected_path(tree: &Entity<FileTree>, visual: &gpui::VisualTestContext) -> Option<PathBuf> {
        tree.read_with(visual, |t, _| {
            let rows = t.state.visible_rows();
            t.selected
                .and_then(|ix| rows.get(ix))
                .map(|row| row.path.clone())
        })
    }

    /// 光标落位辅助：重置 None 后连续 SelectNext 直至目标路径
    /// （真实键路；从 None 起步，步数 = 目标行号 + 1，确定性落位）。
    fn focus_and_step_to(
        tree: &Entity<FileTree>,
        visual: &mut gpui::VisualTestContext,
        target: &Path,
    ) {
        tree.update(visual, |t, _| t.selected = None);
        let steps = tree.read_with(visual, |t, _| {
            t.state
                .visible_rows()
                .iter()
                .position(|r| r.path == target)
                .expect("目标行在可见行集中")
                + 1 // None 起：+1 次到达目标序号
        });
        let focus = tree.read_with(visual, |t, _| t.tree_focus.clone());
        visual.update(|window, cx| focus.focus(window, cx));
        for _ in 0..steps {
            visual.dispatch_action(menu::SelectNext);
        }
        tree.read_with(visual, |t, _| {
            let rows = t.state.visible_rows();
            let row = rows.get(t.selected.unwrap()).unwrap();
            assert_eq!(row.path, target, "键路落位：光标应在目标行");
        });
    }

    /// #595 验收 1：刷新插入排序更早的行后，Enter 打开的仍是当初
    /// 选中的文件（c.md），不是同序号的位移行（基线红：a.md）。
    #[gpui::test]
    fn reload_rebinds_cursor_by_path_not_index(cx: &mut gpui::TestAppContext) {
        let root = temp_fixture("kb-595-rebind");
        let sub = root.join("sub");
        let c_md = sub.join("c.md");
        let opened: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
        let (tree, mut visual) = tree_window(cx, root.clone(), opened.clone());

        // 真实键路：光标到 sub/c.md（行集 [root, sub, c.md, deep, A.md,
        // b.md]，c.md index 2——expand_to 模拟 shell 打开文件后的展开同步）。
        tree.update(&mut visual, |t, _| t.expand_to(&c_md));
        focus_and_step_to(&tree, &mut visual, &c_md);

        // fs 变更：sub 下新建 a.md（排序先于 c.md——行集前插一位）+ reload。
        std::fs::write(sub.join("a.md"), "a").unwrap();
        tree.update(&mut visual, |t, _| t.reload(&sub));

        // Enter：必须仍是 c.md（不是同序号位移后的 a.md）。
        visual.dispatch_action(menu::Confirm);
        assert_eq!(
            opened.lock().unwrap().as_slice(),
            [c_md.clone()],
            "#595 核心：刷新后 Enter 打开的仍是当初选中的 c.md（基线红：a.md）"
        );
        assert_eq!(
            selected_path(&tree, &visual),
            Some(c_md),
            "光标路径身份保持"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// #595 验收 2：删除更早条目（选中路径仍在）不跳；删除选中条目
    /// 或折叠其祖先（forget_subtree）→ 清空光标，Enter 不触发别的路径。
    #[gpui::test]
    fn reload_removal_and_forget_clear_or_keep_cursor(cx: &mut gpui::TestAppContext) {
        let root = temp_fixture("kb-595-removal");
        let sub = root.join("sub");
        let a_md = root.join("A.md");
        let c_md = sub.join("c.md");
        let opened: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
        let (tree, mut visual) = tree_window(cx, root.clone(), opened.clone());
        tree.update(&mut visual, |t, _| t.expand_to(&c_md));

        // ① 光标在 A.md（index 4）；删除更早条目 sub/c.md + reload →
        //    A.md 上移至 3。Enter 必须仍是 A.md（基线红：旧序号 4 落到
        //    b.md 行 → 误开 b.md）。
        focus_and_step_to(&tree, &mut visual, &a_md);
        std::fs::remove_file(&c_md).unwrap();
        tree.update(&mut visual, |t, _| t.reload(&sub));
        visual.dispatch_action(menu::Confirm);
        assert_eq!(
            opened.lock().unwrap().as_slice(),
            [a_md.clone()],
            "更早条目删除后 Enter 仍是 A.md（不跳到 b.md 的旧序号）"
        );

        // ② 删除选中条目 A.md + reload → 光标清空；Enter 不触发别的
        //    路径（基线红：序号 3 落到 b.md 行 → 误开 b.md）。
        std::fs::remove_file(&a_md).unwrap();
        tree.update(&mut visual, |t, _| t.reload(&root));
        assert_eq!(
            selected_path(&tree, &visual),
            None,
            "选中条目删除后光标清空（None 语义）"
        );
        let before_len = tree.read_with(&visual, |t, _| t.state.visible_rows().len());
        visual.dispatch_action(menu::Confirm);
        assert_eq!(
            opened.lock().unwrap().as_slice(),
            [a_md],
            "清空光标后 Enter 零新增打开（仅剩 ① 的那一次）"
        );
        assert_eq!(
            tree.read_with(&visual, |t, _| t.state.visible_rows().len()),
            before_len,
            "Enter 不误触发目录展开（清空光标 = 真空操作）"
        );

        // ③ 折叠其祖先（右键删除目录 → forget_subtree 同款状态面）
        //    ——新夹具防状态串扰：光标在 sub/c.md，遗忘 sub → 不可见
        //    → 清空；Enter 不误开（基线红：序号落到 A.md 行 → 误开）。
        let root3 = temp_fixture("kb-595-forget");
        let sub3 = root3.join("sub");
        let c_md3 = sub3.join("c.md");
        let opened3: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
        let (tree3, mut visual3) = tree_window(cx, root3.clone(), opened3.clone());
        tree3.update(&mut visual3, |t, _| t.expand_to(&c_md3));
        focus_and_step_to(&tree3, &mut visual3, &c_md3);
        tree3.update(&mut visual3, |t, _| t.forget_subtree(&sub3));
        assert_eq!(
            selected_path(&tree3, &visual3),
            None,
            "祖先折叠/子树遗忘后光标清空"
        );
        visual3.dispatch_action(menu::Confirm);
        assert!(
            opened3.lock().unwrap().is_empty(),
            "Enter 不触发别的路径（基线红：误开 A.md）"
        );
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&root3);
    }

    /// #595 验收 3：大树（超视口滚动面）+ expand_to 增行后仍保持
    /// 正确路径；目录 Enter 折叠/展开后光标仍在该目录行；None 状态
    /// 与上下界夹取不回归。
    #[gpui::test]
    fn big_tree_expand_and_dir_toggle_keep_cursor(cx: &mut gpui::TestAppContext) {
        let root = temp_fixture("kb-595-big");
        // 40 个文件（f00–f39）——行集 44 条 > 视口（400px/22px≈18 行），
        // 光标行必然经历虚拟化滚动面。
        for i in 0..40 {
            std::fs::write(root.join(format!("f{i:02}.md")), "f").unwrap();
        }
        let last = root.join("f39.md");
        let sub = root.join("sub");
        let opened: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
        let (tree, mut visual) = tree_window(cx, root.clone(), opened.clone());

        // 光标到末行 f39（None 起步 44 步，含末界夹取面）。
        focus_and_step_to(&tree, &mut visual, &last);

        // expand_to(sub/deep/d.md)——行集前插 4 行（sub/deep 展开）。
        tree.update(&mut visual, |t, _| {
            t.expand_to(&sub.join("deep").join("d.md"))
        });
        assert_eq!(
            selected_path(&tree, &visual),
            Some(last.clone()),
            "增行位移后光标路径身份保持（f39 仍选中；基线红：序号落到 f36）"
        );
        visual.dispatch_action(menu::Confirm);
        assert_eq!(
            opened.lock().unwrap().as_slice(),
            [last.clone()],
            "大树滚动 + expand_to 后 Enter 仍打开当初选中的 f39"
        );

        // 目录 Enter：光标到 sub → Enter 展开 → 光标仍在 sub 行（目录
        // 折叠/展开语义 + 光标身份都不回归）。
        focus_and_step_to(&tree, &mut visual, &sub);
        let sub_len_before = tree.read_with(&visual, |t, _| t.state.visible_rows().len());
        visual.dispatch_action(menu::Confirm);
        let after = tree.read_with(&visual, |t, _| {
            let rows = t.state.visible_rows();
            (
                rows.len(),
                t.selected
                    .and_then(|ix| rows.get(ix))
                    .map(|r| r.path.clone()),
            )
        });
        assert_ne!(after.0, sub_len_before, "Enter 目录 → 展开态翻转");
        assert_eq!(after.1, Some(sub.clone()), "折叠/展开后光标仍在该目录行");
        assert_eq!(opened.lock().unwrap().len(), 1, "目录 Enter 不开文件");

        // 无光标状态回归：None + reload + Enter → 不开、不崩。
        tree.update(&mut visual, |t, _| t.selected = None);
        std::fs::write(root.join("f99.md"), "f").unwrap();
        tree.update(&mut visual, |t, _| t.reload(&root));
        assert_eq!(
            selected_path(&tree, &visual),
            None,
            "None 光标 reload 后仍 None"
        );
        visual.dispatch_action(menu::Confirm);
        assert_eq!(opened.lock().unwrap().len(), 1, "无光标 Enter 零副作用");
        let _ = std::fs::remove_dir_all(&root);
    }
}
