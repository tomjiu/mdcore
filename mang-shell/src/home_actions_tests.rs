//! Home actions: real GPUI input -> native prompt simulation -> disk / Shell state.
use super::*;
use gpui::{TestAppContext, VisualTestContext};

struct Host(Entity<ShellApp>);
impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.0.clone())
    }
}

struct Fixture(PathBuf);
impl Fixture {
    fn new(cx: &TestAppContext) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "mang-home-{}-{}",
            std::process::id(),
            cx.test_function_name().unwrap()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn setup(cx: &mut TestAppContext) -> (Entity<ShellApp>, gpui::WindowHandle<Host>) {
    cx.update(|cx| {
        theme::init(theme::LoadThemes::JustBase, cx);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), cx);
    });
    let mut shell = None;
    let window = cx.add_window(|window, cx| {
        let entity = cx.new(|cx| ShellApp::new(None, None, window, cx));
        entity.update(cx, |shell, _| {
            shell.recent = RecentStore::default();
        });
        shell = Some(entity.clone());
        Host(entity)
    });
    cx.run_until_parked();
    (shell.unwrap(), window)
}

fn draw(cx: &mut TestAppContext, window: gpui::WindowHandle<Host>) {
    cx.update_window(window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .unwrap();
}

fn click(cx: &mut TestAppContext, window: gpui::WindowHandle<Host>, selector: &'static str) {
    draw(cx, window);
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    // On the unfixed baseline the button does not exist. Still dispatch a real
    // click into the home area, then assert the actual prompt, not source text.
    let point = visual
        .debug_bounds(selector)
        .map(|b| b.center())
        .unwrap_or_else(|| gpui::point(px(600.), px(300.)));
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_event(
            gpui::PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position: point,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        window.dispatch_event(
            gpui::PlatformInput::MouseUp(MouseUpEvent {
                button: MouseButton::Left,
                position: point,
                modifiers: Default::default(),
                click_count: 1,
            }),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
}

fn key(cx: &mut TestAppContext, window: gpui::WindowHandle<Host>, key: &'static str) {
    draw(cx, window);
    cx.update_window(window.into(), |_, window, cx| {
        let keystroke = gpui::Keystroke::parse(key).unwrap();
        window.dispatch_event(
            gpui::PlatformInput::KeyDown(KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            }),
            cx,
        );
        window.dispatch_event(
            gpui::PlatformInput::KeyUp(gpui::KeyUpEvent { keystroke }),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui::test]
fn home_click_open_file_prompt_and_content(cx: &mut TestAppContext) {
    let dir = Fixture::new(cx);
    let path = dir.0.join("打开.md");
    std::fs::write(&path, "真实打开内容\n").unwrap();
    let (shell, window) = setup(cx);
    click(cx, window, "home-open-file");
    assert!(
        cx.did_prompt_for_paths(),
        "home file click must request native picker"
    );
    cx.simulate_path_prompt_response(|o| {
        assert!(o.files && !o.directories && !o.multiple);
        Some(vec![path.clone()])
    });
    cx.run_until_parked();
    cx.read(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.tab_count(), 1);
        assert_eq!(s.tab_path(0).unwrap(), path.as_path());
        assert_eq!(
            s.tab_editor(0).unwrap().read(cx).buffer_line(0),
            "真实打开内容"
        );
        assert_eq!(s.recent.recent_files()[0], path);
    });
    window
        .update(cx, |_, window, cx| {
            assert!(
                shell
                    .read(cx)
                    .tab_editor(0)
                    .unwrap()
                    .read(cx)
                    .focus_handle()
                    .is_focused(window)
            );
        })
        .unwrap();
}

#[gpui::test]
fn home_click_open_folder_prompt_and_tree(cx: &mut TestAppContext) {
    let dir = Fixture::new(cx);
    std::fs::write(dir.0.join("tree.md"), "tree").unwrap();
    let (shell, window) = setup(cx);
    click(cx, window, "home-open-folder");
    assert!(
        cx.did_prompt_for_paths(),
        "home folder click must request native picker"
    );
    cx.simulate_path_prompt_response(|o| {
        assert!(!o.files && o.directories && !o.multiple);
        Some(vec![dir.0.clone()])
    });
    cx.run_until_parked();
    cx.read(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.workspace.as_ref().unwrap().read(cx).root(),
            dir.0.as_path()
        );
        assert!(s.git_panel.is_some());
        assert_eq!(s.tab_count(), 0);
        assert_eq!(s.recent.recent_workspaces()[0], dir.0);
    });
}

#[gpui::test]
fn home_click_new_markdown_creates_opens_and_reopens(cx: &mut TestAppContext) {
    let dir = Fixture::new(cx);
    let path = dir.0.join("新文档.MD");
    let (shell, window) = setup(cx);
    click(cx, window, "home-new-markdown");
    assert!(
        cx.did_prompt_for_new_path(),
        "home new click must request save location"
    );
    cx.simulate_new_path_selection(|_| Some(path.clone()));
    cx.run_until_parked();
    assert_eq!(std::fs::read(&path).unwrap(), b"");
    window
        .update(cx, |_, window, cx| {
            let s = shell.read(cx);
            assert_eq!(s.tab_count(), 1);
            assert_eq!(s.tab_path(0).unwrap(), path.as_path());
            assert!(
                s.tab_editor(0)
                    .unwrap()
                    .read(cx)
                    .focus_handle()
                    .is_focused(window)
            );
        })
        .unwrap();
    // Actual platform text -> dirty -> Ctrl+S -> byte assertion -> close/reopen.
    draw(cx, window);
    cx.dispatch_keystroke(window.into(), gpui::Keystroke::parse("x").unwrap());
    cx.run_until_parked();
    cx.read(|cx| assert_eq!(shell.read(cx).tab_is_modified(0, cx), Some(true)));
    key(cx, window, "ctrl-s");
    assert_eq!(std::fs::read(&path).unwrap(), b"x");
    cx.read(|cx| assert_eq!(shell.read(cx).tab_is_modified(0, cx), Some(false)));
    // Reopen the disk file through the production document dispatcher.
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |s, cx| {
                s.close_tab_no_confirm(0, window, cx);
                s.open_document(path.clone(), window, cx);
            });
        })
        .unwrap();
    cx.read(|cx| {
        assert_eq!(
            shell
                .read(cx)
                .tab_editor(0)
                .unwrap()
                .read(cx)
                .buffer_line(0),
            "x"
        )
    });
}

