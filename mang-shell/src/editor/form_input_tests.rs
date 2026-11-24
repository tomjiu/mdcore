//! Protocol tests call actual callbacks; rendered tests additionally exercise
//! window.dispatch_keystroke -> registered platform handler -> isolated field.

use super::*;
struct Host {
    ed: gpui::Entity<Editor>,
}
impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.ed.clone())
    }
}
fn setup(cx: &mut gpui::TestAppContext) -> (gpui::Entity<Editor>, gpui::WindowHandle<Host>) {
    let ed = cx.update(|cx| {
        cx.new(|cx| {
            Editor::new(
                Some("form.md".into()),
                "one 😀
two
"
                .into(),
                cx,
            )
        })
    });
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let host_ed = ed.clone();
    let window = cx.add_window(move |window, cx| {
        let focus = host_ed.read(cx).focus.clone();
        window.focus(&focus, cx);
        Host { ed: host_ed }
    });
    (ed, window)
}
fn draw(w: gpui::WindowHandle<Host>, cx: &mut gpui::TestAppContext) {
    cx.update_window(w.into(), |_, window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    })
    .unwrap();
}
fn dispatch(w: gpui::WindowHandle<Host>, cx: &mut gpui::TestAppContext, key: &'static str) {
    draw(w, cx);
    cx.update_window(w.into(), |_, window, cx| {
        window.dispatch_keystroke(gpui::Keystroke::parse(key).unwrap(), cx);
    })
    .unwrap();
    draw(w, cx);
}
#[gpui::test]
fn form_platform_rendered_find_native_once_escape_save(cx: &mut gpui::TestAppContext) {
    let (ed, w) = setup(cx);
    let file = std::env::temp_dir().join(format!(
        "md-form-{}-{}.md",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&file, "one 😀\ntwo\n").unwrap();
    let revision = ed.read_with(cx, |ed, _| ed.buffer.revision());
    ed.update(cx, |ed, _| ed.file_path = Some(file.clone()));
    dispatch(w, cx, "ctrl-f");
    w.update(cx, |_, window, cx| {
        assert!(ed.read(cx).find_focus.is_focused(window))
    })
    .unwrap();
    for ch in ["a", "😀", "中"] {
        dispatch(w, cx, ch);
    }
    w.update(cx, |_, window, cx| {
        ed.update(cx, |ed, cx| {
            assert_eq!(ed.find.as_ref().unwrap().query(), "a😀中");
            assert_eq!(ed.text_length_utf16(window, cx), Some(4));
            let bounds = ed
                .bounds_for_range(1..3, Bounds::default(), window, cx)
                .expect("field geometry painted");
            assert!(bounds.size.width > px(1.));
            let field = ed
                .form_bounds
                .borrow()
                .iter()
                .find(|(f, _)| *f == FormField::Find)
                .unwrap()
                .1;
            assert_eq!(
                ed.character_index_for_point(
                    point(field.origin.x - px(1.), field.origin.y),
                    window,
                    cx
                ),
                None
            );
            unchanged(ed, revision);
        })
    })
    .unwrap();
    dispatch(w, cx, "escape");
    w.update(cx, |_, window, cx| {
        ed.update(cx, |ed, _| {
            assert!(ed.focus.is_focused(window));
            assert!(ed.find.is_none());
            assert_eq!(ed.save(), SaveOutcome::Unmodified);
        })
    })
    .unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "one 😀\ntwo\n");
    std::fs::remove_file(file).unwrap();
}
#[gpui::test]
fn form_platform_rendered_replacement_clipboard_tab_readonly(cx: &mut gpui::TestAppContext) {
    let (ed, w) = setup(cx);
    let revision = ed.read_with(cx, |ed, _| ed.buffer.revision());
    dispatch(w, cx, "ctrl-h");
    dispatch(w, cx, "a");
    dispatch(w, cx, "tab");
    w.update(cx, |_, window, cx| {
        assert!(ed.read(cx).replacement_focus.is_focused(window))
    })
    .unwrap();
    dispatch(w, cx, "😀");
    dispatch(w, cx, "ctrl-a");
    dispatch(w, cx, "ctrl-c");
    dispatch(w, cx, "ctrl-x");
    dispatch(w, cx, "ctrl-v");
    dispatch(w, cx, "shift-tab");
    ed.update(cx, |ed, _| {
        assert_eq!(ed.find.as_ref().unwrap().replacement(), "😀");
        ed.read_only = true;
    });
    dispatch(w, cx, "ctrl-a");
    dispatch(w, cx, "ctrl-v");
    w.update(cx, |_, window, cx| {
        ed.update(cx, |ed, cx| {
            assert_eq!(ed.find.as_ref().unwrap().query(), "😀");
            ed.set_selected_text_range(2..0, window, cx);
            assert!(ed.selected_text_range(false, window, cx).unwrap().reversed);
            ed.replace_find(true);
            unchanged(ed, revision);
        })
    })
    .unwrap();
    dispatch(w, cx, "escape");
}
#[gpui::test]
fn form_platform_deferred_focus_does_not_steal_other_window(cx: &mut gpui::TestAppContext) {
    let (ed, w) = setup(cx);
    let (other, second) = setup(cx);
    ed.update(cx, |ed, cx| ed.toggle_command_palette(cx));
    draw(w, cx);
    draw(second, cx);
    second
        .update(cx, |_, window, cx| {
            assert!(other.read(cx).focus.is_focused(window));
            assert!(!ed.read(cx).palette_focus.is_focused(window));
        })
        .unwrap();
    w.update(cx, |_, window, cx| {
        assert!(ed.read(cx).palette_focus.is_focused(window))
    })
    .unwrap();
}
#[gpui::test]
fn form_platform_async_adapter_none_ime_and_paste(cx: &mut gpui::TestAppContext) {
    let (ed, w) = setup(cx);
    let revision = ed.read_with(cx, |ed, _| ed.buffer.revision());
    dispatch(w, cx, "ctrl-f");
    // Installed TestWindow handler is gpui-private. This public async adapter
    // uses the painted field bounds; rendered tests separately prove registration.
    let mut native = cx
        .update_window(w.into(), |_, window, cx| {
            let bounds = ed
                .read(cx)
                .form_bounds
                .borrow()
                .iter()
                .find(|(f, _)| *f == FormField::Find)
                .unwrap()
                .1;
            gpui::PlatformInputHandler::new(
                window.to_async(cx),
                Box::new(ElementInputHandler::new(bounds, ed.clone())),
            )
        })
        .unwrap();
    native.replace_and_mark_text_in_range(None, "你😀", Some(1..3));
    assert_eq!(native.selected_text_range(false).unwrap().range, 1..3);
    native.replace_text_in_range(None, "中文");
    native.replace_and_mark_text_in_range(None, "x", Some(1..1));
    native.replace_text_in_range(None, "");
    native.paste(ClipboardItem::new_string("😀".into()));
    let mut adjusted = None;
    assert_eq!(
        native.text_for_range(3..4, &mut adjusted),
        Some("😀".into())
    );
    assert_eq!(adjusted, Some(2..4));
    assert!(native.bounds_for_range(2..4).is_some());
    ed.update(cx, |ed, _| {
        assert_eq!(ed.find.as_ref().unwrap().query(), "中文😀");
        unchanged(ed, revision);
    });
}
fn unchanged(ed: &Editor, revision: u64) {
    assert_eq!(
        ed.buffer.to_plain_text(),
        "one 😀
two
",
        "form text must not enter document"
    );
    assert_eq!(ed.buffer.revision(), revision);
    assert!(!ed.buffer.is_modified());
    assert_eq!(ed.buffer.cursor(), Point::new(0, 0));
}
#[gpui::test]
fn form_platform_find_none_ime_commit_cancel(cx: &mut gpui::TestAppContext) {
    let (ed, w) = setup(cx);
    w.update(cx, |_, window, cx| {
        ed.update(cx, |ed, cx| {
            let revision = ed.buffer.revision();
            ed.open_find(window, cx);
            ed.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);
            assert_eq!(ed.marked_text_range(window, cx), Some(0..1));
            ed.replace_and_mark_text_in_range(None, "你😀", Some(1..3), window, cx);
            ed.replace_text_in_range(None, "你好", window, cx);
            assert_eq!(ed.find.as_ref().unwrap().query(), "你好");
            assert_eq!(ed.marked_text_range(window, cx), None);
            ed.replace_and_mark_text_in_range(None, "x", Some(1..1), window, cx);
            ed.replace_text_in_range(None, "", window, cx);
            assert_eq!(ed.find.as_ref().unwrap().query(), "你好");
            unchanged(ed, revision);
        })
    })
    .unwrap();
}
#[gpui::test]
fn form_platform_find_readonly_utf16_range(cx: &mut gpui::TestAppContext) {
    let (ed, w) = setup(cx);
    w.update(cx, |_, window, cx| {
        ed.update(cx, |ed, cx| {
            let revision = ed.buffer.revision();
            ed.read_only = true;
            ed.open_find(window, cx);
            ed.replace_text_in_range(None, "A😀中", window, cx);
            assert_eq!(ed.find.as_ref().unwrap().query(), "A😀中");
            let mut adjusted = None;
            assert_eq!(
                ed.text_for_range(2..3, &mut adjusted, window, cx),
                Some("😀".into())
            );
            assert_eq!(adjusted, Some(1..3));
            ed.replace_text_in_range(Some(1..3), "B", window, cx);
            assert_eq!(ed.find.as_ref().unwrap().query(), "AB中");
            assert_eq!(
                ed.selected_text_range(false, window, cx).unwrap().range,
                2..2
            );
            unchanged(ed, revision);
        })
    })
    .unwrap();
}
#[gpui::test]
fn form_platform_palette_none_ime(cx: &mut gpui::TestAppContext) {
    let (ed, w) = setup(cx);
    w.update(cx, |_, window, cx| {
        ed.update(cx, |ed, cx| {
            let revision = ed.buffer.revision();
            window.focus(&ed.focus, cx);
            ed.dispatch_key(
                &KeyDownEvent {
                    keystroke: gpui::Keystroke::parse("ctrl-shift-p").unwrap(),
                    is_held: false,
                    prefer_character_input: false,
                },
                window,
                cx,
            );
            assert!(ed.palette_focus.is_focused(window));
            ed.replace_and_mark_text_in_range(None, "查", Some(1..1), window, cx);
            ed.replace_text_in_range(None, "查找", window, cx);
            assert_eq!(ed.palette.as_ref().unwrap().query, "查找");
            assert_eq!(ed.palette.as_ref().unwrap().items, vec![Cmd::OpenFind]);
            unchanged(ed, revision);
        })
    })
    .unwrap();
}
#[gpui::test]
fn form_platform_goto_query_does_not_edit(cx: &mut gpui::TestAppContext) {
    let (ed, w) = setup(cx);
    w.update(cx, |_, window, cx| {
        ed.update(cx, |ed, cx| {
            let revision = ed.buffer.revision();
            window.focus(&ed.focus, cx);
            ed.dispatch_key(
                &KeyDownEvent {
                    keystroke: gpui::Keystroke::parse("ctrl-g").unwrap(),
                    is_held: false,
                    prefer_character_input: false,
                },
                window,
                cx,
            );
            assert!(ed.goto_focus.is_focused(window));
            ed.replace_text_in_range(None, "2", window, cx);
            assert_eq!(ed.goto.as_ref().unwrap().input, "2");
            assert!(ed.goto.as_ref().unwrap().hint.contains("第 2 行"));
            unchanged(ed, revision);
        })
    })
    .unwrap();
}

