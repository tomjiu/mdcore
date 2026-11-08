//! 顶部菜单栏模型层（#209）——纯数据，零 GPUI 依赖。
//!
//! # 分层（#385 批 4 后）
//!
//! - 本模块：**菜单结构**（四菜单 + 一层子菜单 + 动作映射）与灰显/勾选
//!   依据（`MenuContext` 快照 → `build_menus` 纯函数）——单测钉定，
//!   不起窗口可验；
//! - `menu_bar_view.rs`（bin 专属）：桥接层——`build_menu_bar_menu` 把
//!   本模块数据构建成 `ui::ContextMenu` 实体（浮层/行渲染/一层子菜单/
//!   键盘导航/点外关闭全部由组件自带，与 #385 批 1 右键菜单同构）+
//!   `anchored` 锚定挂载 + 自绘标题行（按钮族语系——对照表结论 2）；
//! - `shell.rs`：开合编排（`menu_bar_open`/`menu_bar_entity` 状态 +
//!   `DismissEvent` 订阅）与动作派发（`activate_menu_action`——与右键
//!   菜单/命令面板同一分发单点）。
//!
//! # 批 4 退役面（与批 1 同构的组件接管）
//!
//! 旧版自研的键盘导航状态机（`MenuBarState`/`navigate`——↑↓/Enter/
//! Esc/←→ 转移）、下拉几何估算（`to_context_items`/`row_y_offset`/
//! `estimate_anchor_x`——自研宽度估算与摆放翻转）随渲染层迁移一并
//! 退役：组件按实际渲染尺寸锚定（`gpui::anchored`），键盘语义内建
//! （`menu::SelectNext/Confirm/Cancel` 一族）。
//! 已知行为差异（诚实登记）：←→ 切换顶部菜单 → 组件语义（← 在顶层
//! 关闭菜单 / → 进子菜单）；键盘切换顶部菜单需 Esc 后鼠标点开。
//!
//! # 动作映射铁律（issue #209 层 2 / owner 指令）
//!
//! 菜单项**只做既有动作的映射**，不新增执行逻辑、不新增 Cmd 变体：
//!
//! | 菜单项类别 | 映射目标（既有单点） |
//! |---|---|
//! | 编辑器命令（保存/撤销/查找/替换…） | [`MenuAction::EditorCmd`] → `ShellApp::dispatch_editor_cmd` → `Editor::run_command`（与键位 / 命令面板 / 右键菜单 / 图标条搜索钮同一条链） |
//! | 剪贴板三件（剪切/复制/粘贴） | `Editor::menu_cut/menu_copy/menu_paste`（Cmd 枚举无剪贴板项——右键菜单 ed-cut 同款既有方法） |
//! | shell 级动作（打开/另存为/主题/字号/侧栏/退出/关于） | ShellApp 既有方法（`prompt_open_file` / `prompt_save_as` / `set_theme_mode` / `apply_font_size` / `toggle_left` / `cx.quit` / `show_notice`） |
//! | **无既有命令的项**（新建/状态栏/全屏/快捷键一览/文档） | 置灰禁用 + [`MenuAction::None`]，不绑动作——禁用态正确是本 issue 验收项 |
//!
//! # 快捷键标签的真实性
//!
//! 每个标签对应一条**真实存在的键位**（editor `dispatch_key` / shell
//! `on_key_down` 既有分支——见 `shortcut_labels_pin_real_bindings` 单测的
//! 钉定表）。无键位的项不标：标一条不存在的快捷键是可发现性欺骗，比不标
//! 更糟。重做标 `Ctrl+Shift+Z`（`dispatch_key` 的 `z if ctrl && shift` 臂
//! 实际绑定；`Ctrl+Y` 亦可，单标签取更常见的一条）。

use std::path::PathBuf;

use crate::editor::commands::Cmd;

/// 主题模式串（settings.json `theme` 键的持久化契约值——与
/// `theme_switch::MODE_*` 同值；本模块在 lib 树内取不到 bin 专属的
/// theme_switch，按稳定契约自带一份，`theme_constants_pin_contract`
/// 单测钉住取值域）。
pub const THEME_SYSTEM: &str = "system";
pub const THEME_LIGHT: &str = "light";
pub const THEME_DARK: &str = "dark";

/// 最近打开子菜单的条目上限（与空态引导页 `take(8)` 同口径）。
pub const RECENT_MENU_MAX: usize = 8;

