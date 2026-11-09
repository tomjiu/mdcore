//! 保存产物校验（#633）——原型级最小 zip 读取器。
//!
//! 用途：`save_as` 落盘的临时 DOCX 在「决定替换源文件」前做结构
//! 与内容断言（zip 合法、document.xml 在场、期望文本已变、图片条目
//! 仍在）。**不是通用 zip 实现**：只支持 zip 恒定面（EOCD → 中央
//! 目录 → local header；stored + deflate 条目），防御性解析、拒绝
//! 畸形输入；通用容器操作继续用 mdc-office-io（本 crate 不跨域依赖）。
//!
//! deflate 解压经 flate2（Cargo.lock 现解析版本直依赖，零新编译
//! 单元，见 Cargo.toml 注释）。

use std::io::Read;
use std::path::Path;

use crate::error::{ErrorCode, LokError, LokResult};

const EOCD_SIGN: [u8; 4] = [0x50, 0x4B, 0x05, 0x06];
const CD_SIGN: [u8; 4] = [0x50, 0x4B, 0x01, 0x02];
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
#[derive(Debug, Clone, serde::Serialize)]
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
pub fn verify_saved_docx(path: &Path, spec: &VerifySpec) -> LokResult<VerifyReport> {
    let bytes = std::fs::read(path).map_err(|e| {
        LokError::new(
            ErrorCode::VerifyFailed,
            format!("读取保存产物失败 {path:?}：{e}"),
        )
    })?;
    if bytes.len() < 4 || bytes[0..2] != *b"PK" {
        return Err(LokError::new(
            ErrorCode::VerifyFailed,
            format!("{path:?} 不是 zip（保存产物损坏或为空）"),
        ));
    }
    let eocd = find_eocd(&bytes)
        .ok_or_else(|| LokError::new(ErrorCode::VerifyFailed, "zip 缺 EOCD（中央目录尾）"))?;
    let count = u16le(&bytes[eocd + 10..]) as usize;
    let cd_offset = u32le(&bytes[eocd + 16..]) as usize;

    // 解析中央目录条目。
    // （name, local_offset, csize, usize, method）
    let mut entries: Vec<(String, usize, usize, usize, u16)> = Vec::new();
    let mut p = cd_offset;
    for _ in 0..count {
        if p + 46 > bytes.len() || bytes[p..p + 4] != CD_SIGN {
            return Err(LokError::new(
                ErrorCode::VerifyFailed,
                "中央目录记录签名错位",
            ));
        }
        let method = u16le(&bytes[p + 10..]);
        let csize = u32le(&bytes[p + 20..]) as usize;
        let usize_ = u32le(&bytes[p + 24..]) as usize;
        let name_len = u16le(&bytes[p + 28..]) as usize;
        let extra_len = u16le(&bytes[p + 30..]) as usize;
        let comment_len = u16le(&bytes[p + 32..]) as usize;
        let local_off = u32le(&bytes[p + 42..]) as usize;
        let name = String::from_utf8_lossy(&bytes[p + 46..p + 46 + name_len]).into_owned();
        entries.push((name, local_off, csize, usize_, method));
        p += 46 + name_len + extra_len + comment_len;
    }

    let names: Vec<String> = entries.iter().map(|(n, ..)| n.clone()).collect();
    for want in &spec.must_have_entries {
        if !names.iter().any(|n| n == want) {
            return Err(LokError::new(
                ErrorCode::VerifyFailed,
                format!("保存产物缺条目 {want}（现有：{}）", names.join(", ")),
            ));
        }
    }

    // 提取 word/document.xml 文本。
    let doc_xml = entries
        .iter()
        .find(|(n, ..)| n == "word/document.xml")
        .map(|(_, off, csize, usize_, method)| {
            extract_entry(&bytes, *off, *csize, *usize_, *method)
        })
        .transpose()
        .map_err(|e: std::io::Error| {
            LokError::new(
                ErrorCode::VerifyFailed,
                format!("document.xml 解压失败：{e}"),
            )
        })?
        .ok_or_else(|| LokError::new(ErrorCode::VerifyFailed, "保存产物缺 word/document.xml"))?;
    let text = String::from_utf8_lossy(&doc_xml).into_owned();

    let mut passed = spec.must_have_entries.len();
    for want in &spec.must_contain {
        if !text.contains(want.as_str()) {
            return Err(LokError::new(
                ErrorCode::VerifyFailed,
                format!("document.xml 缺期望文本：{want:?}（编辑未生效或保存丢失）"),
            ));
        }
        passed += 1;
    }
    for banned in &spec.must_not_contain {
        if text.contains(banned.as_str()) {
            return Err(LokError::new(
                ErrorCode::VerifyFailed,
                format!("document.xml 仍含被替换文本：{banned:?}（替换未生效）"),
            ));
        }
        passed += 1;
    }

    Ok(VerifyReport {
        entry_count: entries.len(),
        entries: names,
        document_xml_len: doc_xml.len(),
        checks_passed: passed,
    })
}

