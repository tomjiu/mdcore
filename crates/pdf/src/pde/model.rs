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

    /// `PDEColorSpaceGetNumComps`（批③）——色彩空间的分量数。
    ///
    /// Adobe："Calculates the number of components in a color space"
    ///——`scn` 操作数校验面（分量数对不上 = 操作数错）。设备空间
    /// 四件 + 1.3 六空间直读；资源名（/CS0）走 [`ResourceDict`] 的
    /// `/ColorSpace` 表（表驱动面归资源层）；Pattern = None（Adobe
    /// 官方「supports all PDF version 1.3 color spaces **except the
    /// Pattern color space**」——`scn` 的 Pattern 变体分量面归
    /// PDEPattern 批④）。
    pub fn num_comps(&self) -> Option<usize> {
        space_num_comps(&self.space)
    }
}

/// 空间名 → 分量数（批③覆盖面：1.3 六空间 + 设备四件；Indexed 的
/// 分量数 = base 空间的分量数——由 `IndexedColorSpace::num_comps` 承载）。
fn space_num_comps(space: &[u8]) -> Option<usize> {
    match space {
        b"DeviceGray" | b"G" => Some(1),
        b"DeviceRGB" | b"RGB" => Some(3),
        b"DeviceCMYK" | b"CMYK" => Some(4),
        b"CalGray" => Some(1),
        b"CalRGB" => Some(3),
        b"Lab" => Some(3),
        b"ICCBased" => Some(3), // ICC 流的 /N（1/3/4）——批③保形直读缺省 3，N≠3 归引擎面
        b"Separation" | b"DeviceN" => Some(1), // tint 分量近似面（DeviceN 多分量归后续）
        b"Pattern" => None,     // Adobe 官方边界——Pattern 不进批③
        _ => None,              // 资源名（/CS0 等）——表驱动，归 ResourceDict 面
    }
}

/// Indexed 色彩空间（批③——`PDEColorSpaceGetBase/GetHiVal/GetCTable`
/// 三件套）。
///
/// PDF 形态：`[/Indexed /DeviceRGB 7 <lookup 表>]`。Adobe 语义钉：
/// **hiVal + 1 = 实际表项数**（"the actual number of entries is
/// hiVal + 1"——冗余检查面：lookup 字节数 < (hiVal+1)×base 分量数 =
/// 损坏表，typed 错误语义由调用方承载）。
#[derive(Debug, Clone, PartialEq)]
pub struct IndexedColorSpace {
    /// base 空间名（`GetBase`——"helper routine for indexed color
    /// spaces"）。
    pub base: Vec<u8>,
    /// 最高索引（`GetHiVal`——0..=255）。
    pub hi_val: u8,
    /// lookup 表（原始字节——`GetCTable`"component information for an
    /// indexed color space"）。
    pub lookup: Vec<u8>,
}

impl IndexedColorSpace {
    /// `GetBaseNumComps`——base 空间分量数（Indexed 本身是单分量
    /// 索引空间——`scn` 的操作数 = 1，但**查表值**是 base 分量数）。
    pub fn base_num_comps(&self) -> Option<usize> {
        space_num_comps(&self.base)
    }

