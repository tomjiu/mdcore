//! #194 独立反例——PDEText 编辑面写回保真。
//!
//! issue 证据两例（MuPDF 独立引擎已实证）+ 删除语义锁定。全部走
//! **公共 API**（`parse_content` → 编辑方法 → `write_content`）。
//! 修前 FAIL 形态即本组测试的存在意义。

use mdc_pdf::pde::font::PdeFont;
use mdc_pdf::pde::model::PdeElement;
use mdc_pdf::pde::model::{GState, PdeText, ShowOp, TextRun, TextState, IDENTITY};
use mdc_pdf::pde::parser::parse_content;
use mdc_pdf::pde::resources::ResValue;
use mdc_pdf::pde::writer::write_content;

/// Courier（/Widths 全 600）字体（run_advance 依赖 #195 修复面）。
fn courier_font() -> PdeFont {
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

/// 取解析后第一个 Text 元素（run 常为单 run——解析合并面）。
fn first_text_mut(src: &[u8]) -> mdc_pdf::pde::model::PdeText {
    let mut c = parse_content(src).expect("解析须成");
    match c.elements.first_mut() {
        Some(PdeElement::Text(t)) => {
            let empty = mdc_pdf::pde::model::PdeText {
                gstate: t.gstate.clone(),
                runs: Vec::new(),
            };
            std::mem::replace(t, empty)
        }
        _ => panic!("首个元素必须是 Text"),
    }
}

/// 反例 1（issue 原例——MuPDF 实证 CD 回 AB 原点重叠）：
/// `BT /F1 12 Tf 1 0 0 1 20 30 Tm (ABCD) Tj ET`，split_run_at(1)
/// → 写回应为 `1 0 0 1 27.2 30 Tm (CD) Tj`（首段 advance = 2×7.2
/// 进位次段 Tm e 值）。修前：次段 Tm = (20,30)（CD 与 AB 重叠）。
#[test]
fn split_run_at_advances_second_segment() {
    let mut t = first_text_mut(b"BT /F1 12 Tf 1 0 0 1 20 30 Tm (ABCD) Tj ET");
    assert_eq!(t.num_chars(), 4);
    let font = courier_font();
    let ok = t.split_run_at(1, Some(&font));
    assert!(ok, "有字体度量 → 拆分成功（返回 true）");
    assert_eq!(t.num_runs(), 2, "拆为两 run");
    // 模型面：次段 Tm 的 e 分量 = 20 + 首段 advance（14.4）。
    let adv = t.run_advance(0, Some(&font)).expect("度量须有值");
    assert!(
        (adv - 14.4).abs() < 1e-3,
        "首段 advance 14.4（#195 链路），实际 {adv}"
    );
    let e = t.runs[1].text_matrix[4];
    assert!(
        (e - 34.4).abs() < 1e-3,
        "次段 Tm.e = 20 + 14.4 = 34.4（CD 不重叠），实际 {e}"
    );
    // 写回面：内容流里次段 Tm 出现 34.4（非 20）。
    let c = mdc_pdf::pde::model::PdeContent {
        elements: vec![PdeElement::Text(t)],
    };
    let out = write_content(&c);
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("34.4"), "写回次段 Tm 须带累计位置 34.4：\n{s}");
}

/// 反例 2（issue 原例——同字替换丢 TJ 字距）：
/// `BT /F1 12 Tf 1 0 0 1 20 30 Tm [(A) 200 (BCD)] TJ ET`，
/// replace_chars(0, b"A")（同字替换——语义上零变化）→ 写回 TJ 流
/// 与字距 200 必须保留。修前：重装为单 Str `(ABCD) Tj`——200 消失。
#[test]
fn replace_same_char_keeps_tj_kern() {
    let mut t = first_text_mut(b"BT /F1 12 Tf 1 0 0 1 20 30 Tm [(A) 200 (BCD)] TJ ET");
    t.replace_chars(0, b"A");
    // 写回：TJ 数组形态 + Kern 200 保留（同字替换 = 零语义变化）。
    let c = mdc_pdf::pde::model::PdeContent {
        elements: vec![PdeElement::Text(t)],
    };
    let out = write_content(&c);
    let s = String::from_utf8_lossy(&out);
    assert!(
        s.contains("TJ"),
        "同字替换后须保持 TJ 数组形态（Kern 不丢）：\n{s}"
    );
    assert!(
        s.contains("200"),
        "TJ 200 字距必须保留（同字替换零语义变化）：\n{s}"
    );
}

