//! #47 巨石拆解批③：原 shell.rs 内嵌 `confirm_lifecycle_tests` 纯迁出（零行为变化）。

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
    press_key_with_modifiers(cx, window, key, Default::default());
}

fn press_key_with_modifiers(
    cx: &mut gpui::TestAppContext,
    window: &gpui::WindowHandle<Host>,
    key: &str,
    modifiers: gpui::Modifiers,
) {
    let any: gpui::AnyWindowHandle = (*window).into();
    cx.update_window(any, |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .expect("预绘一帧（派发树在位）");
    cx.update_window(any, |_, window, cx| {
        window.dispatch_event(
            gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                keystroke: gpui::Keystroke {
                    modifiers,
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
            action: ConfirmAction::CloseTab {
                idx: 0,
                target: gpui::EntityId::from(0),
            },
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

/// Owned real file: create_dir succeeds only for this fresh test directory.
struct RealMdFixture {
    dir: PathBuf,
    path: PathBuf,
}

impl RealMdFixture {
    fn new(tag: &str) -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("mdc-confirm-{tag}-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&dir).expect("create owned test directory");
        let path = dir.join("real.md");
        std::fs::write(&path, "L0\n").expect("write real markdown fixture");
        Self { dir, path }
    }
}

impl Drop for RealMdFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn redraw(window: &gpui::WindowHandle<Host>, cx: &mut gpui::TestAppContext) {
    let any: gpui::AnyWindowHandle = (*window).into();
    cx.update_window(any, |_, window, cx| window.draw(cx).clear(cx))
        .expect("draw actual dispatch tree");
}

fn open_real_clean_tab(
    shell: &Entity<ShellApp>,
    window: &gpui::WindowHandle<Host>,
    cx: &mut gpui::TestAppContext,
    path: &Path,
) -> Entity<Editor> {
    let editor = window
        .update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                let focus = shell
                    .open_document_tab(path.to_path_buf(), cx)
                    .expect("real fixture opens through production tab loader");
                window.focus(&focus, cx);
                assert_eq!(shell.tab_count(), 1);
                let editor = shell.tab_editor(0).unwrap().clone();
                assert!(!editor.read(cx).is_modified(), "fixture starts clean");
                editor
            })
        })
        .expect("open real clean tab");
    assert_editor_restored(shell, window, cx, &editor, "L0", false);
    editor
}

fn assert_editor_restored(
    shell: &Entity<ShellApp>,
    window: &gpui::WindowHandle<Host>,
    cx: &mut gpui::TestAppContext,
    expected: &Entity<Editor>,
    content: &str,
    dirty: bool,
) {
    redraw(window, cx);
    window
        .update(cx, |_host, window, cx| {
            let shell = shell.read(cx);
            assert!(shell.confirm.is_none());
            assert_eq!(shell.tab_count(), 1, "cancel retains a valid tab");
            let editor = shell.tab_editor(0).expect("real tab remains");
            assert_eq!(
                editor.entity_id(),
                expected.entity_id(),
                "same Editor entity"
            );
            let editor = editor.read(cx);
            assert_eq!(editor.buffer_line(0), content);
            assert_eq!(editor.is_modified(), dirty);
            let focus = editor.focus_handle();
            assert!(focus.is_focused(window), "actual Editor focus restored");
            assert!(
                shell.root_focus.contains(focus, window),
                "Editor is mounted in drawn tree"
            );
            assert!(
                !shell.confirm_focus.is_focused(window),
                "modal focus released"
            );
        })
        .expect("observe actual Editor and focus");
}

fn show_real_close_confirm(
    shell: &Entity<ShellApp>,
    window: &gpui::WindowHandle<Host>,
    cx: &mut gpui::TestAppContext,
    editor: &Entity<Editor>,
) {
    // True Ctrl+W reaches production close_active_tab, which constructs and
    // focuses the modal. Do not inject ConfirmState or call resolve_confirm.
    press_key_with_modifiers(
        cx,
        window,
        "w",
        gpui::Modifiers {
            control: true,
            ..Default::default()
        },
    );
    redraw(window, cx);
    window
        .update(cx, |_host, window, cx| {
            let shell = shell.read(cx);
            let confirm = shell
                .confirm
                .as_ref()
                .expect("dirty tab produces actual modal");
            assert!(matches!(
                confirm.action,
                ConfirmAction::CloseTab { idx: 0, .. }
            ));
            assert_eq!(confirm.selected, 0);
            assert_eq!(confirm.buttons, CONFIRM_BUTTONS.to_vec());
            assert!(
                shell.confirm_focus.is_focused(window),
                "modal actually takes focus"
            );
            assert!(
                shell.root_focus.contains(&shell.confirm_focus, window),
                "modal mounted in drawn tree"
            );
            assert!(!editor.read(cx).focus_handle().is_focused(window));
        })
        .expect("observe real modal focus transfer");
}

fn assert_root_restored(
    shell: &Entity<ShellApp>,
    window: &gpui::WindowHandle<Host>,
    cx: &mut gpui::TestAppContext,
) {
    redraw(window, cx);
    window
        .update(cx, |_host, window, cx| {
            let shell = shell.read(cx);
            assert_eq!(shell.tab_count(), 0, "positive control really closes tab");
            assert!(shell.confirm.is_none());
            assert!(
                shell.root_focus.is_focused(window),
                "empty-state root focus restored"
            );
            assert!(!shell.confirm_focus.is_focused(window));
        })
        .expect("observe root focus after true close");
}

#[gpui::test]
fn confirm_enter_cancel_retains_real_tab_and_restores_editor_focus(cx: &mut gpui::TestAppContext) {
    let fixture = RealMdFixture::new("enter");
    let (shell, window) = shell_window(cx);
    open_real_clean_tab(&shell, &window, cx, &fixture.path);
    // Positive control: this valid clean tab can truly be closed via the
    // same production close_tab_at reached by a wrong cancel decision.
    press_key_with_modifiers(
        cx,
        &window,
        "w",
        gpui::Modifiers {
            control: true,
            ..Default::default()
        },
    );
    assert_root_restored(&shell, &window, cx);
    let editor = open_real_clean_tab(&shell, &window, cx, &fixture.path);
    editor.update(cx, |editor, cx| {
        editor
            .insert_text_at(0, 2, "X", cx)
            .expect("real edit makes tab dirty");
    });
    show_real_close_confirm(&shell, &window, cx, &editor);
    press_key(cx, &window, "tab");
    press_key(cx, &window, "tab");
    assert_eq!(selected(&shell, cx), 2, "real keys select Cancel");
    press_key(cx, &window, "enter");
    assert_editor_restored(&shell, &window, cx, &editor, "L0X", true);
    assert_eq!(
        std::fs::read_to_string(&fixture.path).unwrap(),
        "L0\n",
        "cancel never saves/discards source"
    );
}

#[gpui::test]
fn confirm_escape_retains_real_tab_and_true_close_restores_root_focus(
    cx: &mut gpui::TestAppContext,
) {
    let fixture = RealMdFixture::new("escape");
    let (shell, window) = shell_window(cx);
    let editor = open_real_clean_tab(&shell, &window, cx, &fixture.path);
    editor.update(cx, |editor, cx| {
        editor
            .insert_text_at(0, 2, "X", cx)
            .expect("real edit makes tab dirty");
    });
    show_real_close_confirm(&shell, &window, cx, &editor);
    press_key(cx, &window, "escape");
    assert_editor_restored(&shell, &window, cx, &editor, "L0X", true);
    assert_eq!(std::fs::read_to_string(&fixture.path).unwrap(), "L0\n");
    // Positive decision control on the same real tab: select Don't Save
    // via Tab and Enter, then verify real removal and root focus return.
    show_real_close_confirm(&shell, &window, cx, &editor);
    press_key(cx, &window, "tab");
    assert_eq!(selected(&shell, cx), 1);
    press_key(cx, &window, "enter");
    assert_root_restored(&shell, &window, cx);
    assert_eq!(std::fs::read_to_string(&fixture.path).unwrap(), "L0\n");
}
