//! pde::writer —— PDE 元素树 → 内容流字节（批①第四件）。
//!
//! Adobe 参照：`PDEContentToCosObj` 的内容流面（"main method for
//! converting a PDEContent into PDF contents and resources"）——批①
//! 只写内容流（资源字典合并归批②）。
//!
//! 写回策略（**重生成**，Adobe 同义——非保形序列化）：
//! - q/Q **展平**：每个元素前发到达该元素状态所需的图形态差分
//!   （CTM/颜色/线宽/ExtGState）；元素间快照比较，只发变化项。
//! - 文本块：BT..ET 一块一 PdeText；run 间发 Tm/Tf/Tc/… 差分。
//! - 路径：m/l/c/v/y/h/re 重生成 + paint 算子；clip（W/W*）在路径
//!   元素后补发（`W n` 纯裁剪形态——元素自身 clip 已吸收进 GState，
//!   写回时以下一元素的差分发出）。
//! - **未识别算子原样保留**（Adobe PDEUnknownGetOpName 面）：操作数
//!   按词法形态重序列化。
//! - 数字序列化：整数值写整数形态（`72` 非 `72.0`）——与常见生产流
//!   一致；浮点按最短往返表示（Rust `{}` 的 Grisu 格式，f32 往返无损）。
//!
//! 序列化保真面（round-trip 语义等价的边界）：
//! - 字符串 `()` 与 `<>` 形态按 `hex` 标志保形；
//! - 名字 `#xx` 转义仅在必要时发（写入时统一转义非安全字符）；
//! - inline image 数据逐字节原样（BI 属性表按原序）。

use super::model::*;
use std::io::Write;

/// 写回主入口：display list → 内容流字节。
pub fn write_content(content: &PdeContent) -> Vec<u8> {
    let mut w = Writer {
        out: Vec::new(),
        last_ctm: None,
        last_fill: None,
        last_stroke: None,
        last_width: None,
        last_extgstate: None,
    };
    w.write_elems(&content.elements);
    w.out
}

struct Writer {
    out: Vec<u8>,
    // 图形态差分基线（None = 未发过任何状态——全量发）
    last_ctm: Option<Matrix>,
    last_fill: Option<ColorSpec>,
    last_stroke: Option<ColorSpec>,
    last_width: Option<f32>,
    last_extgstate: Option<Vec<u8>>,
}

impl Writer {
    fn write_elems(&mut self, elems: &[PdeElement]) {
        for e in elems {
            self.write_elem(e);
        }
    }

    fn write_elem(&mut self, e: &PdeElement) {
        match e {
            PdeElement::Text(t) => self.write_text(t),
            PdeElement::Path(p) => self.write_path(p),
            PdeElement::Image(img) => self.write_image(img),
            PdeElement::XObject(x) => {
                self.write_gstate_diff(&x.gstate);
                self.write_name(&x.name);
                self.op(b"Do");
            }
            PdeElement::Container(c) => {
                self.write_container(c);
            }
            PdeElement::Place(p) => {
                self.write_name(&p.tag);
                match &p.properties {
                    None => self.op(b"MP"),
                    Some(props) => {
                        self.write_operand(props);
                        self.op(b"DP");
                    }
                }
            }
            PdeElement::Shading(s) => {
                self.write_gstate_diff(&s.gstate);
                self.write_name(&s.name);
                self.op(b"sh");
            }
            PdeElement::Unknown(u) => {
                for o in &u.operands {
                    self.write_operand(o);
                }
                self.op(&u.op);
            }
        }
    }

    // ===== 文本 =====

    fn write_text(&mut self, t: &PdeText) {
        // BT 进入态 = 元素 gstate（含 clip 差分先发）
        self.write_gstate_diff(&t.gstate);
        self.op(b"BT");
        let mut last_run_state: Option<TextState> = None;
        let mut last_tm: Option<Matrix> = None;
        for run in &t.runs {
            // run 差分：Tm（每 run 必发——合并 run 已在解析期归并，
            // 写回不重复合并）；文本状态逐字段差分
            self.write_text_state_diff(last_run_state.as_ref(), &run.state);
            self.write_matrix_operand(&run.text_matrix);
            self.op(b"Tm");
            if run.shows.len() == 1 {
                if let Some(ShowOp::Str(bytes)) = run.shows.first() {
                    self.write_str_operand(bytes, false);
                    self.op(b"Tj");
                    last_run_state = Some(run.state.clone());
                    last_tm = Some(run.text_matrix);
                    continue;
                }
            }
            // TJ 数组形态（含 Kern 或多 Str）
            self.out.push(b'[');
            for (k, s) in run.shows.iter().enumerate() {
                if k > 0 {
                    self.out.push(b' ');
                }
                match s {
                    ShowOp::Str(bytes) => self.write_str_operand(bytes, false),
                    ShowOp::Kern(v) => self.write_f32(*v),
                }
            }
            self.out.push(b']');
            self.op(b"TJ");
            last_run_state = Some(run.state.clone());
            last_tm = Some(run.text_matrix);
        }
        let _ = last_tm;
        self.op(b"ET");
    }

