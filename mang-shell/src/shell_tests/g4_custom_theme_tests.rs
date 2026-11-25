//! #47 巨石拆解批③：原 shell.rs 内嵌 `g4_custom_theme_tests` 纯迁出（零行为变化）。

use super::*;

/// #110 G1 批B：四态轴——具名态切换（user:<name>）与回退（dark）。
#[gpui::test]
fn user_mode_switch_and_fallback(cx: &mut gpui::TestAppContext) {
    let xdg = std::env::temp_dir().join(format!("mang-g1b-mode-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&xdg);
    std::fs::create_dir_all(xdg.join("mang").join("themes")).unwrap();
    let prev_xdg = std::env::var_os("XDG_CONFIG_HOME").unwrap_or_default();
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &xdg) };

    let (shell, mut visual) = super::workspace_layout_tests::host(cx);
    // 保存一个具名主题（名输入框驱动——B 批链）。
    visual.update(|_window, app| {
        shell.update(app, |shell, cx| {
            shell.set_theme_name_input("g1b-alpha".into());
            let path = shell.save_custom_theme(cx);
            assert!(path.is_some(), "具名保存成功");
            assert!(path.unwrap().ends_with("g1b-alpha.json"), "文件名按输入名");
            // 保存即切 user: 态（B 批语义）。
            assert_eq!(shell.theme_mode, "user:g1b-alpha", "保存后四态落位");
        });
    });
    // 全局主题名 = g1b-alpha。
    visual.update(|_window, app| {
        let name = theme::GlobalTheme::theme(app).name.clone();
        assert_eq!(name.as_ref(), "g1b-alpha", "全局主题切到具名");
    });
    // set_theme_mode 切回 dark（三态轴并存——坏名回退同链）。
    visual.update(|_window, app| {
        shell.update(app, |shell, cx| {
            shell.set_theme_mode("dark", cx);
            assert_eq!(shell.theme_mode, "dark");
        });
    });
    // 再切回 user:（apply_mode 具名分支 registry 命中）。
    visual.update(|_window, app| {
        shell.update(app, |shell, cx| {
            shell.set_theme_mode("user:g1b-alpha", cx);
            assert_eq!(shell.theme_mode, "user:g1b-alpha");
            let name = theme::GlobalTheme::theme(cx).name.clone();
            assert_eq!(name.as_ref(), "g1b-alpha", "registry 命中即切");
        });
    });
    // 坏名 user:不存在（apply_mode 回退三态轴——mode 不变坏名，
    // GlobalTheme 走 base 三态）。
    visual.update(|_window, app| {
        shell.update(app, |shell, cx| {
            shell.set_theme_mode("user:no-such", cx);
            // mode 落位仍是坏名（持久化原样），但主题回 base（不炸）。
            assert_eq!(shell.theme_mode, "user:no-such", "mode 原样记录");
            let name = theme::GlobalTheme::theme(cx).name.clone();
            assert_ne!(name.as_ref(), "no-such", "坏名不炸（回退非具名）");
        });
    });
    unsafe { std::env::set_var("XDG_CONFIG_HOME", prev_xdg) };
    let _ = std::fs::remove_dir_all(&xdg);
}

/// #110 G1 批B：删除链——删盘 + registry 移除 + 正用回退 dark。
#[gpui::test]
fn delete_user_theme_chain(cx: &mut gpui::TestAppContext) {
    let xdg = std::env::temp_dir().join(format!("mang-g1b-del-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&xdg);
    std::fs::create_dir_all(xdg.join("mang").join("themes")).unwrap();
    let prev_xdg = std::env::var_os("XDG_CONFIG_HOME").unwrap_or_default();
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &xdg) };

    let (shell, mut visual) = super::workspace_layout_tests::host(cx);
    // 建两个主题（alpha + beta），选中 alpha。
    visual.update(|_window, app| {
        shell.update(app, |shell, cx| {
            shell.set_theme_name_input("g1b-alpha".into());
            assert!(shell.save_custom_theme(cx).is_some());
            shell.set_theme_name_input("g1b-beta".into());
            assert!(shell.save_custom_theme(cx).is_some());
            // beta 保存即选中——切回 alpha 再删 alpha？不：删「当前正用」
            // 的回退链才是断言重点。当前 = user:g1b-beta。
            assert_eq!(shell.theme_mode, "user:g1b-beta");
        });
    });
    // 枚举面：两主题都在。
    visual.update(|_window, app| {
        shell.update(app, |shell, cx| {
            let names = shell.user_theme_names(cx);
            assert!(names.contains(&"g1b-alpha".to_string()), "alpha 在列");
            assert!(names.contains(&"g1b-beta".to_string()), "beta 在列");
        });
    });
    // 删 alpha（非当前）——mode 不动。
    visual.update(|_window, app| {
        shell.update(app, |shell, cx| {
            assert!(shell.delete_user_theme("g1b-alpha", cx));
            assert_eq!(shell.theme_mode, "user:g1b-beta", "删非当前不动 mode");
            assert!(
                !shell
                    .user_theme_names(cx)
                    .contains(&"g1b-alpha".to_string())
            );
        });
    });
    // 删 beta（当前正用）——回退 dark + registry 无 beta。
    visual.update(|_window, app| {
        shell.update(app, |shell, cx| {
            assert!(shell.delete_user_theme("g1b-beta", cx));
            assert_eq!(shell.theme_mode, "dark", "删当前回退深色");
            let names = shell.user_theme_names(cx);
            assert!(!names.contains(&"g1b-beta".to_string()), "registry 无 beta");
            let mode_saved = crate::settings::load_theme_mode();
            assert_eq!(mode_saved.as_deref(), Some("dark"), "回退写盘");
        });
    });
    // 删不存在 → false。
    visual.update(|_window, app| {
        shell.update(app, |shell, cx| {
            assert!(!shell.delete_user_theme("g1b-none", cx), "无该主题 false");
        });
    });
    unsafe { std::env::set_var("XDG_CONFIG_HOME", prev_xdg) };
    let _ = std::fs::remove_dir_all(&xdg);
}

