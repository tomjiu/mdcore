//! #47 巨石拆解批③：原 shell.rs 内嵌 `page_view_action_honesty_tests` 纯迁出（零行为变化）。

use super::*;

/// 测试宿主（ShellApp 必须进渲染树——键派发走真实管线，pdf 同款）。
struct WiringHost(Entity<ShellApp>);

impl gpui::Render for WiringHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div().size_full().child(self.0.clone())
    }
}

fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mdc-p176-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 仓内 docx 夹具拷入临时目录（tab_workflow_tests 同款夹具源）。
fn write_docx_fixture(dir: &Path, name: &str) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.docx");
    let path = dir.join(name);
    std::fs::copy(&src, &path).expect("夹具在场（cargo test gen_smoke_docx -- --ignored 生成）");
    path
}

/// 真实键击（预绘一帧 + dispatch_event——pdf_search_wiring_tests 同构）。
fn key_down(
    window: &gpui::WindowHandle<WiringHost>,
    cx: &mut gpui::TestAppContext,
    key: &str,
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
                    key_char: None,
                },
                is_held: false,
                prefer_character_input: false,
            }),
            cx,
        );
    })
    .expect("键击派发");
}

/// #176 验收②：页面态 Ctrl+S 有明确反馈——不再静默返回「无修改」。
/// 真实键管线（ShellAction::Save → save_tab → 页面态卫 → 提示横幅）；
/// 三态语义零变化（仍 Unmodified——关闭/退出流程消费面不破）+ 零写盘。
#[gpui::test]
fn docx_page_view_ctrl_s_gives_notice_not_silence(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("save");
    let docx = write_docx_fixture(&dir, "save.docx");
    let before = std::fs::read(&docx).expect("夹具读入");

    // 渲染面全局件（pdf_search_wiring_tests 同款——整树键派发需要
    // theme provider 在位）。
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
            shell.open_document(docx.clone(), window, cx);
        });
    });

    // 前置：docx 缺省页面视图在位（守卫三条件齐）。
    let _ = window.update(cx, |_host, _window, cx| {
        shell.update(cx, |shell, cx| {
            assert!(shell.docx_page_view_active(cx), "前置：docx 缺省页面视图");
        });
    });

    // ① 状态面：save_tab 三态零变化（Unmodified）+ 零写盘 +
    //    明确提示（旧实现：静默返回，无任何用户可见反馈）。
    let _ = window.update(cx, |_host, _window, cx| {
        shell.update(cx, |shell, cx| {
            assert_eq!(
                shell.save_tab(0, cx),
                SaveOutcome::Unmodified,
                "三态语义零变化（关闭/退出流程消费面不破）"
            );
            assert_eq!(std::fs::read(&docx).unwrap(), before, "零写盘保持");
            let msg = shell.notice_board.message().map(str::to_owned);
            assert!(
                msg.as_deref().is_some_and(|m| m.contains("页面视图")),
                "save_tab 页面态须给提示（不静默）：{msg:?}"
            );
        });
    });

    // ② 键面：真实 Ctrl+S 管线 → 新提示（Lead 复验收口要求：先清
    //    旧提示再派键——旧测试 save_tab 先生成提示后查「提示仍在」，
    //    不能证明快捷键真正触发了路径。此处清空后按键，断言**新增**
    //    通知（ShellAction::Save 同链派发——菜单/键位/命令面板三
    //    入口同一单点））。
    let _ = window.update(cx, |_host, _window, cx| {
        shell.update(cx, |shell, _| {
            shell.notice_board.clear();
            assert!(
                shell.notice_board.message().is_none(),
                "派键前清空提示（前置——排除 save_tab 旧通知干扰）"
            );
        });
    });
    key_down(&window, cx, "s", true);
    let _ = window.update(cx, |_host, _window, cx| {
        shell.update(cx, |shell, _| {
            let msg = shell.notice_board.message().map(str::to_owned);
            assert!(
                msg.as_deref().is_some_and(|m| m.contains("只读渲染")),
                "Ctrl+S 页面态**新增**提示（清空后真实键管线触发）：{msg:?}"
            );
        });
    });

    let _ = std::fs::remove_dir_all(&dir);
}

/// #176 验收①（键面）：页面态 Ctrl+F 不再静默无反应——给明确提示，
/// 且**不**派发到不可见编辑器（find 状态不被隐式置开 = 旧「假按钮」
/// 行为的证据性反断言）。切文本视图后 Ctrl+F 恢复真实开条（验收③
/// 零回归）。
#[gpui::test]
fn docx_page_view_ctrl_f_notice_then_text_view_find_works(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("find");
    let docx = write_docx_fixture(&dir, "find.docx");

    // 渲染面全局件（pdf_search_wiring_tests 同款——整树键派发需要
    // theme provider 在位）。
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
            shell.open_document(docx.clone(), window, cx);
        });
    });

    // ① 页面态 Ctrl+F：提示（非静默）+ 编辑器 find 不被隐式置开
    //    （旧死路把查找条开在不可见编辑器上——状态脏但用户看不见）。
    key_down(&window, cx, "f", true);
    let _ = window.update(cx, |_host, _window, cx| {
        shell.update(cx, |shell, cx| {
            let msg = shell.notice_board.message().map(str::to_owned);
            assert!(
                msg.as_deref().is_some_and(|m| m.contains("查找")),
                "页面态 Ctrl+F 须给提示（不静默无反应）：{msg:?}"
            );
            let ed = shell.tab_editor(0).expect("docx 标签在场").clone();
            assert!(
                !ed.read(cx).find_open(),
                "查找条不被隐式开在不可见编辑器上（不脏状态）"
            );
        });
    });

    // ② 切文本视图：守卫解除（页视图实体拆除——谓词与渲染分流
    //    同源）。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.toggle_docx_view_mode(window, cx);
            assert!(!shell.docx_view_mode, "切文本态成功");
            assert!(
                !shell.docx_page_view_active(cx),
                "文本视图守卫解除（页视图实体已拆）"
            );
        });
    });

    // ③ 文本态 Ctrl+F：编辑器在场且聚焦——真实开条（零回归）。
    key_down(&window, cx, "f", true);
    let _ = window.update(cx, |_host, _window, cx| {
        shell.update(cx, |shell, cx| {
            let ed = shell.tab_editor(0).expect("docx 标签在场").clone();
            assert!(
                ed.read(cx).find_open(),
                "文本视图 Ctrl+F 照常开查找条（零回归）"
            );
        });
    });

    let _ = std::fs::remove_dir_all(&dir);
}
