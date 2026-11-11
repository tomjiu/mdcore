//! git 面板（#68 阶段 F7）——改动 / 历史两视图。
//!
//! 技术路线 = Lead 派单**选项 B**：`std::process::Command` 调系统 git
//! （zed git crate 不在 32 crate 闭包内——选项 A 需扩闭包引依赖雪球，
//! Lead 预案「不在 -> 用 B，不搬新 crate」）。
//!
//! - **改动**：`git status --porcelain`（XY 码 + 路径——工作区相对）；
//!   点击 → 只读 diff 标签（`git diff -- <path>` 行级文本）；
//! - **历史**：`git log --pretty=%h|%s|%cr`（前 50 条）；点击 → 该提交
//!   变更文件清单（`git show --name-status --pretty=format:`）只读标签；
//! - **非 git 目录**：明确提示不崩溃（`is_git_repo` 探测）；
//! - git 不在 PATH / 命令失败 → 同「不可用」提示面。
//!
//! 解析面为纯函数（porcelain/日志行 → 结构——单测钉定）；命令执行为
//! 同步（~10-50ms 量级，UI 线程可接受——异步化登记后续）。

use std::path::{Path, PathBuf};

use gpui::{Context, IntoElement, MouseButton, Render, WeakEntity};
use ui::prelude::*;
use ui::{
    Chip, CountBadge, Icon, IconName, IconSize, Label, LabelSize, List, ListItem, Navigable,
    NavigableEntry,
};

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{FocusHandle, ScrollAnchor, ScrollHandle, Window};

/// 只读标签打开钩子（bin 面 ShellApp::open_readonly_tab + 焦点落位
/// 注入；入参 = 标题 / 文本）。#577 lib 化：diff/commit 的打开面共用。
pub type OpenReadonlyHook = Arc<dyn Fn(String, String, &mut Window, &mut gpui::App)>;

/// Esc 焦点归还钩子（bin 面：焦点还激活编辑器——#571/#575 同语义）。
pub type EscapeHook = Arc<dyn Fn(&mut Window, &mut gpui::App)>;

/// ShellApp 面收口（#577：本模块不引 shell——#575 FileTreeHooks
/// 同型依赖倒置）。
pub struct GitPanelHooks {
    pub open_readonly: OpenReadonlyHook,
    pub escape: EscapeHook,
}

/// #588：纯数据层（结构/命令/解析 + 单测）已迁 [`crate::git_repo`]
/// （lib 树——集成测试可达 + CI `--lib` 面真跑）；本模块只保留
/// 面板实体与渲染。
use crate::git_repo::{
    GitChange, GitCommit, git_available, git_diff, git_log, git_show_files, git_status, is_git_repo,
};

/// 视图模式（改动 / 历史）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GitView {
    Changes,
    History,
}

/// git 面板实体（宿主 ShellApp；工作区打开且是 git 仓库时创建）。
pub struct GitPanel {
    root: PathBuf,
    /// #577：bin 面钩子（open_readonly / escape）。
    hooks: GitPanelHooks,
    view: GitView,
    changes: Vec<GitChange>,
    commits: Vec<GitCommit>,
    /// 探测结论（None = 面板不可用的原因文案——非 git 目录/git 缺失）。
    unavailable: Option<String>,
    /// #577：行焦点句柄（改动面 path 键 / 历史面 hash 键——差量复用，
    /// 刷新后稳定）。
    change_handles: HashMap<String, FocusHandle>,
    commit_handles: HashMap<String, FocusHandle>,
    /// #577：面板根焦点（键位域挂载点）。
    panel_focus: FocusHandle,
    /// #577：行区滚动句柄（NavigableEntry 滚动锚点）。
    scroll: ScrollHandle,
}

impl GitPanel {
    /// 构造并刷新（改动 + 历史双拉）。
    pub fn new(root: PathBuf, hooks: GitPanelHooks, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            root,
            hooks,
            view: GitView::Changes,
            changes: Vec::new(),
            commits: Vec::new(),
            unavailable: None,
            change_handles: HashMap::new(),
            commit_handles: HashMap::new(),
            panel_focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        };
        this.refresh();
        this
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 刷新（状态 + 历史；不可用态记录原因文案）。
    pub fn refresh(&mut self) {
        if !git_available() {
            self.unavailable = Some("git 不可用（未安装或不在 PATH）".to_string());
            eprintln!("git: 不可用（缺 git）");
            return;
        }
        if !is_git_repo(&self.root) {
            self.unavailable = Some("非 git 目录（当前工作区不含 .git）".to_string());
            eprintln!("git: 非 git 目录（{}）", self.root.display());
            return;
        }
        self.unavailable = None;
        self.changes = git_status(&self.root).unwrap_or_default();
        self.commits = git_log(&self.root, 50).unwrap_or_default();
        // #577：行集变化 → 句柄差量清退（键存续复用——刷新后焦点稳定）。
        self.prune_handles();
        eprintln!(
            "git: {} 改动 / {} 提交（{}）",
            self.changes.len(),
            self.commits.len(),
            self.root.display()
        );
    }