    /// **hiVal + 1 冗余检查**（Adobe 语义钉）：lookup 字节数是否足够
    /// 覆盖 (hi_val+1) × base 分量数（设备空间 base 缩写名同收）。
    pub fn lookup_is_consistent(&self) -> bool {
        match self.base_num_comps() {
            Some(n) => self.lookup.len() >= (self.hi_val as usize + 1) * n,
            None => true, // base 未知（资源名）——表检查降级
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

    // ===== 批③编辑深面（issue #162——Adobe kPDETextChar 双寻址） =====

    /// run 的字符计数（`PDETextRunGetNumChars`；与字节区分：多字节
    /// 字体下 chars = bytes / num_code_bytes——本层无字体上下文时
    /// 按单字节即字节数；**语义钉**：调用方持
    /// [`crate::pde::font::PdeFont`] 时应先 `num_code_bytes` 切片）。
    pub fn run_num_chars(&self, i: usize) -> Option<usize> {
        self.runs.get(i).map(|r| run_bytes(r).len())
    }

    /// `PDETextGetNumChars`——全 run 字符累加。
    pub fn num_chars(&self) -> usize {
        self.runs.iter().map(|r| run_bytes(r).len()).sum()
    }

    /// `PDETextRunGetCharOffset`——run 首字符的全局字符偏移（前序
    /// runs 累加）。
    pub fn run_char_offset(&self, i: usize) -> Option<usize> {
        if i >= self.runs.len() {
            return None;
        }
        Some(
            self.runs[..i]
                .iter()
                .map(|r| run_bytes(r).len())
                .sum::<usize>(),
        )
    }

    /// `PDETextGetRunForChar`（"Gets the index of the text run that
    /// contains the nth character in a text object"）。
    pub fn run_for_char(&self, char_index: usize) -> Option<usize> {
        let mut acc = 0usize;
        for (i, r) in self.runs.iter().enumerate() {
            let n = run_bytes(r).len();
            if char_index < acc + n {
                return Some(i);
            }
            acc += n;
        }
        None // 越界
    }

    /// `PDETextReplaceChars`——**语义钉**：字符数守恒（Adobe 原文
    /// "does not change the number of characters in the text object;
    /// extra characters are ignored"）。
    ///
    /// `char_index` 为**全局字符偏移**（kPDETextChar 面）；跨 run 逐
    /// 段替换。`replacement` 更长 = 尾部截断（多余忽略）；更短 = 只
    /// 替换前 `replacement.len()` 字符（短侧对称宽容——其余原样）。
    pub fn replace_chars(&mut self, char_index: usize, replacement: &[u8]) {
        let mut ci = char_index;
        let mut rem = replacement;
        while !rem.is_empty() {
            let run = match self.run_for_char(ci) {
                Some(r) => r,
                None => break,
            };
            let off = ci - self.run_char_offset(run).unwrap_or(0);
            let run_len = run_bytes(&self.runs[run]).len();
            let n = rem.len().min(run_len - off);
            if n == 0 {
                break;
            }
            rewrite_run_bytes(&mut self.runs[run], off, &rem[..n]);
            ci += n;
            rem = &rem[n..];
        }
    }

    /// `PDETextSplitRunAt`——**语义钉**：`split_loc` 为**全局字符索引**
    ///（Adobe 原文 "relative to the text object"——非 run 局部索引）。
    ///
    /// 边界判读（Adobe 原文 "The first text run is from character index
    /// 0 **up to splitLoc**. The second text run is **from splitLoc + 1**
    /// to the end"）：首 run **含** splitLoc 位字符（「up to」含判读 +
    /// 次 run 从 splitLoc+1 起——无字符丢失的自洽解释；splitLoc 字符
    /// 丢失的其他判读与「Run 不无故吞字」矛盾，弃）。
    ///
    /// 两 run 继承同一 state/text_matrix（仅断 run 边界）。无操作面：
    /// `split_loc == 0`、`split_loc >= num_chars()`、splitLoc 为 run 尾
    /// 字符（次 run 空——无边界可断）。
    pub fn split_run_at(&mut self, split_loc: usize) {
        if split_loc == 0 || split_loc >= self.num_chars() {
            return;
        }
        let run = match self.run_for_char(split_loc) {
            Some(r) => r,
            None => return,
        };
        let off = split_loc - self.run_char_offset(run).unwrap_or(0);
        // 切点 = off + 1（首段含 splitLoc 位字符）。
        let cut = off + 1;
        if cut >= run_bytes(&self.runs[run]).len() {
            return; // splitLoc 为 run 尾字符——无边界可断
        }
        let (a, b) = split_run_show_bytes(&self.runs[run], cut);
        let state = self.runs[run].state.clone();
        let matrix = self.runs[run].text_matrix;
        self.runs[run].shows = a;
        let next = TextRun {
            state,
            text_matrix: matrix,
            shows: b,
        };
        self.runs.insert(run + 1, next);
    }

    /// `PDETextRemove`（kPDETextRun 形态）：run 索引删除 `count` 个。
    pub fn remove_runs(&mut self, index: usize, count: usize) {
        if index >= self.runs.len() {
            return;
        }
        let end = (index + count).min(self.runs.len());
        self.runs.drain(index..end);
    }

    /// `PDETextRemove`（kPDETextChar 形态）：全局字符区间删除
    /// `[char_index, char_index + count)`。跨 run 逐段裁剪；清空的
    /// run 移除。
    ///
    /// **坐标语义钉**：`char_index`/`count` 是**调用时**坐标（Adobe
    /// (flags, index, count) 同语义）。删除过程中序列收缩，后续段的
    /// 定位必须按**原始坐标**推算——先对全部 run 一次性计算各自删除
    /// 段（原始坐标），再统一应用（边删边查会把已收缩序列当原坐标
    /// 解释，区间错位——批③实现期踩坑，机器模拟暴露）。
    pub fn remove_chars(&mut self, char_index: usize, count: usize) {
        if count == 0 {
            return;
        }
        let start = char_index.min(self.num_chars());
        let end = (char_index + count).min(self.num_chars());
        if start >= end {
            return;
        }
        // 第一遍：原始坐标下的每 run 删除段 [(run, off, take)]
        let cuts: Vec<(usize, usize, usize)> = {
            let mut c = Vec::new();
            let mut run_off = 0usize; // 当前 run 首字符的全局偏移
            for (i, r) in self.runs.iter().enumerate() {
                let rl = run_bytes(r).len();
                if run_off + rl <= start {
                    run_off += rl;
                    continue; // 全在删除区间前
                }
                if run_off >= end {
                    break; // 全在删除区间后
                }
                let off = start.max(run_off) - run_off;
                let take = (end - start.max(run_off)).min(rl - off);
                c.push((i, off, take));
                run_off += rl;
            }
            c
        };
        // 第二遍：倒序应用（run 索引不因前段删除错位）
        for &(run, off, take) in cuts.iter().rev() {
            delete_run_bytes(&mut self.runs[run], off, take);
            if run_bytes(&self.runs[run]).is_empty() {
                self.runs.remove(run);
            }
        }
    }

    // ===== 批③度量面（issue #162——PDEFont 度量 × 矩阵换算） =====

    /// `PDETextGetAdvance`（run 面）："The advance width is the amount
    /// by which the current point advances when the character is
    /// drawn"——run 内字符宽度累加（字符空间）。
    ///
    /// `font` 供 `width_of`（1000 EM → 字符空间 = × font_size/1000 ×
    /// h_scale/100 + Tc；Tw 只作用于单字节编码的字形码 32——PDF
    /// §5.2.2 多字节编码下 word spacing 不适用）。`None` = 无字体
    /// 上下文 → 返回 None（诚实降级——批①文本 BBox 登记的同一偏差
    /// 面，本方法不猜）。
    ///
    /// **#195 修复**：按 [`ShowOp`] 序列流式累计（不再 `run_bytes`
    /// 拼接丢 Kern 后逐 u8 查宽）：
    /// - `ShowOp::Str` 段按**码长**切分字符码（Type0 = 2 字节 CID、
    ///   简单字体 = 1 字节——`num_code_bytes` 口径），逐码查
    ///   `width_of`（复合字体查 CID 而非逐 u8 字节）；
    /// - `ShowOp::Kern(k)`（TJ 数组数值项）按 PDF §9.4.3 直接平移
    ///   当前点：`k × font_size/1000`（千分之一文本空间单位 →
    ///   字符空间；符号保留——负值紧排同式）。TJ 值不受 Tz/Tc/Tw
    ///   影响（规范语义——缩放与字距只作用于字形宽度）。
    /// - 尾部残段（Str 长非码长整数倍）= 解析层不可信形态：**不猜**
    ///   （丢弃残字节并返回 None——比貌似可靠的错误宽度诚实）。
    pub fn run_advance(&self, i: usize, font: Option<&crate::pde::font::PdeFont>) -> Option<f32> {
        let run = self.runs.get(i)?;
        let f = font?;
        let st = &run.state;
        let single = f.num_code_bytes(&run_bytes(run)) == 1;
        let code_len = f.num_code_bytes(&run_bytes(run));
        let mut adv = 0.0f32;
        for op in &run.shows {
            match op {
                ShowOp::Kern(k) => {
                    // TJ 数组数值项：千分之一文本空间单位（不受
                    // Tz/Tc/Tw 缩放——只乘字号换算到字符空间）。
                    // PDF §9.4.3：正数把后续字形向**左**移（横向排
                    // 版——紧排），即对 advance 为**负**贡献；负数
                    // 松排为正贡献（符号取反）。
                    adv -= k * st.size / 1000.0;
                }
                ShowOp::Str(bytes) => {
                    if bytes.len() % code_len != 0 {
                        return None; // 残字节段——不猜（#195 诚实缺能力）
                    }
                    for chunk in bytes.chunks_exact(code_len) {
                        let code: u32 = if code_len == 1 {
                            chunk[0] as u32
                        } else if code_len == 2 {
                            u16::from_be_bytes([chunk[0], chunk[1]]) as u32
                        } else {
                            // 码长 >2 的可变 CMap 形态（批③未承载）
                            // ——明确缺能力，不猜。
                            return None;
                        };
                        let w1000 = f.width_of(code);
                        adv += w1000 * st.size / 1000.0 * st.h_scale / 100.0 + st.char_spacing;
                        if single && code == 32 {
                            adv += st.word_spacing;
                        }
                    }
                }
            }
        }
        Some(adv)
    }

    /// `PDETextGetBBox`（run 面）——批①登记偏差的闭合路径：字体
    /// ascent/descent（基线系）+ run 宽度，经 text_matrix × ctm 落到
    /// 页面空间。
    ///
    /// **近似面诚实登记**：Ascent/Descent 是全字形域上界（Adobe
    /// `PDETextGetQuad` 的 per-glyph 面更精确——per-glyph 需逐字
    /// 宽度换算，批③以 run 级 BBox 承载 + 本注释登记差异）。
    pub fn run_bbox(&self, i: usize, font: Option<&crate::pde::font::PdeFont>) -> Option<Rect> {
        let run = self.runs.get(i)?;
        let f = font?;
        let asc = f.attrs.ascent.unwrap_or(0.0);
        let desc = f.attrs.descent.unwrap_or(0.0);
        let st = &run.state;
        let adv = self.run_advance(i, font)?;
        // 字符空间盒（基线原点 → 右端；Ts 基线抬升。descent 存绝对值
        // ——基线**下方**偏移取负）。
        let corners_cs = [
            (0.0, st.rise),
            (adv, st.rise),
            (0.0, asc * st.size / 1000.0 + st.rise),
            (adv, st.rise - desc * st.size / 1000.0),
        ];
        // 文本空间 → 用户空间（Tm）→ 页面空间（CTM）
        let m = mat_mul(run.text_matrix, self.gstate.ctm);
        let pts = corners_cs
            .iter()
            .map(|&(x, y)| mat_apply(m, Point { x, y }))
            .collect::<Vec<_>>();
        rect_from_points(&pts)
    }
}

// ---- run 字节面辅助（show 串的字节级寻址——跨 Str/Kern 串行） ----

/// run 的显示字节拼接（Str 段全串；Kern 不占字符位）。
fn run_bytes(run: &TextRun) -> Vec<u8> {
    run.shows
        .iter()
        .filter_map(|s| match s {
            ShowOp::Str(b) => Some(b.as_slice()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .concat()
}

/// 重写 run 的字节串 `off..off+n` 段为 `new`（重装为单 Str——批③
/// 编辑面的统一重装语义：编辑后 run 的字节串是权威形态）。
fn rewrite_run_bytes(run: &mut TextRun, off: usize, new: &[u8]) {
    let bytes = run_bytes(run);
    let off = off.min(bytes.len());
    let mut out = bytes[..off].to_vec();
    out.extend_from_slice(new);
    out.extend_from_slice(&bytes[(off + new.len()).min(bytes.len())..]);
    run.shows = vec![ShowOp::Str(out)];
}

/// 删除 run 字节串 `off..off+n` 段。
fn delete_run_bytes(run: &mut TextRun, off: usize, n: usize) {
    let bytes = run_bytes(run);
    let off = off.min(bytes.len());
    let end = (off + n).min(bytes.len());
    let mut out = bytes[..off].to_vec();
    out.extend_from_slice(&bytes[end..]);
    run.shows = vec![ShowOp::Str(out)];
}

/// run 拆分面：保留 Str/Kern 混合原序（split 后两 run 的 shows 各自
/// 收纳界前/界后的 op——Kern 归属按游标位置）。
fn split_run_show_bytes(run: &TextRun, at: usize) -> (Vec<ShowOp>, Vec<ShowOp>) {
    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut cursor = 0usize;
    let mut filled = false; // 游标过界后 op 全进 b
    for op in &run.shows {
        match op {
            ShowOp::Kern(k) => {
                if filled {
                    b.push(ShowOp::Kern(*k));
                } else {
                    a.push(ShowOp::Kern(*k));
                }
            }
            ShowOp::Str(s) => {
                if filled {
                    b.push(ShowOp::Str(s.clone()));
                } else if cursor + s.len() <= at {
                    a.push(ShowOp::Str(s.clone()));
                    cursor += s.len();
                } else {
                    // 跨界 str：切
                    let l = at - cursor;
                    a.push(ShowOp::Str(s[..l].to_vec()));
                    b.push(ShowOp::Str(s[l..].to_vec()));
                    filled = true;
                }
            }
        }
    }
    (a, b)
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

    /// `PDEElementIsAtPoint`（命中测试——选区编辑的产品闭环面）。
    ///
    /// **语义钉**：Container/Place（无 gstate 的非图形元素）与无 BBox
    /// 面 = None→false（Adobe `GetClip` 同款边界：这两类「should not
    /// be expected to return valid results」）。
    pub fn is_at_point(&self, x: f32, y: f32) -> bool {
        match self.bbox() {
            Some(r) => x >= r.x0() && x <= r.x1() && y >= r.y0() && y <= r.y1(),
            None => false,
        }
    }

    /// `PDEElementIsAtRect`（"Tests whether any **part** of a rectangle
    /// is on the element"——相交判定的 hit 面）。
    pub fn is_at_rect(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> bool {
        match self.bbox() {
            Some(r) => x0 <= r.x1() && x1 >= r.x0() && y0 <= r.y1() && y1 >= r.y0(),
            None => false,
        }
    }

    /// `PDEElementCopy`（"Makes a copy of an element"——撤销栈/剪贴板
    /// 的基元操作）。
    pub fn deep_copy(&self) -> PdeElement {
        self.clone()
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

#[cfg(test)]
mod batch3_tests {
    use super::*;

    fn run(text: &[u8]) -> TextRun {
        TextRun {
            state: TextState::default(),
            text_matrix: IDENTITY,
            shows: vec![ShowOp::Str(text.to_vec())],
        }
    }

    fn text3(a: &[u8], b: &[u8], c: &[u8]) -> PdeText {
        PdeText {
            gstate: GState::default(),
            runs: vec![run(a), run(b), run(c)],
        }
    }

    /// 三方法闭环：RunGetCharOffset / GetRunForChar / GetNumChars
    ///（语义锚 1.1——run→char 双向换算）。
    #[test]
    fn char_addressing_roundtrip() {
        let t = text3(b"AB", b"CDE", b"FGHI");
        assert_eq!(t.num_runs(), 3);
        assert_eq!(t.num_chars(), 9);
        // offset 面
        assert_eq!(t.run_char_offset(0), Some(0));
        assert_eq!(t.run_char_offset(1), Some(2));
        assert_eq!(t.run_char_offset(2), Some(5));
        assert_eq!(t.run_char_offset(3), None); // 越界
                                                // run_for_char 面
        assert_eq!(t.run_for_char(0), Some(0));
        assert_eq!(t.run_for_char(1), Some(0));
        assert_eq!(t.run_for_char(2), Some(1));
        assert_eq!(t.run_for_char(4), Some(1));
        assert_eq!(t.run_for_char(5), Some(2));
        assert_eq!(t.run_for_char(8), Some(2));
        assert_eq!(t.run_for_char(9), None); // 越界
                                             // run_num_chars
        assert_eq!(t.run_num_chars(1), Some(3));
    }

    /// ReplaceChars 语义钉：字符数守恒（多余忽略——Adobe 原文）。
    #[test]
    fn replace_chars_count_conservation() {
        let mut t = text3(b"AB", b"CDE", b"FGHI");
        let before = t.num_chars();
        // 等长替换（跨 run：全局 1..4 换 xyz）
        t.replace_chars(1, b"xyz");
        assert_eq!(t.num_chars(), before, "等长：守恒");
        assert_eq!(t.run_text(0).unwrap(), b"Ax".to_vec());
        assert_eq!(t.run_text(1).unwrap(), b"yzE".to_vec());
        // 长替换：多余忽略（尾截断）
        let mut t2 = text3(b"AB", b"CDE", b"");
        t2.replace_chars(0, b"WXYZ12345");
        assert_eq!(t2.num_chars(), 5, "长替换：多余忽略");
        assert_eq!(t2.run_text(0).unwrap(), b"WX".to_vec());
        assert_eq!(t2.run_text(1).unwrap(), b"YZ1".to_vec());
        // 短替换：前 n 字符替换（短侧对称宽容）
        let mut t3 = text3(b"AB", b"C", b"");
        t3.replace_chars(0, b"Q");
        assert_eq!(t3.run_text(0).unwrap(), b"QB".to_vec());
        assert_eq!(t3.num_chars(), 3, "短替换：其余原样");
    }

    /// SplitRunAt 语义钉：split_loc = **全局字符索引**（非 run 局部）。
    #[test]
    fn split_run_at_global_semantics() {
        let mut t = text3(b"AB", b"CDE", b"");
        // 全局索引 3 = run1 的 "C" 之后（run 局部索引会理解为 run1 内 3 →
        // 越界无操作——反例判别面）
        t.split_run_at(3);
        assert_eq!(t.num_runs(), 4, "全局 3 落在 run1 内部：断成 AB/CD/E");
        assert_eq!(t.run_text(0).unwrap(), b"AB".to_vec());
        assert_eq!(t.run_text(1).unwrap(), b"CD".to_vec());
        assert_eq!(t.run_text(2).unwrap(), b"E".to_vec());
        assert_eq!(t.run_text(3).unwrap(), b"".to_vec());
        // state/matrix 继承
        assert_eq!(t.runs[1].state, t.runs[2].state);
        // 边界无操作面：全局 0 / 全局尾；run 尾字符
        let mut t2 = text3(b"AB", b"CDE", b"");
        t2.split_run_at(0); // 全局首
        assert_eq!(t2.num_runs(), 3);
        t2.split_run_at(5); // 全局尾后
        assert_eq!(t2.num_runs(), 3);
        // run 尾字符（splitLoc=4 = run1 的 "E" 位）：cut=run_len → 无操作
        t2.split_run_at(4);
        assert_eq!(t2.num_runs(), 3);
        // run 首字符（splitLoc=2 = run1 的 "C" 位）：cut=1 → 切 C|DE
        t2.split_run_at(2);
        assert_eq!(t2.num_runs(), 4, "splitLoc 含首段：run1 切成 C|DE");
        assert_eq!(t2.run_text(1).unwrap(), b"C".to_vec());
        assert_eq!(t2.run_text(2).unwrap(), b"DE".to_vec());
    }

    /// Remove 双形态：run 删 / char 全局区间删。
    #[test]
    fn remove_runs_and_chars() {
        // run 形态
        let mut t = text3(b"AB", b"CDE", b"FGHI");
        t.remove_runs(1, 1);
        assert_eq!(t.num_runs(), 2);
        assert_eq!(t.run_text(0).unwrap(), b"AB".to_vec());
        assert_eq!(t.run_text(1).unwrap(), b"FGHI".to_vec());
        // char 形态：跨 run 区间（全局 1..4 → 删 BCD）
        let mut t2 = text3(b"AB", b"CDE", b"FGHI");
        t2.remove_chars(1, 3);
        assert_eq!(t2.num_chars(), 6);
        assert_eq!(t2.run_text(0).unwrap(), b"A".to_vec());
        assert_eq!(t2.run_text(1).unwrap(), b"E".to_vec());
        assert_eq!(t2.run_text(2).unwrap(), b"FGHI".to_vec());
        // 整 run 清空 → run 移除
        let mut t3 = text3(b"AB", b"CDE", b"FGHI");
        t3.remove_chars(2, 3);
        assert_eq!(t3.num_runs(), 2, "run1 整删");
        assert_eq!(t3.run_text(0).unwrap(), b"AB".to_vec());
        assert_eq!(t3.run_text(1).unwrap(), b"FGHI".to_vec());
    }

    /// Kern 位移不占字符位（run_bytes 只串 Str 段——字节寻址语义钉）。
    #[test]
    fn kern_not_counted_as_chars() {
        let r = TextRun {
            state: TextState::default(),
            text_matrix: IDENTITY,
            shows: vec![
                ShowOp::Str(b"AB".to_vec()),
                ShowOp::Kern(-120.0),
                ShowOp::Str(b"CD".to_vec()),
            ],
        };
        let t = PdeText {
            gstate: GState::default(),
            runs: vec![r],
        };
        assert_eq!(t.num_chars(), 4);
        // Kern 跨界拆分：split 在 A|B 之间——Kern 归属后 run（游标过界）
        let mut t2 = t.clone();
        t2.split_run_at(1);
        // splitLoc=1（"B" 位）含首段：首 run 含 AB（字节切点在 B 后）
        assert_eq!(t2.run_text(0).unwrap(), b"AB".to_vec());
        assert_eq!(t2.run_text(1).unwrap(), b"CD".to_vec());
        // Kern 在 show 序里的位置保形（界前 Kern 归前 run）
        let mut t3 = PdeText {
            gstate: GState::default(),
            runs: vec![TextRun {
                state: TextState::default(),
                text_matrix: IDENTITY,
                shows: vec![
                    ShowOp::Kern(-50.0),
                    ShowOp::Str(b"AB".to_vec()),
                    ShowOp::Str(b"CD".to_vec()),
                ],
            }],
        };
        t3.split_run_at(2);
        assert_eq!(t3.runs[0].shows.len(), 3, "界前 Kern+两 Str 归前 run");
        assert_eq!(t3.runs[1].shows.len(), 1);
    }
}
