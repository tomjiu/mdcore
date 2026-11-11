use super::*;
use gpui::{TestAppContext, VisualTestContext, size};

struct Host(Entity<ShellApp>);

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.0.clone())
    }
}

fn host(cx: &mut TestAppContext) -> (Entity<ShellApp>, VisualTestContext) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let editor = cx.update(|app| {
        app.new(|cx| Editor::new(Some("layout.md".into()), "# Layout\n\ntext\n".into(), cx))
    });
    let mut shell_slot = None;
    let window = cx.open_window(size(px(1280.), px(800.)), |window, app| {
        let shell = app.new(|cx| ShellApp::new(Some(editor), None, window, cx));
        shell_slot = Some(shell.clone());
        Host(shell)
    });
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, app| window.draw(app).clear(app));
    (shell_slot.unwrap(), visual)
}

#[gpui::test]
fn tabs_start_at_editor_column_and_never_above_explorer(cx: &mut TestAppContext) {
    let (_shell, mut visual) = host(cx);
    for width in [1280., 900.] {
        visual.simulate_resize(size(px(width), px(800.)));
        visual.update(|window, app| window.draw(app).clear(app));
        let tabs = visual
            .debug_bounds("editor-tabs")
            .expect("document tabs are rendered");
        let editor = visual
            .debug_bounds("editor-content")
            .expect("editor is rendered");
        let left = visual
            .debug_bounds("left-panel")
            .expect("explorer is rendered");
        assert_eq!(
            tabs.origin.x, editor.origin.x,
            "tabs belong to the editor column"
        );
        assert!(
            tabs.origin.x >= left.origin.x + left.size.width,
            "tabs must not cross the explorer"
        );
        assert_eq!(
            tabs.origin.y, left.origin.y,
            "explorer starts beside, rather than below, tabs"
        );
        assert!(
            editor.size.width >= px(240.),
            "editor retains a usable width"
        );
    }
}

#[gpui::test]
fn git_and_menu_controls_keep_visible_bounds_with_document_open(cx: &mut TestAppContext) {
    let (shell, mut visual) = host(cx);
    for width in [1280., 900.] {
        visual.simulate_resize(size(px(width), px(800.)));
        visual.update(|window, app| window.draw(app).clear(app));
        let title = visual.debug_bounds("title-bar").unwrap();
        let menus = visual.debug_bounds("menu-bar-titles").unwrap();
        assert!(menus.size.width > px(120.) && menus.size.height > px(20.));
        assert!(menus.origin.x >= title.origin.x && menus.origin.y >= title.origin.y);
        let git = visual.debug_bounds("act-git").unwrap();
        assert_eq!(git.size, size(px(36.), px(36.)));
        visual.simulate_click(git.center(), Default::default());
        assert!(shell.read_with(&visual, |shell, cx| shell.git_active(cx) && shell.left_open));
        let file = visual.debug_bounds("act-file-tree").unwrap();
        visual.simulate_click(file.center(), Default::default());
        assert!(shell.read_with(&visual, |shell, cx| !shell.git_active(cx)
            && shell.left_open));
    }
}

#[gpui::test]
fn right_dock_navigation_and_document_information_have_separate_lifetimes(cx: &mut TestAppContext) {
    let (shell, mut visual) = host(cx);
    assert!(
        visual.debug_bounds("right-panel").is_some(),
        "right document dock remains available"
    );
    assert!(
        visual.debug_bounds("document-info").is_none(),
        "metadata is not an always-visible dock"
    );
    let controls = visual
        .debug_bounds("document-info-toggle")
        .expect("explicit details action exists");
    visual.simulate_click(controls.center(), Default::default());
    visual.update(|window, app| window.draw(app).clear(app));
    assert!(
        visual.debug_bounds("document-info").is_some(),
        "details open on demand"
    );
    let close = visual.debug_bounds("document-info-close").unwrap();
    visual.simulate_click(close.center(), Default::default());
    visual.update(|window, app| window.draw(app).clear(app));
    assert!(visual.debug_bounds("document-info").is_none());
    assert!(
        shell.read_with(&visual, |shell, _| shell.right_open),
        "closing information does not close document navigation"
    );
}

#[gpui::test]
fn file_tree_open_and_reopen_use_real_shell_without_entity_reentry(cx: &mut TestAppContext) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let root = std::env::temp_dir().join(format!("mang-filetree-shell-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("reentry.md");
    std::fs::write(&path, "# File tree\n\nOpen and reopen.\n").unwrap();
    let mut slot = None;
    let window = cx.open_window(size(px(1280.), px(800.)), |window, app| {
        let shell = app.new(|cx| ShellApp::new(None, Some(root), window, cx));
        slot = Some(shell.clone());
        Host(shell)
    });
    let shell = slot.unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, app| window.draw(app).clear(app));
    for _ in 0..2 {
        let panel = visual
            .debug_bounds("left-panel")
            .expect("real file tree is visible");
        // Root row is followed by the only file in this real temporary workspace.
        visual.simulate_click(
            gpui::point(panel.origin.x + px(80.), panel.origin.y + px(70.)),
            Default::default(),
        );
        cx.run_until_parked();
        visual.update(|window, app| window.draw(app).clear(app));
        shell.read_with(&visual, |shell, _| {
            assert_eq!(shell.tab_count(), 1);
            assert_eq!(shell.tabs[shell.active].path, path);
        });
        visual.update(|window, app| {
            let editor = shell.read(app).editor().unwrap().read(app);
            assert!(editor.focus_handle().contains_focused(window, app));
        });
    }
}

