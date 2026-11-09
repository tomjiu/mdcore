use super::*;
use mdc_office_io::docx::{DocxWriter, RunSpec};
use mdc_office_io::docx_edit::DocxEditor;

struct Fixture(PathBuf);

impl Fixture {
    fn new(tag: &str, bytes: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!(
            "mdcore-629-ui-{tag}-{}-{}.docx",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, bytes).unwrap();
        Self(path)
    }

    fn open(&self, cx: &mut gpui::TestAppContext) -> gpui::Entity<Editor> {
        let item = docx_doc::load_editable(&self.0).unwrap();
        cx.update(|cx| cx.new(|cx| Editor::new_docx_editable(self.0.clone(), item, cx)))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn setup_find(editor: &mut Editor, query: &str, replacement: &str) {
    let mut find = FindState::default();
    find.open_replace(&editor.buffer);
    find.set_query(query, &editor.buffer);
    find.set_focus_replacement(true);
    find.handle_key(
        &editor.buffer,
        &gpui::Keystroke {
            modifiers: Default::default(),
            key: "x".into(),
            key_char: Some(replacement.into()),
        },
    );
    assert_eq!(find.replacement(), replacement);
    editor.find = Some(find);
}

#[gpui::test]
fn find_all_with_newlines_saves_the_right_paragraphs_and_undo_preserves_xml(
    cx: &mut gpui::TestAppContext,
) {
    let mut writer = DocxWriter::new();
    writer.paragraph_text("目标一");
    writer.paragraph_text("未改二");
    writer.paragraph_text("目标三");
    let source = writer.save_to_vec().unwrap();
    let fixture = Fixture::new("find", &source);
    let editor = fixture.open(cx);
    cx.update(|cx| {
        editor.update(cx, |editor, _| {
            setup_find(editor, "目标", "新增\n目标");
            editor.replace_find(true);
            assert_eq!(editor.save(), SaveOutcome::Saved);
            let saved = DocxEditor::open(&std::fs::read(&fixture.0).unwrap()).unwrap();
            assert_eq!(saved.paragraph_text(0), "新增\n目标一");
            assert_eq!(saved.paragraph_text(1), "未改二");
            assert_eq!(saved.paragraph_text(2), "新增\n目标三");
            assert!(editor.undo_edit());
            assert_eq!(editor.save(), SaveOutcome::Saved);
        })
    });
    assert_eq!(std::fs::read(&fixture.0).unwrap(), source);
}

#[gpui::test]
fn find_cannot_replace_a_locked_page_field(cx: &mut gpui::TestAppContext) {
    let mut writer = DocxWriter::new();
    writer.paragraph_text("前文");
    writer.paragraph(&[RunSpec::field_page()]);
    writer.paragraph_text("后文");
    let source = writer.save_to_vec().unwrap();
    let fixture = Fixture::new("field", &source);
    let editor = fixture.open(cx);
    cx.update(|cx| {
        editor.update(cx, |editor, _| {
            let original = editor.buffer.to_plain_text();
            let locked = editor
                .docx_edit
                .as_ref()
                .unwrap()
                .borrow()
                .bridge
                .rows()
                .iter()
                .position(|row| matches!(row, LineBlock::Locked(_)))
                .unwrap();
            let query = editor.buffer.line(locked);
            assert!(!query.is_empty());
            for all in [false, true] {
                setup_find(editor, &query, "破坏");
                editor.replace_find(all);
                assert_eq!(editor.buffer.to_plain_text(), original);
                assert!(!editor.is_modified());
            }
        })
    });
    assert_eq!(std::fs::read(&fixture.0).unwrap(), source);
}

#[gpui::test]
fn indent_undo_keeps_the_previous_edit_and_saves_without_mapping_failure(
    cx: &mut gpui::TestAppContext,
) {
    let mut writer = DocxWriter::new();
    writer.paragraph_text("第一段");
    writer.paragraph_text("第二段");
    let fixture = Fixture::new("indent", &writer.save_to_vec().unwrap());
    let editor = fixture.open(cx);
    cx.update(|cx| {
        editor.update(cx, |editor, _| {
            assert!(
                editor
                    .buffer
                    .replace_range(Point::new(0, 0), Point::new(0, 0), "已有编辑")
            );
            let before_indent = editor.buffer.to_plain_text();
            editor
                .buffer
                .set_selection(Point::new(0, 0), Point::new(1, editor.buffer.line_len(1)));
            editor.sync_cursors_from_buffer();
            editor.indent_key(false);
            assert_ne!(editor.buffer.to_plain_text(), before_indent);
            assert!(editor.undo_edit());
            assert_eq!(editor.buffer.to_plain_text(), before_indent);
            assert_eq!(editor.save(), SaveOutcome::Saved);
            let saved = DocxEditor::open(&std::fs::read(&fixture.0).unwrap()).unwrap();
            assert_eq!(saved.paragraph_text(0), "已有编辑第一段");
            assert_eq!(saved.paragraph_text(1), "第二段");
        })
    });
}
