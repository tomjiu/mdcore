//! pdf_pde_layer —— issue #162 批①验收面：PDE 对象模型 + 内容流
//! 解析/写回 round-trip + Adobe 语义锚（run 合并/路径段模型/paint
//! 算子/addAfterIndex/inline image/marked-content/未知算子保留）。
//!
//! 定位：**验收聚合**——单元语义锁在 `pde::lexer`/`pde::parser` 内联
//! `#[cfg(test)]`，本文件锁 Adobe PDFEdit 语义对照面（用户主指令：
//! 「以 Adobe PDFEdit/PDE 为产品架构参照」——每个测试锚一条 Adobe
//! 文档语义，文档引用见各测试注释）。
//!
//! 覆盖矩阵（Adobe 语义 ← 测试）：
//! | Adobe 语义 | 测试 |
//! |---|---|
//! | PDEContent "workhorse"（acquire→modify→set 生命周期） | `content_lifecycle_*` |
//! | PDEContentAddElem addAfterIndex（kPDEBeforeFirst=-1） | `add_elem_positions` |
//! | PDEText run 合并（"combines text from multiple Tj"） | `text_run_merge_*` |
//! | PDETextGetText/GetTextMatrix/GetNumRuns | `text_run_*` |
//! | PDEElementSetMatrix（元素重定位） | `element_set_matrix_*` |
//! | PDEPath 段模型 + paint op | `path_segments_paint_op` |
//! | PDEClip（W/W* → 元素 GState） | `clip_semantics` |
//! | PDEImage（inline BI/ID/EI） | `inline_image_*` |
//! | PDEForm 承载（Do 名字引用——递归根批②） | `xobject_name_reference` |
//! | PDEContainer（BMC/BDC/EMC 嵌套） | `marked_content_nesting` |
//! | PDEUnknown（未识别算子原样保留） | `unknown_operator_preserved` |
//! | PDEContentToCosObj（写回 round-trip 语义等价） | `roundtrip_*` |
//! | q/Q 展平（元素快照吸收） | `gstate_flatten_*` |

use mdc_pdf::pde::model::*;

/// 构造最小 BT..ET 文本块内容流。
fn text_stream() -> Vec<u8> {
    b"BT /F1 12 Tf 72 720 Td (Hello) Tj ET".to_vec()
}

// ===== 1. PDEContent 生命周期与元素操作 =====

/// Adobe：PDEContent 是 "the workhorse of the PDFEdit API"，从页内容
/// 解析出元素序列（PDPageAcquirePDEContent 的内容流面）。
#[test]
fn content_lifecycle_parse_and_count() {
    let c = PdeContent::parse(&text_stream()).unwrap();
    assert_eq!(c.num_elems(), 1);
    assert_eq!(c.elem(0).unwrap().type_name(), "PDEText");
    // PDEContentCreate（空内容）
    let empty = PdeContent::new();
    assert_eq!(empty.num_elems(), 0);
}

/// Adobe：PDEContentAddElem(pdeContent, addAfterIndex, pdeElement)——
/// addAfterIndex = kPDEBeforeFirst(-1) 插到开头；n 插到第 n 之后。
#[test]
fn add_elem_positions() {
    let mut c = PdeContent::parse(b"1 0 0 1 10 10 cm 0 0 100 50 re f").unwrap();
    assert_eq!(c.num_elems(), 1);
    // -1 → 开头
    let img = PdeElement::Shading(PdeShading {
        gstate: GState::default(),
        name: b"S1".to_vec(),
    });
    c.add_elem(PDE_BEFORE_FIRST, img.clone());
    assert_eq!(c.elem(0).unwrap().type_name(), "PDEShading");
    // 0 → 第 0 个之后
    c.add_elem(
        0,
        PdeElement::Place(PdePlace {
            tag: b"P".to_vec(),
            properties: None,
        }),
    );
    assert_eq!(c.elem(1).unwrap().type_name(), "PDEPlace");
    // 越界（>= num）→ 追加
    c.add_elem(99, img);
    assert_eq!(c.elem(3).unwrap().type_name(), "PDEShading");
    assert_eq!(c.num_elems(), 4);
    // PDEContentRemoveElem
    let removed = c.remove_elem(1).unwrap();
    assert_eq!(removed.type_name(), "PDEPlace");
    assert_eq!(c.num_elems(), 3);
    assert!(c.remove_elem(99).is_none());
}

// ===== 2. PDEText run 语义（Adobe 合并判据） =====

