//! pdf_pde_batch3 —— issue #162 批③验收面：PDEFont 度量 + PDEText
//! 编辑深面 + PDEColorSpace 语义 + PDEElement 基类补齐。
//!
//! 定位与批①②同款：**验收聚合**——单元语义锁在 `pde::font` /
//! `pde::model` 内联 `#[cfg(test)]`，本文件锁 Adobe PDFEdit 语义对照面
//!（round142 语义锚档案 product-deep-dive/pde/batch3-semantic-anchors.md
//! 逐条锚）。
//!
//! 覆盖矩阵（Adobe 语义 ← 测试）：
//! | Adobe 语义 | 测试 |
//! |---|---|
//! | PDEFontCreateFromCosObj（字体字典 → PDEFont） | `font_from_cosobj_full_face` |
//! | PDEFontGetWidths（1000 EM / FirstChar 基址） | `font_widths_and_missing_fallback` |
//! | PDEFontGetNumCodeBytes（CID 多字节探测） | `font_cid_code_bytes` |
//! | PDEFontGetCreateNeedFlags（惰性完成度） | `font_create_need_flags` |
//! | cantEmbed 嵌入禁令（安全语义） | `font_cant_embed_security` |
//! | GetRunForChar/RunGetCharOffset/GetNumChars（双寻址闭环） | `char_addressing_roundtrip` |
//! | PDETextReplaceChars（字符数守恒——Adobe 原文） | `replace_chars_conservation_face` |
//! | PDETextSplitRunAt（splitLoc 全局索引——官方原文） | `split_run_at_global_face` |
//! | PDETextRemove（char 全局区间） | `remove_chars_global_face` |
//! | PDETextGetAdvance/GetBBox（度量 × 矩阵） | `text_advance_bbox_metric_face` |
//! | PDEElementIsAtPoint/IsAtRect（命中） | `element_hit_test_face` |
//! | PDEElementCopy | `element_deep_copy_face` |
//! | PDEColorSpaceGetNumComps | `colorspace_num_comps_face` |
//! | Indexed hiVal+1 冗余 | `indexed_hival_consistency_face` |
//! | 解析→编辑→写回全链 | `parse_edit_write_roundtrip` |

use mdc_pdf::pde::font::{FontType, PdeFont};
use mdc_pdf::pde::model::*;
use mdc_pdf::pde::parser::parse_content;
use mdc_pdf::pde::resources::ResValue;

fn font_descriptor_dict() -> Vec<(Vec<u8>, ResValue)> {
    vec![
        (b"Ascent".to_vec(), ResValue::Int(722)),
        (b"Descent".to_vec(), ResValue::Int(-222)),
        (b"MissingWidth".to_vec(), ResValue::Int(500)),
        (b"StemV".to_vec(), ResValue::Int(80)),
        (b"CapHeight".to_vec(), ResValue::Int(662)),
    ]
}

fn simple_type1() -> ResValue {
    ResValue::Dict(vec![
        (b"Type".to_vec(), ResValue::Name(b"Font".to_vec())),
        (b"Subtype".to_vec(), ResValue::Name(b"Type1".to_vec())),
        (
            b"BaseFont".to_vec(),
            ResValue::Name(b"Times-Roman".to_vec()),
        ),
        (b"FirstChar".to_vec(), ResValue::Int(32)),
        (b"LastChar".to_vec(), ResValue::Int(34)),
        (
            b"Widths".to_vec(),
            ResValue::Array(vec![
                ResValue::Int(250),
                ResValue::Int(500),
                ResValue::Int(750),
            ]),
        ),
        (
            b"FontDescriptor".to_vec(),
            ResValue::Dict(font_descriptor_dict()),
        ),
    ])
}

/// PDEFontCreateFromCosObj 全面：Attrs 字段级 + 度量 + 完成度。
#[test]
fn font_from_cosobj_full_face() {
    let f = PdeFont::from_res_value(&simple_type1()).unwrap();
    assert_eq!(f.attrs.name, b"Times-Roman");
    assert_eq!(f.attrs.font_type, FontType::Type1);
    assert_eq!(f.attrs.ascent, Some(722.0));
    assert_eq!(f.attrs.descent, Some(222.0)); // 取绝对值语义
    assert_eq!(f.attrs.cap_height, Some(662.0));
    assert_eq!(f.attrs.missing_width, Some(500.0));
    assert!(!f.is_embedded);
    assert!(!f.attrs.cant_embed);
    // 惰性完成度三旗标
    let need = f.create_need_flags();
    assert!(!need.need_widths, "/Widths 已就绪");
    assert!(need.need_to_unicode, "无 /ToUnicode");
    assert!(need.need_embed, "无 FontFile*");
}

