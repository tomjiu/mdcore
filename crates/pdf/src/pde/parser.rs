//! pde::parser —— 内容流 → PDE 元素树（批①第三件）。
//!
//! Adobe 参照：`PDEContentCreateFromCosObj` 的语义面——标记流 →
//! display list。核心状态机：
//!
//! - **图形态域**（q/Q/cm/cs/CS/sc/scn/SC/SCN/g/G/rg/RG/k/K/w/W/W*/
//!   gs）：批①策略 = **q/Q 展平**——每个绘制元素的 [`GState`] 快照吸收
//!   到位（Adobe PDEContentToCosObj 写回同样重生成内容流，语义等价；
//!   round-trip 测试锁定）。
//! - **文本域**（BT..ET）：Tf/Tc/Tw/Tz/TL/Ts/Tr 建文本状态（文本状态
//!   属图形态域——q/Q 快照/恢复含此，且跨 BT/ET 持续：BT 只重置
//!   Tm/Tlm）；Tm/Td/TD/T* 推进文本矩阵；Tj/TJ/'/" 产 run。**run 合并
//!   判据**（Adobe："PDFEdit combines text from multiple Tj operators
//!   into a single text run, when possible"）：文本状态逐字段相等
//!   **且**文本矩阵相等——属性/定位算子介入即断开。
//! - **路径域**（m/l/c/v/y/h/re + W/W* + 12 paint 算子）：当前子路径段
//!   序列 + clip 待挂标志；paint 算子触发元素产出（一 paint = 一
//!   PdePath 元素，Adobe 同构——path 对象由 paint 算子定界）。
//! - **inline image**（BI..ID..EI）：BI 后属性表由标记流组装（name-
//!   value 对），ID 后到 EI 为原始数据（经 [`LexOutput::inline_data`]
//!   直读原文——二进制数据不进标记流，避免误词法）。
//! - **marked content**（BMC/BDC/EMC）：栈式嵌套 → PdeContainer。
//! - **Do/sh**：名字引用形态元素（资源表解引用归批②）。
//! - **未识别算子**：PdeUnknown 保留（操作数按词法形态）。
//!
//! 裁剪语义（PDF §8.5.4 → Adobe PDEClip）：`W/W*` 使 paint 后的路径
//! 成为**后续**元素的 clip（`n` 无绘制算子常配 W 做纯裁剪）。本层：
//! clip 挂到后续**每个**绘制元素的 GState（BT/ET 不清 clip——PDF
//! clip 跨 BT 持续，Adobe PDEText "same clip for all sub-elements"
//! 同义）。

use super::lexer::{lex, LexOutput, Token};
use super::model::*;

/// 解析主入口：内容流字节 → display list。
pub fn parse_content(bytes: &[u8]) -> Result<PdeContent, PdeError> {
    let lx = lex(bytes)?;
    Parser {
        lx: &lx,
        bytes,
        elements: Vec::new(),
        gstack: vec![GState::default()],
        in_text: false,
        tm: IDENTITY,
        tlm: IDENTITY,
        cur_text: None,
        segs: Vec::new(),
        cur_start: Option::None,
        clip_pending: Option::None,
        containers: Vec::new(),
        inline_seen: 0,
    }
    .run()
}

struct Parser<'a> {
    lx: &'a LexOutput,
    bytes: &'a [u8],
    elements: Vec<PdeElement>,
    /// 图形态栈（栈底 = 页初始态；q push / Q pop——展平策略下仅作
    /// 解析期状态机，不进模型）。
    gstack: Vec<GState>,
    // 文本域
    in_text: bool,
    tm: Matrix,
    tlm: Matrix,
    /// 当前 BT 块的累积 PdeText（含进入时 gstate 快照）。
    cur_text: Option<PdeText>,
    // 路径域
    segs: Vec<PathSeg>,
    /// 当前子路径起点（CurveToV 隐含首控制点用）。
    cur_start: Option<(f32, f32)>,
    clip_pending: Option<Clip>,
    // marked-content 嵌套帧（解析期栈）
    containers: Vec<ContainerFrame>,
    /// 已消费的 inline image 数据域计数（与 lx.inline_spans 对位）。
    inline_seen: usize,
}

impl<'a> Parser<'a> {
    fn gcur(&self) -> &GState {
        self.gstack.last().expect("gstack 非空（构造含栈底）")
    }

