//! #47 巨石拆解批③：原 shell.rs 内嵌 `settings_lifecycle_tests` 纯迁出（零行为变化）。

use super::*;

/// 测试宿主（**shell 必须进渲染树**——pdf_search_wiring_tests 的
/// WiringHost 同款：空 div 宿主会把整棵树挡在派发树外，键事件
/// 无处可派发）。
struct Host(Entity<ShellApp>);

impl gpui::Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div().size_full().child(self.0.clone())
    }
}

/// 空态 ShellApp + 宿主窗口（theme::init + provider 序列镜像
/// pdf_search_wiring_tests——整树渲染需要 provider 在位；空态
/// 焦点落 root〔shell.rs:439〕——键事件按焦点路径派发）。
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

/// 真实 Esc 派发（tab_workflow_tests 的 Ctrl+F 烟雾同构——先强制
/// 一帧绘制让派发树在位，再 dispatch_event；非直调方法断言）。
fn press_escape(cx: &mut gpui::TestAppContext, window: &gpui::WindowHandle<Host>) {
    let any: gpui::AnyWindowHandle = (*window).into();
    cx.update_window(any, |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .expect("预绘一帧（派发树在位）");
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
}

/// 验收 2：settings 开合循环——toggle 开 → **真实 Esc 派发**关 →
/// 重开 → close_settings（Esc / 点遮罩 / 关闭钮同一条链的 shell
/// 侧终点）关 → 再开再 Esc（循环可靠，非一次性巧合）。
#[gpui::test]
fn settings_modal_cycle_real_escape_dispatch(cx: &mut gpui::TestAppContext) {
    let (shell, window) = shell_window(cx);
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                assert!(!shell.settings_open, "前置：空态关闭");
                shell.toggle_settings(cx);
                assert!(shell.settings_open, "toggle 开");
            });
        })
        .expect("开");
    // 真实 Esc（焦点在 root——on_key_down 监听链收口）。
    press_escape(cx, &window);
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                assert!(
                    !shell.settings_open,
                    "真实 Esc 派发 → 关（shell-root 监听链）"
                );
                // 重开 → close_settings（遮罩/关闭钮同链终点）。
                shell.toggle_settings(cx);
                assert!(shell.settings_open, "重开");
                shell.close_settings(cx);
                assert!(
                    !shell.settings_open,
                    "close_settings 关（Esc/遮罩/关闭钮同链）"
                );
                // 再开 → 第二轮 Esc。
                shell.toggle_settings(cx);
            });
        })
        .expect("重开");
    press_escape(cx, &window);
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, _| {
                assert!(!shell.settings_open, "第二轮真实 Esc 仍关（循环可靠）");
            });
        })
        .expect("断言");
}

/// 验收 3：**Esc 分派优先序锁定**——settings_open + confirm 并置
/// （直接状态置位模拟「两弹窗同开」违规态；产线经 dirty 标签关闭/
/// 退出路径产生，此处锁的是分派序不锁产径）：第一记 Esc 先关设置
/// 且确认框保持（shell.rs on_key_down 序：菜单栏 > 右键菜单 >
/// 设置弹窗 > 确认框 F4）；第二记 Esc 进 F4 模态拦截关确认框。
/// 序即行为契约——倒序会让确认框「吞掉」设置关闭路径。
#[gpui::test]
fn escape_priority_settings_before_confirm(cx: &mut gpui::TestAppContext) {
    let (shell, window) = shell_window(cx);
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                shell.settings_open = true;
                shell.confirm = Some(ConfirmState {
                    message: "有 1 个未保存的文档".to_string(),
                    detail: "退出前保存修改？".to_string(),
                    action: ConfirmAction::Quit,
                    selected: 0,
                    buttons: CONFIRM_BUTTONS.to_vec(),
                });
                cx.notify();
            });
        })
        .expect("并置态");
    press_escape(cx, &window);
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, _| {
                assert!(!shell.settings_open, "第一记 Esc：设置先关（分派序在前）");
                assert!(shell.confirm.is_some(), "确认框保持（F4 拦截链未到）");
            });
        })
        .expect("断言①");
    press_escape(cx, &window);
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, _| {
                assert!(shell.confirm.is_none(), "第二记 Esc：F4 模态拦截关确认框");
            });
        })
        .expect("断言②");
}

/// 验收 4a：右缘 dock 开合——right_open 折叠/重开（状态级断言：
/// 真实点击路径〔dock-toggle-right〕的等价状态终点；#289-⑤ 右栏
/// 开关归右缘 dock 条）。
#[gpui::test]
fn right_dock_toggle_cycle(cx: &mut gpui::TestAppContext) {
    let (shell, _window) = shell_window(cx);
    shell.update(cx, |shell, cx| {
        assert!(shell.right_open, "前置：缺省开（shell.rs:387）");
        shell.toggle_right(cx);
        assert!(!shell.right_open, "折叠");
        shell.toggle_right(cx);
        assert!(shell.right_open, "重开");
    });
}

