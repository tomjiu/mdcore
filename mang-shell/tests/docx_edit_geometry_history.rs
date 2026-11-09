//! #629: actual Buffer -> generic DocxEditing -> real DOCX save/reopen.
//! No images or GUI are required to catch row ownership corruption.

use std::cell::RefCell;
use std::io::Read;
use std::rc::Rc;

use mang_shell::editor::buffer::{Buffer, Point};
use mang_shell::editor::docx_doc::editable_from_bytes;
use mang_shell::editor::docx_edit_bridge::{
    DocxBridge, DocxEditing, DocxPort, LineBlock, TableRef,
};
use mdc_office_io::docx::{DocxWriter, RunSpec};
use mdc_office_io::docx_edit::DocxEditor;

struct Case {
    source: Vec<u8>,
    buffer: Buffer,
    editing: Rc<RefCell<DocxEditing>>,
}

impl Case {
    fn new(source: Vec<u8>) -> Self {
        let (payload, port) = editable_from_bytes(&source);
        let mut bridge = DocxBridge::new(
            payload.blocks.clone().expect("fixture must be editable"),
            (0..port.paragraph_count())
                .map(|i| Some(port.paragraph_text(i)))
                .collect(),
        );
        for (p, text) in &payload.projections {
            bridge.note_projection(*p, text);
        }
        for (p, text) in &payload.fallbacks {
            bridge.note_fallback(*p, text);
        }
        bridge.note_tables(
            port.table_cells()
                .into_iter()
                .enumerate()
                .map(|(index, cells)| TableRef { index, cells })
                .collect(),
        );
        let editing = Rc::new(RefCell::new(DocxEditing::new(bridge, Box::new(port))));
        let mut buffer = Buffer::from_plain_text(&payload.text);
        buffer.set_edit_hook(Some(Box::new(editing.clone())));
        buffer.set_undo_hook(Some(Box::new(editing.clone())));
        Self {
            source,
            buffer,
            editing,
        }
    }

    fn row(&self, paragraph: usize) -> usize {
        self.editing
            .borrow()
            .bridge
            .rows()
            .iter()
            .position(|r| matches!(r, LineBlock::Para { paragraph: p, .. } if *p == paragraph))
            .expect("paragraph row")
    }

    fn flush(&mut self) {
        let lines: Vec<String> = (0..self.buffer.line_count())
            .map(|i| self.buffer.line(i))
            .collect();
        assert_eq!(
            self.editing.borrow().bridge.line_count(),
            lines.len(),
            "generic bridge must track actual Buffer rows"
        );
        self.editing.borrow_mut().flush(&lines);
    }

    fn save(&mut self) -> Vec<u8> {
        self.flush();
        self.editing.borrow().engine.save().expect("safe save")
    }

    fn assert_untouched(&self, saved: &[u8], paragraphs: &[usize]) {
        let before = DocxEditor::open(&self.source).unwrap();
        let after = DocxEditor::open(saved).unwrap();
        let before_xml = document_xml(&self.source);
        let after_xml = document_xml(saved);
        for &p in paragraphs {
            assert_eq!(before.paragraph_text(p), after.paragraph_text(p));
            let a = before.paragraphs()[p].span;
            let b = after.paragraphs()[p].span;
            assert_eq!(
                &before_xml[a.0..a.1],
                &after_xml[b.0..b.1],
                "untouched paragraph {p}: raw XML must remain byte-identical"
            );
        }
        let source_parts = zip_entries(&self.source);
        let saved_parts = zip_entries(saved);
        for (name, method, bytes) in source_parts {
            if name != "word/document.xml" {
                let matching = saved_parts.iter().find(|(n, _, _)| *n == name).unwrap();
                assert_eq!(matching.1, method);
                assert_eq!(matching.2, bytes, "non-target ZIP part {name}");
            }
        }
    }
}

fn three_paragraphs() -> Vec<u8> {
    let mut writer = DocxWriter::new();
    writer.paragraph_text("第一段");
    writer.paragraph(&[
        RunSpec::styled("第二", true, false),
        RunSpec::styled("段", false, true),
    ]);
    writer.paragraph_text("第三段");
    writer.save_to_vec().unwrap()
}

#[test]
fn prepend_two_lines_keeps_later_paragraphs_and_parts_untouched() {
    let mut case = Case::new(three_paragraphs());
    for text in ["新增一行\n", "再添一行\n"] {
        assert!(
            case.buffer
                .replace_range(Point::new(0, 0), Point::new(0, 0), text)
        );
        case.flush();
    }
    let saved = case.save();
    let after = DocxEditor::open(&saved).unwrap();
    assert_eq!(after.paragraph_text(0), "再添一行\n新增一行\n第一段");
    case.assert_untouched(&saved, &[1, 2]);
}