    /// #577：句柄差量清退（refresh 尾调用——行集变化后键仍在的行
    /// 复用句柄，离场行清退；可单测）。
    fn prune_handles(&mut self) {
        self.change_handles
            .retain(|path, _| self.changes.iter().any(|c| &c.path == path));
        self.commit_handles
            .retain(|hash, _| self.commits.iter().any(|c| &c.hash == hash));
    }

    /// 打开改动 diff（只读标签）。#577：ShellApp 面 → open_readonly
    /// 钩子（bin 注入——lib 化）；diff 生成（git 逻辑）留在本面。
    fn open_diff(&self, path: &str, window: &mut gpui::Window, _cx: &mut Context<Self>) {
        let Some(text) = git_diff(&self.root, path) else {
            return;
        };
        let title = format!("diff: {path}");
        let hook = self.hooks.open_readonly.clone();
        hook(title, text, window, _cx);
    }

    /// 打开提交变更清单（只读标签；钩子语义同 [`Self::open_diff`]）。
    fn open_commit(&self, hash: &str, window: &mut gpui::Window, _cx: &mut Context<Self>) {
        let Some(text) = git_show_files(&self.root, hash) else {
            return;
        };
        let title = format!("提交 {hash}");
        let hook = self.hooks.open_readonly.clone();
        hook(title, text, window, _cx);
    }

    /// #577：Enter 激活聚焦行（与点击同链）：活动视图内找含焦点行
    /// → open_diff / open_commit。
    fn activate_focused(&self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        match self.view {
            GitView::Changes => {
                if let Some(change) = self.changes.iter().find(|c| {
                    self.change_handles
                        .get(&c.path)
                        .is_some_and(|h| h.contains_focused(window, cx))
                }) {
                    self.open_diff(&change.path.clone(), window, cx);
                }
            }
            GitView::History => {
                if let Some(commit) = self.commits.iter().find(|c| {
                    self.commit_handles
                        .get(&c.hash)
                        .is_some_and(|h| h.contains_focused(window, cx))
                }) {
                    self.open_commit(&commit.hash.clone(), window, cx);
                }
            }
        }
    }

    /// 首个改动（烟雾面：根 + 路径）。
    pub fn first_change(&self) -> Option<(PathBuf, String)> {
        self.changes
            .first()
            .map(|c| (self.root.clone(), c.path.clone()))
    }
}

impl Render for GitPanel {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let hover = colors.element_hover;
        let selected = colors.element_selected;