/// #47 A4 面板浮层点击面（agent-xt11 2026-11-23 首批红测）：
/// 浮层**非输入区**的点击不得穿透到文档面。此前行为：点击面板列表
/// 区 → 事件冒泡到 editor-surface → dispatch_mouse_down 无条件
/// focus_document_input（夺走输入焦点）+ hit_test 移动文档光标
/// （焦点落在浮层底下的正文）；右键同穿（dispatch_context_menu
/// 同样夺焦点移光标）。行为守恒基线：面板选中仍是键盘 ↑↓ 驱动
/// （palette_view 行为守恒注释），点击不执行不关闭；Esc 关闭语义
/// 不变。
fn palette_click(
    w: gpui::WindowHandle<Host>,
    cx: &mut gpui::TestAppContext,
    ed: &gpui::Entity<Editor>,
    button: gpui::MouseButton,
    below_input: f32,
) {
    let b = cx
        .update_window(w.into(), |_host, _window, cx| {
            ed.update(cx, |ed, _| {
                let (_, b) = ed
                    .form_bounds
                    .borrow()
                    .iter()
                    .find(|(f, _)| *f == FormField::Palette)
                    .expect("面板输入框已绘制（form_bounds 观测面）")
                    .clone();
                b
            })
        })
        .expect("编辑器在场");
    // 实测输入框 bounds——点击点在输入框正下方 `below_input` px 处
    // （浮层列表区：面板容器内、查询输入框外）。
    let pos = gpui::point(
        b.origin.x + gpui::px(20.),
        b.origin.y + b.size.height + gpui::px(below_input),
    );
    let down = gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
        button,
        position: pos,
        modifiers: Default::default(),
        click_count: 1,
        first_mouse: false,
    });
    let up = gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
        button,
        position: pos,
        modifiers: Default::default(),
        click_count: 1,
    });
    cx.update_window(w.into(), |_, window, cx| {
        window.dispatch_event(down, cx);
        window.dispatch_event(up, cx);
    })
    .expect("浮层点击派发");
}

