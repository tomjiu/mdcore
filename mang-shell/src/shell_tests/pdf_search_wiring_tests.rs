//! #47 巨石拆解批③：原 shell.rs 内嵌 `pdf_search_wiring_tests` 纯迁出（零行为变化）。

use super::*;

/// 测试宿主（ShellApp 不作根视图但**必须进渲染树**——#422 的键
/// 派发走真实管线：焦点路径经 shell-root → editor_area → 页视图
/// 的 track_focus 节点；空 div 宿主会把整棵树挡在派发树外）。
struct WiringHost(Entity<ShellApp>);

impl gpui::Render for WiringHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div().size_full().child(self.0.clone())
    }
}

/// smoke.pdf 夹具字节（两页——pdf_pages/#422 视图级测试同源夹具）。
fn smoke_pdf_bytes() -> Vec<u8> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.pdf");
    std::fs::read(&p).unwrap_or_else(|e| panic!("夹具不可读（{}）：{e}", p.display()))
}

/// #422 接线面：页面视图下 Ctrl+F（真实键击管线）→ 开页内查找条
/// （**不切视图**——#297 折中升级为正式能力）；Esc 关条清态；菜单
/// 路径（dispatch_editor_cmd）同源。引擎门控：无 libpdfium 时打开
/// 本身失败（错误进 notice）→ SKIP 横幅（AGENTS 13.1——不伪装通过）。
#[gpui::test]
fn pdf_view_ctrl_f_opens_in_page_search(cx: &mut gpui::TestAppContext) {
    let dir = std::env::temp_dir().join(format!("mdc-422-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pdf = dir.join("s.pdf");
    std::fs::write(&pdf, smoke_pdf_bytes()).unwrap();

    // 渲染面全局件（main.rs 生产同构——空 div 宿主时代不渲染整树
    // 用不到；#422 键派发需要整树，theme provider 必须在位）。
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let mut slot: Option<Entity<ShellApp>> = None;
    let window = cx.add_window(|window, cx| {
        let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
        slot = Some(shell.clone());
        WiringHost(shell)
    });
    let shell = slot.expect("宿主构造即置");

    // 生产同构链：open_document（open_document_tab + 落焦——#422
    // 后页视图态 pdf 标签落焦页视图，键事件可达）。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_document(pdf.clone(), window, cx);
        });
    });
    // 引擎门控：无 libpdfium → pdf_doc::load 即失败（open 走 Failed
    // 分支，无标签无槽，错误文案进 notice）→ SKIP 横幅 + return；
    // 槽在位 = 引擎健康（tab 开成功 ⇔ 引擎开成功——同一定位序）。
    let open_notice = window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, _| {
                shell.notice_board.message().map(str::to_owned)
            })
        })
        .expect("窗口更新");
    let view = cx.read(|app| shell.read(app).pdf_pages.as_ref().map(|(_, e)| e.clone()));
    let view = match (view, open_notice) {
        (None, Some(msg)) if msg.contains("libpdfium") => {
            eprintln!(
                "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                     本机无 libpdfium——#422 Ctrl+F 接线 e2e 未真跑\n  \
                     （pdf 打开面即失败：引擎绑定缺库）。\n  \
                     有 libpdfium 的环境（MDC_PDFIUM_LIB=<库路径> \
                     cargo test）全量执行。\n═══════════════════════════\n"
            );
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        (None, other) => {
            panic!("pdf 标签未开（夹具/真缺陷，非环境缺件）：{other:?}")
        }
        (Some(view), _) => view,
    };
    // 前置：页面视图态（缺省开）。
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, _| {
                assert!(shell.pdf_view_mode, "前置：pdf 标签缺省页面视图");
            });
        })
        .expect("前置");
    // ① 键盘 Ctrl+F（真实派发管线——main.rs 烟雾同构的
    // update_window + dispatch_event）：不切视图，开页内查找条
    // （#297 的「自动切文本视图」在此必须不再发生）。焦点已落页
    // 视图；先强制一帧绘制（派发树在位——键事件按焦点路径派发）。
    let any: gpui::AnyWindowHandle = window.clone().into();
    cx.update_window(any, |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .expect("预绘一帧");
    cx.update_window(any, |_, window, cx| {
        window.dispatch_event(
            gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                keystroke: gpui::Keystroke {
                    modifiers: gpui::Modifiers {
                        control: true,
                        ..Default::default()
                    },
                    key: "f".into(),
                    key_char: None,
                },
                is_held: false,
                prefer_character_input: false,
            }),
            cx,
        );
    })
    .expect("派发 Ctrl+F");
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                assert!(
                    shell.pdf_view_mode,
                    "#422：Ctrl+F 不切文本视图（#297 折中已升级）"
                );
                assert!(view.read(cx).search_open(), "#422：页内查找条应开");
            });
        })
        .expect("断言");
    // ② Esc（查找输入行焦点面）：关条清态。重绘一帧让焦点迁移
    // 落进派发树（open_search 聚焦 search_focus）。
    cx.update_window(any, |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .expect("预绘一帧");
    cx.update_window(any, |_, window, cx| {
        window.dispatch_event(
            gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                keystroke: gpui::Keystroke {
                    modifiers: Default::default(),
                    key: "escape".into(),
                    key_char: None,
                },
                is_held: false,
                prefer_character_input: false,
            }),
            cx,
        );
    })
    .expect("派发 Esc");
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                assert!(!view.read(cx).search_open(), "Esc 关条清态");
                assert!(shell.pdf_view_mode, "关条不动视图模式");
            });
        })
        .expect("断言");
    // ③ 菜单路径（「编辑 → 查找」同源分流——dispatch_editor_cmd）：
    // 同样开页内查找条不切视图。
    window
        .update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.dispatch_editor_cmd(crate::editor::commands::Cmd::OpenFind, window, cx);
            });
        })
        .expect("菜单路径");
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                assert!(shell.pdf_view_mode, "菜单查找同源不切视图");
                assert!(view.read(cx).search_open(), "菜单查找开页内查找条");
            });
        })
        .expect("断言");

    let _ = std::fs::remove_dir_all(&dir);
}

