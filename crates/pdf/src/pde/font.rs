//! pde::font —— PDEFont 建模（issue #162 批③，Adobe Acrobat SDK
//! PDFEdit/PDE 参照）。
//!
//! Adobe 语义锚：`_t_PDEFontAttrs`（PEExpT.h:973——字体字典的字段级
//! 建模）+ 度量方法面（GetWidths / GetNumCodeBytes）+ 惰性完成度模型
//!（GetCreateNeedFlags——读面/写面分离）。证据等级 A（官方公开参考
//! 页，round142 语义锚档案 product-deep-dive/pde/batch3-semantic-anchors.md
//! §二）。
//!
//! # 与批②的关系
//!
//! 批② `ResourceDict::font` 只做名字存在性探针（「语义面批③
//! PDEFont」登记）；本模块补值级：字体字典 `ResValue` → [`PdeFont`]
//! （`PDEFontCreateFromCosObj` 同位——ResValue 路径批②已铺）。
//!
//! # 诚实边界（round142 档案 §六）
//!
//! - PDEFontMetrics 类无独立参考页（404）——度量面在本模块 widths/
//!   attrs 内，不臆造独立类型。
//! - CreateFromSysFont 族 11 方法（系统字体路径）不消费——mdcore 无
//!   sysfont 面，登记不展开。
//! - 嵌入面（EmbedNow/SubsetNow）只建旗标不建字节面——字体流写回
//!   归引擎挂接层。

use super::resources::ResValue;

/// PDEFont 类型（PDF 字典 `/Subtype`——PEExpT.h:973「type」字段）。
///
/// Adobe 原文："It may be: 'Type1' 'TrueType' 'MMType1' 'Type0'"。
#[derive(Debug, Clone, PartialEq)]
pub enum FontType {
    Type1,
    TrueType,
    /// Multiple Master Type1（Type1 变体）。
    MmType1,
    /// Type0（合成字体——CID 键控，`/DescendantFonts` 下挂 CIDFont）。
    Type0,
    /// 未知 Subtype（保形承载——typed 拒绝面归解析入口）。
    Other(Vec<u8>),
}

impl FontType {
    /// Subtype 名 → 枚举（宽容面：未知名保形 Other——解析不因新
    /// Subtype 失败）。
    pub fn from_name(name: &[u8]) -> FontType {
        match name {
            b"Type1" => FontType::Type1,
            b"TrueType" => FontType::TrueType,
            b"MMType1" => FontType::MmType1,
            b"Type0" => FontType::Type0,
            other => FontType::Other(other.to_vec()),
        }
    }

    /// Type0 = 合成字体（多字节码探测的分流判据）。
    pub fn is_composite(&self) -> bool {
        matches!(self, FontType::Type0)
    }
}

/// 字体度量（1000 EM 单位——PEExpT.h:973 fontBBox「in 1000 EM units」
/// 语义钉：与 PDF 裸坐标不同，Widths/FontDescriptor 数值域是千分比
/// 字身）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FontBBox {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

/// `_t_PDEFontAttrs`（PEExpT.h:973）——批③建模主对照。
///
/// 字段覆盖：name/type/encoding/flags/fontBBox/missingWidth/stemV/
/// stemH/capHeight/xHeight/ascent/descent/leading/maxWidth/avgWidth/
/// italicAngle/cid 系（cidFontType/wMode/registry/ordering/supplement）/
/// psName/platformName/cantEmbed/protection。
#[derive(Debug, Clone, PartialEq)]
pub struct FontAttrs {
    /// `/BaseFont`（如 Times-Roman）。
    pub name: Vec<u8>,
    /// `/Subtype` 分流。
    pub font_type: FontType,
    /// 简单字体 `/Encoding`（名字或字典——字典差异编码面保形）。
    pub encoding: Option<Vec<u8>>,
    /// `/FontDescriptor /Flags`。
    pub flags: Option<i64>,
    /// font bbox（**1000 EM**——非 PDF 裸坐标）。
    pub font_bbox: Option<FontBBox>,
    /// 缺字宽度（.notdef——Adobe「width of the missing character」；
    /// 无 `/Widths` 条目时的兜底）。
    pub missing_width: Option<f32>,
    pub stem_v: Option<f32>,
    pub stem_h: Option<f32>,
    pub cap_height: Option<f32>,
    pub x_height: Option<f32>,
    /// 最大上升高度（基线系——文本 BBox 闭合的关键输入）。
    pub ascent: Option<f32>,
    /// 最大下降深度（正数语义与 PDF 相反——FontDescriptor /Descent
    /// 为负值域，此处取绝对值存储，换算面统一）。
    pub descent: Option<f32>,
    pub leading: Option<f32>,
    pub max_width: Option<f32>,
    pub avg_width: Option<f32>,
    /// 斜体角（度）。
    pub italic_angle: Option<f32>,
    /// CID 系：/W 所属的 CIDFontType（0/2）。
    pub cid_font_type: Option<i64>,
    /// 书写方向（0 横 / 1 竖）。
    pub w_mode: Option<i64>,
    /// CID 系三件套（Adobe-Japan1-6 等）。
    pub registry: Option<Vec<u8>>,
    pub ordering: Option<Vec<u8>>,
    pub supplement: Option<i64>,
    pub ps_name: Option<Vec<u8>>,
    /// 嵌入禁令（FontDescriptor /Flags bit 2——**安全语义：禁止嵌入的
    /// 字体不得写回嵌入面**，PEExpT.h cantEmbed「protection」字段）。
    pub cant_embed: bool,
}

