//! #47 巨石拆解批③：原 shell.rs 内嵌 `context_menu_lifecycle_tests` 纯迁出（零行为变化）。

use super::*;
// menu crate（action 定义）：cancel 处理器的入参类型来源。
use menu as menu_crate;

/// 测试宿主（ShellHost 同款——模块间不共享私有设施，各 10 行）。
#[allow(dead_code)]
struct Host(Entity<ShellApp>);

impl gpui::Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div()
    }
}

/// 空态 ShellApp + 宿主窗口（theme::init 序列镜像 main.rs）。
fn shell_window(cx: &mut gpui::TestAppContext) -> (Entity<ShellApp>, gpui::WindowHandle<Host>) {
    cx.update(|app| theme::init(theme::LoadThemes::JustBase, app));
    let mut slot: Option<Entity<ShellApp>> = None;
    let window = cx.add_window(|window, cx| {
        let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
        slot = Some(shell.clone());
        Host(shell)
    });
    (slot.expect("宿主构造即置"), window)
}

/// 开合状态桥：编辑器右键开（实体/位置/来源/目标四态）→ 文件树
/// 右键切换（重建实体 + 换来源 + 带目标）→ 关闭全清 → 可重开。
/// 点击位直传 `gpui::Point<Pixels>`（批 1×4 收口后的签名——不再
/// 经自研 f32 Point 往返）。
#[gpui::test]
fn context_menu_open_switch_close_lifecycle(cx: &mut gpui::TestAppContext) {
    let (shell, window) = shell_window(cx);
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            // 初始全关。
            assert!(shell.context_menu.is_none());
            assert!(shell.context_menu_target.is_none());

            // 编辑器右键（点击位 120,80；无选区 + 只读）。
            let pos = gpui::Point::new(px(120.), px(80.));
            shell.open_editor_context_menu(pos, false, true, window, cx);
            let first = shell.context_menu.clone().expect("菜单实体构建在场");
            assert_eq!(shell.context_menu_pos, pos, "点击位原样落状态");
            assert_eq!(shell.context_menu_origin, MenuOrigin::Editor);
            assert!(shell.context_menu_target.is_none(), "编辑器来源无目标路径");

            // 文件树右键切换（重建实体 + 来源/目标换轨）。
            let pos2 = gpui::Point::new(px(40.), px(300.));
            let target = PathBuf::from("/ws/sub/a.md");
            shell.open_file_tree_context_menu(pos2, target.clone(), true, window, cx);
            assert_ne!(
                shell.context_menu.as_ref(),
                Some(&first),
                "切换 = 重建实体（旧实体不残留）"
            );
            assert_eq!(shell.context_menu_pos, pos2, "切换后位置随新点击位");
            assert_eq!(shell.context_menu_origin, MenuOrigin::FileTree);
            assert_eq!(
                shell.context_menu_target.as_deref(),
                Some(target.as_path()),
                "文件树来源带目标路径"
            );

            // 关闭 → 全清（Esc / 点外部路径的 shell 侧终点）。
            shell.close_context_menu(cx);
            assert!(shell.context_menu.is_none(), "关闭 → 实体清");
            assert!(shell.context_menu_target.is_none(), "关闭 → 目标随清");

            // 可重开。
            shell.open_editor_context_menu(pos, true, false, window, cx);
            assert!(shell.context_menu.is_some(), "关闭后可重开");
        });
    });
}

/// 组件 Cancel → DismissEvent → shell 清态链（点外部 / Esc 的
/// 组件内归一路径——spawn_context_menu 订阅回调收口）。emit 走
/// effect 队列（异步），run_until_parked 等送达再断言。
#[gpui::test]
fn context_menu_cancel_action_dismisses(cx: &mut gpui::TestAppContext) {
    let (shell, window) = shell_window(cx);
    let menu = window
        .update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_file_tree_context_menu(
                    gpui::Point::new(px(60.), px(90.)),
                    PathBuf::from("/ws/b.md"),
                    false,
                    window,
                    cx,
                );
            });
            shell.read(cx).context_menu.clone().expect("菜单实体在场")
        })
        .expect("宿主窗口更新");
    // 组件公有 cancel 处理器（on_action 同一入口——键盘 Esc 与点外
    // 关闭经 on_blur 归一到这，批 1 组件语义）。
    let _ = window.update(cx, |_host, window, cx| {
        menu.update(cx, |menu, cx| {
            menu.cancel(&menu_crate::Cancel, window, cx);
        });
    });
    cx.run_until_parked();
    shell.update(cx, |shell, _| {
        assert!(
            shell.context_menu.is_none(),
            "Cancel → DismissEvent → 订阅回调清态"
        );
        assert!(shell.context_menu_target.is_none(), "目标随清态退役");
    });
}

/// 执行动作 → 菜单态先行清空（「选完即关」——activate 的第一段
/// 就是清 context_menu/context_menu_target 再派发）。ed-select-all
/// 在无编辑器宿主下 dispatch 早退（不 panic），此处只钉「点击后
/// 菜单不悬挂」。
#[gpui::test]
fn context_menu_activate_clears_menu_state(cx: &mut gpui::TestAppContext) {
    let (shell, window) = shell_window(cx);
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_editor_context_menu(
                gpui::Point::new(px(30.), px(30.)),
                true,
                false,
                window,
                cx,
            );
            assert!(shell.context_menu.is_some(), "前置：菜单在场");
            shell.activate_context_menu_item("ed-select-all", window, cx);
            assert!(shell.context_menu.is_none(), "执行动作 → 菜单态清");
            assert!(shell.context_menu_target.is_none(), "目标随清");
        });
    });
}
