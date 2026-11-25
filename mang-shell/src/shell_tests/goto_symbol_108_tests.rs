//! #47 巨石拆解批③：原 shell.rs 内嵌 `goto_symbol_108_tests` 纯迁出（零行为变化）。

use super::*;

/// #108 P2③：转到符号（Ctrl+Shift+O / goto_symbol——开右栏 +
/// 大纲过滤框聚焦；无大纲提示不静默）。构造走 `workspace_layout_
/// tests::host` 同型（带 editor → 大纲在场）。
#[gpui::test]
fn goto_symbol_opens_right_and_focuses_outline_filter(cx: &mut gpui::TestAppContext) {
    let (shell, mut visual) = super::workspace_layout_tests::host(cx);
    visual.update(|_window, app| {
        shell.update(app, |shell, _| {
            shell.right_open = false;
        });
    });
    visual.update(|window, app| {
        shell.update(app, |shell, cx| {
            shell.goto_symbol(window, cx);
        });
    });
    shell.read_with(&visual, |shell, _| {
        assert!(shell.right_open, "goto_symbol 开右栏");
    });
    visual.update(|window, app| {
        shell.read_with(app, |shell, _| {
            let panel = shell.outline.as_ref().expect("带 editor → 大纲在场");
            let filter = panel.read(app).filter_focus_handle();
            assert!(
                filter.contains_focused(window, app),
                "焦点落大纲过滤框（输入即符号过滤）"
            );
        });
    });
    // 无大纲路径：置空 outline → 提示不静默。
    visual.update(|window, app| {
        shell.update(app, |shell, cx| {
            shell.outline = None;
            shell.notice_board.clear();
            shell.goto_symbol(window, cx);
            let msg = shell.notice_board.message().map(str::to_owned);
            assert!(
                msg.as_deref().is_some_and(|m| m.contains("无文档大纲")),
                "无大纲提示不静默：{msg:?}"
            );
        });
    });
}
