//! pde::image —— PDEImage 数据面（批②第二件，issue #162）。
//!
//! Adobe 参照：`PDEImageGetData`——"If the image is an XObject image,
//! data is always returned as **decoded** data"（内联图相反：Adobe 按
//! `PDEImageDataIsEncoded` 区分）。本模块提供流滤镜的**解码链**：
//!
//! | 滤镜 | PDF 名 | 解码 | 实现 |
//! |---|---|---|---|
//! | FlateDecode | `Fl` | zlib/DEFLATE | crate 级共享件 [`crate::codec::zlib_inflate`]（2026-11-13 自 stamp.rs 上移——零三方依赖纪律） |
//! | ASCIIHexDecode | `AHx` | hex → bytes | 本模块 |
//! | ASCII85Decode | `A85` | base85 → bytes | 本模块（PDF 变体：`z` = 四零、`~>` 终结） |
//! | RunLengthDecode | `RL` | RLE → bytes | 本模块（PDF 变体：长度字节 0-127 字面、128 = EOD、129-255 重复） |
//! | LZWDecode | `LZW` | —— | **批②不实现**（登记：早期文件罕见，pdfium 兜底；实现归后续批次，见测试 `unsupported_filter_typed_error`） |
//! | DCTDecode/JPXDecode/CCITTFaxDecode/JBIG2Decode | —— | —— | 已压缩图像域（解 JPEG/JPEG2000/传真位流超出内容流层职责——Adobe PDEImage 对 DCT/JPX 有专门类 PDEImageJPX，语义面归引擎挂接层批③） |
//!
//! 链式滤镜（`/Filter [/AHx /Fl]` 数组形态）按序解码；`/DecodeParms`
//! 批②忽略（Predictor 面在 FlateDecode 图像流罕见，登记后续批次）。
//!
//! 内联图与 named image 的统一面：批① [`PdeImage`] 的 attrs 用内联
//! 缩写名（`/W /H /BPC /CS /F`），本模块 [`image_filters`] 同步接受
//! 缩写与全名（PDF 表 4.7 内联缩写对照）。

use super::model::{Operand, PdeError, PdeImage};

/// 内联图属性缩写 → 全名（PDF 规范 §4.8.6 内联图像缩写表）。
pub fn abbrev_to_full(abbrev: &[u8]) -> &[u8] {
    match abbrev {
        b"W" => b"Width",
        b"H" => b"Height",
        b"BPC" => b"BitsPerComponent",
        b"CS" => b"ColorSpace",
        b"F" => b"Filter",
        b"DP" => b"DecodeParms",
        b"IM" => b"ImageMask",
        b"D" => b"Decode",
        b"I" => b"Interpolate",
        other => other,
    }
}

