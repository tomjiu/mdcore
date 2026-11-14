//! #195 独立反例——TJ 调整与 Type0 CID 度量的错误成功值。
//!
//! issue 证据两例 + spacing/hscale 变体。全部走**公共 API**
//! （`PdeFont::from_res_value` 字典构造 + `PdeText::run_advance`），
//! 不碰产品内部私有面。修前核心两例 FAIL（返回错误成功值——
//! 本组测试的存在意义）。

use mdc_pdf::pde::font::PdeFont;
use mdc_pdf::pde::model::IDENTITY;
use mdc_pdf::pde::model::{GState, PdeText, ShowOp, TextRun, TextState};
use mdc_pdf::pde::resources::ResValue;

/// Courier 简单字体：/Widths 全 600、FirstChar 0、LastChar 255。
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

/// Type0 + Identity-H + descendant CIDFontType2，/W 给 CID258 宽 600。
fn type0_identity_h(missing: Option<i64>) -> PdeFont {
    let mut desc = vec![
        (b"Type".to_vec(), ResValue::Name(b"Font".to_vec())),
        (
            b"Subtype".to_vec(),
            ResValue::Name(b"CIDFontType2".to_vec()),
        ),
        (
            b"BaseFont".to_vec(),
            ResValue::Name(b"Courier-CID".to_vec()),
        ),
        (
            b"W".to_vec(),
            ResValue::Array(vec![
                ResValue::Int(258),
                ResValue::Array(vec![ResValue::Int(600)]),
            ]),
        ),
    ];
    if let Some(m) = missing {
        desc.push((
            b"FontDescriptor".to_vec(),
            ResValue::Dict(vec![(b"MissingWidth".to_vec(), ResValue::Int(m))]),
        ));
    }
    PdeFont::from_res_value(&ResValue::Dict(vec![
        (b"Type".to_vec(), ResValue::Name(b"Font".to_vec())),
        (b"Subtype".to_vec(), ResValue::Name(b"Type0".to_vec())),
        (
            b"BaseFont".to_vec(),
            ResValue::Name(b"Courier-CID".to_vec()),
        ),
        (b"Encoding".to_vec(), ResValue::Name(b"Identity-H".to_vec())),
        (
            b"DescendantFonts".to_vec(),
            ResValue::Array(vec![ResValue::Dict(desc)]),
        ),
    ]))
    .unwrap()
}

/// 单 run 的 PdeText。
fn text_with(state: TextState, shows: Vec<ShowOp>) -> PdeText {
    PdeText {
        gstate: GState::default(),
        runs: vec![TextRun {
            state,
            text_matrix: IDENTITY,
            shows,
        }],
    }
}

/// 反例 1（issue 原例）：`[(A) 200 (BCD)] TJ`——Courier 600、字号 12。
/// 总 advance = 4×600×12/1000 − 200×12/1000 = 28.8 − 2.4 = **26.4**。
/// 修前：run_advance = 28.8（TJ Kern 200 被丢弃）→ FAIL。
#[test]
fn tj_kern_reduces_advance_courier() {
    let font = courier_600();
    let st = TextState {
        size: 12.0,
        ..Default::default()
    };
    let t = text_with(
        st,
        vec![
            ShowOp::Str(b"A".to_vec()),
            ShowOp::Kern(200.0),
            ShowOp::Str(b"BCD".to_vec()),
        ],
    );
    let adv = t.run_advance(0, Some(&font)).expect("有字体须有值");
    assert!(
        (adv - 26.4).abs() < 1e-3,
        "TJ Kern(200) 必须参与 advance：期望 26.4，实际 {adv}"
    );
}

/// 反例 1b：TJ 负调整（松排——负数把后续字形向**右**移）：
/// `[(AB) -150 (C)] TJ`：3×7.2 + 1.8 = 23.4。
#[test]
fn tj_negative_kern() {
    let font = courier_600();
    let st = TextState {
        size: 12.0,
        ..Default::default()
    };
    let t = text_with(
        st,
        vec![
            ShowOp::Str(b"AB".to_vec()),
            ShowOp::Kern(-150.0),
            ShowOp::Str(b"C".to_vec()),
        ],
    );
    let adv = t.run_advance(0, Some(&font)).expect("有字体须有值");
    assert!(
        (adv - 23.4).abs() < 1e-3,
        "TJ 负 Kern（松排）必须参与：期望 23.4，实际 {adv}"
    );
}

