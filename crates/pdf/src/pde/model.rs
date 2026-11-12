//! pde::model —— PDE 对象模型（批①核心件）。
//!
//! 以 Adobe Acrobat SDK **PDFEdit API（PDE 层）** 为产品架构参照（issue
//! #162）：类层次与关键方法名对齐 Adobe 官方参考文档
//! （PDFEdit_Layer，opensource.adobe.com/dc-acrobat-sdk-docs/pdflsdk/
//! apireference/PDFEdit_Layer/），实现为纯 Rust、零引擎依赖——「mdcore
//! 自己的 PDE 层」，底层引擎（pdfium/MuPDF）差异由本层补足。
//!
//! 类对照表（Adobe → 本模块）：
//!
//! | Adobe 类 | 本模块 | 说明 |
//! |---|---|---|
//! | PDEContent | [`PdeContent`] | "the workhorse of the PDFEdit API"——PDPage 的可修改内容（解析后的 display list） |
//! | PDEElement | [`PdeElement`] | display list 元素基类（enum 形态；派生类见各行） |
//! | PDEText | [`PdeText`]/[`TextRun`] | run 容器：PDFEdit 把同一字体属性连续 show 合并为单 run |
//! | PDEPath | [`PdePath`]/[`PathSeg`] | 段模型（Adobe 为 32-bit packed int 数据 + ASFixed 16.16；本层为 typed 段 + f32 用户空间坐标——语义同构，映射见 [`PathSeg`] 文档） |
//! | PDEImage | [`PdeImage`] | 批①覆盖 inline image（BI/ID/EI）；named image XObject 经资源表解引用归批② |
//! | PDEForm | （批②） | form XObject——`PDEFormGetContent` 递归编辑；批①以 [`PdeElement::XObject`] 名字引用形态承载 `Do` |
//! | PDEXObject | [`PdeElement::XObject`] | "an element representing an arbitrary XObject"——批①名字引用形态 |
//! | PDEContainer | [`PdeElement::Container`] | marked-content 分组（BMC/BDC/EMC 对） |
//! | PDEPlace | [`PdeElement::Place`] | MP/DP 标记点 |
//! | PDEShading | [`PdeElement::Shading`] | `sh` 平滑着色 |
//! | PDEUnknown | [`PdeElement::Unknown`] | 未识别算子——**原样保留**，写回逐字节重现 |
//! | PDEClip | [`GState::clip`] | clip 态（path/text 元素列表）——批①单路径形态，列表形态归批② |
//!
//! 生命周期对齐（Adobe：`PDPageAcquirePDEContent → 修改 →
//! PDPageSetPDEContent`）：本层为 [`PdeContent::parse`]（=
//! `PDEContentCreateFromCosObj` 内容流面）→ 元素级修改（`add_elem`/
//! `remove_elem`/`set_matrix`/…）→ [`PdeContent::write`]（=
//! `PDEContentToCosObj` 内容流面；资源合并面归批②）。
//!
//! # 与 Adobe 的已登记偏差（诚实面）
//!
//! - `PDEElementIsAtPoint/IsAtRect`（命中测试）依赖 BBox，而文本 BBox
//!   需要字体度量（PDEFont 面）——批②/③补；当前仅 [`PdeElement::bbox`]
//!   支持路径/inline 图（几何自足面）。
//! - PDEText 的 `kPDETextChar` 字符寻址需要字体编码知识（Adobe 文档：
//!   character offset 语义）；批①提供 **run 索引寻址**（`num_runs`/
//!   `run`），字符寻址归批③（PDEFont 度量面）。
//! - q/Q 的显式作用域在写回时**展平**为逐元素快照差分（Adobe
//!   PDEContentToCosObj 同样重生成内容流而非保形）；对已建模字段
//!   （CTM/颜色/线宽/clip）语义等价，round-trip 测试锁定。

use crate::annotation::{Point, Rect};