/// #51 D4 接线面：页面视图内**文本选择与复制**全链——真实鼠标管线
/// （按下 → 拖动 → 松开，平台事件派发）→ 选择层后台提取 → 选择文
/// 本就绪 → 真实键击管线 Ctrl+C（keymap E18 换绑跟随路由）→ 测试
/// 平台剪贴板读回逐字一致 → Esc 清选择。引擎门控同 Ctrl+F 测试。
#[gpui::test]
fn pdf_view_ctrl_c_copies_text_selection(cx: &mut gpui::TestAppContext) {
    // 语料：mixed_features.pdf（D1 内容对照夹具——必有文本层）。
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/display_corpus/mixed_features.pdf");
    let bytes = std::fs::read(&corpus).unwrap_or_else(|e| panic!("语料不可读（{e}）"));
    let dir = std::env::temp_dir().join(format!("mdc-d4sel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pdf = dir.join("m.pdf");
    let _ = std::fs::write(&pdf, bytes);

    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let mut slot: Option<Entity<ShellApp>> = None;
    let window = cx.add_window(|window, cx| {
        let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
        slot = Some(shell.clone());
        WiringHost(shell)
    });
    let shell = slot.expect("宿主构造即置");
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_document(pdf.clone(), window, cx);
        });
    });
    let open_notice = window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, _| {
                shell.notice_board.message().map(str::to_owned)
            })
        })
        .expect("窗口更新");
    let view = cx.read(|app| shell.read(app).pdf_pages.as_ref().map(|(_, e)| e.clone()));
    let view = match (view, open_notice) {
        (None, Some(msg)) if msg.contains("libpdfium") => {
            eprintln!(
                "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                     本机无 libpdfium——#51 D4 选择复制 e2e 未真跑。\n  \
                     有 libpdfium 的环境（MDC_PDFIUM_LIB=<库路径> \
                     cargo test）全量执行。\n═══════════════════════════\n"
            );
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        (None, other) => {
            panic!("pdf 标签未开（夹具/真缺陷，非环境缺件）：{other:?}")
        }
        (Some(view), _) => view,
    };

    // ① 首屏位图就绪（真实渲染管线——泵同 pdf_display_verify 口径）。
    let any: gpui::AnyWindowHandle = window.clone().into();
    let mut visual = gpui::VisualTestContext::from_window(any.clone(), cx);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(20));
        cx.background_executor.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(2));
        visual.update(|window, app| window.draw(app).clear(app));
        if view.read_with(&visual, |v, _| v.page_ready_for_test(0)) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "D4 首屏位图泵超时（真缺陷不放过）"
        );
    }

    // ② 真实鼠标拖选：页 0 槽内横向拖选（debug_bounds 定位真实槽
    // 几何——title bar 等树内偏移自然计入）。
    let b = visual
        .debug_bounds("pdf-visible-page-0")
        .expect("页 0 槽真实渲染（debug_selector 观测面）");
    let (bx, by, bw, bh) = (
        f32::from(b.origin.x),
        f32::from(b.origin.y),
        f32::from(b.size.width),
        f32::from(b.size.height),
    );
    let pt = |x: f32, y: f32| gpui::point(gpui::px(x), gpui::px(y));
    let down = |cx: &mut gpui::TestAppContext, x: f32, y: f32| {
        cx.update_window(any.clone(), |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                    button: gpui::MouseButton::Left,
                    position: pt(x, y),
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                cx,
            );
        })
        .expect("按下派发");
    };
    let move_to = |cx: &mut gpui::TestAppContext, x: f32, y: f32| {
        cx.update_window(any.clone(), |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                    position: pt(x, y),
                    pressed_button: Some(gpui::MouseButton::Left),
                    modifiers: Default::default(),
                }),
                cx,
            );
        })
        .expect("拖动派发");
    };
    let up = |cx: &mut gpui::TestAppContext, x: f32, y: f32| {
        cx.update_window(any.clone(), |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                    button: gpui::MouseButton::Left,
                    position: pt(x, y),
                    modifiers: Default::default(),
                    click_count: 1,
                }),
                cx,
            );
        })
        .expect("松开派发");
    };
    // 拖选带：x 跨 15% → 85%（覆盖文本主列），y 跨 20% → 60%
    //（避开页头页脚装饰，覆盖正文前中段）。
    down(cx, bx + bw * 0.15, by + bh * 0.20);
    move_to(cx, bx + bw * 0.85, by + bh * 0.60);
    up(cx, bx + bw * 0.85, by + bh * 0.60);
    // 诊断断言：拖选三事件后选择会话必须在位（鼠标管线接线直接
    // 证据——失败即派发链断点定位面）。
    view.read_with(&visual, |v, _| {
        assert!(
            v.has_selection_for_test(),
            "拖选后选择会话必须在位（鼠标事件 → begin/move/finish 接线断点）"
        );
    });

    // ③ 选择文本就绪（层后台提取 + 16ms 轮询 drain 泵）。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(20));
        cx.background_executor.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(2));
        visual.update(|window, app| window.draw(app).clear(app));
        if view.read_with(&visual, |v, _| v.selection_text_for_test().is_some()) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "D4 选择文本泵超时（层提取未就绪——真缺陷不放过）"
        );
    }
    let selected = view.read_with(&visual, |v, _| {
        v.selection_text_for_test().unwrap().to_owned()
    });
    assert!(!selected.trim().is_empty(), "拖选带必须覆盖到文本");
    eprintln!(
        "#51 D4 ✓ 拖选就绪：{} 字符（首 40：{:?}）",
        selected.chars().count(),
        selected.chars().take(40).collect::<String>()
    );

    // ④ 真实键击管线 Ctrl+C（换绑跟随路由——shell 分支 → 视图复制
    // → 测试平台剪贴板）。
    cx.update_window(any.clone(), |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .expect("预绘一帧");
    cx.update_window(any.clone(), |_, window, cx| {
        window.dispatch_event(
            gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                keystroke: gpui::Keystroke {
                    modifiers: gpui::Modifiers {
                        control: true,
                        ..Default::default()
                    },
                    key: "c".into(),
                    key_char: None,
                },
                is_held: false,
                prefer_character_input: false,
            }),
            cx,
        );
    })
    .expect("派发 Ctrl+C");
    let clipboard = cx.read(|app| app.read_from_clipboard());
    let copied = clipboard.and_then(|item| item.text()).unwrap_or_default();
    assert_eq!(
        copied, selected,
        "剪贴板读回必须与选择文本逐字一致（#51 D4 复制链）"
    );
    eprintln!(
        "#51 D4 ✓ Ctrl+C 复制：剪贴板逐字一致（{} 字符）",
        copied.chars().count()
    );

    // ⑤ Esc 清选择（真实键击管线 → 视图键域）。
    cx.update_window(any.clone(), |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .expect("预绘一帧");
    cx.update_window(any.clone(), |_, window, cx| {
        window.dispatch_event(
            gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                keystroke: gpui::Keystroke {
                    modifiers: Default::default(),
                    key: "escape".into(),
                    key_char: None,
                },
                is_held: false,
                prefer_character_input: false,
            }),
            cx,
        );
    })
    .expect("派发 Esc");
    visual.update(|window, app| window.draw(app).clear(app));
    view.read_with(&visual, |v, _| {
        assert!(
            !v.has_selection_for_test() && v.selection_text_for_test().is_none(),
            "Esc 清选择（会话与文本双清零）"
        );
    });
    eprintln!("#51 D4 ✓ Esc 清选择");

    let _ = std::fs::remove_dir_all(&dir);
}