/// Adobe："PDFEdit combines text from multiple Tj operators into a
/// single text run, when possible"——同状态同 Tm 的连续 Tj 合并。
#[test]
fn text_run_merge_same_state() {
    // 两个 Tj，中间无状态/定位算子——合并为一个 run
    let c = PdeContent::parse(b"BT /F1 12 Tf 72 720 Td (Hel) Tj (lo) Tj ET").unwrap();
    match c.elem(0).unwrap() {
        PdeElement::Text(t) => {
            assert_eq!(t.num_runs(), 1, "同状态连续 Tj 应合并为单 run");
            assert_eq!(t.run_text(0).unwrap(), b"Hello");
        }
        _ => panic!("expected PDEText"),
    }
}

/// 状态或 Tm 变化即断开（Adobe："text runs … with the same attributes"）。
#[test]
fn text_run_break_on_state_change() {
    // Td 介入（Tm 变化）→ 两 run
    let c = PdeContent::parse(b"BT /F1 12 Tf 72 720 Td (Hel) Tj 0 -14 Td (lo) Tj ET").unwrap();
    match c.elem(0).unwrap() {
        PdeElement::Text(t) => {
            assert_eq!(t.num_runs(), 2, "Tm 变化应断开 run");
            assert_eq!(t.run_text(0).unwrap(), b"Hel");
            assert_eq!(t.run_text(1).unwrap(), b"lo");
        }
        _ => panic!("expected PDEText"),
    }
    // Tf 介入（字号变化）→ 两 run
    let c2 = PdeContent::parse(b"BT /F1 12 Tf 72 720 Td (Hel) Tj /F1 14 Tf (lo) Tj ET").unwrap();
    match c2.elem(0).unwrap() {
        PdeElement::Text(t) => {
            assert_eq!(t.num_runs(), 2, "文本状态变化应断开 run");
            assert_eq!(t.runs[0].state.size, 12.0);
            assert_eq!(t.runs[1].state.size, 14.0);
        }
        _ => panic!("expected PDEText"),
    }
}

/// Tm 直接设定 + TJ 数组（kern 混合）+ GetTextMatrix 面。
#[test]
fn text_run_matrix_and_tj_array() {
    let src = b"BT /F2 10 Tf 1 0 0 1 100 200 Tm [(A) -120 (B) 40 (C)] TJ ET";
    let c = PdeContent::parse(src).unwrap();
    match c.elem(0).unwrap() {
        PdeElement::Text(t) => {
            assert_eq!(t.num_runs(), 1);
            let r = &t.runs[0];
            assert_eq!(r.text_matrix, [1.0, 0.0, 0.0, 1.0, 100.0, 200.0]);
            // TJ 数组：3 Str + 2 Kern
            assert_eq!(r.shows.len(), 5);
            assert_eq!(t.run_text(0).unwrap(), b"ABC");
        }
        _ => panic!("expected PDEText"),
    }
}

/// BT..ET 块的 gstate 快照 = BT 进入时的图形态（PDF：BT 不重置
/// 图形态；Adobe PDEText 元素持一份 gstate）。
#[test]
fn text_element_gstate_snapshot() {
    let src = b"1 0 0 1 5 5 cm 0 0 1 rg BT /F1 12 Tf (x) Tj ET";
    let c = PdeContent::parse(src).unwrap();
    match c.elem(0).unwrap() {
        PdeElement::Text(t) => {
            assert_eq!(t.gstate.ctm, [1.0, 0.0, 0.0, 1.0, 5.0, 5.0]);
            assert_eq!(t.gstate.fill.space, b"DeviceRGB");
        }
        _ => panic!("expected PDEText"),
    }
}

/// q/Q 嵌套的 CTM 复合（行向量级联：内层 cm 在左）。
#[test]
fn q_q_ctm_composition() {
    let src = b"q 2 0 0 2 0 0 cm q 1 0 0 1 10 20 cm 0 0 1 1 re f Q Q";
    let c = PdeContent::parse(src).unwrap();
    match c.elem(0).unwrap() {
        PdeElement::Path(p) => {
            // 期望：cm(10,20) × cm(scale2) = [2 0 0 2 20 40]
            assert_eq!(p.gstate.ctm, [2.0, 0.0, 0.0, 2.0, 20.0, 40.0]);
        }
        _ => panic!("expected PDEPath"),
    }
}

// ===== 3. PDEPath 段模型与 paint =====

