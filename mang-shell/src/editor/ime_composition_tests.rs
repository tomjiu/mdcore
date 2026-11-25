//! #47 巨石拆解批②：原 editor/mod.rs 内嵌 `ime_composition_tests` 纯迁出（零行为变化——
//! 字节原样剥壳，挂载 #[cfg(test)] + #[path] 在 mod.rs）。

use super::*;
use gpui::EntityInputHandler;

/// 测试宿主（PalHost 同款——trait 方法需要真实 `&mut Window`）。
struct ImeHost {
    _ed: gpui::Entity<Editor>,
}

impl gpui::Render for ImeHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div()
    }
}

fn ime_window(
    cx: &mut gpui::TestAppContext,
    text: &str,
) -> (gpui::Entity<Editor>, gpui::WindowHandle<ImeHost>) {
    let ed = cx.update(|cx| cx.new(|cx| Editor::new(Some("ime.md".into()), text.to_string(), cx)));
    let ed_for_host = ed.clone();
    let window = cx.add_window(move |_window, _cx| ImeHost {
        _ed: ed_for_host.clone(),
    });
    (ed, window)
}

fn plain_key(key: &str, key_char: Option<&str>) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: gpui::Keystroke {
            modifiers: gpui::Modifiers::default(),
            key: key.into(),
            key_char: key_char.map(str::to_string),
        },
        is_held: false,
        prefer_character_input: false,
    }
}

struct InputRouteHost {
    ed: gpui::Entity<Editor>,
}

impl Render for InputRouteHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ed = self.ed.clone();
        gpui::div()
            .track_focus(&ed.read(cx).focus)
            .on_key_down(move |event, window, cx| {
                ed.update(cx, |ed, cx| ed.dispatch_key(event, window, cx));
            })
    }
}

#[gpui::test]
fn input_sequence_shell_keys_propagate(cx: &mut gpui::TestAppContext) {
    let ed = cx.update(|cx| cx.new(|cx| Editor::new(None, "AB".into(), cx)));
    let ed_host = ed.clone();
    let window = cx.add_window(move |window, cx| {
        ed_host.read(cx).focus.clone().focus(window, cx);
        InputRouteHost { ed: ed_host }
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
        for key in ["s", "w", "tab", "k", "o"] {
            let mut event = plain_key(key, None);
            event.keystroke.modifiers.control = true;
            let result = window.dispatch_event(gpui::PlatformInput::KeyDown(event), cx);
            assert!(result.propagate, "shell must receive Ctrl+{key}");
        }
        assert_eq!(ed.read(cx).buffer_line(0), "AB");
    });
}

// Model the platform sequence, not only a synthetic keydown: Windows WM_CHAR
// and the Linux/macOS fallback deliver text after the key event.
#[gpui::test]
fn input_sequence_ascii_once(cx: &mut gpui::TestAppContext) {
    let ed = cx.update(|cx| cx.new(|cx| Editor::new(None, String::new(), cx)));
    let ed_host = ed.clone();
    let window = cx.add_window(move |window, cx| {
        ed_host.read(cx).focus.clone().focus(window, cx);
        InputRouteHost { ed: ed_host }
    });
    cx.update_window(window.into(), |_, window, cx| {
        for ch in ["a", "b", "c"] {
            window.draw(cx).clear(cx);
            let result =
                window.dispatch_event(gpui::PlatformInput::KeyDown(plain_key(ch, Some(ch))), cx);
            assert!(result.propagate, "platform must deliver printable text");
            ed.update(cx, |ed, cx| ed.replace_text_in_range(None, ch, window, cx));
        }
        assert_eq!(ed.read(cx).buffer_line(0), "abc");
        assert!(ed.read(cx).is_modified());
    });
}

#[gpui::test]
fn input_sequence_ime_none_commit(cx: &mut gpui::TestAppContext) {
    let (ed, window) = ime_window(cx, "X");
    let path = std::env::temp_dir().join(format!("mang-input-ime-{}.md", std::process::id()));
    window.update(cx, |_host, window, cx| {
        ed.update(cx, |ed, cx| {
            ed.file_path = Some(path.clone());
            ed.replace_and_mark_text_in_range(None, "zh", Some(1..1), window, cx);
            ed.replace_text_in_range(None, "中", window, cx);
            assert_eq!(ed.buffer_line(0), "中X");
            assert_eq!(ed.marked, None);
            assert_eq!(ed.buffer.cursor(), Point::new(0, 1));
            assert_eq!(ed.save(), SaveOutcome::Saved);
            assert!(!ed.is_modified());
        });
    });
    let saved = std::fs::read_to_string(&path).unwrap();
    let reopened = cx.update(|cx| cx.new(|cx| Editor::new(Some(path.clone()), saved, cx)));
    assert_eq!(cx.update(|cx| reopened.read(cx).buffer_line(0)), "中X");
    std::fs::remove_file(path).unwrap();
}

