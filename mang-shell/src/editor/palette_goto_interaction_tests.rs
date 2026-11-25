//! #47 巨石拆解批②：原 editor/mod.rs 内嵌 `palette_goto_interaction_tests` 纯迁出（零行为变化——
//! 字节原样剥壳，挂载 #[cfg(test)] + #[path] 在 mod.rs）。

use super::*;

/// 测试宿主（EditorHost 同款——模块间不共享私有设施，各 10 行）。
struct PalHost {
    _ed: gpui::Entity<Editor>,
}

impl gpui::Render for PalHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div()
    }
}

/// md 编辑器 + 宿主窗口（面板键处理需要真实 `Window`）。
fn pal_window(
    cx: &mut gpui::TestAppContext,
    text: &str,
) -> (gpui::Entity<Editor>, gpui::WindowHandle<PalHost>) {
    let ed = cx.update(|cx| cx.new(|cx| Editor::new(Some("pal.md".into()), text.to_string(), cx)));
    let ed_for_host = ed.clone();
    let window = cx.add_window(move |window, cx| {
        let focus = ed_for_host.read(cx).focus.clone();
        window.focus(&focus, cx);
        PalHost {
            _ed: ed_for_host.clone(),
        }
    });
    (ed, window)
}

/// 无修饰键事件（`key_char` = 可打印字符入查询框；None = 控制键）。
fn key(key: &str, key_char: Option<&str>) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: gpui::Keystroke {
            modifiers: gpui::Modifiers::default(),
            key: key.into(),
            key_char: key_char.map(str::to_string),
        },
        is_held: false,
        prefer_character_input: false,
    }
}

/// Ctrl 组合键事件。
fn ctrl(key: &str) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: gpui::Keystroke {
            modifiers: gpui::Modifiers {
                control: true,
                ..Default::default()
            },
            key: key.into(),
            key_char: None,
        },
        is_held: false,
        prefer_character_input: false,
    }
}

/// 经真实分发入口派一个键（dispatch_key = 焦点路径到达的同一分发体）。
fn press(ed: &gpui::Entity<Editor>, window: &mut Window, cx: &mut App, event: KeyDownEvent) {
    // Legacy command lifecycle assertions remain unchanged. Text now uses the
    // platform callback, not the removed printable-key fallback.
    if let Some(text) = event.keystroke.key_char.as_deref() {
        ed.update(cx, |ed, cx| {
            ed.replace_text_in_range(None, text, window, cx)
        });
    } else {
        ed.update(cx, |ed, cx| ed.dispatch_key(&event, window, cx));
    }
}

/// 开合 + 键入过滤 + ↑↓ 选中移动 + Esc 关闭 + 重开换新（状态桥四段）。
#[gpui::test]
/// #112 P2 批④：Esc 两段式——查询非空先清查询（面板保持开，
/// 候选回全集）；查询空才关面板。先红面：现行为一步直接关。
#[gpui::test]
fn palette_esc_two_stage_clears_query_first(cx: &mut gpui::TestAppContext) {
    let (ed, window) = pal_window(cx, "# A\nbody\n");
    let _ = window.update(cx, |_host, window, cx| {
        let mut e = ctrl("p");
        e.keystroke.modifiers.shift = true;
        press(&ed, window, cx, e);
    });
    // 键入「折叠」——查询非空。
    let _ = window.update(cx, |_host, window, cx| {
        for ch in ["折", "叠"] {
            press(&ed, window, cx, key(ch, Some(ch)));
        }
    });
    // 第一记 Esc：清查询（面板保持开——两段式第一段）。
    let _ = window.update(cx, |_host, window, cx| {
        press(&ed, window, cx, key("escape", None));
    });
    ed.update(cx, |ed, _| {
        let pal = ed.palette.as_ref().expect("第一记 Esc 后面板保持开");
        assert_eq!(pal.query, "", "Esc 第一段：清查询");
        assert_eq!(
            pal.items.len(),
            commands::search("").len(),
            "候选回全集（与空查询同源）"
        );
        assert_eq!(pal.selected, 0, "清查询后选中复位 0");
    });
    // 第二记 Esc：关面板（两段式第二段——回到一步语义）。
    let _ = window.update(cx, |_host, window, cx| {
        press(&ed, window, cx, key("escape", None));
    });
    ed.update(cx, |ed, _| {
        assert!(ed.palette.is_none(), "第二记 Esc：关面板");
    });
}

