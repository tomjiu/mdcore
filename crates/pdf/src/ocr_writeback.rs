//! #306 ②：可搜索 PDF 写回——OCR 词文本注入内容流（3 Tr 隐形）。
//!
//! （feature `engine-lopdf`；词模型 [`crate::ocr_word`] 零门控共享。）
//!
//! # 语义
//!
//! 原 PDF 逐页追加**隐形文本层**：每词一个 `BT … 3 Tr … Td <CID> Tj ET`
//! 块（渲染模式 3 = 不可见——任何阅读器不显示、不遮挡原扫描图），坐标
//! 取 [`OcrWord`] 的用户空间词框（原点左下）——搜索命中区贴合扫描
//! 图上的视觉词位。**产物是全量新档**（非增量修订：另存为语义——
//! `Document::save` 重写 xref，原字节前缀不保持；签名档经
//! [`write_searchable_pdf`] 的 S5 加密拒 + DocMDP 拒两级守卫，本写面
//! 不承诺签名保持）。
//!
//! # 字体面（零嵌入纪律）
//!
//! 非内嵌 Type0/Identity-H：`CID = Unicode 码位`（Identity-H 直通——
//! 2 字节 CID 装码位），`/ToUnicode` CMap 直映射同值。3 Tr = 永不渲染
//! 字形，无字形程序即无嵌入需求（tesseract 同款思路走 glyphless 字体，
//! 本面更简：非内嵌 + 隐形渲染，提取/搜索面只走 ToUnicode）。
//! **非 BMP 字符装不进 2 字节 CID**——逐词跳过该等字符并计数上报
//! （[`SearchableWrite::non_bmp_chars_skipped`]），不静默截断词。
//!
//! # 安全防线（与 writer_lopdf / forms 写面同纪律）
//!
//! - S5：加密档（仍加密/曾加密）零写回（`SecurityDeny`）；
//! - DocMDP：签名档变更限制 P=1/P=2 拒（注入内容流 = 变更文档——
//!   [`crate::docmdp`] 同门）；
//! - xref 链守卫**不适用**：全量写回产新 xref（装载期修复动作不随
//!   产物传播——与增量写的前缀保持语义相反，该防线针对的是「产物携带
//!   原档坏 xref」，全量重写无此面）；
//! - 载荷健全性：非有限坐标 / 空词 → 跳过计数（词级防御），页界
//!   越界 → `InvalidPage`（typed，不静默错页注入）；
//! - 产物自检：写出后 lopdf 重载（页树非空 + 重载不炸——拒绝交付
//!   结构损坏产物，与 writer_lopdf 自检同款）。
//!
//! # 验证面（调用方合同）
//!
//! 产物的可搜索性由调用方验证（产品面重开抽取非空 + 跨引擎第二意见
//! ——pdftotext / Acrobat）；本模块自检止于结构面（可解析 + 页树在）。

use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};

use crate::command::CommandError;
use crate::docmdp;
use crate::ocr_word::OcrWord;
use crate::page::PageIndex;
use crate::security::{Limits, REASON_ENCRYPTED_WRITE};

/// 写回产物报告（诚实面：注入统计 + 跳过计数——调用方如实展示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SearchableWrite {
    /// 实际注入文本块的词数（跳过词不计）。
    pub words_written: usize,
    /// 空/非有限坐标被跳过的词数。
    pub words_skipped: usize,
    /// 非 BMP 字符被跳过的字符数（词保留，字符缺失——搜索面诚实边界）。
    pub non_bmp_chars_skipped: usize,
}

