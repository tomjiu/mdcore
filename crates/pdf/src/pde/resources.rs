//! pde::resources —— 资源表解引用（批②第一件，issue #162）。
//!
//! Adobe 参照：`PDEFormCreateFromCosObjEx` / `PDEFormGetContent` 的资源
//! 供给面——`Do` 的名字引用经页（或 form 自身）的 `/Resources` 字典
//! 解引用为 XObject 对象，form 的内容流同为 PDEContent（"A PDEContent
//! may be obtained from a PDEForm to edit the form's display list"）。
//!
//! # 形态边界（批②覆盖面，诚实登记）
//!
//! - **输入形态**：PDF 页节点资源的**语义子集**——本模块自带轻量对象
//!   图值模型 [`ResValue`]（字典/数组/流引用/名字/数字），不依赖 lopdf
//!   （default = [] 零引擎依赖纪律保持；lopdf 互转门归 engine-lopdf
//!   feature 的挂接层，见 mod.rs 批③预告）。
//! - **Do 解引用**：`/XObject /Name` → [`XObjectKind::Form`]（递归
//!   [`PdeContent::parse`] = `PDEFormGetContent`）或
//!   [`XObjectKind::Image`]（named image，PDEImageAttrs 面）。找不到名
//!   字 = 解析错误（Adobe `peErrUnknownResType` 同位）。
//! - **gs 解引用**：`/ExtGState /Name` → 字典值形态（批②仅解引用承载，
//!   语义建模面批③）。
//! - **深度限制**：form 递归默认 8 层（Adobe 同款防护——病态环引用防
//!   死循环），超深 = 错误 [`PdeError::Parse`]（op = "Do"）。
//!
//! # 与 Adobe 的偏差登记
//!
//! Adobe 的资源查找是**文档对象图**运行期行为；本层为**显式注入**的
//! 值模型（调用方从任意引擎取资源字典注入）——这是「补足开源引擎之
//! 间差异」的产品定位面：pdfium/lopdf 各自的对象图在引擎挂接层归一
//! 为 [`ResourceDict`]，PDE 层只认本模块形态。

use super::model::{Operand, PdeContent, PdeElement, PdeError, PdeImage};

/// 资源值模型（PDF 对象图的 PDE 子集——字典/数组/流/标量）。
///
/// 批②建模面：XObject（form/image 流对象含字典+数据）、ExtGState
/// 字典、Font/Shading/ColorSpace/Pattern 的**名字存在性**（引用收集
/// 面，字典语义建模归批③/④）。
#[derive(Debug, Clone, PartialEq)]
pub enum ResValue {
    /// 名字对象（`/DeviceRGB` 等）。
    Name(Vec<u8>),
    /// 数字（整数形态保形——`/Nums [n]`、宽度等）。
    Int(i64),
    /// 数字（实数形态保形）。
    Real(f32),
    /// 布尔。
    Bool(bool),
    /// 数组。
    Array(Vec<ResValue>),
    /// 字典（键为名字字节）。
    Dict(Vec<(Vec<u8>, ResValue)>),
    /// 流对象：字典 + 原始数据（未解滤镜——解码走 [`super::image`]）。
    Stream {
        dict: Vec<(Vec<u8>, ResValue)>,
        data: Vec<u8>,
    },
}

impl ResValue {
    /// 名字字节视图（名字对象）。
    pub fn as_name(&self) -> Option<&[u8]> {
        match self {
            ResValue::Name(b) => Some(b),
            _ => None,
        }
    }

    /// 数字视图（Int/Real → f32）。
    pub fn as_num(&self) -> Option<f32> {
        match self {
            ResValue::Int(i) => Some(*i as f32),
            ResValue::Real(r) => Some(*r),
            _ => None,
        }
    }

    /// 字典键查找（首匹配）。
    pub fn dict_get(&self, key: &[u8]) -> Option<&ResValue> {
        match self {
            ResValue::Dict(pairs) | ResValue::Stream { dict: pairs, .. } => {
                pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
            }
            _ => None,
        }
    }