/// #47 A4 残面（外点击关面板）：palette 打开态下点击编辑区文本
/// ——面板应关 + 焦点回文档。此前面板保持开：键入被 palette 键域
/// 拦截而焦点已在文档——「面板开着但键盘不知去哪」歧义态
/// （VS Code/IntelliJ 对照：外点击 = 关浮层）。
#[gpui::test]
fn palette_open_click_document_closes_palette(cx: &mut gpui::TestAppContext) {
    let (ed, window) = pal_window(cx, "# A\nbody\n");
    // Ctrl+Shift+P 开面板（键位真实入口——keymap 表解析）。
    let _ = window.update(cx, |_host, window, cx| {
        let mut e = ctrl("p");
        e.keystroke.modifiers.shift = true;
        press(&ed, window, cx, e);
    });
    ed.update(cx, |ed, _| {
        assert!(ed.palette.is_some(), "前置：面板开");
    });
    // 编辑区单击（方法级直测 dispatch_mouse_down——视图监听链
    // 绑定的就是它；断言面在面板态与焦点）。
    let _ = window.update(cx, |_host, window, cx| {
        let event = MouseDownEvent {
            button: MouseButton::Left,
            position: gpui::point(px(4.), px(4.)),
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        };
        ed.update(cx, |ed, cx| ed.dispatch_mouse_down(&event, window, cx));
    });
    ed.update(cx, |ed, _| {
        assert!(ed.palette.is_none(), "外点击关面板（歧义态根除）");
    });
    // 焦点回文档（关闭语义与 Esc 同款）。
    let _ = window.update(cx, |_host, window, cx| {
        let doc_focus = ed.read(cx).focus.clone();
        assert!(
            doc_focus.is_focused(window),
            "焦点回文档输入（不是浮层残留）"
        );
    });
}

/// #112 P2 批④：面板内 Ctrl 组合令旁路——Ctrl+F 在面板打开时
/// 照常打开查找框（面板不吞编辑类组合令——冲突裁决归编辑键层，
/// Q-IDE R4 对照）。先红面：现面板吞一切 ctrl 组合。
#[gpui::test]
fn palette_open_ctrl_combo_bypasses_to_editor(cx: &mut gpui::TestAppContext) {
    let (ed, window) = pal_window(cx, "# A\nbody\n");
    // 开面板。
    let _ = window.update(cx, |_host, window, cx| {
        let mut e = ctrl("p");
        e.keystroke.modifiers.shift = true;
        press(&ed, window, cx, e);
    });
    ed.update(cx, |ed, _| {
        assert!(ed.palette.is_some(), "前置：面板开");
        assert!(ed.find.is_none(), "前置：查找框未开");
    });
    // 面板内 Ctrl+F：旁路执行（打开查找框——不被面板吞）。
    let _ = window.update(cx, |_host, window, cx| {
        press(&ed, window, cx, ctrl("f"));
    });
    ed.update(cx, |ed, _| {
        assert!(ed.find.is_some(), "Ctrl+F 旁路：查找框打开（面板不吞）");
    });
}

