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
        assert!(shell.read_with(&visual, |shell, _| shell.git_view && shell.left_open));
        let file = visual.debug_bounds("act-file-tree").unwrap();
        visual.simulate_click(file.center(), Default::default());
        assert!(shell.read_with(&visual, |shell, _| !shell.git_view && shell.left_open));
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