/// #51 D4d 接线面：**跨页拖选与复制**全链——真实鼠标管线（页 0
/// 按下 → 跨槽拖入页 1 → 页 1 松开）→ 触达页层串行泵取（队列
/// 续泵）→ 两页文本读序拼接（页间断行恰一条）→ Ctrl+C 剪贴板
/// 逐字一致 → Esc 清。语料 zh_embedded_font.pdf（两页均有文本
/// 层——页 1 头部「中文嵌入字体测试 · 第 2 页」/页 0 尾行
/// 「mdcore 显示语料」为跨页读序锚点）。引擎门控同 D4a。
#[gpui::test]
fn pdf_view_ctrl_c_copies_cross_page_selection(cx: &mut gpui::TestAppContext) {
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/display_corpus/zh_embedded_font.pdf");
    let bytes = std::fs::read(&corpus).unwrap_or_else(|e| panic!("语料不可读（{e}）"));
    let dir = std::env::temp_dir().join(format!("mdc-d4d-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pdf = dir.join("m.pdf");
    let _ = std::fs::write(&pdf, bytes);

    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let mut slot: Option<Entity<ShellApp>> = None;
    let window = cx.add_window(|window, cx| {
        let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
        slot = Some(shell.clone());
        WiringHost(shell)
    });
    let shell = slot.expect("宿主构造即置");
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_document(pdf.clone(), window, cx);
        });
    });
    let view = cx.read(|app| shell.read(app).pdf_pages.as_ref().map(|(_, e)| e.clone()));
    let view = match view {
        Some(v) => v,
        None => {
            eprintln!(
                "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                     本机无 libpdfium——#51 D4d 跨页拖选 e2e 未真跑。\n  \
                     有 libpdfium 的环境（MDC_PDFIUM_LIB=<库路径> \
                     cargo test）全量执行。\n═════════════════════════\n"
            );
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
    };

    // ① 首屏位图就绪（真实渲染管线——泵同 D4a 口径）。
    let any: gpui::AnyWindowHandle = window.clone().into();
    let mut visual = gpui::VisualTestContext::from_window(any.clone(), cx);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(20));
        cx.background_executor.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(2));
        visual.update(|window, app| window.draw(app).clear(app));
        if view.read_with(&visual, |v, _| v.page_ready_for_test(0)) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "D4d 首屏位图泵超时（真缺陷不放过）"
        );
    }

    // ② 中途滚动把页界拉进同帧视口：页 1 顶对齐视口顶（scroll_
    // to_page——页空间锚点）再回滚 300px——页 0 尾带 + 页 1 头带
    // 同帧可见（跨页拖选产品手势 = 页界附近拖；gpui::list 槽实例域
    // = 视口 ± overdraw，槽高超视口时两侧不同帧存在则跨页手势
    // 不可派发）。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    cx.update(|app| view.update(app, |v, _| v.scroll_to_page(1)));
    loop {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(20));
        cx.background_executor.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(2));
        visual.update(|window, app| window.draw(app).clear(app));
        if view.read_with(&visual, |v, _| v.top_visible_page() == 1) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "D4d 页 1 对齐泵超时（真缺陷不放过）"
        );
    }
    cx.update(|app| view.update(app, |v, _| v.scroll_by_for_test(-300.)));
    visual.update(|window, app| window.draw(app).clear(app));
    // 两页槽真实几何（页界两侧同帧在位——debug_selector 观测面）。
    let b0 = visual
        .debug_bounds("pdf-visible-page-0")
        .expect("页 0 槽真实渲染（回滚后尾带可见）");
    let b1 = visual
        .debug_bounds("pdf-visible-page-1")
        .expect("页 1 槽真实渲染（回滚后头带可见——跨页拖选两端同帧在位）");
    let rect = |b: &gpui::Bounds<gpui::Pixels>| {
        (
            f32::from(b.origin.x),
            f32::from(b.origin.y),
            f32::from(b.size.width),
            f32::from(b.size.height),
        )
    };
    let (bx0, _by0, bw0, _bh0) = rect(&b0);
    let (bx1, by1, bw1, _bh1) = rect(&b1);
    let pt = |x: f32, y: f32| gpui::point(gpui::px(x), gpui::px(y));
    let down = |cx: &mut gpui::TestAppContext, x: f32, y: f32| {
        cx.update_window(any.clone(), |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                    button: gpui::MouseButton::Left,
                    position: pt(x, y),
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                cx,
            );
        })
        .expect("按下派发");
    };
    let move_to = |cx: &mut gpui::TestAppContext, x: f32, y: f32| {
        cx.update_window(any.clone(), |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                    position: pt(x, y),
                    pressed_button: Some(gpui::MouseButton::Left),
                    modifiers: Default::default(),
                }),
                cx,
            );
        })
        .expect("拖动派发");
    };
    let up = |cx: &mut gpui::TestAppContext, x: f32, y: f32| {
        cx.update_window(any.clone(), |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                    button: gpui::MouseButton::Left,
                    position: pt(x, y),
                    modifiers: Default::default(),
                    click_count: 1,
                }),
                cx,
            );
        })
        .expect("松开派发");
    };
    // 跨页拖选带：页 0 尾带（页界上方 130px——尾部文本行区）按下
    // → 跨槽拖入页 1 头带（页界下方 200px——顶边距 72pt 下探到首
    // 文本行区；A4 适宽槽 200px ≈ 103pt——头部行深度实测 > 57pt）松
    // 开。角域语义：页 0 = 锚点 → 页底、页 1 = 页顶 → 游标。页界 y
    // = 页 1 顶（槽邻接）。
    let boundary_y = by1;
    down(cx, bx0 + bw0 * 0.15, boundary_y - 130.);
    move_to(cx, bx1 + bw1 * 0.85, boundary_y + 200.);
    up(cx, bx1 + bw1 * 0.85, boundary_y + 200.);
    view.read_with(&visual, |v, _| {
        assert!(
            v.has_selection_for_test(),
            "跨页拖选后选择会话必须在位（页 0 按下 → 页 1 松开跨槽接线）"
        );
    });

    // ③ 两页层就绪 + 拼接文本（串行队列续泵——一页一线程）。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(20));
        cx.background_executor.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(2));
        visual.update(|window, app| window.draw(app).clear(app));
        if view.read_with(&visual, |v, _| v.selection_text_for_test().is_some()) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "D4d 跨页选择文本泵超时（两页层串行提取未就绪——真缺陷不放过）"
        );
    }
    let selected = view.read_with(&visual, |v, _| {
        v.selection_text_for_test().unwrap().to_owned()
    });
    // 跨页读序双锚点：页 0 尾行（角域 → 页底）在前、页 1 头部行
    // （角域 ← 页顶）在后；页 0 头部行（锚点上方）不得入选（角域
    // 语义反证）。
    let pos_tail = selected
        .find("mdcore 显示语料")
        .expect("页 0 尾行「mdcore 显示语料」必须入选（锚点 → 页底角域）");
    let pos_p1head = selected
        .find("第 2 页")
        .expect("页 1 头部「第 2 页」必须入选（页顶 → 游标角域——跨页触达）");
    assert!(
        pos_tail < pos_p1head,
        "读序拼接恒 lo → hi（页 0 尾在前、页 1 头在后；实得 {pos_tail} vs {pos_p1head}）"
    );
    assert!(
        !selected.contains("第 1 页"),
        "页 0 头部行在锚点上方——角域语义不得入选（倒推反证）"
    );
    // 页间断行存在性（单侧守卫——直接拼接无分隔即红）。
    assert!(
        !selected.contains("语料中文"),
        "页界必须有断行分隔（直接拼接 = join 缺失）"
    );
    eprintln!(
        "#51 D4d ✓ 跨页拖选就绪：{} 字符（页 0 尾 + 页界 + 页 1 头读序拼接）",
        selected.chars().count()
    );

    // ④ 真实键击管线 Ctrl+C → 剪贴板逐字一致。
    cx.update_window(any.clone(), |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .expect("预绘一帧");
    cx.update_window(any.clone(), |_, window, cx| {
        window.dispatch_event(
            gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                keystroke: gpui::Keystroke {
                    modifiers: gpui::Modifiers {
                        control: true,
                        ..Default::default()
                    },
                    key: "c".into(),
                    key_char: None,
                },
                is_held: false,
                prefer_character_input: false,
            }),
            cx,
        );
    })
    .expect("派发 Ctrl+C");
    let clipboard = cx.read(|app| app.read_from_clipboard());
    let copied = clipboard.and_then(|item| item.text()).unwrap_or_default();
    assert_eq!(
        copied, selected,
        "剪贴板读回必须与跨页选择文本逐字一致（#51 D4d 复制链）"
    );
    eprintln!(
        "#51 D4d ✓ Ctrl+C 复制：剪贴板逐字一致（{} 字符）",
        copied.chars().count()
    );

    // ⑤ Esc 清选择（真实键击管线 → 视图键域）。
    cx.update_window(any.clone(), |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .expect("预绘一帧");
    cx.update_window(any.clone(), |_, window, cx| {
        window.dispatch_event(
            gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                keystroke: gpui::Keystroke {
                    modifiers: Default::default(),
                    key: "escape".into(),
                    key_char: None,
                },
                is_held: false,
                prefer_character_input: false,
            }),
            cx,
        );
    })
    .expect("派发 Esc");
    visual.update(|window, app| window.draw(app).clear(app));
    view.read_with(&visual, |v, _| {
        assert!(
            !v.has_selection_for_test() && v.selection_text_for_test().is_none(),
            "Esc 清跨页选择（会话与文本双清零）"
        );
    });
    eprintln!("#51 D4d ✓ Esc 清跨页选择");

    let _ = std::fs::remove_dir_all(&dir);
}