#[gpui::test]
fn input_sequence_ime_cancel_and_explicit_range(cx: &mut gpui::TestAppContext) {
    let (ed, window) = ime_window(cx, "😀AB");
    window.update(cx, |_host, window, cx| {
        ed.update(cx, |ed, cx| {
            ed.buffer.set_cursor(Point::new(0, 1));
            ed.replace_and_mark_text_in_range(None, "zh", None, window, cx);
            ed.replace_text_in_range(None, "", window, cx);
            assert_eq!(ed.buffer_line(0), "😀AB", "empty commit cancels preedit");
            assert_eq!(ed.marked, None);
            ed.replace_and_mark_text_in_range(None, "zh", None, window, cx);
            // Explicit UTF16 replacement still takes precedence over marked.
            ed.replace_text_in_range(Some(2..5), "中", window, cx);
            assert_eq!(ed.buffer_line(0), "😀中B");
            assert_eq!(ed.marked, None);
        });
    });
}

#[gpui::test]
fn input_sequence_multicursor_callback(cx: &mut gpui::TestAppContext) {
    let (ed, window) = ime_window(cx, "a\nb");
    window.update(cx, |_host, window, cx| {
        ed.update(cx, |ed, cx| {
            ed.cursors.push(Point::new(1, 0), Point::new(1, 0));
            ed.sync_buffer_from_cursors_primary();
            ed.dispatch_key(&plain_key("x", Some("x")), window, cx);
            ed.replace_text_in_range(None, "x", window, cx);
            assert_eq!(ed.buffer_line(0), "xa");
            assert_eq!(ed.buffer_line(1), "xb");
            assert_eq!(ed.cursors.len(), 2);
        });
    });
}

#[gpui::test]
fn input_sequence_character_preference_preserves_selection(cx: &mut gpui::TestAppContext) {
    let (ed, window) = ime_window(cx, "AB");
    window.update(cx, |_host, window, cx| {
        ed.update(cx, |ed, cx| {
            ed.buffer.set_cursor(Point::new(0, 1));
            ed.sync_cursors_from_buffer();
            // AltGr/layout text must win over a coincident Ctrl+A binding.
            let mut event = plain_key("a", Some("á"));
            event.keystroke.modifiers.control = true;
            event.prefer_character_input = true;
            ed.dispatch_key(&event, window, cx);
            ed.replace_text_in_range(None, "á", window, cx);
            assert_eq!(ed.buffer_line(0), "AáB");
        });
    });
}

#[gpui::test]
fn input_sequence_enter_undo_dirty_save(cx: &mut gpui::TestAppContext) {
    let (ed, window) = ime_window(cx, "AB");
    let path = std::env::temp_dir().join(format!("mang-input-enter-{}.md", std::process::id()));
    window.update(cx, |_host, window, cx| {
        ed.update(cx, |ed, cx| {
            ed.file_path = Some(path.clone());
            for col in [0, 1, 2] {
                ed.buffer.set_cursor(Point::new(0, col));
                ed.sync_cursors_from_buffer();
                ed.dispatch_key(&plain_key("enter", None), window, cx);
                assert_eq!(ed.buffer.line_count(), 2);
                assert_eq!(ed.buffer.cursor(), Point::new(1, 0));
                assert!(ed.is_modified());
                let mut undo = plain_key("z", None);
                undo.keystroke.modifiers.control = true;
                ed.dispatch_key(&undo, window, cx);
                assert_eq!(ed.buffer_line(0), "AB");
                assert_eq!(ed.buffer.line_count(), 1);
                assert!(!ed.is_modified());
                let mut redo = plain_key("y", None);
                redo.keystroke.modifiers.control = true;
                ed.dispatch_key(&redo, window, cx);
                assert_eq!(ed.buffer.line_count(), 2);
                assert!(ed.is_modified());
                ed.dispatch_key(&undo, window, cx);
            }
            ed.dispatch_key(&plain_key("enter", None), window, cx);
            assert_eq!(ed.save(), SaveOutcome::Saved);
            assert!(!ed.is_modified());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "AB\n");
            // Enter on the trailing empty line, then continue typing without refocus.
            ed.dispatch_key(&plain_key("enter", None), window, cx);
            assert_eq!(ed.buffer.line_count(), 3);
            ed.dispatch_key(&plain_key("x", Some("x")), window, cx);
            ed.replace_text_in_range(None, "x", window, cx);
            assert_eq!(ed.buffer_line(2), "x");
        });
    });
    std::fs::remove_file(path).unwrap();
}