    /// 文本状态差分（Tf/Tc/Tw/Tz/TL/Ts/Tr——逐字段比较，只发变化项）。
    fn write_text_state_diff(&mut self, last: Option<&TextState>, cur: &TextState) {
        let d = |a: &Option<Vec<u8>>, b: &Option<Vec<u8>>| match (a, b) {
            (Some(x), Some(y)) => x != y,
            (None, None) => false,
            _ => true,
        };
        let is_first = last.is_none();
        let l = last.cloned().unwrap_or_default();
        if d(&l.font, &cur.font) || (is_first && cur.font.is_some()) {
            let name = cur.font.clone().unwrap_or_default();
            self.write_name(&name);
            self.out.push(b' ');
            self.write_f32(cur.size);
            self.op(b"Tf");
        }
        if l.char_spacing != cur.char_spacing || is_first && cur.char_spacing != 0.0 {
            self.write_f32(cur.char_spacing);
            self.op(b"Tc");
        }
        if l.word_spacing != cur.word_spacing || is_first && cur.word_spacing != 0.0 {
            self.write_f32(cur.word_spacing);
            self.op(b"Tw");
        }
        if l.h_scale != cur.h_scale || is_first && cur.h_scale != 100.0 {
            self.write_f32(cur.h_scale);
            self.op(b"Tz");
        }
        if l.leading != cur.leading || is_first && cur.leading != 0.0 {
            self.write_f32(cur.leading);
            self.op(b"TL");
        }
        if l.rise != cur.rise || is_first && cur.rise != 0.0 {
            self.write_f32(cur.rise);
            self.op(b"Ts");
        }
        if l.render_mode != cur.render_mode || is_first && cur.render_mode != 0 {
            self.write_i32(cur.render_mode);
            self.op(b"Tr");
        }
    }

    // ===== 路径 =====

    fn write_path(&mut self, p: &PdePath) {
        self.write_gstate_diff(&p.gstate);
        for s in &p.segs {
            match *s {
                PathSeg::MoveTo { x, y } => {
                    self.write_f32(x);
                    self.out.push(b' ');
                    self.write_f32(y);
                    self.op(b"m");
                }
                PathSeg::LineTo { x, y } => {
                    self.write_f32(x);
                    self.out.push(b' ');
                    self.write_f32(y);
                    self.op(b"l");
                }
                PathSeg::CurveTo {
                    c1x,
                    c1y,
                    c2x,
                    c2y,
                    x,
                    y,
                } => {
                    for v in [c1x, c1y, c2x, c2y, x, y] {
                        self.write_f32(v);
                        self.out.push(b' ');
                    }
                    self.op(b"c");
                }
                PathSeg::CurveToV { c2x, c2y, x, y } => {
                    for v in [c2x, c2y, x, y] {
                        self.write_f32(v);
                        self.out.push(b' ');
                    }
                    self.op(b"v");
                }
                PathSeg::CurveToY { c1x, c1y, x, y } => {
                    for v in [c1x, c1y, x, y] {
                        self.write_f32(v);
                        self.out.push(b' ');
                    }
                    self.op(b"y");
                }
                PathSeg::Close => self.op(b"h"),
                PathSeg::Rect { x, y, w, h } => {
                    for v in [x, y, w, h] {
                        self.write_f32(v);
                        self.out.push(b' ');
                    }
                    self.op(b"re");
                }
            }
        }
        // clip 吸收进元素 GState 的写回面：路径自身绘制前发 W/W*
        if p.clipped {
            self.op(
                if p.gstate.clip.as_ref().map(|c| c.even_odd).unwrap_or(false) {
                    b"W*"
                } else {
                    b"W"
                },
            );
        }
        self.op(p.paint.op_name().as_bytes());
    }

    // ===== inline image =====

    fn write_image(&mut self, img: &PdeImage) {
        self.write_gstate_diff(&img.gstate);
        self.op(b"BI");
        for (k, v) in &img.attrs {
            self.write_name(k);
            self.out.push(b' ');
            self.write_operand(v);
            self.out.push(b' ');
        }
        // ID 后恰一空白再接数据（词法器按单空白跳过——round-trip 契约）
        self.out.extend_from_slice(b" ID ");
        self.out.extend_from_slice(&img.data);
        self.out.extend_from_slice(b"\nEI\n");
    }