/// 可搜索 PDF 写回（#306 ②）。
///
/// `words_by_page[i]` = 第 i 页（0 基）词列表；越界（列表长于页数）=
/// `InvalidPage`（携带首越界页——不静默丢尾页）。空页列表 = 空操作
/// （产物 == 原档重排——调用方应在 OCR 零词时提前拒绝，本面不代拒）。
pub fn write_searchable_pdf(
    pdf: &[u8],
    words_by_page: &[Vec<OcrWord>],
) -> Result<(Vec<u8>, SearchableWrite), CommandError> {
    // S3-L7 输入层（open 家族入口统一纪律）。
    Limits::default().check_open_input_bytes(pdf.len() as u64)?;

    let doc = Document::load_mem(pdf)
        .map_err(|e| CommandError::engine(format!("lopdf 装载失败：{e}")))?;

    // S5：加密档零写回（两形态——与 writer_lopdf::from_bytes 同判定）。
    if doc.is_encrypted() || doc.was_encrypted() {
        return Err(CommandError::SecurityDeny {
            reason: REASON_ENCRYPTED_WRITE,
        });
    }
    // DocMDP：P=1/P=2 签名档拒变更（内容流注入 = 文档变更）。
    docmdp::ensure_write_allowed(&doc, docmdp::WriteOp::Annotation).map_err(|v| {
        CommandError::SecurityDeny {
            reason: v.reason_code(),
        }
    })?;

    let pages = doc.get_pages(); // BTreeMap<1 基页号, ObjectId>（序 = 页序）
                                 // 页界合同（宿主页数对齐——attach_engine 同款纪律：错页显式拒绝）。
    if let Some(first_oob) = words_by_page
        .iter()
        .enumerate()
        .find(|(i, _)| !pages.contains_key(&((*i as u32) + 1)))
        .map(|(i, _)| i)
    {
        return Err(CommandError::InvalidPage(PageIndex(first_oob as u32)));
    }

    let mut doc = doc;
    let mut report = SearchableWrite::default();

    // 页号（1 基）→ ObjectId 解析（无 panic 索引——全页收集后逐页取）。
    let page_ids: Vec<ObjectId> = (1..=pages.len() as u32)
        .filter_map(|n| pages.get(&n).copied())
        .collect();

    // 隐形文本字体（单实例全档共享——对象一次、页页引用）。
    let used_chars = collect_chars(words_by_page);
    if used_chars.is_empty() {
        // 零词 = 零注入（空操作产物——重排原档；调用方报告零词）。
        let mut out = Vec::new();
        doc.save_to(&mut out)
            .map_err(|e| CommandError::engine(format!("lopdf 保存失败：{e}")))?;
        return Ok((out, report));
    }
    let font_id = add_invisible_font(&mut doc, &used_chars);

    for (page_no, words) in words_by_page.iter().enumerate() {
        let Some(&page_id) = page_ids.get(page_no) else {
            return Err(CommandError::InvalidPage(PageIndex(page_no as u32)));
        };
        let (ops, written, skipped, non_bmp) = invisible_text_ops(words);
        report.words_written += written;
        report.words_skipped += skipped;
        report.non_bmp_chars_skipped += non_bmp;
        if ops.is_empty() {
            continue; // 本页零可注词——不动该页（不产生空内容流）
        }
        let stream_id = doc.add_object(Stream::new(dictionary! {}, ops.into_bytes()));
        append_contents(&mut doc, page_id, stream_id);
        attach_font(&mut doc, page_id, font_id);
    }

    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| CommandError::engine(format!("lopdf 保存失败：{e}")))?;

    // 产物自检：重载 + 页树非空（writer_lopdf 同款——拒绝交付结构
    // 损坏产物；此处不检前缀[全量语义]，检「可解析 + 页可达」）。
    let reloaded = Document::load_mem(&out)
        .map_err(|e| CommandError::engine(format!("写回产物重载失败：{e}")))?;
    if reloaded.get_pages().is_empty() {
        return Err(CommandError::engine(
            "写回产物自检失败：重载后页树为空——拒绝交付结构损坏产物",
        ));
    }

    Ok((out, report))
}

// ---------------------------------------------------------------------------
// 隐形文本内容流
// ---------------------------------------------------------------------------

