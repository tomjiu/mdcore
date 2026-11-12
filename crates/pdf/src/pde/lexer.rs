//! pde::lexer —— PDF 内容流词法器（批①第二件）。
//!
//! Adobe 参照：`PDEContentCreateFromCosObj` 的底层——内容流字节 →
//! （操作数 … 操作符）标记序列。覆盖 PDF 参考手册 §7.8.2 内容流语法：
//! 数字（整数/实数）、名字（`/A`，##xx 十六进制转义）、字符串（`()` 嵌套
//! + 反斜杠转义 / `<>` 十六进制）、数组 `[]`、字典 `<<>>`、
//! true/false/null、inline image（BI/ID/EI——数据域原始字节直读，属性
//! 表走标记流）、注释（%）、空白/CR/LF/FF/NUL 分隔。
//!
//! 设计纪律：
//! - **不识别的算子不报错**——产出 [`Token::Op`]，语义分类归 parser；
//!   Adobe PDEUnknown 的设计同源（未识别算子原样保留写回）。
//! - 字符串**原始字节**语义：不做编码假设（PDEFont 面批②/③）。
//! - true/false/null 关键字在**词尾收口**统一转操作数（首字符路径不
//!   分叉——`n` 同时是 `n`/`null` 的首字母，分叉处理会漏判）。
//! - inline image 的 ID..EI 数据：PDF 规范说 EI 前应恰一空白，但生产
//!   文件常见偏差（Acrobat 自身容忍）——数据域由 parser 经
//!   [`LexOutput::inline_data`] 直读原文（词法器只产 `ID`/`EI` 算子，
//!   其间字节**不进标记流**，避免二进制数据被误词法）。

/// 词法标记（操作数 | 操作符）。
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// 数字/名字/字符串/布尔/null（结构化操作数形态）。
    Operand(super::model::Operand),
    /// 操作符（非空白非定界符的短字节串）。
    Op(Vec<u8>),
}

/// 词法结果：标记流 + inline image 数据域直读面。
pub struct LexOutput {
    pub tokens: Vec<Token>,
    /// `tokens[i]` 的起始字节偏移。
    pub offsets: Vec<usize>,
    /// inline image 数据域字节区间（`BI` 块出现序）：`(start, end)`
    /// 为 ID 之后数据在原文中的区间（不含 ID 后跳过的单一空白与
    /// EI 前的分隔空白）。
    pub inline_spans: Vec<(usize, usize)>,
}

/// EI 终结扫描（inline image 数据域）：找第一个「空白 + EI + 定界符
/// /EOF」候选。返回 `(分隔空白位置, EI 之后位置)`。Acrobat 同款启发式
/// ——数据域含 `\sEI\s` 字节序列时的歧义是 inline image 的固有面
/// （无 Length 字段可依），批②再按 W/H/BPC 校验收紧。
fn scan_inline_ei(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    // 消歧策略（数据含 ` EI ` 字面时）：**取最后一个候选**——inline
    // image 无 Length 字段可依（固有歧义面），pdfium/mupdf 同取末位
    // 策略：数据域后面只剩后续算子，真 EI 是「其后不再出现候选」的
    // 那个；先到先得会截断数据（测试 inline_image_binary_with_ei_
    // ambiguity 锁此语义）。
    let n = bytes.len();
    let mut last: Option<(usize, usize)> = None;
    let mut q = from;
    while q + 2 < n {
        if is_ws(bytes[q])
            && bytes[q + 1] == b'E'
            && bytes[q + 2] == b'I'
            && (q + 3 >= n
                || is_ws(bytes[q + 3])
                || matches!(
                    bytes[q + 3],
                    b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'/' | b'%'
                ))
        {
            last = Some((q, q + 3));
        }
        q += 1;
    }
    last
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 0x0c | 0x00)
}