    // ===== marked content =====

    fn write_container(&mut self, c: &PdeContainer) {
        self.write_name(&c.tag);
        match &c.properties {
            None => self.op(b"BMC"),
            Some(props) => {
                self.write_operand(props);
                self.op(b"BDC");
            }
        }
        self.write_elems(&c.content.elements);
        self.op(b"EMC");
    }

    // ===== 图形态差分 =====

    fn write_gstate_diff(&mut self, g: &GState) {
        // CTM：非单位即发 cm（差分基线为单位阵）
        if g.ctm != IDENTITY && self.last_ctm != Some(g.ctm) {
            for v in g.ctm {
                self.write_f32(v);
                self.out.push(b' ');
            }
            self.op(b"cm");
            self.last_ctm = Some(g.ctm);
        } else if g.ctm == IDENTITY && self.last_ctm.is_some() {
            // 回到单位阵——发反向 cm？不可靠（浮点逆）。发 q/Q 对包夹：
            // 这里采用「状态重置」策略：Q 之后基线归位。但 Q 需要配对
            // q——展平策略下无栈。改发恒等 cm（1 0 0 1 0 0）。
            self.out.extend_from_slice(b"1 0 0 1 0 0 cm ");
            self.last_ctm = Some(IDENTITY);
        } else if g.ctm != IDENTITY {
            self.last_ctm = Some(g.ctm);
        }
        // 填充色
        if self.last_fill.as_ref() != Some(&g.fill) {
            self.write_color(&g.fill, false);
            self.last_fill = Some(g.fill.clone());
        }
        // 描边色
        if self.last_stroke.as_ref() != Some(&g.stroke) {
            self.write_color(&g.stroke, true);
            self.last_stroke = Some(g.stroke.clone());
        }
        // 线宽
        if self.last_width != Some(g.line_width) {
            self.write_f32(g.line_width);
            self.op(b"w");
            self.last_width = Some(g.line_width);
        }
        // ExtGState
        if self.last_extgstate != g.extgstate {
            if let Some(name) = &g.extgstate {
                self.write_name(name);
                self.op(b"gs");
            }
            self.last_extgstate = g.extgstate.clone();
        }
        // clip：元素吸收面——写回时发 `path W n` 形态（路径构造重发 +
        // 纯裁剪）。**批①简化**：clip 的 path 重发（段全量）+ W + n。
        if let Some(clip) = &g.clip {
            for s in &clip.segs {
                self.write_clip_seg(s);
            }
            self.op(if clip.even_odd { b"W*" } else { b"W" });
            self.op(b"n");
        }
    }

    /// clip 段重发（不带状态差分——clip 路径构造与绘制路径同构）。
    fn write_clip_seg(&mut self, s: &PathSeg) {
        match *s {
            PathSeg::MoveTo { x, y } => {
                self.write_f32(x);
                self.out.push(b' ');
                self.write_f32(y);
                self.op(b"m");
            }
            PathSeg::LineTo { x, y } => {
                self.write_f32(x);
                self.out.push(b' ');
                self.write_f32(y);
                self.op(b"l");
            }
            PathSeg::CurveTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => {
                for v in [c1x, c1y, c2x, c2y, x, y] {
                    self.write_f32(v);
                    self.out.push(b' ');
                }
                self.op(b"c");
            }
            PathSeg::CurveToV { c2x, c2y, x, y } => {
                for v in [c2x, c2y, x, y] {
                    self.write_f32(v);
                    self.out.push(b' ');
                }
                self.op(b"v");
            }
            PathSeg::CurveToY { c1x, c1y, x, y } => {
                for v in [c1x, c1y, x, y] {
                    self.write_f32(v);
                    self.out.push(b' ');
                }
                self.op(b"y");
            }
            PathSeg::Close => self.op(b"h"),
            PathSeg::Rect { x, y, w, h } => {
                for v in [x, y, w, h] {
                    self.write_f32(v);
                    self.out.push(b' ');
                }
                self.op(b"re");
            }
        }
    }

