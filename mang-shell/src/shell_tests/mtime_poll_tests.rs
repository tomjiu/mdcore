//! #47 巨石拆解批③：原 shell.rs 内嵌 `mtime_poll_tests` 纯迁出（零行为变化）。

fn temp_md(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mang-mtime-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("note.md");
    std::fs::write(&p, "# 起步\n旧内容\n").unwrap();
    p
}

/// 干净标签：外部改 → 自动重载 + toast。
#[gpui::test]
fn mtime_poll_reloads_clean_tab(cx: &mut gpui::TestAppContext) {
    let path = temp_md("clean");
    let (shell, mut visual) = super::workspace_layout_tests::host(cx);
    // host() 自带 editor（layout.md）——用真文件开标签（走 open_document_tab
    // 链含基线建立）：
    let shell2 = shell.clone();
    let p2 = path.clone();
    visual.update(|_window, app| {
        shell2.update(app, |shell, cx| {
            let _ = shell.open_document_tab(p2, cx);
        });
    });
    // 首步建基线：
    visual.update(|_window, app| {
        shell2.update(app, |shell, cx| {
            let n = shell.poll_mtimes_once(cx);
            assert_eq!(n, 0, "首步建基线——零变化");
        });
    });
    // 外部改写（mtime 变 + 内容变）。
    std::fs::write(&path, "# 起步\n外部新内容\n").unwrap();
    visual.update(|_window, app| {
        shell2.update(app, |shell, cx| {
            let n = shell.poll_mtimes_once(cx);
            assert_eq!(n, 1, "感知 1 个变化");
        });
    });
    visual.update(|_window, app| {
        shell2.read_with(app, |shell, _| {
            let idx = shell.tabs.iter().position(|t| t.path == path).unwrap();
            let text = shell.tab_editor(idx).unwrap().read(app).buffer_line(1);
            assert!(text.contains("外部新内容"), "重载后内容同步：{text:?}");
            let msg = shell.notice_board.message().map(str::to_owned);
            assert!(
                msg.as_deref().is_some_and(|m| m.contains("已重载")),
                "toast：{msg:?}"
            );
        });
    });
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// 脏标签：外部改 → 不自动重载（警示交还决策权）。
#[gpui::test]
fn mtime_poll_warns_not_overwrites_dirty_tab(cx: &mut gpui::TestAppContext) {
    let path = temp_md("dirty");
    let (shell, mut visual) = super::workspace_layout_tests::host(cx);
    let shell2 = shell.clone();
    let p2 = path.clone();
    visual.update(|_window, app| {
        shell2.update(app, |shell, cx| {
            let _ = shell.open_document_tab(p2, cx);
        });
    });
    visual.update(|_window, app| {
        shell2.update(app, |shell, cx| {
            let _ = shell.poll_mtimes_once(cx);
        });
    });
    // 用户编辑（脏）——insert_text_at 先例（dirty_close 测试同法）。
    let dirty_idx = shell2.read_with(&visual, |shell, _| {
        shell.tabs.iter().position(|t| t.path == path).unwrap()
    });
    shell2.update(&mut visual, |shell, cx| {
        let ed = shell.tab_editor(dirty_idx).unwrap().clone();
        ed.update(cx, |ed, cx| {
            ed.insert_text_at(0, 2, "X", cx);
        });
        assert!(
            shell.tab_is_modified(dirty_idx, cx).unwrap_or(false),
            "前置：编辑后标签脏"
        );
    });
    // 外部改写。
    std::fs::write(&path, "# 外部覆盖\n").unwrap();
    visual.update(|_window, app| {
        shell2.update(app, |shell, cx| {
            let n = shell.poll_mtimes_once(cx);
            assert_eq!(n, 1, "感知变化");
        });
    });
    visual.update(|_window, app| {
        shell2.read_with(app, |shell, _| {
            // 不自动重载（脏保护——Buffer 仍是用户编辑态）。
            let text = shell
                .tab_editor(dirty_idx)
                .unwrap()
                .read(app)
                .buffer_line(0);
            assert!(
                text.contains("X") || text.contains("起步"),
                "未被外部覆盖：{text:?}"
            );
            let msg = shell.notice_board.message().map(str::to_owned);
            assert!(
                msg.as_deref().is_some_and(|m| m.contains("外部修改")),
                "警示 toast：{msg:?}"
            );
        });
    });
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