/// 3×2 仿射矩阵（PDF 记法 `[a b c d e f]`，行向量约定：
/// `[x y 1]·M = (a·x + c·y + e, b·x + d·y + f)`）。
pub type Matrix = [f32; 6];

/// 单位矩阵。
pub const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `kPDEBeforeFirst`（Adobe PDEContentAddElem 的 addAfterIndex 哨兵值：
/// 插到 display list 开头）。
pub const PDE_BEFORE_FIRST: i32 = -1;

/// 行向量矩阵乘（先应用 `a` 再应用 `b`：`[p]·(a×b) = ([p]·a)·b`）。
/// PDF `cm` 的拼接 `CTM' = M × CTM` 与 `Td` 的 `Tlm' = T × Tlm`
/// 均用本函数（M/T 在左）。
pub fn mat_mul(a: Matrix, b: Matrix) -> Matrix {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

/// 平移矩阵（生成用；`Td` 的 T = `[1 0 0 1 tx ty]`）。
pub fn mat_translate(tx: f32, ty: f32) -> Matrix {
    [1.0, 0.0, 0.0, 1.0, tx, ty]
}

/// 用矩阵变换点（行向量）。
pub fn mat_apply(m: Matrix, p: Point) -> Point {
    Point {
        x: m[0] * p.x + m[2] * p.y + m[4],
        y: m[1] * p.x + m[3] * p.y + m[5],
    }
}

/// PDE 层错误（词法/解析两段；写回不失败——模型自洽即合法）。
#[derive(Debug, Clone, PartialEq)]
pub enum PdeError {
    /// 词法错误（字节偏移 + 原因）。
    Lex { offset: usize, reason: String },
    /// 解析错误（算子名 + 原因——含已知算子的元数/类型不符）。
    Parse { op: String, reason: String },
}

impl std::fmt::Display for PdeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PdeError::Lex { offset, reason } => {
                write!(f, "pde lex error at byte {offset}: {reason}")
            }
            PdeError::Parse { op, reason } => {
                write!(f, "pde parse error at operator `{op}`: {reason}")
            }
        }
    }
}

impl std::error::Error for PdeError {}

/// 内容流操作数（词法器产出/模型复用的 PDF 值形态）。
///
/// 字符串保持**原始字节**（字体编码字节——不是 Unicode；解码归
/// PDEFont 面批②/③）；`hex` 记录来源形态供写回保形。
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Int(i64),
    Real(f32),
    /// 名字对象（原始字节——PDF 名字是字节串，非保证 UTF-8）。
    Name(Vec<u8>),
    /// 字符串（原始字节 + 是否十六进制形态）。
    Str {
        bytes: Vec<u8>,
        hex: bool,
    },
    Bool(bool),
    Null,
    Array(Vec<Operand>),
    /// 字典（键为 [`Operand::Name`]；内容流里出现在 BDC 属性/inline
    /// image 属性表等处）。
    Dict(Vec<(Operand, Operand)>),
}

impl Operand {
    /// 数值视图（Int/Real → f32；其余 None）。
    pub fn as_num(&self) -> Option<f32> {
        match self {
            Operand::Int(i) => Some(*i as f32),
            Operand::Real(r) => Some(*r),
            _ => None,
        }
    }

    /// 字符串字节视图。
    pub fn as_str_bytes(&self) -> Option<&[u8]> {
        match self {
            Operand::Str { bytes, .. } => Some(bytes),
            _ => None,
        }
    }

    /// 名字字节视图。
    pub fn as_name_bytes(&self) -> Option<&[u8]> {
        match self {
            Operand::Name(b) => Some(b),
            _ => None,
        }
    }

    /// 名字的 String 视图（有损 UTF-8——仅诊断/模型 String 字段用；
    /// 写回永远走 [`Operand::as_name_bytes`] 原始字节）。
    pub fn name_str_lossy(&self) -> Option<String> {
        self.as_name_bytes()
            .map(|b| String::from_utf8_lossy(b).into_owned())
    }
}

