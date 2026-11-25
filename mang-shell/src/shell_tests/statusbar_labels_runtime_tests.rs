//! #47 巨石拆解批①：原 shell.rs 内嵌 `statusbar_labels_runtime_tests` 纯迁出（零行为变化——
//! 字节原样，挂载点 #[cfg(test)] + #[path] 在 shell.rs）。

use super::*;
use crate::editor::buffer::Point;
use crate::status_bar::{StatusBarLabels, status_bar_labels};

fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mdc-sbl-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 写 md 夹具（UTF-8 + CRLF——编码/换行符两探测段的真判据）。
fn write_md(dir: &Path, name: &str, content: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

/// 造真 .git/HEAD（find_git_root + git_branch_at 的实际解析对象——
/// 不启 git 子进程，HEAD 文件即两函数的全部输入）。
fn fake_git_root(dir: &Path, branch: &str) {
    let git = dir.join(".git");
    std::fs::create_dir_all(&git).unwrap();
    std::fs::write(git.join("HEAD"), format!("ref: refs/heads/{branch}\n")).unwrap();
}

struct Host(Entity<ShellApp>);

impl gpui::Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div().size_full().child(self.0.clone())
    }
}

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

/// A5 运行时证据主链：真 md（UTF-8+CRLF+中文）+ 真 .git/HEAD →
/// 文案计算面六段全真值。
#[gpui::test]
fn md_runtime_labels_six_segments_real_values(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("runtime");
    // 中文正文 + CRLF（编码/换行段真判据）；文件名进 display_name。
    let md = write_md(&dir, "状态栏证据.md", "第一行中文\r\n第二行中文\r\n");
    fake_git_root(&dir, "a5-runtime");
    let (shell, window) = shell_window(cx);
    let _ = window.update(cx, |_host, _window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_document_tab(md.clone(), cx);
        });
    });

    let _ = window.update(cx, |_host, _window, cx| {
        let ed = shell.read(cx).tab_editor(0).unwrap().clone();
        let labels: StatusBarLabels = status_bar_labels(Some(&ed), Some(dir.clone()), None, cx);
        // ① 左段：文件名 + 行,列 + 字数（真实光标/字数——光标在文件头）。
        assert!(
            labels.left.contains("状态栏证据.md"),
            "左段应含真实文件名：{}",
            labels.left
        );
        assert!(
            labels.left.contains("行, 列"),
            "左段应含行,列段：{}",
            labels.left
        );
        assert!(
            labels.left.contains("字") || labels.left.contains("字符"),
            "左段应含字数段：{}",
            labels.left
        );
        assert!(
            !labels.left.contains('●'),
            "干净文档无脏标：{}",
            labels.left
        );
        // ② 工作区段（真实路径渲染——workspace_display 产物）。
        let ws = labels.workspace.as_deref().unwrap_or("（缺工作区段）");
        assert!(ws.contains("工作区"), "工作区段文案：{ws}");
        // ③ git 分支：真 .git/HEAD 解析产物（非占位）。
        assert_eq!(
            labels.branch.as_deref(),
            Some(" a5-runtime"),
            "分支段 = 真 HEAD 解析值"
        );
        // ④/⑤ 编码 + 换行符：真字节探测（UTF-8 + CRLF 夹具）。
        assert_eq!(labels.encoding.as_deref(), Some("UTF-8"), "编码段真判");
        assert_eq!(labels.eol.as_deref(), Some("CRLF"), "换行符段真判");
        // ⑥ 无选区不占位。
        assert!(labels.selection.is_none(), "无选区 → 选区段 None");
    });

    // 真实编辑链：insert → 脏标 ●；真实选区 → 选中字数段。
    let _ = window.update(cx, |_host, _window, cx| {
        shell.update(cx, |shell, cx| {
            let ed = shell.tab_editor(0).unwrap().clone();
            ed.update(cx, |ed, cx| {
                ed.insert_text_at(0, 2, "增", cx);
            });
            assert!(shell.tab_is_modified(0, cx).unwrap(), "置脏前置");
        });
    });
    let _ = window.update(cx, |_host, _window, cx| {
        let ed = shell.read(cx).tab_editor(0).unwrap().clone();
        let labels = status_bar_labels(Some(&ed), Some(dir.clone()), None, cx);
        assert!(labels.left.contains('●'), "编辑后脏标：{}", labels.left);
    });

    // 程序化选区（set_selection_range——RPC SetSelection 同链）→
    // 选中字数段真值（含中文正文跨行：字/行双计数）。
    let _ = window.update(cx, |_host, _window, cx| {
        let ed = shell.read(cx).tab_editor(0).unwrap().clone();
        ed.update(cx, |ed, cx| {
            ed.set_selection_range(Point { line: 0, col: 0 }, Point { line: 2, col: 0 }, cx);
        });
    });
    let _ = window.update(cx, |_host, _window, cx| {
        let ed = shell.read(cx).tab_editor(0).unwrap().clone();
        let labels = status_bar_labels(Some(&ed), Some(dir.clone()), None, cx);
        let sel = labels.selection.as_deref().unwrap_or("（缺选区段）");
        assert!(sel.contains("选中"), "选区段文案：{sel}");
        assert!(sel.contains("字"), "选区段含字数：{sel}");
    });
}

/// 无文档就绪态：全部数据段 None + 就绪提示语（零占位纪律）。
#[gpui::test]
fn no_doc_ready_state_all_segments_none(cx: &mut gpui::TestAppContext) {
    let _dir = fixture_dir("ready");
    let (_shell, window) = shell_window(cx);
    let _ = window.update(cx, |_host, _window, cx| {
        let labels: StatusBarLabels = status_bar_labels(None, None, None, cx);
        assert_eq!(
            labels.left, "就绪 —— Ctrl+O 打开文件 · Ctrl+K Ctrl+O 打开文件夹",
            "无文档 = 就绪提示语"
        );
        assert!(labels.workspace.is_none());
        assert!(labels.selection.is_none());
        assert!(labels.branch.is_none());
        assert!(labels.encoding.is_none());
        assert!(labels.eol.is_none(), "无文档 = 全段 None（零占位）");
    });
}

/// 非 git 目录：分支段 None 不占位（单文件打开、目录无 .git）。
#[gpui::test]
fn md_outside_git_repo_branch_segment_none(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("nogit");
    let md = write_md(&dir, "裸文件.md", "内容\n");
    let (shell, window) = shell_window(cx);
    let _ = window.update(cx, |_host, _window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_document_tab(md, cx);
        });
    });
    let _ = window.update(cx, |_host, _window, cx| {
        let ed = shell.read(cx).tab_editor(0).unwrap().clone();
        let labels = status_bar_labels(Some(&ed), None, None, cx);
        assert!(
            labels.branch.is_none(),
            "非 git 目录分支段 None（实得 {:?}）",
            labels.branch
        );
        // 无工作区 → 工作区段同样不占位。
        assert!(labels.workspace.is_none());
    });
}
