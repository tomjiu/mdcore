use super::*;

fn redraw(window: &gpui::WindowHandle<Host>, cx: &mut gpui::TestAppContext) {
    cx.update_window((*window).into(), |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
}
fn bounds(
    window: &gpui::WindowHandle<Host>,
    cx: &mut gpui::TestAppContext,
    selector: &'static str,
) -> gpui::Bounds<Pixels> {
    let mut visual = gpui::VisualTestContext::from_window((*window).into(), cx);
    visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing rendered {selector}"))
}
fn click(
    window: &gpui::WindowHandle<Host>,
    cx: &mut gpui::TestAppContext,
    selector: &'static str,
    button: MouseButton,
) {
    redraw(window, cx);
    let position = bounds(window, cx, selector).center();
    cx.update_window((*window).into(), |_, window, cx| {
        window.dispatch_event(
            gpui::PlatformInput::MouseDown(MouseDownEvent {
                button,
                position,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        window.dispatch_event(
            gpui::PlatformInput::MouseUp(MouseUpEvent {
                button,
                position,
                modifiers: Default::default(),
                click_count: 1,
            }),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    redraw(window, cx);
}
fn press(
    window: &gpui::WindowHandle<Host>,
    cx: &mut gpui::TestAppContext,
    key: &str,
    ctrl: bool,
    shift: bool,
) {
    redraw(window, cx);
    cx.update_window((*window).into(), |_, window, cx| {
        window.dispatch_event(
            gpui::PlatformInput::KeyDown(KeyDownEvent {
                keystroke: gpui::Keystroke {
                    modifiers: gpui::Modifiers {
                        control: ctrl,
                        shift,
                        ..Default::default()
                    },
                    key: key.into(),
                    key_char: None,
                },
                is_held: false,
                prefer_character_input: false,
            }),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    redraw(window, cx);
}
fn menu_ready(
    shell: &Entity<ShellApp>,
    window: &gpui::WindowHandle<Host>,
    cx: &mut gpui::TestAppContext,
) {
    // Draw both deferred frames, with the actual menu in the dispatch tree.
    for _ in 0..3 {
        redraw(window, cx);
        cx.update_window((*window).into(), |_, window, cx| {
            window.simulate_next_frame(cx)
        })
        .unwrap();
        cx.run_until_parked();
    }
    cx.update_window((*window).into(), |_, window, cx| {
        let menu = shell.read(cx).context_menu.clone().unwrap();
        assert!(
            menu.read(cx).focus_handle(cx).contains_focused(window, cx),
            "deferred frames naturally focus live menu"
        );
    })
    .unwrap();
}

#[gpui::test]
fn tab_menu_actual_right_click_copy_close_and_keyboard_reopen(cx: &mut gpui::TestAppContext) {
    let (shell, window, dir) = setup(cx, "actual");
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| {
                for name in ["a.md", "b.md", "c.md"] {
                    open(shell, &dir, name, window, cx);
                }
            })
        })
        .unwrap();
    click(&window, cx, "tab-0", MouseButton::Right);
    shell.read_with(cx, |shell, _| {
        assert_eq!(
            shell.active, 2,
            "right click inactive tab never activates it"
        );
        assert_eq!(
            shell.context_tab_target.as_ref().unwrap().0,
            shell.tabs[0].editor.entity_id()
        );
    });
    click(&window, cx, "context-action-复制路径", MouseButton::Left);
    cx.update(|app| {
        assert_eq!(
            app.read_from_clipboard().unwrap().text(),
            Some(dir.join("a.md").display().to_string())
        )
    });
    click(&window, cx, "tab-0", MouseButton::Right);
    click(&window, cx, "context-action-关闭", MouseButton::Left);
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.tab_count(), 2);
        assert_eq!(shell.tab_path(shell.active), Some(dir.join("c.md")));
    });
    press(&window, cx, "t", true, true);
    shell.read_with(cx, |shell, app| {
        assert_eq!(shell.tab_count(), 3, "Ctrl+Shift+T works after close");
        assert_eq!(shell.tab_path(shell.active), Some(dir.join("a.md")));
        assert_eq!(
            shell.editor().unwrap().read(app).buffer_text_for_probe(),
            "original\n"
        );
    });
    // Menu keyboard navigation executes the same captured inactive tab target.
    click(&window, cx, "tab-1", MouseButton::Right);
    menu_ready(&shell, &window, cx);
    press(&window, cx, "down", false, false);
    press(&window, cx, "enter", false, false);
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.tab_index_by_path(&dir.join("c.md")), None)
    });
    std::fs::remove_dir_all(dir).unwrap();
}

#[gpui::test]
fn tab_menu_bulk_save_cancel_failure_halts_queue(cx: &mut gpui::TestAppContext) {
    let (shell, window, dir) = setup(cx, "bulk");
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| {
                for name in ["a.md", "b.md", "c.md", "d.md"] {
                    open(shell, &dir, name, window, cx);
                }
                for i in [1, 2] {
                    shell.tabs[i]
                        .editor
                        .update(cx, |ed, cx| ed.insert_text_at(0, 0, "dirty ", cx));
                }
            })
        })
        .unwrap();
    click(&window, cx, "tab-0", MouseButton::Right);
    click(
        &window,
        cx,
        "context-action-关闭所有标签",
        MouseButton::Left,
    );
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.tab_count(), 3, "clean A closes, then dirty B stops");
        assert_eq!(shell.close_queue.len(), 2);
    });
    // A second close or right-click cannot replace a modal's pending intent.
    click(&window, cx, "tab-2", MouseButton::Right);
    shell.read_with(cx, |shell, _| assert!(shell.context_menu.is_none()));
    press(&window, cx, "enter", false, false); // Save B, next confirm C.
    assert_eq!(
        std::fs::read_to_string(dir.join("b.md")).unwrap(),
        "dirty original\n"
    );
    shell.read_with(cx, |shell, app| {
        assert_eq!(shell.tab_count(), 2);
        assert!(shell.confirm.is_some());
        assert_eq!(shell.tab_path(0), Some(dir.join("c.md")));
        assert!(shell.tab_is_modified(0, app).unwrap());
    });
    press(&window, cx, "escape", false, false);
    shell.read_with(cx, |shell, _| {
        assert!(shell.close_queue.is_empty(), "Escape aborts remaining D");
        assert_eq!(shell.tab_count(), 2);
    });
    // Real filesystem failure: replace C's file with a directory.
    std::fs::remove_file(dir.join("c.md")).unwrap();
    std::fs::create_dir(dir.join("c.md")).unwrap();
    click(&window, cx, "tab-0", MouseButton::Right);
    click(
        &window,
        cx,
        "context-action-关闭所有标签",
        MouseButton::Left,
    );
    press(&window, cx, "enter", false, false);
    shell.read_with(cx, |shell, app| {
        assert_eq!(shell.tab_count(), 2, "failed save cannot proceed to D");
        assert!(shell.close_queue.is_empty());
        assert!(shell.tab_is_modified(0, app).unwrap());
        assert!(
            shell.notice_board.message().is_some(),
            "failed write is visible"
        );
    });
    std::fs::remove_dir(dir.join("c.md")).unwrap();
    std::fs::write(dir.join("c.md"), "original\n").unwrap();
    click(&window, cx, "tab-0", MouseButton::Right);
    click(
        &window,
        cx,
        "context-action-关闭所有标签",
        MouseButton::Left,
    );
    press(&window, cx, "enter", false, false);
    shell.read_with(cx, |shell, _| assert_eq!(shell.tab_count(), 0));
    press(&window, cx, "t", true, true);
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.tab_path(0), Some(dir.join("d.md")))
    });
    std::fs::remove_dir_all(dir).unwrap();
}

