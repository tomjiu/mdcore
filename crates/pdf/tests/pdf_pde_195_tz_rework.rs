//! #195 返工反例（Lead 复验 2026-11-14）——Tz 组合语义。
//!
//! Lead 实证（MuPDF 同流加 `(X) Tj` 测当前点）：Courier 600、
//! `BT /F1 12 Tf 50 Tz 2 Tc 4 Tw [(A ) 200 (B)] TJ ET`
//! 正确 advance = (3×7.2 + 3×2 + 4 − 2.4)×0.5 = **14.6**；
//! 候选（Tz 只乘字形宽、不乘 Tc/Tw/TJ）= 18.4。
//!
//! 规范口径（PDF 3200-1:2008 Table 51）：**Tz 是整体乘子**——
//! `tx = ((w0/1000 − tj/1000) × Tf + Tc + Tw) × Tz/100`，即字形宽、
//! Tc、Tw、TJ 位移全部乘 Tz/100。修前「TJ 不受 Tz 影响」注释
//! 与实现均错误。

use mdc_pdf::pde::font::PdeFont;
use mdc_pdf::pde::model::{GState, PdeText, ShowOp, TextRun, TextState, IDENTITY};
use mdc_pdf::pde::resources::ResValue;

fn courier_600() -> PdeFont {
    PdeFont::from_res_value(&ResValue::Dict(vec![
        (b"Subtype".to_vec(), ResValue::Name(b"Type1".to_vec())),
        (b"BaseFont".to_vec(), ResValue::Name(b"Courier".to_vec())),
        (b"FirstChar".to_vec(), ResValue::Int(0)),
        (b"LastChar".to_vec(), ResValue::Int(255)),
        (
            b"Widths".to_vec(),
            ResValue::Array(vec![ResValue::Int(600); 256]),
        ),
    ]))
    .unwrap()
}

/// 解析内容流取首 Text 元素（走完整解析链——与 Lead 反例流一致）。
fn parse_first(src: &[u8]) -> PdeText {
    use mdc_pdf::pde::model::PdeElement;
    use mdc_pdf::pde::parser::parse_content;
    let mut c = parse_content(src).expect("解析须成");
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
}

/// **Lead 反例原例**：`50 Tz 2 Tc 4 Tw [(A ) 200 (B)] TJ ET`。
/// 期望 14.6（逐项：A 4.6 + 空格 6.6 + TJ −1.2 + B 4.6）。
#[test]
fn lead_tz50_tc2_tw4_tj200() {
    let t = parse_first(b"BT /F1 12 Tf 50 Tz 2 Tc 4 Tw [(A ) 200 (B)] TJ ET");
    let font = courier_600();
    let adv = t.run_advance(0, Some(&font)).expect("度量须有值");
    assert!(
        (adv - 14.6).abs() < 1e-3,
        "Tz 整体乘子（Tc/Tw/TJ 均乘）：期望 14.6，实际 {adv}"
    );
}

/// Tz=200 放大侧：`(3×7.2 + 3×2 + 4 − 2.4)×2 = 58.4`。
#[test]
fn tz200_amplifies_all_terms() {
    let t = parse_first(b"BT /F1 12 Tf 200 Tz 2 Tc 4 Tw [(A ) 200 (B)] TJ ET");
    let font = courier_600();
    let adv = t.run_advance(0, Some(&font)).expect("度量须有值");
    assert!(
        (adv - 58.4).abs() < 1e-3,
        "Tz 200 全项 ×2：期望 58.4，实际 {adv}"
    );
}

/// 负 TJ（松排）× Tz：`[(AB) -150 (C)]` Tz 50 →
/// (3×7.2 + 3×2 + 1.8)×0.5 = 29.4×0.5 = 14.7。
#[test]
fn negative_tj_with_tz() {
    let t = parse_first(b"BT /F1 12 Tf 50 Tz 2 Tc [(AB) -150 (C)] TJ ET");
    let font = courier_600();
    let adv = t.run_advance(0, Some(&font)).expect("度量须有值");
    assert!(
        (adv - 14.7).abs() < 1e-3,
        "负 TJ（松排）也乘 Tz：期望 14.7，实际 {adv}"
    );
}

/// Tc 单独 × Tz（无 Tw/TJ）：`"AB"` Tz 50 Tc 3 → (2×7.2 + 2×3)×0.5
/// = 10.2。
#[test]
fn tc_alone_multiplied_by_tz() {
    let t = parse_first(b"BT /F1 12 Tf 50 Tz 3 Tc (AB) Tj ET");
    let font = courier_600();
    let adv = t.run_advance(0, Some(&font)).expect("度量须有值");
    assert!((adv - 10.2).abs() < 1e-3, "Tc 乘 Tz：期望 10.2，实际 {adv}");
}

/// Tw 单独 × Tz：`"A B"`（空格 code 32）Tz 50 Tw 6 →
/// (3×7.2 + 6)×0.5 = 13.8。
#[test]
fn tw_alone_multiplied_by_tz() {
    let t = parse_first(b"BT /F1 12 Tf 50 Tz 6 Tw (A B) Tj ET");
    let font = courier_600();
    let adv = t.run_advance(0, Some(&font)).expect("度量须有值");
    assert!((adv - 13.8).abs() < 1e-3, "Tw 乘 Tz：期望 13.8，实际 {adv}");
}