/// Enter 执行命令闭环：键入唯一命中「查找」→ Enter → 面板关 +
/// 查找条开 + 最近使用记账（#183 A-3——下次打开面板顶部可见）。
#[gpui::test]
fn palette_enter_executes_and_touches_recent(cx: &mut gpui::TestAppContext) {
    let (ed, window) = pal_window(cx, "# A\nbody\n");
    ed.update(cx, |ed, _| {
        assert!(ed.find.is_none(), "前置：查找条未开");
        // #112 P1：构造点会从 settings 恢复 MRU（持久化接线后
        // 本机/前序测试可能留档）——单测前置显式清零回到旧假设。
        ed.palette_recent = crate::command_groups::RecentRing::new(4);
        assert!(!ed.palette_recent.contains(Cmd::OpenFind), "前置：无记账");
    });
    let _ = window.update(cx, |_host, window, cx| {
        let mut e = ctrl("p");
        e.keystroke.modifiers.shift = true;
        press(&ed, window, cx, e);
        for ch in ["查", "找"] {
            press(&ed, window, cx, key(ch, Some(ch)));
        }
    });
    ed.update(cx, |ed, _| {
        let pal = ed.palette.as_ref().expect("面板开");
        // 「查找」在命令标题中唯一命中 OpenFind（替换条无此子序列）。
        assert_eq!(
            pal.items,
            vec![Cmd::OpenFind],
            "「查找」唯一命中 OpenFind（选中 0 即目标）"
        );
    });
    let _ = window.update(cx, |_host, window, cx| {
        press(&ed, window, cx, key("enter", None));
    });
    ed.update(cx, |ed, _| {
        assert!(ed.palette.is_none(), "Enter 执行后面板关（选完即关）");
        assert!(ed.find.is_some(), "OpenFind 执行 → 查找条开（状态变化）");
        assert!(
            ed.palette_recent.contains(Cmd::OpenFind),
            "#183 A-3：面板执行记入最近使用"
        );
    });
    // 重开面板：最近使用段置顶该命令（记账的消费面）。
    let _ = window.update(cx, |_host, window, cx| {
        let mut e = ctrl("p");
        e.keystroke.modifiers.shift = true;
        press(&ed, window, cx, e);
    });
    ed.update(cx, |ed, _| {
        let pal = ed.palette.as_ref().expect("重开面板");
        assert_eq!(
            pal.items.first(),
            Some(&Cmd::OpenFind),
            "最近使用段置顶（扁平序首位）"
        );
    });
}