/// 路径段（Adobe `PDEPathElementType` 的 typed 映射）。
///
/// Adobe 的 `PDEPathGetData` 产 32-bit packed int 数组（段类型 +
/// ASFixed 16.16 坐标）；本层为 tagged enum + f32 用户空间坐标——
/// 段集合一一对应（kPDEMoveTo/kPDELineTo/kPDECurveTo/kPDECurveToV/
/// kPDECurveToY/kPDEClose/矩形），写回按 PDF 算子 `m/l/c/v/y/h/re`
/// 重生成。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathSeg {
    MoveTo {
        x: f32,
        y: f32,
    },
    LineTo {
        x: f32,
        y: f32,
    },
    /// 三次贝塞尔（`c`）：控制点 1、控制点 2、终点。
    CurveTo {
        c1x: f32,
        c1y: f32,
        c2x: f32,
        c2y: f32,
        x: f32,
        y: f32,
    },
    /// `v`：第一控制点 = 当前点。
    CurveToV {
        c2x: f32,
        c2y: f32,
        x: f32,
        y: f32,
    },
    /// `y`：第二控制点 = 终点。
    CurveToY {
        c1x: f32,
        c1y: f32,
        x: f32,
        y: f32,
    },
    Close,
    /// `re`：矩形子路径（左下角 + 宽高）。
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
}

/// 路径绘制算子（Adobe `PDEPathGetPaintOp` 的 enum 形态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintOp {
    Stroke,                 // S
    CloseStroke,            // s
    Fill,                   // f（F 为废弃别名——统一归此）
    FillEvenOdd,            // f*
    FillStroke,             // B
    FillStrokeEvenOdd,      // B*
    CloseFillStroke,        // b
    CloseFillStrokeEvenOdd, // b*
    /// `n`：不绘制（结束路径构造；常与 W/W* 连用做纯裁剪）。
    NoPaint,
}

impl PaintOp {
    /// PDF 算子名（写回用；F 废弃别名统一写 f）。
    pub fn op_name(self) -> &'static str {
        match self {
            PaintOp::Stroke => "S",
            PaintOp::CloseStroke => "s",
            PaintOp::Fill => "f",
            PaintOp::FillEvenOdd => "f*",
            PaintOp::FillStroke => "B",
            PaintOp::FillStrokeEvenOdd => "B*",
            PaintOp::CloseFillStroke => "b",
            PaintOp::CloseFillStrokeEvenOdd => "b*",
            PaintOp::NoPaint => "n",
        }
    }
}

/// 颜色值（`sc/scn/SC/SCN` 的操作数形态：数值分量或 Pattern 名）。
#[derive(Debug, Clone, PartialEq)]
pub enum ColorValues {
    Nums(Vec<f32>),
    /// 图案名（`/P1 scn`——分量面归批② PDEPattern）。
    Name(Vec<u8>),
}

/// 颜色规格：色彩空间名 + 分量。
#[derive(Debug, Clone, PartialEq)]
pub struct ColorSpec {
    /// 空间名（DeviceGray/DeviceRGB/DeviceCMYK 或资源名——`cs`/`CS` 设定）。
    pub space: Vec<u8>,
    pub values: ColorValues,
}

impl ColorSpec {
    /// 设备灰度默认（PDF 初始填充/描边色 = 黑）。
    pub fn device_gray(v: f32) -> Self {
        ColorSpec {
            space: b"DeviceGray".to_vec(),
            values: ColorValues::Nums(vec![v]),
        }
    }
}

/// 裁剪态（Adobe PDEClip 的批①单路径形态：一条 W/W* 路径；
/// 多元素列表形态归批②）。
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub segs: Vec<PathSeg>,
    /// true = W*（奇偶规则）；false = W（非零环绕）。
    pub even_odd: bool,
}

