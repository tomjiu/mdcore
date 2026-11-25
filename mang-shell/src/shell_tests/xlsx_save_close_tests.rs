//! #47 巨石拆解批③：原 shell.rs 内嵌 `xlsx_save_close_tests` 纯迁出（零行为变化）。

use super::*;
use crate::editor::commands::Cmd;
use gpui::{
    AnyWindowHandle, KeyDownEvent, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent,
    PlatformInput, point, px,
};

/// 宿主：shell 进整渲染树（网格体 canvas 命中/焦点注册需要——
/// confirm_lifecycle_tests Host 同款；空 div 宿主把网格挡在派发
/// 树外，真实点击无法路由）。
struct Host(Entity<ShellApp>);

impl gpui::Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div().size_full().child(self.0.clone())
    }
}

/// 整树宿主 + 窗口（theme provider 双注册——#422 同款）。
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

/// 重绘一帧（命中盒/焦点节点随最新状态重注册）。
fn redraw(window: &gpui::WindowHandle<Host>, cx: &mut gpui::TestAppContext) {
    let any: AnyWindowHandle = (*window).into();
    cx.update_window(any, |_, window, cx| window.draw(cx).clear(cx))
        .expect("重绘一帧");
}

/// 真实鼠标按下+抬起（click_count 参数化——双击 = 2）。
fn click_at(
    window: &gpui::WindowHandle<Host>,
    cx: &mut gpui::TestAppContext,
    x: f32,
    y: f32,
    click_count: usize,
) {
    let any: AnyWindowHandle = (*window).into();
    let down = PlatformInput::MouseDown(MouseDownEvent {
        button: MouseButton::Left,
        position: point(px(x), px(y)),
        modifiers: Modifiers::default(),
        click_count,
        first_mouse: false,
    });
    let up = PlatformInput::MouseUp(MouseUpEvent {
        button: MouseButton::Left,
        position: point(px(x), px(y)),
        modifiers: Modifiers::default(),
        click_count,
    });
    cx.update_window(any, |_, window, cx| {
        window.dispatch_event(down, cx);
        window.dispatch_event(up, cx);
    })
    .expect("鼠标派发");
}