#[test]
fn insertion_at_later_paragraph_start_belongs_to_that_paragraph() {
    let mut case = Case::new(three_paragraphs());
    let row = case.row(1);
    assert!(
        case.buffer
            .replace_range(Point::new(row, 0), Point::new(row, 0), "前缀\n")
    );
    let saved = case.save();
    assert_eq!(
        DocxEditor::open(&saved).unwrap().paragraph_text(1),
        "前缀\n第二段"
    );
    case.assert_untouched(&saved, &[0, 2]);
}

#[test]
fn undo_redo_at_distinct_positions_restores_the_matching_row_ownership() {
    let mut case = Case::new(three_paragraphs());
    assert!(
        case.buffer
            .replace_range(Point::new(0, 0), Point::new(0, 0), "首插\n")
    );
    case.flush();
    let row = case.row(2);
    assert!(
        case.buffer
            .replace_range(Point::new(row, 0), Point::new(row, 0), "尾插\n")
    );
    case.flush();
    assert!(case.buffer.undo());
    let saved = case.save();
    assert_eq!(
        DocxEditor::open(&saved).unwrap().paragraph_text(0),
        "首插\n第一段"
    );
    case.assert_untouched(&saved, &[1, 2]);
    assert!(case.buffer.undo());
    let saved = case.save();
    case.assert_untouched(&saved, &[0, 1, 2]);
    assert!(case.buffer.redo());
    assert!(case.buffer.redo());
    let saved = case.save();
    let after = DocxEditor::open(&saved).unwrap();
    assert_eq!(after.paragraph_text(0), "首插\n第一段");
    assert_eq!(after.paragraph_text(2), "尾插\n第三段");
    case.assert_untouched(&saved, &[1]);
}

#[test]
fn batch_edits_and_one_step_undo_redo_keep_each_paragraph_identity() {
    let mut case = Case::new(three_paragraphs());
    assert_eq!(
        case.buffer.replace_batch(vec![
            (Point::new(0, 0), Point::new(0, 0), "首批\n".into()),
            (Point::new(2, 0), Point::new(2, 0), "尾批\n".into()),
        ]),
        2
    );
    let saved = case.save();
    let after = DocxEditor::open(&saved).unwrap();
    assert_eq!(after.paragraph_text(0), "首批\n第一段");
    assert_eq!(after.paragraph_text(2), "尾批\n第三段");
    case.assert_untouched(&saved, &[1]);
    assert!(case.buffer.undo());
    let saved = case.save();
    case.assert_untouched(&saved, &[0, 1, 2]);
    assert!(case.buffer.redo());
    let saved = case.save();
    assert_eq!(
        DocxEditor::open(&saved).unwrap().paragraph_text(2),
        "尾批\n第三段"
    );
}

#[test]
fn replacing_soft_breaks_uses_the_old_removed_range() {
    let mut case = Case::new(three_paragraphs());
    assert!(
        case.buffer
            .replace_range(Point::new(0, 1), Point::new(0, 1), "甲\n乙\n丙")
    );
    case.flush();
    assert!(
        case.buffer
            .replace_range(Point::new(0, 1), Point::new(2, 1), "替换")
    );
    let saved = case.save();
    assert_eq!(
        DocxEditor::open(&saved).unwrap().paragraph_text(0),
        "第替换一段"
    );
    case.assert_untouched(&saved, &[1, 2]);
    assert!(case.buffer.undo());
    let saved = case.save();
    assert_eq!(
        DocxEditor::open(&saved).unwrap().paragraph_text(0),
        "第甲\n乙\n丙一段"
    );
    case.assert_untouched(&saved, &[1, 2]);
}

#[test]
fn derived_edits_still_notify_generic_mapping_and_have_undo_identity() {
    let mut case = Case::new(three_paragraphs());
    assert!(
        case.buffer
            .replace_range_derived(Point::new(0, 0), Point::new(0, 0), "派生\n")
    );
    let saved = case.save();
    assert_eq!(
        DocxEditor::open(&saved).unwrap().paragraph_text(0),
        "派生\n第一段"
    );
    case.assert_untouched(&saved, &[1, 2]);
    assert!(case.buffer.undo());
    let saved = case.save();
    case.assert_untouched(&saved, &[0, 1, 2]);
}