/// 词法主入口。失败 = [`super::model::PdeError::Lex`]（字节偏移 +
/// 原因）。
pub fn lex(bytes: &[u8]) -> Result<LexOutput, PdeError> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut offsets = Vec::new();
    let mut inline_spans: Vec<(usize, usize)> = Vec::new();
    // inline image 状态：BI 后（等待 ID）词法走属性表正常面；ID 后
    // 数据域直读原文。
    let mut in_image = false;
    let mut i = 0usize;
    let n = bytes.len();

    while i < n {
        let b = bytes[i];
        match b {
            b' ' | b'\t' | b'\r' | b'\n' | 0x0c | 0x00 => i += 1,
            b'%' => {
                while i < n && bytes[i] != b'\n' && bytes[i] != b'\r' {
                    i += 1;
                }
            }
            b'B' if i + 1 < n && bytes[i + 1] == b'I' && !in_image => {
                // BI：inline image 开（属性表仍是普通标记流——名字/数字
                // /数组；值里 `ID` 算子终结属性表）。
                tokens.push(Token::Op(b"BI".to_vec()));
                offsets.push(i);
                i += 2;
                in_image = true;
            }
            b'0'..=b'9' | b'+' | b'-' | b'.' => {
                let (tok, next) = lex_number(bytes, i)?;
                tokens.push(tok);
                offsets.push(i);
                i = next;
            }
            b'/' => {
                let (tok, next) = lex_name(bytes, i)?;
                tokens.push(tok);
                offsets.push(i);
                i = next;
            }
            b'(' => {
                let (tok, next) = lex_string(bytes, i)?;
                tokens.push(tok);
                offsets.push(i);
                i = next;
            }
            b'<' if i + 1 < n && bytes[i + 1] == b'<' => {
                tokens.push(Token::Op(b"<<".to_vec()));
                offsets.push(i);
                i += 2;
            }
            b'<' => {
                let (tok, next) = lex_hex_string(bytes, i)?;
                tokens.push(tok);
                offsets.push(i);
                i = next;
            }
            b'>' if i + 1 < n && bytes[i + 1] == b'>' => {
                tokens.push(Token::Op(b">>".to_vec()));
                offsets.push(i);
                i += 2;
            }
            b'[' | b']' => {
                tokens.push(Token::Op(vec![b]));
                offsets.push(i);
                i += 1;
            }
            b'I' if i + 1 < n && bytes[i + 1] == b'D' && in_image => {
                // ID：inline image 数据起始。数据域直读原文（不进标记
                // 流——二进制数据会误词法）。
                // 数据起点 = ID 字面后跳一空白。
                let mut d = i + 2;
                if d < n && is_ws(bytes[d]) {
                    d += 1;
                }
                let (sep, after) = scan_inline_ei(bytes, d).ok_or_else(|| PdeError::Lex {
                    offset: i,
                    reason: "inline image 未找到 EI 终结".into(),
                })?;
                tokens.push(Token::Op(b"ID".to_vec()));
                offsets.push(i);
                tokens.push(Token::Op(b"EI".to_vec()));
                offsets.push(after - 2);
                inline_spans.push((d, sep));
                i = after;
                in_image = false;
            }
            _ => {
                // 关键字/算子词（含 true/false/null——词尾统一收口转
                // 操作数；其余为 Op）
                let (tok, next) = lex_keyword_or_op(bytes, i)?;
                tokens.push(tok);
                offsets.push(i);
                i = next;
            }
        }
    }
    // 词尾收口：true/false/null → Operand（O(n) 单遍——首字符分叉
    // 不可靠：`n` 既是 paint 算子又是 null 首字母）
    for t in tokens.iter_mut() {
        if let Token::Op(w) = t {
            match w.as_slice() {
                b"true" => *t = Token::Operand(Operand::Bool(true)),
                b"false" => *t = Token::Operand(Operand::Bool(false)),
                b"null" => *t = Token::Operand(Operand::Null),
                _ => {}
            }
        }
    }
    Ok(LexOutput {
        tokens,
        offsets,
        inline_spans,
    })
}