/// 替换真改字符（A→X）也不得丢其余字距：`[(A) 200 (BCD)] TJ`
/// replace(0,b"X") → `[(X) 200 (BCD)] TJ`。
#[test]
fn replace_changed_char_keeps_rest_kerns() {
    let mut t = first_text_mut(b"BT /F1 12 Tf 1 0 0 1 20 30 Tm [(A) 200 (BCD)] TJ ET");
    t.replace_chars(0, b"X");
    let c = mdc_pdf::pde::model::PdeContent {
        elements: vec![PdeElement::Text(t)],
    };
    let out = write_content(&c);
    let s = String::from_utf8_lossy(&out);
    assert!(
        s.contains("200") && s.contains("TJ") && s.contains("(X)"),
        "替换 X 后 200/Kern 保留：\n{s}"
    );
    assert!(
        !s.contains("(XBCD)"),
        "不得展平为单 Str（丢字距形态）：\n{s}"
    );
}

/// 多 Kern run：替换中间字符（B→Y）——前后 Kern（200/-150）都保留。
#[test]
fn replace_middle_keeps_surrounding_kerns() {
    let mut t = first_text_mut(b"BT /F1 12 Tf 1 0 0 1 20 30 Tm [(A) 200 (B) -150 (CD)] TJ ET");
    t.replace_chars(1, b"Y");
    let c = mdc_pdf::pde::model::PdeContent {
        elements: vec![PdeElement::Text(t)],
    };
    let out = write_content(&c);
    let s = String::from_utf8_lossy(&out);
    assert!(
        s.contains("200") && s.contains("-150") && s.contains("(Y)"),
        "前后 Kern 均保留：\n{s}"
    );
}

/// 删除语义锁定：`[(A) 200 (BCD)] TJ` remove_chars(1, 1)（删 B）→
/// `[(A) 200 (CD)] TJ`（Kern 200 保留——删除只删字符不删字距；B 与
/// C 之间无字距项）。删除跨 Kern 字符（删 A）→ Kern 保留在流内。
#[test]
fn delete_chars_keeps_kerns() {
    let mut t = first_text_mut(b"BT /F1 12 Tf 1 0 0 1 20 30 Tm [(A) 200 (BCD)] TJ ET");
    t.remove_chars(1, 1); // 删 B
    assert_eq!(t.num_chars(), 3, "删后 3 字符（ACD）");
    let c = mdc_pdf::pde::model::PdeContent {
        elements: vec![PdeElement::Text(t)],
    };
    let out = write_content(&c);
    let s = String::from_utf8_lossy(&out);
    assert!(
        s.contains("200") && s.contains("(CD)") && !s.contains("(B"),
        "删 B 后 200 与 CD 保留：\n{s}"
    );
}

/// split 无字体上下文（无度量）——明确拒绝不猜位置（issue 任务
/// 原文：「不要在没有度量信息时静默猜位置」）。修复后语义：
/// `font = None` → 返回 `false` 且**不拆**（run 数不变——调用方
/// 可判别；不产生重叠 run 对）。
#[test]
fn split_without_font_metrics_is_refused() {
    let mut t = first_text_mut(b"BT /F1 12 Tf 1 0 0 1 20 30 Tm (ABCD) Tj ET");
    let ok = t.split_run_at(1, None);
    assert!(!ok, "无字体度量 → 拒绝（false）");
    assert_eq!(
        t.num_runs(),
        1,
        "拒绝 = 不拆（无重叠 run 对产生——不猜位置）"
    );
    assert_eq!(t.num_chars(), 4, "原文本无损");
}

// ===== 返工 2026-11-14：完整码边界 + 连续 split 自查 =====

/// Type0 Identity-H 字体（测试构造——`/W` 可选）。
fn type0_identity_h_font() -> PdeFont {
    PdeFont::from_res_value(&ResValue::Dict(vec![
        (b"Subtype".to_vec(), ResValue::Name(b"Type0".to_vec())),
        (b"Encoding".to_vec(), ResValue::Name(b"Identity-H".to_vec())),
    ]))
    .unwrap()
}