/// 反例 2（issue 原例）：Type0 Identity-H、内容 `<0102>`（一个 CID
/// 0x0102=258，/W 给 600）、字号 10 → advance = 600×10/1000 = **6**。
/// 修前：逐 u8 查宽（1 与 2 各走缺省 250）→ 5.0 → FAIL。
#[test]
fn type0_cid_two_byte_code_lookup() {
    let font = type0_identity_h(None);
    assert_eq!(
        font.num_code_bytes(b"\x01\x02"),
        Some(2),
        "Identity-H 码长 = 2"
    );
    let st = TextState {
        size: 10.0,
        ..Default::default()
    };
    let t = text_with(st, vec![ShowOp::Str(vec![0x01, 0x02])]);
    let adv = t.run_advance(0, Some(&font)).expect("有字体须有值");
    assert!(
        (adv - 6.0).abs() < 1e-3,
        "CID 258 宽 600 字号 10 → 6.0（按码长切 CID 查宽），实际 {adv}"
    );
}

/// 反例 2b：两 CID 连写 `<0102 0103>`（258=600、259 走 missing 250）
/// → (600+250)×10/1000 = 8.5——锁「连续 CID 均按 2 字节切分」。
#[test]
fn type0_two_cids_sequential() {
    let font = type0_identity_h(Some(250));
    let st = TextState {
        size: 10.0,
        ..Default::default()
    };
    let t = text_with(st, vec![ShowOp::Str(vec![0x01, 0x02, 0x01, 0x03])]);
    let adv = t.run_advance(0, Some(&font)).expect("有字体须有值");
    assert!(
        (adv - 8.5).abs() < 1e-3,
        "两 CID (600+250)×10/1000 = 8.5，实际 {adv}"
    );
}

/// 变体：char spacing（Tc）与 word spacing（Tw，简单字体空格生效）。
/// Courier 600、字号 10、Tc=0.5：`"AB C"` → 4×6.0 + 4×0.5 + Tw=2.0
/// （仅 code 32）= 24 + 2 + 2 = 28。
#[test]
fn char_and_word_spacing_simple_font() {
    let font = courier_600();
    let st = TextState {
        size: 10.0,
        char_spacing: 0.5,
        word_spacing: 2.0,
        ..Default::default()
    };
    let t = text_with(st, vec![ShowOp::Str(b"AB C".to_vec())]);
    let adv = t.run_advance(0, Some(&font)).expect("有字体须有值");
    assert!(
        (adv - 28.0).abs() < 1e-3,
        "Tc/Tw 进 advance：期望 28.0，实际 {adv}"
    );
}

/// 变体：hscale（Tz=50%）——Courier 600、字号 10、`"AB"` →
/// (600×10/1000×0.5)×2 = 6.0。
#[test]
fn h_scale_halves_advance() {
    let font = courier_600();
    let st = TextState {
        size: 10.0,
        h_scale: 50.0,
        ..Default::default()
    };
    let t = text_with(st, vec![ShowOp::Str(b"AB".to_vec())]);
    let adv = t.run_advance(0, Some(&font)).expect("有字体须有值");
    assert!(
        (adv - 6.0).abs() < 1e-3,
        "Tz 50% 半缩 advance：期望 6.0，实际 {adv}"
    );
}

/// 多 run：无字体 → None（不猜——已有行为锁定为契约面）。
#[test]
fn no_font_is_none_not_guess() {
    let st = TextState {
        size: 12.0,
        ..Default::default()
    };
    let t = text_with(st, vec![ShowOp::Str(b"ABCD".to_vec())]);
    assert!(
        t.run_advance(0, None).is_none(),
        "无字体上下文必须 None（诚实降级契约）"
    );
}