/// GetWidths：1000 EM 域 + FirstChar 基址 + notdef 兜底链。
#[test]
fn font_widths_and_missing_fallback() {
    let f = PdeFont::from_res_value(&simple_type1()).unwrap();
    // Widths 域（FirstChar=32 基址）
    assert_eq!(f.width_of(32), 250.0);
    assert_eq!(f.width_of(33), 500.0);
    assert_eq!(f.width_of(34), 750.0);
    // 缺条目 → MissingWidth（notdef）兜底
    assert_eq!(f.width_of(35), 500.0);
    // 无 descriptor 的字体：PDF 规范缺省 250 兜底
    let bare = ResValue::Dict(vec![
        (b"Subtype".to_vec(), ResValue::Name(b"Type1".to_vec())),
        (b"BaseFont".to_vec(), ResValue::Name(b"Bare".to_vec())),
    ]);
    let fb = PdeFont::from_res_value(&bare).unwrap();
    assert_eq!(fb.width_of(65), 250.0);
    assert!(fb.create_need_flags().need_widths);
}

/// GetNumCodeBytes：Type0 双字节 / 简单字体单字节。
#[test]
fn font_cid_code_bytes() {
    let f = PdeFont::from_res_value(&simple_type1()).unwrap();
    assert_eq!(f.num_code_bytes(b"A"), Some(1));
    // Type0（无 descendant——保形降级仍是 2）
    let t0 = ResValue::Dict(vec![
        (b"Subtype".to_vec(), ResValue::Name(b"Type0".to_vec())),
        (b"BaseFont".to_vec(), ResValue::Name(b"Cid".to_vec())),
    ]);
    let fc = PdeFont::from_res_value(&t0).unwrap();
    // 返工（Lead 指认）：Type0 无 /Encoding = 未知 CMap → None
    //（不再恒 2 返回错误成功值）
    assert_eq!(fc.num_code_bytes(b"AB"), None, "缺 Encoding 拒绝不猜");
    assert!(fc.attrs.font_type.is_composite());
    // Identity-H（横排 WMode 0）= Some(2)——支持面
    let ih = ResValue::Dict(vec![
        (b"Subtype".to_vec(), ResValue::Name(b"Type0".to_vec())),
        (b"BaseFont".to_vec(), ResValue::Name(b"Cid".to_vec())),
        (b"Encoding".to_vec(), ResValue::Name(b"Identity-H".to_vec())),
    ]);
    let fi = PdeFont::from_res_value(&ih).unwrap();
    assert_eq!(fi.num_code_bytes(b"AB"), Some(2), "Identity-H 双字节");
    // Identity-V（垂直 WMode 1）→ None（拒绝）
    let iv = ResValue::Dict(vec![
        (b"Subtype".to_vec(), ResValue::Name(b"Type0".to_vec())),
        (b"Encoding".to_vec(), ResValue::Name(b"Identity-V".to_vec())),
    ]);
    let fv = PdeFont::from_res_value(&iv).unwrap();
    assert_eq!(fv.num_code_bytes(b"AB"), None, "垂直布局拒绝");
    // 预定义 CMap 名（非 Identity）→ None（CMap 表未承载不猜）
    let gb = ResValue::Dict(vec![
        (b"Subtype".to_vec(), ResValue::Name(b"Type0".to_vec())),
        (
            b"Encoding".to_vec(),
            ResValue::Name(b"UniGB-UCS2-H".to_vec()),
        ),
    ]);
    let fg = PdeFont::from_res_value(&gb).unwrap();
    assert_eq!(fg.num_code_bytes(b"AB"), None, "未知预定义 CMap 拒绝");
    // 未知名保形
    let unk = ResValue::Dict(vec![(
        b"Subtype".to_vec(),
        ResValue::Name(b"OpenType".to_vec()),
    )]);
    let fu = PdeFont::from_res_value(&unk).unwrap();
    assert_eq!(fu.attrs.font_type, FontType::Other(b"OpenType".to_vec()));
}