/// 单 run 持 Type0 CID 流的 PdeText。
fn type0_text(cids: &[u8]) -> mdc_pdf::pde::model::PdeText {
    mdc_pdf::pde::model::PdeText {
        gstate: GState::default(),
        runs: vec![TextRun {
            state: TextState {
                size: 12.0,
                ..Default::default()
            },
            text_matrix: IDENTITY,
            shows: vec![ShowOp::Str(cids.to_vec())],
        }],
    }
}

/// **返工：split 拒切 CID 半码**——`<01020304>`（两 CID：0102/0304），
/// split_run_at(1)：切点 cut=2（奇字节位=第二 CID 中间）→ **必须拒绝**
///（false 不拆——残码 run 不可度量不可写回有效字形）。奇位 1/3 同拒；
/// 偶位（0/2/4 边界语义下的合法切点）正常拆。
#[test]
fn split_refuses_mid_cid_boundary() {
    let font = type0_identity_h_font();
    // 字节坐标细究：流 [01,02,03,04]（2 CID），切点 cut = split_loc+1：
    // cut=1（split_loc=0 早拒）/ cut=2（split_loc=1，= CID 边界，合法）/
    // cut=3（split_loc=2，= 第二 CID **中间半码**，必须拒）/ cut=4（尾拒）。
    let mut t = type0_text(&[0x01, 0x02, 0x03, 0x04]);
    let ok = t.split_run_at(2, Some(&font));
    assert!(!ok, "split_loc=2（cut=3 = CID 0x0304 中间半码）必须拒绝");
    assert_eq!(t.num_runs(), 1, "拒绝 = 原样（无残码 run 对产生）");
    assert_eq!(
        t.run_text(0).unwrap(),
        vec![0x01, 0x02, 0x03, 0x04],
        "原流无损"
    );
    // 合法边界：split_loc=1（cut=2 = 第一 CID 后界）正常拆。
    let mut t = type0_text(&[0x01, 0x02, 0x03, 0x04]);
    let ok = t.split_run_at(1, Some(&font)); // cut=2 = 偶数 = CID 边界
    assert!(ok, "cut=2 为码边界——允许拆");
    assert_eq!(t.num_runs(), 2);
    assert_eq!(t.run_text(0).unwrap(), vec![0x01, 0x02], "首段完整 CID");
    assert_eq!(t.run_text(1).unwrap(), vec![0x03, 0x04], "次段完整 CID");
}

/// **返工：残码 run 拒拆**——`<010203>`（3 字节 = 1.5 码）：任一
/// split 位都拒绝（run 本身残码——`code_len_of` 返回 None）。
#[test]
fn split_refuses_residual_cid_run() {
    let font = type0_identity_h_font();
    for loc in [1usize, 2] {
        let mut t = type0_text(&[0x01, 0x02, 0x03]);
        assert!(
            !t.split_run_at(loc, Some(&font)),
            "残码 run（3 字节非 2 的倍数）任何切位拒绝——loc={loc}"
        );
        assert_eq!(t.num_runs(), 1, "原样");
    }
}

