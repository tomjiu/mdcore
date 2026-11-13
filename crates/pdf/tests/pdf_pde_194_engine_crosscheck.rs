//! #194 验收面——编辑保真 × 独立引擎（pypdfium2）字符原点核对。
//!
//! 流程：解析 → 编辑（split/replace/delete）→ 产品写回 → 拼最小
//! PDF → pypdfium2 逐字符 charbox → 与**未编辑基线**逐字符对照。
//!
//! 核心判据（issue 验收原文）：「仅拆分不改文本，写回后独立引擎的
//! 各字符原点/页面图像**保持**；同字替换在 TJ 流上语义不变」——即
//! **编辑前后同位字符的原点差分一致**（拆分不挪字、替换不动其他）。
//!
//! charbox 语义（#195 已实证）：字形墨迹盒（左沿含 bearing）；等宽
//! Courier 上中点差分 ≈ advance（ink 宽差被 /2 压缩）。引擎不可用
//! = SKIP 通道。

use mdc_pdf::pde::font::PdeFont;
use mdc_pdf::pde::model::PdeElement;
use mdc_pdf::pde::parser::parse_content;
use mdc_pdf::pde::resources::ResValue;
use mdc_pdf::pde::writer::write_content;
use std::process::Command;

fn courier_res() -> Vec<(Vec<u8>, ResValue)> {
    vec![
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

fn courier_font() -> PdeFont {
    PdeFont::from_res_value(&ResValue::Dict(courier_res())).unwrap()
}

/// 拼最小 PDF（复用 #195 的组装口径）。
fn assemble_pdf(path: &str, content: &[u8]) -> std::io::Result<()> {
    let widths: Vec<String> = (0..256).map(|_| "600".to_string()).collect();
    let font_dict = format!(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier /FirstChar 0 /LastChar 255 /Widths [{}] >>",
        widths.join(" ")
    );
    let objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            String::from_utf8_lossy(content)
        )
        .into_bytes(),
        font_dict.into_bytes(),
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

/// pypdfium2 逐字符中点 x（引擎不在场 = None → SKIP）。
fn pdfium_mid_xs(pdf: &str) -> Option<Vec<f64>> {
    let py = Command::new("python3")
        .arg("-c")
        .arg(
            r#"
import pypdfium2 as pdfium
pdf = pdfium.PdfDocument('__PDF__')
tp = pdf[0].get_textpage()
n = tp.count_chars()
for i in range(n):
    l, b, r, t = tp.get_charbox(i, loose=False)
    print(f"{(l+r)/2:.4f} {b:.4f}")
"#
            .replace("__PDF__", pdf),
        )
        .output()
        .ok()?;
    if !py.status.success() {
        return None;
    }
    let xs: Vec<f64> = String::from_utf8_lossy(&py.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().next()?.parse().ok())
        .collect();
    if xs.is_empty() {
        None
    } else {
        Some(xs)
    }
}

/// 首个 Text 元素（取所有权——供编辑）。
fn take_text(src: &[u8]) -> mdc_pdf::pde::model::PdeText {
    let mut c = parse_content(src).expect("解析须成");
    match c.elements.first_mut() {
        Some(PdeElement::Text(t)) => {
            let empty = mdc_pdf::pde::model::PdeText {
                gstate: t.gstate.clone(),
                runs: Vec::new(),
            };
            std::mem::replace(t, empty)
        }
        _ => panic!("首元素须为 Text"),
    }
}

/// 写回并落 PDF，返回中点 x 列表。
fn render_mid_xs(t: mdc_pdf::pde::model::PdeText, tag: &str) -> Option<Vec<f64>> {
    let c = mdc_pdf::pde::model::PdeContent {
        elements: vec![PdeElement::Text(t)],
    };
    let out = write_content(&c);
    let dir = std::env::temp_dir().join(format!("pde194-{tag}-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let pdf = dir.join(format!("{tag}.pdf"));
    assemble_pdf(pdf.to_str().unwrap(), &out).expect("PDF 组装");
    pdfium_mid_xs(pdf.to_str().unwrap())
}

/// **验收 1：仅拆分不改文本——各字符原点保持**。
/// 基线 `BT /F1 12 Tf 1 0 0 1 20 30 Tm (ABCD) Tj ET` 四字符中点
/// x=[23.6,28.4,35.57,42.79]；split_run_at(1) 后写回——**逐字符**
/// 中点差 < 0.45（拆分零位移——修前 CD 回 AB 原点：差 7.2/14.4）。
#[test]
fn split_keeps_every_char_origin() {
    let src = b"BT /F1 12 Tf 1 0 0 1 20 30 Tm (ABCD) Tj ET";
    let Some(base) = render_mid_xs(take_text(src), "base1") else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    assert_eq!(base.len(), 4, "基线四字符");

    let mut t = take_text(src);
    let font = courier_font();
    assert!(t.split_run_at(1, Some(&font)), "拆分成功");
    let Some(after) = render_mid_xs(t, "split1") else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    assert_eq!(after.len(), 4, "拆后仍四字符（无损）");
    for (i, (b, a)) in base.iter().zip(after.iter()).enumerate() {
        assert!(
            (a - b).abs() < 0.45,
            "字符 {i} 原点保持：基线 {b:.3} vs 拆后 {a:.3}（修前 C/D 回 AB 原点）"
        );
    }
}

/// **验收 2：TJ 流上同字替换——语义不变**。
/// 基线 `[(A) 200 (BCD)] TJ`：中点 x=[23.6,28.4,35.57,42.79]；
/// replace_chars(0, b"A")（同字）→ 逐字符中点差 < 0.45（200 字距
/// 保留——修前重装单 Str 后 B 原点右移 2.4、C/D 各 2.4）。
#[test]
fn same_char_replace_keeps_origins_on_tj() {
    let src = b"BT /F1 12 Tf 1 0 0 1 20 30 Tm [(A) 200 (BCD)] TJ ET";
    let Some(base) = render_mid_xs(take_text(src), "base2") else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    assert_eq!(base.len(), 4);

    let mut t = take_text(src);
    t.replace_chars(0, b"A"); // 同字替换
    let Some(after) = render_mid_xs(t, "rep2") else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    assert_eq!(after.len(), 4);
    for (i, (b, a)) in base.iter().zip(after.iter()).enumerate() {
        assert!(
            (a - b).abs() < 0.45,
            "字符 {i} 原点保持（TJ 200 不丢）：基线 {b:.3} vs 替换后 {a:.3}"
        );
    }
}

/// **验收 3：真改字符（A→X）不挪其余字符**。
/// replace_chars(0, b"X") 后 B/C/D 原点不变（200 字距仍在 B 前）。
#[test]
fn changed_char_replace_keeps_rest_origins() {
    let src = b"BT /F1 12 Tf 1 0 0 1 20 30 Tm [(A) 200 (BCD)] TJ ET";
    let Some(base) = render_mid_xs(take_text(src), "base3") else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    let mut t = take_text(src);
    t.replace_chars(0, b"X");
    let Some(after) = render_mid_xs(t, "rep3") else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    // A→X 同宽（Courier 600）——四字符全保持。
    for (i, (b, a)) in base.iter().zip(after.iter()).enumerate() {
        assert!(
            (a - b).abs() < 0.45,
            "字符 {i} 原点保持（换 X 不挪位）：基线 {b:.3} vs {a:.3}"
        );
    }
}

/// **验收 4：多 run 拆分（跨 TJ Kern）**——`[(A) 200 (BCD)] TJ`
/// split_run_at(1)（拆点在 Kern 之前的 A|BCD）：A 后的 200 属首段尾
/// 还是次段头的归属语义——拆后四字符原点仍逐位保持（200 归首段尾
/// 时次段 Tm 进位含 200；归次段头时次段 shows 带 200——两种合法
/// 归属都必须保位置）。
#[test]
fn split_across_kern_keeps_origins() {
    let src = b"BT /F1 12 Tf 1 0 0 1 20 30 Tm [(A) 200 (BCD)] TJ ET";
    let Some(base) = render_mid_xs(take_text(src), "base4") else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    let mut t = take_text(src);
    let font = courier_font();
    assert!(t.split_run_at(1, Some(&font)), "拆分成功（跨 Kern 点）");
    let Some(after) = render_mid_xs(t, "split4") else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    assert_eq!(after.len(), 4, "无损");
    for (i, (b, a)) in base.iter().zip(after.iter()).enumerate() {
        assert!(
            (a - b).abs() < 0.45,
            "字符 {i} 原点保持（跨 Kern 拆分）：基线 {b:.3} vs {a:.3}"
        );
    }
}

/// **验收 5：删除语义——删 B 后 C/D 原点左移一个 advance 位**
/// （`[(A) 200 (BCD)] TJ` 删 B：C 从 32.0 移到 24.8——即 B 位的
/// 200 后；D 同步左移 7.2；A 不动）。
#[test]
fn delete_char_shifts_following_correctly() {
    let src = b"BT /F1 12 Tf 1 0 0 1 20 30 Tm [(A) 200 (BCD)] TJ ET";
    let Some(base) = render_mid_xs(take_text(src), "base5") else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    let mut t = take_text(src);
    t.remove_chars(1, 1); // 删 B
    let Some(after) = render_mid_xs(t, "del5") else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    assert_eq!(after.len(), 3, "删后三字符（ACD）");
    // A 不动。
    assert!(
        (after[0] - base[0]).abs() < 0.45,
        "A 原点不动：{:.3} vs {:.3}",
        base[0],
        after[0]
    );
    // C 移到 B 的位（base[1]——200 后原点位）。
    assert!(
        (after[1] - base[1]).abs() < 0.45,
        "C 移入 B 位（200 字距保留在前）：{:.3} vs {:.3}",
        base[1],
        after[1]
    );
    // D 左移一个 advance（base[3] - 7.2）。
    assert!(
        (after[2] - (base[3] - 7.2)).abs() < 0.45,
        "D 左移一档：{:.3} vs {:.3}",
        base[3] - 7.2,
        after[2]
    );
}

/// **验收 6：矩阵变换下的拆分保真**——Tm 带 2× 缩放：
/// `2 0 0 2 40 60 Tm (ABCD) Tj` split(1) 后四字符原点保持
///（进位 advance 经 Tm 线性部分 ×2——模型换算核对）。
#[test]
fn split_with_matrix_scale_keeps_origins() {
    let src = b"BT /F1 12 Tf 2 0 0 2 40 60 Tm (ABCD) Tj ET";
    let Some(base) = render_mid_xs(take_text(src), "base6") else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    let mut t = take_text(src);
    let font = courier_font();
    assert!(t.split_run_at(1, Some(&font)));
    // 模型面：次段 Tm.e = 40 + 2×14.4 = 68.8（进位经线性部分 ×2）。
    assert!(
        (t.runs[1].text_matrix[4] - 68.8).abs() < 1e-3,
        "次段 Tm.e = 40 + 2×14.4 = 68.8，实际 {}",
        t.runs[1].text_matrix[4]
    );
    let Some(after) = render_mid_xs(t, "split6") else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    for (i, (b, a)) in base.iter().zip(after.iter()).enumerate() {
        assert!(
            (a - b).abs() < 0.9,
            "字符 {i} 原点保持（×2 缩放容差 0.9）：{b:.3} vs {a:.3}"
        );
    }
}
