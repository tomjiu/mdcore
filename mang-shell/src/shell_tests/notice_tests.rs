//! #47 巨石拆解批①：原 shell.rs 内嵌 `notice_tests` 纯迁出（零行为变化——
//! 字节原样，挂载点 #[cfg(test)] + #[path] 在 shell.rs）。

use super::NoticeBoard;

#[test]
fn show_returns_incrementing_generations() {
    let mut board = NoticeBoard::default();
    assert_eq!(board.message(), None, "初始无消息");
    let g1 = board.show("第一条");
    assert_eq!(g1, 1);
    assert_eq!(board.message(), Some("第一条"));
    let g2 = board.show("第二条");
    assert_eq!(g2, 2, "代际递增");
    assert_eq!(board.message(), Some("第二条"), "新消息覆盖旧消息");
}

#[test]
fn dismiss_only_matches_current_generation() {
    let mut board = NoticeBoard::default();
    let g1 = board.show("第一条");
    let g2 = board.show("第二条");
    // 旧代际定时器到达——不得误清新消息。
    assert!(!board.dismiss(g1), "过时代际不清除");
    assert_eq!(board.message(), Some("第二条"));
    // 当前代际定时器到达——正常清除。
    assert!(board.dismiss(g2));
    assert_eq!(board.message(), None);
    // 空板再 dismiss 恒 false。
    assert!(!board.dismiss(g2));
}

#[test]
fn clear_wipes_unconditionally() {
    let mut board = NoticeBoard::default();
    let g = board.show("消息");
    board.clear();
    assert_eq!(board.message(), None);
    // clear 后旧代际定时到达仍无效（无消息可清）。
    assert!(!board.dismiss(g));
}

#[test]
fn generation_survives_clear() {
    let mut board = NoticeBoard::default();
    let _g1 = board.show("第一条");
    board.clear();
    let g2 = board.show("第二条");
    // clear 不重置代际——防 clear 前后同号定时器误配。
    assert_eq!(g2, 2);
}