/// 宽度表（简单字体 `/Widths` / Type0 `/W` 的统一读面）。
///
/// - 简单字体：`/Widths[first_char..last_char]` 连续数组（1000 EM）。
/// - Type0：`/W` 数组的 `[c [w1 w2..]]` 或 `[c1 c2 w]` 区间形态。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Widths {
    /// 简单字体面：code → 宽度（连续数组直装）。
    simple: Vec<Option<f32>>,
    /// 简单字体 /FirstChar（simple[0] 的码点基址）。
    first_char: i64,
    /// Type0 面：CID → 宽度（/W 区间展开装）。
    cid: std::collections::BTreeMap<u32, f32>,
}

impl Widths {
    /// 码点宽度（1000 EM；缺条目 = None → 调用方走 missing_width 兜底）。
    pub fn width_of(&self, code: u32) -> Option<f32> {
        if self.cid.is_empty() {
            let idx = code as i64 - self.first_char;
            if idx >= 0 && (idx as usize) < self.simple.len() {
                return self.simple[idx as usize];
            }
            None
        } else {
            self.cid.get(&code).copied()
        }
    }

    /// 是否有 CID 面（Type0 /W 路径的判定）。
    pub fn is_cid(&self) -> bool {
        !self.cid.is_empty()
    }
}

/// 惰性完成度旗标（PEExpT.h——`PDEFontGetCreateNeedFlags` 的
/// 「what needs to be done to make PDEFont complete」语义）。
///
/// kPDEFontCreateNeedWidths：`/Widths` 缺失（读面未就绪——
/// `width_of` 全走 missing_width 兜底）。
/// kPDEFontCreateNeedToUnicode：`/ToUnicode` 缺失（提取文本面
/// 未就绪——批③登记不实现，旗标仅反映状态）。
/// kPDEFontCreateNeedEmbed：未嵌入（写回嵌入面未就绪）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CreateNeedFlags {
    pub need_widths: bool,
    pub need_to_unicode: bool,
    pub need_embed: bool,
}

/// PDEFont（Adobe `PDEFont`——`PDEFontCreateFromCosObj` 的 ResValue 路）。
#[derive(Debug, Clone, PartialEq)]
pub struct PdeFont {
    pub attrs: FontAttrs,
    pub widths: Widths,
    /// `/ToUnicode` 存在性（CMap 流——语义面后续批次，存在性先行）。
    pub has_to_unicode: bool,
    /// `PDEFontIsEmbedded`（in-document 语义：字典带 FontFile*）。
    pub is_embedded: bool,
    /// Type0 的 descendant（CIDFont 字典——批③保形单层，嵌套 CIDFont
    /// 的 /W 读面在 `widths.cid`——descendant 自身的 FontDescriptor
    /// 度量并入本层 attrs）。
    pub descendant: Option<Box<ResValue>>,
}

impl PdeFont {
    /// 码点宽度（1000 EM，缺条目走 `missing_width` 兜底——Adobe
    /// 「width of the missing character」语义：notdef 宽度是**最终**
    /// 兜底，再缺省 PDF 规范缺省 250）。
    pub fn width_of(&self, code: u32) -> f32 {
        self.widths
            .width_of(code)
            .or(self.attrs.missing_width)
            .unwrap_or(250.0)
    }