        // 头部：视图切换（改动/历史）+ 刷新。刷新钮在不可用态（非 git
        // 目录 / 缺 git）进入**禁用态**（#47 E-2 禁用态夹具——按钮六问①
        // 「点击有响应」的反面：无动作可执行时如实禁用，不再伪装可点）：
        // 无 hover 反馈、无点击监听、Label 用 ui::Color::Disabled。
        let refresh_disabled = self.unavailable.is_some();
        let view = self.view;
        let changes_count = self.changes.len();
        let header = h_flex()
            .id("git-header")
            .w_full()
            .flex_shrink_0()
            .gap_1()
            .mb_1()
            .child(
                h_flex()
                    .id("git-tab-changes")
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .gap_1()
                    .when(view == GitView::Changes, move |el| el.bg(selected))
                    .hover(move |el| el.bg(hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.view = GitView::Changes;
                            cx.notify();
                        }),
                    )
                    .child(
                        Icon::new(IconName::GitBranch)
                            .size(IconSize::XSmall)
                            .color(ui::Color::Muted),
                    )
                    .child(Label::new("改动").size(LabelSize::Small))
                    .child(CountBadge::new(changes_count)),
            )
            .child(
                h_flex()
                    .id("git-tab-history")
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .when(view == GitView::History, move |el| el.bg(selected))
                    .hover(move |el| el.bg(hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.view = GitView::History;
                            cx.notify();
                        }),
                    )
                    .child(
                        Icon::new(IconName::GitCommit)
                            .size(IconSize::XSmall)
                            .color(ui::Color::Muted),
                    )
                    .child(Label::new("历史").size(LabelSize::Small)),
            )
            .child(div().flex_1())
            .child({
                let btn =
                    h_flex()
                        .id("git-refresh")
                        // #47 E-2 禁用态批：测试定位锚（shell.rs editor-tabs
                        // 同款 id+debug_selector 配对——debug_bounds 需显式选择器）。
                        .debug_selector(|| "git-refresh".into())
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .when(!refresh_disabled, |el| {
                            el.hover(move |el| el.bg(hover)).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.refresh();
                                    cx.notify();
                                }),
                            )
                        })
                        .child(Label::new("刷新").size(LabelSize::Small).color(
                            if refresh_disabled {
                                ui::Color::Disabled
                            } else {
                                ui::Color::Muted
                            },
                        ));
                btn
            });

        // 体：不可用提示 / 对应视图列表。
        let body: gpui::AnyElement = if let Some(reason) = &self.unavailable {
            List::new().empty_message(reason.clone()).into_any_element()
        } else {
            // #577：行焦点句柄预建（键差量——path/hash 键取或建；刷新
            // 后键存续行复用同句柄，焦点身份稳定）。
            for change in &self.changes {
                self.change_handles
                    .entry(change.path.clone())
                    .or_insert_with(|| cx.focus_handle());
            }
            for commit in &self.commits {
                self.commit_handles
                    .entry(commit.hash.clone())
                    .or_insert_with(|| cx.focus_handle());
            }
            let scroll = self.scroll.clone();
            match self.view {
                GitView::Changes => {
                    if self.changes.is_empty() {
                        List::new()
                            .empty_message("工作区干净（无改动）")
                            .into_any_element()
                    } else {
                        let mut entries = Vec::with_capacity(self.changes.len());
                        let rows = List::new().children(self.changes.iter().map(|change| {
                            let path = change.path.clone();
                            let path_for_id = path.clone();
                            let path_for_click = path.clone();
                            let path_for_debug = path.clone();
                            let code = change.code.clone();
                            let handle = self.change_handles[&path].clone();
                            let anchor = ScrollAnchor::for_handle(scroll.clone());
                            entries.push(NavigableEntry {
                                focus_handle: handle.clone(),
                                scroll_anchor: Some(anchor.clone()),
                            });
                            div()
                                .id(gpui::SharedString::from(format!(
                                    "git-chg-row-{path_for_id}"
                                )))
                                .debug_selector(move || format!("git-chg-row-{path_for_debug}"))
                                .track_focus(&handle)
                                .anchor_scroll(Some(anchor))
                                // 聚焦与打开在同一 child 回调；避免父 click 冒泡抢回焦点。
                                .child(
                                    ListItem::new(gpui::SharedString::from(format!(
                                        "git-chg-{path}"
                                    )))
                                    .inset(true)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        handle.focus(window, cx);
                                        this.open_diff(&path_for_click, window, cx);
                                    }))
                                    .child(
                                        h_flex()
                                            .w_full()
                                            .gap_2()
                                            .items_center()
                                            // 状态码 → Chip（accent 色系与旧
                                            // Label::Accent 同源；M/A/D/?? 等
                                            // git 语义码一屏可辨）。
                                            .child(
                                                Chip::new(code)
                                                    .label_size(LabelSize::XSmall)
                                                    .label_color(ui::Color::Accent),
                                            )
                                            .child(
                                                Label::new(path.as_str()).size(LabelSize::Small),
                                            ),
                                    ),
                                )
                        }));
                        // #577：Navigable 包裹（↑↓ 遍历 + 滚动锚点——
                        // 批 7 同款；行列表非虚拟化，适配）。
                        let mut nav = Navigable::new(
                            v_flex()
                                .id("git-changes")
                                .size_full()
                                .overflow_y_scroll()
                                .track_scroll(&scroll)
                                .child(rows)
                                .into_any_element(),
                        );
                        for entry in entries {
                            nav = nav.entry(entry);
                        }
                        nav.into_any_element()
                    }
                }
                GitView::History => {
                    let mut entries = Vec::with_capacity(self.commits.len());
                    let rows = List::new().children(self.commits.iter().map(|commit| {
                        let hash = commit.hash.clone();
                        let hash_for_debug = hash.clone();
                        let handle = self.commit_handles[&hash].clone();
                        let anchor = ScrollAnchor::for_handle(scroll.clone());
                        entries.push(NavigableEntry {
                            focus_handle: handle.clone(),
                            scroll_anchor: Some(anchor.clone()),
                        });
                        div()
                            .id(gpui::SharedString::from(format!("git-cmt-row-{hash}")))
                            .debug_selector(move || format!("git-cmt-row-{hash_for_debug}"))
                            .track_focus(&handle)
                            .anchor_scroll(Some(anchor))
                            .child(
                                ListItem::new(gpui::SharedString::from(format!("git-cmt-{hash}")))
                                    .inset(true)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        handle.focus(window, cx);
                                        this.open_commit(&hash, window, cx);
                                    }))
                                    .child(
                                        v_flex()
                                            .w_full()
                                            .gap_0p5()
                                            .child(
                                                h_flex()
                                                    .gap_2()
                                                    .child(
                                                        Label::new(commit.hash.as_str())
                                                            .size(LabelSize::XSmall)
                                                            .color(ui::Color::Accent),
                                                    )
                                                    .child(
                                                        Label::new(commit.time.as_str())
                                                            .size(LabelSize::XSmall)
                                                            .color(ui::Color::Hidden),
                                                    ),
                                            )
                                            .child(
                                                Label::new(commit.subject.as_str())
                                                    .size(LabelSize::Small),
                                            ),
                                    ),
                            )
                    }));
                    let mut nav = Navigable::new(
                        v_flex()
                            .id("git-history")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&scroll)
                            .child(rows)
                            .into_any_element(),
                    );
                    for entry in entries {
                        nav = nav.entry(entry);
                    }
                    nav.into_any_element()
                }
            }
        };

        // #577：键位域根（menu 上下文 = #571/#575 同源）+ 焦点挂载。
        let panel_focus = self.panel_focus.clone();
        let escape = self.hooks.escape.clone();
        v_flex()
            .id("git-panel")
            .key_context("menu")
            .track_focus(&panel_focus)
            // Enter（menu::Confirm）：激活聚焦行（活动视图）。
            .on_action(cx.listener(|this, _: &menu::Confirm, window, cx| {
                this.activate_focused(window, cx);
            }))
            // Esc（menu::Cancel）：焦点还编辑器（钩子——bin 注入）。
            .on_action(cx.listener(move |_, _: &menu::Cancel, window, cx| {
                escape(window, cx);
            }))
            .w_full()
            .flex_1()
            .min_h_0()
            .gap_1()
            .child(header)
            // Navigable 自身 size_full；外层为剩余高度提供有界滚动 viewport。
            .child(div().w_full().flex_1().min_h_0().child(body))
    }
}