/// 元素级图形状态快照（Adobe `PDEElementGetGState`/`GetMatrix` 的
/// 批①建模面：CTM/填充/描边/线宽/ExtGState/clip/文本状态；线帽线接/
/// dash 等次要面归批②——当前以 [`PdeElement::Unknown`] 原样保留）。
#[derive(Debug, Clone, PartialEq)]
pub struct GState {
    /// 绘制时 CTM（`PDEElementGetMatrix`）。
    pub ctm: Matrix,
    pub fill: ColorSpec,
    pub stroke: ColorSpec,
    pub line_width: f32,
    /// ExtGState 资源名（`gs`——字典解引用归批②）。
    pub extgstate: Option<Vec<u8>>,
    pub clip: Option<Clip>,
    /// 文本状态（PDF §9.3——**属图形状态域**：q/Q 快照/恢复含此，
    /// 且跨 BT/ET 持续（BT 只重置 Tm/Tlm）。Adobe PDEGraphicState
    /// 同含 PDETextState）。
    pub text: TextState,
}

impl Default for GState {
    fn default() -> Self {
        GState {
            ctm: IDENTITY,
            fill: ColorSpec::device_gray(0.0),
            stroke: ColorSpec::device_gray(0.0),
            line_width: 1.0,
            extgstate: None,
            clip: None,
            text: TextState::default(),
        }
    }
}

/// 文本状态（PDF §9.3 文本状态参数——Tf/Tc/Tw/Tz/TL/Ts/Tr；
/// Adobe `PDETextState`）。属于图形状态（q/Q 快照域，见
/// [`GState::text`]）。
#[derive(Debug, Clone, PartialEq)]
pub struct TextState {
    /// Tf 字体资源名（BT 前未设定 = None；show 时 None → 解析错误——
    /// 与 PDF 语义一致：无字体的 show 不可绘制）。
    pub font: Option<Vec<u8>>,
    /// Tf 字号（文本空间单位）。
    pub size: f32,
    pub char_spacing: f32, // Tc
    pub word_spacing: f32, // Tw
    /// Tz 水平缩放（百分比，**默认 100**）。
    pub h_scale: f32,
    pub leading: f32,     // TL
    pub rise: f32,        // Ts
    pub render_mode: i32, // Tr
}

impl Default for TextState {
    fn default() -> Self {
        TextState {
            font: None,
            size: 0.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            h_scale: 100.0,
            leading: 0.0,
            rise: 0.0,
            render_mode: 0,
        }
    }
}

/// show 操作项（TJ 数组元素 / Tj 字符串的统一承载）。
#[derive(Debug, Clone, PartialEq)]
pub enum ShowOp {
    /// 显示字符串（原始字节——字体编码）。
    Str(Vec<u8>),
    /// TJ 数组里的位移调整（千分之一文本空间单位）。
    Kern(f32),
}

/// 文本 run（Adobe：同一属性连续 show 的合并单元——"PDFEdit combines
/// text from multiple Tj operators into a single text run, when
/// possible"）。合并判据 = 文本状态逐字段相等 **且** 文本矩阵相等
/// （属性/定位算子介入即断开）。run 的文本状态从 BT 进入时图形状态
/// 快照的 [`GState::text`] 取值——Adobe PDEText 元素持一份 gstate。
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub state: TextState,
    /// show 时的文本矩阵 Tm（Adobe `PDETextGetTextMatrix`）。
    pub text_matrix: Matrix,
    pub shows: Vec<ShowOp>,
}

/// PDEText 元素（Adobe：一个 BT..ET 块的可绘制文本；单一 clip 对全部
/// 子元素生效——本层 clip 在元素级 [`GState`] 快照）。
#[derive(Debug, Clone, PartialEq)]
pub struct PdeText {
    pub gstate: GState,
    pub runs: Vec<TextRun>,
}

impl PdeText {
    /// `PDETextGetNumRuns`。
    pub fn num_runs(&self) -> usize {
        self.runs.len()
    }