    /// `PDEFontGetNumCodeBytes`——多字节码长探测（**返工 2026-11-14
    /// 改 `Option<usize>` 三态**——Lead 复验：恒 2 对未知 CMap 返回
    /// 错误成功值）。
    ///
    /// Adobe 语义："Gets the number of bytes comprising the next code
    /// in a string of single or multi-byte character codes"。
    ///
    /// 支持面（**明确仅 Identity-H**——度量真源的地基）：
    /// - 简单字体（Type1/TrueType/MMType1）= `Some(1)`（单字节码）；
    /// - Type0 + `/Encoding` = `Identity-H`（横排，WMode 0/缺省）=
    ///   `Some(2)`（2 字节 CID 大端）；
    /// - **其余一律 `None`**（明确缺能力，调用方不得当 2 字节 CID
    ///   使用）：未知/预定义 CMap 名（UniGB-UCS2-H 等 2 字节流我们
    ///   未承载其 CMap 表——码长≠CID 语义可通约，但 CMap 内容未解
    ///   析即不可靠）、`Identity-V`（垂直——WMode 1，W 索引语义不
    ///   同）、`/Encoding` 缺失或非名形态（CMap 流形态）。返回
    ///   错误成功值比缺能力更糟（#195 原判）。
    pub fn num_code_bytes(&self, _text: &[u8]) -> Option<usize> {
        if self.attrs.font_type.is_composite() {
            // 垂直布局（WMode 1 / Identity-V）明确拒绝。
            if self.attrs.w_mode == Some(1) {
                return None;
            }
            match self.attrs.encoding.as_deref() {
                Some(b"Identity-H") => Some(2),
                _ => None, // 未知 CMap / 缺 Encoding / Identity-V——不猜
            }
        } else {
            Some(1)
        }
    }

    /// 码长可判别的显示串（整串均为完整码——`None` = 未知 CMap/含
    /// 残码；度量与编辑面的共用前置（#195/#194 真源统一））。
    pub fn code_len_of(&self, text: &[u8]) -> Option<usize> {
        let len = self.num_code_bytes(text)?;
        if text.len() % len == 0 {
            Some(len)
        } else {
            None // 残码（长非码长整数倍）——不猜
        }
    }

    /// `PDEFontGetCreateNeedFlags`（惰性完成度——读面/写面分离模型）。
    pub fn create_need_flags(&self) -> CreateNeedFlags {
        CreateNeedFlags {
            need_widths: self.widths.simple.is_empty() && self.widths.cid.is_empty(),
            need_to_unicode: !self.has_to_unicode,
            need_embed: !self.is_embedded,
        }
    }