    fn gcur_mut(&mut self) -> &mut GState {
        self.gstack.last_mut().expect("gstack 非空")
    }

    fn run(mut self) -> Result<PdeContent, PdeError> {
        let tokens = &self.lx.tokens;
        let n = tokens.len();
        let mut i = 0usize;
        // 操作数缓冲（操作符前累积；`[`/`]`/`<<`/`>>` 结构算子直接
        // 压数组/字典构建栈）
        let mut ops: Vec<Operand> = Vec::new();
        // 数组/字典构建栈（`[` 与 `<<` 压层，`]`/`>>` 收层——内容流里
        // 数组仅出现在 TJ 与 inline image 属性值；字典仅出现在 BDC/DP
        // 属性，但词法层不区分，统一按栈构建）
        let mut stack: Vec<Operand> = Vec::new();
        while i < n {
            match &tokens[i] {
                Token::Operand(o) => {
                    push_operand(&mut stack, &mut ops, o.clone());
                    i += 1;
                }
                Token::Op(op) => {
                    let opname = String::from_utf8_lossy(op).into_owned();
                    match opname.as_str() {
                        "[" => {
                            stack.push(Operand::Array(Vec::new()));
                            i += 1;
                        }
                        "]" => {
                            // 收数组：弹出已收集的 Array 哨兵层，作为
                            // 操作数交回上层（父层为数组则继续进层，
                            // 栈空则落 ops）
                            match stack.pop() {
                                Some(Operand::Array(items)) => {
                                    push_operand(&mut stack, &mut ops, Operand::Array(items));
                                }
                                Some(other) => {
                                    // [ 与 << 交错（病态流）——宽收：把
                                    // 字典层降为普通操作数
                                    push_operand(&mut stack, &mut ops, other);
                                    let leftovers = std::mem::take(&mut ops);
                                    push_operand(&mut stack, &mut ops, Operand::Array(Vec::new()));
                                    // leftovers 并回降级数组层（尽力保数据）
                                    if let Some(Operand::Array(items)) = stack.last_mut() {
                                        items.extend(leftovers);
                                    }
                                }
                                None => {
                                    return Err(PdeError::Parse {
                                        op: "]".into(),
                                        reason: "] without [（数组未开启）".into(),
                                    });
                                }
                            }
                            i += 1;
                        }
                        "<<" => {
                            stack.push(Operand::Dict(Vec::new()));
                            i += 1;
                        }
                        ">>" => {
                            // 收字典：弹出已收集的 Dict 哨兵层
                            match stack.pop() {
                                Some(Operand::Dict(pairs)) => {
                                    push_operand(&mut stack, &mut ops, Operand::Dict(pairs));
                                }
                                Some(other) => {
                                    // << 与 [ 交错（病态流）——宽收：把
                                    // 数组层降为普通操作数
                                    push_operand(&mut stack, &mut ops, other);
                                    let leftovers = std::mem::take(&mut ops);
                                    push_operand(&mut stack, &mut ops, Operand::Dict(Vec::new()));
                                    // leftovers 并回降级数组层（尽力保数据）
                                    if let Some(Operand::Dict(pairs)) = stack.last_mut() {
                                        for (k, v) in
                                            leftovers.into_iter().map(|v| (Operand::Null, v))
                                        {
                                            pairs.push((k, v));
                                        }
                                    }
                                }
                                None => {
                                    return Err(PdeError::Parse {
                                        op: ">>".into(),
                                        reason: ">> without <<（字典未开启）".into(),
                                    });
                                }
                            }
                            i += 1;
                        }
                        _ => {
                            self.dispatch(&opname, &mut ops, &mut i)?;
                            i += 1;
                        }
                    }
                }
            }
        }
        // 未闭合的 [ / <<（病态流）——按宽收把残留 ops 抛给未知算子面
        if !ops.is_empty() {
            self.elements.push(PdeElement::Unknown(PdeUnknown {
                op: b"<eof-unclosed-structure>".to_vec(),
                operands: ops,
            }));
        }
        // EOF 收口：未闭合 BT 或未 paint 的路径——按宽收语义把已有内容
        // 产出（残缺比丢失好——Adobe 同容错面），gstack 不必配平（已展平）。
        self.flush_text();
        self.flush_path(PaintOp::NoPaint, false);
        Ok(PdeContent {
            elements: self.elements,
        })
    }