fn find_eocd(bytes: &[u8]) -> Option<usize> {
    // EOCD 在尾部 64KB 内（zip 注释段上限）。
    let start = bytes.len().saturating_sub(66_000);
    let mut i = bytes.len() - 4;
    loop {
        if bytes[i..i + 4] == EOCD_SIGN {
            return Some(i);
        }
        if i == start {
            return None;
        }
        i -= 1;
    }
}

fn u16le(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}

fn u32le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// 从 local file header 起提取条目数据：stored 直拷 csize 字节；
/// deflate 切 csize 字节后解压（usize 仅作长度合理性对照）。
fn extract_entry(
    bytes: &[u8],
    lfh_offset: usize,
    csize: usize,
    usize_: usize,
    method: u16,
) -> std::io::Result<Vec<u8>> {
    if lfh_offset + 4 > bytes.len() || bytes[lfh_offset..lfh_offset + 4] != LFH_SIGN {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "local header 签名错位",
        ));
    }
    let name_len = u16le(&bytes[lfh_offset + 26..]) as usize;
    let extra_len = u16le(&bytes[lfh_offset + 28..]) as usize;
    let data_start = lfh_offset + 30 + name_len + extra_len;
    let data_end = data_start + csize;
    if data_end > bytes.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "条目数据越界",
        ));
    }
    let data = &bytes[data_start..data_end];
    match method {
        0 => Ok(data.to_vec()),
        8 => {
            let mut out = Vec::with_capacity(usize_.min(64 * 1024 * 1024));
            flate2::read::DeflateDecoder::new(data).read_to_end(&mut out)?;
            if usize_ != 0 && out.len() != usize_ {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("解压长度不符：{} != {usize_}", out.len()),
                ));
            }
            Ok(out)
        }
        m => Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!("不支持的压缩方法 {m}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_zip() {
        let dir = std::env::temp_dir().join("mdc-lok-verify-t");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bad.docx");
        std::fs::write(&p, b"not a zip at all").unwrap();
        let err = verify_saved_docx(&p, &VerifySpec::new()).unwrap_err();
        assert_eq!(err.code, ErrorCode::VerifyFailed);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stored_and_deflate_entries_both_read() {
        let dir = std::env::temp_dir().join("mdc-lok-verify-t2");
        std::fs::create_dir_all(&dir).unwrap();
        // 复用 mock 的 stored 写器。
        let p = dir.join("stored.docx");
        crate::mock::write_stored_zip(
            &p.to_string_lossy(),
            &[
                ("[Content_Types].xml", b"<Types/>".to_vec()),
                ("word/document.xml", "<w:p>甲乙丙</w:p>".as_bytes().to_vec()),
                ("word/media/image1.png", vec![0x89, b'P', b'N', b'G']),
            ],
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

        // deflate 条目（真实 LO saveAs 的压缩形态）：手造 CD 标 method=8。
        let raw =
            "<w:document><w:body><w:p>丁戊己 REPLACEME</w:p></w:body></w:document>".as_bytes();
        let mut enc =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut enc, raw).unwrap();
        let deflated = enc.finish().unwrap();
        let mut out: Vec<u8> = Vec::new();
        // LFH（method=8）
        out.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]);
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&8u16.to_le_bytes()); // method=deflate
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(deflated.len() as u32).to_le_bytes()); // csize
        out.extend_from_slice(&(raw.len() as u32).to_le_bytes()); // usize
        out.extend_from_slice(&17u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(b"word/document.xml");
        out.extend_from_slice(&deflated);
        let cd_off = out.len();
        // CD
        out.extend_from_slice(&[0x50, 0x4B, 0x01, 0x02]);
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&8u16.to_le_bytes()); // method=deflate
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(deflated.len() as u32).to_le_bytes());
        out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        out.extend_from_slice(&17u16.to_le_bytes()); // nlen @28
        out.extend_from_slice(&0u16.to_le_bytes()); // extra @30
        out.extend_from_slice(&0u16.to_le_bytes()); // comment @32
        out.extend_from_slice(&0u16.to_le_bytes()); // disk @34
        out.extend_from_slice(&0u16.to_le_bytes()); // iattr @36
        out.extend_from_slice(&0u32.to_le_bytes()); // eattr @38
        out.extend_from_slice(&0u32.to_le_bytes()); // offset @42
        out.extend_from_slice(b"word/document.xml");
        let cd_size = (out.len() - cd_off) as u32;
        // EOCD
        out.extend_from_slice(&[0x50, 0x4B, 0x05, 0x06, 0, 0, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&(cd_off as u32).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
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
        assert_eq!(err.code, ErrorCode::VerifyFailed);
        std::fs::remove_dir_all(&dir).ok();
    }
}
