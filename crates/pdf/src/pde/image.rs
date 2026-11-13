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
//! 链式滤镜（`/Filter [/AHx /Fl]` 数组形态）按序解码。
//!
//! # 安全面与诚实面（#180/#182 修复轮，2026-11-13）
//!
//! - **输出预算**：解码是 DoS 放大器（11 字节链式输入实测膨胀
//!   47,662 倍）。有界面走 [`decode_filters_bounded`]；入口面
//!   [`decode_image_data`] 按 W×H×BPC×通道几何上限 + 全局硬顶双层
//!   预算（#180）。
//! - **Adler-32**：FlateDecode 流经 [`crate::codec::zlib_inflate`]
//!   尾校验（#180 修复）。
//! - **Predictor 诚实拒绝**（#182）：`/DecodeParms` 带 Predictor ≥ 2
//!   = typed Err 拒绝——绝不静默错解（Adobe `peErrUnsupportedFilter`
//!   同位；「忽略」在解码语境 = 解出错误数据且返回 Ok）。Predictor 1
//!   /缺省 = 无 predictor，直通。
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

/// 单滤镜解码（无预算——兼容面；#180 修复后 Flate 仍带 Adler-32 校验）。
///
/// Adobe 语义锚：`PDEImageGetData`——XObject image 数据「always
/// returned as decoded data」。未知/未实现滤镜返回 typed 错误（登记
/// 面：LZW 归后续批次，DCT/JPX/CCITT 归引擎挂接层——不在内容流层
/// 重造）。
pub fn decode_filter(filter: &[u8], data: &[u8]) -> Result<Vec<u8>, PdeError> {
    decode_filter_bounded(filter, data, usize::MAX)
}