/// #110 G1 批B：名输入过滤（非法字符拒 + 40 界）。
#[gpui::test]
fn theme_name_input_filters(cx: &mut gpui::TestAppContext) {
    let (shell, mut visual) = super::workspace_layout_tests::host(cx);
    visual.update(|_window, app| {
        shell.update(app, |shell, _cx| {
            shell.set_theme_name_input("good-name".into());
            assert_eq!(shell.theme_name_input(), "good-name");
            // 路径分隔符拒（磁盘安全）。
            shell.set_theme_name_input("bad/name".into());
            assert_eq!(shell.theme_name_input(), "badname", "斜杠滤除");
            // 控制字符拒。
            shell.set_theme_name_input("a\u{1}b".into());
            assert_eq!(shell.theme_name_input(), "ab", "控制符滤除");
            // 40 界。
            let long = "x".repeat(60);
            shell.set_theme_name_input(long.clone());
            assert_eq!(shell.theme_name_input().chars().count(), 40, "40 界");
        });
    });
}

/// 空草稿 → 槽默认当前主题实际色 → 写盘 + 装载 + 全局主题切换。
#[gpui::test]
fn save_custom_theme_writes_loads_and_switches(cx: &mut gpui::TestAppContext) {
    let xdg = std::env::temp_dir().join(format!("mang-g4-save-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&xdg);
    std::fs::create_dir_all(xdg.join("mang").join("themes")).unwrap();
    let prev_xdg = std::env::var_os("XDG_CONFIG_HOME").unwrap_or_default();
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &xdg) };

    let (shell, mut visual) = super::workspace_layout_tests::host(cx);
    // 空草稿（初建）→ 保存应成功：6 槽全默认当前色。
    visual.update(|_window, app| {
        shell.update(app, |shell, cx| {
            let path = shell.save_custom_theme(cx);
            assert!(path.is_some(), "空草稿保存成功（槽默认当前色）");
            let p = path.unwrap();
            assert!(p.ends_with("my-theme.json"), "文件名：{:?}", p);
        });
    });
    // 全局主题已切到 my-theme（GlobalTheme 名）。
    visual.update(|_window, app| {
        let name = theme::GlobalTheme::theme(app).name.clone();
        assert_eq!(name.as_ref(), "my-theme", "全局主题切换 my-theme");
    });
    // 坏槽 → None 拒。
    visual.update(|_window, app| {
        shell.update(app, |shell, cx| {
            shell.set_custom_theme_slot(0, "#zzzzz".into()); // 坏 hex（6 字）
            assert!(
                shell.save_custom_theme(cx).is_none(),
                "坏槽拒保存（行内报错同源判定）"
            );
        });
    });
    // XDG 还原。
    unsafe { std::env::set_var("XDG_CONFIG_HOME", prev_xdg) };
    let _ = std::fs::remove_dir_all(&xdg);
}

/// 草稿合法自定义值 → 写盘内容带槽色（读回 JSON 验）。
#[gpui::test]
fn save_custom_theme_reflects_draft(cx: &mut gpui::TestAppContext) {
    let xdg = std::env::temp_dir().join(format!("mang-g4-draft-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&xdg);
    std::fs::create_dir_all(xdg.join("mang").join("themes")).unwrap();
    let prev_xdg = std::env::var_os("XDG_CONFIG_HOME").unwrap_or_default();
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &xdg) };

    let (shell, mut visual) = super::workspace_layout_tests::host(cx);
    visual.update(|_window, app| {
        shell.update(app, |shell, cx| {
            shell.set_custom_theme_slot(0, "#1a2b3c".into());
            let path = shell.save_custom_theme(cx).expect("合法草稿保存成功");
            let text = std::fs::read_to_string(&path).unwrap();
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(v["background"], "#1a2b3c", "草稿背景槽落盘");
        });
    });
    unsafe { std::env::set_var("XDG_CONFIG_HOME", prev_xdg) };
    let _ = std::fs::remove_dir_all(&xdg);
}
