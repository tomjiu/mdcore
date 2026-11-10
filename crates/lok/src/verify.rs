//! 保存产物独立校验：复用 zip 解析、CRC 与存储/deflate 支持。
//! 所有条目有解压总量限制，document.xml 单独限制大小；损坏输入返回错误。
//! 校验完成后再由宿主决定目标替换，不在此覆盖源文件。

use std::io::Read;
use std::path::Path;

#[cfg(test)]
const CD_SIGN: [u8; 4] = [0x50, 0x4B, 0x01, 0x02];
#[cfg(test)]
const LFH_SIGN: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];

/// 一次校验的期望断言集。
#[derive(Debug, Clone, Default)]
pub struct VerifySpec {
    /// 必须在 word/document.xml（解压后）中出现的文本片段。
    pub must_contain: Vec<String>,
    /// 必须不在 word/document.xml 中出现的文本片段。
    pub must_not_contain: Vec<String>,
    /// 必须存在的其他条目（如 word/media/image1.png）。
    pub must_have_entries: Vec<String>,
}

/// 校验报告（冒烟日志与测试断言用）。
#[derive(Debug, Clone)]
pub struct VerifyReport {
    pub entry_count: usize,
    pub entries: Vec<String>,
    pub document_xml_len: usize,
    pub checks_passed: usize,
}

impl VerifySpec {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn must_contain(mut self, s: impl Into<String>) -> Self {
        self.must_contain.push(s.into());
        self
    }

    pub fn must_not_contain(mut self, s: impl Into<String>) -> Self {
        self.must_not_contain.push(s.into());
        self
    }

    pub fn must_have_entry(mut self, s: impl Into<String>) -> Self {
        self.must_have_entries.push(s.into());
        self
    }
}