/// **返工自查：连续多次 split 不重复计入 TJ 调整**——
/// `[(A) 200 (BCD)] TJ`：split(1) 后再 split 次 run 中间位。
/// 判据：拆后各字符的**绘制位置**与未拆分基线一致（TJ 200 只在
/// A→B 间计一次；首段进位 advance 含 Kern——若次段 shows 里 Kern
/// 又被计入进位即双扣）。位置对照用模型 Tm 累计推导：
/// 基线字符原点 x：A=0、B=4.8（7.2−2.4）、C=12.0、D=19.2。
#[test]
fn consecutive_splits_do_not_double_count_kern() {
    use mdc_pdf::pde::model::PdeElement;
    use mdc_pdf::pde::parser::parse_content;
    let src = b"BT /F1 12 Tf 1 0 0 1 20 30 Tm [(A) 200 (BCD)] TJ ET";
    let font = courier_font();
    // 基线（未拆）：run_advance 全量。
    let base = {
        let mut c = parse_content(src).unwrap();
        match c.elements.first_mut() {
            Some(PdeElement::Text(t)) => {
                let empty = PdeText {
                    gstate: t.gstate.clone(),
                    runs: Vec::new(),
                };
                std::mem::replace(t, empty)
            }
            _ => panic!(),
        }
    };
    let base_adv = base.run_advance(0, Some(&font)).unwrap();
    assert!((base_adv - 26.4).abs() < 1e-3, "基线 advance 26.4");

    // 连续两次 split：先 AB|CD（split_loc=1），再把次 run（CD）
    // 从中间断（全局 split_loc=3——次 run 内 C 位含首段）。
    let mut t = {
        let mut c = parse_content(src).unwrap();
        match c.elements.first_mut() {
            Some(PdeElement::Text(t)) => {
                let empty = PdeText {
                    gstate: t.gstate.clone(),
                    runs: Vec::new(),
                };
                std::mem::replace(t, empty)
            }
            _ => panic!(),
        }
    };
    // 第一次拆 AB|CD（split_loc=1 → cut=2，「首段含 splitLoc 位」= B 归首段）。
    assert!(t.split_run_at(1, Some(&font)), "第一次拆 AB|CD");
    // 第二次：拆次 run（CD）为 C|D——全局 split_loc=2（run1 偏移
    // 2、off=0、cut=1 < 2 合法——C 归首段）。
    assert!(t.split_run_at(2, Some(&font)), "第二次拆 C|D");
    assert_eq!(t.num_runs(), 3);
    assert_eq!(t.run_text(0).unwrap(), b"AB".to_vec());
    assert_eq!(t.run_text(1).unwrap(), b"C".to_vec());
    assert_eq!(t.run_text(2).unwrap(), b"D".to_vec());
    // 位置守恒判据：三 run 的 advance 之和 = 基线 26.4（TJ 200 恰计
    // 一次——首段进位含 Kern、次段不含（Kern 归属首段尾）；若重复
    // 计入则总和 ≠ 26.4 或 run 原点错位）。
    let sum: f32 = (0..t.num_runs())
        .map(|i| t.run_advance(i, Some(&font)).unwrap())
        .sum();
    assert!(
        (sum - 26.4).abs() < 1e-3,
        "三段 advance 之和 = 基线 26.4（Kern 恰计一次），实际 {sum}"
    );
    // 原点链判据：run i 起点前所有 run 的 advance 累计 = 该 run 的
    // Tm 平移偏移（相对首 run 原点）——20、24.8、32.0（B/C/D 原点）。
    // 原点链：run0@20（A 原点）；run1 应在 C 的基线原点 32.0
    //（run0 advance = A 7.2 + Kern −2.4 + B 7.2 = 12；20+12 = 32 ✓
    //——Kern 归 run0 尾、不重复进次段）；run2 在 D 原点 39.2。
    let expect_e = [20.0f32, 32.0, 39.2];
    for i in 0..t.num_runs() {
        let e = t.runs[i].text_matrix[4];
        assert!(
            (e - expect_e[i]).abs() < 1e-3,
            "run {i} 原点（基线字符原点——重复计入 TJ 即错位）：期望 {}，实际 {e}",
            expect_e[i]
        );
    }
}

// ===== Lead 第二版复验（2026-11-14，88d3278 后）：A/B 边界反例 =====