/// Adobe PDEPath：段序列 + paint op（S/s/f/f*/B/B*/b/b*/n）。
#[test]
fn path_segments_paint_op() {
    let src = b"10 10 m 20 10 l 20 20 l h S";
    let c = PdeContent::parse(src).unwrap();
    match c.elem(0).unwrap() {
        PdeElement::Path(p) => {
            assert_eq!(p.paint, PaintOp::Stroke);
            assert_eq!(p.segs.len(), 4);
            assert!(matches!(p.segs[0], PathSeg::MoveTo { x: 10.0, y: 10.0 }));
            assert!(matches!(p.segs[3], PathSeg::Close));
        }
        _ => panic!("expected PDEPath"),
    }
    // f* 奇偶填充 + re 矩形
    let c2 = PdeContent::parse(b"5 5 30 20 re f*").unwrap();
    match c2.elem(0).unwrap() {
        PdeElement::Path(p) => {
            assert_eq!(p.paint, PaintOp::FillEvenOdd);
            assert!(matches!(
                p.segs[0],
                PathSeg::Rect {
                    x: 5.0,
                    y: 5.0,
                    w: 30.0,
                    h: 20.0
                }
            ));
        }
        _ => panic!("expected PDEPath"),
    }
    // 曲线三形态
    let c3 = PdeContent::parse(b"0 0 m 1 2 3 4 5 6 c 7 8 9 10 v 1 2 3 4 y h B").unwrap();
    match c3.elem(0).unwrap() {
        PdeElement::Path(p) => {
            assert_eq!(p.paint, PaintOp::FillStroke);
            assert!(matches!(p.segs[1], PathSeg::CurveTo { .. }));
            assert!(matches!(p.segs[2], PathSeg::CurveToV { .. }));
            assert!(matches!(p.segs[3], PathSeg::CurveToY { .. }));
        }
        _ => panic!("expected PDEPath"),
    }
}

/// PDEElementGetBBox（路径面）：段顶点 × CTM 的轴对齐包围盒。
#[test]
fn path_bbox_via_ctm() {
    let src = b"q 1 0 0 1 100 200 cm 0 0 50 30 re f Q";
    let c = PdeContent::parse(src).unwrap();
    match c.elem(0).unwrap() {
        PdeElement::Path(p) => {
            let bb = PdeElement::Path(p.clone()).bbox().unwrap();
            assert_eq!(bb.x0(), 100.0);
            assert_eq!(bb.y0(), 200.0);
            assert_eq!(bb.x1(), 150.0);
            assert_eq!(bb.y1(), 230.0);
        }
        _ => panic!("expected PDEPath"),
    }
}

/// PDEClip：W/W* 使路径成为后续元素 clip（GState 吸收面）。
#[test]
fn clip_semantics() {
    let src = b"0 0 100 100 re W n 1 0 0 1 10 10 cm 50 50 m 60 50 l S";
    let c = PdeContent::parse(src).unwrap();
    // W n：纯裁剪不产出路径元素；后续路径元素吸收 clip
    assert_eq!(c.num_elems(), 1);
    match c.elem(0).unwrap() {
        PdeElement::Path(p) => {
            let clip = p.gstate.clip.as_ref().expect("clip 应被吸收");
            assert!(!clip.even_odd);
            assert_eq!(clip.segs.len(), 1);
        }
        _ => panic!("expected PDEPath"),
    }
}

// ===== 4. inline image =====

/// PDEImage（inline）：属性表 + 原始数据字节。
#[test]
fn inline_image_attrs_and_data() {
    // /F [/AHx]：数据域为 ASCII hex 文本——批①语义 = 原始字节保留
    //（解码归批② PDEImage 数据面）
    let src = b"q 100 0 0 50 10 10 cm BI /W 2 /H 1 /BPC 8 /CS /G /F [/AHx] ID <FF00FF> EI Q";
    let c = PdeContent::parse(src).unwrap();
    match c.elem(0).unwrap() {
        PdeElement::Image(img) => {
            assert_eq!(img.attrs.len(), 5);
            assert_eq!(img.attrs[0].0, b"W");
            assert_eq!(img.attrs[0].1, Operand::Int(2));
            assert_eq!(img.data, b"<FF00FF>");
            // CTM 落位（图像空间 [0,1]² × CTM）
            let bb = PdeElement::Image(img.clone()).bbox().unwrap();
            assert_eq!(bb.x0(), 10.0);
            assert_eq!(bb.y0(), 10.0);
            assert_eq!(bb.x1(), 110.0);
            assert_eq!(bb.y1(), 60.0);
        }
        _ => panic!("expected PDEImage"),
    }
}