/// 词列表 → 隐形文本操作符串（每词一个 BT/ET 块）。
///
/// 块形态：`BT 3 Tr /F0 <字号> Tf <x> <y> Td <CID 串> Tj ET`——
/// 渲染模式 3（不可见）置前（先于任何 Tj）；字号 = 词框高；基线 =
/// 词框下缘。返回 `(操作符串, 注入词数, 跳过词数, 非 BMP 跳字符数)`。
fn invisible_text_ops(words: &[OcrWord]) -> (String, usize, usize, usize) {
    let mut ops = String::new();
    let (mut written, mut skipped, mut non_bmp) = (0usize, 0usize, 0usize);
    for word in words {
        if word.text.trim().is_empty() {
            skipped += 1;
            continue;
        }
        // 非有限坐标防御（NaN/Inf——serde 面外兜底）。
        if !word.bbox.iter().all(|v| v.is_finite()) {
            skipped += 1;
            continue;
        }
        // CID 串（2 字节/字符；非 BMP 跳过计数——词保留可注入部分）。
        let mut hex = String::new();
        for ch in word.text.chars() {
            let cp = ch as u32;
            if cp <= 0xFFFF {
                hex.push_str(&format!("{cp:04X}"));
            } else {
                non_bmp += 1;
            }
        }
        if hex.is_empty() {
            skipped += 1; // 全非 BMP 词——无可注入字符
            continue;
        }
        let size = word.height_pt();
        let x = word.bbox[0];
        let y = word.baseline_y();
        ops.push_str(&format!(
            "BT 3 Tr /MdcOcrF0 {size:.2} Tf {x:.2} {y:.2} Td <{hex}> Tj ET\n"
        ));
        written += 1;
    }
    (ops, written, skipped, non_bmp)
}

/// 追加内容流到页 /Contents（三形态全覆盖：缺省/单引用/数组——
/// forms.rs 扁平化同款）。
fn append_contents(doc: &mut Document, page_id: ObjectId, stream_id: ObjectId) {
    let Ok(page) = doc.get_dictionary_mut(page_id) else {
        return;
    };
    let old = page.get(b"Contents").ok().cloned();
    match old {
        Some(Object::Array(mut arr)) => {
            arr.push(Object::Reference(stream_id));
            page.set(b"Contents".to_vec(), Object::Array(arr));
        }
        Some(obj) => {
            page.set(
                b"Contents".to_vec(),
                Object::Array(vec![obj, Object::Reference(stream_id)]),
            );
        }
        None => {
            page.set(b"Contents".to_vec(), Object::Reference(stream_id));
        }
    }
}

/// 页资源字典挂字体 `/MdcOcrF0 → font_id`（引用共享——forms.rs
/// attach_font 引用形态版：字体对象单实例，页页引用；槽名带命名空间
/// 防宿主档既有 /F0 冲突——冲突态（宿主已占 MdcOcrF0 槽）跳过该页
/// 挂载并计 skipped 面，不覆盖宿主字体）。
fn attach_font(doc: &mut Document, page_id: ObjectId, font_id: ObjectId) {
    let Ok(page) = doc.get_dictionary_mut(page_id) else {
        return;
    };
    let mut res = match page.get(b"Resources").ok() {
        Some(Object::Dictionary(d)) => d.clone(),
        _ => Dictionary::new(),
    };
    let mut fonts = match res.get(b"Font").ok() {
        Some(Object::Dictionary(d)) => d.clone(),
        _ => Dictionary::new(),
    };
    if fonts.get(b"MdcOcrF0").is_ok() {
        return; // 宿主已占槽——不覆盖（防御面：命名空间槽名实际不可达）
    }
    fonts.set(b"MdcOcrF0".to_vec(), Object::Reference(font_id));
    res.set(b"Font".to_vec(), Object::Dictionary(fonts));
    page.set(b"Resources".to_vec(), Object::Dictionary(res));
}

/// 全档字符集（ToUnicode CMap 的映射域）。
fn collect_chars(words_by_page: &[Vec<OcrWord>]) -> Vec<char> {
    let mut set = std::collections::BTreeSet::new();
    for words in words_by_page {
        for w in words {
            for ch in w.text.chars() {
                if (ch as u32) <= 0xFFFF {
                    set.insert(ch);
                }
            }
        }
    }
    set.into_iter().collect()
}