    /// `PDEFontCreateFromCosObj`（ResValue 路——批②资源字典 Font 表
    /// 的值级升级）。
    ///
    /// 非 Dict 形态 = None（调用方走名字探针回退——批②存在性面）。
    pub fn from_res_value(v: &ResValue) -> Option<PdeFont> {
        let dict = match v {
            ResValue::Stream { dict, .. } => dict,
            ResValue::Dict(pairs) => pairs,
            _ => return None,
        };
        let get = |k: &[u8]| dict_get(dict, k);
        let get_name = |k: &[u8]| match get(k) {
            Some(ResValue::Name(n)) => Some(n.clone()),
            _ => None,
        };

        let font_type = get_name(b"Subtype")
            .map(|n| FontType::from_name(&n))
            .unwrap_or(FontType::Other(Vec::new()));
        // Type0：descendant 保形 + 其 /W 读面；简单字体直接本体字典。
        let descendant: Option<Box<ResValue>> = if font_type.is_composite() {
            match get(b"DescendantFonts") {
                Some(ResValue::Array(items)) if !items.is_empty() => {
                    Some(Box::new(items[0].clone()))
                }
                _ => None,
            }
        } else {
            None
        };
        // FontDescriptor：本体直查；Type0 时 descendant 字典内查。
        let fd: &[(Vec<u8>, ResValue)] = match get(b"FontDescriptor") {
            Some(ResValue::Dict(fd)) => fd,
            Some(ResValue::Stream { dict: fd, .. }) => fd,
            _ => match descendant.as_deref() {
                Some(ResValue::Dict(d)) | Some(ResValue::Stream { dict: d, .. }) => {
                    match dict_get(d, b"FontDescriptor") {
                        Some(ResValue::Dict(fd)) => fd,
                        Some(ResValue::Stream { dict: fd, .. }) => fd,
                        _ => &[],
                    }
                }
                _ => &[],
            },
        };
        let fd_get = |k: &[u8]| dict_get(fd, k);
        let fd_num = |k: &[u8]| match fd_get(k) {
            Some(ResValue::Int(i)) => Some(*i as f32),
            Some(ResValue::Real(r)) => Some(*r),
            _ => None,
        };

        let flags = match fd_get(b"Flags") {
            Some(ResValue::Int(i)) => Some(*i),
            _ => None,
        };
        // FontDescriptor /Flags bit 2 (value 4) = 嵌入禁令（Adobe
        // 「must be embedded」位取反语义——cantEmbed 字段位）。
        let cant_embed = flags.is_some_and(|f| f & 0x4 != 0);

        let font_bbox = get(b"FontBBox").map(bbox_from);
        let missing_width = fd_num(b"MissingWidth");

        let widths = read_widths(dict, &font_type, descendant.as_deref());
        // descendant 字典（CID 系字段读面）。
        let dd = match descendant.as_deref() {
            Some(ResValue::Dict(d)) | Some(ResValue::Stream { dict: d, .. }) => d,
            _ => dict,
        };

        let has_to_unicode = get(b"ToUnicode").is_some();
        let is_embedded = [b"FontFile" as &[u8], b"FontFile2", b"FontFile3"]
            .iter()
            .any(|k| fd_get(k).is_some());

        Some(PdeFont {
            attrs: FontAttrs {
                name: get_name(b"BaseFont").unwrap_or_default(),
                font_type,
                encoding: get_name(b"Encoding"),
                flags,
                font_bbox,
                missing_width,
                stem_v: fd_num(b"StemV"),
                stem_h: fd_num(b"StemH"),
                cap_height: fd_num(b"CapHeight"),
                x_height: fd_num(b"XHeight"),
                ascent: fd_num(b"Ascent"),
                descent: fd_num(b"Descent").map(f32::abs),
                leading: fd_num(b"Leading"),
                max_width: fd_num(b"MaxWidth"),
                avg_width: fd_num(b"AvgWidth"),
                italic_angle: fd_num(b"ItalicAngle"),
                cid_font_type: match dict_get(dd, b"Subtype") {
                    Some(ResValue::Name(n)) if n == b"CIDFontType0" || n == b"CIDFontType2" => {
                        Some(if n == b"CIDFontType0" { 0 } else { 2 })
                    }
                    _ => None,
                },
                w_mode: match dict_get(dd, b"WMode") {
                    Some(ResValue::Int(i)) => Some(*i),
                    _ => None,
                },
                registry: registry_of(descendant.as_deref()),
                ordering: ordering_of(descendant.as_deref()),
                supplement: supplement_of(descendant.as_deref()),
                ps_name: get_name(b"BaseFont"),
                cant_embed,
            },
            widths,
            has_to_unicode,
            is_embedded,
            descendant,
        })
    }
}

// ---- 辅助读面 ----

fn dict_get<'a>(dict: &'a [(Vec<u8>, ResValue)], k: &[u8]) -> Option<&'a ResValue> {
    dict.iter().find(|(key, _)| key == k).map(|(_, v)| v)
}

fn bbox_from(v: &ResValue) -> FontBBox {
    if let ResValue::Array(items) = v {
        let nums: Vec<f32> = items
            .iter()
            .filter_map(|i| match i {
                ResValue::Int(n) => Some(*n as f32),
                ResValue::Real(r) => Some(*r),
                _ => None,
            })
            .collect();
        if nums.len() == 4 {
            return FontBBox {
                x1: nums[0],
                y1: nums[1],
                x2: nums[2],
                y2: nums[3],
            };
        }
    }
    FontBBox::default()
}