/// A1（#47 批次 2）：右 dock 窄窗让位——窗口宽低于临界值（900px）时右栏
/// 在渲染期让位，编辑区保 ≥240px 可用宽；`right_open` 用户开关态不被
/// 改写，宽度恢复后右栏按原开关态回归。
#[gpui::test]
fn right_dock_yields_below_critical_width_and_returns_when_restored(cx: &mut TestAppContext) {
    let (shell, mut visual) = host(cx);
    // 常宽（1280）：右栏在场（right_open 缺省开）。
    visual.update(|window, app| window.draw(app).clear(app));
    assert!(
        visual.debug_bounds("right-panel").is_some(),
        "precondition: right dock is rendered at normal width"
    );
    // 临界以下（760×480 = 强制的最小窗宽）：右栏让位。
    visual.simulate_resize(size(px(760.), px(480.)));
    visual.update(|window, app| window.draw(app).clear(app));
    assert!(
        visual.debug_bounds("right-panel").is_none(),
        "right dock yields below the critical width"
    );
    let editor = visual
        .debug_bounds("editor-content")
        .expect("editor is rendered");
    assert!(
        editor.size.width >= px(240.),
        "editor keeps a usable width after the dock yields: {:?}",
        editor.size.width
    );
    let tabs = visual
        .debug_bounds("editor-tabs")
        .expect("tabs are rendered");
    let left = visual
        .debug_bounds("left-panel")
        .expect("explorer is intact");
    assert_eq!(
        tabs.origin.x, editor.origin.x,
        "tabs belong to the editor column while yielded"
    );
    assert!(
        tabs.origin.x >= left.origin.x + left.size.width,
        "tabs must not cross the explorer while yielded"
    );
    assert!(
        shell.read_with(&visual, |shell, _| shell.right_open),
        "the user's right_open toggle is not rewritten while yielded"
    );
    // 恢复宽度：右栏按原开关态回归。
    visual.simulate_resize(size(px(1280.), px(800.)));
    visual.update(|window, app| window.draw(app).clear(app));
    assert!(
        visual.debug_bounds("right-panel").is_some(),
        "right dock returns when the width is restored"
    );
}

/// A1（#47 批次 2）：最小窗宽（760×480，main.rs 强制的 window_min_size）
/// 下布局保持——菜单/Git 入口可达、标签在编辑区上沿、编辑区可用。
#[gpui::test]
fn layout_holds_at_enforced_minimum_window_size(cx: &mut TestAppContext) {
    let (_shell, mut visual) = host(cx);
    visual.simulate_resize(size(px(760.), px(480.)));
    visual.update(|window, app| window.draw(app).clear(app));
    let title = visual.debug_bounds("title-bar").unwrap();
    let menus = visual.debug_bounds("menu-bar-titles").unwrap();
    assert!(menus.size.width > px(120.) && menus.size.height > px(20.));
    assert!(menus.origin.x >= title.origin.x && menus.origin.y >= title.origin.y);
    let git = visual.debug_bounds("act-git").unwrap();
    assert_eq!(git.size, size(px(36.), px(36.)));
    let tabs = visual.debug_bounds("editor-tabs").unwrap();
    let editor = visual.debug_bounds("editor-content").unwrap();
    let left = visual.debug_bounds("left-panel").unwrap();
    assert_eq!(
        tabs.origin.x, editor.origin.x,
        "tabs belong to the editor column"
    );
    assert!(tabs.origin.x >= left.origin.x + left.size.width);
    assert!(
        editor.size.width >= px(240.),
        "editor keeps a usable width at the enforced minimum: {:?}",
        editor.size.width
    );
}