/// cantEmbed（FontDescriptor /Flags bit 2）安全语义 + IsEmbedded。
#[test]
fn font_cant_embed_security() {
    let fd = vec![
        (b"Flags".to_vec(), ResValue::Int(4)), // bit 2 = 嵌入禁令
        (
            b"FontFile2".to_vec(),
            ResValue::Stream {
                dict: vec![],
                data: vec![0x00, 0x01],
            },
        ),
    ];
    let v = ResValue::Dict(vec![
        (b"Subtype".to_vec(), ResValue::Name(b"TrueType".to_vec())),
        (b"BaseFont".to_vec(), ResValue::Name(b"Locked".to_vec())),
        (b"FontDescriptor".to_vec(), ResValue::Dict(fd)),
    ]);
    let f = PdeFont::from_res_value(&v).unwrap();
    assert!(
        f.attrs.cant_embed,
        "Flags bit2 = 嵌入禁令（写回嵌入面拒绝）"
    );
    assert!(f.is_embedded, "FontFile2 在 = in-document 嵌入");
    // bit 2 未设：无禁令
    let fd2 = vec![(b"Flags".to_vec(), ResValue::Int(32))];
    let v2 = ResValue::Dict(vec![
        (b"Subtype".to_vec(), ResValue::Name(b"Type1".to_vec())),
        (b"FontDescriptor".to_vec(), ResValue::Dict(fd2)),
    ]);
    assert!(!PdeFont::from_res_value(&v2).unwrap().attrs.cant_embed);
}

/// 双寻址三方法闭环（GetRunForChar / RunGetCharOffset / GetNumChars）。
#[test]
fn char_addressing_roundtrip() {
    // 解析真内容流（BT..ET 三 run 断链：Tm 变化）
    let stream = b"BT /F1 12 Tf 1 0 0 1 10 20 Tm (AB) Tj 1 0 0 1 40 20 Tm (CDE) Tj 1 0 0 1 80 20 Tm (FGHI) Tj ET";
    let content = parse_content(stream).unwrap();
    match content.elements.first() {
        Some(PdeElement::Text(t)) => {
            assert_eq!(t.num_runs(), 3, "Tm 断链 = 3 run");
            assert_eq!(t.num_chars(), 9);
            // 闭环换算
            for c in 0..9 {
                let run = t.run_for_char(c).unwrap();
                let off = t.run_char_offset(run).unwrap();
                assert!(
                    c >= off && c < off + t.run_num_chars(run).unwrap(),
                    "闭环失败 char={c} run={run}"
                );
            }
            assert_eq!(t.run_char_offset(0), Some(0));
            assert_eq!(t.run_char_offset(1), Some(2));
            assert_eq!(t.run_char_offset(2), Some(5));
        }
        _ => panic!("应为文本元素"),
    }
}

/// ReplaceChars 守恒语义钉（Adobe "extra characters are ignored"）。
#[test]
fn replace_chars_conservation_face() {
    let stream = b"BT /F1 12 Tf 1 0 0 1 10 20 Tm (AB) Tj ET";
    let mut content = parse_content(stream).unwrap();
    if let Some(PdeElement::Text(t)) = content.elements.first_mut() {
        let before = t.num_chars();
        t.replace_chars(0, b"XYZ12345678");
        assert_eq!(t.num_chars(), before, "长替换多余忽略——字符数守恒");
        assert_eq!(t.run_text(0).unwrap(), b"XY".to_vec());
        // 跨 run 替换（全局坐标）
        let stream2 = b"BT /F1 12 Tf (AB) Tj 1 0 0 1 40 20 Tm (CDE) Tj ET";
        let mut c2 = parse_content(stream2).unwrap();
        if let Some(PdeElement::Text(t2)) = c2.elements.first_mut() {
            t2.replace_chars(1, b"q"); // 全局 1 = B 位
            assert_eq!(t2.run_text(0).unwrap(), b"Aq".to_vec());
            assert_eq!(t2.num_chars(), 5);
        }
    }
}

/// SplitRunAt 全局索引语义钉（"relative to the text object"）。
#[test]
fn split_run_at_global_face() {
    // run 局部索引反例：全局 3 落 run1（"CDE"）内部位置 1——
    // run 局部语义会把 3 当 run1 内越界（无操作）——判别面。
    let stream = b"BT /F1 12 Tf (AB) Tj 1 0 0 1 40 20 Tm (CDE) Tj ET";
    let mut content = parse_content(stream).unwrap();
    let split_font = PdeFont::from_res_value(&simple_type1()).unwrap();
    if let Some(PdeElement::Text(t)) = content.elements.first_mut() {
        t.split_run_at(3, Some(&split_font));
        assert_eq!(
            t.num_runs(),
            3,
            "全局 3 → run1 断成 CD|E（首段含 splitLoc 位）"
        );
        assert_eq!(t.run_text(1).unwrap(), b"CD".to_vec());
        assert_eq!(t.run_text(2).unwrap(), b"E".to_vec());
        // 写回仍是合法内容流
        let out = content.write();
        assert!(String::from_utf8_lossy(&out).contains("(CD)"));
    }
}