    fn dispatch(
        &mut self,
        opname: &str,
        ops: &mut Vec<Operand>,
        i: &mut usize,
    ) -> Result<(), PdeError> {
        match opname {
            // ===== 图形态域 =====
            "q" => {
                ops.clear();
                let cur = self.gcur().clone();
                self.gstack.push(cur);
            }
            "Q" => {
                ops.clear();
                if self.gstack.len() > 1 {
                    self.gstack.pop();
                }
            }
            "cm" => {
                let m = take_matrix(ops, "cm")?;
                let cur_ctm = self.gcur().ctm;
                self.gcur_mut().ctm = mat_mul(m, cur_ctm);
            }
            "cs" => {
                let name = take_name(ops, "cs")?;
                self.gcur_mut().fill.space = name;
            }
            "CS" => {
                let name = take_name(ops, "CS")?;
                self.gcur_mut().stroke.space = name;
            }
            "sc" | "scn" => {
                let vals = take_nums(ops, opname)?;
                self.gcur_mut().fill.values = ColorValues::Nums(vals);
            }
            "SC" | "SCN" => {
                let vals = take_nums(ops, opname)?;
                self.gcur_mut().stroke.values = ColorValues::Nums(vals);
            }
            "g" => {
                let v = take_f32(ops, "g")?;
                self.gcur_mut().fill = ColorSpec::device_gray(v);
            }
            "G" => {
                let v = take_f32(ops, "G")?;
                self.gcur_mut().stroke = ColorSpec::device_gray(v);
            }
            "rg" => {
                let v = take_rgb(ops, "rg")?;
                self.gcur_mut().fill = ColorSpec {
                    space: b"DeviceRGB".to_vec(),
                    values: ColorValues::Nums(v),
                };
            }
            "RG" => {
                let v = take_rgb(ops, "RG")?;
                self.gcur_mut().stroke = ColorSpec {
                    space: b"DeviceRGB".to_vec(),
                    values: ColorValues::Nums(v),
                };
            }
            "k" => {
                let v = take_cmyk(ops, "k")?;
                self.gcur_mut().fill = ColorSpec {
                    space: b"DeviceCMYK".to_vec(),
                    values: ColorValues::Nums(v),
                };
            }
            "K" => {
                let v = take_cmyk(ops, "K")?;
                self.gcur_mut().stroke = ColorSpec {
                    space: b"DeviceCMYK".to_vec(),
                    values: ColorValues::Nums(v),
                };
            }
            "w" => {
                let v = take_f32(ops, "w")?;
                self.gcur_mut().line_width = v;
            }
            "gs" => {
                let name = take_name(ops, "gs")?;
                self.gcur_mut().extgstate = Some(name);
            }
            // ===== 文本域 =====
            "BT" => {
                ops.clear();
                if self.in_text {
                    // 嵌套 BT（病态流）——先收口前块再重开（宽收）
                    self.flush_text();
                }
                self.in_text = true;
                self.tm = IDENTITY;
                self.tlm = IDENTITY;
                self.cur_text = Some(PdeText {
                    gstate: self.gcur().clone(),
                    runs: Vec::new(),
                });
            }
            "ET" => {
                ops.clear();
                if !self.in_text {
                    return Err(PdeError::Parse {
                        op: "ET".into(),
                        reason: "ET without BT（文本对象未开启）".into(),
                    });
                }
                self.flush_text();
                self.in_text = false;
            }
            "Tf" => {
                let (name, size) = take_font(ops, "Tf")?;
                self.tstate().font = Some(name);
                self.tstate().size = size;
            }
            "Tc" => {
                let v = take_f32(ops, "Tc")?;
                self.tstate().char_spacing = v;
            }
            "Tw" => {
                let v = take_f32(ops, "Tw")?;
                self.tstate().word_spacing = v;
            }
            "Tz" => {
                let v = take_f32(ops, "Tz")?;
                self.tstate().h_scale = v;
            }
            "TL" => {
                let v = take_f32(ops, "TL")?;
                self.tstate().leading = v;
            }
            "Ts" => {
                let v = take_f32(ops, "Ts")?;
                self.tstate().rise = v;
            }
            "Tr" => {
                let v = take_i32(ops, "Tr")?;
                self.tstate().render_mode = v;
            }
            "Tm" => {
                let m = take_matrix(ops, "Tm")?;
                self.tm = m;
                self.tlm = m;
            }
            "Td" => {
                let (tx, ty) = take_pair(ops, "Td")?;
                self.tlm = mat_mul(mat_translate(tx, ty), self.tlm);
                self.tm = self.tlm;
            }
            "TD" => {
                let (tx, ty) = take_pair(ops, "TD")?;
                self.tstate().leading = -ty;
                self.tlm = mat_mul(mat_translate(tx, ty), self.tlm);
                self.tm = self.tlm;
            }
            "T*" => {
                let leading = self.tstate().leading;
                self.tlm = mat_mul(mat_translate(0.0, -leading), self.tlm);
                self.tm = self.tlm;
            }
            "Tj" | "'" | "\"" => {
                self.show_text(ops, opname)?;
            }
            "TJ" => {
                self.show_tj_array(ops)?;
            }
            // ===== 路径域 =====
            "m" => {
                let (x, y) = take_pair(ops, "m")?;
                self.cur_start = Some((x, y));
                self.segs.push(PathSeg::MoveTo { x, y });
            }
            "l" => {
                let (x, y) = take_pair(ops, "l")?;
                self.segs.push(PathSeg::LineTo { x, y });
            }
            "c" => {
                let v = take_nums6(ops, "c")?;
                self.segs.push(PathSeg::CurveTo {
                    c1x: v[0],
                    c1y: v[1],
                    c2x: v[2],
                    c2y: v[3],
                    x: v[4],
                    y: v[5],
                });
            }
            "v" => {
                let v = take_nums4(ops, "v")?;
                self.segs.push(PathSeg::CurveToV {
                    c2x: v[0],
                    c2y: v[1],
                    x: v[2],
                    y: v[3],
                });
            }
            "y" => {
                let v = take_nums4(ops, "y")?;
                self.segs.push(PathSeg::CurveToY {
                    c1x: v[0],
                    c1y: v[1],
                    x: v[2],
                    y: v[3],
                });
            }
            "h" => {
                ops.clear();
                self.segs.push(PathSeg::Close);
            }
            "re" => {
                let v = take_nums4(ops, "re")?;
                self.cur_start = Some((v[0], v[1]));
                self.segs.push(PathSeg::Rect {
                    x: v[0],
                    y: v[1],
                    w: v[2],
                    h: v[3],
                });
            }
            "W" => {
                ops.clear();
                self.clip_pending = Some(Clip {
                    segs: self.segs.clone(),
                    even_odd: false,
                });
            }
            "W*" => {
                ops.clear();
                self.clip_pending = Some(Clip {
                    segs: self.segs.clone(),
                    even_odd: true,
                });
            }
            "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" => {
                let paint = match opname {
                    "S" => PaintOp::Stroke,
                    "s" => PaintOp::CloseStroke,
                    "f" | "F" => PaintOp::Fill,
                    "f*" => PaintOp::FillEvenOdd,
                    "B" => PaintOp::FillStroke,
                    "B*" => PaintOp::FillStrokeEvenOdd,
                    "b" => PaintOp::CloseFillStroke,
                    "b*" => PaintOp::CloseFillStrokeEvenOdd,
                    _ => PaintOp::NoPaint,
                };
                ops.clear();
                self.flush_path(paint, true);
            }
            // ===== XObject / Shading / marked content =====
            "Do" => {
                let name = take_name(ops, "Do")?;
                let mut g = self.gcur().clone();
                if let Some(c) = self.clip_pending.take() {
                    g.clip = Some(c);
                }
                self.elements
                    .push(PdeElement::XObject(PdeXObject { gstate: g, name }));
            }
            "sh" => {
                let name = take_name(ops, "sh")?;
                let mut g = self.gcur().clone();
                if let Some(c) = self.clip_pending.take() {
                    g.clip = Some(c);
                }
                self.elements
                    .push(PdeElement::Shading(PdeShading { gstate: g, name }));
            }
            "MP" => {
                let tag = take_name(ops, "MP")?;
                self.elements.push(PdeElement::Place(PdePlace {
                    tag,
                    properties: None,
                }));
            }
            "DP" => {
                let (tag, props) = take_tagged(ops, "DP")?;
                self.elements.push(PdeElement::Place(PdePlace {
                    tag,
                    properties: props,
                }));
            }
            "BMC" => {
                let tag = take_name(ops, "BMC")?;
                self.containers.push(ContainerFrame {
                    tag,
                    properties: None,
                    elements: Vec::new(),
                });
            }
            "BDC" => {
                let (tag, props) = take_tagged(ops, "BDC")?;
                self.containers.push(ContainerFrame {
                    tag,
                    properties: props,
                    elements: Vec::new(),
                });
            }
            "EMC" => {
                ops.clear();
                match self.containers.pop() {
                    Some(frame) => {
                        let elem = PdeElement::Container(PdeContainer {
                            tag: frame.tag,
                            properties: frame.properties,
                            content: PdeContent {
                                elements: frame.elements,
                            },
                        });
                        if let Some(top) = self.containers.last_mut() {
                            top.elements.push(elem);
                        } else {
                            self.elements.push(elem);
                        }
                    }
                    None => {
                        return Err(PdeError::Parse {
                            op: "EMC".into(),
                            reason: "EMC without BDC/BMC（marked-content 未开启）".into(),
                        });
                    }
                }
            }
            // ===== inline image =====
            "BI" => {
                ops.clear();
                self.parse_inline_image(i)?;
            }
            "ID" | "EI" => {
                // BI 之外出现的 ID/EI 算子（病态流）——按未知算子保留
                ops.clear();
            }
            // ===== 未识别算子 → PdeUnknown =====
            _ => {
                let elem = PdeElement::Unknown(PdeUnknown {
                    op: opname.as_bytes().to_vec(),
                    operands: std::mem::take(ops),
                });
                if let Some(top) = self.containers.last_mut() {
                    top.elements.push(elem);
                } else {
                    self.elements.push(elem);
                }
            }
        }
        Ok(())
    }