/// #421 尾接线面：**超宽档横向平移导航**全链——真实滚轮管线
/// （触控板横扫 dx / Shift+纵向）+ 中键拖拽（按下 → 拖 → 松开）
/// → pan 状态推进 + 页槽真实位移（debug_bounds 视口坐标断言）→
/// 两端硬钳制 → 缩放档切回适宽 pan 复位。无溢出（适宽基线）滚轮
/// 平移不吞（pan 恒 0）。语料 mixed_features.pdf（A4 适宽 → 步进
/// 125% 超视口）。引擎门控同 D4a。
#[gpui::test]
fn pdf_view_zoom_pan_navigation(cx: &mut gpui::TestAppContext) {
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/display_corpus/mixed_features.pdf");
    let bytes = std::fs::read(&corpus).unwrap_or_else(|e| panic!("语料不可读（{e}）"));
    let dir = std::env::temp_dir().join(format!("mdc-421pan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pdf = dir.join("m.pdf");
    let _ = std::fs::write(&pdf, bytes);

    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let mut slot: Option<Entity<ShellApp>> = None;
    let window = cx.add_window(|window, cx| {
        let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
        slot = Some(shell.clone());
        WiringHost(shell)
    });
    let shell = slot.expect("宿主构造即置");
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_document(pdf.clone(), window, cx);
        });
    });
    let view = cx.read(|app| shell.read(app).pdf_pages.as_ref().map(|(_, e)| e.clone()));
    let view = match view {
        Some(v) => v,
        None => {
            eprintln!(
                "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                     本机无 libpdfium——#421 横向平移 e2e 未真跑。\n  \
                     有 libpdfium 的环境（MDC_PDFIUM_LIB=<库路径> \
                     cargo test）全量执行。\n═════════════════════════\n"
            );
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
    };

    // ① 首屏位图就绪（真实渲染管线——泵同 D4a 口径）。
    let any: gpui::AnyWindowHandle = window.clone().into();
    let mut visual = gpui::VisualTestContext::from_window(any.clone(), cx);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(20));
        cx.background_executor.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(2));
        visual.update(|window, app| window.draw(app).clear(app));
        if view.read_with(&visual, |v, _| v.page_ready_for_test(0)) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "421pan 首屏位图泵超时（真缺陷不放过）"
        );
    }
    // 适宽基线：无溢出（FitWidth 量子化伪溢出 32px 经量子门控归
    // 零——平移面不启用，滚轮横扫不吞照常冒泡）。
    view.read_with(&visual, |v, _| {
        assert!(
            v.pan_overflow_for_test() < 0.5,
            "适宽基线无溢出（实得 {}）",
            v.pan_overflow_for_test()
        );
        assert_eq!(v.pan_x_for_test(), 0., "基线 pan = 0");
    });

    // ② 步进放大 125%（Ctrl+= 真键路径）→ 光栅宽超视口 → 溢出面
    // 启用。A4 适宽 ≈ 视口宽 → 125% 溢出 ≈ 25% 视口宽。
    cx.update(|app| {
        view.update(app, |v, cx| v.dispatch_zoom_key("=", true, false, cx));
    });
    visual.update(|window, app| window.draw(app).clear(app));
    let overflow = view.read_with(&visual, |v, _| v.pan_overflow_for_test());
    assert!(
        overflow > 50.,
        "125% 档溢出面必须启用（实得 {overflow}——夹具/视口几何先查）"
    );
    assert_eq!(
        view.read_with(&visual, |v, _| v.pan_x_for_test()),
        0.,
        "溢出启用时基线 pan 仍 0（左缘对齐）"
    );
    let base = visual
        .debug_bounds("pdf-visible-page-0")
        .expect("页 0 槽真实渲染");
    let base_x = f32::from(base.origin.x);

    // ③ 触控板横扫（真实滚轮管线）：dx = +200 → pan = 200 → 页槽
    // 真实左移 200（视口坐标断言——debug_selector 观测面）。派发
    // 位置 = 视口域中心（125% 档页槽纵向超出视口——槽中心已出
    // 探针域；滚轮命中域 = 视口探针，槽内位置无关）。
    let viewport_b = visual
        .debug_bounds("pdf-pages-list")
        .expect("视口观测面在位");
    let (cx0, cy0) = (
        f32::from(viewport_b.origin.x) + f32::from(viewport_b.size.width) / 2.,
        f32::from(viewport_b.origin.y) + f32::from(viewport_b.size.height) / 2.,
    );
    let scroll = |cx: &mut gpui::TestAppContext, dx: f32, dy: f32, shift: bool| {
        cx.update_window(any.clone(), |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::Scroll(gpui::ScrollEvent {
                    position: gpui::point(gpui::px(cx0), gpui::px(cy0)),
                    delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(dx), gpui::px(dy))),
                    modifiers: gpui::Modifiers {
                        shift,
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                cx,
            );
        })
        .expect("滚轮派发");
    };
    scroll(cx, 200., 0., false);
    visual.update(|window, app| window.draw(app).clear(app));
    assert!(
        (view.read_with(&visual, |v, _| v.pan_x_for_test()) - 200.).abs() < 1.,
        "横扫 dx=+200 → pan=200（实得 {}）",
        view.read_with(&visual, |v, _| v.pan_x_for_test())
    );
    let shifted = visual
        .debug_bounds("pdf-visible-page-0")
        .expect("平移后页 0 槽仍在观测面");
    assert!(
        (f32::from(shifted.origin.x) - (base_x - 200.)).abs() < 2.,
        "页槽真实左移 200（实得 x {} vs 基线 {}-200）",
        f32::from(shifted.origin.x),
        base_x
    );
    eprintln!("#421 ✓ 横扫平移：pan 200 / 槽位移 200（基线 x={base_x}）");

    // ④ 硬钳制：大幅右扫超溢出 → 钳到 overflow（右缘对齐——两端
    // 硬边界不越界）。
    scroll(cx, 5000., 0., false);
    visual.update(|window, app| window.draw(app).clear(app));
    let pan_max = view.read_with(&visual, |v, _| v.pan_x_for_test());
    assert!(
        (pan_max - overflow).abs() < 1.,
        "右扫钳到溢出宽（实得 {pan_max} vs {overflow}）"
    );

    // ⑤ Shift+纵向滚轮横滚惯例（真键族手势）：dy = -100（上滚）→
    // pan 减 100（左向——键盘族横滚与触控板同源方向）。
    scroll(cx, 0., -100., true);
    visual.update(|window, app| window.draw(app).clear(app));
    assert!(
        (view.read_with(&visual, |v, _| v.pan_x_for_test()) - (pan_max - 100.)).abs() < 1.,
        "Shift+上滚 100 → pan -100（实得 {}）",
        view.read_with(&visual, |v, _| v.pan_x_for_test())
    );

    // ⑥ 中键拖拽（真鼠标管线）：按下 → 右拖 60（内容随游标右移 =
    // pan 减 60）→ 左拖回 60 → 松开收尾（会话清）。
    let mid_down = |cx: &mut gpui::TestAppContext, x: f32| {
        cx.update_window(any.clone(), |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                    button: gpui::MouseButton::Middle,
                    position: gpui::point(gpui::px(x), gpui::px(cy0)),
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                cx,
            );
        })
        .expect("中键按下派发");
    };
    let mid_move = |cx: &mut gpui::TestAppContext, x: f32| {
        cx.update_window(any.clone(), |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                    position: gpui::point(gpui::px(x), gpui::px(cy0)),
                    pressed_button: Some(gpui::MouseButton::Middle),
                    modifiers: Default::default(),
                }),
                cx,
            );
        })
        .expect("中键拖动派发");
    };
    let mid_up = |cx: &mut gpui::TestAppContext, x: f32| {
        cx.update_window(any.clone(), |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                    button: gpui::MouseButton::Middle,
                    position: gpui::point(gpui::px(x), gpui::px(cy0)),
                    modifiers: Default::default(),
                    click_count: 1,
                }),
                cx,
            );
        })
        .expect("中键松开派发");
    };
    let pan_before_drag = view.read_with(&visual, |v, _| v.pan_x_for_test());
    mid_down(cx, cx0);
    mid_move(cx, cx0 + 60.);
    visual.update(|window, app| window.draw(app).clear(app));
    assert!(
        (view.read_with(&visual, |v, _| v.pan_x_for_test()) - (pan_before_drag - 60.)).abs() < 1.,
        "中键右拖 60 → 内容随游标右移（pan -60；实得 {}）",
        view.read_with(&visual, |v, _| v.pan_x_for_test())
    );
    mid_move(cx, cx0);
    visual.update(|window, app| window.draw(app).clear(app));
    assert!(
        (view.read_with(&visual, |v, _| v.pan_x_for_test()) - pan_before_drag).abs() < 1.,
        "左拖回 60 → pan 复位拖前值"
    );
    mid_up(cx, cx0);
    eprintln!("#421 ✓ 中键拖拽平移（右拖 -60 / 左拖复位 / 松开收尾）");

    // ⑦ 切回适宽（Ctrl+Shift+0 真键路径）：溢出消除 → pan 钳制复位
    // 0 + 页槽回基线（缩放档切换横向锚对称）。
    cx.update(|app| {
        view.update(app, |v, cx| v.dispatch_zoom_key("0", true, true, cx));
    });
    visual.update(|window, app| window.draw(app).clear(app));
    assert_eq!(
        view.read_with(&visual, |v, _| v.pan_x_for_test()),
        0.,
        "切回适宽 pan 复位（缩放档切换钳制）"
    );
    let restored = visual
        .debug_bounds("pdf-visible-page-0")
        .expect("复位后页 0 槽仍在观测面");
    assert!(
        (f32::from(restored.origin.x) - base_x).abs() < 2.,
        "页槽回基线（实得 x {} vs {}）",
        f32::from(restored.origin.x),
        base_x
    );
    eprintln!("#421 ✓ 适宽复位：pan 0 / 槽回基线");

    let _ = std::fs::remove_dir_all(&dir);
}