/// Remove（char 全局区间）+ run 删除。
#[test]
fn remove_chars_global_face() {
    let stream = b"BT /F1 12 Tf (AB) Tj 1 0 0 1 40 20 Tm (CDE) Tj 1 0 0 1 80 20 Tm (FG) Tj ET";
    let mut content = parse_content(stream).unwrap();
    if let Some(PdeElement::Text(t)) = content.elements.first_mut() {
        // 删全局 1..4（B、C、D——跨三个 run 的原始坐标区间）
        t.remove_chars(1, 3);
        assert_eq!(t.num_chars(), 4);
        assert_eq!(t.run_text(0).unwrap(), b"A".to_vec());
        assert_eq!(t.run_text(1).unwrap(), b"E".to_vec());
        assert_eq!(t.run_text(2).unwrap(), b"FG".to_vec());
        // 整 run 清空 → run 移除
        t.remove_chars(0, 1); // 删 A
        assert_eq!(t.num_runs(), 2, "run0 清空移除");
    }
}

/// 度量面：advance（1000 EM × fontSize × Tz）+ bbox（ascent/descent
/// 基线系 × Tm×CTM 落页）。
#[test]
fn text_advance_bbox_metric_face() {
    // 字体：Ascent 722 / Descent -222 / Widths(32..33)=250,500
    let font = PdeFont::from_res_value(&simple_type1()).unwrap();
    // 内容流：fontSize 10、Tz 缺省 100、" "（code 32 → 250 EM）
    let stream = b"BT /F1 10 Tf 1 0 0 1 10 20 Tm ( ) Tj ET";
    let content = parse_content(stream).unwrap();
    match content.elements.first() {
        Some(PdeElement::Text(t)) => {
            let adv = t.run_advance(0, Some(&font)).unwrap();
            // 250 EM × 10/1000 × 100/100 = 2.5
            assert!((adv - 2.5).abs() < 1e-4, "advance = {adv}（应 2.5）");
            // bbox：基线 y=20（Tm 位移）， ascent 顶 = 20 + 722×10/1000
            let bb = t.run_bbox(0, Some(&font)).unwrap();
            assert!((bb.y1() - 27.22).abs() < 1e-3, "bbox 顶 = {}", bb.y1());
            assert!((bb.y0() - 17.78).abs() < 1e-3, "bbox 底 = {}", bb.y0());
            assert!((bb.x1() - 12.5).abs() < 1e-3, "bbox 右 = {}", bb.x1());
            // 无字体上下文 = None（诚实降级）
            assert!(t.run_advance(0, None).is_none());
            assert!(t.run_bbox(0, None).is_none());
        }
        _ => panic!("应为文本元素"),
    }
}

/// IsAtPoint / IsAtRect 命中（选区产品闭环）+ 无 BBox 面 false。
#[test]
fn element_hit_test_face() {
    let stream = b"1 0 0 1 10 20 cm 0 0 100 50 re f";
    let content = parse_content(stream).unwrap();
    let elem = content.elements.first().unwrap();
    assert!(elem.is_at_point(50.0, 30.0), "矩形中心命中");
    assert!(!elem.is_at_point(200.0, 300.0), "远点不命中");
    assert!(elem.is_at_rect(0.0, 0.0, 20.0, 25.0), "左下角块相交");
    assert!(!elem.is_at_rect(200.0, 200.0, 250.0, 250.0), "远块不相交");
    // Container（无 gstate 面）：bbox 无 → false
    let cstream = b"/P0 << /Metadata 0 >> BDC 1 0 0 1 10 20 cm 0 0 100 50 re f EMC";
    let ccontent = parse_content(cstream).unwrap();
    if let Some(PdeElement::Container(_)) = ccontent.elements.first() {
        // Container 元素本身（整块）无 bbox 面：批③语义钉 = false
        assert!(!ccontent.elements.first().unwrap().is_at_point(50.0, 30.0));
    }
    // deep_copy：拷贝等价
    let copy = elem.deep_copy();
    assert_eq!(copy, *elem);
}