/// `/Widths`（简单字体）+ `/W`（Type0 descendant）读面。
fn read_widths(
    dict: &[(Vec<u8>, ResValue)],
    font_type: &FontType,
    descendant: Option<&ResValue>,
) -> Widths {
    let mut w = Widths::default();
    // 简单字体 /Widths：/FirstChar 基址连续数组
    if let Some(ResValue::Array(items)) = dict_get(dict, b"Widths") {
        let first = match dict_get(dict, b"FirstChar") {
            Some(ResValue::Int(i)) => *i,
            _ => 0,
        };
        w.simple = items
            .iter()
            .map(|i| match i {
                ResValue::Int(n) => Some(*n as f32),
                ResValue::Real(r) => Some(*r),
                _ => None,
            })
            .collect();
        w.first_char = first;
        if !font_type.is_composite() {
            return w;
        }
    }
    // Type0 /W（descendant 字典）：区间/数组两形态
    let d_dict = match descendant {
        Some(ResValue::Stream { dict, .. }) | Some(ResValue::Dict(dict)) => dict,
        _ => return w,
    };
    if let Some(ResValue::Array(items)) = dict_get(d_dict, b"W") {
        let mut i = 0;
        while i < items.len() {
            let c = match &items[i] {
                ResValue::Int(n) => *n as u32,
                _ => break,
            };
            match items.get(i + 1) {
                // [c [w1 w2 ...]] 形态：c 起连续逐 CID
                Some(ResValue::Array(ws)) => {
                    for (k, item) in ws.iter().enumerate() {
                        match item {
                            ResValue::Int(v) => {
                                w.cid.insert(c + k as u32, *v as f32);
                            }
                            ResValue::Real(v) => {
                                w.cid.insert(c + k as u32, *v);
                            }
                            _ => {}
                        }
                    }
                    i += 2;
                }
                // [c1 c2 w] 形态：c1..=c2 等宽
                Some(ResValue::Int(c2)) => {
                    if let Some(ResValue::Int(_) | ResValue::Real(_)) = items.get(i + 2) {
                        let width = match items.get(i + 2) {
                            Some(ResValue::Int(n)) => *n as f32,
                            Some(ResValue::Real(r)) => *r,
                            _ => unreachable!(),
                        };
                        for k2 in c..=(*c2 as u32) {
                            w.cid.insert(k2, width);
                        }
                    }
                    i += 3;
                }
                _ => break,
            }
        }
    }
    w
}

fn cid_dict(d: Option<&ResValue>) -> Option<&[(Vec<u8>, ResValue)]> {
    match d {
        Some(ResValue::Stream { dict, .. }) | Some(ResValue::Dict(dict)) => Some(dict),
        _ => None,
    }
}