/// 最近打开子菜单的静态 id 池（`MenuItemSpec.id` 是 `&'static str`——
/// 动态条目按序取池位，超池截断；id 唯一性由 `ids_unique_across_menus`
/// 单测钉定）。
const RECENT_IDS: [&str; RECENT_MENU_MAX] = [
    "file-recent-0",
    "file-recent-1",
    "file-recent-2",
    "file-recent-3",
    "file-recent-4",
    "file-recent-5",
    "file-recent-6",
    "file-recent-7",
];

/// 菜单构建上下文（ShellApp 状态快照——纯数据，可单测）。
///
/// 每次「打开菜单 / 渲染浮层 / 键盘导航」时由 shell 现场采集：菜单可用性
/// （灰显）与勾选标记全部由这一份快照决定，模型自身零状态。
pub struct MenuContext {
    /// 有活动标签（保存/另存为/编辑类命令的门槛）。
    pub has_document: bool,
    /// 活动标签只读态（docx/pdf 渲染面——变更类命令灰显）。
    pub read_only: bool,
    /// 活动标签有选区（剪切/复制门槛——与编辑器右键菜单同判定源）。
    pub has_selection: bool,
    /// 当前编辑器字号（px——字号子菜单的域界灰显，8-32）。
    pub font_size: f32,
    /// 当前主题模式（主题子菜单的勾选标记）。
    pub theme_mode: &'static str,
    /// 左栏开合（侧栏子菜单勾选标记）。
    pub left_open: bool,
    /// 左栏当前是 git 视图（侧栏子菜单勾选标记）。
    pub git_view: bool,
    /// 活动标签是 DOCX（#307「导出为 PDF」门槛——仅 docx 形态提供）。
    pub active_is_docx: bool,
    /// 活动标签是 PDF（#307 ②「导出为 DOCX」门槛——仅 pdf 形态提供）。
    pub active_is_pdf: bool,
    /// 最近打开的文件（路径序 = 子菜单条目序；`OpenRecent(usize)` 按此序回查）。
    pub recent_files: Vec<PathBuf>,
}

/// 菜单动作目标（shell `activate_menu_action` 的分派键——只映射既有动作）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    /// 编辑器命令（Cmd 枚举——与键位/命令面板同一分派路径）。
    EditorCmd(Cmd),
    /// 剪贴板：剪切（`Editor::menu_cut`——Cmd 枚举无剪贴板项）。
    Cut,
    /// 剪贴板：复制（`Editor::menu_copy`）。
    Copy,
    /// 剪贴板：粘贴（`Editor::menu_paste`）。
    Paste,
    /// 打开文件（shell 既有 `prompt_open_file`——Ctrl+O 同链）。
    OpenFile,
    /// 打开文件夹（shell 既有 `prompt_open_workspace`——Ctrl+K Ctrl+O 同链）。
    OpenFolder,
    /// 打开最近文件（序 = `MenuContext.recent_files` 序；shell 回查路径打开）。
    OpenRecent(usize),
    /// 另存为（shell 既有 `prompt_save_as`——Ctrl+Shift+S 同链）。
    SaveAs,
    /// #307 阶段①：导出为 PDF（shell `export_active_pdf`——DOCX 经
    /// LibreOffice headless 转 PDF；仅 docx 标签启用）。
    ExportPdf,
    /// #307 ②功能独立项：导出为 DOCX（shell `export_active_docx`——
    /// PDF 文本层经 mdc-pdf 引擎提取 + office-io 写面构造；仅 pdf 标签
    /// 启用；扫描件零文本显式拒（OCR 面 #306））。
    ExportDocx,
    /// 退出（与标题栏 × 钮同一条 `cx.quit` 既有路径）。
    Quit,
    /// 关于（既有提示横幅 `show_notice` 承载——层 3 可发现性验收项）。
    About,
    /// 命令面板（`Editor::toggle_command_palette`——Ctrl+Shift+P 键位臂
    /// 的既有逻辑提取为公开方法，两入口共用，行为零变化）。
    TogglePalette,
    /// #289-⑤：打开设置弹窗（shell `open_settings`——左条设置钮同链；
    /// 弹窗形态，不占任何 dock 视图）。
    OpenSettings,
    /// 设置主题模式（shell 既有 `set_theme_mode`）。
    SetTheme(&'static str),
    /// 字号增大 1px（shell 既有 `apply_font_size`——Ctrl+= 同链）。
    FontBigger,
    /// 字号减小 1px（Ctrl+- 同链）。
    FontSmaller,
    /// 字号重置 15px（Ctrl+0 同链）。
    FontReset,
    /// 切换文件树面板（shell 既有 `toggle_left`——图标条文件树钮同链）。
    ToggleFileTreePanel,
    /// 切换 git 面板（shell 既有 `toggle_git`——图标条 git 钮同链）。
    ToggleGitPanel,
    /// 占位（无既有命令可映射——置灰项不绑动作，派发面只留证据行）。
    None,
}