/// 真实键派发（预绘一帧——派发树在位；confirm_lifecycle_tests
/// press_key_with_modifiers 同构）。
fn press_key(
    cx: &mut gpui::TestAppContext,
    window: &gpui::WindowHandle<Host>,
    key: &str,
    key_char: Option<&str>,
    ctrl: bool,
) {
    let any: AnyWindowHandle = (*window).into();
    cx.update_window(any, |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .expect("预绘一帧");
    cx.update_window(any, |_, window, cx| {
        window.dispatch_event(
            PlatformInput::KeyDown(KeyDownEvent {
                keystroke: gpui::Keystroke {
                    modifiers: Modifiers {
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
    .expect("键派发");
}

/// 临时夹具目录 + smoke.xlsx 副本（黄金夹具只读——保存写回源
/// 路径，必须复制到临时区；#584 merge 测试同纪律）。
fn xlsx_fixture(tag: &str) -> PathBuf {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.xlsx");
    let dir = std::env::temp_dir().join(format!("mdc-604-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let dst = dir.join("book.xlsx");
    std::fs::copy(&src, &dst).expect("复制黄金夹具");
    dst
}

/// 读网格槽实体（同文件测试面直读私有槽——tab_workflow_tests
/// 读 confirm/active 同口径）。
fn grid_entity(
    shell: &Entity<ShellApp>,
    cx: &App,
) -> Option<Entity<crate::xlsx_grid_view::XlsxGridView>> {
    shell.read(cx).xlsx_grid.as_ref().map(|(_, e)| e.clone())
}

/// 真实编辑置脏：候选点扫描**双击**（网格 canvas 的窗口级观察器
/// → `begin_edit_at`），以**双击后进入编辑态**为命中判据（编辑态
/// 只能由本次双击产生——不受陈旧选中残留干扰；smoke 夹具仅 5 行
/// ×3 列，可命中面 = 网格体 canvas 顶部的行区带，扫描域覆盖整个
/// 编辑区即可，坐标不硬编码）→ 命中即真实键入 `suffix` + Enter
/// 提交 → `is_modified` 为成功判据（公式格拒改/缓冲未变均继续
/// 扫描）。返回被编辑格的 A1 引用（None = 全域未进编辑态——
/// 测试 panic 交给调用方，报错带扫描面信息）。
fn real_grid_edit(
    shell: &Entity<ShellApp>,
    window: &gpui::WindowHandle<Host>,
    cx: &mut gpui::TestAppContext,
    suffix: &str,
) -> Option<String> {
    // 扫描域：编辑区 x ∈ [340, 1500]（含行号槽 48px + 数据列）、
    // y ∈ [110, 420]（顶部标题+标签 ≤110，行区带在其下方——
    // 超出行区的点击 hit_test 过滤 `r < sheet_rows` 自然不进编辑）。
    // 步长取质数间隔避免与行列周期对齐漏扫。
    for y in (110..=420).step_by(13) {
        for x in (340..=1500).step_by(61) {
            redraw(window, cx);
            click_at(window, cx, x as f32, y as f32, 2);
            // 命中判据：双击后进入编辑态（begin_edit_at 的唯一
            // 产物——面板遮挡/行号槽/超行区/间隙均不会置位）。
            let editing = window
                .update(cx, |_host, _window, cx| {
                    grid_entity(shell, cx)
                        .map(|view| view.read(cx).editing_active())
                        .unwrap_or(false)
                })
                .expect("编辑态检查");
            if !editing {
                continue;
            }
            for ch in suffix.chars() {
                press_key(cx, window, &ch.to_string(), Some(&ch.to_string()), false);
            }
            press_key(cx, window, "enter", None, false);
            let (dirty, a1) = window
                .update(cx, |_host, _window, cx| {
                    grid_entity(shell, cx)
                        .map(|view| {
                            let v = view.read(cx);
                            (v.is_modified(), v.selected_a1())
                        })
                        .unwrap_or((false, None))
                })
                .expect("脏态检查");
            if dirty {
                return a1;
            }
            // 进编辑但未置脏（公式格拒改/缓冲与原值同）——继续扫描。
        }
    }
    None
}

/// 独立读回：盘上文件的指定格文本（XlsxEditor 第二意见——不经
/// 网格模型缓存）。
fn cell_text_on_disk(path: &Path, sheet: usize, a1: &str) -> Option<String> {
    let bytes = std::fs::read(path).expect("盘上文件可读");
    let mut reopened = mdc_office_io::xlsx_edit::XlsxEditor::open(&bytes).expect("重开会话");
    reopened
        .cell_info(sheet, a1)
        .expect("格可查")
        .map(|info| info.text)
}

/// #604 验收 1（菜单/命令面板面）：真实编辑 → dispatch_editor_cmd
/// （菜单「文件 → 保存」同一调用面，menu_bar.rs EditorCmd 链）→
/// 文件实际读回新值 + 未动格原值保持 + 脏清除 + 标签在场。
#[gpui::test]
fn xlsx_menu_cmd_save_persists_and_keeps_contract(cx: &mut gpui::TestAppContext) {
    let path = xlsx_fixture("cmd");
    let (shell, window) = shell_window(cx);
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_document_tab(path.clone(), cx);
            });
        })
        .expect("开 xlsx 标签");
    cx.run_until_parked();
    let a1 = real_grid_edit(&shell, &window, cx, "7788")
        .expect("真实双击+键入+Enter 置脏（编辑区扫描全域未命中 = 布局回归）");

    // 前置：标签脏（会话脏计入——tab_is_modified 同源）；文本面
    // 自身干净（只读——脏来自网格会话，语义分层）。
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                assert!(shell.tab_is_modified(0, cx).unwrap(), "会话脏计入标签判定");
                assert!(
                    !shell.tab_editor(0).unwrap().read(cx).is_modified(),
                    "只读文本面无未存修改"
                );
            });
        })
        .expect("脏态断言");

    // 保存（菜单面——dispatch_editor_cmd 与菜单点击同一调用链）。
    window
        .update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.dispatch_editor_cmd(Cmd::Save, window, cx);
            });
        })
        .expect("菜单保存");

    // 脏清除 + 标签在场 + toast 反馈。
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                assert!(!shell.tab_is_modified(0, cx).unwrap(), "保存后脏清除");
                assert_eq!(shell.tab_count(), 1, "保存不动标签");
                let notice = shell.notice_board.message().map(str::to_owned);
                assert_eq!(notice.as_deref(), Some("已保存（xlsx 编辑会话）"));
            });
        })
        .expect("保存后断言");

    // 独立读回：被编辑格带新值；未动格原值保持（B2=100 数值、
    // A2=一月共享串——夹具已知面；编辑格与它们不同格时才断言
    // 「未动」，同格则只断言新值——扫描不预设命中哪格）。
    let edited = cell_text_on_disk(&path, 0, &a1).expect("编辑格非空");
    assert!(edited.contains("7788"), "{a1} 落盘带键入值（得 {edited}）");
    if a1 != "B2" {
        assert_eq!(cell_text_on_disk(&path, 0, "B2").as_deref(), Some("100"));
    }
    if a1 != "A2" {
        assert_eq!(cell_text_on_disk(&path, 0, "A2").as_deref(), Some("一月"));
    }

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// #604 验收 2（关闭确认三态）：真实编辑 → 真实 Ctrl+W → 确认框
/// 弹出 → Esc 取消（标签留 + 脏保持 + 盘上不动）→ 再 Ctrl+W →
/// Enter「保存」→ 标签关 + 文件落盘（修复前：假 Unmodified 直接
/// 关标签，编辑静默丢）。
#[gpui::test]
fn xlsx_close_confirm_save_persists_cancel_keeps_dirty(cx: &mut gpui::TestAppContext) {
    let path = xlsx_fixture("close");
    let (shell, window) = shell_window(cx);
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_document_tab(path.clone(), cx);
            });
        })
        .expect("开 xlsx 标签");
    cx.run_until_parked();
    let a1 = real_grid_edit(&shell, &window, cx, "369").expect("真实编辑置脏");
    let original = std::fs::read(&path).expect("原文件");

    // 真实 Ctrl+W（网格持焦——ctrl+w 不被网格消费，冒泡 shell 根
    // → ShellAction::CloseTab → close_active_tab）。
    press_key(cx, &window, "w", None, true);
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, _cx| {
                let confirm = shell.confirm.as_ref().expect("会话脏关闭必弹确认");
                assert!(
                    matches!(confirm.action, ConfirmAction::CloseTab { idx: 0, .. }),
                    "关闭确认（得 {:?}）",
                    confirm.action
                );
                assert_eq!(confirm.buttons, CONFIRM_BUTTONS, "保存/不保存/取消三选");
            });
        })
        .expect("确认框断言");

    // Esc 取消：标签留、脏保持、盘上不动。
    press_key(cx, &window, "escape", None, false);
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                assert!(shell.confirm.is_none(), "Esc 关确认");
                assert_eq!(shell.tab_count(), 1, "取消 = 标签留");
                assert!(shell.tab_is_modified(0, cx).unwrap(), "取消 = 脏保持");
            });
        })
        .expect("取消断言");
    assert_eq!(std::fs::read(&path).unwrap(), original, "取消不落盘");

    // 再 Ctrl+W → Enter（selected=0「保存」）→ 标签关 + 落盘。
    press_key(cx, &window, "w", None, true);
    press_key(cx, &window, "enter", None, false);
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, _cx| {
                assert_eq!(shell.tab_count(), 0, "保存后关标签");
                assert!(shell.confirm.is_none());
                assert!(shell.xlsx_grid.is_none(), "标签关 → 网格槽清空");
            });
        })
        .expect("关闭断言");
    let edited = cell_text_on_disk(&path, 0, &a1).expect("编辑格非空");
    assert!(edited.contains("369"), "{a1} 落盘（得 {edited}）");

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// #604 验收 2 补（不保存态）：真实编辑 → Ctrl+W → Tab+Enter
/// 「不保存」→ 标签关 + 盘上原样（放弃语义——与 md 确认契约
/// 同款三态）。
#[gpui::test]
fn xlsx_close_confirm_discard_keeps_file(cx: &mut gpui::TestAppContext) {
    let path = xlsx_fixture("discard");
    let (shell, window) = shell_window(cx);
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_document_tab(path.clone(), cx);
            });
        })
        .expect("开 xlsx 标签");
    cx.run_until_parked();
    let _ = real_grid_edit(&shell, &window, cx, "555").expect("真实编辑置脏");
    let original = std::fs::read(&path).expect("原文件");

    press_key(cx, &window, "w", None, true);
    press_key(cx, &window, "tab", None, false); // selected 0→1（不保存）
    press_key(cx, &window, "enter", None, false);
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, _cx| {
                assert_eq!(shell.tab_count(), 0, "不保存 = 关标签");
                assert!(shell.confirm.is_none());
            });
        })
        .expect("关闭断言");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        original,
        "不保存 = 盘上原样（放弃编辑）"
    );

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// #604 验收 3（退出面）：会话脏 → simulate_close（真实
/// should-close 回调链）→ 关闭被拦 + 退出确认弹出（修复前：脏
/// 收集漏 xlsx，直接放行 = 静默丢）→ Enter「保存」→ 全标签经
/// save_tab 落盘（测试平台 quit 无操作——不断言进程行为）。
#[gpui::test]
fn xlsx_dirty_quit_confirm_blocks_and_saves_all(cx: &mut gpui::TestAppContext) {
    let path = xlsx_fixture("quit");
    let (shell, window) = shell_window(cx);
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_document_tab(path.clone(), cx);
            });
        })
        .expect("开 xlsx 标签");
    cx.run_until_parked();
    let a1 = real_grid_edit(&shell, &window, cx, "246").expect("真实编辑置脏");

    // 真实关闭钮回调链：脏 → 拦截 + 退出确认。
    let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
    let closed = vcx.simulate_close();
    assert!(
        !closed,
        "#604：会话脏 → should-close 返回 false（关闭被拦）"
    );
    vcx.update(|_window, cx| {
        shell.update(cx, |shell, _cx| {
            let confirm = shell.confirm.as_ref().expect("退出前确认弹出");
            assert!(
                matches!(confirm.action, ConfirmAction::Quit),
                "确认动作 = Quit（得 {:?}）",
                confirm.action
            );
            assert!(
                confirm.message.contains('1'),
                "文案含未保存计数（得 {:?}）",
                confirm.message
            );
            assert_eq!(shell.tab_count(), 1, "被拦 = 标签还在");
        });
    });

    // Enter「保存」（selected=0）→ Quit 分支逐标签 save_tab 落盘
    //（测试平台 quit() 无操作——文件落盘为本断言面）。
    vcx.update(|_window, cx| {
        shell.update(cx, |shell, cx| {
            shell.resolve_confirm(0, _window, cx);
        });
    });
    let edited = cell_text_on_disk(&path, 0, &a1).expect("编辑格非空");
    assert!(edited.contains("246"), "{a1} 退出保存落盘（得 {edited}）");

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// #604 验收 4（失败拒关）：真实编辑 → 删除夹具目录（写盘临时
/// 文件创建 ENOENT——root 也拦不住的真实 IO 失败）→ 菜单保存
/// Failed + toast + 脏保持 → Ctrl+W 确认「保存」→ 保存失败**不
/// 关标签**（脏态保留不静默丢）。
#[gpui::test]
fn xlsx_save_failure_keeps_dirty_and_blocks_close(cx: &mut gpui::TestAppContext) {
    let path = xlsx_fixture("fail");
    let (shell, window) = shell_window(cx);
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_document_tab(path.clone(), cx);
            });
        })
        .expect("开 xlsx 标签");
    cx.run_until_parked();
    let _ = real_grid_edit(&shell, &window, cx, "123").expect("真实编辑置脏");

    // 注入真实写失败：夹具目录整删（write_xlsx_atomically 同目录
    // 临时文件创建 ENOENT；会话在内存，读面不受影响）。
    std::fs::remove_dir_all(path.parent().unwrap()).expect("删夹具目录注入失败");

    // 菜单保存 → Failed（toast 示警不静默）+ 脏保持 + 标签在场。
    window
        .update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.dispatch_editor_cmd(Cmd::Save, window, cx);
            });
        })
        .expect("失败面保存");
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                assert!(shell.tab_is_modified(0, cx).unwrap(), "失败后脏保持");
                assert_eq!(shell.tab_count(), 1, "失败不动标签");
                let notice = shell.notice_board.message().map(str::to_owned);
                assert!(
                    notice.as_deref().is_some_and(|m| m.contains("保存失败")),
                    "失败 toast 在场（得 {notice:?}）"
                );
            });
        })
        .expect("失败断言");

    // Ctrl+W → 确认「保存」→ 保存失败 → 标签不关（Failed 拒关）。
    press_key(cx, &window, "w", None, true);
    press_key(cx, &window, "enter", None, false);
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                assert_eq!(shell.tab_count(), 1, "保存失败不关标签（脏态保留不静默丢）");
                assert!(shell.confirm.is_none(), "确认框已消费");
                assert!(
                    shell.tab_is_modified(0, cx).unwrap(),
                    "拒关后脏保持（可重试保存）"
                );
            });
        })
        .expect("拒关断言");
    // 夹具目录已删——无清理面。
}