    /// 当前文本状态（文本状态属图形态域——q/Q 快照/恢复含此）。
    fn tstate(&mut self) -> &mut TextState {
        &mut self.gcur_mut().text
    }

    /// show 算子（Tj/'/"）：操作数 → run 累积。
    fn show_text(&mut self, ops: &mut Vec<Operand>, opname: &str) -> Result<(), PdeError> {
        if !self.in_text {
            return Err(PdeError::Parse {
                op: opname.into(),
                reason: "show 算子在 BT..ET 之外".into(),
            });
        }
        match opname {
            "'" => {
                // 换行 + show：T* 先行
                let leading = self.gcur().text.leading;
                self.tlm = mat_mul(mat_translate(0.0, -leading), self.tlm);
                self.tm = self.tlm;
            }
            "\"" => {
                // 字距/词距 + 换行 + show
                let (aw, ac) = take_pair(ops, "\"")?;
                self.tstate().word_spacing = aw;
                self.tstate().char_spacing = ac;
                let leading = self.gcur().text.leading;
                self.tlm = mat_mul(mat_translate(0.0, -leading), self.tlm);
                self.tm = self.tlm;
            }
            _ => {}
        }
        // 无字体的 show 不可绘制（PDF 语义；模型文档 TextState::font 注释）
        if self.gcur().text.font.is_none() {
            return Err(PdeError::Parse {
                op: opname.into(),
                reason: "show 算子前未设字体（Tf 缺失）".into(),
            });
        }
        let bytes = match ops.last() {
            Some(Operand::Str { bytes, .. }) => bytes.clone(),
            _ => {
                return Err(PdeError::Parse {
                    op: opname.into(),
                    reason: "show 算子缺字符串操作数".into(),
                });
            }
        };
        ops.clear();
        let state = self.gcur().text.clone();
        self.push_run(state, self.tm, vec![ShowOp::Str(bytes)]);
        Ok(())
    }