// ---------------------------------------------------------------------------
// #119（mdc-dock 批①）：GitPanel 作为 dock 面板（行为层）。
//
// 左栏 git 视图经 [`mdc_dock::Dock`] 激活协议接管（git_view 布尔退役）。
// 激活焦点 = 面板根句柄（蓝本 dock.rs:44 缺省口径——本面板的
// `panel_focus` 即键位域根）。
// ---------------------------------------------------------------------------

/// #119：git 面板的 dock 身份键（左栏激活协议）。发布后不可改
/// （布局序列化依赖——蓝本 dock.rs:37 警告同款）。
pub const GIT_PANEL_KEY: &str = "git";

impl gpui::EventEmitter<mdc_dock::PanelEvent> for GitPanel {}

impl gpui::Focusable for GitPanel {
    fn focus_handle(&self, _cx: &gpui::App) -> gpui::FocusHandle {
        self.panel_focus.clone()
    }
}

impl mdc_dock::Panel for GitPanel {
    fn persistent_name() -> &'static str {
        GIT_PANEL_KEY
    }

    fn panel_key() -> &'static str {
        GIT_PANEL_KEY
    }

    fn position(&self) -> mdc_dock::DockPosition {
        mdc_dock::DockPosition::Left
    }

    fn position_is_valid(&self, position: mdc_dock::DockPosition) -> bool {
        position == mdc_dock::DockPosition::Left
    }
}
// ---------------------------------------------------------------------------
// #575/#577 键盘导航无头测试（lib 树——CI 可见）。焦点面用合成行
// （零 git 依赖）；激活面用临时真仓库（git init + 提交 + 改动——
// git 缺席时如实跳过并在输出登记，不假绿）。
// ---------------------------------------------------------------------------
#[cfg(test)]
mod keyboard_tests {
    use super::*;
    // #588 拆分后纯函数层的 Command 随迁 git_repo——键盘测试的
    // 「临时真仓库」夹具（git init/提交/改动）仍需直接建仓。
    use std::process::Command;
    use std::sync::Mutex;

    /// 测试钩子（open_readonly 捕获标题序列；escape no-op）。
    fn capturing_hooks(opened: Arc<Mutex<Vec<String>>>) -> GitPanelHooks {
        GitPanelHooks {
            open_readonly: Arc::new(move |title, _, _, _| opened.lock().unwrap().push(title)),
            escape: Arc::new(|_, _| {}),
        }
    }

    struct Host(gpui::Entity<GitPanel>);