#[gpui::test]
fn tab_menu_snapshot_excludes_new_tabs_and_clean_intent_rechecks_dirty(
    cx: &mut gpui::TestAppContext,
) {
    let (shell, window, dir) = setup(cx, "snapshot");
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| {
                for name in ["a.md", "b.md", "c.md"] {
                    open(shell, &dir, name, window, cx);
                }
                let target = shell.tabs[0].editor.entity_id();
                shell.open_tab_context_menu(target, gpui::point(px(10.), px(70.)), window, cx);
                open(shell, &dir, "new.md", window, cx);
                shell.activate_context_menu_item("tab-close-right", window, cx);
                assert_eq!(
                    shell.tab_count(),
                    2,
                    "newly opened tab excluded from menu snapshot"
                );
                assert_eq!(shell.tab_path(1), Some(dir.join("new.md")));
                // A clean-only queued identity becomes dirty before consumption: skip it.
                let id = shell.tabs[0].editor.entity_id();
                shell.close_queue.push_back((id, true));
                shell.tabs[0]
                    .editor
                    .update(cx, |ed, cx| ed.insert_text_at(0, 0, "later ", cx));
                shell.drain_close_queue(window, cx);
                assert_eq!(shell.tab_count(), 2);
                assert!(shell.confirm.is_none());
                // Reopen existing path only activates its current editor, never reloads dirty data.
                shell.closed_tabs.push(dir.join("a.md"));
                shell.reopen_closed_tab(window, cx);
                assert_eq!(shell.tab_count(), 2);
                assert!(shell.tab_is_modified(shell.active, cx).unwrap());
                shell.closed_tabs.push(dir.join("missing.md"));
                shell.reopen_closed_tab(window, cx);
                assert_eq!(shell.tab_count(), 2);
                assert_eq!(shell.closed_tabs.last(), Some(&dir.join("missing.md")));
                assert!(shell.notice_board.message().unwrap().contains("失败"));
            })
        })
        .unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[gpui::test]