/// 跳行框闭环：Ctrl+G 开 → 键入行号 → Enter 跳转（光标可见移动）→
/// 越界 clamp → 非法输入不跳不关 → Esc 关闭不移动。
#[gpui::test]
fn goto_open_type_enter_jump_and_edges(cx: &mut gpui::TestAppContext) {
    let (ed, window) = pal_window(cx, "L0\nL1\nL2\nL3\nL4");
    // Ctrl+G 开框 → 键入 3 → Enter → 跳第 3 行（0 基 2）。
    let _ = window.update(cx, |_host, window, cx| {
        press(&ed, window, cx, ctrl("g"));
        press(&ed, window, cx, key("3", Some("3")));
        press(&ed, window, cx, key("enter", None));
    });
    ed.update(cx, |ed, _| {
        assert!(ed.goto.is_none(), "Enter 跳转后框关");
        assert_eq!(ed.buffer.cursor().line, 2, "跳到第 3 行（0 基 2）");
    });
    // 越界：999 → clamp 到末行。
    let _ = window.update(cx, |_host, window, cx| {
        press(&ed, window, cx, ctrl("g"));
        for ch in ["9", "9", "9"] {
            press(&ed, window, cx, key(ch, Some(ch)));
        }
        press(&ed, window, cx, key("enter", None));
    });
    ed.update(cx, |ed, _| {
        assert!(ed.goto.is_none(), "越界跳转同样关框");
        assert_eq!(ed.buffer.cursor().line, 4, "越界 clamp 到末行（0 基 4）");
    });
    // 非法输入：Enter 不跳不关（提示行可见——不静默吞）。
    let _ = window.update(cx, |_host, window, cx| {
        press(&ed, window, cx, ctrl("g"));
        for ch in ["a", "b"] {
            press(&ed, window, cx, key(ch, Some(ch)));
        }
        press(&ed, window, cx, key("enter", None));
    });
    ed.update(cx, |ed, _| {
        let goto = ed.goto.as_ref().expect("非法输入不关框（改提示）");
        assert_eq!(goto.input, "ab");
        assert_eq!(goto.hint, "未跳——请输入有效行号");
        assert_eq!(ed.buffer.cursor().line, 4, "光标不动（不跳）");
    });
    // Esc 关闭不移动。
    let _ = window.update(cx, |_host, window, cx| {
        press(&ed, window, cx, key("escape", None));
    });
    ed.update(cx, |ed, _| {
        assert!(ed.goto.is_none(), "Esc 关框");
        assert_eq!(ed.buffer.cursor().line, 4, "关框不移动光标");
    });
}
/// #112 P1（MRU 持久化）：跨「重启」回放——settings 往返 + restore 序 +
/// 落盘序三段真链（**注入形态**：直接走 `_for_test` 目录注入，不动进程
/// env——并行套件下 XDG 重定向会让同进程其他 Editor 构造测试读到污染窗，
/// 曾间歇红；构造点全链（restore_palette_recent）与段2同构，由
/// 段2的注入等价覆盖）。
#[gpui::test]
fn palette_recent_survives_restart(cx: &mut gpui::TestAppContext) {
    let cfg = std::env::temp_dir().join(format!("mdc-112p1-mru-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cfg);
    std::fs::create_dir_all(&cfg).unwrap();
    // 段1：settings 层往返（注入形态）。
    crate::settings::save_palette_recent_for_test(
        &cfg,
        &["Save".to_string(), "OpenFind".to_string()],
    );
    let names = crate::settings::load_palette_recent_from_test(&cfg);
    assert_eq!(
        names,
        vec!["Save".to_string(), "OpenFind".to_string()],
        "档往返"
    );

    // 段2：restore 序真链（与构造点 restore_palette_recent 同构——
    // 稳定名反解 + touch 顺序回放到 RecentRing）。
    let (ed, _window) = pal_window(cx, "# t\n正文");
    ed.update(cx, |this, _| {
        this.palette_recent = RecentRing::new(palette_view::RECENT_CAPACITY);
        for name in names.clone() {
            if let Some(c) = commands::from_key_name(&name) {
                this.palette_recent.touch(c);
            }
        }
    });
    assert!(
        ed.read_with(cx, |this, _| this.palette_recent.contains(Cmd::Save)),
        "恢复 Save（最新在前档）"
    );
    assert!(
        ed.read_with(cx, |this, _| this.palette_recent.contains(Cmd::OpenFind)),
        "恢复 OpenFind"
    );
    // 面板空查询 → 最近段在最前且含两条（palette_sections 真链）。
    let sections = crate::palette_view::palette_sections(
        "",
        &ed.read_with(cx, |this, _| {
            let mut r = crate::command_groups::RecentRing::new(4);
            for c in this.palette_recent.iter() {
                r.touch(c);
            }
            r
        }),
    );
    assert!(matches!(
        sections.first().map(|s| s.group),
        Some(crate::command_groups::Group::Recent)
    ));
    assert_eq!(sections[0].entries.len(), 2, "最近段两条");

    // 段3：touch 后的落盘序真链（persist 同构——names 化后写注入档）。
    let names_after: Vec<String> = ed.read_with(cx, |this, _| {
        let mut r = this.palette_recent.clone();
        r.touch(Cmd::GotoLine);
        r.iter().map(|c| c.key_name().to_string()).collect()
    });
    crate::settings::save_palette_recent_for_test(&cfg, &names_after);
    let after = std::fs::read_to_string(cfg.join("settings.json")).unwrap();
    assert!(
        after.contains("GotoLine"),
        "touch 即落盘（write-through 序）"
    );
    assert!(after.contains("Save"), "既有项保留");

    let _ = std::fs::remove_dir_all(&cfg);
}

/// #112 P1（命中高亮）：palette_sections 高亮集贯通（查询 → Entry.highlights）。
#[gpui::test]
fn palette_sections_carry_highlights(cx: &mut gpui::TestAppContext) {
    let (ed, _window) = pal_window(cx, "# t\n正文");
    let recent = ed.read_with(cx, |this, _| {
        let mut r = crate::command_groups::RecentRing::new(4);
        for c in this.palette_recent.iter() {
            r.touch(c);
        }
        r
    });
    let sections = crate::palette_view::palette_sections("移左", &recent);
    // 「移左」命中「移动：左」——该 Entry 的 highlights = [0, 3]。
    let hit = sections
        .iter()
        .flat_map(|s| s.entries.iter())
        .find(|e| e.cmd == Cmd::MoveLeft)
        .expect("移左 命中 移动：左");
    assert_eq!(hit.highlights, vec![0, 3], "命中下标集贯通到条目");
    // 空查询无高亮（等价展示）。
    let all = crate::palette_view::palette_sections("", &recent);
    assert!(
        all.iter()
            .flat_map(|s| s.entries.iter())
            .all(|e| e.highlights.is_empty()),
        "空查询零高亮"
    );
}