/// #51 D4b 接线面：**当前页/缩放状态一致性**两面全链——
///
/// 1. **页码读数活更新**：页视图 emit（TopPageChanged）在页变化
///    时真实发出（Shell 订阅 → 状态栏「第 N / M 页」跟随滚动）；
/// 2. **标签切换状态保持**：ActualSize 档 + 页 2 → 切走（异路径
///    重建）→ 切回 → 缩放档与顶部页**原样回放**（快照机制）；
///    关标签重开同文件亦回放（关标签路径快照）。
#[gpui::test]
fn pdf_view_state_snapshot_and_page_label(cx: &mut gpui::TestAppContext) {
    let corpus = |name: &str| {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/display_corpus")
                .join(name),
        )
        .unwrap_or_else(|e| panic!("语料不可读（{name}）：{e}"))
    };
    let dir = std::env::temp_dir().join(format!("mdc-d4b-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pdf_a = dir.join("a.pdf");
    let pdf_b = dir.join("b.pdf");
    let _ = std::fs::write(&pdf_a, corpus("mixed_features.pdf"));
    let _ = std::fs::write(&pdf_b, corpus("landscape_mixed.pdf"));

    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let mut slot: Option<Entity<ShellApp>> = None;
    let window = cx.add_window(|window, cx| {
        let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
        slot = Some(shell.clone());
        WiringHost(shell)
    });
    let shell = slot.expect("宿主构造即置");
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_document(pdf_a.clone(), window, cx);
        });
    });
    let view_a = cx.read(|app| shell.read(app).pdf_pages.as_ref().map(|(_, e)| e.clone()));
    let view_a = match view_a {
        Some(v) => v,
        None => {
            eprintln!(
                "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                     本机无 libpdfium——#51 D4b 状态一致性 e2e 未真跑。\n  \
                     有 libpdfium 的环境（MDC_PDFIUM_LIB=<库路径> \
                     cargo test）全量执行。\n═══════════════════════════\n"
            );
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
    };
    let any: gpui::AnyWindowHandle = window.clone().into();
    let mut visual = gpui::VisualTestContext::from_window(any.clone(), cx);
    // 泵（实体可换——A/B/重建 A/重开 A 各阶段传各实体）。
    let pump = |cx: &mut gpui::TestAppContext,
                visual: &mut gpui::VisualTestContext,
                view: &Entity<PdfPagesView>,
                what: &str,
                ready: &dyn Fn(&PdfPagesView) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            cx.background_executor
                .advance_clock(std::time::Duration::from_millis(20));
            cx.background_executor.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(2));
            visual.update(|window, app| window.draw(app).clear(app));
            if view.read_with(visual, |v, _| ready(v)) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{what} 泵超时（真缺陷不放过）"
            );
        }
    };
    pump(cx, &mut visual, &view_a, "A 首屏", &|v| {
        v.page_ready_for_test(0)
    });

    // ① 页变化 emit（Shell 订阅 → 状态栏活更新的信号源）。
    let emit_count = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let counter = emit_count.clone();
    {
        let counter = counter.clone();
        cx.update(|app| {
            app.subscribe(&view_a, move |_, _: &crate::pdf_pages::PdfPagesEvent, _| {
                counter.set(counter.get() + 1);
            })
            .detach();
        });
    }
    cx.update(|app| view_a.update(app, |v, _| v.scroll_to_page(2)));
    pump(cx, &mut visual, &view_a, "A 跳页 2", &|v| {
        v.top_visible_page() == 2 && v.page_ready_for_test(2)
    });
    assert!(
        emit_count.get() >= 1,
        "页变化必须 emit（状态栏信号源），实得 {}",
        emit_count.get()
    );
    eprintln!(
        "#51 D4b ✓ 页变化 emit ×{}（状态栏信号源在位）",
        emit_count.get()
    );

    // ② ActualSize 档（Ctrl+1 真路径——快照素材）。
    cx.update(|app| view_a.update(app, |v, cx| v.dispatch_zoom_key("1", true, false, cx)));
    pump(cx, &mut visual, &view_a, "A 实际大小档", &|v| {
        v.page_ready_for_test(2) && v.raster_width_for_test() == 640
    });
    eprintln!("#51 D4b ✓ ActualSize 档就位（光栅宽 640）");

    // ③ 切走：开 B（异路径重建——A 快照落账）。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_document(pdf_b.clone(), window, cx);
        });
    });
    let view_b = cx.read(|app| shell.read(app).pdf_pages.as_ref().map(|(_, e)| e.clone()));
    let view_b = view_b.expect("B 实体在位（打开链成功）");
    pump(cx, &mut visual, &view_b, "B 首屏", &|v| {
        v.page_ready_for_test(0)
    });

    // ④ 切回 A：快照回放——顶部页 2 + ActualSize 档原样。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| shell.activate_tab(0, window, cx));
    });
    let view_a2 = cx.read(|app| shell.read(app).pdf_pages.as_ref().map(|(_, e)| e.clone()));
    let view_a2 = view_a2.expect("A 实体重建（快照回放路径）");
    pump(cx, &mut visual, &view_a2, "A 回放", &|v| {
        v.top_visible_page() == 2 && v.page_ready_for_test(2)
    });
    assert_eq!(
        view_a2.read_with(&visual, |v, _| v.raster_width_for_test()),
        640,
        "切回后缩放档回放（ActualSize 640——不从 FitWidth 576 重来）"
    );
    eprintln!("#51 D4b ✓ 切标签回放：页 2 + ActualSize 档原样");

    // ⑤ 关标签路径快照：关 A → 重开同文件 → 原页原档。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.close_tab_no_confirm(0, window, cx);
        });
    });
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_document(pdf_a.clone(), window, cx);
        });
    });
    let view_a3 = cx.read(|app| shell.read(app).pdf_pages.as_ref().map(|(_, e)| e.clone()));
    let view_a3 = view_a3.expect("A 重开实体");
    pump(cx, &mut visual, &view_a3, "A 重开回放", &|v| {
        v.top_visible_page() == 2 && v.page_ready_for_test(2)
    });
    assert_eq!(
        view_a3.read_with(&visual, |v, _| v.raster_width_for_test()),
        640,
        "关后重开缩放档回放"
    );
    eprintln!("#51 D4b ✓ 关标签重开回放：页 2 + ActualSize 档原样");

    let _ = std::fs::remove_dir_all(&dir);
}