fn tab_menu_focus_restore_and_late_frames_do_not_steal_modal(cx: &mut gpui::TestAppContext) {
    let (shell, window, dir) = setup(cx, "focus");
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| {
                open(shell, &dir, "a.md", window, cx);
                shell.tabs[0]
                    .editor
                    .update(cx, |ed, cx| ed.insert_text_at(0, 0, "dirty ", cx));
            })
        })
        .unwrap();
    click(&window, cx, "tab-0", MouseButton::Right);
    menu_ready(&shell, &window, cx);
    press(&window, cx, "escape", false, false);
    cx.update_window((*window).into(), |_, window, cx| {
        assert!(shell.read(cx).context_menu.is_none());
        assert!(
            shell
                .read(cx)
                .editor()
                .unwrap()
                .read(cx)
                .focus_handle()
                .contains_focused(window, cx),
            "Escape restores original editor focus"
        );
    })
    .unwrap();
    // Opening then closing before either deferred callback runs cannot focus its orphan handle.
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_tab_context_menu(
                    shell.tabs[0].editor.entity_id(),
                    gpui::point(px(20.), px(70.)),
                    window,
                    cx,
                );
                shell.request_close_tab(0, window, cx);
            })
        })
        .unwrap();
    for _ in 0..3 {
        redraw(&window, cx);
        cx.update_window(window.into(), |_, window, cx| {
            window.simulate_next_frame(cx)
        })
        .unwrap();
        cx.run_until_parked();
    }
    cx.update_window(window.into(), |_, window, cx| {
        assert!(shell.read(cx).confirm.is_some());
        assert!(
            shell.read(cx).confirm_focus.contains_focused(window, cx),
            "late context focus never steals modal"
        );
    })
    .unwrap();
    press(&window, cx, "escape", false, false);
    // Dismiss must not restore previous focus after business has deliberately moved elsewhere.
    click(&window, cx, "tab-0", MouseButton::Right);
    menu_ready(&shell, &window, cx);
    let alternate = cx.update(|app| app.focus_handle());
    cx.update_window(window.into(), |_, window, cx| {
        window.focus(&alternate, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        assert!(
            alternate.is_focused(window),
            "outside focus is not stolen back"
        );
    })
    .unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[gpui::test]
fn tab_menu_actual_bulk_variants_and_disabled_entries(cx: &mut gpui::TestAppContext) {
    for (tag, selector, remaining) in [
        ("others", "context-action-关闭其他标签", vec!["b.md"]),
        ("left", "context-action-关闭左侧标签", vec!["b.md", "c.md"]),
        ("right", "context-action-关闭右侧标签", vec!["a.md", "b.md"]),
        ("clean", "context-action-关闭已保存标签", vec!["b.md"]),
    ] {
        let (shell, window, dir) = setup(cx, tag);
        window
            .update(cx, |_, window, cx| {
                shell.update(cx, |shell, cx| {
                    for name in ["a.md", "b.md", "c.md"] {
                        open(shell, &dir, name, window, cx);
                    }
                    if tag == "clean" {
                        shell.tabs[1]
                            .editor
                            .update(cx, |ed, cx| ed.insert_text_at(0, 0, "dirty ", cx));
                    }
                })
            })
            .unwrap();
        click(&window, cx, "tab-1", MouseButton::Right);
        click(&window, cx, selector, MouseButton::Left);
        shell.read_with(cx, |shell, _| {
            assert_eq!(
                shell
                    .tabs
                    .iter()
                    .map(|tab| tab.path.clone())
                    .collect::<Vec<_>>(),
                remaining
                    .iter()
                    .map(|name| dir.join(name))
                    .collect::<Vec<_>>()
            );
            assert!(
                shell.confirm.is_none(),
                "clean-only never prompts or discards dirty B"
            );
        });
        std::fs::remove_dir_all(dir).unwrap();
    }
    let (shell, window, dir) = setup(cx, "disabled");
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| open(shell, &dir, "only.md", window, cx))
        })
        .unwrap();
    click(&window, cx, "tab-0", MouseButton::Right);
    click(
        &window,
        cx,
        "context-action-关闭其他标签",
        MouseButton::Left,
    );
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.tab_count(), 1, "disabled others cannot execute")
    });
    click(&window, cx, "tab-0", MouseButton::Right);
    click(
        &window,
        cx,
        "context-action-关闭左侧标签",
        MouseButton::Left,
    );
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.tab_count(), 1, "disabled left cannot execute")
    });
    click(&window, cx, "tab-0", MouseButton::Right);
    menu_ready(&shell, &window, cx);
    press(&window, cx, "down", false, false); // Close
    press(&window, cx, "down", false, false); // Skip disabled others/left/right -> clean
    shell.read_with(cx, |shell, app| {
        assert_eq!(
            shell
                .context_menu
                .as_ref()
                .unwrap()
                .read(app)
                .selected_index(),
            Some(4)
        )
    });
    std::fs::remove_dir_all(dir).unwrap();
}

