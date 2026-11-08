use mdc_office_io::docx::{DocxError, DocxWriter, RunSpec};
use mdc_office_io::docx_revision::RevisionMark;

#[test]
fn insert_empty_character_style_must_reject_v12() {
    let mut writer = DocxWriter::new();
    writer.insert_runs_revised(
        &[RunSpec::char_styled("text", "")],
        &RevisionMark::default(),
    );
    assert!(
        matches!(writer.save_to_vec(), Err(DocxError::InvalidInput(m)) if m.starts_with("V12")),
        "empty style must reject through the same V12 policy as paragraph()"
    );
}

#[test]
fn delete_empty_character_style_must_reject_v12() {
    let mut writer = DocxWriter::new();
    writer.delete_runs_revised(
        &[RunSpec::char_styled("text", "")],
        &RevisionMark::default(),
    );
    assert!(
        matches!(writer.save_to_vec(), Err(DocxError::InvalidInput(m)) if m.starts_with("V12"))
    );
}

#[test]
fn insert_undefined_character_style_must_reject_v12() {
    let mut writer = DocxWriter::new();
    writer.insert_runs_revised(
        &[RunSpec::char_styled("text", "NoSuchStyle")],
        &RevisionMark::default(),
    );
    assert!(
        matches!(writer.save_to_vec(), Err(DocxError::InvalidInput(m)) if m.starts_with("V12")),
        "undefined style must not serialize a dangling rStyle"
    );
}

#[test]
fn delete_undefined_character_style_must_reject_v12() {
    let mut writer = DocxWriter::new();
    writer.delete_runs_revised(
        &[RunSpec::char_styled("text", "NoSuchStyle")],
        &RevisionMark::default(),
    );
    assert!(
        matches!(writer.save_to_vec(), Err(DocxError::InvalidInput(m)) if m.starts_with("V12"))
    );
}

#[test]
fn preset_styles_and_zero_runs_remain_legal() {
    let mut writer = DocxWriter::new();
    writer.insert_runs_revised(
        &[RunSpec::char_styled("text", "Strong")],
        &RevisionMark::default(),
    );
    writer.delete_runs_revised(
        &[RunSpec::char_styled("text", "Emphasis")],
        &RevisionMark::default(),
    );
    writer.insert_runs_revised(&[], &RevisionMark::default());
    writer.delete_runs_revised(&[], &RevisionMark::default());
    assert!(writer.save_to_vec().is_ok());
}

#[test]
fn ordinary_paragraph_already_rejects_the_same_undefined_style() {
    let mut writer = DocxWriter::new();
    writer.paragraph(&[RunSpec::char_styled("text", "NoSuchStyle")]);
    assert!(
        matches!(writer.save_to_vec(), Err(DocxError::InvalidInput(m)) if m.starts_with("V12"))
    );
}