    /// TJ 数组 show（[str num str …] TJ）。
    fn show_tj_array(&mut self, ops: &mut Vec<Operand>) -> Result<(), PdeError> {
        if !self.in_text {
            return Err(PdeError::Parse {
                op: "TJ".into(),
                reason: "TJ 在 BT..ET 之外".into(),
            });
        }
        // TJ 的操作数形态：数组（直接压栈）或扁平序列（多个数组——
        // 病态流宽收，拼接处理）
        let arr = take_array_flat(ops, "TJ")?;
        let mut shows: Vec<ShowOp> = Vec::new();
        for item in arr {
            match item {
                Operand::Str { bytes, .. } => shows.push(ShowOp::Str(bytes)),
                Operand::Int(k) => shows.push(ShowOp::Kern(k as f32)),
                Operand::Real(k) => shows.push(ShowOp::Kern(k)),
                _ => {
                    return Err(PdeError::Parse {
                        op: "TJ".into(),
                        reason: "TJ 数组内非字符串/数字元素".into(),
                    });
                }
            }
        }
        let state = self.gcur().text.clone();
        self.push_run(state, self.tm, shows);
        Ok(())
    }

    /// run 合并面（Adobe 判据：文本状态逐字段相等且 Tm 相等才合并）。
    fn push_run(&mut self, state: TextState, tm: Matrix, shows: Vec<ShowOp>) {
        let t = self.cur_text.get_or_insert_with(|| PdeText {
            gstate: GState::default(),
            runs: Vec::new(),
        });
        if let Some(last) = t.runs.last_mut() {
            if last.state == state && last.text_matrix == tm {
                last.shows.extend(shows);
                return;
            }
        }
        t.runs.push(TextRun {
            state,
            text_matrix: tm,
            shows,
        });
    }