/// 多字节字体下 Tw 不适用（§5.2.2）——Tz 仍作用于宽度与 Tc：
/// Type0 Identity-H `<0102 0103>`（两 CID 600/缺省 250）、Tz 50
/// Tc 2、字号 10 → ((600+250)/1000×10 + 2×Tc)×0.5 = (8.5+4)×0.5
/// = 6.25（Tc 对两个 CID 各计一次）。
#[test]
fn tz_with_type0_tc_no_tw() {
    let font = PdeFont::from_res_value(&ResValue::Dict(vec![
        (b"Subtype".to_vec(), ResValue::Name(b"Type0".to_vec())),
        (b"Encoding".to_vec(), ResValue::Name(b"Identity-H".to_vec())),
        (
            b"DescendantFonts".to_vec(),
            ResValue::Array(vec![ResValue::Dict(vec![
                (
                    b"Subtype".to_vec(),
                    ResValue::Name(b"CIDFontType2".to_vec()),
                ),
                (
                    b"W".to_vec(),
                    ResValue::Array(vec![
                        ResValue::Int(258),
                        ResValue::Array(vec![ResValue::Int(600)]),
                    ]),
                ),
            ])]),
        ),
    ]))
    .unwrap();
    let st = TextState {
        size: 10.0,
        char_spacing: 2.0,
        word_spacing: 6.0, // 多字节下不适用——不进结果
        h_scale: 50.0,
        ..Default::default()
    };
    let t = PdeText {
        gstate: GState::default(),
        runs: vec![TextRun {
            state: st,
            text_matrix: IDENTITY,
            shows: vec![ShowOp::Str(vec![0x01, 0x02, 0x01, 0x03])],
        }],
    };
    let adv = t.run_advance(0, Some(&font)).expect("度量须有值");
    assert!(
        (adv - 6.25).abs() < 1e-3,
        "Type0 + Tz50 + Tc2（Tw 不适用）：期望 6.25，实际 {adv}"
    );
}

/// **返工扩面：未知 CMap 拒绝**——Type0 无 /Encoding（或非 Identity-H
/// 名）时 `run_advance` 必须返回 None（不按 2 字节 CID 返回错误成功
/// 值）。Lead 指认：`num_code_bytes` 恒 2 与「未知 CMap 返回 None」
/// 声明不符。
#[test]
fn unknown_cmap_advance_is_none() {
    // 无 /Encoding 的 Type0。
    let unknown = PdeFont::from_res_value(&ResValue::Dict(vec![
        (b"Subtype".to_vec(), ResValue::Name(b"Type0".to_vec())),
        (
            b"BaseFont".to_vec(),
            ResValue::Name(b"HeiseiMin-W3".to_vec()),
        ),
    ]))
    .unwrap();
    assert_eq!(unknown.num_code_bytes(b"\x01\x02"), None, "未知 CMap 拒绝");
    let st = TextState {
        size: 10.0,
        ..Default::default()
    };
    let t = PdeText {
        gstate: GState::default(),
        runs: vec![TextRun {
            state: st,
            text_matrix: IDENTITY,
            shows: vec![ShowOp::Str(vec![0x01, 0x02])],
        }],
    };
    assert!(
        t.run_advance(0, Some(&unknown)).is_none(),
        "未知 CMap → run_advance 必须 None（错误成功值比缺能力更糟）"
    );
}

/// 预定义 CMap 名（UniGB-UCS2-H）与 Identity-V（垂直）同样拒绝。
#[test]
fn predefined_cmap_and_vertical_are_refused() {
    for enc in [&b"UniGB-UCS2-H"[..], &b"Identity-V"[..]] {
        let f = PdeFont::from_res_value(&ResValue::Dict(vec![
            (b"Subtype".to_vec(), ResValue::Name(b"Type0".to_vec())),
            (b"Encoding".to_vec(), ResValue::Name(enc.to_vec())),
        ]))
        .unwrap();
        assert_eq!(
            f.num_code_bytes(b"\x01\x02"),
            None,
            "Encoding {} → 拒绝（仅 Identity-H 支持）",
            String::from_utf8_lossy(enc)
        );
    }
}

/// 残码（Str 长非码长整数倍）拒绝——简单字体面不存在（码长 1 恒整），
/// Identity-H 下 3 字节串 = 1.5 码 → None。
#[test]
fn odd_length_cid_string_is_refused() {
    let font = PdeFont::from_res_value(&ResValue::Dict(vec![
        (b"Subtype".to_vec(), ResValue::Name(b"Type0".to_vec())),
        (b"Encoding".to_vec(), ResValue::Name(b"Identity-H".to_vec())),
    ]))
    .unwrap();
    assert_eq!(font.code_len_of(&[0x01, 0x02, 0x03]), None, "残码拒绝");
    let st = TextState {
        size: 10.0,
        ..Default::default()
    };
    let t = PdeText {
        gstate: GState::default(),
        runs: vec![TextRun {
            state: st,
            text_matrix: IDENTITY,
            shows: vec![ShowOp::Str(vec![0x01, 0x02, 0x03])],
        }],
    };
    assert!(
        t.run_advance(0, Some(&font)).is_none(),
        "残码串 → None（不猜）"
    );
}