#[gpui::test]
fn tab_menu_rename_stale_target_relative_copy_and_menu_reopen(cx: &mut gpui::TestAppContext) {
    let (shell, window, dir) = setup(cx, "rename");
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| {
                open(shell, &dir, "a.md", window, cx);
                open(shell, &dir, "b.md", window, cx);
                shell.open_workspace(dir.clone(), cx);
            })
        })
        .unwrap();
    click(&window, cx, "tab-0", MouseButton::Right);
    click(
        &window,
        cx,
        "context-action-复制相对路径",
        MouseButton::Left,
    );
    cx.update(|app| {
        assert_eq!(
            app.read_from_clipboard().unwrap().text(),
            Some("a.md".into())
        )
    });
    click(&window, cx, "tab-0", MouseButton::Right);
    window
        .update(cx, |_, _, cx| {
            shell.update(cx, |shell, cx| {
                shell.rename_path(dir.join("a.md"), dir.join("renamed.md"), cx);
            })
        })
        .unwrap();
    click(&window, cx, "context-action-关闭", MouseButton::Left);
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.tab_count(), 1);
        assert_eq!(
            shell.closed_tabs.last(),
            Some(&dir.join("renamed.md")),
            "rename follows stable target and reopened path"
        );
    });
    click(&window, cx, "tab-0", MouseButton::Right);
    click(
        &window,
        cx,
        "context-action-重新打开已关闭标签",
        MouseButton::Left,
    );
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.tab_path(shell.active), Some(dir.join("renamed.md")))
    });
    // A target removed by an external close is not replaced by the same path's new editor.
    click(&window, cx, "tab-0", MouseButton::Right);
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.close_tab_no_confirm(0, window, cx);
                shell.open_document(dir.join("b.md"), window, cx);
            })
        })
        .unwrap();
    click(
        &window,
        cx,
        "context-action-关闭所有标签",
        MouseButton::Left,
    );
    shell.read_with(cx, |shell, _| {
        assert_eq!(
            shell.tab_count(),
            2,
            "stale pivot cancels action, not new editor"
        )
    });
    std::fs::remove_dir_all(dir).unwrap();
}