    /// BT 块收口（ET / 嵌套 BT / EOF）：cur_text → elements（若非空）。
    fn flush_text(&mut self) {
        if let Some(t) = self.cur_text.take() {
            if !t.runs.is_empty() {
                if let Some(top) = self.containers.last_mut() {
                    top.elements.push(PdeElement::Text(t));
                } else {
                    self.elements.push(PdeElement::Text(t));
                }
            }
        }
    }

    /// 路径收口（paint 算子触发）：segs → PdePath 元素（若非空）。
    /// `painting` = 是否绘制语义（false = EOF 残尾收口，产出 NoPaint）。
    fn flush_path(&mut self, paint: PaintOp, _painting: bool) {
        if self.segs.is_empty() {
            return;
        }
        let segs = std::mem::take(&mut self.segs);
        let mut g = self.gcur().clone();
        let clipped = self.clip_pending.is_some();
        if let Some(c) = self.clip_pending.take() {
            // W 路径自身也绘制（path W S 形态）：clip 进元素 GState；
            // W 后 n（纯裁剪）不产出元素——clip 留待后续元素吸收。
            if paint == PaintOp::NoPaint {
                self.clip_pending = Some(c);
                return;
            }
            g.clip = Some(c);
        }
        let elem = PdeElement::Path(PdePath {
            gstate: g,
            segs,
            paint,
            clipped,
        });
        if let Some(top) = self.containers.last_mut() {
            top.elements.push(elem);
        } else {
            self.elements.push(elem);
        }
    }

