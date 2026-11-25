//! #47 巨石拆解批②：原 editor/mod.rs 内嵌 `completion_interaction_tests` 纯迁出（零行为变化——
//! 字节原样剥壳，挂载 #[cfg(test)] + #[path] 在 mod.rs）。

use super::*;
use gpui::{Entity, PlatformInput};

/// 键事件构造（ime_composition_tests::plain_key 同款——两模块
/// 私有互不可见，本地副本）。
fn plain_key(key: &str) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: gpui::Keystroke {
            modifiers: gpui::Modifiers::default(),
            key: key.into(),
            key_char: None,
        },
        is_held: false,
        prefer_character_input: false,
    }
}

/// 键路由宿主（InputRouteHost 同款本地副本——焦点 + on_key_down
/// → dispatch_key，全窗口真实键派发面）。
struct CompletionKeyHost {
    ed: Entity<Editor>,
}

impl Render for CompletionKeyHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ed = self.ed.clone();
        gpui::div()
            .track_focus(&ed.read(cx).focus)
            .on_key_down(move |event, window, cx| {
                ed.update(cx, |ed, cx| ed.dispatch_key(event, window, cx));
            })
    }
}

/// 触发面文档（两行词 + 光标行）：insert "st" 于行 2 → 前缀 "st"，
/// 文档词候选 = structure / structural（"st" 自身排除）。
const TRIGGER_DOC: &str = "structure\nstructural\n";

/// 触发：光标行打入 ≥ 阈值前缀 → 弹窗开（文档词 + 关键词池过滤）。
#[gpui::test]
fn completion_triggers_on_two_char_prefix(cx: &mut gpui::TestAppContext) {
    let ed = cx.update(|cx| cx.new(|cx| Editor::new(None, TRIGGER_DOC.into(), cx)));
    // 行 2 打入 "st"（用户键入同链——insert_text_at → after_edit）。
    cx.update(|cx| ed.update(cx, |ed, cx| ed.insert_text_at(2, 0, "st", cx)));
    let state = ed.read_with(cx, |ed, _| ed.completion.clone());
    let Some(c) = state else {
        panic!("G3 先红：前缀 st 应触发补全弹窗（现实现无此面——None）");
    };
    assert_eq!(c.prefix, "st", "触发前缀 = 光标前词尾段");
    assert_eq!(c.prefix_start, Point::new(2, 0), "前缀锚点 = 行内词首列");
    assert!(
        c.items.iter().any(|w| w == "structure"),
        "文档词 structure 在候选：{:?}",
        c.items
    );
    assert!(
        c.items.iter().any(|w| w == "structural"),
        "文档词 structural 在候选：{:?}",
        c.items
    );
    assert_eq!(c.selected, 0, "触发时选中复位首行");
    assert!(c.items.len() <= completion::MAX_ITEMS, "候选截断上限");
}

/// 阈值下不触发：1 字符前缀 = 噪声源，不开。
#[gpui::test]
fn completion_no_trigger_below_threshold(cx: &mut gpui::TestAppContext) {
    let ed = cx.update(|cx| cx.new(|cx| Editor::new(None, TRIGGER_DOC.into(), cx)));
    cx.update(|cx| ed.update(cx, |ed, cx| ed.insert_text_at(2, 0, "s", cx)));
    assert!(
        ed.read_with(cx, |ed, _| ed.completion.is_none()),
        "单字符前缀不达阈值（MIN_PREFIX_CHARS=2）——不弹"
    );
}

/// 接受：前缀段整体替换为完整词（用户编辑同链——undo 一键复原），
/// 接受后不立即重开（hold）。
#[gpui::test]
fn completion_accept_edits_via_user_chain(cx: &mut gpui::TestAppContext) {
    let ed = cx.update(|cx| cx.new(|cx| Editor::new(None, TRIGGER_DOC.into(), cx)));
    cx.update(|cx| ed.update(cx, |ed, cx| ed.insert_text_at(2, 0, "st", cx)));
    // 选中文档词 structure（默认 selected=0）。
    let accepted = cx.update(|cx| ed.update(cx, |ed, cx| ed.accept_completion(cx)));
    assert!(accepted, "接受成功（先红：空壳恒 false）");
    assert_eq!(
        ed.read_with(cx, |ed, _| ed.buffer.line(2)),
        "structure",
        "前缀段替换为完整词"
    );
    assert_eq!(
        ed.read_with(cx, |ed, _| ed.buffer.cursor()),
        Point::new(2, 9),
        "光标落词尾（英文 9 字符）"
    );
    assert!(
        ed.read_with(cx, |ed, _| ed.completion.is_none()),
        "接受后弹窗关 + hold 抑制重开（structure 的候选 structural 不再弹出）"
    );
    // undo 一键复原（replace_range 单事务）。
    let undone = cx.update(|cx| ed.update(cx, |ed, _| ed.undo_edit()));
    assert!(undone, "接受进撤销栈");
    assert_eq!(
        ed.read_with(cx, |ed, _| ed.buffer.line(2)),
        "st",
        "undo 复原到触发前缀态"
    );
}

/// 只读态永不触发（insert 拒 → completion 恒 None）。
#[gpui::test]
fn completion_readonly_never_opens(cx: &mut gpui::TestAppContext) {
    let ed = cx.update(|cx| cx.new(|cx| Editor::new_readonly(None, TRIGGER_DOC.into(), cx)));
    // 只读门禁拦下插入（insert_text_at None）——补全面无从触发。
    let inserted = cx.update(|cx| ed.update(cx, |ed, cx| ed.insert_text_at(2, 0, "st", cx)));
    assert!(inserted.is_none(), "只读插入被拒（前置口径）");
    assert!(
        ed.read_with(cx, |ed, _| ed.completion.is_none()),
        "只读态无补全面"
    );
}