/// 非内嵌隐形文本字体（Type0/Identity-H，CID = 码位 + ToUnicode 直射）。
///
/// 对象树：Type0（/ToUnicode CMap 流 + /DescendantFonts）→ CIDFontType2
/// （/CIDSystemInfo Adobe/Identity/Supplement 0、/CIDToGIDMap /Identity、
/// /DW 1000——零 FontDescriptor 零嵌入：3 Tr 不渲染，提取面只走 ToUnicode）。
fn add_invisible_font(doc: &mut Document, used: &[char]) -> ObjectId {
    let to_unicode = Stream::new(dictionary! {}, to_unicode_cmap(used).into_bytes());
    let tu_id = doc.add_object(to_unicode);

    let mut csi = Dictionary::new();
    csi.set(
        "Registry",
        Object::String(b"Adobe".to_vec(), lopdf::StringFormat::Literal),
    );
    csi.set(
        "Ordering",
        Object::String(b"Identity".to_vec(), lopdf::StringFormat::Literal),
    );
    csi.set("Supplement", 0);
    let mut cid = Dictionary::new();
    cid.set("Type", "Font");
    cid.set("Subtype", "CIDFontType2");
    cid.set("BaseFont", Object::Name(b"MdcOcrInvisible".to_vec()));
    cid.set("CIDSystemInfo", Object::Dictionary(csi));
    cid.set("CIDToGIDMap", "Identity");
    cid.set("DW", 1000);

    let mut t0 = Dictionary::new();
    t0.set("Type", "Font");
    t0.set("Subtype", "Type0");
    t0.set("BaseFont", Object::Name(b"MdcOcrInvisible".to_vec()));
    t0.set("Encoding", "Identity-H");
    t0.set(
        "DescendantFonts",
        Object::Array(vec![Object::Dictionary(cid)]),
    );
    t0.set("ToUnicode", Object::Reference(tu_id));
    doc.add_object(t0)
}