#[gpui::test]
fn form_platform_palette_overlay_click_does_not_leak_to_document(cx: &mut gpui::TestAppContext) {
    let (ed, w) = setup(cx);
    let revision = ed.read_with(cx, |ed, _| ed.buffer.revision());
    dispatch(w, cx, "ctrl-shift-p");
    w.update(cx, |_, window, cx| {
        assert!(ed.read(cx).palette_focus.is_focused(window));
    })
    .unwrap();
    palette_click(w, cx, &ed, gpui::MouseButton::Left, 24.);
    w.update(cx, |_, window, cx| {
        ed.update(cx, |ed, _| {
            assert!(ed.palette.is_some(), "点击浮层列表区不得关闭面板");
            assert!(
                ed.palette_focus.is_focused(window),
                "点击浮层列表区不得夺走输入焦点"
            );
            assert_eq!(
                ed.buffer.cursor(),
                Point::new(0, 0),
                "点击浮层列表区不得移动文档光标（点击穿透）"
            );
        })
    })
    .unwrap();
    ed.read_with(cx, |ed, _| unchanged(ed, revision));
    // 右键同面：浮层内右键不得触发文档光标移动/焦点夺走
    //（真实窗口里还会弹文档右键菜单——menu_opener 面）。
    palette_click(w, cx, &ed, gpui::MouseButton::Right, 24.);
    w.update(cx, |_, window, cx| {
        ed.update(cx, |ed, _| {
            assert!(ed.palette.is_some(), "浮层右键不得关闭面板");
            assert!(
                ed.palette_focus.is_focused(window),
                "浮层右键不得夺走输入焦点"
            );
            assert_eq!(
                ed.buffer.cursor(),
                Point::new(0, 0),
                "浮层右键不得移动文档光标（右键穿透）"
            );
        })
    })
    .unwrap();
    ed.read_with(cx, |ed, _| unchanged(ed, revision));
}