    /// `PDETextGetText`（run 面）：run 的显示字节拼接。
    pub fn run_text(&self, i: usize) -> Option<Vec<u8>> {
        self.runs.get(i).map(|r| {
            r.shows
                .iter()
                .filter_map(|s| match s {
                    ShowOp::Str(b) => Some(b.as_slice()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .concat()
        })
    }

    /// `PDETextAdd`（run 插入面；Adobe 以 (flags, index) 寻址——批①
    /// run 索引寻址）。
    pub fn insert_run(&mut self, index: usize, run: TextRun) {
        let at = index.min(self.runs.len());
        self.runs.insert(at, run);
    }
}

/// PDEPath 元素（构造段序列 + 绘制算子；`clipped` = 该路径同时被
/// W/W* 用作裁剪——Adobe 语义里绘制与裁剪可并存）。
#[derive(Debug, Clone, PartialEq)]
pub struct PdePath {
    pub gstate: GState,
    pub segs: Vec<PathSeg>,
    pub paint: PaintOp,
    /// 本路径绘制同时设定了裁剪（`path W S` 形态）。
    pub clipped: bool,
}

/// PDEImage 元素——批①为 **inline image**（BI/ID/EI）：属性表按原序
/// 保留，数据为原始字节（未解滤镜——解码归批② PDEImage 数据面）。
/// named image XObject 见 [`PdeElement::XObject`]。
#[derive(Debug, Clone, PartialEq)]
pub struct PdeImage {
    pub gstate: GState,
    /// 属性表（BI..ID 间的 name-value 对，原序）。
    pub attrs: Vec<(Vec<u8>, Operand)>,
    /// ID 与 EI 间的原始数据字节。
    pub data: Vec<u8>,
}

/// PDEXObject 元素（`/Name Do`——名字引用形态；form/image 区分与
/// `PDEFormGetContent` 递归编辑需资源表解引用，归批②）。
#[derive(Debug, Clone, PartialEq)]
pub struct PdeXObject {
    pub gstate: GState,
    pub name: Vec<u8>,
}

/// PDEContainer 元素（marked-content 分组：BMC/BDC..EMC）。
#[derive(Debug, Clone, PartialEq)]
pub struct PdeContainer {
    pub tag: Vec<u8>,
    /// BDC 的属性操作数（名字或字典；BMC = None）。
    pub properties: Option<Operand>,
    pub content: PdeContent,
}

/// PDEPlace 元素（MP/DP 标记点——无绘制、无内容域）。
#[derive(Debug, Clone, PartialEq)]
pub struct PdePlace {
    pub tag: Vec<u8>,
    /// DP 的属性操作数；MP = None。
    pub properties: Option<Operand>,
}

/// PDEShading 元素（`/Name sh`）。
#[derive(Debug, Clone, PartialEq)]
pub struct PdeShading {
    pub gstate: GState,
    pub name: Vec<u8>,
}

/// PDEUnknown 元素（未识别算子 + 操作数——写回逐字节原样重现，
/// Adobe `PDEUnknownGetOpName` 面）。
#[derive(Debug, Clone, PartialEq)]
pub struct PdeUnknown {
    pub op: Vec<u8>,
    pub operands: Vec<Operand>,
}

/// PDEElement——display list 元素（Adobe 基类的 enum 承载；公共方法
/// 面见下方 impl）。
#[derive(Debug, Clone, PartialEq)]
pub enum PdeElement {
    Text(PdeText),
    Path(PdePath),
    Image(PdeImage),
    XObject(PdeXObject),
    Container(PdeContainer),
    Place(PdePlace),
    Shading(PdeShading),
    Unknown(PdeUnknown),
}

impl PdeElement {
    /// `PDEObjectGetType` 的名字形态（Adobe 类名对齐，诊断/审计用）。
    pub fn type_name(&self) -> &'static str {
        match self {
            PdeElement::Text(_) => "PDEText",
            PdeElement::Path(_) => "PDEPath",
            PdeElement::Image(_) => "PDEImage",
            PdeElement::XObject(_) => "PDEXObject",
            PdeElement::Container(_) => "PDEContainer",
            PdeElement::Place(_) => "PDEPlace",
            PdeElement::Shading(_) => "PDEShading",
            PdeElement::Unknown(_) => "PDEUnknown",
        }
    }

    /// `PDEElementGetMatrix`（绘制时 CTM；无自身态的
    /// Container/Place/Unknown 返回单位阵）。
    pub fn matrix(&self) -> Matrix {
        self.gstate().map(|g| g.ctm).unwrap_or(IDENTITY)
    }

    /// `PDEElementSetMatrix`（重定位元素——路径/图/着色/XObject 直换
    /// CTM；文本直换 CTM = 整块平移旋转（run 的 Tm 不动，Adobe 同义）；
    /// Container/Place/Unknown 无绘制态，no-op）。
    pub fn set_matrix(&mut self, m: Matrix) {
        if let Some(g) = self.gstate_mut() {
            g.ctm = m;
        }
    }

    /// 元素级图形状态（无自身态的标记类元素 = None）。
    pub fn gstate(&self) -> Option<&GState> {
        match self {
            PdeElement::Text(e) => Some(&e.gstate),
            PdeElement::Path(e) => Some(&e.gstate),
            PdeElement::Image(e) => Some(&e.gstate),
            PdeElement::XObject(e) => Some(&e.gstate),
            PdeElement::Shading(e) => Some(&e.gstate),
            _ => None,
        }
    }

    /// 元素级图形状态（可变）。
    pub fn gstate_mut(&mut self) -> Option<&mut GState> {
        match self {
            PdeElement::Text(e) => Some(&mut e.gstate),
            PdeElement::Path(e) => Some(&mut e.gstate),
            PdeElement::Image(e) => Some(&mut e.gstate),
            PdeElement::XObject(e) => Some(&mut e.gstate),
            PdeElement::Shading(e) => Some(&mut e.gstate),
            _ => None,
        }
    }

    /// `PDEElementGetBBox` 的批①覆盖面：路径（段顶点 × CTM）与
    /// inline 图（单位方格 × CTM）。文本需要 PDEFont 度量、XObject
    /// 需要资源表 BBox、shading 需要 shading 字典——均归批②/③，
    /// 返回 None（诚实降级，不猜）。
    pub fn bbox(&self) -> Option<Rect> {
        match self {
            PdeElement::Path(p) => path_bbox(&p.segs, &p.gstate.ctm),
            PdeElement::Image(img) => {
                // inline 图：图像空间 [0,1]² 经 CTM 落位（PDF §8.3.3）。
                let m = img.gstate.ctm;
                let corners = [
                    mat_apply(m, Point { x: 0.0, y: 0.0 }),
                    mat_apply(m, Point { x: 1.0, y: 0.0 }),
                    mat_apply(m, Point { x: 0.0, y: 1.0 }),
                    mat_apply(m, Point { x: 1.0, y: 1.0 }),
                ];
                rect_from_points(&corners)
            }
            _ => None,
        }
    }
}

/// 路径段顶点集合 → 轴对齐包围盒（控制点计入——上界盒，Adobe BBox
/// 同为近似面；`re` 展开、曲线取控制点凸包）。
fn path_bbox(segs: &[PathSeg], ctm: &Matrix) -> Option<Rect> {
    let mut pts: Vec<Point> = Vec::new();
    for s in segs {
        match *s {
            PathSeg::MoveTo { x, y } | PathSeg::LineTo { x, y } => {
                pts.push(mat_apply(*ctm, Point { x, y }));
            }
            PathSeg::CurveTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => {
                for (px, py) in [(c1x, c1y), (c2x, c2y), (x, y)] {
                    pts.push(mat_apply(*ctm, Point { x: px, y: py }));
                }
            }
            PathSeg::CurveToV { c2x, c2y, x, y } => {
                for (px, py) in [(c2x, c2y), (x, y)] {
                    pts.push(mat_apply(*ctm, Point { x: px, y: py }));
                }
            }
            PathSeg::CurveToY { c1x, c1y, x, y } => {
                for (px, py) in [(c1x, c1y), (x, y)] {
                    pts.push(mat_apply(*ctm, Point { x: px, y: py }));
                }
            }
            PathSeg::Close => {}
            PathSeg::Rect { x, y, w, h } => {
                for (px, py) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
                    pts.push(mat_apply(*ctm, Point { x: px, y: py }));
                }
            }
        }
    }
    rect_from_points(&pts)
}

fn rect_from_points(pts: &[Point]) -> Option<Rect> {
    if pts.is_empty() {
        return None;
    }
    let mut x0 = f32::MAX;
    let mut y0 = f32::MAX;
    let mut x1 = f32::MIN;
    let mut y1 = f32::MIN;
    for p in pts {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    Rect::new(x0, y0, x1, y1).ok()
}

/// PDEContent——"the workhorse of the PDFEdit API"：解析后的可修改
/// display list（元素序列；marked-content 嵌套在 Container 元素内）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PdeContent {
    pub elements: Vec<PdeElement>,
}

impl PdeContent {
    /// `PDEContentCreate`（空内容）。
    pub fn new() -> Self {
        PdeContent {
            elements: Vec::new(),
        }
    }