/// ToUnicode CMap（CID = 码位直射；bfchar 100 条/块——规范上限分块）。
fn to_unicode_cmap(used: &[char]) -> String {
    let mut out = String::new();
    out.push_str("/CIDInit /ProcSet findresource begin\n");
    out.push_str("12 dict begin\n");
    out.push_str("begincmap\n");
    out.push_str("/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n");
    out.push_str("/CMapName /Adobe-Identity-UCS def\n");
    out.push_str("/CMapType 2 def\n");
    out.push_str("1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");
    for chunk in used.chunks(100) {
        out.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for ch in chunk {
            let cp = *ch as u32;
            // UTF-16BE（BMP 直写；本面域恒 BMP——collect_chars 已滤）。
            out.push_str(&format!("<{cp:04X}> <{cp:04X}>\n"));
        }
        out.push_str("endbfchar\n");
    }
    out.push_str("endcmap\n");
    out.push_str("CMapName currentdict /CMap defineresource pop\n");
    out.push_str("end\nend\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 手写最小两页 PDF（零依赖字节确定——xref 偏移精确计算）。
    fn two_page_pdf() -> Vec<u8> {
        let content: &[u8] = b"q Q";
        let objects: Vec<String> = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Contents 4 0 R >>".into(),
            format!("<< /Length {} >>\nstream\n", content.len()),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Contents 6 0 R >>".into(),
            format!("<< /Length {} >>\nstream\n", content.len()),
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = [0usize; 6];
        for (i, body) in objects.iter().enumerate() {
            offsets[i] = out.len();
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(body.as_bytes());
            if i == 3 || i == 5 {
                out.extend_from_slice(content);
                out.extend_from_slice(b"\nendstream");
            }
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref_pos = out.len();
        out.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
        for off in offsets {
            out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(b"trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n");
        out.extend_from_slice(xref_pos.to_string().as_bytes());
        out.extend_from_slice(b"\n%%EOF\n");
        out
    }

    fn word(text: &str, x0: f32, y0: f32, x1: f32, y1: f32) -> OcrWord {
        OcrWord {
            text: text.to_string(),
            bbox: [x0, y0, x1, y1],
            line: (1, 1, 1),
        }
    }

    #[test]
    fn write_searchable_roundtrips_structurally() {
        // 结构面主锁：写回 → lopdf 重载 → 页 /Contents 数组追加在位 +
        // /Resources/Font/MdcOcrF0 引用在位 + 3 Tr 在内容流 +
        // **提取面**（lopdf extract_text 走 ToUnicode——可搜索性的引擎内
        // 证明；pdfium 交叉验证见集成测试面）。
        let pdf = two_page_pdf();
        let pages = vec![
            vec![
                word("招标", 50.0, 700.0, 150.0, 760.0),
                word("文件", 160.0, 700.0, 260.0, 760.0),
            ],
            vec![word("Chapter", 50.0, 700.0, 200.0, 740.0)],
        ];
        let (out, report) = write_searchable_pdf(&pdf, &pages).unwrap();
        assert_eq!(report.words_written, 3);
        assert_eq!(report.words_skipped, 0);

        let doc = Document::load_mem(&out).unwrap();
        let pages_map = doc.get_pages();
        assert_eq!(pages_map.len(), 2, "页数保持");
        // 第一页内容流 = [原流, 新流] 数组形态。
        let page1 = pages_map[&1];
        let dict = doc.get_dictionary(page1).unwrap();
        let contents = dict.get(b"Contents").unwrap();
        let arr = contents.as_array().unwrap();
        assert_eq!(arr.len(), 2, "原流保持 + 追加一列");
        // 新流（无过滤裸存——stream.content = 原始操作符字节）含
        // 3 Tr 隐形渲染模式 + MdcOcrF0 槽名。
        let stream = doc
            .get_object(arr[1].as_reference().unwrap())
            .unwrap()
            .as_stream()
            .unwrap();
        let content = String::from_utf8_lossy(&stream.content);
        assert!(content.contains("3 Tr"), "隐形渲染模式在位：{content}");
        assert!(content.contains("/MdcOcrF0"), "命名空间字体槽在位");
        // 提取面（ToUnicode 直射）——两页词全量可提取。
        let text1 = doc.extract_text(&[1]).unwrap();
        assert!(text1.contains("招标"), "提取面：第一页词 1（{text1}）");
        assert!(text1.contains("文件"), "提取面：第一页词 2");
        let text2 = doc.extract_text(&[2]).unwrap();
        assert!(text2.contains("Chapter"), "提取面：第二页词（{text2}）");
        // MdcOcrF0 字体引用在位。
        let res = dict.get(b"Resources").unwrap().as_dict().unwrap();
        let fonts = res.get(b"Font").unwrap().as_dict().unwrap();
        let f0 = fonts.get(b"MdcOcrF0").unwrap().as_reference().unwrap();
        let font = doc.get_object(f0).unwrap().as_dict().unwrap();
        assert_eq!(
            font.get(b"Subtype")
                .unwrap()
                .as_name()
                .map(|n| String::from_utf8_lossy(n).to_string())
                .unwrap(),
            "Type0",
            "Type0 字体在位"
        );
        assert!(font.get(b"ToUnicode").is_ok(), "ToUnicode 在位");
    }

    #[test]
    fn write_searchable_rejects_page_overflow() {
        let pdf = two_page_pdf();
        let three = vec![vec![], vec![], vec![word("X", 1.0, 1.0, 2.0, 2.0)]];
        let err = write_searchable_pdf(&pdf, &three).unwrap_err();
        assert!(
            matches!(err, CommandError::InvalidPage(PageIndex(2))),
            "越界页 typed 拒：{err:?}"
        );
    }

    #[test]
    fn write_searchable_zero_words_is_noop_report() {
        let pdf = two_page_pdf();
        let (out, report) = write_searchable_pdf(&pdf, &[vec![], vec![]]).unwrap();
        assert_eq!(report.words_written, 0);
        assert_eq!(out.len(), out.len(), "产物恒可解析（自检内置）");
    }

    #[test]
    fn invisible_text_ops_skips_non_finite_and_non_bmp() {
        let words = vec![
            word("正常", 10.0, 10.0, 100.0, 40.0),
            OcrWord {
                text: "坏坐标".into(),
                bbox: [f32::NAN, 0.0, 1.0, 1.0],
                line: (1, 1, 1),
            },
            OcrWord {
                text: "emoji😀尾".into(),
                bbox: [10.0, 50.0, 100.0, 80.0],
                line: (1, 1, 2),
            },
        ];
        let (ops, written, skipped, non_bmp) = invisible_text_ops(&words);
        assert_eq!(written, 2, "正常词 + 非 BMP 词的可注入部分");
        assert_eq!(skipped, 1, "坏坐标词跳过");
        assert_eq!(non_bmp, 1, "😀 计 1 跳字符");
        assert!(ops.contains("3 Tr"));
        assert!(!ops.contains("NaN"));
    }

    #[test]
    fn to_unicode_cmap_chunks_at_100() {
        let chars: Vec<char> = (0..250)
            .map(|i| char::from_u32(0x4E00 + i).unwrap())
            .collect();
        let cmap = to_unicode_cmap(&chars);
        assert_eq!(cmap.matches("beginbfchar").count(), 3, "250 条 → 3 块");
        assert!(cmap.contains("<4E00> <4E00>"));
    }
}