/// #51 D4c 接线面：**旋转/CropBox 档批注命中与保存重开**（视图级
/// 全链）——真鼠标管线（右键拖框）→ 控制器写链（旋转/CropBox 变
/// 换单元面已在 pdf_annotate_e2e 锚定；本测补**视图级命中链**：
/// 探针命中域 → finish_drag → commit → flush → rev 换代重渲）→
/// save_annotations 落盘（磁盘 /Highlight 面在位）→ 重开渲染成功。
/// 夹具双档：rotated_90.pdf（旋转页）+ cropbox_offset.pdf（偏移
/// CropBox letterbox 档）。
#[gpui::test]
fn pdf_view_right_drag_annotate_rotated_and_cropbox(cx: &mut gpui::TestAppContext) {
    for name in ["rotated_90.pdf", "cropbox_offset.pdf"] {
        let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/display_corpus")
            .join(name);
        let bytes = std::fs::read(&corpus).unwrap_or_else(|e| panic!("语料不可读（{name}）：{e}"));
        let dir = std::env::temp_dir().join(format!("mdc-d4c-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pdf = dir.join("t.pdf");
        let _ = std::fs::write(&pdf, bytes.clone());

        cx.update(|app| {
            theme::init(theme::LoadThemes::JustBase, app);
            theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
        });
        let mut slot: Option<Entity<ShellApp>> = None;
        let window = cx.add_window(|window, cx| {
            let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
            slot = Some(shell.clone());
            WiringHost(shell)
        });
        let shell = slot.expect("宿主构造即置");
        let _ = window.update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_document(pdf.clone(), window, cx);
            });
        });
        let view = cx.read(|app| shell.read(app).pdf_pages.as_ref().map(|(_, e)| e.clone()));
        let Some(view) = view else {
            eprintln!(
                "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                     本机无 libpdfium——#51 D4c {name} 批注链 e2e 未真跑。\n  \
                     有 libpdfium 的环境（MDC_PDFIUM_LIB=<库路径> \
                     cargo test）全量执行。\n═══════════════════════════\n"
            );
            let _ = std::fs::remove_dir_all(&dir);
            return;
        };
        let any: gpui::AnyWindowHandle = window.clone().into();
        let mut visual = gpui::VisualTestContext::from_window(any.clone(), cx);
        let pump = |cx: &mut gpui::TestAppContext,
                    visual: &mut gpui::VisualTestContext,
                    view: &Entity<PdfPagesView>,
                    what: &str,
                    ready: &dyn Fn(&PdfPagesView) -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                cx.background_executor
                    .advance_clock(std::time::Duration::from_millis(20));
                cx.background_executor.run_until_parked();
                std::thread::sleep(std::time::Duration::from_millis(2));
                visual.update(|window, app| window.draw(app).clear(app));
                if view.read_with(visual, |v, _| ready(v)) {
                    return;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "{name}：{what} 泵超时（真缺陷不放过）"
                );
            }
        };
        pump(cx, &mut visual, &view, "首屏", &|v| {
            v.page_ready_for_test(0)
        });
        let bytes_before = view.read_with(&visual, |v, _| v.bytes());

        // ① 真右键拖框（探针命中域 → drag_start → finish → 三手势
        //    默认 Highlight → commit 真写链）。
        let b = visual
            .debug_bounds("pdf-visible-page-0")
            .expect("页 0 槽真实渲染");
        let (bx, by, bw, bh) = (
            f32::from(b.origin.x),
            f32::from(b.origin.y),
            f32::from(b.size.width),
            f32::from(b.size.height),
        );
        let pt = |x: f32, y: f32| gpui::point(gpui::px(x), gpui::px(y));
        cx.update_window(any.clone(), |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                    button: gpui::MouseButton::Right,
                    position: pt(bx + bw * 0.30, by + bh * 0.30),
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                cx,
            );
        })
        .expect("右键按下");
        cx.update_window(any.clone(), |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                    button: gpui::MouseButton::Right,
                    position: pt(bx + bw * 0.60, by + bh * 0.50),
                    modifiers: Default::default(),
                    click_count: 1,
                }),
                cx,
            );
        })
        .expect("右键松开");
        // ② flush 换代重渲（rev 推进 + 新 rev 位图就绪 + 脏标）。
        pump(cx, &mut visual, &view, "换代重渲", &|v| {
            v.annotate_rev_for_test() == 1 && v.page_ready_for_test(0)
        });
        assert!(
            view.read_with(&visual, |v, _| v.is_dirty()),
            "批注后脏标在位"
        );
        let bytes_after = view.read_with(&visual, |v, _| v.bytes());
        assert!(
            bytes_after.len() > bytes_before.len(),
            "{name}：flush 后字节追加（增量写链真实发生）"
        );
        assert_eq!(
            view.read_with(&visual, |v, _| v.cache_rev_families_for_test()),
            vec![1u64],
            "{name}：换代后缓存 rev 族只剩新 rev（D3b 语义在此复验）"
        );
        eprintln!("#51 D4c ✓ {name}：右键拖框 → flush 换代 → 新 rev 重渲");

        // ③ 落盘（save_annotations 增量写——前缀保持）+ 磁盘面在位。
        let saved = window
            .update(cx, |_host, _window, cx| {
                view.update(cx, |v, _| v.save_annotations(&pdf))
            })
            .expect("保存调用");
        let stats = saved
            .expect("保存成功（真 pdfium 增量写链）")
            .expect("有未落盘批注（脏标在位）");
        // 既有语义：commit 内 flush 已追加（内存产物）→ save = 零脏
        // 幂等写盘（零追加——不产生空修订；e2e 语义锚定沿用）。
        assert_eq!(
            stats.bytes_appended, 0,
            "{name}：已 flush 后 save = 零脏幂等（追加发生在 commit 的 flush）"
        );
        let disk = std::fs::read(&pdf).expect("磁盘产物可读");
        let marker: &[u8] = b"/Highlight";
        assert!(
            disk.windows(marker.len()).any(|w| w == marker),
            "{name}：磁盘 /Highlight 面在位（保存重开的物证）"
        );
        assert!(
            disk.starts_with(bytes.as_slice()),
            "{name}：产物 = 原字节逐位前缀（增量语义）"
        );
        eprintln!(
            "#51 D4c ✓ {name}：落盘 {}B 追加（前缀保持 + /Highlight 在位）",
            stats.bytes_appended
        );

        // ④ 关标签重开（保存重开）：磁盘字节新会话渲染成功。
        let _ = window.update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.close_tab_no_confirm(0, window, cx);
            });
        });
        let _ = window.update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_document(pdf.clone(), window, cx);
            });
        });
        let view2 = cx
            .read(|app| shell.read(app).pdf_pages.as_ref().map(|(_, e)| e.clone()))
            .expect("重开实体");
        pump(cx, &mut visual, &view2, "重开首屏", &|v| {
            v.page_ready_for_test(0)
        });
        eprintln!("#51 D4c ✓ {name}：保存重开渲染成功（批注字节新会话）");

        // 清场：本档窗口/实体随循环丢弃（下一档新窗口）。
        drop(visual);
        let _ = window.update(cx, |_, window, _| window.remove_window());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// WiringHost 整树 + md 标签窗口（#47 A4 查找框焦点链——md 面）。