/// num_comps（GetNumComps）+ Indexed hiVal+1 冗余。
#[test]
fn colorspace_num_comps_face() {
    let g = ColorSpec::device_gray(0.5);
    assert_eq!(g.num_comps(), Some(1));
    let mut rgb = g.clone();
    rgb.space = b"DeviceRGB".to_vec();
    assert_eq!(rgb.num_comps(), Some(3));
    let mut cmyk = g.clone();
    cmyk.space = b"DeviceCMYK".to_vec();
    assert_eq!(cmyk.num_comps(), Some(4));
    // Pattern = None（Adobe 官方边界）
    let mut pat = g.clone();
    pat.space = b"Pattern".to_vec();
    assert_eq!(pat.num_comps(), None);
    // 资源名 = None（表驱动归资源层）
    let mut cs0 = g.clone();
    cs0.space = b"CS0".to_vec();
    assert_eq!(cs0.num_comps(), None);
}

/// Indexed：hiVal+1 冗余检查（hiVal 语义钉）。
#[test]
fn indexed_hival_consistency_face() {
    let idx = IndexedColorSpace {
        base: b"DeviceRGB".to_vec(),
        hi_val: 7,
        // (7+1)×3 = 24 字节恰好
        lookup: vec![0u8; 24],
    };
    assert!(idx.lookup_is_consistent(), "24 字节 = (hiVal+1)×3 恰好");
    let short = IndexedColorSpace {
        base: b"DeviceRGB".to_vec(),
        hi_val: 7,
        lookup: vec![0u8; 23],
    };
    assert!(!short.lookup_is_consistent(), "23 字节 < 24：损坏表");
    assert_eq!(idx.base_num_comps(), Some(3));
}

/// 全链：解析 → 编辑（replace/split）→ 写回 → 再解析（结构守恒）。
#[test]
fn parse_edit_write_roundtrip() {
    let stream = b"BT /F1 12 Tf 1 0 0 1 10 20 Tm (Hello) Tj ET";
    let mut content = parse_content(stream).unwrap();
    let rt_font = PdeFont::from_res_value(&simple_type1()).unwrap();
    if let Some(PdeElement::Text(t)) = content.elements.first_mut() {
        t.replace_chars(0, b"Jello"); // H→J（Adobe 编辑最经典路径）
                                      // #194：split 带字体度量（位置进位 Tm——写回后次段不重叠）；
                                      // 字节超出 simple_type1 宽度域走 missing 250/500 缺省（确定值非猜）。
        let rt_font = PdeFont::from_res_value(&simple_type1()).unwrap();
        t.split_run_at(2, Some(&rt_font)); // 全局 2 = "l" 位（首段含 l → Je|llo）
    }
    let out = content.write();
    let s = String::from_utf8_lossy(&out);
    // split 全局 2（含首段）→ Jel|lo 两 run
    assert!(s.contains("(Jel)"), "写回含编辑后文本首段：{s}");
    assert!(s.contains("(lo)"), "写回含编辑后文本次段：{s}");
    // 再解析：字符数守恒；**#194 位置保真后**次 run Tm 已进位（首段
    // advance）→ Adobe 合并判据（Tm 相同才合并）**不再合并**——两 run
    // 保持、各字符绘制位置与未拆分形态一致（旧「合并回单 run」断言
    // 正是次段 Tm 复制原值的重叠病灶面——返工同步为更强形态）。
    let reparsed = parse_content(&out).unwrap();
    if let Some(PdeElement::Text(t)) = reparsed.elements.first() {
        assert_eq!(t.num_chars(), 5, "字符数守恒跨写回");
        assert_eq!(t.num_runs(), 2, "进位 Tm 的两 run 不再合并（位置保真）");
        assert_eq!(t.run_text(0).unwrap(), b"Jel".to_vec());
        assert_eq!(t.run_text(1).unwrap(), b"lo".to_vec());
        // 次段进位核对：J/e/l 码值 74/101/108 均超 simple_type1 的
        // /Widths 域（32..34）→ 走 MissingWidth 500：3×500×12/1000=18
        // → 次 run Tm.e = 10 + 18 = 28（进位——缺省确定值非猜）。
        let e2 = t.runs[1].text_matrix[4];
        assert!(
            (e2 - 28.0).abs() < 1e-3,
            "次段 Tm.e = 10 + 3×500×12/1000 = 28（进位），实际 {e2}"
        );
    }
}