/// 菜单动作项规格（标签 / 快捷键标签 / 动作 / 灰显 / 勾选）。
#[derive(Clone, Debug, PartialEq)]
pub struct MenuItemSpec {
    /// 派发键（渲染层 element id 与日志定位用；全菜单唯一——单测钉定）。
    pub id: &'static str,
    pub label: String,
    /// 右侧快捷键标签（**从 keymap 表生成**——单一事实源，用户换绑后
    /// 跟随；只标真实键位，见模块文档「快捷键标签的真实性」）。
    pub shortcut: Option<String>,
    pub action: MenuAction,
    /// false = 置灰不可点（灰显而非消失——菜单长短不跳变，右键菜单同款）。
    pub enabled: bool,
    /// 左侧勾选标记（主题/侧栏子菜单的当前态）。
    pub checked: bool,
}

/// 一条菜单条目（动作 / 分隔线 / 子菜单——子菜单只一层，build_menus
/// 结构保证）。
#[derive(Clone, Debug, PartialEq)]
pub enum MenuEntry {
    Item(MenuItemSpec),
    Separator,
    Submenu {
        label: String,
        items: Vec<MenuEntry>,
    },
}

impl MenuEntry {
    /// 动作项构造（缺省可用、无快捷键、无勾选）。
    pub fn item(id: &'static str, label: impl Into<String>) -> Self {
        MenuEntry::Item(MenuItemSpec {
            id,
            label: label.into(),
            shortcut: None,
            action: MenuAction::None,
            enabled: true,
            checked: false,
        })
    }

    /// 子菜单构造（本菜单栏只允许一层子菜单——查看菜单的「主题/字号/
    /// 侧栏」与文件菜单的「最近打开」）。
    pub fn submenu(label: impl Into<String>, items: Vec<MenuEntry>) -> Self {
        MenuEntry::Submenu {
            label: label.into(),
            items,
        }
    }

    /// 从 keymap 表取该动作的当前键位标签（#217 单一事实源：默认表
    /// = 现状键位镜像；用户换绑后菜单标签跟随。无绑定 → 不标）。
    pub fn shortcut_for(mut self, action_id: &str) -> Self {
        if let MenuEntry::Item(spec) = &mut self {
            spec.shortcut = crate::keymap::store().label(action_id);
        }
        self
    }

    /// 绑定动作。
    pub fn act(mut self, action: MenuAction) -> Self {
        if let MenuEntry::Item(spec) = &mut self {
            spec.action = action;
        }
        self
    }

    /// 置灰/启用。
    pub fn with_enabled(mut self, on: bool) -> Self {
        if let MenuEntry::Item(spec) = &mut self {
            spec.enabled = on;
        }
        self
    }

    /// 勾选标记。
    pub fn checked(mut self, on: bool) -> Self {
        if let MenuEntry::Item(spec) = &mut self {
            spec.checked = on;
        }
        self
    }
}

/// 一个顶部菜单（文件/编辑/查看/帮助）。
#[derive(Clone, Debug, PartialEq)]
pub struct TopMenu {
    pub id: &'static str,
    /// 标题栏里的标题文案（也是键盘切菜单时锚点估算的宽度输入）。
    pub label: &'static str,
    pub entries: Vec<MenuEntry>,
}