/// 数字词法。失败形态：孤立 + / - / .（无数字跟随）。
fn lex_number(bytes: &[u8], start: usize) -> Result<(Token, usize), PdeError> {
    let n = bytes.len();
    let mut i = start;
    let mut is_real = false;
    if matches!(bytes[i], b'+' | b'-') {
        i += 1;
    }
    let digit_start = i;
    while i < n && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i < n && bytes[i] == b'.' {
        is_real = true;
        i += 1;
        while i < n && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i == digit_start || (i == digit_start + 1 && bytes[digit_start] == b'.') {
        return Err(PdeError::Lex {
            offset: start,
            reason: "孤立符号或小数点（无数字跟随）".into(),
        });
    }
    let text = &bytes[start..i];
    if is_real {
        match std::str::from_utf8(text)
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
        {
            Some(v) => Ok((Token::Operand(Operand::Real(v)), i)),
            None => Err(PdeError::Lex {
                offset: start,
                reason: "实数解析失败".into(),
            }),
        }
    } else {
        match std::str::from_utf8(text)
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
        {
            Some(v) => Ok((Token::Operand(Operand::Int(v)), i)),
            None => Err(PdeError::Lex {
                offset: start,
                reason: "整数解析失败".into(),
            }),
        }
    }
}

/// 名字词法（`/A` / `/A#20B`——## 转义为十六进制字节）。
fn lex_name(bytes: &[u8], start: usize) -> Result<(Token, usize), PdeError> {
    let n = bytes.len();
    let mut i = start + 1;
    let mut out = Vec::new();
    while i < n && !is_delimiter(bytes[i]) {
        if bytes[i] == b'#'
            && i + 2 < n
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            out.push(hex_val(bytes[i + 1]) * 16 + hex_val(bytes[i + 2]));
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Ok((Token::Operand(Operand::Name(out)), i))
}

/// 字面字符串（嵌套括号 + 八进制/行接续/标准转义——PDF §7.3.4.2）。
fn lex_string(bytes: &[u8], start: usize) -> Result<(Token, usize), PdeError> {
    let n = bytes.len();
    let mut i = start + 1;
    let mut depth = 1usize;
    let mut out = Vec::new();
    while i < n {
        match bytes[i] {
            b'(' => {
                depth += 1;
                out.push(b'(');
                i += 1;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok((
                        Token::Operand(Operand::Str {
                            bytes: out,
                            hex: false,
                        }),
                        i + 1,
                    ));
                }
                out.push(b')');
                i += 1;
            }
            b'\\' => {
                i += 1;
                if i >= n {
                    return Err(PdeError::Lex {
                        offset: i,
                        reason: "字符串在转义序列处截断".into(),
                    });
                }
                match bytes[i] {
                    b'n' => {
                        out.push(b'\n');
                        i += 1;
                    }
                    b'r' => {
                        out.push(b'\r');
                        i += 1;
                    }
                    b't' => {
                        out.push(b'\t');
                        i += 1;
                    }
                    b'b' => {
                        out.push(0x08);
                        i += 1;
                    }
                    b'f' => {
                        out.push(0x0c);
                        i += 1;
                    }
                    b'(' => {
                        out.push(b'(');
                        i += 1;
                    }
                    b')' => {
                        out.push(b')');
                        i += 1;
                    }
                    b'\\' => {
                        out.push(b'\\');
                        i += 1;
                    }
                    b'\n' => i += 1,
                    b'\r' => {
                        i += 1;
                        if i < n && bytes[i] == b'\n' {
                            i += 1;
                        }
                    }
                    c if c.is_ascii_digit() => {
                        let mut v = 0u32;
                        let mut k = 0;
                        while i < n && k < 3 && bytes[i].is_ascii_digit() {
                            v = v * 8 + (bytes[i] - b'0') as u32;
                            i += 1;
                            k += 1;
                        }
                        out.push((v & 0xff) as u8);
                    }
                    c => {
                        out.push(c);
                        i += 1;
                    }
                }
            }
            _ => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    Err(PdeError::Lex {
        offset: start,
        reason: "字符串未闭合（EOF 前无右括号）".into(),
    })
}

/// 十六进制字符串（`<48656C6C6F>`——奇数位补零）。
fn lex_hex_string(bytes: &[u8], start: usize) -> Result<(Token, usize), PdeError> {
    let n = bytes.len();
    let mut i = start + 1;
    let mut hex_digits: Vec<u8> = Vec::new();
    while i < n && bytes[i] != b'>' {
        if bytes[i].is_ascii_hexdigit() {
            hex_digits.push(bytes[i]);
        } else if is_ws(bytes[i]) {
            // 空白容忍（PDF 允许）
        } else {
            return Err(PdeError::Lex {
                offset: i,
                reason: "十六进制字符串内非法字符".into(),
            });
        }
        i += 1;
    }
    if i >= n {
        return Err(PdeError::Lex {
            offset: start,
            reason: "十六进制字符串未闭合".into(),
        });
    }
    if hex_digits.len() % 2 == 1 {
        hex_digits.push(b'0');
    }
    let mut out = Vec::with_capacity(hex_digits.len() / 2);
    for pair in hex_digits.chunks(2) {
        out.push(hex_val(pair[0]) * 16 + hex_val(pair[1]));
    }
    Ok((
        Token::Operand(Operand::Str {
            bytes: out,
            hex: true,
        }),
        i + 1,
    ))
}

/// 关键字/算子词法（连续非定界符字节；典型 1-3 字母——BT/ET/Tj/TJ…）。
fn lex_keyword_or_op(bytes: &[u8], start: usize) -> Result<(Token, usize), PdeError> {
    let n = bytes.len();
    let mut i = start;
    let mut word = Vec::new();
    while i < n && !is_delimiter(bytes[i]) {
        word.push(bytes[i]);
        i += 1;
    }
    Ok((Token::Op(word), i))
}

/// PDF 定界符集合（() <> [] { } / % 与空白）。
fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    ) || is_ws(b)
}

fn hex_val(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}

use super::model::{Operand, PdeError};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_operators_and_operands() {
        let out = lex(b"BT /F1 12 Tf 72 720 Td (Hi) Tj ET").unwrap();
        let ops: Vec<&[u8]> = out
            .tokens
            .iter()
            .filter_map(|t| match t {
                Token::Op(o) => Some(o.as_slice()),
                _ => None,
            })
            .collect();
        assert_eq!(
            ops,
            vec![&b"BT"[..], &b"Tf"[..], &b"Td"[..], &b"Tj"[..], &b"ET"[..]]
        );
        assert!(matches!(
            out.tokens[1],
            Token::Operand(Operand::Name(ref n)) if n == b"F1"
        ));
    }

    #[test]
    fn lexes_numbers_all_shapes() {
        let out = lex(b"0.5 -3 42. +7.25 -0.75").unwrap();
        let nums: Vec<f32> = out
            .tokens
            .iter()
            .filter_map(|t| match t {
                Token::Operand(o) => o.as_num(),
                _ => None,
            })
            .collect();
        assert_eq!(nums, vec![0.5, -3.0, 42.0, 7.25, -0.75]);
    }

    #[test]
    fn lexes_string_escapes_and_nesting() {
        let out = lex(b"(a(b)c\\065d) Tj").unwrap();
        match &out.tokens[0] {
            Token::Operand(Operand::Str { bytes, hex }) => {
                assert_eq!(bytes, b"a(b)c5d");
                assert!(!*hex);
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn lexes_hex_string_odd_digits_padded() {
        // 偶数位原样：<4A> → 0x4A
        let out = lex(b"<4A>").unwrap();
        match &out.tokens[0] {
            Token::Operand(Operand::Str { bytes, hex }) => {
                assert_eq!(bytes, &[0x4a]);
                assert!(*hex);
            }
            _ => panic!("expected hex string"),
        }
        // 奇数位补零：<4> → 0x40（PDF §7.3.4.3）
        let out2 = lex(b"<4>").unwrap();
        match &out2.tokens[0] {
            Token::Operand(Operand::Str { bytes, .. }) => assert_eq!(bytes, &[0x40]),
            _ => panic!("expected hex string"),
        }
    }

    #[test]
    fn lexes_name_hash_escapes() {
        let out = lex(b"/A#20B cs").unwrap();
        match &out.tokens[0] {
            Token::Operand(Operand::Name(n)) => assert_eq!(n, b"A B"),
            _ => panic!("expected name"),
        }
    }

    #[test]
    fn comments_skipped_and_keywords() {
        let out = lex(b"1 2 TL % spacing\ntrue false null (x) Tj").unwrap();
        // 流：[Int, Int, Op(TL), Bool, Bool, Null, Str, Op(Tj)]
        assert!(matches!(out.tokens[3], Token::Operand(Operand::Bool(true))));
        assert!(matches!(
            out.tokens[4],
            Token::Operand(Operand::Bool(false))
        ));
        assert!(matches!(out.tokens[5], Token::Operand(Operand::Null)));
        // (x) 在 true/false/null 之后：索引 6
        assert!(matches!(out.tokens[6], Token::Operand(Operand::Str { .. })));
    }

    #[test]
    fn array_and_dict_tokens() {
        let out = lex(b"[(A) -120 (B)] TJ << /Type /XObject >>").unwrap();
        let ops: Vec<&[u8]> = out
            .tokens
            .iter()
            .filter_map(|t| match t {
                Token::Op(o) => Some(o.as_slice()),
                _ => None,
            })
            .collect();
        assert_eq!(
            ops,
            vec![&b"["[..], &b"]"[..], &b"TJ"[..], &b"<<"[..], &b">>"[..]]
        );
    }

    #[test]
    fn unclosed_string_errors() {
        assert!(lex(b"(unclosed").is_err());
        assert!(lex(b"<4A").is_err());
    }
}