/// 单滤镜解码（#180 有界面——输出预算 = 解码输出字节上限）。
pub fn decode_filter_bounded(
    filter: &[u8],
    data: &[u8],
    budget: usize,
) -> Result<Vec<u8>, PdeError> {
    match filter {
        b"FlateDecode" | b"Fl" => crate::codec::zlib_inflate_bounded(data, budget)
            .map_err(|e| decode_err("FlateDecode", &e)),
        b"ASCIIHexDecode" | b"AHx" => ascii_hex_decode_bounded(data, budget),
        b"ASCII85Decode" | b"A85" => ascii85_decode_bounded(data, budget),
        b"RunLengthDecode" | b"RL" => run_length_decode_bounded(data, budget),
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
///
/// 兼容面（无预算）；有界面走 [`decode_filters_bounded`]（#180）。
pub fn decode_filters(filters: &[Vec<u8>], data: &[u8]) -> Result<Vec<u8>, PdeError> {
    decode_filters_bounded(filters, data, usize::MAX)
}

/// 滤镜链解码（#180 有界面——**每层输出** ≤ budget：中间层同受约束
/// （Lead 实测 11 字节三层链正是中间层膨胀打穿内存——层间是放大器，
/// 不是只有最终输出危险）；中间产物不消耗预算池（合法压缩链的中间
/// 层可以大于最终输出——如 AHx 解出的 zlib 流比最终解压输出大，这不
/// 是膨胀炸弹）。炸弹拒绝语义 = 任一层输出一超 budget 即拒。
pub fn decode_filters_bounded(
    filters: &[Vec<u8>],
    data: &[u8],
    budget: usize,
) -> Result<Vec<u8>, PdeError> {
    let mut cur = data.to_vec();
    for f in filters {
        cur = decode_filter_bounded(f, &cur, budget)?;
    }
    Ok(cur)
}

/// image 数据面解码主入口（`PDEImageGetData` 的 decoded 语义）。
///
/// 无滤镜 → 原样字节（Vec 拷贝语义与 Adobe buffer 语义对齐）。
///
/// # 安全面（#180 修复轮）
///
/// 双层预算：**元素级几何上限**（W×H×BPC×通道数——attrs 可算即算，
/// attrs 缺项/不可算 = 跳过）+ **全局硬顶** [`DECODE_HARD_LIMIT`]。
/// 几何上限 = 1 字节超界的宽容面（行尾填充等规范外零头），硬顶 =
/// attrs 缺失时的兜底。超预算 = typed Err。
pub fn decode_image_data(img: &PdeImage) -> Result<Vec<u8>, PdeError> {
    decode_image_data_bounded(img, DECODE_HARD_LIMIT)
}

/// 解码输出全局硬顶（#180：attrs 几何面缺失时的兜底上限）。
///
/// 取 64 MiB：覆盖真实文档最大图像面（PDF/A 归档级 16K×16K×8bpc 灰度
/// = 256 MiB 的极罕见档不在内容流层解码承诺内——那种走引擎层），
/// 同时 11 字节→52 万字节级炸弹在预算池第一层即拒。
pub const DECODE_HARD_LIMIT: usize = 64 * 1024 * 1024;

/// image attrs 的几何上限（W×H×BPC×通道；缺项 = None——由硬顶兜底）。
fn image_geometry_budget(img: &PdeImage) -> Option<usize> {
    let w = image_attr_num(img, b"Width")?;
    let h = image_attr_num(img, b"Height")?;
    let bpc = image_attr_num(img, b"BitsPerComponent").unwrap_or(8);
    let channels = image_attr_channels(img).unwrap_or(1);
    if w <= 0 || h <= 0 || bpc <= 0 {
        return None;
    }
    // 每行字节向上取整 × 行数（位流 → 字节流），+1 宽容零头。
    let row_bytes = (w as usize * bpc as usize * channels).div_ceil(8);
    Some(row_bytes.saturating_mul(h as usize).saturating_add(1))
}

/// attrs 色彩空间 → 通道数（Device 族 + 缩写；其余 = None 走硬顶）。
fn image_attr_channels(img: &PdeImage) -> Option<usize> {
    for (k, v) in &img.attrs {
        if abbrev_to_full(k) == b"ColorSpace" {
            if let Operand::Name(n) = v {
                return match n.as_slice() {
                    b"DeviceGray" | b"G" => Some(1),
                    b"DeviceRGB" | b"RGB" => Some(3),
                    b"DeviceCMYK" | b"CMYK" => Some(4),
                    _ => None, // /CS0 等资源名——语义面批③，走硬顶
                };
            }
            // 数组形态（[/Indexed ...]）语义面批③——None 走硬顶
            return None;
        }
    }
    None
}

/// image 数据面解码（#180 有界面——显式预算入口）。
///
/// `budget` 为几何上限与硬顶之外的**第三个**调用方约束（取三者最小；
/// usize::MAX = 只走几何+硬顶双层）。
pub fn decode_image_data_bounded(img: &PdeImage, budget: usize) -> Result<Vec<u8>, PdeError> {
    let filters = image_filters(img);
    if filters.is_empty() {
        // 无滤镜：原样字节也过预算门（拷贝即输出）。
        if img.data.len() > budget {
            return Err(decode_err(
                "Image",
                &format!("图像数据 {} 字节超预算 {}", img.data.len(), budget),
            ));
        }
        return Ok(img.data.clone());
    }
    // #182 Predictor 诚实拒绝：DecodeParms 带 Predictor ≥ 2 → typed Err。
    reject_unsupported_predictor(img)?;
    let geo = image_geometry_budget(img)
        .map(|g| g.min(budget))
        .unwrap_or(budget);
    let effective = geo.min(DECODE_HARD_LIMIT);
    decode_filters_bounded(&filters, &img.data, effective)
}

/// #182：`/DecodeParms` 的 Predictor 检查（诚实拒绝面）。
///
/// Predictor ≥ 2（PNG/TIFF 预测器）未实现——返回 typed Err 而非
/// 静默错解。Predictor 1 / 缺省 = 无预测，直通。多个 DecodeParms
/// 与滤镜数组对位（PDF 规范：数组可含 null 占位）。
fn reject_unsupported_predictor(img: &PdeImage) -> Result<(), PdeError> {
    for (k, v) in &img.attrs {
        if abbrev_to_full(k) != b"DecodeParms" {
            continue;
        }
        let dicts: Vec<&Operand> = match v {
            Operand::Dict(_) => vec![v],
            Operand::Array(items) => items.iter().collect(),
            _ => continue,
        };
        for d in dicts {
            if let Operand::Dict(pairs) = d {
                for (key, val) in pairs {
                    if key.as_name_bytes() == Some(b"Predictor") {
                        if let Operand::Int(p) = val {
                            if *p >= 2 {
                                return Err(decode_err(
                                    "FlateDecode",
                                    &format!(
                                        "Predictor {p} 未实现——拒绝而非静默错解（#182：\
                                         预测器流须反预测后才有效，忽略 = 返回错误图像数据）"
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// ASCIIHexDecode（`>` 终结；空白忽略；奇数位补零——PDF 规范宽容面，
/// Acrobat 同容错）。无预算兼容面（2 倍膨胀上限天然安全——输入 2 字节
/// 产 1 字节输出）。
pub fn ascii_hex_decode(data: &[u8]) -> Result<Vec<u8>, PdeError> {
    ascii_hex_decode_bounded(data, usize::MAX)
}

/// ASCIIHexDecode（#180 有界面）。
pub fn ascii_hex_decode_bounded(data: &[u8], budget: usize) -> Result<Vec<u8>, PdeError> {
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
                        if out.len() >= budget {
                            return Err(hex_budget_err(out.len(), budget));
                        }
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
///
/// 无预算兼容面；有界面走 [`ascii85_decode_bounded`]（#180）。
pub fn ascii85_decode(data: &[u8]) -> Result<Vec<u8>, PdeError> {
    ascii85_decode_bounded(data, usize::MAX)
}

/// 五字符组值折叠（u64 中间量——#180 修复核心）。
///
/// 组值合法域 = ≤ 2³²−1（PDF 规范「the value of the group must be at
/// most 2³² − 1」）。旧实现纯 u32 累加：`uuuuu` = 4,437,053,124 >
/// u32::MAX → debug panic / release wrapping 错字节。本函数 u64 折叠
/// （85⁴×84 < 2⁴⁸，u64 无溢出），域外组 = typed Err（非法输入拒绝，
/// 非静默错值）。
fn a85_group_value(group: &[u8; 5]) -> Result<u32, PdeError> {
    let v: u64 = group.iter().fold(0u64, |acc, &c| acc * 85 + c as u64);
    if v > u32::MAX as u64 {
        return Err(decode_err(
            "ASCII85Decode",
            &format!("组值 {v} 超出 2³²−1（PDF 规范非法组——编码器保证，对抗输入不保证）"),
        ));
    }
    Ok(v as u32)
}

/// ASCII85Decode（#180 有界面——输出预算 + 组值域校验）。
///
/// 膨胀比 = 4 字节输出 / 5 字节输入（组）+ z 组 4/1——`z`×N 是最小
/// 炸弹形态（每输入字节产 4 输出字节），预算埋点在 z 组与完整组两处。
pub fn ascii85_decode_bounded(data: &[u8], budget: usize) -> Result<Vec<u8>, PdeError> {
    let mut out = Vec::new();
    let mut group: [u8; 5] = [0; 5];
    let mut n = 0usize;
    for &b in data {
        match b {
            b' ' | b'\t' | b'\r' | b'\n' | 0x0c => continue,
            b'z' if n == 0 => {
                if out.len() + 4 > budget {
                    return Err(a85_budget_err(out.len() + 4, budget));
                }
                out.extend_from_slice(&[0, 0, 0, 0]);
            }
            b'~' => break,
            b'!'..=b'u' => {
                group[n] = b.wrapping_sub(b'!');
                n += 1;
                if n == 5 {
                    let v = a85_group_value(&group)?;
                    if out.len() + 4 > budget {
                        return Err(a85_budget_err(out.len() + 4, budget));
                    }
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
        let v = a85_group_value(&group)?;
        let bytes = v.to_be_bytes();
        if out.len() + (n - 1) > budget {
            return Err(a85_budget_err(out.len() + (n - 1), budget));
        }
        out.extend_from_slice(&bytes[..n - 1]);
    }
    Ok(out)
}

/// A85 预算错误（诊断面）。
fn a85_budget_err(at: usize, budget: usize) -> PdeError {
    decode_err(
        "ASCII85Decode",
        &format!("解码输出超预算：第 {at} 字节处超出上限 {budget}（z 组炸弹拒绝）"),
    )
}

/// RunLengthDecode（PDF 变体：0-127 = 后随 length+1 字节字面；128 =
/// EOD；129-255 = 下一字节重复 257-length 次）。无预算兼容面。
pub fn run_length_decode(data: &[u8]) -> Result<Vec<u8>, PdeError> {
    run_length_decode_bounded(data, usize::MAX)
}

/// RunLengthDecode（#180 有界面——重复段是膨胀点：2 输入字节最多产
/// 128 输出字节）。
pub fn run_length_decode_bounded(data: &[u8], budget: usize) -> Result<Vec<u8>, PdeError> {
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
                if out.len() + take > budget {
                    return Err(rl_budget_err(out.len() + take, budget));
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
                if out.len() + rep > budget {
                    return Err(rl_budget_err(out.len() + rep, budget));
                }
                let b = data[i];
                i += 1;
                out.extend(std::iter::repeat_n(b, rep));
            }
        }
    }
    Ok(out)
}

/// RL 预算错误（诊断面）。
fn rl_budget_err(at: usize, budget: usize) -> PdeError {
    decode_err(
        "RunLengthDecode",
        &format!("解码输出超预算：第 {at} 字节处超出上限 {budget}（重复段炸弹拒绝）"),
    )
}

/// AHx 预算错误（诊断面）。
fn hex_budget_err(at: usize, budget: usize) -> PdeError {
    decode_err(
        "ASCIIHexDecode",
        &format!("解码输出超预算：第 {at} 字节处超出上限 {budget}"),
    )
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

    // ===== #180/#182 修复轮验收（round143 双向验收面：反例全灭） =====

    /// #180：uuuuu 组 = 4,437,053,124 > u32::MAX——必须 typed Err
    /// （旧实现 debug panic / release wrapping 错字节）。
    #[test]
    fn a85_overflow_group_rejected() {
        let r = ascii85_decode(b"uuuuu~>");
        assert!(r.is_err(), "uuuuu 组值超 2³²−1 必须拒绝");
        match r.unwrap_err() {
            PdeError::Parse { op, reason } => {
                assert_eq!(op, "ASCII85Decode");
                assert!(reason.contains("2³²"), "错误信息带规范语义：{reason}");
            }
            _ => panic!("应返回 Parse 形态"),
        }
    }

    /// #180 边界：合法最大组 s8W-! = 0xFFFFFFFF 恰好 = 2³²−1——必须
    /// 继续正常解码（机器核验：Python a85encode(b"\\xff\\xff\\xff\\xff")）。
    #[test]
    fn a85_max_legal_group_still_ok() {
        let out = ascii85_decode(b"s8W-!~>").unwrap();
        assert_eq!(out, vec![0xFF, 0xFF, 0xFF, 0xFF]);
    }

    /// #180 预算：z 组炸弹（500,002 输入 → 2,000,000 输出）在预算内拒。
    #[test]
    fn a85_z_bomb_budget_rejected() {
        let bomb = vec![b'z'; 500_000];
        let mut enc = bomb;
        enc.extend_from_slice(b"~>");
        // 预算 1000：第 1000 字节即拒（不再产 2MB）
        let r = ascii85_decode_bounded(&enc, 1000);
        assert!(r.is_err(), "z 组炸弹必须在预算处拒绝");
        assert!(r.unwrap_err().to_string().contains("超预算"));
        // 等值预算放行（4×250=1000 = 边界含等号）
        let small = vec![b'z'; 250];
        let mut s = small;
        s.extend_from_slice(b"~>");
        assert_eq!(ascii85_decode_bounded(&s, 1000).unwrap().len(), 1000);
    }

    /// #180 预算：RL 重复段炸弹（129 = 128 字节重复 / 2 输入字节）。
    #[test]
    fn rl_repeat_bomb_budget_rejected() {
        // 255,0xFF,128 → 0xFF×2 + EOD；×100 段 = 200 输入 → 200 输出
        let mut data = Vec::new();
        for _ in 0..100 {
            data.extend_from_slice(&[255, 0xFF]);
        }
        data.push(128);
        // 预算 50：第 50 字节即拒
        assert!(run_length_decode_bounded(&data, 50).is_err());
        // 预算 200 放行
        let out = run_length_decode_bounded(&data, 200).unwrap();
        assert_eq!(out.len(), 200);
    }

    /// #180 预算：链式滤镜每层输出 ≤ budget（中间层同受约束——
    /// Lead 的 11 字节炸弹正是中间层膨胀）。
    #[test]
    fn chain_budget_layered() {
        // AHx(Fl("Hello"))：首层 AHx 出 13 字节 zlib 流，次层 Fl 出 5
        // 字节——budget 13 放行（每层 ≤ 13）
        let hexed: Vec<u8> = ZLIB_HELLO
            .iter()
            .flat_map(|b| format!("{:02X}", b).into_bytes())
            .collect();
        let mut with_end = hexed;
        with_end.push(b'>');
        let out =
            decode_filters_bounded(&[b"AHx".to_vec(), b"Fl".to_vec()], &with_end, 13).unwrap();
        assert_eq!(out, b"Hello");
        // budget 4：首层 AHx 输出 13 字节即拒（中间层超限 = 炸弹拒绝）
        assert!(decode_filters_bounded(&[b"AHx".to_vec(), b"Fl".to_vec()], &with_end, 4).is_err());
    }

    /// #182：Predictor ≥ 2 必须拒绝（旧实现静默 Ok + 错误图像数据）。
    #[test]
    fn predictor_rejected_typed() {
        let img = PdeImage {
            gstate: Default::default(),
            attrs: vec![
                (b"W".to_vec(), Operand::Int(4)),
                (b"H".to_vec(), Operand::Int(2)),
                (b"BPC".to_vec(), Operand::Int(8)),
                (b"CS".to_vec(), Operand::Name(b"DeviceGray".to_vec())),
                (b"F".to_vec(), Operand::Name(b"FlateDecode".to_vec())),
                (
                    b"DP".to_vec(),
                    Operand::Dict(vec![
                        (Operand::Name(b"Predictor".to_vec()), Operand::Int(12)),
                        (Operand::Name(b"Columns".to_vec()), Operand::Int(4)),
                    ]),
                ),
            ],
            data: ZLIB_RANGE8.to_vec(),
        };
        let r = decode_image_data(&img);
        assert!(r.is_err(), "Predictor 12 未实现必须拒绝而非静默错解");
        match r.unwrap_err() {
            PdeError::Parse { op, reason } => {
                assert_eq!(op, "FlateDecode");
                assert!(reason.contains("Predictor"), "错误信息带语义：{reason}");
            }
            _ => panic!("应返回 Parse 形态"),
        }
    }

    /// #182 边界：Predictor 1 = 无预测器——正常直通解码。
    #[test]
    fn predictor_one_passes_through() {
        let img = PdeImage {
            gstate: Default::default(),
            attrs: vec![
                (b"W".to_vec(), Operand::Int(8)),
                (b"H".to_vec(), Operand::Int(1)),
                (b"BPC".to_vec(), Operand::Int(8)),
                (b"F".to_vec(), Operand::Name(b"FlateDecode".to_vec())),
                (
                    b"DP".to_vec(),
                    Operand::Dict(vec![(
                        Operand::Name(b"Predictor".to_vec()),
                        Operand::Int(1),
                    )]),
                ),
            ],
            data: ZLIB_RANGE8.to_vec(),
        };
        assert_eq!(
            decode_image_data(&img).unwrap(),
            (0u8..8).collect::<Vec<u8>>()
        );
    }

    /// #180 几何预算：attrs 几何上限（W×H×BPC×通道 + 1）作为解码约束。
    #[test]
    fn geometry_budget_from_attrs() {
        // 几何 = 8×1×8bpc×1ch = 8 字节 +1 = 9——8 字节输出放行
        let img = PdeImage {
            gstate: Default::default(),
            attrs: vec![
                (b"W".to_vec(), Operand::Int(8)),
                (b"H".to_vec(), Operand::Int(1)),
                (b"BPC".to_vec(), Operand::Int(8)),
                (b"F".to_vec(), Operand::Name(b"FlateDecode".to_vec())),
            ],
            data: ZLIB_RANGE8.to_vec(),
        };
        assert_eq!(decode_image_data(&img).unwrap().len(), 8);
    }

    /// zlib.compress(bytes(range(8))) 固定向量（与 codec 单测同源——
    /// 机器生成核验，adler-32 = 0x5C001D）。
    const ZLIB_RANGE8: &[u8] = &[
        0x78, 0x9c, 0x63, 0x60, 0x64, 0x62, 0x66, 0x61, 0x65, 0x63, 0x07, 0x00, 0x00, 0x5c, 0x00,
        0x1d,
    ];
}