/// 验收 4b：左栏/git 视图开合双分支——toggle_left（git 态回切
/// 文件树不折叠 / 非 git 态折叠展开）+ toggle_git（git 态折叠 /
/// 非 git 态〔含栏折叠〕双置直达）。#119：git_view 布尔退役，
/// 视图归属从 dock 激活键读（git_active——真实激活协议终点）。
#[gpui::test]
fn left_git_toggle_branches(cx: &mut gpui::TestAppContext) {
    let (shell, _window) = shell_window(cx);
    shell.update(cx, |shell, cx| {
        // 前置：左栏开（dock is_open 缺省）/ 非 git 视图。
        assert!(shell.left_open(cx) && !shell.git_active(cx));

        shell.toggle_git(cx);
        assert!(
            shell.left_open(cx) && shell.git_active(cx),
            "toggle_git：非 git 态直达"
        );

        // toggle_left 分支①：git 态 → 回切文件树（栏不折叠；
        // 预物化意图被实际换面丢弃——树视图承接）。
        shell.toggle_left(cx);
        assert!(
            shell.left_open(cx) && !shell.git_active(cx),
            "toggle_left：git 态回切文件树"
        );

        // toggle_left 分支②：非 git 态 → 折叠/展开。
        shell.toggle_left(cx);
        assert!(!shell.left_open(cx), "toggle_left：非 git 态折叠");
        shell.toggle_left(cx);
        assert!(shell.left_open(cx), "toggle_left：重开");

        // toggle_git 分支②：git 态 → 折叠左栏（git 激活态保持——
        // 重开直达 git 视图的状态记忆）。
        shell.toggle_git(cx);
        assert!(shell.left_open(cx) && shell.git_active(cx), "再直达");
        shell.toggle_git(cx);
        assert!(
            !shell.left_open(cx) && shell.git_active(cx),
            "toggle_git：git 态折叠左栏（git 激活态保持）"
        );

        // 折叠态直达 git：双置（左栏开 + git 激活）。
        shell.toggle_git(cx);
        assert!(
            shell.left_open(cx) && shell.git_active(cx),
            "折叠态 toggle_git：双置直达"
        );
    });
}

/// #114 P1④：关窗先存链——标签快照面 + settings session_tabs/
/// window_bounds 双键落盘一致性（钩注册面在 main——本测验证钩内
/// 落存调用的数据面：快照 = tabs 真实序）。
#[gpui::test]
fn close_persist_snapshot_matches_tabs(cx: &mut gpui::TestAppContext) {
    // XDG 全局窗口互斥（见 test_env 模块头）。
    let _xdg = crate::test_env::xdg_lock();
    let tmp = std::env::temp_dir().join(format!("mclose-persist-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    let prev = std::env::var_os("XDG_CONFIG_HOME");
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &tmp) };
    let file = tmp.join("sess-a.md");
    std::fs::write(&file, "# 会话标签 A").unwrap();
    let (shell, _window) = shell_window(cx);
    // 快照只读面（关窗钩读——shell 实体直读）。
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.tabs_paths_snapshot().len(), 0, "初始零标签快照");
        assert_eq!(shell.active_index(), 0);
    });
    match prev {
        Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },
        None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

/// #120（批②）验收 ④：开合态跨「重启」恢复——收起右栏/左栏 →
/// 新建 ShellApp（读 settings 恢复）→ 仍是收起。环境隔离：
/// 临时 XDG_CONFIG_HOME（settings.json 全落临时区，测试结束还原）。
#[gpui::test]
fn dock_open_states_persist_across_restart(cx: &mut gpui::TestAppContext) {
    // XDG 全局窗口互斥（见 test_env 模块头）。
    let _xdg = crate::test_env::xdg_lock();
    let tmp = std::env::temp_dir().join(format!("mdock-restart-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    let prev = std::env::var_os("XDG_CONFIG_HOME");
    // 测试进程单线程跑（--test-threads=1 门禁）——env 变更不进并行面。
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &tmp) };
    // ① 模拟「上次会话」落盘：左栏收起 + 右栏收起 + 树宽 305。
    let last = mdc_dock::DockAreaState {
        version: mdc_dock::DOCK_LAYOUT_VERSION,
        open: false,
        active: Some(crate::file_tree::FILE_TREE_PANEL_KEY.to_owned()),
        panels: vec![mdc_dock::PanelState {
            key: crate::file_tree::FILE_TREE_PANEL_KEY.to_owned(),
            size: Some(305.),
        }],
    };
    crate::settings::save_dock_layout(&last, false);
    // ② 「重启」：新建 ShellApp（构造尾恢复——面板未物化，尺寸走
    // 预物化意图）。
    let (shell, _window) = shell_window(cx);
    shell.update(cx, |shell, cx| {
        assert!(!shell.left_open(cx), "验收④：左栏收起跨重启保持");
        assert!(!shell.right_open, "验收④：右栏收起跨重启保持");
        assert!(!shell.git_active(cx), "激活键未物化 → 缺省树视图");
    });
    // ③ 「打开文件夹」物化面板——预物化尺寸兑现 + 渲染宽同步。
    let root = std::env::temp_dir().join(format!("mdock-ws-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&root);
    shell.update(cx, |shell, cx| shell.open_workspace(root, cx));
    shell.update(cx, |shell, _cx| {
        let left = f32::from(shell.left_width);
        assert_eq!(left, 305., "验收③：预物化树宽在物化后兑现并同步渲染宽");
        assert!(
            shell.git_panel.is_some(),
            "工作区打开即建 git 面（既例保持）"
        );
    });
    // ④ 清理：还原环境 + 删临时目录（后续测试不受污染）。
    match prev {
        Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },
        None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