#[gpui::test]
fn tab_menu_actual_discard_reopens_disk_and_history_is_bounded(cx: &mut gpui::TestAppContext) {
    let (shell, window, dir) = setup(cx, "discard-history");
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| {
                open(shell, &dir, "discard.md", window, cx);
                shell.tabs[0]
                    .editor
                    .update(cx, |ed, cx| ed.insert_text_at(0, 0, "unsaved ", cx));
            })
        })
        .unwrap();
    click(&window, cx, "tab-0", MouseButton::Right);
    click(&window, cx, "context-action-关闭", MouseButton::Left);
    press(&window, cx, "tab", false, false); // Save -> Don't save
    press(&window, cx, "enter", false, false);
    shell.read_with(cx, |shell, _| assert_eq!(shell.tab_count(), 0));
    assert_eq!(
        std::fs::read_to_string(dir.join("discard.md")).unwrap(),
        "original\n"
    );
    press(&window, cx, "t", true, true);
    shell.read_with(cx, |shell, app| {
        assert_eq!(
            shell.editor().unwrap().read(app).buffer_text_for_probe(),
            "original\n"
        );
        assert!(!shell.tab_is_modified(0, app).unwrap());
    });
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.close_active_tab(window, cx);
                for i in 0..22 {
                    open(shell, &dir, &format!("history-{i}.md"), window, cx);
                    shell.close_active_tab(window, cx);
                }
                assert_eq!(shell.closed_tabs.len(), 20);
                assert_eq!(shell.closed_tabs.first(), Some(&dir.join("history-2.md")));
                shell.open_readonly_tab("diff".into(), "synthetic".into(), cx);
                shell.close_active_tab(window, cx);
                assert_eq!(
                    shell.closed_tabs.len(),
                    20,
                    "synthetic git view never enters disk history"
                );
            })
        })
        .unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[gpui::test]
fn tab_menu_reopened_xlsx_focuses_mounted_grid(cx: &mut gpui::TestAppContext) {
    let (shell, window, dir) = setup(cx, "xlsx-focus");
    let path = dir.join("book.xlsx");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.xlsx"),
        &path,
    )
    .unwrap();
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_document(path.clone(), window, cx);
                shell.close_active_tab(window, cx);
            })
        })
        .unwrap();
    press(&window, cx, "t", true, true);
    cx.update_window(window.into(), |_, window, cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.tab_path(0), Some(path.clone()));
        assert!(
            shell
                .xlsx_grid
                .as_ref()
                .unwrap()
                .1
                .read(cx)
                .focus_handle(cx)
                .contains_focused(window, cx),
            "reopen focuses actual mounted grid, not invisible editor"
        );
    })
    .unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