#[gpui::test]
fn tab_close_button_closes_document_and_honors_dirty_cancel(cx: &mut TestAppContext) {
    let (shell, mut visual) = host(cx);
    let close = visual
        .debug_bounds("tab-close-0")
        .expect("each document tab has a visible close button");
    visual.update(|_, app| {
        let editor = shell.read(app).editor().unwrap().clone();
        editor.update(app, |editor, cx| {
            editor.insert_text_at(0, 0, "unsaved ", cx)
        });
    });
    visual.update(|window, app| window.draw(app).clear(app));
    visual.simulate_click(close.center(), Default::default());
    assert!(shell.read_with(&visual, |shell, _| shell.confirm.is_some()));
    visual.update(|window, app| {
        shell.update(app, |shell, cx| shell.resolve_confirm(2, window, cx));
    });
    assert_eq!(
        shell.read_with(&visual, |shell, _| shell.tab_count()),
        1,
        "cancel retains the modified tab"
    );
    visual.update(|window, app| window.draw(app).clear(app));
    let close = visual.debug_bounds("tab-close-0").unwrap();
    visual.simulate_click(close.center(), Default::default());
    visual.update(|window, app| {
        shell.update(app, |shell, cx| shell.resolve_confirm(1, window, cx));
    });
    visual.update(|window, app| window.draw(app).clear(app));
    assert_eq!(shell.read_with(&visual, |shell, _| shell.tab_count()), 0);
    assert!(
        visual.debug_bounds("editor-tabs").is_none(),
        "last tab closes back to empty state"
    );
}

/// A5（#47 批次 20）：**高 DPI 缩放因子**下布局保持——`simulate_scale_
/// factor_change`（真实缩放事件——跨显示器拖拽换档的同源管线）：
/// 2.0 档（HiDPI 双倍屏）下核心布局不变量逐项复核（标题/菜单/活动条
/// 在场、标签在编辑区上沿、左栏完整、编辑区可用宽、右栏按开关态在
/// 位）；回落 1.0 档布局与基线逐项一致（换档不残留）。
#[gpui::test]
fn layout_holds_at_hidpi_scale_factor_and_returns(cx: &mut TestAppContext) {
    let (_shell, mut visual) = host(cx);
    // 基线（1.0 档）边界快照。
    visual.update(|window, app| window.draw(app).clear(app));
    let baseline = (
        visual.debug_bounds("title-bar").unwrap(),
        visual.debug_bounds("menu-bar-titles").unwrap(),
        visual.debug_bounds("act-git").unwrap(),
        visual.debug_bounds("left-panel").unwrap(),
        visual.debug_bounds("editor-tabs").unwrap(),
        visual.debug_bounds("editor-content").unwrap(),
        visual.debug_bounds("right-panel").unwrap(),
    );

    // 2.0 档（HiDPI 双倍屏）：缩放事件 + 重绘 → 不变量逐项复核。
    visual.simulate_scale_factor_change(2.0);
    visual.update(|window, app| window.draw(app).clear(app));
    assert!(
        visual.debug_bounds("title-bar").is_some(),
        "HiDPI 档标题栏在场"
    );
    assert!(
        visual.debug_bounds("menu-bar-titles").is_some(),
        "HiDPI 档菜单入口在场（可达性）"
    );
    assert!(
        visual.debug_bounds("act-git").is_some(),
        "HiDPI 档 Git 入口在场（可达性）"
    );
    let left = visual.debug_bounds("left-panel").expect("HiDPI 档左栏完整");
    let tabs = visual
        .debug_bounds("editor-tabs")
        .expect("HiDPI 档标签栏在场");
    let editor = visual
        .debug_bounds("editor-content")
        .expect("HiDPI 档编辑区在场");
    assert_eq!(
        tabs.origin.x, editor.origin.x,
        "HiDPI 档标签仍只在编辑区上沿（列归属不变）"
    );
    assert!(
        tabs.origin.x >= left.origin.x + left.size.width,
        "HiDPI 档标签不越左栏"
    );
    assert!(
        editor.size.width >= px(240.),
        "HiDPI 档编辑区保可用宽：{:?}",
        editor.size.width
    );
    assert!(
        visual.debug_bounds("right-panel").is_some(),
        "HiDPI 档右栏按开关态在位（right_open 缺省开）"
    );

    // 回落 1.0 档：布局与基线逐项一致（换档不残留）。
    visual.simulate_scale_factor_change(1.0);
    visual.update(|window, app| window.draw(app).clear(app));
    let restored = (
        visual.debug_bounds("title-bar").unwrap(),
        visual.debug_bounds("menu-bar-titles").unwrap(),
        visual.debug_bounds("act-git").unwrap(),
        visual.debug_bounds("left-panel").unwrap(),
        visual.debug_bounds("editor-tabs").unwrap(),
        visual.debug_bounds("editor-content").unwrap(),
        visual.debug_bounds("right-panel").unwrap(),
    );
    let same = |name: &str, a: &gpui::Bounds<gpui::Pixels>, b: &gpui::Bounds<gpui::Pixels>| {
        assert_eq!(a.origin, b.origin, "{name} 原点回落一致");
        assert_eq!(a.size, b.size, "{name} 尺寸回落一致");
    };
    let ((t0, m0, g0, l0, tb0, e0, r0), (t1, m1, g1, l1, tb1, e1, r1)) = (baseline, restored);
    same("title-bar", &t0, &t1);
    same("menu-bar-titles", &m0, &m1);
    same("act-git", &g0, &g1);
    same("left-panel", &l0, &l1);
    same("editor-tabs", &tb0, &tb1);
    same("editor-content", &e0, &e1);
    same("right-panel", &r0, &r1);
}