/// 关门守卫（钉测——实现后守住不回归）：选区中 / IME 预编辑中，
/// 即使前缀达标也不开。
#[gpui::test]
fn completion_guards_selection_and_ime(cx: &mut gpui::TestAppContext) {
    let ed = cx.update(|cx| cx.new(|cx| Editor::new(None, TRIGGER_DOC.into(), cx)));
    // ① 选区中：refresh 直接评估（守卫面单测——不经编辑链）。
    cx.update(|cx| {
        ed.update(cx, |ed, _| {
            ed.buffer.set_selection(Point::new(0, 0), Point::new(0, 3));
            ed.refresh_completion();
        })
    });
    assert!(
        ed.read_with(cx, |ed, _| ed.completion.is_none()),
        "选区中不开（替换前缀与选区语义冲突）"
    );
    // ② IME 预编辑中：先清选区（隔离变量——只测 IME 守卫面），
    // 候选弹窗与组合窗打架——不开。
    cx.update(|cx| {
        ed.update(cx, |ed, _| {
            ed.buffer.set_cursor(Point::new(0, 0));
            ed.marked = Some(0..1);
            ed.refresh_completion();
        })
    });
    assert!(
        ed.read_with(cx, |ed, _| ed.completion.is_none()),
        "IME 预编辑中不开"
    );
    // ③ 预编辑结束 + 前缀达标 → 正常开（守卫只挡预编辑期；
    // replace_range 后 buffer 光标落插入尾部——与键入链同口径）。
    cx.update(|cx| {
        ed.update(cx, |ed, _| {
            ed.marked = None;
            ed.buffer
                .replace_range(Point::new(2, 0), Point::new(2, 0), "st");
            ed.sync_cursors_from_buffer();
            ed.refresh_completion();
        })
    });
    assert!(
        ed.read_with(cx, |ed, _| ed.completion.is_some()),
        "预编辑结束 + 前缀 st → 正常触发"
    );
}

/// 键拦截（真窗口全键派发面）：↑↓ 导航、Enter 接受、Esc 关、
/// 光标移动键关且不吞（照常移动）。
#[gpui::test]
fn completion_key_interception_full_window(cx: &mut gpui::TestAppContext) {
    let ed = cx.update(|cx| cx.new(|cx| Editor::new(None, TRIGGER_DOC.into(), cx)));
    let ed_host = ed.clone();
    let window = cx.add_window(move |window, cx| {
        ed_host.read(cx).focus.clone().focus(window, cx);
        CompletionKeyHost { ed: ed_host }
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
        // 触发（键入面）。
        ed.update(cx, |ed, cx| ed.insert_text_at(2, 0, "st", cx));
        let items = ed
            .read(cx)
            .completion
            .as_ref()
            .map(|c| c.items.clone())
            .expect("先红：键拦截面以触发为前置");
        assert!(items.contains(&"structure".to_string()));

        // ↓×2 ↑：导航到 selected=1（structural）。
        window.dispatch_event(PlatformInput::KeyDown(plain_key("down")), cx);
        window.dispatch_event(PlatformInput::KeyDown(plain_key("down")), cx);
        window.dispatch_event(PlatformInput::KeyDown(plain_key("up")), cx);
        assert_eq!(
            ed.read(cx).completion.as_ref().unwrap().selected,
            1,
            "↑↓ 归弹窗导航（光标不移动）"
        );
        assert_eq!(
            ed.read(cx).completion.as_ref().unwrap().items[1],
            "structural"
        );
        // 光标未动（↑↓ 被吞）。
        assert_eq!(ed.read(cx).buffer.cursor().line, 2);

        // Enter：接受 selected=1 → structural。
        window.dispatch_event(PlatformInput::KeyDown(plain_key("enter")), cx);
        assert_eq!(ed.read(cx).buffer.line(2), "structural", "Enter 接受候选");
        assert!(ed.read(cx).completion.is_none(), "接受后弹窗关");
        // Enter 不落正文换行（被吞）——行数不变（"structure\n
        // structural\n" = 3 行，行 2 被补全改写）。
        assert_eq!(ed.read(cx).buffer.line_count(), 3);

        // Esc 阶段：重新触发（行 2 续打）后 Esc 只关不落编辑。
        ed.update(cx, |ed, cx| ed.insert_text_at(2, 9, " ", cx));
        ed.update(cx, |ed, cx| ed.insert_text_at(2, 10, "st", cx));
        assert!(ed.read(cx).completion.is_some(), "重新触发");
        window.dispatch_event(PlatformInput::KeyDown(plain_key("escape")), cx);
        assert!(ed.read(cx).completion.is_none(), "Esc 关弹窗");

        // 光标移动键：关 + 不吞（照常移动）。
        ed.update(cx, |ed, cx| ed.insert_text_at(2, 12, "r", cx));
        assert!(ed.read(cx).completion.is_some(), "三打触发（str）");
        let col_before = ed.read(cx).buffer.cursor().col;
        window.dispatch_event(PlatformInput::KeyDown(plain_key("left")), cx);
        assert!(ed.read(cx).completion.is_none(), "left 关弹窗");
        assert_eq!(
            ed.read(cx).buffer.cursor().col,
            col_before - 1,
            "left 未被吞——光标照常左移"
        );
    });
}
