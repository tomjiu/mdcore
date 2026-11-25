//! #47 巨石拆解批③：原 shell.rs 内嵌 `menu_bar_lifecycle_tests` 纯迁出（零行为变化）。

use super::*;
// menu crate（action 定义——批 3 起依赖）：本地 `menu` 变量名会
// 遮蔽 crate 名，测试内以别名引用。
use menu as menu_crate;

/// 测试宿主（ShellHost 同款——模块间不共享私有设施，各 10 行）。
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

/// 开合状态桥：open（实体在场 + open 序）→ 切换（重建实体）→
/// close（全清）→ 重开；外加组件 Cancel 路径（DismissEvent →
/// 订阅回调清态——点外关闭的等价通道）。
#[gpui::test]
fn menu_bar_open_switch_close_lifecycle(cx: &mut gpui::TestAppContext) {
    let (shell, window) = shell_window(cx);
    window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            // 初始全关。
            assert_eq!(shell.menu_bar_open, None);
            assert!(shell.menu_bar_entity.is_none());

            // 开文件菜单（标题点击路径——anchor_x = 点击位）。
            shell.open_menu_bar(0, 120.0, window, cx);
            assert_eq!(shell.menu_bar_open, Some(0), "开菜单 → open 序");
            let first = shell.menu_bar_entity.clone().expect("菜单实体构建在场");
            assert!(
                shell.menu_bar_subscription.is_some(),
                "DismissEvent 订阅在场"
            );

            // 一键切换到帮助菜单（idx 3）——实体重建（旧订阅退役）。
            shell.open_menu_bar(3, 400.0, window, cx);
            assert_eq!(shell.menu_bar_open, Some(3), "切换 → 新 open 序");
            assert_ne!(
                shell.menu_bar_entity.as_ref(),
                Some(&first),
                "切换 = 重建实体（旧实体不残留）"
            );

            // 关闭 → 全清（Esc / 点外部路径的 shell 侧终点）。
            shell.close_menu_bar(window, cx);
            assert_eq!(shell.menu_bar_open, None, "关闭 → open 清");
            assert!(shell.menu_bar_entity.is_none(), "关闭 → 实体清");
            assert!(shell.menu_bar_subscription.is_none(), "关闭 → 订阅退役");

            // 可重开。
            shell.open_menu_bar(2, 300.0, window, cx);
            assert_eq!(shell.menu_bar_open, Some(2), "关闭后可重开");
        });
    });
}

/// 组件 Cancel → DismissEvent → shell 清态链（菜单实体的
/// `menu::Cancel` action 处理器 emit DismissEvent——shell 订阅
/// 回调收口；等效于「点菜单外部」与「Esc」两条关闭路）。
///
/// emit 的派发走 effect 队列（异步）——cancel 后 `run_until_parked`
/// 等订阅回调实际送达再断言（TestAppContext 的标准等待原语）。
#[gpui::test]
fn menu_bar_cancel_action_dismisses(cx: &mut gpui::TestAppContext) {
    let (shell, window) = shell_window(cx);
    let menu = window
        .update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_menu_bar(0, 120.0, window, cx);
            });
            // 读实体（update 返回 Result——TestWindow 签名）。
            shell
                .read(cx)
                .menu_bar_entity
                .clone()
                .expect("菜单实体在场")
        })
        .expect("宿主窗口更新");
    // 组件公有的 cancel 处理器（on_action 同一入口——键盘 Esc 经
    // KeyBinding 到达的就是它，批 4 的 bind_keys 注册面）。
    window.update(cx, |_host, window, cx| {
        menu.update(cx, |menu, cx| {
            menu.cancel(&menu_crate::Cancel, window, cx);
        });
    });
    // DismissEvent 派发 → shell 订阅回调（close_menu_bar_internal）。
    cx.run_until_parked();
    shell.update(cx, |shell, _| {
        assert_eq!(
            shell.menu_bar_open, None,
            "Cancel → DismissEvent → 订阅回调清态"
        );
        assert!(shell.menu_bar_entity.is_none(), "实体随清态退役");
    });
}