/// 按上下文快照构建四菜单（纯函数——结构与灰显/勾选全部由 ctx 决定）。
pub fn build_menus(ctx: &MenuContext) -> Vec<TopMenu> {
    let doc = ctx.has_document;
    let writable = doc && !ctx.read_only;
    vec![
        TopMenu {
            id: "menu-file",
            label: "文件",
            entries: vec![
                // 无既有「新建文档」逻辑（Ctrl+N 未接线，也无空标签构造
                // 通道）——置灰不绑动作（owner 指令：无命令的项禁用）。
                MenuEntry::item("file-new", "新建").with_enabled(false),
                MenuEntry::item("file-open", "打开")
                    .shortcut_for("prompt_open_file")
                    .act(MenuAction::OpenFile),
                MenuEntry::item("file-open-folder", "打开文件夹")
                    .shortcut_for("prompt_open_workspace")
                    .act(MenuAction::OpenFolder),
                MenuEntry::item("file-save", "保存")
                    .shortcut_for("save")
                    .act(MenuAction::EditorCmd(Cmd::Save))
                    .with_enabled(doc),
                MenuEntry::item("file-save-as", "另存为")
                    .shortcut_for("prompt_save_as")
                    .act(MenuAction::SaveAs)
                    .with_enabled(doc),
                MenuEntry::item("file-export-pdf", "导出为 PDF")
                    .act(MenuAction::ExportPdf)
                    .with_enabled(ctx.active_is_docx),
                MenuEntry::item("file-export-docx", "导出为 DOCX（文本版）")
                    .act(MenuAction::ExportDocx)
                    .with_enabled(ctx.active_is_pdf),
                MenuEntry::Separator,
                MenuEntry::submenu("最近打开", recent_entries(ctx)),
                MenuEntry::Separator,
                MenuEntry::item("file-quit", "退出").act(MenuAction::Quit),
            ],
        },
        TopMenu {
            id: "menu-edit",
            label: "编辑",
            entries: vec![
                MenuEntry::item("edit-undo", "撤销")
                    .shortcut_for("undo")
                    .act(MenuAction::EditorCmd(Cmd::Undo))
                    .with_enabled(writable),
                // dispatch_key 的 redo 实际绑定是 Ctrl+Shift+Z（z if
                // ctrl && shift 臂）与 Ctrl+Y 双臂——标签取前者。
                MenuEntry::item("edit-redo", "重做")
                    .shortcut_for("redo")
                    .act(MenuAction::EditorCmd(Cmd::Redo))
                    .with_enabled(writable),
                MenuEntry::Separator,
                MenuEntry::item("edit-cut", "剪切")
                    .shortcut_for("cut")
                    .act(MenuAction::Cut)
                    .with_enabled(writable && ctx.has_selection),
                MenuEntry::item("edit-copy", "复制")
                    .shortcut_for("copy")
                    .act(MenuAction::Copy)
                    .with_enabled(doc && ctx.has_selection),
                MenuEntry::item("edit-paste", "粘贴")
                    .shortcut_for("paste")
                    .act(MenuAction::Paste)
                    .with_enabled(writable),
                MenuEntry::Separator,
                MenuEntry::item("edit-find", "查找")
                    .shortcut_for("open_find")
                    .act(MenuAction::EditorCmd(Cmd::OpenFind))
                    .with_enabled(doc),
                MenuEntry::item("edit-replace", "替换")
                    .shortcut_for("open_replace")
                    .act(MenuAction::EditorCmd(Cmd::OpenReplace))
                    .with_enabled(writable),
                MenuEntry::item("edit-palette", "命令面板")
                    .shortcut_for("toggle_command_palette")
                    .act(MenuAction::TogglePalette)
                    .with_enabled(doc),
            ],
        },
        TopMenu {
            id: "menu-view",
            label: "查看",
            entries: vec![
                MenuEntry::submenu(
                    "主题",
                    vec![
                        MenuEntry::item("view-theme-system", "跟随系统")
                            .act(MenuAction::SetTheme(THEME_SYSTEM))
                            .checked(ctx.theme_mode == THEME_SYSTEM),
                        MenuEntry::item("view-theme-light", "浅色")
                            .act(MenuAction::SetTheme(THEME_LIGHT))
                            .checked(ctx.theme_mode == THEME_LIGHT),
                        MenuEntry::item("view-theme-dark", "深色")
                            .act(MenuAction::SetTheme(THEME_DARK))
                            .checked(ctx.theme_mode == THEME_DARK),
                    ],
                ),
                MenuEntry::submenu(
                    "字号",
                    vec![
                        // 域界灰显（设置面板步进钮同口径：8-32）。
                        MenuEntry::item("view-font-inc", "增大")
                            .shortcut_for("font_bigger")
                            .act(MenuAction::FontBigger)
                            .with_enabled(ctx.font_size < 32.),
                        MenuEntry::item("view-font-dec", "减小")
                            .shortcut_for("font_smaller")
                            .act(MenuAction::FontSmaller)
                            .with_enabled(ctx.font_size > 8.),
                        MenuEntry::item("view-font-reset", "重置")
                            .shortcut_for("font_reset")
                            .act(MenuAction::FontReset),
                    ],
                ),
                MenuEntry::submenu(
                    "侧栏",
                    vec![
                        MenuEntry::item("view-side-file-tree", "文件树")
                            .act(MenuAction::ToggleFileTreePanel)
                            .checked(ctx.left_open && !ctx.git_view),
                        MenuEntry::item("view-side-git", "改动 / 历史（git）")
                            .act(MenuAction::ToggleGitPanel)
                            .checked(ctx.left_open && ctx.git_view),
                        // 搜索 = 既有 Cmd::OpenFind（图标条搜索钮同链）。
                        MenuEntry::item("view-side-search", "搜索")
                            .shortcut_for("open_find")
                            .act(MenuAction::EditorCmd(Cmd::OpenFind))
                            .with_enabled(doc),
                    ],
                ),
                MenuEntry::Separator,
                // 状态栏开关无既有命令（status_bar 恒渲染）——置灰。
                MenuEntry::item("view-status-bar", "状态栏").with_enabled(false),
                // 全屏无既有命令（owner 点名的禁用示例）——置灰。
                MenuEntry::item("view-fullscreen", "全屏").with_enabled(false),
                // #289-⑤：设置入口（弹窗形态——vscode「偏好沉底」同款
                // 位置；与左条设置钮同一条 open_settings 链，不占右栏）。
                MenuEntry::item("view-settings", "设置").act(MenuAction::OpenSettings),
            ],
        },
        TopMenu {
            id: "menu-help",
            label: "帮助",
            entries: vec![
                MenuEntry::item("help-about", "关于").act(MenuAction::About),
                // 无既有快捷键一览面——置灰。
                MenuEntry::item("help-shortcuts", "快捷键一览").with_enabled(false),
                // 无应用内文档面（owner 点名的禁用示例）——置灰。
                MenuEntry::item("help-docs", "文档").with_enabled(false),
            ],
        },
    ]
}