    impl gpui::Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
            v_flex().size_full().child(self.0.clone())
        }
    }

    /// 合成行面板 + 宿主窗口（非 git 临时目录构造 → unavailable →
    /// 注入合成行——渲染/键盘链与真实行无差）。
    fn synth_panel_window(
        cx: &mut gpui::TestAppContext,
    ) -> (
        gpui::Entity<GitPanel>,
        gpui::VisualTestContext,
        Arc<Mutex<Vec<String>>>,
    ) {
        cx.update(|app| {
            theme::init(theme::LoadThemes::JustBase, app);
            theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
        });
        let dir = std::env::temp_dir().join(format!("mang-git-kb-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let opened: Arc<Mutex<Vec<String>>> = Arc::default();
        let hooks = capturing_hooks(opened.clone());
        let panel = cx.update(|cx| {
            cx.new(|cx| {
                let mut p = GitPanel::new(dir, hooks, cx);
                p.unavailable = None;
                p.changes = vec![
                    GitChange {
                        code: "M".into(),
                        path: "src/main.rs".into(),
                    },
                    GitChange {
                        code: "??".into(),
                        path: "notes.md".into(),
                    },
                ];
                p.commits = vec![GitCommit {
                    hash: "abc1234".into(),
                    time: "2026-11-08".into(),
                    subject: "初始提交".into(),
                }];
                p
            })
        });
        let window = cx.open_window(gpui::size(gpui::px(260.), gpui::px(400.)), {
            let panel = panel.clone();
            move |_, _| Host(panel)
        });
        cx.run_until_parked();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        (panel, visual, opened)
    }

    /// 焦点面：↑↓ 移动（改动视图，Navigable 遍历）+ 末界夹取。
    #[gpui::test]
    fn focus_moves_through_changes_rows(cx: &mut gpui::TestAppContext) {
        let (panel, mut visual, _opened) = synth_panel_window(cx);

        let (first, second) = panel.read_with(&visual, |p, _| {
            (
                p.change_handles["src/main.rs"].clone(),
                p.change_handles["notes.md"].clone(),
            )
        });
        visual.update(|window, cx| first.focus(window, cx));
        visual.dispatch_action(menu::SelectNext);
        visual.update(|window, cx| {
            assert!(
                second.contains_focused(window, cx),
                "SelectNext → 次行含焦点（Navigable 遍历）"
            );
        });
        // Navigable 组件语义（navigable.rs SelectNext 的
        // unwrap_or(0)）：末行再 ↓ **回卷首行**（环绕遍历）——按
        // 组件真实行为断言，非自设夹取语义。
        visual.dispatch_action(menu::SelectNext);
        visual.update(|window, cx| {
            assert!(
                first.contains_focused(window, cx),
                "末行再 ↓ 回卷首行（Navigable 环绕语义）"
            );
        });
    }

    /// 激活面（真实仓库）：Enter 改动行 → open_readonly 钩子携
    /// diff 标题触发。git 缺席环境如实跳过（输出登记，不假绿断言）。
    #[gpui::test]
    fn confirm_activates_focused_change_in_real_repo(cx: &mut gpui::TestAppContext) {
        if !git_available() {
            eprintln!("git 缺席——confirm 激活测跳过（环境登记）");
            return;
        }
        // 临时真仓库：init + 提交 + 改动（git_status/diff 全链可用）。
        let dir = std::env::temp_dir().join(format!("mang-git-act-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .expect("git 进程")
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        git(&["add", "."]);
        git(&["commit", "-qm", "init"]);
        std::fs::write(dir.join("src/main.rs"), "fn main() { changed }\n").unwrap();

        cx.update(|app| {
            theme::init(theme::LoadThemes::JustBase, app);
            theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
        });
        let opened: Arc<Mutex<Vec<String>>> = Arc::default();
        let hooks = capturing_hooks(opened.clone());
        let panel = cx.update(|cx| {
            cx.new(|cx| {
                let mut p = GitPanel::new(dir, hooks, cx);
                // 构造内 refresh 已双拉（changes = 1 条 M src/main.rs）。
                assert_eq!(p.changes.len(), 1, "真仓库改动行 = 1");
                p
            })
        });
        let window = cx.open_window(gpui::size(gpui::px(260.), gpui::px(400.)), {
            let panel = panel.clone();
            move |_, _| Host(panel)
        });
        cx.run_until_parked();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));

        let handle = panel.read_with(&visual, |p, _| p.change_handles["src/main.rs"].clone());
        visual.update(|window, cx| handle.focus(window, cx));
        visual.dispatch_action(menu::Confirm);
        assert_eq!(
            opened.lock().unwrap().as_slice(),
            ["diff: src/main.rs"],
            "Confirm 改动行 → open_diff → 钩子携 diff 标题触发"
        );
        let _ = std::fs::remove_dir_all(
            std::env::temp_dir().join(format!("mang-git-act-{}", std::process::id())),
        );
    }

    /// 双视图长列表走同一真实 Navigable/下一帧布局链：只移动焦点不算滚动成功。
    fn assert_long_view_navigation_scrolls(
        cx: &mut gpui::TestAppContext,
        view: GitView,
        target_selector: &'static str,
    ) {
        let (panel, mut visual, _) = synth_panel_window(cx);
        panel.update(&mut visual, |p, cx| {
            p.changes = (0..64)
                .map(|i| GitChange {
                    code: "M".into(),
                    path: format!("src/row-{i:02}.rs"),
                })
                .collect();
            p.commits = (0..64)
                .map(|i| GitCommit {
                    hash: format!("c{i:06}"),
                    subject: format!("Commit {i}"),
                    time: "1 hour ago".into(),
                })
                .collect();
            p.view = view;
            cx.notify();
        });
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let (first, target) = panel.read_with(&visual, |p, _| match view {
            GitView::Changes => (
                p.change_handles["src/row-00.rs"].clone(),
                p.change_handles["src/row-35.rs"].clone(),
            ),
            GitView::History => (
                p.commit_handles["c000000"].clone(),
                p.commit_handles["c000035"].clone(),
            ),
        });
        let viewport = panel.read_with(&visual, |p, _| p.scroll.bounds());
        assert!(viewport.size.height > gpui::px(0.), "滚动体必须有实际高度");
        assert!(
            viewport.bottom() <= gpui::px(400.),
            "滚动体必须受窗口高度约束：{viewport:?}"
        );
        if let Some(before) = visual.debug_bounds(target_selector) {
            assert!(
                !viewport.contains(&before.origin),
                "前置：第36行必须超出初始viewport"
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
        let (viewport, offset) =
            panel.read_with(&visual, |p, _| (p.scroll.bounds(), p.scroll.offset()));
        let after = visual
            .debug_bounds(target_selector)
            .expect("目标行必须真实渲染并进入viewport");
        assert!(
            offset.y < gpui::px(0.),
            "必须改变实际滚动offset，不能只移动焦点：{offset:?}"
        );
        assert!(
            viewport.contains(&after.origin),
            "目标行起点必须入viewport：{after:?} / {viewport:?}"
        );
        assert!(
            after.bottom() <= viewport.bottom(),
            "目标行完整高度必须可见"
        );

        // 原 Navigable 环绕/反向链也必须使前一行可见，而非回到默认原点。
        visual.dispatch_action(menu::SelectPrevious);
        visual.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        let previous = panel.read_with(&visual, |p, _| match view {
            GitView::Changes => p.change_handles["src/row-34.rs"].clone(),
            GitView::History => p.commit_handles["c000034"].clone(),
        });
        visual.update(|window, cx| assert!(previous.contains_focused(window, cx)));
        let selector = match view {
            GitView::Changes => "git-chg-row-src/row-34.rs",
            GitView::History => "git-cmt-row-c000034",
        };
        let previous_bounds = visual.debug_bounds(selector).expect("反向导航行真实渲染");
        let viewport = panel.read_with(&visual, |p, _| p.scroll.bounds());
        assert!(viewport.contains(&previous_bounds.origin));
        assert!(previous_bounds.bottom() <= viewport.bottom());
    }

    #[gpui::test]
    fn long_changes_navigation_scrolls_target_into_view(cx: &mut gpui::TestAppContext) {
        assert_long_view_navigation_scrolls(cx, GitView::Changes, "git-chg-row-src/row-35.rs");
    }

    #[gpui::test]
    fn long_history_navigation_scrolls_target_into_view(cx: &mut gpui::TestAppContext) {
        assert_long_view_navigation_scrolls(cx, GitView::History, "git-cmt-row-c000035");
    }

    /// 注入的只读标签焦点必须挂在真实宿主树中，才能检查点击最终焦点。
    struct ClickHost {
        panel: gpui::Entity<GitPanel>,
        opened_focus: FocusHandle,
    }

    impl Render for ClickHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            v_flex().size_full().child(self.panel.clone()).child(
                div()
                    .id("git-opened-readonly-target")
                    .w_full()
                    .h(gpui::px(1.))
                    .flex_shrink_0()
                    .track_focus(&self.opened_focus),
            )
        }
    }

    /// 实际 mouse down/up 冒泡 + 真 Git 命令 + hook 落焦，不只断言标题/次数。
    fn assert_mouse_open_preserves_hook_focus(cx: &mut gpui::TestAppContext, view: GitView) {
        assert!(git_available(), "真 Git 点击回归需要 git");
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("mang-git-click-{}-{unique}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("probe.txt"), "original\n").unwrap();
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .expect("git 进程");
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "click-test@example.invalid"]);
        git(&["config", "user.name", "Git click test"]);
        git(&["add", "probe.txt"]);
        git(&["commit", "-qm", "Initial"]);
        std::fs::write(dir.join("probe.txt"), "changed\n").unwrap();

        cx.update(|app| {
            theme::init(theme::LoadThemes::JustBase, app);
            theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
        });
        let opened_focus = cx.update(|app| app.focus_handle());
        let opened: Arc<Mutex<Vec<String>>> = Arc::default();
        let hook_focus = opened_focus.clone();
        let captured = opened.clone();
        let panel = cx.update(|app| {
            app.new(|cx| {
                let mut panel = GitPanel::new(
                    dir.clone(),
                    GitPanelHooks {
                        open_readonly: Arc::new(move |title, _, window, cx| {
                            captured.lock().unwrap().push(title);
                            hook_focus.focus(window, cx);
                        }),
                        escape: Arc::new(|_, _| panic!("unexpected Escape in click test")),
                    },
                    cx,
                );
                panel.view = view;
                assert_eq!(panel.changes.len(), 1, "真仓库含一条改动");
                assert_eq!(panel.commits.len(), 1, "真仓库含一条提交");
                panel
            })
        });
        let window = cx.open_window(gpui::size(gpui::px(260.), gpui::px(400.)), {
            let panel = panel.clone();
            let opened_focus = opened_focus.clone();
            move |_, _| ClickHost {
                panel,
                opened_focus,
            }
        });
        cx.run_until_parked();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let (row_focus, selector, expected_title) = panel.read_with(&visual, |p, _| match view {
            GitView::Changes => (
                p.change_handles["probe.txt"].clone(),
                "git-chg-row-probe.txt".to_string(),
                "diff: probe.txt".to_string(),
            ),
            GitView::History => {
                let hash = &p.commits[0].hash;
                (
                    p.commit_handles[hash].clone(),
                    format!("git-cmt-row-{hash}"),
                    format!("提交 {hash}"),
                )
            }
        });
        // GPUI debug_bounds 要求 static selector；仅测试进程保留真实提交的短哈希键。
        let selector: &'static str = Box::leak(selector.into_boxed_str());
        let bounds = visual.debug_bounds(selector).expect("真实 Git 行布局");
        visual.simulate_click(bounds.center(), gpui::Modifiers::default());
        visual.update(|window, cx| {
            assert!(
                opened_focus.contains_focused(window, cx),
                "点击打开后最终焦点必须留在注入的只读标签目标"
            );
            assert!(
                !row_focus.contains_focused(window, cx),
                "父行 click 冒泡不得把焦点抢回 Git 行"
            );
        });
        assert_eq!(*opened.lock().unwrap(), vec![expected_title]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[gpui::test]
    fn clicking_change_keeps_focus_on_opened_readonly_target(cx: &mut gpui::TestAppContext) {
        assert_mouse_open_preserves_hook_focus(cx, GitView::Changes);
    }

    #[gpui::test]
    fn clicking_commit_keeps_focus_on_opened_readonly_target(cx: &mut gpui::TestAppContext) {
        assert_mouse_open_preserves_hook_focus(cx, GitView::History);
    }

    /// 句柄差量：键存续复用 / 离场清退（prune_handles 纯状态面）。
    #[gpui::test]
    fn prune_handles_drops_departed_rows(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join("mang-git-prune");
        let _ = std::fs::create_dir_all(&dir);
        let panel = cx.update(|cx| {
            cx.new(|cx| {
                let mut p = GitPanel::new(
                    dir,
                    GitPanelHooks {
                        open_readonly: Arc::new(|_, _, _, _| {}),
                        escape: Arc::new(|_, _| {}),
                    },
                    cx,
                );
                p.unavailable = None;
                p.changes = vec![
                    GitChange {
                        code: "M".into(),
                        path: "a.rs".into(),
                    },
                    GitChange {
                        code: "D".into(),
                        path: "b.rs".into(),
                    },
                ];
                p
            })
        });
        // 句柄预建（模拟渲染面）。
        panel.update(cx, |p, cx| {
            for c in &p.changes {
                p.change_handles
                    .entry(c.path.clone())
                    .or_insert_with(|| cx.focus_handle());
            }
        });
        let a_handle = panel.read_with(cx, |p, _| p.change_handles["a.rs"].clone());
        // b.rs 离场（改动只剩 a.rs）→ prune 清退。
        panel.update(cx, |p, _| {
            p.changes.retain(|c| c.path == "a.rs");
            p.prune_handles();
        });
        panel.read_with(cx, |p, _| {
            assert!(p.change_handles.contains_key("a.rs"), "键存续行句柄保留");
            assert!(!p.change_handles.contains_key("b.rs"), "离场行句柄清退");
            assert_eq!(
                p.change_handles["a.rs"], a_handle,
                "存续行句柄身份不变（刷新后焦点稳定）"
            );
        });
    }

    // --------------------------------------------------------------
    // #47 批次 6（E-2 禁用态）：刷新钮禁用行为——不可用态点击冒泡零
    // 状态变化；可用态点击真实拉取磁盘新改动。GUI 像素面由
    // --smoke-disabled-state 双面断言（theme_visual_smoke.sh），本组为
    // 头无单测面（debug_bounds 锚 + simulate_click 全事件管线）。
    // --------------------------------------------------------------

    /// 构造指定根的 GitPanel 并在测试窗口中渲染；返回 (visual, panel,
    /// 刷新钮 bounds)。夹具卫生：temp_dir + 首尾自清（#47 批次 4 范式）。
    fn refresh_host(
        cx: &mut gpui::TestAppContext,
        root: std::path::PathBuf,
    ) -> (
        gpui::VisualTestContext,
        gpui::Entity<GitPanel>,
        gpui::Bounds<gpui::Pixels>,
    ) {
        cx.update(|app| {
            theme::init(theme::LoadThemes::JustBase, app);
            theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
        });
        let opened_focus = cx.update(|app| app.focus_handle());
        let panel = cx.update(|app| {
            app.new(|cx| {
                GitPanel::new(
                    root,
                    GitPanelHooks {
                        open_readonly: Arc::new(|_, _, _, _| {}),
                        escape: Arc::new(|_, _| {}),
                    },
                    cx,
                )
            })
        });
        let window = cx.open_window(gpui::size(gpui::px(260.), gpui::px(400.)), {
            let panel = panel.clone();
            let opened_focus = opened_focus.clone();
            move |_, _| ClickHost {
                panel,
                opened_focus,
            }
        });
        cx.run_until_parked();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let bounds = visual
            .debug_bounds("git-refresh")
            .expect("刷新钮渲染在场（debug_selector 锚）");
        (visual, panel, bounds)
    }

    #[gpui::test]
    fn refresh_click_is_inert_when_root_not_a_git_repo(cx: &mut gpui::TestAppContext) {
        // 非 git 根（夹具卫生：temp_dir + 自清）。
        let dir =
            std::env::temp_dir().join(format!("mang-git-refresh-disabled-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("普通.md"), "非 git 根\n").unwrap();

        let (mut visual, panel, bounds) = refresh_host(cx, dir.clone());
        panel.read_with(&visual, |p, _| {
            let reason = p.unavailable.as_ref().expect("非 git 根 → 不可用态");
            assert!(reason.contains("非 git"), "不可用文案语义: {reason}");
        });
        // 禁用钮点击：冒泡到无监听元素——状态零变化。
        visual.simulate_click(bounds.center(), gpui::Modifiers::default());
        panel.read_with(&visual, |p, _| {
            let reason = p.unavailable.as_ref().expect("点击后仍不可用态");
            assert!(reason.contains("非 git"), "禁用点击不改文案: {reason}");
            assert!(p.changes.is_empty(), "禁用点击不产生改动");
            assert!(p.commits.is_empty(), "禁用点击不产生提交");
        });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[gpui::test]
    fn refresh_click_pulls_disk_changes_when_available(cx: &mut gpui::TestAppContext) {
        assert!(git_available(), "真 Git 刷新回归需要 git");
        let dir =
            std::env::temp_dir().join(format!("mang-git-refresh-enabled-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("probe.txt"), "original\n").unwrap();
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .expect("git 进程");
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "refresh-test@example.invalid"]);
        git(&["config", "user.name", "Refresh test"]);
        git(&["add", "probe.txt"]);
        git(&["commit", "-qm", "Initial"]);

        let (mut visual, panel, bounds) = refresh_host(cx, dir.clone());
        panel.read_with(&visual, |p, _| {
            assert!(p.unavailable.is_none(), "git 根可用");
            assert!(p.changes.is_empty(), "构造期干净");
        });
        // 磁盘弄脏 → 点击刷新 → 真实拉取（点击冒泡 + refresh 同步）。
        std::fs::write(dir.join("probe.txt"), "changed\n").unwrap();
        visual.simulate_click(bounds.center(), gpui::Modifiers::default());
        panel.read_with(&visual, |p, _| {
            assert!(p.unavailable.is_none());
            assert_eq!(p.changes.len(), 1, "刷新拉到磁盘新改动");
            assert_eq!(p.changes[0].path, "probe.txt");
        });
        let _ = std::fs::remove_dir_all(&dir);
    }
}