/// #201：Editor 层链路——编辑发生在任何 wrap 回填之前（零帧
/// render_lines 的回填块从未执行）时，display_row_count 仍须立即
/// 反映 buffer 行数。旧 reset_wraps 空 wraps 空转 → 行表陈旧 →
/// uniform_list 永不派新行（真机形态：脏点亮/磁盘已写/画面无
/// 新增文本）。纯 DisplayMap 单测（display_map.rs）之外，这里锁
/// `after_edit → refresh_caches → reset_wraps` 真实编辑路径。
#[gpui::test]
fn edit_before_wrap_backfill_editor_level_row_freshness(cx: &mut gpui::TestAppContext) {
    let (ed, window) = ime_window(cx, "AB");
    window.update(cx, |_host, _window, cx| {
        ed.update(cx, |ed, cx| {
            // 前置：不调用任何渲染路径——display_map.wraps 保持空
            //（真实形态：开文件后 0/1 帧内的自动化连打）。
            // 查询一次行数构建 RowTable 缓存（首帧 draw 的行为）。
            let before = ed.display_map.display_row_count(&ed.buffer);
            assert_eq!(before, 1, "「AB」1 行");
            // 平台文本通道插入带换行文本（IME commit 形态）：
            // 初始光标 (0,0) 无选区 → 行首插入 → "CD\nEF\nGHAB" 3 行。
            ed.replace_text_in_range(None, "CD\nEF\nGH", _window, cx);
            // #201 断言：编辑后行数立即反映 buffer（旧缺陷形态
            // = 旧表 1 行卡死——首查询构建的缓存永不刷新）
            let after = ed.display_map.display_row_count(&ed.buffer);
            assert_eq!(
                after, 3,
                "#201：wrap 未回填时编辑后行表必须仍新鲜（行首插 CD/EF/GH + AB = 3 行）"
            );
            assert_eq!(ed.buffer.line_count(), 3);
        });
    });
}

/// 预编辑全链：起（ni）→ 更新（你好，带选中）→ 提交（unmark 保文）
/// ——marked 区间（UTF16）逐态断言 + 提交后脏。
#[gpui::test]
fn ime_preedit_update_commit_lifecycle(cx: &mut gpui::TestAppContext) {
    let (ed, window) = ime_window(cx, "AB\n");
    window.update(cx, |_host, window, cx| {
        // 组合起：光标处预编辑 "ni"。
        ed.update(cx, |ed, cx| {
            ed.replace_and_mark_text_in_range(None, "ni", None, window, cx);
        });
        ed.update(cx, |ed, cx| {
            assert_eq!(
                ed.marked_text_range(window, cx),
                Some(0..2),
                "预编辑起：marked = ni 的 UTF16 区间"
            );
            assert_eq!(ed.buffer_line(0), "niAB", "预编辑文本入 buffer");
        });

        // 组合更新：替换旧预编辑段为 "你好"（IME 候选确认的标准流），
        // 带段内选中（new_selected_range——候选栏高亮语义）。
        ed.update(cx, |ed, cx| {
            ed.replace_and_mark_text_in_range(None, "你好", Some(0..2), window, cx);
        });
        ed.update(cx, |ed, cx| {
            assert_eq!(
                ed.marked_text_range(window, cx),
                Some(0..2),
                "更新后 marked = 你好（2 字 × 1 UTF16）"
            );
            assert_eq!(ed.buffer_line(0), "你好AB", "预编辑段整体替换（不叠字）");
            assert_eq!(
                ed.selection_points(),
                Some((Point::new(0, 0), Point::new(0, 2))),
                "new_selected_range 段内选中"
            );
        });

        // 提交：unmark（保文提交——平台提交流之一）。
        ed.update(cx, |ed, cx| {
            ed.unmark_text(window, cx);
            assert_eq!(ed.marked_text_range(window, cx), None, "提交清 marked");
            assert_eq!(ed.buffer_line(0), "你好AB", "提交保文");
            assert!(ed.is_modified(), "组合提交计入脏");
        });
    });
}