struct Host(Entity<ShellApp>);
impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.0.clone())
    }
}
fn setup(
    cx: &mut gpui::TestAppContext,
    tag: &str,
) -> (Entity<ShellApp>, gpui::WindowHandle<Host>, PathBuf) {
    cx.update(|app| {
        app.bind_keys([
            gpui::KeyBinding::new("escape", menu::Cancel, Some("menu")),
            gpui::KeyBinding::new("down", menu::SelectNext, Some("menu")),
            gpui::KeyBinding::new("up", menu::SelectPrevious, Some("menu")),
            gpui::KeyBinding::new("enter", menu::Confirm, Some("menu")),
        ]);
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let dir = std::env::temp_dir().join(format!("tab-menu-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut slot = None;
    let window = cx.add_window(|window, cx| {
        let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
        slot = Some(shell.clone());
        Host(shell)
    });
    (slot.unwrap(), window, dir)
}
fn open(
    shell: &mut ShellApp,
    dir: &Path,
    name: &str,
    window: &mut Window,
    cx: &mut Context<ShellApp>,
) {
    let path = dir.join(name);
    std::fs::write(&path, "original\n").unwrap();
    shell.open_document(path, window, cx);
}

#[gpui::test]
fn tab_menu_confirmation_tracks_identity_after_index_shift(cx: &mut gpui::TestAppContext) {
    let (shell, window, dir) = setup(cx, "identity");
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| {
                for name in ["a.md", "b.md", "c.md"] {
                    open(shell, &dir, name, window, cx);
                }
                shell.tabs[1]
                    .editor
                    .update(cx, |ed, cx| ed.insert_text_at(0, 0, "dirty ", cx));
                let wanted = shell.tabs[1].editor.clone();
                shell.request_close_tab(1, window, cx);
                shell.close_tab_no_confirm(0, window, cx);
                shell.resolve_confirm(1, window, cx);
                assert!(
                    !shell.tabs.iter().any(|t| t.editor == wanted),
                    "confirm must close original B, not new occupant of index 1"
                );
                assert_eq!(shell.tab_path(0), Some(dir.join("c.md")));
                assert_eq!(
                    std::fs::read_to_string(dir.join("b.md")).unwrap(),
                    "original\n"
                );
            })
        })
        .unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[gpui::test]
fn tab_menu_late_dismiss_cannot_close_replacement(cx: &mut gpui::TestAppContext) {
    let (shell, window, dir) = setup(cx, "late-dismiss");
    let old = window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_editor_context_menu(
                    gpui::point(px(10.), px(10.)),
                    false,
                    true,
                    window,
                    cx,
                );
                let old = shell.context_menu.clone().unwrap();
                shell.open_file_tree_context_menu(
                    gpui::point(px(20.), px(20.)),
                    dir.clone(),
                    true,
                    window,
                    cx,
                );
                old
            })
        })
        .unwrap();
    window
        .update(cx, |_, window, cx| {
            old.update(cx, |menu, cx| menu.cancel(&menu::Cancel, window, cx))
        })
        .unwrap();
    cx.run_until_parked();
    shell.read_with(cx, |shell, _| {
        assert!(
            shell.context_menu.is_some(),
            "old DismissEvent must not remove new menu"
        );
        assert_eq!(shell.context_menu_target, Some(dir.clone()));
    });
    std::fs::remove_dir_all(dir).unwrap();

}

/// #107 P2：树面「复制路径」（ft-copy-path——右键目标绝对路径进系统
/// 剪贴板；tab 菜单早有同能力（「复制路径」——本测试对树面补齐对拍）。
/// 先红面：ft-copy-path 未接线时菜单项不存在 → 点不到 → 剪贴板为空。
#[gpui::test]
fn file_tree_menu_copy_path_to_clipboard(cx: &mut gpui::TestAppContext) {
    let (shell, window, dir) = setup(cx, "ft-copy");
    std::fs::write(dir.join("note.md"), "x").unwrap();
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_file_tree_context_menu(
                    gpui::point(px(10.), px(70.)),
                    dir.join("note.md"),
                    false,
                    window,
                    cx,
                );
                shell.activate_context_menu_item("ft-copy-path", window, cx);
            });
        })
        .unwrap();
    cx.update(|app| {
        assert_eq!(
            app.read_from_clipboard().unwrap().text(),
            Some(dir.join("note.md").display().to_string()),
            "剪贴板应含右键目标绝对路径"
        );
    });
    std::fs::remove_dir_all(dir).unwrap();
}