    /// inline image 解析：BI 后属性表 → ID 数据 → EI。
    /// `*i` 位于 `BI` 算子 token 序号；返回时 `*i` 已指向 EI 的序号
    /// （循环 +1 跳过）。
    fn parse_inline_image(&mut self, i: &mut usize) -> Result<(), PdeError> {
        let tokens = &self.lx.tokens;
        let n = tokens.len();
        let mut j = *i + 1;
        // 属性表：name-value 对（词法产出 Name + Operand；值可为
        // 名字/数字/数组——`/W 16 /H 16 /BPC 8 /CS /RGB /F [/Fl]`）
        let mut attrs: Vec<(Vec<u8>, Operand)> = Vec::new();
        while j < n {
            match &tokens[j] {
                Token::Operand(Operand::Name(key)) => {
                    let key = key.clone();
                    j += 1;
                    // 值形态：名字/数字/布尔/字符串直接取；`[` 数组按
                    // 标记流收集（`/F [/AHx /DCT]`）；`ID` 紧随 = 无值
                    // 名字（病态流宽收为 null 值）
                    let val = match tokens.get(j) {
                        Some(Token::Operand(o)) => {
                            j += 1;
                            o.clone()
                        }
                        Some(Token::Op(o)) if o == b"[" => {
                            // 数组值：收集到 ]
                            j += 1;
                            let mut items = Vec::new();
                            while j < n {
                                match &tokens[j] {
                                    Token::Operand(o) => {
                                        items.push(o.clone());
                                        j += 1;
                                    }
                                    Token::Op(o) if o == b"]" => {
                                        j += 1;
                                        break;
                                    }
                                    _ => break,
                                }
                            }
                            Operand::Array(items)
                        }
                        Some(Token::Op(o)) if o == b"ID" => Operand::Null,
                        _ => {
                            return Err(PdeError::Parse {
                                op: "BI".into(),
                                reason: "inline image 属性表缺值".into(),
                            });
                        }
                    };
                    attrs.push((key, val));
                }
                Token::Op(o) if o == b"ID" => break,
                _ => {
                    return Err(PdeError::Parse {
                        op: "BI".into(),
                        reason: "inline image 属性表后未跟 ID".into(),
                    });
                }
            }
        }
        if j >= n {
            return Err(PdeError::Parse {
                op: "ID".into(),
                reason: "inline image 缺 ID 或属性表未闭合".into(),
            });
        }
        // `j` 指在 ID 上。数据区间 = lx.inline_spans[self.inline_seen]
        // （词法器已定位）；EI 紧随其后。
        let span = self
            .lx
            .inline_spans
            .get(self.inline_seen)
            .ok_or_else(|| PdeError::Parse {
                op: "ID".into(),
                reason: "词法器未记录 inline image 数据域".into(),
            })?;
        self.inline_seen += 1;
        let data = self.bytes[span.0..span.1].to_vec();
        // EI token 序号 = ID 序号 + 1（词法器成对产出）
        let ei_idx = j + 1;
        let mut g = self.gcur().clone();
        if let Some(c) = self.clip_pending.take() {
            g.clip = Some(c);
        }
        let elem = PdeElement::Image(PdeImage {
            gstate: g,
            attrs,
            data,
        });
        if let Some(top) = self.containers.last_mut() {
            top.elements.push(elem);
        } else {
            self.elements.push(elem);
        }
        *i = ei_idx;
        Ok(())
    }
}

/// marked-content 嵌套帧（解析期栈）。
struct ContainerFrame {
    tag: Vec<u8>,
    properties: Option<Operand>,
    elements: Vec<PdeElement>,
}

/// 操作数压栈（哨兵收集模型：`[`/`<<` 压 Array/Dict 哨兵层，裸操作数
/// 进**栈顶层**收集；`]`/`>>` 弹层作为整体操作数交回父层；栈空时裸
/// 操作数直接落 `ops`）。
fn push_operand(stack: &mut Vec<Operand>, ops: &mut Vec<Operand>, o: Operand) {
    match stack.last_mut() {
        Some(Operand::Array(items)) => items.push(o),
        Some(Operand::Dict(pairs)) => {
            // 字典层：奇数位收键（须 Name——病态流宽收为 Null 键标记）
            pairs.push((Operand::Null, o));
        }
        _ => ops.push(o),
    }
}

// ===== 操作数取用助手（元数/类型校验——错误带算子名） =====

fn need(ops: &[Operand], count: usize, op: &str) -> Result<(), PdeError> {
    if ops.len() != count {
        return Err(PdeError::Parse {
            op: op.into(),
            reason: format!("需要 {count} 个操作数，实得 {}", ops.len()),
        });
    }
    Ok(())
}

fn take_f32(ops: &mut Vec<Operand>, op: &str) -> Result<f32, PdeError> {
    need(ops, 1, op)?;
    let v = ops[0].as_num().ok_or_else(|| PdeError::Parse {
        op: op.into(),
        reason: "操作数非数字".into(),
    })?;
    ops.clear();
    Ok(v)
}

fn take_i32(ops: &mut Vec<Operand>, op: &str) -> Result<i32, PdeError> {
    Ok(take_f32(ops, op)? as i32)
}

fn take_pair(ops: &mut Vec<Operand>, op: &str) -> Result<(f32, f32), PdeError> {
    need(ops, 2, op)?;
    let a = ops[0].as_num().ok_or_else(|| bad_num(op))?;
    let b = ops[1].as_num().ok_or_else(|| bad_num(op))?;
    ops.clear();
    Ok((a, b))
}