fn registry_of(d: Option<&ResValue>) -> Option<Vec<u8>> {
    cid_dict(d).and_then(|dd| match dict_get(dd, b"CIDSystemInfo") {
        Some(ResValue::Dict(pairs)) => match dict_get(pairs, b"Registry") {
            Some(ResValue::Name(s)) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    })
}

fn ordering_of(d: Option<&ResValue>) -> Option<Vec<u8>> {
    cid_dict(d).and_then(|dd| match dict_get(dd, b"CIDSystemInfo") {
        Some(ResValue::Dict(pairs)) => match dict_get(pairs, b"Ordering") {
            Some(ResValue::Name(s)) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    })
}

fn supplement_of(d: Option<&ResValue>) -> Option<i64> {
    cid_dict(d).and_then(|dd| match dict_get(dd, b"CIDSystemInfo") {
        Some(ResValue::Dict(pairs)) => match dict_get(pairs, b"Supplement") {
            Some(ResValue::Int(i)) => Some(*i),
            _ => None,
        },
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font_dict(pairs: Vec<(Vec<u8>, ResValue)>) -> ResValue {
        ResValue::Dict(pairs)
    }

    fn descriptor() -> Vec<(Vec<u8>, ResValue)> {
        vec![
            (b"Ascent".to_vec(), ResValue::Int(722)),
            (b"Descent".to_vec(), ResValue::Int(-222)),
            (b"MissingWidth".to_vec(), ResValue::Int(500)),
            (b"StemV".to_vec(), ResValue::Int(80)),
        ]
    }

    /// 简单字体全路径：/Widths + /FirstChar + FontDescriptor。
    #[test]
    fn simple_font_widths_and_attrs() {
        let v = font_dict(vec![
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
            (b"FontDescriptor".to_vec(), ResValue::Dict(descriptor())),
        ]);
        let f = PdeFont::from_res_value(&v).unwrap();
        assert_eq!(f.attrs.name, b"Times-Roman");
        assert_eq!(f.attrs.font_type, FontType::Type1);
        // 1000 EM 宽度：FirstChar=32 基址
        assert_eq!(f.width_of(32), 250.0);
        assert_eq!(f.width_of(33), 500.0);
        assert_eq!(f.width_of(34), 750.0);
        // 缺条目 → MissingWidth 兜底
        assert_eq!(f.width_of(35), 500.0);
        // FontDescriptor 度量（descent 取绝对值）
        assert_eq!(f.attrs.ascent, Some(722.0));
        assert_eq!(f.attrs.descent, Some(222.0));
        assert_eq!(f.attrs.missing_width, Some(500.0));
        assert!(!f.attrs.cant_embed);
        assert!(!f.is_embedded);
        // 简单字体单字节码
        assert_eq!(f.num_code_bytes(b"AB"), Some(1));
        // 完成度：widths 已就绪 / ToUnicode 缺 / 未嵌入
        let need = f.create_need_flags();
        assert!(!need.need_widths);
        assert!(need.need_to_unicode);
        assert!(need.need_embed);
    }

    /// Type0：descendant /W 两形态 + CID 双字节探测。
    #[test]
    fn type0_cid_widths() {
        let w_array = ResValue::Array(vec![
            // [c [w...]] 形态
            ResValue::Int(100),
            ResValue::Array(vec![ResValue::Int(500), ResValue::Int(600)]),
            // [c1 c2 w] 区间形态
            ResValue::Int(200),
            ResValue::Int(202),
            ResValue::Int(700),
        ]);
        let cid_font = ResValue::Dict(vec![
            (
                b"Subtype".to_vec(),
                ResValue::Name(b"CIDFontType2".to_vec()),
            ),
            (b"W".to_vec(), w_array),
            (
                b"CIDSystemInfo".to_vec(),
                ResValue::Dict(vec![
                    (b"Registry".to_vec(), ResValue::Name(b"Adobe".to_vec())),
                    (b"Ordering".to_vec(), ResValue::Name(b"Japan1".to_vec())),
                    (b"Supplement".to_vec(), ResValue::Int(6)),
                ]),
            ),
        ]);
        let v = font_dict(vec![
            (b"Type".to_vec(), ResValue::Name(b"Font".to_vec())),
            (b"Subtype".to_vec(), ResValue::Name(b"Type0".to_vec())),
            (
                b"BaseFont".to_vec(),
                ResValue::Name(b"HeiseiMin-W3".to_vec()),
            ),
            (b"Encoding".to_vec(), ResValue::Name(b"Identity-H".to_vec())),
            (b"DescendantFonts".to_vec(), ResValue::Array(vec![cid_font])),
        ]);
        let f = PdeFont::from_res_value(&v).unwrap();
        assert!(f.attrs.font_type.is_composite());
        assert_eq!(f.width_of(100), 500.0);
        assert_eq!(f.width_of(101), 600.0);
        assert_eq!(f.width_of(200), 700.0);
        assert_eq!(f.width_of(202), 700.0);
        // CID 缺条目：无 MissingWidth → PDF 规范缺省 250
        assert_eq!(f.width_of(9999), 250.0);
        // Type0 双字节码
        assert_eq!(f.num_code_bytes(b"\x01\x02\x03"), Some(2));
        // CID 系三件套
        assert_eq!(f.attrs.registry, Some(b"Adobe".to_vec()));
        assert_eq!(f.attrs.ordering, Some(b"Japan1".to_vec()));
        assert_eq!(f.attrs.supplement, Some(6));
        assert_eq!(f.attrs.cid_font_type, Some(2));
        // descendant 保形
        assert!(f.descendant.is_some());
    }

    /// 嵌入禁令（FontDescriptor /Flags bit 2）+ IsEmbedded（FontFile*）。
    #[test]
    fn cant_embed_and_is_embedded() {
        let fd = vec![
            (b"Flags".to_vec(), ResValue::Int(4)), // bit 2 = cantEmbed
            (
                b"FontFile".to_vec(),
                ResValue::Stream {
                    dict: vec![],
                    data: vec![0x80],
                },
            ),
        ];
        let v = font_dict(vec![
            (b"Subtype".to_vec(), ResValue::Name(b"Type1".to_vec())),
            (b"BaseFont".to_vec(), ResValue::Name(b"F".to_vec())),
            (b"FontDescriptor".to_vec(), ResValue::Dict(fd)),
        ]);
        let f = PdeFont::from_res_value(&v).unwrap();
        assert!(f.attrs.cant_embed, "Flags bit2 = 嵌入禁令");
        assert!(f.is_embedded, "FontFile* 存在 = in-document 嵌入");
        // 无 Widths → need_widths
        assert!(f.create_need_flags().need_widths);
    }

    /// 非 Dict 形态 = None（名字探针回退面——批②兼容）。
    #[test]
    fn non_dict_returns_none() {
        assert!(PdeFont::from_res_value(&ResValue::Name(b"F1".to_vec())).is_none());
    }
}