fn md_wiring_window(
    cx: &mut gpui::TestAppContext,
) -> (Entity<ShellApp>, gpui::WindowHandle<WiringHost>, PathBuf) {
    let dir = std::env::temp_dir().join(format!("mdc-a4-find-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let md = dir.join("a.md");
    std::fs::write(&md, "L0\n").unwrap();
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let mut slot: Option<Entity<ShellApp>> = None;
    let window = cx.add_window(|window, cx| {
        let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
        slot = Some(shell.clone());
        WiringHost(shell)
    });
    let shell = slot.expect("宿主构造即置");
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_document(md.clone(), window, cx);
        });
    });
    (shell, window, dir)
}

/// 真实键击派发（update_window + dispatch_event——main.rs 烟雾同构；
/// 预绘一帧保证派发树在位）。
fn dispatch_key_real(
    window: &gpui::WindowHandle<WiringHost>,
    cx: &mut gpui::TestAppContext,
    key: &str,
    key_char: Option<&str>,
    ctrl: bool,
) {
    let any: gpui::AnyWindowHandle = window.clone().into();
    cx.update_window(any, |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .expect("预绘一帧");
    cx.update_window(any, |_, window, cx| {
        window.dispatch_event(
            gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                keystroke: gpui::Keystroke {
                    modifiers: gpui::Modifiers {
                        control: ctrl,
                        ..Default::default()
                    },
                    key: key.into(),
                    key_char: key_char.map(str::to_string),
                },
                is_held: false,
                prefer_character_input: false,
            }),
            cx,
        );
    })
    .expect("键击派发");
}

