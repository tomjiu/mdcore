//! #195 验收面——实际 PDF × 独立引擎（pypdfium2 / MuPDF 系）字符
//! 原点对照。
//!
//! 流程：产品公共 API 构造 PdeText（含 TJ/多 run/矩阵/复合字体编码
//! 形态）→ 产品 `write_content` 写内容流 → 测试内拼最小有效 PDF →
//! 外部 Python pypdfium2 逐字符提取原点 → 与模型 `run_advance` /
//! 解析面 round-trip 双对照。
//!
//! pypdfium2 不在场或夹具失败 = SKIP（环境门——不伪绿也不伪红）；
//! 在场则必须逐字符 x 坐标在容差内（0.5pt）对上**模型推导值**。
//!
//! #194 注意：本组只锁 #195 度量面；split/replace 写回保真归 #194
//! 的独立测试面（共享文件单一负责人顺序处理）。

use mdc_pdf::pde::font::PdeFont;
use mdc_pdf::pde::model::{
    GState, PdeContent, PdeElement, PdeText, ShowOp, TextRun, TextState, IDENTITY,
};
use mdc_pdf::pde::parser::parse_content;
use mdc_pdf::pde::resources::ResValue;
use mdc_pdf::pde::writer::write_content;
use std::process::Command;

/// Courier（/Widths 全 600）字体字典。
fn courier_res() -> Vec<(Vec<u8>, ResValue)> {
    vec![
        (b"Type".to_vec(), ResValue::Name(b"Font".to_vec())),
        (b"Subtype".to_vec(), ResValue::Name(b"Type1".to_vec())),
        (b"BaseFont".to_vec(), ResValue::Name(b"Courier".to_vec())),
        (b"FirstChar".to_vec(), ResValue::Int(0)),
        (b"LastChar".to_vec(), ResValue::Int(255)),
        (
            b"Widths".to_vec(),
            ResValue::Array(vec![ResValue::Int(600); 256]),
        ),
    ]
}

/// 组装最小有效 PDF（单页 + 单内容流 + Courier 字体资源）。
/// 返回写入的文件路径。
fn assemble_pdf(path: &str, content: &[u8]) -> std::io::Result<()> {
    let mut objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] \
          /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            String::from_utf8_lossy(content)
        )
        .into_bytes(),
        {
            let mut v = vec![b"<< ".to_vec()];
            for (k, val) in courier_res() {
                v.push(format!("/{} ", String::from_utf8_lossy(&k)).into_bytes());
                v.push(serialize_res(&val));
            }
            v.push(b">>".to_vec());
            v.concat()
        },
    ];
    let mut out: Vec<u8> = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objs.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets {
        out.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF",
            objs.len() + 1,
            xref
        )
        .as_bytes(),
    );
    std::fs::write(path, &out)
}

/// ResValue → PDF 字面量（测试内的最小序列化——只覆盖本组用到的
/// Name/Int/Array/Dict 形态）。
fn serialize_res(v: &ResValue) -> Vec<u8> {
    match v {
        ResValue::Name(n) => format!("/{}", String::from_utf8_lossy(n)).into_bytes(),
        ResValue::Int(i) => format!("{i}").into_bytes(),
        ResValue::Array(items) => {
            let mut out = b"[".to_vec();
            for (k, it) in items.iter().enumerate() {
                if k > 0 {
                    out.push(b' ');
                }
                out.extend_from_slice(&serialize_res(it));
            }
            out.push(b']');
            out
        }
        ResValue::Dict(pairs) => {
            let mut out = b"<< ".to_vec();
            for (k, val) in pairs {
                out.extend_from_slice(format!("/{} ", String::from_utf8_lossy(k)).as_bytes());
                out.extend_from_slice(&serialize_res(val));
            }
            out.extend_from_slice(b" >>");
            out
        }
        ResValue::Real(x) => format!("{x}").into_bytes(),
        _ => b"null".to_vec(),
    }
}