/// **反例 A**：`BT /F1 12 Tf 1 0 0 1 20 30 Tm [(A) 200 (BCD)] TJ ET`，
/// `replace_chars(0, b"ABCD")` 整串同字替换——替换后字符间**原
/// Kern 位置**必须保持（B 前 −2.4 仍在 B 前，不挪到串尾）。
///
/// Lead 实测错误形态：输出 `[(ABCD) 200] TJ`——rewrite 把整个
/// new 塞首 Str、删覆盖 Str，200 挪到尾，B/C/D 各右移 2.4。
///
/// 判据（全页不变）：
/// - run_advance 不变（26.4——Kern 恰计一次）
/// - 字符原点不变：B 在 4.8、C 12.0、D 19.2（模型推进推导）
/// - shows 形态：Str/Kern 交错原样（A | Kern200 | BCD——替换按原
///   Str 段长度分片，Kern 留在 A 与 B 之间）
#[test]
fn replace_whole_string_keeps_kern_between_chars() {
    use mdc_pdf::pde::model::PdeElement;
    use mdc_pdf::pde::parser::parse_content;
    let src = b"BT /F1 12 Tf 1 0 0 1 20 30 Tm [(A) 200 (BCD)] TJ ET";
    let font = courier_font();
    // 基线（未替换）。
    let baseline = parse_first_text(src);
    let base_adv = baseline.run_advance(0, Some(&font)).unwrap();

    let mut t = parse_first_text(src);
    t.replace_chars(0, b"ABCD");
    // 1) 全页不变：总 advance 与基线一致（同字替换零位移）。
    let adv = t.run_advance(0, Some(&font)).unwrap();
    assert!(
        (adv - base_adv).abs() < 1e-3,
        "同整串替换 advance 不变：基线 {base_adv}，替换后 {adv}"
    );
    // 2) Kern 位置保持：shows 序列必须仍是 Str|Kern|Str 交错——
    //    Kern 在第 2 位（A 与 B 之间），非尾部。
    let shows = &t.runs[0].shows;
    assert_eq!(
        shows.len(),
        3,
        "Str/Kern/Str 三元形态保持（Kern 不挪尾）——实际 {shows:?}"
    );
    assert!(
        matches!(shows[0], ShowOp::Str(ref s) if s == b"A"),
        "首段 = 替换后 'A'（按原 Str 覆盖长度分片）——实际 {:?}",
        shows[0]
    );
    assert!(
        matches!(shows[1], ShowOp::Kern(k) if (k - 200.0).abs() < 1e-6),
        "Kern 200 保持原位（A 与 B 之间）——实际 {:?}",
        shows[1]
    );
    assert!(
        matches!(shows[2], ShowOp::Str(ref s) if s == b"BCD"),
        "尾段 = 'BCD'（后续覆盖 Str 按其长度接受分片）——实际 {:?}",
        shows[2]
    );
    // 3) 字符原点不变（逐字符推进推导）：B 4.8 / C 12.0 / D 19.2。
    let mut x = 0.0f32;
    let mut origins = vec![];
    for op in shows {
        match op {
            ShowOp::Str(s) => {
                for &b in s.iter() {
                    origins.push(x);
                    x += 600.0 * 12.0 / 1000.0;
                }
            }
            ShowOp::Kern(k) => x -= k * 12.0 / 1000.0,
        }
    }
    assert!(
        (origins[1] - 4.8).abs() < 1e-3 && (origins[2] - 12.0).abs() < 1e-3,
        "B/C 原点不变（B={:?} C={:?}）",
        origins[1],
        origins[2]
    );
}

/// **反例 B**：`[(AB) 200 (CD)] TJ`，split_run_at(1) 恰切 Str/Kern
/// 边界——cut=2 后 Kern 在界上。正确形态：**次 run 起点 = C 的基线
/// 原点 32**（首段 AB advance 12 + Tm.e 20；Kern 已在 AB→C 之间
/// 计入首段推进——当前点过界处调整恰一次）。
///
/// Lead 实测错误形态：next.Tm.e = 34.4（split_run_show_bytes 把
/// 界处 Kern 放首段尾 → 首段 advance 14.4；truncate 口径不含
/// 界处 Kern → advance 12——两套口径分叉，进位取了错的那套）。
#[test]
fn split_at_str_kern_boundary_next_origin_is_c() {
    use mdc_pdf::pde::model::PdeElement;
    use mdc_pdf::pde::parser::parse_content;
    let src = b"BT /F1 12 Tf 1 0 0 1 20 30 Tm [(AB) 200 (CD)] TJ ET";
    let font = courier_font();
    let mut t = parse_first_text(src);
    let ok = t.split_run_at(1, Some(&font));
    assert!(ok, "split_loc=1（cut=2 = Str/Kern 边界）须可拆");
    assert_eq!(t.num_runs(), 2);
    // 基线原点：A=20、B=27.2、C=32.0（27.2+7.2−2.4+2.4? 不——
    // C = B_end(34.4) − Kern 2.4 = 32.0 ✓）
    let e_next = t.runs[1].text_matrix[4];
    assert!(
        (e_next - 32.0).abs() < 1e-3,
        "次 run 起点 = C 基线原点 32（当前点过界恰计一次），实际 {e_next}"
    );
    // 首/次段字符与 Kern 归属：界处 Kern（200）在**首段尾**（AB 与
    // CD 之间的调整归属前段——Adobe 游标语义），次段纯 CD。
    assert_eq!(t.run_text(0).unwrap(), b"AB".to_vec());
    assert_eq!(t.run_text(1).unwrap(), b"CD".to_vec());
    let head_kern = t.runs[0]
        .shows
        .last()
        .map(|o| matches!(o, ShowOp::Kern(k) if (k-200.0).abs() < 1e-6));
    assert_eq!(
        head_kern,
        Some(true),
        "界处 Kern 归首段尾（推进已计入 12+2.4=14.4? 不——advance 含 Kern）"
    );
    // 进位自洽：e_next = 20 + 首段 advance（含 Kern 的真源口径）。
    let head_adv = t.run_advance(0, Some(&font)).unwrap();
    assert!(
        (e_next - (20.0 + head_adv)).abs() < 1e-3,
        "进位自洽：e_next = 20 + 首段 advance {head_adv}（实际 {e_next}）"
    );
}