    /// 从 [`Operand`]（内容流词法形态）转换——BDC 属性字典等资源侧
    /// 复用面。
    pub fn from_operand(op: &Operand) -> ResValue {
        match op {
            Operand::Int(i) => ResValue::Int(*i),
            Operand::Real(r) => ResValue::Real(*r),
            Operand::Name(n) => ResValue::Name(n.clone()),
            Operand::Str { bytes, .. } => ResValue::Name(bytes.clone()),
            Operand::Bool(b) => ResValue::Bool(*b),
            Operand::Null => ResValue::Name(b"null".to_vec()),
            Operand::Array(items) => {
                ResValue::Array(items.iter().map(ResValue::from_operand).collect())
            }
            Operand::Dict(pairs) => ResValue::Dict(
                pairs
                    .iter()
                    .map(|(k, v)| {
                        let key = k.as_name_bytes().map(|b| b.to_vec()).unwrap_or_default();
                        (key, ResValue::from_operand(v))
                    })
                    .collect(),
            ),
        }
    }
}

/// XObject 种类（解引用结果形态）。
#[derive(Debug, Clone, PartialEq)]
pub enum XObjectKind {
    /// Form XObject——递归 PdeContent（`PDEFormGetContent` 面）。
    Form {
        /// `/Matrix`（缺省 = 单位阵——PDF 规范默认值）。
        matrix: [f32; 6],
        /// `/BBox`（缺省 = None：几何自足性不足，Adobe PDEFormGetBBox
        /// 依赖 Cos 层 BBox，缺失即诚实降级）。
        bbox: Option<[f32; 4]>,
        /// form 自身的 `/Resources`（缺省 = 继承调用方注入面）。
        resources: Option<ResourceDict>,
        /// 递归解析的 display list。
        content: PdeContent,
    },
    /// Image XObject（named image）——PDEImage named 形态。
    Image {
        /// `/Width`。
        width: i64,
        /// `/Height`。
        height: i64,
        /// `/ColorSpace` 值（名字或数组——语义建模归批③，批②保形承载）。
        colorspace: Option<ResValue>,
        /// `/BitsPerComponent`。
        bpc: i64,
        /// `/Filter`（名字或数组）。
        filter: Option<ResValue>,
        /// 原始流数据（未解滤镜——解码走 [`super::image`]）。
        data: Vec<u8>,
    },
    /// PostScript XObject——Adobe PDEPS 面（内容流直承载，无递归）。
    Ps { data: Vec<u8> },
}

/// 资源字典（PDE 批②形态：五类资源的名字 → 值表）。
///
/// Adobe 语义对照：页对象 `/Resources` 的 PDE 可消费子集。`Font` /
/// `Shading` / `ColorSpace` / `Pattern` 批②只做**存在性承载**（内容流
/// 引用面 [`super::model::TextState::font`] 等已持名字）；`XObject` /
/// `ExtGState` 做**值级解引用**（form 递归与 gs 字典是编辑链刚需）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResourceDict {
    /// `/XObject`：名字 → 流对象（Subtype 分流点）。
    pub xobject: Vec<(Vec<u8>, ResValue)>,
    /// `/ExtGState`：名字 → 字典。
    pub extgstate: Vec<(Vec<u8>, ResValue)>,
    /// `/Font`：名字 → 字体字典（批②存在性；语义面批③ PDEFont）。
    pub font: Vec<(Vec<u8>, ResValue)>,
    /// `/Shading`：名字 → shading 字典（批②存在性；PDEShading 深面批④）。
    pub shading: Vec<(Vec<u8>, ResValue)>,
    /// `/ColorSpace`：名字 → 色彩空间（批②存在性；PDEColorSpace 批③）。
    pub colorspace: Vec<(Vec<u8>, ResValue)>,
    /// `/Pattern`：名字 → pattern（批②存在性；PDEPattern 批③）。
    pub pattern: Vec<(Vec<u8>, ResValue)>,
}

impl ResourceDict {
    /// 空资源字典。
    pub fn new() -> Self {
        ResourceDict::default()
    }