/// 「最近打开」子菜单（文件名标签；已删除的置灰——空态引导页同口径；
/// 空列表给一条置灰占位，子菜单不塌成零高度）。
fn recent_entries(ctx: &MenuContext) -> Vec<MenuEntry> {
    if ctx.recent_files.is_empty() {
        return vec![MenuEntry::item("file-recent-empty", "（暂无记录）").with_enabled(false)];
    }
    ctx.recent_files
        .iter()
        .take(RECENT_MENU_MAX)
        .enumerate()
        .map(|(i, path)| {
            let label = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            MenuEntry::item(RECENT_IDS[i], label)
                .act(MenuAction::OpenRecent(i))
                .with_enabled(path.exists())
        })
        .collect()
}

// ─── 标题栏布局常量（视图层布局消费——menu_titles 唯一使用方） ─────

/// 菜单标题按钮左右内边距（px_3——menu_bar_view 同值）。
///
/// #289-②：菜单栏贴窗口左缘（旧 TITLE_LEFT_PAD/TITLE_TO_MENUS_GAP
/// 随「左侧标题列」一起废弃——首个菜单自身的 px 即左缘内边距）。
pub const TITLE_BTN_PAD_X: f32 = 12.;
/// 菜单标题按钮间距（gap_1）。
pub const TITLES_GAP: f32 = 4.;