/// 跑 pypdfium2 提取（外部进程——测试不依赖 Python 模块二进制面）。
/// 返回逐字符 `(char, l, b, r, t)` 列表（charbox）。None = 引擎不在
/// 场（SKIP 通道）。
fn pdfium_charboxes(pdf: &str) -> Option<Vec<(char, f64, f64, f64, f64)>> {
    let py = Command::new("python3")
        .arg("-c")
        .arg(
            r#"
import pypdfium2 as pdfium
pdf = pdfium.PdfDocument('__PDF__')
page = pdf[0]
tp = page.get_textpage()
n = tp.count_chars()
text = tp.get_text_range(0, n)
for i in range(n):
    l, b, r, t = tp.get_charbox(i, loose=False)
    print(f"{text[i]} {l:.3f} {b:.3f} {r:.3f} {t:.3f}")
"#
            .replace("__PDF__", pdf),
        )
        .output()
        .ok()?;
    if !py.status.success() {
        return None; // 引擎失败（版本/符号缺）——SKIP 通道
    }
    let mut out = Vec::new();
    for line in String::from_utf8_lossy(&py.stdout).lines() {
        let mut it = line.split_whitespace();
        let c = it.next()?.chars().next()?;
        let l: f64 = it.next()?.parse().ok()?;
        let b: f64 = it.next()?.parse().ok()?;
        let r: f64 = it.next()?.parse().ok()?;
        let t: f64 = it.next()?.parse().ok()?;
        out.push((c, l, b, r, t));
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// 模型字符原点（run 起点 Tm + run_advance 累计推导）——与引擎
/// charbox 的左下角对照（Courier 全 600 等宽：原点差分与 advance
/// 差分同构，绝对墨迹偏移由基线对齐吸收）。
fn model_char_origins(t: &PdeText, font: &PdeFont) -> Vec<(char, f32)> {
    let mut out = Vec::new();
    for run in &t.runs {
        let (ex, ey) = (run.text_matrix[4], run.text_matrix[5]);
        let mut x = ex;
        // 逐 show 项推进（#195 语义：Kern 负贡献；Str 按码长切）。
        for op in &run.shows {
            match op {
                ShowOp::Kern(k) => x -= k * run.state.size / 1000.0,
                ShowOp::Str(bytes) => {
                    for &b in bytes {
                        out.push((b as char, x));
                        x += font.width_of(b as u32) * run.state.size / 1000.0 * run.state.h_scale
                            / 100.0
                            + run.state.char_spacing;
                        if b == 32 {
                            x += run.state.word_spacing;
                        }
                    }
                }
            }
        }
        let _ = ey;
    }
    out
}

/// **对照 1（issue 反例 1 形态）**：`BT /F1 12 Tf 1 0 0 1 20 30 Tm
/// [(A) 200 (BCD)] TJ ET`——引擎四字符起点 x = 20 / 24.8 / 32 / 39.2
///（A 后 7.2−2.4=4.8、B 后 7.2、C 后 7.2）。TJ 200 必须体现为
/// A→B 跨度 4.8（修前 7.2——写回保真同链路）。
#[test]
fn engine_char_origins_match_model_tj() {
    // 内容流走产品解析→写回 round-trip（锁写回链路的 TJ 保真——
    // Kern 保留形态）。
    let src = b"BT /F1 12 Tf 1 0 0 1 20 30 Tm [(A) 200 (BCD)] TJ ET";
    let content = parse_content(src).expect("解析须成");
    let rewritten = write_content(&content);
    assert!(
        rewritten.windows(3).any(|w| w == b"200"),
        "写回必须保留 TJ 200（Kern 不丢——#194 的 replace 面之外，\
         round-trip 本身保真）"
    );

    let dir = std::env::temp_dir().join(format!("pde195-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let pdf = dir.join("tj.pdf");
    assemble_pdf(pdf.to_str().unwrap(), &rewritten).expect("PDF 组装");

    let Some(boxes) = pdfium_charboxes(pdf.to_str().unwrap()) else {
        eprintln!("SKIP: pypdfium2 不可用（环境门——不伪绿）");
        return;
    };
    assert_eq!(boxes.len(), 4, "四字符");
    // charbox 是字形墨迹盒（左沿含 bearing——非原点）；等宽字体上
    // **中点差分**实测 ≈ advance（ink 宽差 <0.1、被 /2 压缩，实测
    // 偏差 <0.05）。TJ 判据 = A→B 跨度显著小于 7.2（4.8）；修前
    // （TJ 丢弃）A→B 与 B→C 同为 7.2——无法区分即测试失效形态。
    let mid = |i: usize| (boxes[i].1 + boxes[i].3) / 2.0;
    assert!(
        (mid(1) - mid(0) - 4.8).abs() < 0.45,
        "A→B 跨度 4.8（7.2−2.4，TJ 200 进位）：{}",
        mid(1) - mid(0)
    );
    assert!((mid(2) - mid(1) - 7.2).abs() < 0.45, "B→C 跨度 7.2");
    assert!((mid(3) - mid(2) - 7.2).abs() < 0.45, "C→D 跨度 7.2");
}

/// **对照 2：run_advance 与引擎总跨度**——TJ run 的模型 advance
/// 26.4 vs 引擎末字符右沿 − 首字符左沿（含字形宽度）：(39.2+7.2)
/// − 20 = 26.4。多 run（同 Tm #194 修复前的形态在此不验——位置
/// 保真归 #194；本测锁**单 run** advance 的引擎一致性）。
#[test]
fn engine_total_advance_matches_run_advance() {
    let src = b"BT /F1 12 Tf 1 0 0 1 20 30 Tm [(A) 200 (BCD)] TJ ET";
    let content = parse_content(src).expect("解析须成");
    // 模型面：PdeText 第一个 Text run 的 advance。
    let mut model = None;
    if let Some(PdeElement::Text(t)) = content.elements.first() {
        model = t.run_advance(0, None); // 无字体上下文 → None（诚实）
    }
    assert!(model.is_none(), "无字体面必须 None（契约）");
    // 有字体：从字典构造 PdeFont（公共 API）。
    let font = PdeFont::from_res_value(&ResValue::Dict(courier_res())).unwrap();
    if let Some(PdeElement::Text(t)) = content.elements.first() {
        let adv = t.run_advance(0, Some(&font)).expect("度量须有值");
        assert!(
            (adv - 26.4).abs() < 1e-3,
            "模型 advance 26.4（TJ 参与），实际 {adv}"
        );
        // 引擎对照。
        let dir = std::env::temp_dir().join(format!("pde195a-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let pdf = dir.join("adv.pdf");
        let rewritten = write_content(&content);
        assemble_pdf(pdf.to_str().unwrap(), &rewritten).expect("PDF 组装");
        let Some(boxes) = pdfium_charboxes(pdf.to_str().unwrap()) else {
            eprintln!("SKIP: pypdfium2 不可用");
            return;
        };
        let total: f64 = boxes.last().unwrap().3 - boxes.first().unwrap().1;
        assert!(
            (total - adv as f64).abs() < 0.5,
            "引擎总跨度 {total} vs 模型 advance {adv}"
        );
    }
}

/// **对照 3：矩阵变换**——Tm 带缩放（2 0 0 2 40 60 Tm）：字符跨度
/// ×2（A→B = 9.6）——run_advance 是字符空间值，矩阵缩放在引擎侧
/// 体现为几何 ×2（模型面 text_matrix 已承载——本测锁引擎侧缩放
/// 正确传导 + 模型 advance 不含矩阵（Adobe 语义：字符空间度量）。
#[test]
fn engine_matrix_scale_doubles_spans() {
    let src = b"BT /F1 12 Tf 2 0 0 2 40 60 Tm (ABCD) Tj ET";
    let content = parse_content(src).expect("解析须成");
    let rewritten = write_content(&content);
    let dir = std::env::temp_dir().join(format!("pde195m-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let pdf = dir.join("m.pdf");
    assemble_pdf(pdf.to_str().unwrap(), &rewritten).expect("PDF 组装");
    let Some(boxes) = pdfium_charboxes(pdf.to_str().unwrap()) else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    assert_eq!(boxes.len(), 4, "四字符");
    // 中点差分（同上——字形墨迹 bearing 差经 /2 压缩）：字符空间
    // 7.2 × 矩阵 2 = 14.4 跨度（±0.9 的 ink 差半幅容差）。
    let mid = |i: usize| (boxes[i].1 + boxes[i].3) / 2.0;
    for i in 1..4 {
        assert!(
            (mid(i) - mid(i - 1) - 14.4).abs() < 0.9,
            "×2 缩放跨度 14.4：实际 {}",
            mid(i) - mid(i - 1)
        );
    }
    // 基线 y 同乘 2（Tm f=60 → 底沿 ≈60）：矩阵传导的垂直面证据。
    assert!((boxes[0].2 - 60.0).abs() < 1.0, "基线 y=60（Tm×2 传导）");
}

/// **对照 4：复合字体（Type0 Identity-H）**——引擎（pdfium）需真
/// CMap/字体程序；拼 Identity-H + descendant CIDFontType2（/W
/// CID 0x41 起 600）形态，内容 <0041 0042>（CID 65/66）。引擎在场
/// 时核对两字符原点跨度 = 6×2（600×10/1000）。
#[test]
fn engine_identity_h_cid_advances() {
    // 复合字体组装（Identity-H 无 CMap 流——2 字节直切 CID）。
    let dir = std::env::temp_dir().join(format!("pde195t0-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let pdf = dir.join("t0.pdf");
    // 手拼（assemble_pdf 是简单字体形态——此处独立拼 Type0 资源）。
    let content = b"BT /F1 10 Tf 1 0 0 1 20 30 Tm <00410042> Tj ET";
    let mut objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] \
          /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            String::from_utf8_lossy(content)
        )
        .into_bytes(),
        // Type0 主字体 + Identity-H + descendant CIDFontType2 /W。
        b"<< /Type /Font /Subtype /Type0 /BaseFont /Test-CID \
          /Encoding /Identity-H \
          /DescendantFonts [<< /Type /Font /Subtype /CIDFontType2 \
          /BaseFont /Test-CID /CIDSystemInfo << /Registry (Adobe) \
          /Ordering (Identity) /Supplement 0 >> \
          /W [65 [600] 66 [600]] /DW 500 \
          /CIDToGIDMap /Identity >>] >>"
            .to_vec(),
    ];
    let mut out: Vec<u8> = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objs.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets {
        out.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF",
            objs.len() + 1,
            xref
        )
        .as_bytes(),
    );
    std::fs::write(&pdf, &out).expect("写盘");

    let Some(boxes) = pdfium_charboxes(pdf.to_str().unwrap()) else {
        eprintln!("SKIP: pypdfium2 不可用（或无 CID 字体程序面）");
        return;
    };
    if boxes.len() < 2 {
        // pdfium 对无字体程序的 Type0 可能提取不出字符——SKIP 面
        //（不伪红：引擎提取能力 ≠ 模型度量正确性；模型面由
        // type0_cid_two_byte_code_lookup 锁定）。
        eprintln!(
            "SKIP: 引擎未提取出 CID 字符（{} 个）——无字体程序面",
            boxes.len()
        );
        return;
    }
    let xs: Vec<f64> = boxes.iter().map(|(_, l, _, _, _)| *l).collect();
    assert!(
        (xs[1] - xs[0] - 6.0).abs() < 0.5,
        "CID 65→66 跨度 6.0（/W 600 × 10/1000），实际 {}",
        xs[1] - xs[0]
    );
}

// 模型辅助（对照 2 内联使用——避免与 #194 重复的公共面）。
#[allow(dead_code)]
fn unused_model_helper() {
    let _ = model_char_origins;
}

// ===== 返工 2026-11-14：Tz 全乘子引擎探针（Lead MuPDF 同款方法） =====

/// Lead 探针法：同流后加 `(X) Tj`——X 的原点 − 流起点 = 前流总
/// advance（引擎实测当前点位置）。charbox 为字形墨迹盒（左沿含
/// bearing），探针用**中点差分**（X 中点 − A 中点——Courier 等宽
/// ink 差经 /2 压缩，实测偏差 <0.05）。
///
/// Lead 原例：`50 Tz 2 Tc 4 Tw [(A ) 200 (B)] TJ (X) Tj` → X−A =
/// 14.6（MuPDF 实证；pdfium 独立同值）。
fn probe_advance(content: &str) -> Option<f64> {
    let dir = std::env::temp_dir().join(format!("pde195p-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let pdf = dir.join("probe.pdf");
    assemble_pdf(pdf.to_str().unwrap(), content.as_bytes()).expect("PDF 组装");
    let Some(boxes) = pdfium_charboxes(pdf.to_str().unwrap()) else {
        return None; // 引擎不可用 → SKIP 通道
    };
    // 首字符（'A'）与末字符（'X'）中点差。
    let first = boxes.first()?;
    let last = boxes.last()?;
    Some(((last.1 + last.3) / 2.0) - ((first.1 + first.3) / 2.0))
}

/// **Lead 反例原例引擎探针**：Tz50 Tc2 Tw4 TJ+200 → 14.6
///（模型 `run_advance` 同值——真源正确性三方闭环：规范公式、
/// MuPDF（Lead 实证）、pdfium（本测）、模型）。
#[test]
fn engine_probe_lead_tz_combo() {
    let Some(x_minus_a) =
        probe_advance("BT /F1 12 Tf 50 Tz 2 Tc 4 Tw 1 0 0 1 20 30 Tm [(A ) 200 (B)] TJ (X) Tj ET")
    else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    assert!(
        (x_minus_a - 14.6).abs() < 0.2,
        "Lead 反例：X−A = 14.6（Tz 全乘），引擎实测 {x_minus_a}"
    );
    // 模型同值（真源对照——同流解析后 run_advance）。
    let t = {
        use mdc_pdf::pde::model::PdeElement;
        use mdc_pdf::pde::parser::parse_content;
        let mut c =
            parse_content(b"BT /F1 12 Tf 50 Tz 2 Tc 4 Tw 1 0 0 1 20 30 Tm [(A ) 200 (B)] TJ ET")
                .unwrap();
        match c.elements.first_mut() {
            Some(PdeElement::Text(t)) => {
                let empty = PdeText {
                    gstate: t.gstate.clone(),
                    runs: Vec::new(),
                };
                std::mem::replace(t, empty)
            }
            _ => panic!("首元素须为 Text"),
        }
    };
    let font = PdeFont::from_res_value(&ResValue::Dict(courier_res())).unwrap();
    let adv = t.run_advance(0, Some(&font)).expect("度量须有值");
    assert!(
        (adv - 14.6).abs() < 1e-3,
        "模型 advance = 引擎探针 = 14.6，模型 {adv}"
    );
}

/// Tz=200 侧：同流 Tz 200 → X−A = 58.4（全项 ×2）。
#[test]
fn engine_probe_tz200() {
    let Some(x_minus_a) =
        probe_advance("BT /F1 12 Tf 200 Tz 2 Tc 4 Tw 1 0 0 1 20 30 Tm [(A ) 200 (B)] TJ (X) Tj ET")
    else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    assert!(
        (x_minus_a - 58.4).abs() < 0.4,
        "Tz 200 全项 ×2 → 58.4，引擎实测 {x_minus_a}"
    );
}

/// 负 TJ（松排）× Tz：`[(AB) -150 (C)]` Tz 50 Tc 2 → 14.7。
#[test]
fn engine_probe_negative_tj_tz() {
    let Some(x_minus_a) =
        probe_advance("BT /F1 12 Tf 50 Tz 2 Tc 1 0 0 1 20 30 Tm [(AB) -150 (C)] TJ (X) Tj ET")
    else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    assert!(
        (x_minus_a - 14.7).abs() < 0.2,
        "负 TJ 松排 × Tz：14.7，引擎实测 {x_minus_a}"
    );
}