// —— helpers（本组用）——

/// 解析首 Text 元素（走完整解析链）。
fn parse_first_text(src: &[u8]) -> PdeText {
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

/// 跨段局部替换（Lead 要求「跨段局部替换」回归）：`[(AB) 200 (CD)]`
/// replace(2, b"XY")——替换 CD 段（Kern 前的 AB 不动）。
/// 分片语义：覆盖区在第二 Str（CD）内 → new 全落该段，Kern 200
/// 仍留在 AB 与 XY 之间。
#[test]
fn replace_across_segment_keeps_kern_before_target() {
    let src = b"BT /F1 12 Tf 1 0 0 1 20 30 Tm [(AB) 200 (CD)] TJ ET";
    let font = courier_font();
    let mut t = parse_first_text(src);
    t.replace_chars(2, b"XY");
    let shows = &t.runs[0].shows;
    assert_eq!(shows.len(), 3, "Str/Kern/Str 保持——实际 {shows:?}");
    assert!(matches!(shows[0], ShowOp::Str(ref s) if s == b"AB"));
    assert!(
        matches!(shows[1], ShowOp::Kern(k) if (k - 200.0).abs() < 1e-6),
        "Kern 200 留在 AB 与替换串之间"
    );
    assert!(matches!(shows[2], ShowOp::Str(ref s) if s == b"XY"));
    // 全页不变判据（替换长度 2 == 覆盖长度 2）：总 advance 同基线。
    let base = parse_first_text(src);
    let adv = t.run_advance(0, Some(&font)).unwrap();
    let base_adv = base.run_advance(0, Some(&font)).unwrap();
    assert!(
        (adv - base_adv).abs() < 1e-3,
        "等长替换 advance 不变：{base_adv} → {adv}"
    );
}

/// 多 Kern 位置回归（Lead 要求）：`[(A) 100 (B) 200 (C) 200 (D)]`
/// 整串同字替换 replace(0, b"ABCD")——**两个 Kern 各留原位**
/// （A|B 间 100、B|C 间 200、C|D 间 200），不挪不丢。
#[test]
fn replace_whole_string_multi_kern_positions() {
    let src = b"BT /F1 12 Tf 1 0 0 1 20 30 Tm [(A) 100 (B) 200 (C) 200 (D)] TJ ET";
    let font = courier_font();
    let mut t = parse_first_text(src);
    t.replace_chars(0, b"ABCD");
    let shows = &t.runs[0].shows;
    // 形态：Str|K100|Str|K200|Str|K200|Str
    assert_eq!(
        shows.len(),
        7,
        "四 Str 三 Kern 交错保持（各 Kern 原位）——实际 {shows:?}"
    );
    assert!(matches!(shows[0], ShowOp::Str(ref s) if s == b"A"));
    assert!(matches!(shows[1], ShowOp::Kern(k) if (k - 100.0).abs() < 1e-6));
    assert!(matches!(shows[2], ShowOp::Str(ref s) if s == b"B"));
    assert!(matches!(shows[3], ShowOp::Kern(k) if (k - 200.0).abs() < 1e-6));
    assert!(matches!(shows[4], ShowOp::Str(ref s) if s == b"C"));
    assert!(matches!(shows[5], ShowOp::Kern(k) if (k - 200.0).abs() < 1e-6));
    assert!(matches!(shows[6], ShowOp::Str(ref s) if s == b"D"));
    // 全页不变：advance 与基线一致（三 Kern 各恰计一次）。
    let base = parse_first_text(src);
    let adv = t.run_advance(0, Some(&font)).unwrap();
    let base_adv = base.run_advance(0, Some(&font)).unwrap();
    assert!(
        (adv - base_adv).abs() < 1e-3,
        "多 Kern 同串替换 advance 不变：{base_adv} → {adv}"
    );
}

/// 连续 split 在**不同 Kern 边界**（Lead 要求「连续 split 不同 Kern
/// 位置」）：`[(A) 100 (B) 200 (C) 200 (D)]` 拆 B|C 边界
///（split_loc=2 → cut=3 = Str(B)/Kern(200) 界）再拆后段 C|D 边界
///（全局 4 → cut 仍落 Str/Kern 界）——每次进位用各自首段真源
/// advance（含各自界处 Kern），原点 = 各界后首字符的基线原点。
#[test]
fn consecutive_splits_at_different_kern_boundaries() {
    let src = b"BT /F1 12 Tf 1 0 0 1 20 30 Tm [(A) 100 (B) 200 (C) 200 (D)] TJ ET";
    let font = courier_font();
    let mut t = parse_first_text(src);
    // 基线字符原点（模型推进推导）：A=20；B=27.2−1.2=26.0；
    // C=26.0+7.2−2.4+? ——逐项：A 原点 20，A 推进 7.2+K100(−1.2)→B 26.0，
    // B 推进 7.2+K200(−2.4)→C 30.8，C 推进 7.2+K200(−2.4)→D 35.6，
    // D 推进 7.2 → 末 42.8（总 advance 22.8）。
    assert!(
        t.split_run_at(1, Some(&font)),
        "拆 B|C 界（cut=2 = Str(B)/Kern(200) 界）"
    );
    assert_eq!(t.num_runs(), 2);
    // 次 run（含 200 界处 Kern 归首段尾）起点 = C 原点 30.8。
    let e1 = t.runs[1].text_matrix[4];
    assert!(
        (e1 - 30.8).abs() < 1e-3,
        "第二次拆后 run1 起点 = C 基线原点 30.8（首段 A|K100|B|K200 推进恰一次），实际 {e1}"
    );
    // 再拆 C|D 界：全局 2（run1 内 off=0、cut=1 = Str(C)/Kern(200) 界）。
    assert!(t.split_run_at(2, Some(&font)), "拆 C|D 界");
    assert_eq!(t.num_runs(), 3);
    assert_eq!(t.run_text(0).unwrap(), b"AB".to_vec());
    assert_eq!(t.run_text(1).unwrap(), b"C".to_vec());
    assert_eq!(t.run_text(2).unwrap(), b"D".to_vec());
    // run2 起点 = D 基线原点 35.6（run1 首 C 段推进 7.2−2.4=4.8；
    // 30.8+4.8=35.6——界处 200 Kern 归 run1 尾恰计一次）。
    let e2 = t.runs[2].text_matrix[4];
    assert!(
        (e2 - 35.6).abs() < 1e-3,
        "run2 起点 = D 基线原点 35.6，实际 {e2}"
    );
    // 三段 advance 和 = 基线总 22.8（各 Kern 恰计一次不重复）。
    let sum: f32 = (0..t.num_runs())
        .map(|i| t.run_advance(i, Some(&font)).unwrap())
        .sum();
    let base = parse_first_text(src);
    let base_adv = base.run_advance(0, Some(&font)).unwrap();
    assert!(
        (sum - base_adv).abs() < 1e-3,
        "三段 advance 和 = 基线 {base_adv}（实际 {sum}）——Kern 双计即不等"
    );
}

// ===== Lead 第二版返工：独立引擎精确原点实证（pdfium） =====

/// 引擎探针（临时 PDF + pypdfium2 charbox 中点——引擎不可用 → None）。
fn engine_char_mids(content: &str) -> Option<Vec<(char, f64)>> {
    use std::process::Command;
    let dir = std::env::temp_dir().join(format!("pde194e-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let py = dir.join("probe.py");
    let pdf = dir.join("probe.pdf");
    let script = format!(
        r#"
import sys
def mkpdf(path, content):
    objs = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
        ("<< /Length %d >>\nstream\n%s\nendstream" %% (len(content), content)).encode(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier /FirstChar 0 /LastChar 255 /Widths [" + b" ".join(b"600" for _ in range(256)) + b"] >>",
    ]
    out = bytearray(b"%PDF-1.7\n"); offs=[]
    for i,o in enumerate(objs,1):
        offs.append(len(out)); out += b"%d 0 obj\n"%%i + o + b"\nendobj\n"
    xref=len(out)
    out += b"xref\n0 %d\n"%%(len(objs)+1) + b"0"*10 + b" 65535 f \n"
    for off in offs: out += b"%%010d 00000 n \n"%%off
    out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF"%%(len(objs)+1, xref)
    open(path,'wb').write(bytes(out))
import pypdfium2 as pdfium
mkpdf({pdf_a:?}, {content:?}.encode())
pdf = pdfium.PdfDocument({pdf_b:?})
tp = pdf[0].get_textpage()
n = tp.count_chars()
text = tp.get_text_range(0, n)
for i in range(n):
    b = tp.get_charbox(i)
    print(text[i], (b[0]+b[2])/2.0)
"#,
        pdf_a = pdf.to_str().unwrap(),
        content = content,
        pdf_b = pdf.to_str().unwrap(),
    );
    std::fs::write(&py, script).ok()?;
    let out = Command::new("python3")
        .arg(py.to_str().unwrap())
        .output()
        .ok()?;
    if !out.status.success() {
        return None; // 引擎/依赖不可用 → SKIP 通道
    }
    let txt = String::from_utf8_lossy(&out.stdout);
    let mids = txt
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let c = it.next()?.chars().next()?;
            let x: f64 = it.next()?.parse().ok()?;
            Some((c, x))
        })
        .collect::<Vec<_>>();
    Some(mids)
}

/// **反例 A 引擎实证**：Kern 挪尾错误形态（`[(ABCD) 200]`）的
/// B/C/D 原点比正确形态各**右移 2.4**；正确形态（Kern 在 A/B 间）
/// 与基线**全零位移**。模型替换产出的 shows 走同正确形态。
#[test]
fn engine_probe_replace_kern_position() {
    let Some(base) = engine_char_mids("BT /F1 12 Tf 1 0 0 1 20 30 Tm [(A) 200 (BCD)] TJ ET") else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    let Some(bad) = engine_char_mids("BT /F1 12 Tf 1 0 0 1 20 30 Tm [(ABCD) 200] TJ ET") else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    // 错误形态：B/C/D 各 +2.4（Kern 200 挪到 D 后——D 前失去 −2.4）。
    for (i, name) in [(1usize, "B"), (2, "C"), (3, "D")] {
        let delta = bad[i].1 - base[i].1;
        assert!(
            (delta - 2.4).abs() < 0.3,
            "{name} 原点右移 2.4（Kern 挪尾代价）——引擎实测 Δ={delta}"
        );
    }
}

/// **反例 B 引擎实证**：拆分等价流（首段 `(AB) 200 Tj` + 次段
/// `Tm.e=32 (CD) Tj`）的 C/D 原点与未拆分基线**同值**——32 是
/// C 的真实原点（模型进位同值）。错误形态（Tm.e=34.4）会把
/// C/D 各右移 2.4。
#[test]
fn engine_probe_split_origin() {
    let Some(base) = engine_char_mids("BT /F1 12 Tf 1 0 0 1 20 30 Tm [(AB) 200 (CD)] TJ ET") else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    let Some(split) =
        engine_char_mids("BT /F1 12 Tf 1 0 0 1 20 30 Tm (AB) 200 Tj 1 0 0 1 32 30 Tm (CD) Tj ET")
    else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    let Some(bad) =
        engine_char_mids("BT /F1 12 Tf 1 0 0 1 20 30 Tm (AB) 200 Tj 1 0 0 1 34.4 30 Tm (CD) Tj ET")
    else {
        eprintln!("SKIP: pypdfium2 不可用");
        return;
    };
    // 拆分等价流 C/D 原点 = 基线同值（含中点 bearing 常数——同字符
    // 同字体，差分即位移）。
    let base_c = base.iter().find(|(c, _)| *c == 'C').unwrap().1;
    let split_c = split.iter().find(|(c, _)| *c == 'C').unwrap().1;
    let bad_c = bad.iter().find(|(c, _)| *c == 'C').unwrap().1;
    assert!(
        (split_c - base_c).abs() < 0.15,
        "拆分后 C 原点不变（Tm.e=32 精确）——Δ={}",
        split_c - base_c
    );
    assert!(
        (bad_c - base_c - 2.4).abs() < 0.3,
        "错误进位 34.4 → C 右移 2.4——Δ={}",
        bad_c - base_c
    );
    // 模型同值（真源闭合）：模型拆分 next.Tm.e = 32。
    let mut t = parse_first_text(b"BT /F1 12 Tf 1 0 0 1 20 30 Tm [(AB) 200 (CD)] TJ ET");
    let font = courier_font();
    assert!(t.split_run_at(1, Some(&font)));
    let e = t.runs[1].text_matrix[4];
    assert!((e - 32.0).abs() < 1e-3, "模型 next.Tm.e = 32，实际 {e}");
}