#[gpui::test]
fn home_cancellation_preserves_workspace_and_recent(cx: &mut TestAppContext) {
    let dir = Fixture::new(cx);
    let (shell, window) = setup(cx);
    shell.update(cx, |s, cx| s.open_workspace(dir.0.clone(), cx));
    let before = cx.read(|cx| shell.read(cx).recent.clone());
    for selector in ["home-open-file", "home-open-folder", "home-new-markdown"] {
        click(cx, window, selector);
        if selector == "home-new-markdown" {
            assert!(cx.did_prompt_for_new_path());
            cx.simulate_new_path_selection(|initial| {
                assert_eq!(initial, dir.0);
                None
            });
        } else {
            assert!(cx.did_prompt_for_paths());
            cx.simulate_path_prompt_response(|_| None);
        }
        cx.run_until_parked();
        cx.read(|cx| {
            let s = shell.read(cx);
            assert_eq!(s.tab_count(), 0);
            assert_eq!(
                s.workspace.as_ref().unwrap().read(cx).root(),
                dir.0.as_path()
            );
            assert_eq!(s.recent, before);
            assert!(s.notice_board.message().is_none(), "cancel is silent no-op");
        });
    }
}

#[gpui::test]
fn home_new_markdown_racing_existing_file_never_overwrites(cx: &mut TestAppContext) {
    let dir = Fixture::new(cx);
    let path = dir.0.join("race.md");
    let (shell, window) = setup(cx);
    click(cx, window, "home-new-markdown");
    assert!(cx.did_prompt_for_new_path());
    // Another process wins while the picker is outstanding. No exists-check
    // may authorize a later truncate; create_new must reject atomically.
    std::fs::write(&path, b"OTHER PROCESS CONTENT").unwrap();
    cx.simulate_new_path_selection(|_| Some(path.clone()));
    cx.run_until_parked();
    assert_eq!(std::fs::read(&path).unwrap(), b"OTHER PROCESS CONTENT");
    cx.read(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.tab_count(), 0);
        assert!(s.recent.recent_files().is_empty());
        assert!(
            s.notice_board
                .message()
                .unwrap()
                .contains("新建 Markdown 失败")
        );
    });
}

#[gpui::test]
fn home_new_markdown_invalid_directory_and_extension_report_failure(cx: &mut TestAppContext) {
    let dir = Fixture::new(cx);
    let (shell, window) = setup(cx);
    for path in [
        dir.0.join("absent").join("file.md"),
        dir.0.join("wrong.pdf"),
        dir.0.join("directory.md"),
    ] {
        if path.file_name().unwrap() == "directory.md" {
            std::fs::create_dir(&path).unwrap();
        }
        click(cx, window, "home-new-markdown");
        assert!(cx.did_prompt_for_new_path());
        cx.simulate_new_path_selection(|_| Some(path.clone()));
        cx.run_until_parked();
        cx.read(|cx| {
            let s = shell.read(cx);
            assert_eq!(s.tab_count(), 0);
            assert!(
                s.notice_board.message().is_some(),
                "failure must be visible"
            );
            assert!(s.recent.recent_files().is_empty());
        });
        assert!(!path.is_file());
    }
}