/// 提交第二流：replace_text_in_range（终文本替换预编辑段）——
/// marked 清 + 段替换正确（"zh" 预编辑 → "中" 提交）。
#[gpui::test]
fn ime_commit_via_replace_text_swaps_preedit(cx: &mut gpui::TestAppContext) {
    let (ed, window) = ime_window(cx, "X\n");
    window.update(cx, |_host, window, cx| {
        ed.update(cx, |ed, cx| {
            ed.replace_and_mark_text_in_range(None, "zh", None, window, cx);
        });
        ed.update(cx, |ed, cx| {
            assert_eq!(ed.marked_text_range(window, cx), Some(0..2));
        });
        ed.update(cx, |ed, cx| {
            // IME 终选提交通道：替换 0..2（预编辑段）为终文本。
            ed.replace_text_in_range(Some(0..2), "中", window, cx);
            assert_eq!(ed.marked_text_range(window, cx), None, "提交清 marked");
            assert_eq!(ed.buffer_line(0), "中X", "终文本替换预编辑段");
        });
    });
}

/// 只读门禁（IME 双通道）：只读文档不接预编辑/替换——buffer 与
/// marked 均零变化。
#[gpui::test]
fn ime_readonly_rejects_both_channels(cx: &mut gpui::TestAppContext) {
    let ed = cx.update(|cx| {
        cx.new(|cx| Editor::new_readonly(Some("ro.md".into()), "RO\n".to_string(), cx))
    });
    let ed_for_host = ed.clone();
    let window = cx.add_window(move |_window, _cx| ImeHost {
        _ed: ed_for_host.clone(),
    });
    window.update(cx, |_host, window, cx| {
        ed.update(cx, |ed, cx| {
            ed.replace_and_mark_text_in_range(None, "拼", None, window, cx);
            assert_eq!(ed.marked_text_range(window, cx), None, "只读不接预编辑");
            assert_eq!(ed.buffer_line(0), "RO", "buffer 零变化");
            ed.replace_text_in_range(None, "写", window, cx);
            assert_eq!(ed.buffer_line(0), "RO", "只读拒替换");
        });
    });
}

/// 「不互相吞」卫（#47 A4 修复面）：预编辑期真实键分发（key_char
/// 直输路径）零插入——组合输入归输入法所有（直插会破坏 marked 区
/// 间与 buffer 的对应，提交时替换错段）；提交后直输路径恢复。
#[gpui::test]
fn ime_composition_guards_printable_keys(cx: &mut gpui::TestAppContext) {
    let (ed, window) = ime_window(cx, "AB\n");
    window.update(cx, |_host, window, cx| {
        ed.update(cx, |ed, cx| {
            ed.replace_and_mark_text_in_range(None, "ni", None, window, cx);
        });
        // 预编辑期：真实键分发（'h'，无修饰）——不得直插。
        ed.update(cx, |ed, cx| {
            ed.dispatch_key(&plain_key("h", Some("h")), window, cx);
            assert_eq!(
                ed.buffer_line(0),
                "niAB",
                "组合期 key_char 零直插（IME 所有）"
            );
            assert_eq!(
                ed.marked_text_range(window, cx),
                Some(0..2),
                "marked 区间不被键入破坏"
            );
        });
        // 提交后：同一键分发——直输路径恢复（正常键入生效）。
        ed.update(cx, |ed, cx| {
            ed.unmark_text(window, cx);
            ed.dispatch_key(&plain_key("h", Some("h")), window, cx);
            ed.replace_text_in_range(None, "h", window, cx);
            assert_eq!(
                ed.buffer_line(0),
                // 光标在预编辑段尾（replace_range 后 = (0,2)）——
                // 提交后直输在 "ni" 与 "AB" 之间。
                "nihAB",
                "提交后直输恢复"
            );
        });
    });
}