    /// 从语义字典树构造（调用方注入面——引擎挂接层把 pdfium/lopdf 的
    /// 对象图归一为本形态）。
    ///
    /// `root` 须为 [`ResValue::Dict`]；未知资源类（如 `/Properties`）
    /// **宽收忽略**——PDE 批②消费面之外的类不进模型（Adobe PDE 层同
    /// 样只消费其声明的资源类）。
    pub fn from_res_value(root: &ResValue) -> ResourceDict {
        let mut rd = ResourceDict::new();
        let pairs = match root {
            ResValue::Dict(pairs) | ResValue::Stream { dict: pairs, .. } => pairs,
            _ => return rd,
        };
        for (k, v) in pairs {
            let table = match k.as_slice() {
                b"XObject" => &mut rd.xobject,
                b"ExtGState" => &mut rd.extgstate,
                b"Font" => &mut rd.font,
                b"Shading" => &mut rd.shading,
                b"ColorSpace" => &mut rd.colorspace,
                b"Pattern" => &mut rd.pattern,
                _ => continue, // 宽收：非 PDE 消费类忽略
            };
            if let ResValue::Dict(items) = v {
                for (name, val) in items {
                    table.push((name.clone(), val.clone()));
                }
            }
        }
        rd
    }

    /// XObject 名字查找。
    pub fn xobject(&self, name: &[u8]) -> Option<&ResValue> {
        self.xobject.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// ExtGState 名字查找。
    pub fn extgstate(&self, name: &[u8]) -> Option<&ResValue> {
        self.extgstate
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
    }

    /// Font 名字查找（存在性探针）。
    pub fn font(&self, name: &[u8]) -> Option<&ResValue> {
        self.font.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// Shading 名字查找（存在性探针）。
    pub fn shading(&self, name: &[u8]) -> Option<&ResValue> {
        self.shading.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// ColorSpace 名字查找（存在性探针）。
    pub fn colorspace(&self, name: &[u8]) -> Option<&ResValue> {
        self.colorspace
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
    }

    /// Pattern 名字查找（存在性探针）。
    pub fn pattern(&self, name: &[u8]) -> Option<&ResValue> {
        self.pattern.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// form 的资源合并面（`PDEFormCreateFromCosObjEx` 的 resourcesP 参
    /// 数语义）：form 自带 `/Resources` 缺失时**继承**调用方注入面。
    pub fn or_inherit(&self, form_res: Option<&ResourceDict>) -> ResourceDict {
        match form_res {
            None => self.clone(),
            Some(r) => r.clone(),
        }
    }
}

/// form 递归默认深度上限（Adobe 同款防护：病态环引用防死循环）。
pub const FORM_DEPTH_LIMIT: usize = 8;

/// `Do` 名字解引用（`PDEFormCreateFromCosObjEx` + `PDEFormGetContent`
/// 的批②合并面）。
///
/// 按 XObject `/Subtype` 分流：`/Form` → 递归解析内容流（form 的
/// display list 也是 PdeContent）；`/Image` → named image 形态；
/// `/PS` → PostScript 直承载。名字缺失 = `peErrUnknownResType` 同位
/// 错误。
pub fn deref_xobject(
    name: &[u8],
    res: &ResourceDict,
    depth: usize,
) -> Result<XObjectKind, PdeError> {
    if depth > FORM_DEPTH_LIMIT {
        return Err(PdeError::Parse {
            op: "Do".into(),
            reason: format!("form 递归超深度上限 {FORM_DEPTH_LIMIT}（疑环引用）"),
        });
    }
    let obj = res.xobject(name).ok_or_else(|| PdeError::Parse {
        op: "Do".into(),
        reason: format!(
            "XObject 名字 {} 不在资源表（peErrUnknownResType 同位）",
            String::from_utf8_lossy(name)
        ),
    })?;
    // form / image 的承载形态必须是流对象（字典 + 数据）。
    let (dict, data): (&Vec<(Vec<u8>, ResValue)>, &[u8]) = match obj {
        ResValue::Stream { dict, data } => (dict, data),
        _ => {
            return Err(PdeError::Parse {
                op: "Do".into(),
                reason: "XObject 资源非流对象（缺字典+数据面）".into(),
            });
        }
    };
    let lookup =
        |key: &[u8]| -> Option<&ResValue> { dict.iter().find(|(k, _)| k == key).map(|(_, v)| v) };
    /// 数组 → 定长 f32（Matrix 6 / BBox 4——长度不符 = None 诚实降级）。
    fn nums_array(v: Option<&ResValue>, n: usize) -> Option<Vec<f32>> {
        match v {
            Some(ResValue::Array(items)) => {
                let nums: Option<Vec<f32>> = items.iter().map(|i| i.as_num()).collect();
                nums.filter(|ns| ns.len() == n)
            }
            _ => None,
        }
    }
    match lookup(b"Subtype").and_then(ResValue::as_name) {
        Some(b"Form") => {
            let matrix = match nums_array(lookup(b"Matrix"), 6) {
                Some(n) => [n[0], n[1], n[2], n[3], n[4], n[5]],
                None => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            };
            let bbox = nums_array(lookup(b"BBox"), 4)
                .map(|n| [n[0], n[1], n[2], n[3]]);
            let resources = lookup(b"Resources").map(ResourceDict::from_res_value);
            // 递归解析（PDEFormGetContent）：form 的资源继承调用方面。
            let content = super::parser::parse_content(data)?;
            Ok(XObjectKind::Form {
                matrix,
                bbox,
                resources,
                content,
            })
        }
        Some(b"Image") => {
            let width = lookup(b"Width")
                .and_then(|v| match v {
                    ResValue::Int(i) => Some(*i),
                    ResValue::Real(r) => Some(*r as i64),
                    _ => None,
                })
                .unwrap_or(0);
            let height = lookup(b"Height")
                .and_then(|v| match v {
                    ResValue::Int(i) => Some(*i),
                    ResValue::Real(r) => Some(*r as i64),
                    _ => None,
                })
                .unwrap_or(0);
            let bpc = lookup(b"BitsPerComponent")
                .and_then(|v| match v {
                    ResValue::Int(i) => Some(*i),
                    ResValue::Real(r) => Some(*r as i64),
                    _ => None,
                })
                .unwrap_or(0);
            Ok(XObjectKind::Image {
                width,
                height,
                colorspace: lookup(b"ColorSpace").cloned(),
                filter: lookup(b"Filter").cloned(),
                bpc,
                data: data.to_vec(),
            })
        }
        Some(b"PS") => Ok(XObjectKind::Ps {
            data: data.to_vec(),
        }),
        other => Err(PdeError::Parse {
            op: "Do".into(),
            reason: format!(
                "XObject Subtype 未知：{}",
                other
                    .map(|n| String::from_utf8_lossy(n).into_owned())
                    .unwrap_or_else(|| "(缺失)".into())
            ),
        }),
    }
}

/// 把 XObject 引用元素**解析并替换**为可编辑形态（编辑链主入口）。
///
/// - `PdeElement::XObject` + `/Form` → 保持名字引用（Adobe PDEForm 是
///   独立元素类；本层批②把解引用结果放回 [`PdeElement::XObject`] 的
///   扩展面：`PdeXObject::deref`，写回仍按名字引用 `Do` 发出——
///   「编辑 form 内部 → 写回 form 流」由调用方经 [`XObjectKind::Form`]
///   的 content 单独落盘）。
/// - `PdeElement::XObject` + `/Image` → 转为 [`PdeElement::Image`] 的
///   named 形态（attrs 承载 Width/Height/ColorSpace/BPC/Filter + 数据）。
///
/// 返回：`(元素, Option<XObjectKind>)`——解引用产物给调用方（form 的
/// 递归 content 需要独立写回流，不能内联进页内容流）。
pub fn resolve_xobject_element(
    elem: &PdeElement,
    res: &ResourceDict,
) -> Result<(PdeElement, Option<XObjectKind>), PdeError> {
    match elem {
        PdeElement::XObject(x) => {
            let kind = deref_xobject(&x.name, res, 0)?;
            match kind {
                XObjectKind::Form { .. } => {
                    // form：保持名字引用形态（写回面 Do 原样）+ 解引用
                    // 产物交调用方。
                    Ok((elem.clone(), Some(kind)))
                }
                XObjectKind::Image {
                    width,
                    height,
                    colorspace,
                    bpc,
                    filter,
                    data,
                } => {
                    // named image → PdeImage named 形态（attrs 面向
                    // PDEImageAttrs 对齐）。
                    let mut attrs: Vec<(Vec<u8>, Operand)> = vec![
                        (b"W".to_vec(), Operand::Int(width)),
                        (b"H".to_vec(), Operand::Int(height)),
                        (b"BPC".to_vec(), Operand::Int(bpc)),
                    ];
                    if let Some(cs) = &colorspace {
                        attrs.push((b"CS".to_vec(), resvalue_to_operand(cs)));
                    }
                    if let Some(f) = &filter {
                        attrs.push((b"F".to_vec(), resvalue_to_operand(f)));
                    }
                    let img = PdeElement::Image(PdeImage {
                        gstate: x.gstate.clone(),
                        attrs,
                        data: data.clone(),
                    });
                    Ok((
                        img,
                        Some(XObjectKind::Image {
                            width,
                            height,
                            colorspace,
                            bpc,
                            filter,
                            data,
                        }),
                    ))
                }
                XObjectKind::Ps { .. } => Ok((elem.clone(), Some(kind))),
            }
        }
        // Container 递归面：内部元素的 XObject 引用同样解引用（form 的
        // display list 里可以再 Do form——Adobe 嵌套 form 同构）。
        PdeElement::Container(c) => {
            let mut inner = c.clone();
            let mut first_kind: Option<XObjectKind> = None;
            let mut next = Vec::with_capacity(inner.content.elements.len());
            for e in inner.content.elements.drain(..) {
                let (ne, k) = resolve_xobject_element(&e, res)?;
                if first_kind.is_none() {
                    first_kind = k;
                }
                next.push(ne);
            }
            inner.content.elements = next;
            Ok((PdeElement::Container(inner), first_kind))
        }
        _ => Ok((elem.clone(), None)),
    }
}

/// [`ResValue`] → [`Operand`]（named image attrs 承载面——批①
/// [`PdeImage::attrs`] 的 Operand 形态复用）。
fn resvalue_to_operand(v: &ResValue) -> Operand {
    match v {
        ResValue::Name(n) => Operand::Name(n.clone()),
        ResValue::Int(i) => Operand::Int(*i),
        ResValue::Real(r) => Operand::Real(*r),
        ResValue::Bool(b) => Operand::Bool(*b),
        ResValue::Array(items) => Operand::Array(items.iter().map(resvalue_to_operand).collect()),
        ResValue::Dict(pairs) => Operand::Dict(
            pairs
                .iter()
                .map(|(k, v)| (Operand::Name(k.clone()), resvalue_to_operand(v)))
                .collect(),
        ),
        ResValue::Stream { dict, data } => {
            // 流形态不进 attrs（数据已另存 data 字段）——以字典形态承载。
            Operand::Dict(
                dict.iter()
                    .map(|(k, v)| (Operand::Name(k.clone()), resvalue_to_operand(v)))
                    .chain(std::iter::once((
                        Operand::Name(b"Length".to_vec()),
                        Operand::Int(data.len() as i64),
                    )))
                    .collect(),
            )
        }
    }
}

/// 内容流全量解引用（`PDPageAcquirePDEContent` 的资源供给完成形态）：
/// 遍历元素树把 XObject 名字引用全部解析。form 的递归 content 在
/// [`XObjectKind::Form`] 里返回（调用方继续对 form 的 resources 递归
/// 调用本函数即得完整可编辑树）。
pub fn resolve_content(
    content: &PdeContent,
    res: &ResourceDict,
) -> Result<(PdeContent, Vec<XObjectKind>), PdeError> {
    let mut kinds = Vec::new();
    let mut elements = Vec::with_capacity(content.elements.len());
    for e in &content.elements {
        let (ne, k) = resolve_xobject_element(e, res)?;
        if let Some(kind) = k {
            kinds.push(kind);
        }
        elements.push(ne);
    }
    Ok((PdeContent { elements }, kinds))
}

/// 写回资源收集（`PDEContentToCosObj` 的 `resourcesP` 输出面——批②
/// 第三件）。
///
/// Adobe 语义：**"This is the main method for converting a PDEContent
/// into PDF contents and resources"**——`PDEContentToCosObj` 出参两件
/// （contentsP + resourcesP），批①只做了内容流面（`PdeContent::write`），
/// 本函数补齐资源面：遍历元素树收集**内容流引用的全部资源名**，产出
/// 写回所需的资源字典形态。
///
/// 收集面（内容流→资源名的引用来源）：
/// - [`PdeElement::XObject`]（`Do`）→ `/XObject` 名字集合
/// - [`PdeElement::Shading`]（`sh`）→ `/Shading` 名字集合
/// - 文本 [`super::model::TextState::font`]（`Tf`）→ `/Font` 名字集合
/// - [`super::model::GState::extgstate`]（`gs`）→ `/ExtGState` 名字集合
/// - 颜色空间名（`cs`/`CS` 设非设备空间名）→ `/ColorSpace` 名字集合
///
/// 名字集合**不含资源对象本体**——本体在编辑流里来自调用方注入的
/// [`ResourceDict`]（[`ResourceDict::merge_collected`] 把本体按引用面
/// 过滤合并），与 Adobe「caller is responsible for adding the contents
/// and the resources to the Page Object」的责任划分同构。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CollectedResources {
    /// `Do` 引用的 XObject 名字（原序去重）。
    pub xobject: Vec<Vec<u8>>,
    /// `sh` 引用的 shading 名字。
    pub shading: Vec<Vec<u8>>,
    /// `Tf` 引用的字体名字。
    pub font: Vec<Vec<u8>>,
    /// `gs` 引用的 ExtGState 名字。
    pub extgstate: Vec<Vec<u8>>,
    /// `cs`/`CS` 设定的非设备色彩空间名字。
    pub colorspace: Vec<Vec<u8>>,
    /// `scn`/`SCN` 的 Pattern 名字分量。
    pub pattern: Vec<Vec<u8>>,
}

/// 设备色彩空间名（`cs`/`CS` 直名不进资源表——PDF 规范内建）。
const DEVICE_SPACES: [&[u8]; 7] = [
    b"DeviceGray",
    b"DeviceRGB",
    b"DeviceCMYK",
    b"Pattern",
    b"G",
    b"RGB",
    b"CMYK",
];

fn push_dedup(list: &mut Vec<Vec<u8>>, name: &[u8]) {
    if !list.iter().any(|n| n == name) {
        list.push(name.to_vec());
    }
}

impl CollectedResources {
    /// 空收集。
    pub fn new() -> Self {
        CollectedResources::default()
    }

    /// 收集入口：元素树全量遍历（Container 递归进 content）。
    pub fn from_content(content: &PdeContent) -> CollectedResources {
        let mut c = CollectedResources::new();
        c.collect_elems(&content.elements);
        c
    }

    fn collect_elems(&mut self, elems: &[PdeElement]) {
        for e in elems {
            self.collect_elem(e);
        }
    }

    fn collect_elem(&mut self, e: &PdeElement) {
        match e {
            PdeElement::XObject(x) => push_dedup(&mut self.xobject, &x.name),
            PdeElement::Shading(s) => push_dedup(&mut self.shading, &s.name),
            PdeElement::Text(t) => {
                // 字体引用：run 的文本状态逐 run 收集（同 run 合并判据
                // 同态——Adobe PDEText 每 run 持一份 state）。
                for run in &t.runs {
                    if let Some(f) = &run.state.font {
                        push_dedup(&mut self.font, f);
                    }
                }
                self.collect_gstate(&t.gstate);
            }
            PdeElement::Path(p) => self.collect_gstate(&p.gstate),
            PdeElement::Image(img) => self.collect_gstate(&img.gstate),
            PdeElement::Container(c) => self.collect_elems(&c.content.elements),
            PdeElement::Place(_) | PdeElement::Unknown(_) => {}
        }
    }

    fn collect_gstate(&mut self, g: &super::model::GState) {
        if let Some(gs) = &g.extgstate {
            push_dedup(&mut self.extgstate, gs);
        }
        // 色彩空间：非设备名 = 资源名引用（`cs /CS0 scn` 形态）。
        if !DEVICE_SPACES.contains(&g.fill.space.as_slice()) {
            push_dedup(&mut self.colorspace, &g.fill.space);
        }
        if !DEVICE_SPACES.contains(&g.stroke.space.as_slice()) {
            push_dedup(&mut self.colorspace, &g.stroke.space);
        }
        // Pattern 名字分量（`/P1 scn` 形态——ColorValues::Name）。
        if let Some(n) = match &g.fill.values {
            super::model::ColorValues::Name(n) => Some(n.clone()),
            _ => None,
        } {
            push_dedup(&mut self.pattern, &n);
        }
        if let Some(n) = match &g.stroke.values {
            super::model::ColorValues::Name(n) => Some(n.clone()),
            _ => None,
        } {
            push_dedup(&mut self.pattern, &n);
        }
    }

    /// 引用面是否为空（「内容流零资源引用」判定——空 resources 字典
    /// 可省略的 PDF 规范面）。
    pub fn is_empty(&self) -> bool {
        self.xobject.is_empty()
            && self.shading.is_empty()
            && self.font.is_empty()
            && self.extgstate.is_empty()
            && self.colorspace.is_empty()
            && self.pattern.is_empty()
    }
}

impl ResourceDict {
    /// 按收集面过滤合并本体（写回资源面主入口）：
    /// 取注入资源表中**被内容流引用**的资源，产出写回用资源字典——
    /// 未引用资源不进写回产物（Adobe `PDEContentToCosObj` 只输出
    /// contents 引用到的 resources，同语义）。
    ///
    /// 引用面缺失本体（名字在内容流、本体不在注入表）= 静默跳过 +
    /// [`CollectedResources::missing_from`] 显式可查（编辑流完整性由
    /// 调用方决定是否硬失败——Adobe 同位错误
    /// `peErrUnknownResType`，本函数不代作主张）。
    pub fn merge_collected(&self, collected: &CollectedResources) -> ResourceDict {
        let mut out = ResourceDict::new();
        for name in &collected.xobject {
            if let Some(v) = self.xobject(name) {
                out.xobject.push((name.clone(), v.clone()));
            }
        }
        for name in &collected.extgstate {
            if let Some(v) = self.extgstate(name) {
                out.extgstate.push((name.clone(), v.clone()));
            }
        }
        for name in &collected.font {
            if let Some(v) = self.font(name) {
                out.font.push((name.clone(), v.clone()));
            }
        }
        for name in &collected.shading {
            if let Some(v) = self.shading(name) {
                out.shading.push((name.clone(), v.clone()));
            }
        }
        for name in &collected.colorspace {
            if let Some(v) = self.colorspace(name) {
                out.colorspace.push((name.clone(), v.clone()));
            }
        }
        for name in &collected.pattern {
            if let Some(v) = self.pattern(name) {
                out.pattern.push((name.clone(), v.clone()));
            }
        }
        out
    }
}

impl CollectedResources {
    /// 引用面里不在注入表的名单（缺失本体探针——写回前完整性检查）。
    pub fn missing_from(&self, res: &ResourceDict) -> Vec<Vec<u8>> {
        let mut missing = Vec::new();
        for n in &self.xobject {
            if res.xobject(n).is_none() {
                missing.push(n.clone());
            }
        }
        for n in &self.extgstate {
            if res.extgstate(n).is_none() {
                missing.push(n.clone());
            }
        }
        for n in &self.font {
            if res.font(n).is_none() {
                missing.push(n.clone());
            }
        }
        for n in &self.shading {
            if res.shading(n).is_none() {
                missing.push(n.clone());
            }
        }
        for n in &self.colorspace {
            if res.colorspace(n).is_none() {
                missing.push(n.clone());
            }
        }
        for n in &self.pattern {
            if res.pattern(n).is_none() {
                missing.push(n.clone());
            }
        }
        missing
    }

    /// 资源字典序列化（写回产物面——`/Resources << /XObject << /Fm0
    /// 12 0 R >> >>` 的**名字清单形态**：本层不产间接引用号，引用号
    /// 归引擎挂接层批③；此处输出形如 `/XObject << /Fm0 (Fm0) >>` 的
    /// 名字占位形态，测试与结构检查用）。
    pub fn to_bytes(&self) -> Vec<u8> {
        if self.is_empty() {
            return b"<< >>".to_vec();
        }
        let mut out = b"<< ".to_vec();
        let section = |out: &mut Vec<u8>, key: &[u8], names: &[Vec<u8>]| {
            if names.is_empty() {
                return;
            }
            out.extend_from_slice(b"/");
            out.extend_from_slice(key);
            out.extend_from_slice(b" << ");
            for n in names {
                out.push(b'/');
                out.extend_from_slice(n);
                out.extend_from_slice(b" ");
            }
            out.extend_from_slice(b">> ");
        };
        section(&mut out, b"ExtGState", &self.extgstate);
        section(&mut out, b"Font", &self.font);
        section(&mut out, b"XObject", &self.xobject);
        section(&mut out, b"ColorSpace", &self.colorspace);
        section(&mut out, b"Pattern", &self.pattern);
        section(&mut out, b"Shading", &self.shading);
        out.extend_from_slice(b">>");
        out
    }
}