#[gpui::test]
fn home_open_missing_file_reports_failure_and_empty_selection_is_noop(cx: &mut TestAppContext) {
    let dir = Fixture::new(cx);
    let (shell, window) = setup(cx);
    click(cx, window, "home-open-file");
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| Some(vec![]));
    cx.run_until_parked();
    cx.read(|cx| {
        assert_eq!(shell.read(cx).tab_count(), 0);
        assert!(shell.read(cx).notice_board.message().is_none());
    });
    click(cx, window, "home-open-file");
    cx.simulate_path_prompt_response(|_| Some(vec![dir.0.join("missing.md")]));
    cx.run_until_parked();
    cx.read(|cx| {
        assert_eq!(shell.read(cx).tab_count(), 0);
        assert!(shell.read(cx).notice_board.message().is_some());
        assert!(shell.read(cx).recent.recent_files().is_empty());
    });
}

#[gpui::test]
fn home_folder_disappearing_during_picker_preserves_workspace(cx: &mut TestAppContext) {
    let dir = Fixture::new(cx);
    let (shell, window) = setup(cx);
    shell.update(cx, |s, cx| s.open_workspace(dir.0.clone(), cx));
    let before = cx.read(|cx| shell.read(cx).recent.clone());
    click(cx, window, "home-open-folder");
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| Some(vec![dir.0.join("gone")]));
    cx.run_until_parked();
    cx.read(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.workspace.as_ref().unwrap().read(cx).root(),
            dir.0.as_path()
        );
        assert_eq!(s.recent, before);
        assert_eq!(s.tab_count(), 0);
        assert!(
            s.notice_board.message().is_some(),
            "invalid folder must be visible failure, not empty tree success"
        );
    });
}

#[gpui::test]
fn home_new_without_extension_adds_md_and_recent_survives_reload(cx: &mut TestAppContext) {
    let dir = Fixture::new(cx);
    let path = dir.0.join("无扩展名");
    let expected = path.with_extension("md");
    let (shell, window) = setup(cx);
    click(cx, window, "home-new-markdown");
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| Some(path.clone()));
    cx.run_until_parked();
    assert!(expected.is_file());
    assert!(!path.exists());
    cx.read(|cx| {
        assert_eq!(shell.read(cx).tab_path(0).unwrap(), expected.as_path());
        assert_eq!(shell.read(cx).recent.recent_files()[0], expected);
    });
    if crate::settings::config_dir().is_some() {
        assert_eq!(
            RecentStore::load().recent_files()[0],
            expected,
            "recent JSON reload preserves created file"
        );
    } else {
        eprintln!(
            "SKIP: home_recent_disk_reload; reason: ENV_MISSING: XDG_CONFIG_HOME/HOME; set isolated XDG_CONFIG_HOME to verify recent JSON reload (file creation/open assertions still ran)"
        );
    }
}

#[gpui::test]
fn home_keyboard_tab_enter_space_use_same_prompts(cx: &mut TestAppContext) {
    let (shell, window) = setup(cx);
    key(cx, window, "tab");
    window
        .update(cx, |_, window, cx| {
            assert!(shell.read(cx).home_focus[0].is_focused(window));
        })
        .unwrap();
    key(cx, window, "enter");
    assert!(
        cx.did_prompt_for_paths(),
        "Tab -> file -> Enter must activate native picker"
    );
    cx.simulate_path_prompt_response(|o| {
        assert!(o.files);
        None
    });
    cx.run_until_parked();
    key(cx, window, "tab");
    key(cx, window, "space");
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|o| {
        assert!(o.directories);
        None
    });
    cx.run_until_parked();
    key(cx, window, "tab");
    key(cx, window, "enter");
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    key(cx, window, "shift-tab");
    window
        .update(cx, |_, window, cx| {
            assert!(
                shell.read(cx).home_focus[1].is_focused(window),
                "Shift+Tab moves back to folder"
            );
        })
        .unwrap();
    key(cx, window, "shift-tab");
    key(cx, window, "shift-tab");
    window
        .update(cx, |_, window, cx| {
            assert!(
                shell.read(cx).home_focus[2].is_focused(window),
                "reverse traversal wraps to new Markdown"
            );
        })
        .unwrap();
}