/// 二进制数据含 `EI` 歧义字节的宽收（Acrobat 启发式：空白+EI+定界）。
#[test]
fn inline_image_binary_with_ei_ambiguity() {
    // 数据里嵌 "E I"——扫描器必须找到真正的终结 EI（其后是定界符 q）
    let data = vec![0x00, 0x01, 0x02, b' ', b'E', b'I', b' ', 0xFF, 0xFE];
    let mut src = b"BI /W 3 /H 1 /BPC 8 /CS /G ID ".to_vec();
    src.extend_from_slice(&data);
    src.extend_from_slice(b" EI q");
    let c = PdeContent::parse(&src).unwrap();
    match c.elem(0).unwrap() {
        PdeElement::Image(img) => {
            assert_eq!(img.data, data);
        }
        _ => panic!("expected PDEImage"),
    }
}

// ===== 5. XObject / Shading / marked content / unknown =====

/// `/Name Do` → PDEXObject 名字引用形态（PDEFormGetContent 递归编辑
/// 需资源表解引用——批②；批①锁定引用面）。
#[test]
fn xobject_name_reference() {
    let c = PdeContent::parse(b"q 1 0 0 1 5 5 cm /Fm0 Do Q").unwrap();
    match c.elem(0).unwrap() {
        PdeElement::XObject(x) => {
            assert_eq!(x.name, b"Fm0");
            assert_eq!(x.gstate.ctm, [1.0, 0.0, 0.0, 1.0, 5.0, 5.0]);
        }
        _ => panic!("expected PDEXObject"),
    }
}

/// PDEContainer：BMC/BDC..EMC 嵌套分组。
#[test]
fn marked_content_nesting() {
    let src =
        b"/P <</MCID 0>> BDC (inner) BT /F1 12 Tf (x) Tj ET EMC /Artifact BMC 0 0 10 10 re f EMC";
    let c = PdeContent::parse(src).unwrap();
    assert_eq!(c.num_elems(), 2);
    match c.elem(0).unwrap() {
        PdeElement::Container(cont) => {
            assert_eq!(cont.tag, b"P");
            assert!(cont.properties.is_some());
            assert_eq!(cont.content.num_elems(), 1);
            assert_eq!(cont.content.elem(0).unwrap().type_name(), "PDEText");
        }
        _ => panic!("expected PDEContainer"),
    }
    match c.elem(1).unwrap() {
        PdeElement::Container(cont) => {
            assert_eq!(cont.tag, b"Artifact");
            assert!(cont.properties.is_none());
            assert_eq!(cont.content.elem(0).unwrap().type_name(), "PDEPath");
        }
        _ => panic!("expected PDEContainer"),
    }
}

/// PDEUnknown：未识别算子（操作数原样）。
#[test]
fn unknown_operator_preserved() {
    let c = PdeContent::parse(b"3.14 /X [1 2 (s)] d0").unwrap();
    match c.elem(0).unwrap() {
        PdeElement::Unknown(u) => {
            assert_eq!(u.op, b"d0");
            assert_eq!(u.operands.len(), 3);
            assert_eq!(u.operands[0], Operand::Real(3.14));
        }
        _ => panic!("expected PDEUnknown"),
    }
}

// ===== 6. PDEElementSetMatrix（元素重定位） =====

/// Adobe：PDEElementSetMatrix 重定位元素（路径 CTM 直换）。
#[test]
fn element_set_matrix_relocates() {
    let mut c = PdeContent::parse(b"q 1 0 0 1 10 10 cm 0 0 5 5 re f Q").unwrap();
    c.elem_mut(0)
        .unwrap()
        .set_matrix([1.0, 0.0, 0.0, 1.0, 500.0, 600.0]);
    match c.elem(0).unwrap() {
        PdeElement::Path(p) => {
            assert_eq!(p.gstate.ctm, [1.0, 0.0, 0.0, 1.0, 500.0, 600.0]);
        }
        _ => panic!("expected PDEPath"),
    }
}

// ===== 7. round-trip（PDEContentToCosObj 内容流面） =====