/// 从 image attrs 取滤镜名列表（缩写/全名同收；名字或数组形态）。
///
/// 返回空 = 无滤镜（数据原样）。Adobe `PDEImageGetFilters` 面。
pub fn image_filters(img: &PdeImage) -> Vec<Vec<u8>> {
    let mut filters = Vec::new();
    for (k, v) in &img.attrs {
        if abbrev_to_full(k) == b"Filter" {
            match v {
                Operand::Name(n) => filters.push(n.clone()),
                Operand::Array(items) => {
                    for i in items {
                        if let Operand::Name(n) = i {
                            filters.push(n.clone());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    filters
}

/// 从 image attrs 取数值（W/H/BPC 面批②共用取值器）。
pub fn image_attr_num(img: &PdeImage, key: &[u8]) -> Option<i64> {
    for (k, v) in &img.attrs {
        if abbrev_to_full(k) == key {
            match v {
                Operand::Int(i) => return Some(*i),
                Operand::Real(r) => return Some(*r as i64),
                _ => {}
            }
        }
    }
    None
}

/// 解码错误（带滤镜名——诊断面与 [`PdeError::Parse`] 的 op 语义对齐）。
fn decode_err(filter: &str, reason: &str) -> PdeError {
    PdeError::Parse {
        op: filter.to_string(),
        reason: reason.to_string(),
    }
}

/// 单滤镜解码。
///
/// Adobe 语义锚：`PDEImageGetData`——XObject image 数据「always
/// returned as decoded data」。未知/未实现滤镜返回 typed 错误（登记
/// 面：LZW 归后续批次，DCT/JPX/CCITT 归引擎挂接层——不在内容流层
/// 重造）。
pub fn decode_filter(filter: &[u8], data: &[u8]) -> Result<Vec<u8>, PdeError> {
    match filter {
        b"FlateDecode" | b"Fl" => {
            crate::codec::zlib_inflate(data).map_err(|e| decode_err("FlateDecode", &e))
        }
        b"ASCIIHexDecode" | b"AHx" => ascii_hex_decode(data),
        b"ASCII85Decode" | b"A85" => ascii85_decode(data),
        b"RunLengthDecode" | b"RL" => run_length_decode(data),
        b"LZWDecode" | b"LZW" => Err(decode_err(
            "LZWDecode",
            "批②未实现（早期文件罕见，pdfium 兜底；登记后续批次）",
        )),
        b"DCTDecode" | b"DCT" => Err(decode_err(
            "DCTDecode",
            "JPEG 域——不在内容流层解码（引擎挂接层批③）",
        )),
        b"JPXDecode" => Err(decode_err(
            "JPXDecode",
            "JPEG2000 域——不在内容流层解码（引擎挂接层批③）",
        )),
        b"CCITTFaxDecode" | b"CCF" => Err(decode_err(
            "CCITTFaxDecode",
            "传真位流域——不在内容流层解码（引擎挂接层批③）",
        )),
        b"JBIG2Decode" => Err(decode_err(
            "JBIG2Decode",
            "JBIG2 位流域——不在内容流层解码（引擎挂接层批③）",
        )),
        other => Err(decode_err(
            "Filter",
            &format!("未知滤镜 {}", String::from_utf8_lossy(other)),
        )),
    }
}

/// 滤镜链解码（`/Filter [/AHx /Fl]`——按序解码；单滤镜同走）。
pub fn decode_filters(filters: &[Vec<u8>], data: &[u8]) -> Result<Vec<u8>, PdeError> {
    let mut cur = data.to_vec();
    for f in filters {
        cur = decode_filter(f, &cur)?;
    }
    Ok(cur)
}

/// image 数据面解码主入口（`PDEImageGetData` 的 decoded 语义）。
///
/// 无滤镜 → 原样字节（Vec 拷贝语义与 Adobe buffer 语义对齐）。
pub fn decode_image_data(img: &PdeImage) -> Result<Vec<u8>, PdeError> {
    let filters = image_filters(img);
    if filters.is_empty() {
        return Ok(img.data.clone());
    }
    decode_filters(&filters, &img.data)
}

/// ASCIIHexDecode（`>` 终结；空白忽略；奇数位补零——PDF 规范宽容面，
/// Acrobat 同容错）。
pub fn ascii_hex_decode(data: &[u8]) -> Result<Vec<u8>, PdeError> {
    let mut out = Vec::new();
    let mut hi: Option<u8> = None;
    for &b in data {
        match b {
            b'>' => break,
            b' ' | b'\t' | b'\r' | b'\n' | 0x0c => continue,
            b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F' => {
                let v = match b {
                    b'0'..=b'9' => b - b'0',
                    b'a'..=b'f' => b - b'a' + 10,
                    _ => b - b'A' + 10,
                };
                match hi {
                    None => hi = Some(v),
                    Some(h) => {
                        out.push((h << 4) | v);
                        hi = None;
                    }
                }
            }
            other => {
                return Err(decode_err(
                    "ASCIIHexDecode",
                    &format!("非法十六进制字符 0x{:02X}", other),
                ));
            }
        }
    }
    if let Some(h) = hi {
        // 奇数位补零（PDF 规范：最后一个半字节右对齐补 0）
        out.push(h << 4);
    }
    Ok(out)
}

/// ASCII85Decode（PDF 变体：`z` = 00000 四零组；`~>` 终结；空白忽略）。
pub fn ascii85_decode(data: &[u8]) -> Result<Vec<u8>, PdeError> {
    let mut out = Vec::new();
    let mut group: [u8; 5] = [0; 5];
    let mut n = 0usize;
    for &b in data {
        match b {
            b' ' | b'\t' | b'\r' | b'\n' | 0x0c => continue,
            b'z' if n == 0 => {
                out.extend_from_slice(&[0, 0, 0, 0]);
            }
            b'~' => break,
            b'!'..=b'u' => {
                group[n] = b.wrapping_sub(b'!');
                n += 1;
                if n == 5 {
                    let v: u32 = group.iter().fold(0u32, |acc, &c| acc * 85 + c as u32);
                    out.extend_from_slice(&v.to_be_bytes());
                    n = 0;
                }
            }
            other => {
                return Err(decode_err(
                    "ASCII85Decode",
                    &format!("非法 base85 字符 0x{:02X}", other),
                ));
            }
        }
    }
    // 尾组：n-1 个额外字符 → n-1 字节（PDF 规范尾组语义）
    if n > 0 {
        if n == 1 {
            return Err(decode_err(
                "ASCII85Decode",
                "尾组仅 1 字符（非法——至少 2 字符才可编码）",
            ));
        }
        for g in &mut group[n..] {
            *g = b'u' - b'!'; // 空位按最大值填充（解码语义）
        }
        let v: u32 = group.iter().fold(0u32, |acc, &c| acc * 85 + c as u32);
        let bytes = v.to_be_bytes();
        out.extend_from_slice(&bytes[..n - 1]);
    }
    Ok(out)
}

/// RunLengthDecode（PDF 变体：0-127 = 后随 length+1 字节字面；128 =
/// EOD；129-255 = 下一字节重复 257-length 次）。
pub fn run_length_decode(data: &[u8]) -> Result<Vec<u8>, PdeError> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < data.len() {
        let l = data[i];
        i += 1;
        match l {
            0..=127 => {
                let take = l as usize + 1;
                if i + take > data.len() {
                    return Err(decode_err("RunLengthDecode", "字面段超出数据末尾"));
                }
                out.extend_from_slice(&data[i..i + take]);
                i += take;
            }
            128 => break, // EOD
            129..=255 => {
                let rep = 257 - l as usize;
                if i >= data.len() {
                    return Err(decode_err("RunLengthDecode", "重复段缺数据字节"));
                }
                let b = data[i];
                i += 1;
                out.extend(std::iter::repeat_n(b, rep));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// zlib.compress(b"Hello") 固定向量（Python zlib 标准产出）。
    const ZLIB_HELLO: &[u8] = &[
        0x78, 0x9c, 0xf3, 0x48, 0xcd, 0xc9, 0xc9, 0x07, 0x00, 0x05, 0x8c, 0x01, 0xf5,
    ];

    #[test]
    fn flate_decode_hello() {
        // zlib("Hello") 固定向量
        let out = decode_filter(b"FlateDecode", ZLIB_HELLO).unwrap();
        assert_eq!(out, b"Hello");
    }

    #[test]
    fn flate_abbrev_and_bad_data() {
        let out = decode_filter(b"Fl", ZLIB_HELLO).unwrap();
        assert_eq!(out, b"Hello");
        assert!(decode_filter(b"FlateDecode", b"not-zlib").is_err());
    }

    #[test]
    fn ascii_hex_basic() {
        let out = ascii_hex_decode(b"48656C6C6F>").unwrap();
        assert_eq!(out, b"Hello");
    }

    #[test]
    fn ascii_hex_whitespace_and_odd() {
        let out = ascii_hex_decode(b"48 65\n6C 6C 6F>").unwrap();
        assert_eq!(out, b"Hello");
        // 奇数位补零：7 → 0x70
        let out = ascii_hex_decode(b"7>").unwrap();
        assert_eq!(out, vec![0x70]);
    }

    #[test]
    fn ascii_hex_illegal_char() {
        assert!(ascii_hex_decode(b"4G>").is_err());
    }

    #[test]
    fn ascii85_basic_roundtrip() {
        // "Hello" 的 Adobe flavor base85 = <~87cURDZ~>（Python
        // base64.a85encode(adobe=True) 交叉验证）。
        let enc = b"87cURDZ~>";
        let out = ascii85_decode(enc).unwrap();
        assert_eq!(out, b"Hello");
    }

    #[test]
    fn ascii85_z_zero_group() {
        // z = 00000000 四零
        let out = ascii85_decode(b"z~>").unwrap();
        assert_eq!(out, vec![0, 0, 0, 0]);
    }

    #[test]
    fn ascii85_tail_group() {
        // 尾组语义（Python base64.a85encode 交叉验证）：
        // b"A" → <~5l~>（尾组 2 字符 → 1 字节）
        assert_eq!(ascii85_decode(b"5l~>").unwrap(), b"A");
        // b"AB" → <~5sb~>（尾组 3 字符 → 2 字节）
        assert_eq!(ascii85_decode(b"5sb~>").unwrap(), b"AB");
        // 尾组仅 1 字符 = 非法
        assert!(ascii85_decode(b"5~>").is_err());
    }

    #[test]
    fn run_length_literals() {
        // 2 字面 ("He") + 2 字面 ("ll") + 1 字面 ("o") + EOD
        let out = run_length_decode(&[1, b'H', b'e', 1, b'l', b'l', 0, b'o', 128]).unwrap();
        assert_eq!(out, b"Hello");
    }

    #[test]
    fn run_length_repeat() {
        // 257-255 = 2 次 'a'；257-129 = 128 次 'b'
        let mut input = vec![255, b'a', 129, b'b', 128];
        input[0] = 254; // 257-254=3 次 'a'
        let out = run_length_decode(&[254, b'a', 129, b'b', 128]).unwrap();
        let mut expect = vec![b'a'; 3];
        expect.extend(std::iter::repeat_n(b'b', 128));
        assert_eq!(out, expect);
        let _ = input;
    }

    #[test]
    fn run_length_truncated_literal() {
        // 字面段声明 3 字节但只剩 1
        assert!(run_length_decode(&[2, b'a']).is_err());
    }

    #[test]
    fn unsupported_filters_typed_error() {
        assert!(decode_filter(b"LZWDecode", b"x").is_err());
        assert!(decode_filter(b"DCTDecode", b"\xff\xd8").is_err());
        assert!(decode_filter(b"CCITTFaxDecode", b"x").is_err());
        // 错误信息带滤镜名（诊断面）
        let e = decode_filter(b"LZWDecode", b"x").unwrap_err();
        match e {
            PdeError::Parse { op, .. } => assert_eq!(op, "LZWDecode"),
            _ => panic!("应返回 Parse 形态"),
        }
    }

    #[test]
    fn chain_ahx_then_flate() {
        // AHx(Fl("Hello"))——链式两段
        let hexed: Vec<u8> = ZLIB_HELLO
            .iter()
            .flat_map(|b| format!("{:02X}", b).into_bytes())
            .collect();
        let mut with_end = hexed;
        with_end.push(b'>');
        let out = decode_filters(&[b"AHx".to_vec(), b"Fl".to_vec()], &with_end).unwrap();
        assert_eq!(out, b"Hello");
    }

    #[test]
    fn abbrev_mapping() {
        assert_eq!(abbrev_to_full(b"W"), b"Width");
        assert_eq!(abbrev_to_full(b"BPC"), b"BitsPerComponent");
        assert_eq!(abbrev_to_full(b"Width"), b"Width"); // 全名直通
    }
}