#[test]
fn protected_field_inside_a_range_cannot_be_erased_or_bypassed_by_batch() {
    let mut writer = DocxWriter::new();
    writer.paragraph_text("前文");
    writer.paragraph(&[RunSpec::field_page()]);
    writer.paragraph_text("后文");
    let source = writer.save_to_vec().unwrap();
    let mut case = Case::new(source.clone());
    let end = case.row(2);
    assert!(
        !case
            .buffer
            .replace_range(Point::new(0, 1), Point::new(end, 1), "删域")
    );
    assert_eq!(
        case.buffer
            .replace_batch(vec![(Point::new(0, 1), Point::new(end, 1), "删域".into()),]),
        0
    );
    let saved = case.save();
    case.assert_untouched(&saved, &[0, 1, 2]);
}

#[test]
fn table_cell_edits_stay_in_the_cell_and_cannot_insert_paragraph_rows() {
    let mut writer = DocxWriter::new();
    writer.paragraph_text("表前");
    writer.table(&[vec!["单元甲".into(), "单元乙".into()]]);
    writer.paragraph_text("表后");
    let mut case = Case::new(writer.save_to_vec().unwrap());
    let cell = case
        .editing
        .borrow()
        .bridge
        .rows()
        .iter()
        .position(|r| matches!(r, LineBlock::Cell { col: 0, .. }))
        .unwrap();
    assert!(
        case.buffer
            .replace_range(Point::new(cell, 0), Point::new(cell, 3), "新单元")
    );
    assert!(
        !case
            .buffer
            .replace_range(Point::new(cell, 1), Point::new(cell, 1), "\n")
    );
    let saved = case.save();
    let after = DocxEditor::open(&saved).unwrap();
    assert_eq!(after.paragraph_text(1), "新单元");
    case.assert_untouched(&saved, &[0, 2, 3]);
    assert!(case.buffer.undo());
    let saved = case.save();
    case.assert_untouched(&saved, &[0, 1, 2, 3]);
}

#[test]
fn inconsistent_mapping_refuses_even_direct_engine_save_and_keeps_dirty_text() {
    let mut case = Case::new(three_paragraphs());
    assert!(
        case.buffer
            .replace_range(Point::new(0, 0), Point::new(0, 0), "尚未保存")
    );
    case.flush();
    case.editing.borrow_mut().bridge.rows_mut().pop();
    let lines: Vec<_> = (0..case.buffer.line_count())
        .map(|i| case.buffer.line(i))
        .collect();
    let plan = case.editing.borrow_mut().flush(&lines);
    assert!(plan.sets.is_empty(), "不确定映射不得继续写其他段");
    let error = case.editing.borrow().engine.save().unwrap_err();
    assert!(error.contains("映射"), "直接 save 也必须显式拒绝：{error}");
    assert!(case.buffer.is_modified(), "失败不能清除 buffer 脏态");
    assert!(
        case.buffer.line(0).starts_with("尚未保存"),
        "用户编辑仍留在内存"
    );
}

// Independent ZIP entry inspection, matching the existing DOCX fidelity tests.
fn zip_entries(bytes: &[u8]) -> Vec<(String, u16, Vec<u8>)> {
    let eocd = (0..=bytes.len().saturating_sub(22))
        .rev()
        .find(|&i| bytes[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
        .unwrap();
    let u16_at = |i| u16::from_le_bytes(bytes[i..i + 2].try_into().unwrap()) as usize;
    let u32_at = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
    let mut p = u32_at(eocd + 16);
    let mut out = Vec::new();
    for _ in 0..u16_at(eocd + 10) {
        assert_eq!(&bytes[p..p + 4], b"PK\x01\x02");
        let method = u16_at(p + 10) as u16;
        let size = u32_at(p + 20);
        let (name_len, extra_len, comment_len) = (u16_at(p + 28), u16_at(p + 30), u16_at(p + 32));
        let local = u32_at(p + 42);
        let name = String::from_utf8(bytes[p + 46..p + 46 + name_len].to_vec()).unwrap();
        let start = local + 30 + u16_at(local + 26) + u16_at(local + 28);
        out.push((name, method, bytes[start..start + size].to_vec()));
        p += 46 + name_len + extra_len + comment_len;
    }
    out
}

fn document_xml(bytes: &[u8]) -> Vec<u8> {
    let (_, method, compressed) = zip_entries(bytes)
        .into_iter()
        .find(|(n, _, _)| n == "word/document.xml")
        .unwrap();
    if method == 0 {
        compressed
    } else {
        assert_eq!(method, 8);
        let mut out = Vec::new();
        flate2::read::DeflateDecoder::new(compressed.as_slice())
            .read_to_end(&mut out)
            .unwrap();
        out
    }
}