    /// `PDEContentGetNumElems`。
    pub fn num_elems(&self) -> usize {
        self.elements.len()
    }

    /// `PDEContentGetElem`。
    pub fn elem(&self, index: usize) -> Option<&PdeElement> {
        self.elements.get(index)
    }

    /// `PDEContentGetElem`（可变）。
    pub fn elem_mut(&mut self, index: usize) -> Option<&mut PdeElement> {
        self.elements.get_mut(index)
    }

    /// `PDEContentAddElem(content, addAfterIndex, elem)`：
    /// `add_after_index` = [`PDE_BEFORE_FIRST`]（-1）插到开头；
    /// `0..num-1` 插到该序号元素之后；`>= num` 追加到末尾（越界
    /// 宽收——Adobe 文档未定义该行为，此处取追加并在此登记）。
    pub fn add_elem(&mut self, add_after_index: i32, elem: PdeElement) {
        if add_after_index < 0 {
            // 哨兵 -1 与 < -1 均 = 插到开头（防御性宽收——Adobe 仅定义
            // -1 哨兵 kPDEBeforeFirst）。
            self.elements.insert(0, elem);
        } else {
            let at = (add_after_index as usize + 1).min(self.elements.len());
            self.elements.insert(at, elem);
        }
    }

    /// `PDEContentRemoveElem`（返回被移除元素；越界 = None）。
    pub fn remove_elem(&mut self, index: usize) -> Option<PdeElement> {
        if index < self.elements.len() {
            Some(self.elements.remove(index))
        } else {
            None
        }
    }

    /// `PDEContentCreateFromCosObj` 的内容流面：字节 → display list。
    /// （对象图/资源表解引用归批②。）
    pub fn parse(bytes: &[u8]) -> Result<PdeContent, PdeError> {
        super::parser::parse_content(bytes)
    }

    /// `PDEContentToCosObj` 的内容流面：display list → 内容流字节
    /// （重生成语义——q/Q 展平为逐元素快照差分；资源合并归批②）。
    pub fn write(&self) -> Vec<u8> {
        super::writer::write_content(self)
    }

    /// 元素深拷贝列表（遍历面——`PDEContentEnumElems` 的 Rust 迭代
    /// 形态；克隆成本可接受，PDE 层编辑流典型量级 < 数百元素）。
    pub fn iter_elems(&self) -> impl Iterator<Item = &PdeElement> {
        self.elements.iter()
    }
}