fn take_nums(ops: &mut Vec<Operand>, op: &str) -> Result<Vec<f32>, PdeError> {
    let out: Result<Vec<f32>, _> = ops
        .iter()
        .map(|o| o.as_num().ok_or_else(|| bad_num(op)))
        .collect();
    ops.clear();
    out
}

fn take_nums4(ops: &mut Vec<Operand>, op: &str) -> Result<[f32; 4], PdeError> {
    need(ops, 4, op)?;
    let mut v = [0f32; 4];
    for (k, o) in ops.iter().enumerate() {
        v[k] = o.as_num().ok_or_else(|| bad_num(op))?;
    }
    ops.clear();
    Ok(v)
}

fn take_nums6(ops: &mut Vec<Operand>, op: &str) -> Result<[f32; 6], PdeError> {
    need(ops, 6, op)?;
    let mut v = [0f32; 6];
    for (k, o) in ops.iter().enumerate() {
        v[k] = o.as_num().ok_or_else(|| bad_num(op))?;
    }
    ops.clear();
    Ok(v)
}

fn take_rgb(ops: &mut Vec<Operand>, op: &str) -> Result<Vec<f32>, PdeError> {
    need(ops, 3, op)?;
    let v = take_nums(ops, op)?;
    Ok(v)
}

fn take_cmyk(ops: &mut Vec<Operand>, op: &str) -> Result<Vec<f32>, PdeError> {
    need(ops, 4, op)?;
    let v = take_nums(ops, op)?;
    Ok(v)
}

fn take_name(ops: &mut Vec<Operand>, op: &str) -> Result<Vec<u8>, PdeError> {
    need(ops, 1, op)?;
    let v = ops[0]
        .as_name_bytes()
        .ok_or_else(|| PdeError::Parse {
            op: op.into(),
            reason: "操作数非名字".into(),
        })?
        .to_vec();
    ops.clear();
    Ok(v)
}

/// BDC/DP 形态：tag + 属性（名字或字典）。
fn take_tagged(ops: &mut Vec<Operand>, op: &str) -> Result<(Vec<u8>, Option<Operand>), PdeError> {
    if ops.len() < 1 || ops.len() > 2 {
        return Err(PdeError::Parse {
            op: op.into(),
            reason: format!("需要 1-2 个操作数，实得 {}", ops.len()),
        });
    }
    let tag = ops[0]
        .as_name_bytes()
        .ok_or_else(|| PdeError::Parse {
            op: op.into(),
            reason: "首操作数非名字（tag）".into(),
        })?
        .to_vec();
    let props = if ops.len() == 2 {
        Some(ops[1].clone())
    } else {
        None
    };
    ops.clear();
    Ok((tag, props))
}

fn take_font(ops: &mut Vec<Operand>, op: &str) -> Result<(Vec<u8>, f32), PdeError> {
    need(ops, 2, op)?;
    let name = ops[0]
        .as_name_bytes()
        .ok_or_else(|| PdeError::Parse {
            op: op.into(),
            reason: "Tf 首操作数非名字（字体资源名）".into(),
        })?
        .to_vec();
    let size = ops[1].as_num().ok_or_else(|| bad_num(op))?;
    ops.clear();
    Ok((name, size))
}

/// TJ 数组取用：支持单数组操作数或扁平多数组（病态流宽收拼接）。
fn take_array_flat(ops: &mut Vec<Operand>, op: &str) -> Result<Vec<Operand>, PdeError> {
    let mut out = Vec::new();
    for o in ops.iter() {
        match o {
            Operand::Array(items) => out.extend(items.iter().cloned()),
            other => out.push(other.clone()),
        }
    }
    ops.clear();
    if out.is_empty() {
        return Err(PdeError::Parse {
            op: op.into(),
            reason: "TJ 缺数组操作数".into(),
        });
    }
    Ok(out)
}

fn take_matrix(ops: &mut Vec<Operand>, op: &str) -> Result<Matrix, PdeError> {
    need(ops, 6, op)?;
    let mut m = [0f32; 6];
    for (k, o) in ops.iter().enumerate() {
        m[k] = o.as_num().ok_or_else(|| bad_num(op))?;
    }
    ops.clear();
    Ok(m)
}

fn bad_num(op: &str) -> PdeError {
    PdeError::Parse {
        op: op.into(),
        reason: "操作数非数字".into(),
    }
}