/// 校验保存产物。任一断言失败 → `VerifyFailed`（信息指明哪条）。
pub fn verify_saved_docx(path: &Path, spec: &VerifySpec) -> Result<VerifyReport, String> {
    const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
    const MAX_XML_BYTES: u64 = 64 * 1024 * 1024;
    const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
    let file = std::fs::File::open(path).map_err(|e| format!("读取保存产物失败 {path:?}：{e}"))?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_ARCHIVE_BYTES {
        return Err("保存产物超过容器大小限制".into());
    }
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("{path:?} 不是 zip 或容器损坏：{e}"))?;
    if archive.len() > 100_000 {
        return Err("保存产物条目过多".into());
    }
    let mut names = Vec::with_capacity(archive.len());
    let mut document = None;
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("保存产物条目读取失败：{e}"))?;
        let name = entry.name().to_owned();
        names.push(name.clone());
        if entry.is_dir() {
            continue;
        }
        let declared = entry.size();
        let limit = if name == "word/document.xml" {
            MAX_XML_BYTES
        } else {
            MAX_TOTAL_BYTES
        };
        if declared > limit
            || total
                .checked_add(declared)
                .is_none_or(|n| n > MAX_TOTAL_BYTES)
        {
            return Err(format!("保存产物解压大小超限：{name}"));
        }
        let actual = if name == "word/document.xml" {
            let mut bytes = Vec::new();
            entry
                .by_ref()
                .take(limit + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| format!("document.xml 解压/CRC校验失败：{e}"))?;
            let len = bytes.len() as u64;
            if len > limit {
                return Err("document.xml 解压大小超限".into());
            }
            document = Some(bytes);
            len
        } else {
            std::io::copy(&mut entry.by_ref().take(limit + 1), &mut std::io::sink())
                .map_err(|e| format!("保存产物条目解压/CRC校验失败 {name}：{e}"))?
        };
        if actual != declared {
            return Err(format!("保存产物条目长度不符：{name}"));
        }
        total = total
            .checked_add(actual)
            .filter(|n| *n <= MAX_TOTAL_BYTES)
            .ok_or_else(|| "保存产物解压总大小超限".to_string())?;
    }
    for want in &spec.must_have_entries {
        if !names.contains(want) {
            return Err(format!(
                "保存产物缺条目 {want}（现有：{}）",
                names.join(", ")
            ));
        }
    }
    let bytes = document.ok_or_else(|| "保存产物缺 word/document.xml".to_string())?;
    let text = std::str::from_utf8(&bytes).map_err(|e| format!("document.xml 编码无效：{e}"))?;
    let mut passed = spec.must_have_entries.len();
    for want in &spec.must_contain {
        if !text.contains(want.as_str()) {
            return Err(format!(
                "document.xml 缺期望文本：{want:?}（编辑未生效或保存丢失）"
            ));
        }
        passed += 1;
    }
    for banned in &spec.must_not_contain {
        if text.contains(banned.as_str()) {
            return Err(format!(
                "document.xml 仍含被替换文本：{banned:?}（替换未生效）"
            ));
        }
        passed += 1;
    }
    Ok(VerifyReport {
        entry_count: names.len(),
        entries: names,
        document_xml_len: bytes.len(),
        checks_passed: passed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_bad_container_is_error_without_panic(tag: &str, bytes: &[u8]) {
        let dir =
            std::env::temp_dir().join(format!("mdc-lok-bad-zip-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.docx");
        std::fs::write(&path, bytes).unwrap();
        let result = std::panic::catch_unwind(|| verify_saved_docx(&path, &VerifySpec::new()));
        std::fs::remove_dir_all(dir).unwrap();
        assert!(
            result.is_ok(),
            "malformed ZIP must return an error, not panic"
        );
        assert!(result.unwrap().is_err());
    }

    #[test]
    fn short_eocd_returns_error_without_panic() {
        assert_bad_container_is_error_without_panic("eocd", b"PK\x05\x06");
    }

    #[test]
    fn oversized_directory_name_returns_error_without_panic() {
        let mut bytes = stored_zip(&[("word/document.xml", b"text".to_vec())]);
        let cd = bytes.windows(4).position(|w| w == CD_SIGN).unwrap();
        bytes[cd + 28..cd + 30].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_bad_container_is_error_without_panic("name", &bytes);
    }

    #[test]
    fn truncated_local_header_returns_error_without_panic() {
        let mut bytes = stored_zip(&[("word/document.xml", b"text".to_vec())]);
        let cd = bytes.windows(4).position(|w| w == CD_SIGN).unwrap();
        let off = bytes.len() as u32;
        bytes[cd + 42..cd + 46].copy_from_slice(&off.to_le_bytes());
        bytes.extend_from_slice(&LFH_SIGN);
        assert_bad_container_is_error_without_panic("local", &bytes);
    }

    #[test]
    fn corrupt_payload_crc_is_rejected() {
        let mut bytes = stored_zip(&[("word/document.xml", b"CRC_SENTINEL".to_vec())]);
        let payload = bytes
            .windows(12)
            .position(|w| w == b"CRC_SENTINEL")
            .unwrap();
        bytes[payload] = b'X';
        assert_bad_container_is_error_without_panic("crc", &bytes);
    }

    #[test]
    fn rejects_non_zip() {
        let dir = std::env::temp_dir().join("mdc-lok-verify-t1");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bad.docx");
        std::fs::write(&p, b"not a zip at all").unwrap();
        let err = verify_saved_docx(&p, &VerifySpec::new()).unwrap_err();
        assert!(err.contains("不是 zip"), "拒非 zip（实际：{err}）");
        std::fs::remove_dir_all(&dir).ok();
    }

    fn zip_bytes(entries: &[(&str, Vec<u8>)], method: zip::CompressionMethod) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(method);
        for (name, bytes) in entries {
            writer.start_file(*name, options).unwrap();
            std::io::Write::write_all(&mut writer, bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn stored_zip(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        zip_bytes(entries, zip::CompressionMethod::Stored)
    }

    #[test]
    fn stored_and_deflate_entries_both_read() {
        let dir = std::env::temp_dir().join("mdc-lok-verify-t2");
        std::fs::create_dir_all(&dir).unwrap();
        // stored 条目
        let p = dir.join("stored.docx");
        std::fs::write(
            &p,
            stored_zip(&[
                ("[Content_Types].xml", b"<Types/>".to_vec()),
                ("word/document.xml", "<w:p>甲乙丙</w:p>".as_bytes().to_vec()),
                ("word/media/image1.png", vec![0x89, b'P', b'N', b'G']),
            ]),
        )
        .unwrap();
        let report = verify_saved_docx(
            &p,
            &VerifySpec::new()
                .must_contain("甲乙丙")
                .must_have_entry("word/media/image1.png"),
        )
        .unwrap();
        assert_eq!(report.entry_count, 3);
        assert_eq!(report.checks_passed, 2);

        // deflate 条目（真实 LO saveAs 的压缩形态）：手造。
        let raw =
            "<w:document><w:body><w:p>丁戊己 REPLACEME</w:p></w:body></w:document>".as_bytes();
        let out = zip_bytes(
            &[("word/document.xml", raw.to_vec())],
            zip::CompressionMethod::Deflated,
        );
        let p2 = dir.join("deflated.docx");
        std::fs::write(&p2, &out).unwrap();
        let report = verify_saved_docx(
            &p2,
            &VerifySpec::new()
                .must_contain("REPLACEME")
                .must_not_contain("不存在的词"),
        )
        .unwrap();
        assert_eq!(report.document_xml_len, raw.len());

        // 期望文本缺失要红：
        let err =
            verify_saved_docx(&p2, &VerifySpec::new().must_contain("不存在的词")).unwrap_err();
        assert!(err.contains("缺期望文本"), "缺失断言红（实际：{err}）");
        std::fs::remove_dir_all(&dir).ok();
    }
}