    /// 颜色写回（差分面：设备色直发 g/G/rg/RG/k/K；资源空间名 +
    /// scn/SCN——cs/CS 只在空间切换时发）。
    fn write_color(&mut self, c: &ColorSpec, stroke: bool) {
        let nums = match &c.values {
            ColorValues::Nums(v) => v.clone(),
            ColorValues::Name(n) => {
                // Pattern 名形态：cs/CS + 名字 scn/SCN
                self.write_name(&c.space);
                self.op(if stroke { b"CS" } else { b"cs" });
                self.write_name(n);
                self.op(if stroke { b"SCN" } else { b"scn" });
                return;
            }
        };
        match (c.space.as_slice(), nums.as_slice(), stroke) {
            (b"DeviceGray", [v], _) => {
                self.write_f32(*v);
                self.op(if stroke { b"G" } else { b"g" });
            }
            (b"DeviceRGB", [r, g, b], _) => {
                for v in [*r, *g, *b] {
                    self.write_f32(v);
                    self.out.push(b' ');
                }
                self.op(if stroke { b"RG" } else { b"rg" });
            }
            (b"DeviceCMYK", [c_, m, y, k], _) => {
                for v in [*c_, *m, *y, *k] {
                    self.write_f32(v);
                    self.out.push(b' ');
                }
                self.op(if stroke { b"K" } else { b"k" });
            }
            _ => {
                // 资源空间或分量数不符（1/3/4 之外——如 separation 2+）
                self.write_name(&c.space);
                self.op(if stroke { b"CS" } else { b"cs" });
                for v in &nums {
                    self.write_f32(*v);
                    self.out.push(b' ');
                }
                self.op(if stroke { b"SCN" } else { b"scn" });
            }
        }
    }

    // ===== 原子序列化 =====

    fn op(&mut self, name: &[u8]) {
        self.out.push(b' ');
        self.out.extend_from_slice(name);
        self.out.push(b'\n');
    }

    fn write_f32(&mut self, v: f32) {
        if v.fract() == 0.0 && v.abs() < 1e15 {
            write!(self.out, "{}", v as i64).ok();
        } else {
            write!(self.out, "{}", v).ok();
        }
    }

    fn write_i32(&mut self, v: i32) {
        write!(self.out, "{}", v).ok();
    }

    fn write_matrix_operand(&mut self, m: &Matrix) {
        for v in m {
            self.write_f32(*v);
            self.out.push(b' ');
        }
    }

    fn write_name(&mut self, name: &[u8]) {
        self.out.push(b'/');
        for &b in name {
            // 名字安全字符：可见 ASCII 非定界非 #；其余 #xx 转义
            if (0x21..=0x7e).contains(&b)
                && !matches!(
                    b,
                    b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%' | b'#'
                )
            {
                self.out.push(b);
            } else {
                write!(self.out, "#{:02X}", b).ok();
            }
        }
    }

    fn write_str_operand(&mut self, bytes: &[u8], hex: bool) {
        if hex {
            self.out.push(b'<');
            for b in bytes {
                write!(self.out, "{:02X}", b).ok();
            }
            self.out.push(b'>');
        } else {
            self.out.push(b'(');
            for &b in bytes {
                match b {
                    b'(' | b')' | b'\\' => {
                        self.out.push(b'\\');
                        self.out.push(b);
                    }
                    b'\n' => self.out.extend_from_slice(b"\\n"),
                    b'\r' => self.out.extend_from_slice(b"\\r"),
                    _ => self.out.push(b),
                }
            }
            self.out.push(b')');
        }
    }

    /// 通用操作数序列化（Unknown 保留面 / BDC 属性 / inline 属性值）。
    fn write_operand(&mut self, o: &Operand) {
        match o {
            Operand::Int(v) => {
                write!(self.out, "{}", v).ok();
            }
            Operand::Real(v) => self.write_f32(*v),
            Operand::Name(n) => self.write_name(n),
            Operand::Str { bytes, hex } => self.write_str_operand(bytes, *hex),
            Operand::Bool(b) => {
                write!(self.out, "{}", b).ok();
            }
            Operand::Null => self.out.extend_from_slice(b"null"),
            Operand::Array(items) => {
                self.out.push(b'[');
                for (k, item) in items.iter().enumerate() {
                    if k > 0 {
                        self.out.push(b' ');
                    }
                    self.write_operand(item);
                }
                self.out.push(b']');
            }
            Operand::Dict(pairs) => {
                // 宽收形态（Null 键）恢复：键值对里 Null 键丢弃，只写值
                // 间隔——重解析时按 << 收层配对（round-trip 契约：写回
                // 键值均带前后空白）
                self.out.extend_from_slice(b"<< ");
                for (k, v) in pairs {
                    if !matches!(k, Operand::Null) {
                        self.write_operand(k);
                        self.out.push(b' ');
                    }
                    self.write_operand(v);
                    self.out.push(b' ');
                }
                self.out.extend_from_slice(b">>");
            }
        }
    }
}