/// #604 验收 5（跨标签键位面 + 会话银行往返）：xlsx+md 双标签 →
/// xlsx 真实编辑 → 切 md → 切回（会话经银行 stash/回接，脏随行）
/// → 焦点在文本面（网格不持焦）→ 真实 Ctrl+S 冒泡至 shell →
/// ShellAction::Save → save_tab → 文件落盘。
#[gpui::test]
fn xlsx_cross_tab_ctrl_s_saves_session_bank(cx: &mut gpui::TestAppContext) {
    let path = xlsx_fixture("crosstab");
    let dir = path.parent().unwrap();
    let md = dir.join("notes.md");
    std::fs::write(&md, "N0\n").expect("写 md 夹具");
    let (shell, window) = shell_window(cx);
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_document_tab(path.clone(), cx);
                shell.open_document_tab(md.clone(), cx);
            });
        })
        .expect("开双标签");
    cx.run_until_parked();

    // 切回 xlsx（active 0）做真实编辑。
    window
        .update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.activate_tab(0, window, cx);
            });
        })
        .expect("切回 xlsx");
    cx.run_until_parked();
    let a1 = real_grid_edit(&shell, &window, cx, "135").expect("真实编辑置脏");

    // 切走（md 持焦）再切回（xlsx）——会话银行 stash/回接往返，
    // 脏随行；回接后焦点落网格视图（#604 修复：activate_tab_focus
    // 与 focus_active 同源——网格态主显示面）。
    window
        .update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.activate_tab(1, window, cx);
                shell.activate_tab(0, window, cx);
            });
        })
        .expect("银行往返");
    cx.run_until_parked();
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                assert!(shell.tab_is_modified(0, cx).unwrap(), "银行往返后脏随行");
            });
        })
        .expect("脏随行断言");

    // 切到文本视图（状态栏「网格/文本」钮同一调用面——#604 修复
    // 后切换即重聚焦：文本编辑器持焦，键事件可达）。
    window
        .update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.toggle_xlsx_view_mode(window, cx);
            });
        })
        .expect("切文本视图");
    cx.run_until_parked();

    // 真实 Ctrl+S（焦点在文本面——网格 Ctrl+S 直通不触发，键位
    // 冒泡至 shell 根 → ShellAction::Save → save_tab xlsx 分流）。
    press_key(cx, &window, "s", None, true);
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                assert!(
                    !shell.tab_is_modified(0, cx).unwrap(),
                    "Ctrl+S 后会话脏清除"
                );
                assert_eq!(shell.tab_count(), 2, "保存不动标签结构");
            });
        })
        .expect("保存断言");
    let edited = cell_text_on_disk(&path, 0, &a1).expect("编辑格非空");
    assert!(edited.contains("135"), "{a1} 跨标签保存落盘（得 {edited}）");

    let _ = std::fs::remove_dir_all(dir);
}
#[gpui::test]
fn xlsx_quit_write_failure_keeps_retry_confirmation(cx: &mut gpui::TestAppContext) {
    let path = xlsx_fixture("quit-fail");
    let (shell, window) = shell_window(cx);
    window
        .update(cx, |_, _, app| {
            shell.update(app, |shell, cx| {
                shell.open_document_tab(path.clone(), cx);
            });
        })
        .unwrap();
    cx.run_until_parked();
    real_grid_edit(&shell, &window, cx, "321").expect("real grid edit");
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    window
        .update(cx, |_, _, app| {
            shell.update(app, |shell, cx| {
                shell.confirm = Some(ConfirmState {
                    message: "有未保存的文档".into(),
                    detail: "退出前保存？".into(),
                    action: ConfirmAction::Quit,
                    selected: 0,
                    buttons: CONFIRM_BUTTONS.to_vec(),
                });
                cx.notify();
            });
        })
        .unwrap();
    press_key(cx, &window, "enter", None, false);
    window
        .update(cx, |_, _, app| {
            shell.update(app, |shell, cx| {
                assert_eq!(shell.tab_count(), 1);
                assert!(shell.tab_is_modified(0, cx).unwrap());
                assert!(
                    shell
                        .confirm
                        .as_ref()
                        .is_some_and(|c| matches!(c.action, ConfirmAction::Quit)),
                    "failed save must retain the quit dialog for retry rather than quit"
                );
            });
        })
        .unwrap();
}