#[gpui::test]
fn form_platform_palette_esc_two_stage_and_enter_restore(cx: &mut gpui::TestAppContext) {
    let (ed, w) = setup(cx);
    let revision = ed.read_with(cx, |ed, _| ed.buffer.revision());
    dispatch(w, cx, "ctrl-shift-p");
    // 查询非空 → Esc 第一段：清查询（面板保持开、焦点不动）
    w.update(cx, |_, window, cx| {
        ed.update(cx, |ed, cx| {
            ed.replace_text_in_range(None, "查", window, cx);
            assert_eq!(ed.palette.as_ref().unwrap().query, "查");
        })
    })
    .unwrap();
    dispatch(w, cx, "escape");
    w.update(cx, |_, window, cx| {
        ed.update(cx, |ed, _| {
            assert!(ed.palette.is_some(), "查询非空 Esc 只清查询不关面板");
            assert_eq!(ed.palette.as_ref().unwrap().query, "");
            assert!(
                ed.palette_focus.is_focused(window),
                "清查询后焦点保持在输入框"
            );
        })
    })
    .unwrap();
    // 查询空 → Esc 第二段：关面板回文档焦点
    dispatch(w, cx, "escape");
    w.update(cx, |_, window, cx| {
        ed.update(cx, |ed, _| {
            assert!(ed.palette.is_none(), "查询空 Esc 关面板");
            assert!(ed.focus.is_focused(window), "关闭后面板焦点回文档");
        })
    })
    .unwrap();
    ed.read_with(cx, |ed, _| unchanged(ed, revision));
    // Enter 执行链：面板 → 执行选中命令 → 面板关 + 后续表单焦点成立
    //（「查」唯一命中 Cmd::OpenFind——执行后查找框获得焦点）。
    dispatch(w, cx, "ctrl-shift-p");
    w.update(cx, |_, window, cx| {
        ed.update(cx, |ed, cx| {
            ed.replace_text_in_range(None, "查", window, cx);
            assert_eq!(ed.palette.as_ref().unwrap().items, vec![Cmd::OpenFind]);
        })
    })
    .unwrap();
    dispatch(w, cx, "enter");
    w.update(cx, |_, window, cx| {
        ed.update(cx, |ed, _| {
            assert!(ed.palette.is_none(), "Enter 执行后面板关闭");
            assert!(
                ed.find.is_some() && ed.find_focus.is_focused(window),
                "Enter 执行 OpenFind 后查找框获得焦点（终裁焦点链）"
            );
        })
    })
    .unwrap();
    ed.read_with(cx, |ed, _| unchanged(ed, revision));
}
