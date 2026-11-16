//! Independent Lead oracle through public multicursor APIs, not the implementation mapper.
use mang_shell::editor::buffer::{Buffer, Point};
use mang_shell::editor::multi_cursor::Cursors;

fn point_at(text: &str, offset: usize) -> Point {
    let mut line = 0;
    let mut col = 0;
    for ch in text.chars().take(offset) {
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    Point::new(line, col)
}
fn expected_insert(text: &str, offsets: &[usize], insert: &str) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    for &offset in offsets.iter().rev() {
        chars.splice(offset..offset, insert.chars());
    }
    chars.into_iter().collect()
}
#[test]
fn lead_review_exhaustive_two_cursor_insert_undo_redo_oracle() {
    for original in ["abc", "a😀中", "a\nb", "a\r\nb\r", "\n\n", "# H\n\nZ\n"] {
        let count = original.chars().count();
        for left in 0..count {
            for right in left + 1..=count {
                for insert in ["X", "中😀", "\n", "\n\n"] {
                    let mut buffer = Buffer::from_plain_text(original);
                    buffer.set_coalesce_ms(0);
                    let mut cursors = Cursors::single(point_at(original, left));
                    assert!(cursors.push(point_at(original, right), point_at(original, right)));
                    let expected = expected_insert(original, &[left, right], insert);
                    cursors.insert_all(&mut buffer, insert);
                    assert_eq!(
                        buffer.to_plain_text(),
                        expected,
                        "insert {original:?} {left},{right} {insert:?}"
                    );
                    let n = insert.chars().count();
                    let expected_cursors = [
                        point_at(&expected, left + n),
                        point_at(&expected, right + 2 * n),
                    ];
                    let actual: Vec<Point> =
                        cursors.entries().iter().map(|(_, head)| *head).collect();
                    assert_eq!(
                        actual, expected_cursors,
                        "tails {original:?} {left},{right} {insert:?}"
                    );
                    assert!(buffer.undo());
                    assert_eq!(
                        buffer.to_plain_text(),
                        original,
                        "undo {original:?} {left},{right} {insert:?}"
                    );
                    assert!(!buffer.undo(), "gesture left extra history");
                    assert!(buffer.redo());
                    assert_eq!(
                        buffer.to_plain_text(),
                        expected,
                        "redo {original:?} {left},{right} {insert:?}"
                    );
                    assert!(!buffer.redo());
                }
            }
        }
    }
}
#[test]
fn lead_review_consecutive_source_only_gestures_restore_saved_identity() {
    let mut buffer = Buffer::from_plain_text("abc");
    buffer.set_coalesce_ms(0);
    let mut snapshots = vec![buffer.to_plain_text()];
    for _ in 0..4 {
        let eof = Point::new(
            buffer.line_count() - 1,
            buffer.line_len(buffer.line_count() - 1),
        );
        assert_eq!(buffer.replace_batch(vec![(eof, eof, "\n".into())]), 1);
        snapshots.push(buffer.to_plain_text());
    }
    buffer.mark_saved();
    assert!(!buffer.is_modified());
    for i in (0..4).rev() {
        assert!(buffer.undo());
        assert_eq!(buffer.to_plain_text(), snapshots[i]);
        assert!(buffer.is_modified());
    }
    for expected in snapshots.iter().skip(1) {
        assert!(buffer.redo());
        assert_eq!(&buffer.to_plain_text(), expected);
    }
    assert!(
        !buffer.is_modified(),
        "redo to actual saved identity must be clean"
    );
}
#[test]
fn lead_review_exhaustive_two_cursor_delete_oracle() {
    for original in ["abc", "a😀中", "a\nb", "a\r\nb\r", "\n\n", "# H\n\nZ\n"] {
        let count = original.chars().count();
        for left in 0..count {
            for right in left + 1..=count {
                for backwards in [false, true] {
                    let mut buffer = Buffer::from_plain_text(original);
                    buffer.set_coalesce_ms(0);
                    let mut cursors = Cursors::single(point_at(original, left));
                    assert!(cursors.push(point_at(original, right), point_at(original, right)));
                    let mut removed: Vec<usize> = [left, right]
                        .into_iter()
                        .filter_map(|offset| {
                            if backwards {
                                offset.checked_sub(1)
                            } else if offset < count {
                                Some(offset)
                            } else {
                                None
                            }
                        })
                        .collect();
                    removed.sort_unstable();
                    removed.dedup();
                    let expected: String = original
                        .chars()
                        .enumerate()
                        .filter_map(|(i, ch)| (!removed.contains(&i)).then_some(ch))
                        .collect();
                    if backwards {
                        cursors.backspace_all(&mut buffer);
                    } else {
                        cursors.delete_all(&mut buffer);
                    }
                    assert_eq!(
                        buffer.to_plain_text(),
                        expected,
                        "delete {original:?} {left},{right} back={backwards}"
                    );
                    let mut tails: Vec<Point> = [left, right]
                        .into_iter()
                        .map(|old| {
                            point_at(
                                &expected,
                                old - removed.iter().filter(|&&del| del < old).count(),
                            )
                        })
                        .collect();
                    tails.sort_unstable();
                    tails.dedup();
                    let actual: Vec<Point> =
                        cursors.entries().iter().map(|(_, head)| *head).collect();
                    assert_eq!(
                        actual, tails,
                        "delete tails {original:?} {left},{right} back={backwards}"
                    );
                    assert!(buffer.undo());
                    assert_eq!(buffer.to_plain_text(), original, "delete undo exact source");
                    assert!(!buffer.undo());
                    assert!(buffer.redo());
                    assert_eq!(buffer.to_plain_text(), expected, "delete redo exact result");
                    assert!(!buffer.redo());
                }
            }
        }
    }
}
