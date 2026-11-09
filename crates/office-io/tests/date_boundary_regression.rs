use mdc_office_io::docx::{read_docx_from_bytes_with_options, DocxWriter, RunSpec};
use mdc_office_io::docx_revision::RevisionMark;
use mdc_office_io::io::ReadOptions;
use std::io::Read;

fn emit(writer: &mut DocxWriter, face: usize, date: &str) {
    let mark = RevisionMark::new("author", date);
    match face {
        0 => writer.insert_run_revised("text", &mark),
        1 => writer.delete_run_revised("text", &mark),
        2 => writer.insert_runs_revised(&[RunSpec::text("text")], &mark),
        3 => writer.delete_runs_revised(&[RunSpec::text("text")], &mark),
        _ => unreachable!(),
    }
}

#[test]
fn shared_parser_must_reject_invalid_calendar_and_xsd_metadata() {
    for date in [
        "2025-02-29T00:00:00Z",
        "1900-02-29T00:00:00Z",
        "2024-02-30T00:00:00Z",
        "2024-04-31T00:00:00Z",
        "0000-01-01T00:00:00Z",
        "2026-01-01T00:00:60Z",
        "2026-01-01T00:00:00+14:01",
        "2026-01-01T00:00:00-14:01",
        "2026-01-01T00:00:00+15:00",
        "2026-01-01T00:00:00-23:59",
        "2026-01-01t00:00:00Z",
        "2026-01-01T00:00:00z",
    ] {
        for face in 0..4 {
            let mut writer = DocxWriter::new();
            emit(&mut writer, face, date);
            assert!(
                writer.save_to_vec().is_err(),
                "face={face} must reject {date}"
            );
        }
    }
}

#[test]
fn all_four_faces_must_reject_attribute_injection_and_controls() {
    for date in [
        "2026-01-01T00:00:00Z\" injected=\"yes",
        "2026-01-01T00:00:00Z<x/>",
        "2026-01-01T00:00:00Z&bad;",
        "2026-01-01T00:00:00\nZ",
        "2026-01-01T00:00:00\0Z",
        "2026-01-01T00:00:00Z'",
        "",
    ] {
        for face in 0..4 {
            let mut writer = DocxWriter::new();
            emit(&mut writer, face, date);
            assert!(
                writer.save_to_vec().is_err(),
                "face={face} must reject {date:?}"
            );
        }
    }
}

#[test]
fn valid_gregorian_and_timezone_boundaries_still_save_on_all_faces() {
    for date in [
        "2024-02-29T23:59:59Z",
        "2000-02-29T00:00:00Z",
        "1900-02-28T00:00:00Z",
        "0001-01-01T00:00:00Z",
        "9999-12-31T23:59:59.123456789Z",
        "2026-11-08T00:00:00+14:00",
        "2026-11-08T00:00:00-14:00",
        "2026-11-08T00:00:00+13:59",
        "2026-11-08T00:00:00-13:59",
        "2026-11-08T00:00:00",
        "2026-11-08T00:00:00.0000000001",
    ] {
        for face in 0..4 {
            let mut writer = DocxWriter::new();
            emit(&mut writer, face, date);
            let bytes = writer.save_to_vec().expect("valid date must save");
            let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
            let mut xml = String::new();
            archive
                .by_name("word/document.xml")
                .unwrap()
                .read_to_string(&mut xml)
                .unwrap();
            let expected = format!("w:date=\"{date}\"");
            assert_eq!(
                xml.matches(expected.as_str()).count(),
                1,
                "face={face} must preserve the exact date attribute"
            );
            read_docx_from_bytes_with_options(&bytes, &ReadOptions { compat: true })
                .expect("valid date output must read back");
        }
    }
}

#[test]
fn rich_date_errors_must_be_sticky_and_preserve_first_endpoint_context() {
    for face in 2..4 {
        let mut writer = DocxWriter::new();
        emit(&mut writer, face, "2026-01-01T00:00:00Z\" injected=\"first");
        emit(&mut writer, 1, "2026-13-01T00:00:00Z");
        emit(&mut writer, 0, "2026-01-01T00:00:00Z");
        let error = writer
            .save_to_vec()
            .expect_err("first date error is sticky")
            .to_string();
        let name = if face == 2 {
            "insert_runs_revised"
        } else {
            "delete_runs_revised"
        };
        assert!(
            error.contains(name) && error.contains("first"),
            "first rich error lost: {error}"
        );
    }
}

#[test]
fn invalid_dates_must_not_create_path_artifacts() {
    for face in 0..4 {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "revision-date-593-{}-{nonce}-{face}",
            std::process::id()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("output.docx");
        let mut writer = DocxWriter::new();
        emit(&mut writer, face, "2025-02-29T00:00:00Z");
        let result = writer.save_to_path(path.to_str().unwrap());
        let artifact = path.exists();
        if artifact {
            std::fs::remove_file(&path).unwrap();
        }
        std::fs::remove_dir(&dir).unwrap();
        assert!(
            result.is_err() && !artifact,
            "face={face} accepted invalid calendar and wrote an artifact"
        );
    }
}

#[test]
fn unsupported_payload_keeps_v13_priority_when_date_is_also_invalid() {
    for face in 2..4 {
        let mut writer = DocxWriter::new();
        let runs = [RunSpec::link("text", "https://example.invalid")];
        let mark = RevisionMark::new("author", "invalid-date");
        if face == 2 {
            writer.insert_runs_revised(&runs, &mark);
        } else {
            writer.delete_runs_revised(&runs, &mark);
        }
        let error = writer
            .save_to_vec()
            .expect_err("payload must reject")
            .to_string();
        assert!(
            error.contains("V13"),
            "existing payload error priority changed: {error}"
        );
    }
}