/// parse → write → parse 语义等价（元素数/类型/run 文本/段/颜色）。
#[test]
fn roundtrip_semantic_equivalence() {
    let cases: Vec<Vec<u8>> = vec![
        text_stream(),
        b"0 0 100 50 re f".to_vec(),
        b"q 1 0 0 1 5 5 cm 0 0 1 rg 10 10 m 20 20 l S Q".to_vec(),
        b"BT /F1 12 Tf 72 720 Td (Hello) Tj 0 -14 Td (World) Tj ET".to_vec(),
        b"BT /F2 10 Tf [(A) -120 (B)] TJ ET".to_vec(),
        b"/Fm0 Do".to_vec(),
        b"/S1 sh".to_vec(),
        b"q 100 0 0 50 10 10 cm BI /W 2 /H 1 /BPC 8 /CS /G ID \x01\x02 EI Q".to_vec(),
        b"/P <</MCID 7>> BDC 0 0 9 9 re f EMC".to_vec(),
        b"1.5 w 2 setlinecap 0 0 5 5 re S".to_vec(), // setlinecap 未建模 → Unknown
    ];
    for src in cases {
        let c1 = PdeContent::parse(&src)
            .unwrap_or_else(|e| panic!("parse 失败 {e:?}: {}", String::from_utf8_lossy(&src)));
        let bytes = c1.write();
        let c2 = PdeContent::parse(&bytes)
            .unwrap_or_else(|e| panic!("re-parse 失败 {e:?}: {}", String::from_utf8_lossy(&bytes)));
        assert_eq!(
            c1,
            c2,
            "round-trip 语义不等价\nsrc: {}\nout: {}",
            String::from_utf8_lossy(&src),
            String::from_utf8_lossy(&bytes)
        );
    }
}

/// round-trip 后 PDEText 的 run 数与文本保持（多次写回幂等）。
#[test]
fn roundtrip_idempotent_text() {
    let src = b"BT /F1 12 Tf 72 720 Td (Hello) Tj 0 -14 Td (World) Tj ET";
    let c1 = PdeContent::parse(src).unwrap();
    let b1 = c1.write();
    let c2 = PdeContent::parse(&b1).unwrap();
    let b2 = c2.write();
    assert_eq!(b1, b2, "二次写回应逐字节幂等");
    match (c1.elem(0).unwrap(), c2.elem(0).unwrap()) {
        (PdeElement::Text(t1), PdeElement::Text(t2)) => {
            assert_eq!(t1.num_runs(), t2.num_runs());
            assert_eq!(t1.run_text(0).unwrap(), t2.run_text(0).unwrap());
        }
        _ => panic!("expected PDEText"),
    }
}

// ===== 8. 错误面 =====

/// typed 错误：ET 无 BT / show 无 BT / show 缺字体。
#[test]
fn parse_errors_typed() {
    let e = PdeContent::parse(b"ET").unwrap_err();
    assert!(matches!(e, PdeError::Parse { ref op, .. } if op == "ET"));
    let e2 = PdeContent::parse(b"(x) Tj").unwrap_err();
    assert!(matches!(e2, PdeError::Parse { ref op, .. } if op == "Tj"));
    // show 无字体（Tf 未设）——Adobe：无字体的 show 不可绘制
    let e3 = PdeContent::parse(b"BT (x) Tj ET").unwrap_err();
    assert!(matches!(e3, PdeError::Parse { ref op, .. } if op == "Tj"));
}

// ===== 9. 宽收面（生产文件病态形态） =====

/// q/Q 不配平（EOF 残留栈）/ BT 未闭合——宽收产出而非报错。
#[test]
fn lenient_eof_unclosed() {
    let c = PdeContent::parse(b"q 1 0 0 1 5 5 cm 0 0 9 9 re f").unwrap();
    assert_eq!(c.num_elems(), 1); // q 未闭合：元素照常产出
    let c2 = PdeContent::parse(b"BT /F1 12 Tf (x) Tj").unwrap();
    assert_eq!(c2.num_elems(), 1); // BT 未闭合：文本照常产出
}

// ===== 10. Adobe 命名对照（API 面一致性） =====

/// 方法名与 Adobe PDFEdit 对齐（编译期对照清单——本测试锁存在性）。
#[test]
fn adobe_method_surface_present() {
    let mut c = PdeContent::parse(b"0 0 1 1 re f").unwrap();
    // PDEContentGetNumElems / GetElem / RemoveElem / AddElem
    let n = c.num_elems();
    let _ = c.elem(0);
    let _ = c.remove_elem(0);
    c.add_elem(
        PDE_BEFORE_FIRST,
        PdeElement::Place(PdePlace {
            tag: b"P".to_vec(),
            properties: None,
        }),
    );
    assert_eq!(c.num_elems(), n); // remove 1 + add 1
                                  // PDEText 面（run 数/文本/矩阵）
    let t = PdeContent::parse(&text_stream()).unwrap();
    match t.elem(0).unwrap() {
        PdeElement::Text(tex) => {
            assert_eq!(tex.num_runs(), 1);
            assert_eq!(tex.run_text(0).unwrap(), b"Hello");
        }
        _ => panic!(),
    }
}