/// #47 A4：md 查找框真实键链与焦点恢复——Ctrl+F 开（焦点落查找条，
/// 键入进查询框不进正文）→ Esc 关（焦点回编辑区）→ 键入恢复进正文
/// （焦点恢复的**行为**证据，非仅句柄断言）。
#[gpui::test]
fn md_ctrl_f_find_bar_esc_focus_chain(cx: &mut gpui::TestAppContext) {
    let (shell, window, dir) = md_wiring_window(cx);

    // 前置：焦点在编辑器（open_document 落焦链）。
    let ed_focus = window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                shell.tab_editor(0).unwrap().read(cx).focus_handle().clone()
            })
        })
        .expect("取编辑器焦点句柄");
    window
        .update(cx, |_host, window, cx| {
            assert!(ed_focus.contains_focused(window, cx), "前置：焦点在编辑器");
        })
        .expect("前置断言");

    // ① Ctrl+F（真实派发）→ 查找条开：焦点离开编辑器（落查找条输入框）。
    dispatch_key_real(&window, cx, "f", Some("f"), true);
    window
        .update(cx, |_host, window, cx| {
            assert!(
                !ed_focus.contains_focused(window, cx),
                "Ctrl+F 后焦点落查找条输入框（编辑器让焦）"
            );
        })
        .expect("① 断言");

    // ② 查找条聚焦期键入：进查询框，不进正文（焦点去向的行为证据）。
    dispatch_key_real(&window, cx, "z", Some("z"), false);
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                let ed = shell.tab_editor(0).unwrap().clone();
                assert_eq!(
                    ed.read(cx).buffer_line(0),
                    "L0",
                    "查找条聚焦期键入不进正文（进查询框）"
                );
            });
        })
        .expect("② 断言");

    // ③ Esc（真实派发）→ 查找条关：焦点回编辑区。
    dispatch_key_real(&window, cx, "escape", None, false);
    window
        .update(cx, |_host, window, cx| {
            assert!(
                ed_focus.contains_focused(window, cx),
                "Esc 后焦点回编辑区（close_find 归焦）"
            );
        })
        .expect("③ 断言");

    // ④ 键入恢复进正文（焦点恢复的行为证据——键事件真到达编辑器）。
    // 光标在 (0,0)（无选区开查找条 + 零命中查询不动光标）——插入在
    // 行首（"xL0"）；键入真到达编辑器即焦点恢复的行为面。
    // Include platform text delivery after keydown; dispatch_event alone
    // does not model WM_CHAR / native text input.
    cx.dispatch_keystroke(window.into(), gpui::Keystroke::parse("x").unwrap());
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                let ed = shell.tab_editor(0).unwrap().clone();
                assert_eq!(
                    ed.read(cx).buffer_line(0),
                    "xL0",
                    "Esc 后键入恢复进正文（光标处插入——焦点恢复的行为面）"
                );
            });
        })
        .expect("④ 断言");

    let _ = std::fs::remove_dir_all(&dir);
}

/// WiringHost 整树 + **空态** shell 窗口（无文档——工作区测试用；
/// md_wiring_window 的无打开文档变体）。
fn md_wiring_window_root(
    cx: &mut gpui::TestAppContext,
) -> (Entity<ShellApp>, gpui::WindowHandle<WiringHost>) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let mut slot: Option<Entity<ShellApp>> = None;
    let window = cx.add_window(|window, cx| {
        let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
        slot = Some(shell.clone());
        WiringHost(shell)
    });
    (slot.expect("宿主构造即置"), window)
}

/// 单击派发（MouseDown + MouseUp 同点——statusbar click_at 同构；
/// WiringHost 版）。
fn click_wiring_at(
    window: &gpui::WindowHandle<WiringHost>,
    cx: &mut gpui::TestAppContext,
    x: f32,
    y: f32,
) {
    let any: gpui::AnyWindowHandle = window.clone().into();
    cx.update_window(any, |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .expect("预绘一帧");
    let down = gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
        button: gpui::MouseButton::Left,
        position: gpui::point(gpui::px(x), gpui::px(y)),
        modifiers: Default::default(),
        click_count: 1,
        first_mouse: false,
    });
    let up = gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
        button: gpui::MouseButton::Left,
        position: gpui::point(gpui::px(x), gpui::px(y)),
        modifiers: Default::default(),
        click_count: 1,
    });
    cx.update_window(any, |_, window, cx| {
        window.dispatch_event(down, cx);
        window.dispatch_event(up, cx);
    })
    .expect("单击派发");
}

/// #47 A3：打开文件夹全链——open_workspace（真实临时目录）→ 文件
/// 树 + git 面板就位 → **真实点击**文件树文件行 → open_file 钩子 →
/// 标签打开（路径 = 工作区文件）。扫描从下往上（先中 a.md 文件行
/// ——唯一文件行；root 折叠行在最上，自下而上永不先触）。
#[gpui::test]
fn open_workspace_click_tree_opens_tab(cx: &mut gpui::TestAppContext) {
    let dir = std::env::temp_dir().join(format!("mdc-a3-ws-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let md = dir.join("a.md");
    std::fs::write(&md, "W0\n").unwrap();

    let (shell, window) = md_wiring_window_root(cx);
    let _ = window.update(cx, |_host, _window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_workspace(dir.clone(), cx);
        });
    });
    cx.run_until_parked();

    // ① 工作区就位：文件树（根 = 工作区目录）+ git 面板（非 git
    //    目录同样就位——#183 提示态不崩溃不拦截）。
    let _ = window.update(cx, |_host, _window, cx| {
        shell.update(cx, |shell, cx| {
            let tree = shell.workspace.as_ref().expect("文件树实体在场");
            assert_eq!(tree.read(cx).root(), dir.as_path(), "树根 = 工作区目录");
            assert!(
                shell.git_panel.is_some(),
                "git 面板就位（非 git 目录提示态）"
            );
        });
    });

    // ② 自下而上扫描左栏文件树（x = 活动条 52 + 左栏中部）——空树区
    //    点击零副作用；首个交互行 = a.md 文件行（唯一文件，root 折叠
    //    行在其上方——扫描方向保证不先触）→ open_file 钩子 → 标签开。
    let mut opened: Option<f32> = None;
    let mut y = 170.0f32;
    while y > 50.0 {
        click_wiring_at(&window, cx, 150.0, y);
        let tabs = window
            .update(cx, |_host, _window, cx| {
                shell.update(cx, |shell, cx| shell.tab_count())
            })
            .expect("读标签数");
        if tabs > 0 {
            opened = Some(y);
            break;
        }
        y -= 4.0;
    }
    let hit_y = opened.unwrap_or_else(|| panic!("扫描未命中文件行（y 170→50 未开标签）"));

    // ③ 标签 = 工作区文件（路径 + 内容），激活即聚焦。
    let _ = window.update(cx, |_host, _window, cx| {
        shell.update(cx, |shell, cx| {
            assert_eq!(shell.tab_count(), 1, "点击文件行开一标签");
            let path = shell.tab_path(0).unwrap();
            assert_eq!(
                path,
                std::path::absolute(&md).unwrap_or(md.clone()),
                "标签路径 = 工作区文件（open_file 钩子链）"
            );
            let ed = shell.tab_editor(0).unwrap().clone();
            assert_eq!(ed.read(cx).buffer_line(0), "W0", "标签内容 = 文件内容");
            eprintln!("ws-click: 命中 y={hit_y}（自下而上扫描首个交互行）");
        });
    });

    let _ = std::fs::remove_dir_all(&dir);
}