// ─── 单测（纯逻辑——零 gpui 依赖，无窗口可验） ───────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 满配上下文（有文档/有选区/可写/浅色主题/左栏开文件树/有最近记录）。
    fn full_ctx() -> MenuContext {
        MenuContext {
            has_document: true,
            read_only: false,
            has_selection: true,
            font_size: 15.,
            theme_mode: THEME_LIGHT,
            left_open: true,
            git_view: false,
            active_is_docx: true,
            active_is_pdf: false,
            recent_files: vec![
                PathBuf::from("/tmp/m209-a.md"),
                PathBuf::from("/tmp/不存在-b.md"),
            ],
        }
    }

    /// 空态上下文（无文档/跟随系统/无最近记录）。
    fn empty_ctx() -> MenuContext {
        MenuContext {
            has_document: false,
            read_only: false,
            has_selection: false,
            font_size: 15.,
            theme_mode: THEME_SYSTEM,
            left_open: true,
            git_view: false,
            active_is_docx: false,
            active_is_pdf: false,
            recent_files: Vec::new(),
        }
    }

    /// 按 id 找条目（含子菜单——返回 (所在层路径, 规格)）。
    fn find<'a>(entries: &'a [MenuEntry], id: &str) -> Option<&'a MenuItemSpec> {
        for entry in entries {
            match entry {
                MenuEntry::Item(spec) if spec.id == id => return Some(spec),
                MenuEntry::Submenu { items, .. } => {
                    if let Some(found) = find(items, id) {
                        return Some(found);
                    }
                }
                _ => {}
            }
        }
        None
    }

    #[test]
    fn four_menus_in_canonical_order() {
        let menus = build_menus(&empty_ctx());
        let labels: Vec<&str> = menus.iter().map(|m| m.label).collect();
        assert_eq!(
            labels,
            vec!["文件", "编辑", "查看", "帮助"],
            "四菜单按约定序"
        );
        assert!(menus.iter().all(|m| !m.entries.is_empty()), "菜单不空");
    }

    #[test]
    fn ids_unique_across_menus() {
        let menus = build_menus(&full_ctx());
        let mut seen: Vec<&'static str> = Vec::new();
        fn collect(entries: &[MenuEntry], seen: &mut Vec<&'static str>) {
            for entry in entries {
                match entry {
                    MenuEntry::Item(spec) => {
                        assert!(!seen.contains(&spec.id), "菜单项 id 重复：{}", spec.id);
                        seen.push(spec.id);
                    }
                    MenuEntry::Submenu { items, .. } => collect(items, seen),
                    MenuEntry::Separator => {}
                }
            }
        }
        for menu in &menus {
            collect(&menu.entries, &mut seen);
        }
    }

    /// 快捷键标签 ↔ 真实键位钉定表（editor dispatch_key / shell on_key_down
    /// 的既有分支——标签标了就必须真有这条键位，否则是可发现性欺骗）。
    #[test]
    fn shortcut_labels_pin_real_bindings() {
        let menus = build_menus(&full_ctx());
        let pinned: &[(&str, &str)] = &[
            ("file-open", "Ctrl+O"),
            ("file-open-folder", "Ctrl+K Ctrl+O"),
            ("file-save", "Ctrl+S"),
            ("file-save-as", "Ctrl+Shift+S"),
            ("edit-undo", "Ctrl+Z"),
            ("edit-redo", "Ctrl+Shift+Z"),
            ("edit-cut", "Ctrl+X"),
            ("edit-copy", "Ctrl+C"),
            ("edit-paste", "Ctrl+V"),
            ("edit-find", "Ctrl+F"),
            ("edit-replace", "Ctrl+H"),
            ("edit-palette", "Ctrl+Shift+P"),
            ("view-font-inc", "Ctrl+="),
            ("view-font-dec", "Ctrl+-"),
            ("view-font-reset", "Ctrl+0"),
            ("view-side-search", "Ctrl+F"),
        ];
        for (id, key) in pinned {
            let spec = find(&menus[0].entries, id)
                .or_else(|| find(&menus[1].entries, id))
                .or_else(|| find(&menus[2].entries, id))
                .unwrap_or_else(|| panic!("{id} 缺失"));
            assert_eq!(spec.shortcut.as_deref(), Some(*key), "{id} 快捷键标签漂移");
        }
        // 无键位的项不标（新建/退出/主题/侧栏切换等——不存在「标了但按了没反应」）。
        for id in [
            "file-new",
            "file-quit",
            "help-about",
            "view-status-bar",
            "view-fullscreen",
        ] {
            let spec = find(&menus[0].entries, id)
                .or_else(|| find(&menus[2].entries, id))
                .or_else(|| find(&menus[3].entries, id))
                .unwrap_or_else(|| panic!("{id} 缺失"));
            assert_eq!(spec.shortcut, None, "{id} 不应有快捷键标签（无真实键位）");
        }
    }

    /// EditorCmd 映射的对应关系（菜单项 → Cmd 变体——层 2 对接的核心表）。
    #[test]
    fn editor_cmd_mappings() {
        let menus = build_menus(&full_ctx());
        let cases: &[(&str, Cmd)] = &[
            ("file-save", Cmd::Save),
            ("edit-undo", Cmd::Undo),
            ("edit-redo", Cmd::Redo),
            ("edit-find", Cmd::OpenFind),
            ("edit-replace", Cmd::OpenReplace),
            ("view-side-search", Cmd::OpenFind),
        ];
        for (id, want) in cases {
            let spec = find(&menus[0].entries, id)
                .or_else(|| find(&menus[1].entries, id))
                .or_else(|| find(&menus[2].entries, id))
                .unwrap_or_else(|| panic!("{id} 缺失"));
            assert_eq!(
                spec.action,
                MenuAction::EditorCmd(*want),
                "{id} Cmd 映射漂移"
            );
        }
    }

    /// 无既有命令的项：置灰 + 不绑动作（禁用态是验收项）。
    #[test]
    fn placeholder_items_disabled_with_no_action() {
        let menus = build_menus(&full_ctx());
        for id in [
            "file-new",
            "view-status-bar",
            "view-fullscreen",
            "help-shortcuts",
            "help-docs",
        ] {
            let spec = find(&menus[0].entries, id)
                .or_else(|| find(&menus[2].entries, id))
                .or_else(|| find(&menus[3].entries, id))
                .unwrap_or_else(|| panic!("{id} 缺失"));
            assert!(!spec.enabled, "{id} 应置灰（无既有命令）");
            assert_eq!(spec.action, MenuAction::None, "{id} 置灰项不得绑动作");
        }
    }

    /// 空态（无文档）：需要文档的项灰显，shell 级动作保持可用。
    #[test]
    fn no_document_disables_editor_actions_only() {
        let menus = build_menus(&empty_ctx());
        let disabled = [
            "file-save",
            "file-save-as",
            "edit-undo",
            "edit-redo",
            "edit-cut",
            "edit-copy",
            "edit-paste",
            "edit-find",
            "edit-replace",
            "edit-palette",
            "view-side-search",
        ];
        for id in disabled {
            let spec = find(&menus[0].entries, id)
                .or_else(|| find(&menus[1].entries, id))
                .or_else(|| find(&menus[2].entries, id))
                .unwrap_or_else(|| panic!("{id} 缺失"));
            assert!(!spec.enabled, "无文档时 {id} 应灰显");
        }
        // shell 级动作不依赖文档。
        let enabled = ["file-open", "file-open-folder", "file-quit", "help-about"];
        for id in enabled {
            let spec = find(&menus[0].entries, id)
                .or_else(|| find(&menus[3].entries, id))
                .unwrap_or_else(|| panic!("{id} 缺失"));
            assert!(spec.enabled, "无文档时 {id} 仍应可用（shell 级动作）");
        }
    }

    /// 只读态（docx/pdf）：变更类灰显，浏览类（复制/查找）可用——与编辑器
    /// 右键菜单同口径（readonly_still_allows_copy_and_find 的镜像）。
    #[test]
    fn read_only_disables_writes_allows_browsing() {
        let mut ctx = full_ctx();
        ctx.read_only = true;
        let menus = build_menus(&ctx);
        for id in [
            "edit-undo",
            "edit-redo",
            "edit-cut",
            "edit-paste",
            "edit-replace",
        ] {
            let spec = find(&menus[1].entries, id).unwrap_or_else(|| panic!("{id} 缺失"));
            assert!(!spec.enabled, "只读态 {id} 应灰显");
        }
        for id in ["edit-copy", "edit-find"] {
            let spec = find(&menus[1].entries, id).unwrap_or_else(|| panic!("{id} 缺失"));
            assert!(spec.enabled, "只读态 {id} 仍应可用");
        }
    }

    /// 无选区：剪切/复制灰显（粘贴不受选区影响——右键菜单同判定源）。
    #[test]
    fn no_selection_disables_cut_copy() {
        let mut ctx = full_ctx();
        ctx.has_selection = false;
        let menus = build_menus(&ctx);
        for id in ["edit-cut", "edit-copy"] {
            let spec = find(&menus[1].entries, id).unwrap_or_else(|| panic!("{id} 缺失"));
            assert!(!spec.enabled, "无选区 {id} 应灰显");
        }
        let paste = find(&menus[1].entries, "edit-paste").unwrap();
        assert!(paste.enabled, "粘贴不受选区影响");
    }

    /// 主题子菜单：任一模式下恰好一项勾选（三选一 radio 语义）。
    #[test]
    fn theme_submenu_checks_exactly_one() {
        for mode in [THEME_SYSTEM, THEME_LIGHT, THEME_DARK] {
            let mut ctx = full_ctx();
            ctx.theme_mode = mode;
            let menus = build_menus(&ctx);
            let ids = ["view-theme-system", "view-theme-light", "view-theme-dark"];
            let checked: Vec<bool> = ids
                .iter()
                .map(|id| {
                    find(&menus[2].entries, id)
                        .unwrap_or_else(|| panic!("{id} 缺失"))
                        .checked
                })
                .collect();
            assert_eq!(
                checked.iter().filter(|c| **c).count(),
                1,
                "模式 {mode} 应恰好一项勾选：{checked:?}"
            );
        }
    }

    /// 主题子菜单动作值 = 持久化契约串（与 theme_switch::MODE_* 同值）。
    #[test]
    fn theme_constants_pin_contract() {
        assert_eq!(THEME_SYSTEM, "system");
        assert_eq!(THEME_LIGHT, "light");
        assert_eq!(THEME_DARK, "dark");
        let menus = build_menus(&full_ctx());
        assert_eq!(
            find(&menus[2].entries, "view-theme-system").unwrap().action,
            MenuAction::SetTheme("system")
        );
        assert_eq!(
            find(&menus[2].entries, "view-theme-light").unwrap().action,
            MenuAction::SetTheme("light")
        );
        assert_eq!(
            find(&menus[2].entries, "view-theme-dark").unwrap().action,
            MenuAction::SetTheme("dark")
        );
    }

    /// 侧栏子菜单勾选：文件树/git 互斥，跟随 left_open/git_view。
    #[test]
    fn sidebar_submenu_checks_panel_state() {
        for (left, git, filetree_checked, git_checked) in [
            (true, false, true, false),
            (true, true, false, true),
            (false, false, false, false),
        ] {
            let mut ctx = full_ctx();
            ctx.left_open = left;
            ctx.git_view = git;
            let menus = build_menus(&ctx);
            assert_eq!(
                find(&menus[2].entries, "view-side-file-tree")
                    .unwrap()
                    .checked,
                filetree_checked
            );
            assert_eq!(
                find(&menus[2].entries, "view-side-git").unwrap().checked,
                git_checked
            );
        }
    }

    /// 字号子菜单域界：8 下限不能再减、32 上限不能再增（设置面板步进钮
    /// 同口径）。
    #[test]
    fn font_submenu_bounds() {
        let mut ctx = full_ctx();
        ctx.font_size = 32.;
        let menus = build_menus(&ctx);
        assert!(
            !find(&menus[2].entries, "view-font-inc").unwrap().enabled,
            "32 上限增大应灰显"
        );
        assert!(find(&menus[2].entries, "view-font-dec").unwrap().enabled);
        ctx.font_size = 8.;
        let menus = build_menus(&ctx);
        assert!(
            !find(&menus[2].entries, "view-font-dec").unwrap().enabled,
            "8 下限减小应灰显"
        );
        assert!(find(&menus[2].entries, "view-font-inc").unwrap().enabled);
    }

    /// 最近打开子菜单：序号动作映射、文件名标签、缺失文件置灰、空态占位。
    #[test]
    fn recent_submenu_semantics() {
        // 存在性判定走真文件（temp 目录落一个临时件——exists() 是
        // build_menus 的灰显依据，不 mock）。
        let live = std::env::temp_dir().join(format!("m209-live-{}.md", std::process::id()));
        std::fs::write(&live, "").expect("临时夹具落盘");
        let mut ctx = full_ctx();
        ctx.recent_files = vec![live.clone(), PathBuf::from("/tmp/m209-不存在-b.md")];
        let menus = build_menus(&ctx);
        let first = find(&menus[0].entries, "file-recent-0").unwrap();
        assert_eq!(first.action, MenuAction::OpenRecent(0));
        assert!(
            first.label.ends_with(".md"),
            "标签取文件名：{}",
            first.label
        );
        assert!(first.enabled, "存在的文件可点");
        let missing = find(&menus[0].entries, "file-recent-1").unwrap();
        assert!(!missing.enabled, "已删除的最近文件置灰（空态引导页同口径）");
        std::fs::remove_file(&live).ok();

        let empty = build_menus(&empty_ctx());
        let placeholder = find(&empty[0].entries, "file-recent-empty").unwrap();
        assert!(!placeholder.enabled, "空记录占位置灰");
    }

    /// 最近打开超池截断（>8 条只取前 8——id 池只有 8 位）。
    #[test]
    fn recent_submenu_caps_at_pool_size() {
        let mut ctx = full_ctx();
        ctx.recent_files = (0..12)
            .map(|i| PathBuf::from(format!("/tmp/m209-{i}.md")))
            .collect();
        let menus = build_menus(&ctx);
        assert!(
            find(&menus[0].entries, "file-recent-7").is_some(),
            "第 8 条在池内"
        );
        assert!(
            find(&menus[0].entries, "file-recent-8").is_none(),
            "第 9 条截断（无池位）"
        );
    }
}
