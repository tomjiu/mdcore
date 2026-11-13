//! pdf_pde_batch2 —— issue #162 批②验收面：资源表解引用 + PDEImage
//! 数据面 + 写回资源收集。
//!
//! 定位与批①（pdf_pde_layer.rs）同款：**验收聚合**——单元语义锁在
//! `pde::image`/`pde::resources` 内联 `#[cfg(test)]`，本文件锁 Adobe
//! PDFEdit 语义对照面。
//!
//! 覆盖矩阵（Adobe 语义 ← 测试）：
//! | Adobe 语义 | 测试 |
//! |---|---|
//! | PDEFormGetContent（form 递归 display list） | `form_deref_recurses_content` |
//! | PDEFormCreateFromCosObjEx（Matrix/BBox/Resources 供给） | `form_matrix_bbox_resources` |
//! | peErrUnknownResType（资源名字缺失） | `deref_missing_name_typed_error` |
//! | form 环引用深度防护 | `form_depth_limit_cycle` |
//! | PDEImage named 形态（XObject → PdeImage attrs） | `named_image_attrs_surface` |
//! | PDEImageGetData（"always returned as decoded data"） | `image_data_decoded_semantics` |
//! | 滤镜链（多段按序解码） | `filter_chain_order` |
//! | 内联图缩写/全名同收 | `inline_abbrev_full_name` |
//! | PDEContentToCosObj resourcesP（写回资源面） | `collect_resources_roundtrip` |
//! | 未引用资源不进写回产物 | `merge_collected_filters_unreferenced` |
//! | 引用面缺失本体探针 | `missing_from_probe` |
//! | 资源字典序列化（写回产物结构） | `resources_to_bytes_shape` |
//! | Container 内嵌 Do 解引用 | `container_nested_deref` |

use mdc_pdf::pde::image::{decode_image_data, image_attr_num, image_filters};
use mdc_pdf::pde::model::*;
use mdc_pdf::pde::resources::{
    deref_xobject, resolve_content, resolve_xobject_element, CollectedResources, ResValue,
    ResourceDict, XObjectKind, FORM_DEPTH_LIMIT,
};

/// 构造 form XObject 流资源：内容流 + Matrix/BBox。
fn form_stream(content: &[u8], matrix: Option<[f32; 6]>, bbox: Option<[f32; 4]>) -> ResValue {
    let mut dict: Vec<(Vec<u8>, ResValue)> = vec![
        (b"Type".to_vec(), ResValue::Name(b"XObject".to_vec())),
        (b"Subtype".to_vec(), ResValue::Name(b"Form".to_vec())),
    ];
    if let Some(m) = matrix {
        dict.push((
            b"Matrix".to_vec(),
            ResValue::Array(m.iter().map(|&v| ResValue::Real(v)).collect()),
        ));
    }
    if let Some(b) = bbox {
        dict.push((
            b"BBox".to_vec(),
            ResValue::Array(b.iter().map(|&v| ResValue::Real(v)).collect()),
        ));
    }
    dict.push((b"Length".to_vec(), ResValue::Int(content.len() as i64)));
    ResValue::Stream {
        dict,
        data: content.to_vec(),
    }
}

/// 构造 image XObject 流资源。
fn image_stream(w: i64, h: i64, bpc: i64, filter: Option<ResValue>, data: &[u8]) -> ResValue {
    let mut dict: Vec<(Vec<u8>, ResValue)> = vec![
        (b"Type".to_vec(), ResValue::Name(b"XObject".to_vec())),
        (b"Subtype".to_vec(), ResValue::Name(b"Image".to_vec())),
        (b"Width".to_vec(), ResValue::Int(w)),
        (b"Height".to_vec(), ResValue::Int(h)),
        (b"BitsPerComponent".to_vec(), ResValue::Int(bpc)),
        (
            b"ColorSpace".to_vec(),
            ResValue::Name(b"DeviceRGB".to_vec()),
        ),
    ];
    if let Some(f) = filter {
        dict.push((b"Filter".to_vec(), f));
    }
    dict.push((b"Length".to_vec(), ResValue::Int(data.len() as i64)));
    ResValue::Stream {
        dict,
        data: data.to_vec(),
    }
}

/// 页级资源字典：单 form + 单 image。
fn page_resources() -> ResourceDict {
    let root = ResValue::Dict(vec![
        (
            b"XObject".to_vec(),
            ResValue::Dict(vec![
                (
                    b"Fm0".to_vec(),
                    form_stream(
                        b"0 0 100 50 re S",
                        Some([2.0, 0.0, 0.0, 2.0, 10.0, 20.0]),
                        Some([0.0, 0.0, 100.0, 50.0]),
                    ),
                ),
                (
                    b"Im0".to_vec(),
                    image_stream(
                        2,
                        1,
                        8,
                        Some(ResValue::Name(b"ASCIIHexDecode".to_vec())),
                        b"FFFFFF000000>", // 2×1 RGB：白 + 黑
                    ),
                ),
            ]),
        ),
        (
            b"ExtGState".to_vec(),
            ResValue::Dict(vec![(
                b"GS0".to_vec(),
                ResValue::Dict(vec![(b"CA".to_vec(), ResValue::Real(0.5))]),
            )]),
        ),
        (
            b"Font".to_vec(),
            ResValue::Dict(vec![(
                b"F1".to_vec(),
                ResValue::Dict(vec![(b"Type".to_vec(), ResValue::Name(b"Font".to_vec()))]),
            )]),
        ),
        (
            b"Shading".to_vec(),
            ResValue::Dict(vec![(
                b"S1".to_vec(),
                ResValue::Dict(vec![(b"ShadingType".to_vec(), ResValue::Int(2))]),
            )]),
        ),
        // 非消费类（宽收忽略面）
        (
            b"Properties".to_vec(),
            ResValue::Dict(vec![(b"MC0".to_vec(), ResValue::Dict(vec![]))]),
        ),
    ]);
    ResourceDict::from_res_value(&root)
}

// ===== 1. Do 名字解引用 =====

/// Adobe：PDEFormGetContent——"A PDEContent may be obtained from a
/// PDEForm to edit the form's display list"。form 的内容流递归解析为
/// PdeContent（同 parse 入口，形态同构）。
#[test]
fn form_deref_recurses_content() {
    let res = page_resources();
    let kind = deref_xobject(b"Fm0", &res, 0).unwrap();
    match kind {
        mdc_pdf::pde::resources::XObjectKind::Form { content, .. } => {
            // form 的 display list：一条描边矩形路径
            assert_eq!(content.num_elems(), 1);
            assert_eq!(content.elem(0).unwrap().type_name(), "PDEPath");
            // round-trip：form content 也能写回内容流（批① write 面）
            let bytes = content.write();
            let re = PdeContent::parse(&bytes).unwrap();
            assert_eq!(re.num_elems(), 1);
        }
        _ => panic!("应为 Form 形态"),
    }
}

/// Adobe：PDEFormCreateFromCosObjEx(xObjectP, resourcesP, matrixP)——
/// form 的 /Matrix、/BBox、/Resources 从 Cos 层对象进入 PDEForm。
#[test]
fn form_matrix_bbox_resources() {
    let res = page_resources();
    let kind = deref_xobject(b"Fm0", &res, 0).unwrap();
    match kind {
        mdc_pdf::pde::resources::XObjectKind::Form {
            matrix,
            bbox,
            resources,
            ..
        } => {
            assert_eq!(matrix, [2.0, 0.0, 0.0, 2.0, 10.0, 20.0]);
            assert_eq!(bbox, Some([0.0, 0.0, 100.0, 50.0]));
            // 本 form 未带 /Resources → None（继承调用方面）
            assert!(resources.is_none());
        }
        _ => panic!("应为 Form 形态"),
    }
    // Matrix 缺省 = 单位阵（PDF 规范默认值面）
    let root = ResValue::Dict(vec![(
        b"XObject".to_vec(),
        ResValue::Dict(vec![(
            b"Fm1".to_vec(),
            form_stream(b"0 0 1 1 re f", None, None),
        )]),
    )]);
    let res = ResourceDict::from_res_value(&root);
    match deref_xobject(b"Fm1", &res, 0).unwrap() {
        mdc_pdf::pde::resources::XObjectKind::Form { matrix, bbox, .. } => {
            assert_eq!(matrix, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
            assert_eq!(bbox, None);
        }
        _ => panic!(),
    }
}

/// Adobe：peErrUnknownResType 同位——资源名字不在表 = typed 解析错误。
#[test]
fn deref_missing_name_typed_error() {
    let res = page_resources();
    let err = deref_xobject(b"NoSuch", &res, 0).unwrap_err();
    match err {
        PdeError::Parse { op, reason } => {
            assert_eq!(op, "Do");
            assert!(reason.contains("peErrUnknownResType"));
        }
        _ => panic!("应为 Parse 形态"),
    }
}

/// form 环引用防护：深度超限 = typed 错误（病态 A→B→A 环的防护面）。
#[test]
fn form_depth_limit_cycle() {
    let res = page_resources();
    let err = deref_xobject(b"Fm0", &res, FORM_DEPTH_LIMIT + 1).unwrap_err();
    match err {
        PdeError::Parse { op, reason } => {
            assert_eq!(op, "Do");
            assert!(reason.contains("环"));
        }
        _ => panic!("应为 Parse 形态"),
    }
}

// ===== 2. named image 形态 =====

/// Adobe：PDEImage 含 image XObject 与 inline image 两形态（"A PDEImage
/// is a PDEElement that contains an image XObject or inline image"）。
/// named XObject → PdeImage 的 attrs 面（W/H/BPC/CS/F——PDEImageAttrs
/// 对齐）。
#[test]
fn named_image_attrs_surface() {
    let res = page_resources();
    let content = PdeContent::parse(b"q 100 0 0 50 10 10 cm /Im0 Do Q").unwrap();
    let (resolved, kinds) = resolve_content(&content, &res).unwrap();
    assert_eq!(kinds.len(), 1);
    // XObject 引用元素 → Image 元素（named 形态转 PdeImage）
    match resolved.elem(0).unwrap() {
        PdeElement::Image(img) => {
            assert_eq!(image_attr_num(img, b"Width"), Some(2));
            assert_eq!(image_attr_num(img, b"Height"), Some(1));
            assert_eq!(image_attr_num(img, b"BitsPerComponent"), Some(8));
            // 滤镜与色彩空间进 attrs（保形承载）
            assert_eq!(image_filters(img), vec![b"ASCIIHexDecode".to_vec()]);
            // gstate 承载 CTM（q 100 0 0 50 10 10 cm 的快照）
            assert_eq!(img.gstate.ctm, [100.0, 0.0, 0.0, 50.0, 10.0, 10.0]);
        }
        _ => panic!("应为 Image 形态"),
    }
}

// ===== 3. PDEImageGetData decoded 语义 =====

/// Adobe：PDEImageGetData——"If the image is an XObject image, data is
/// always returned as decoded data"。滤镜解码链（含链式）。
#[test]
fn image_data_decoded_semantics() {
    let res = page_resources();
    let content = PdeContent::parse(b"q /Im0 Do Q").unwrap();
    let (resolved, _) = resolve_content(&content, &res).unwrap();
    match resolved.elem(0).unwrap() {
        PdeElement::Image(img) => {
            // AHx 解码：FFFFFF000000> → 6 字节 RGB 数据
            let decoded = decode_image_data(img).unwrap();
            assert_eq!(decoded, [0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00]);
        }
        _ => panic!("应为 Image 形态"),
    }
}

/// 滤镜链按序解码（AHx → Fl）：数据先 hex 解码再 zlib 解压。
#[test]
fn filter_chain_order() {
    // zlib.compress(b"Hi") = 78 9c f3 c8 04 00 00 fb 00 b2（Python zlib 产出）
    let zlib_hi: Vec<u8> = vec![0x78, 0x9c, 0xf3, 0xc8, 0x04, 0x00, 0x00, 0xfb, 0x00, 0xb2];
    let hexed: Vec<u8> = zlib_hi
        .iter()
        .flat_map(|b| format!("{:02X}", b).into_bytes())
        .collect();
    let mut data = hexed;
    data.push(b'>');
    let img = PdeImage {
        gstate: GState::default(),
        attrs: vec![(
            b"F".to_vec(),
            Operand::Array(vec![
                Operand::Name(b"AHx".to_vec()),
                Operand::Name(b"Fl".to_vec()),
            ]),
        )],
        data,
    };
    assert_eq!(decode_image_data(&img).unwrap(), b"Hi");
}

/// 内联图缩写与全名同收（PDF §4.8.6 缩写表——批① attrs 用缩写，
/// named image 用全名，两面同一取值器）。
#[test]
fn inline_abbrev_full_name() {
    let mk = |key: &[u8]| PdeImage {
        gstate: GState::default(),
        attrs: vec![
            (key.to_vec(), Operand::Name(b"AHx".to_vec())),
            (b"W".to_vec(), Operand::Int(3)),
        ],
        data: b"414243>".to_vec(),
    };
    let abbrev = mk(b"F");
    let full = mk(b"Filter");
    assert_eq!(image_filters(&abbrev), image_filters(&full));
    assert_eq!(decode_image_data(&abbrev).unwrap(), b"ABC");
    assert_eq!(decode_image_data(&full).unwrap(), b"ABC");
    assert_eq!(image_attr_num(&abbrev, b"Width"), Some(3));
    assert_eq!(
        image_attr_num(&full, b"Width"),
        Some(3),
        "W 缩写与 Width 全名同收"
    );
}

// ===== 4. 写回资源面（PDEContentToCosObj 的 resourcesP）=====

/// Adobe：PDEContentToCosObj 出参 contentsP + resourcesP——写回产物
/// 两件套。资源收集覆盖 Do/sh/Tf/gs/cs 五引用源。
#[test]
fn collect_resources_roundtrip() {
    // 注：`/CS0 cs` 后必须接数值分量 scn（名字分量 = Pattern 形态，
    // stream2 单测覆盖——两者不能在同一 stream 混用）。
    let stream = b"q /GS0 gs /CS0 cs 0.4 scn BT /F1 12 Tf 72 720 Td (Hi) Tj ET /Fm0 Do /S1 sh Q";
    let content = PdeContent::parse(stream).unwrap();
    let c = CollectedResources::from_content(&content);
    assert_eq!(c.xobject, vec![b"Fm0".to_vec()]);
    assert_eq!(c.shading, vec![b"S1".to_vec()]);
    assert_eq!(c.font, vec![b"F1".to_vec()]);
    assert_eq!(c.extgstate, vec![b"GS0".to_vec()]);
    assert_eq!(c.colorspace, vec![b"CS0".to_vec()]);
    // Pattern 名分量：scn 的名字形态
    let stream2 = b"/P1 scn 0 0 100 50 re f";
    let c2 = CollectedResources::from_content(&PdeContent::parse(stream2).unwrap());
    assert_eq!(c2.pattern, vec![b"P1".to_vec()]);
    // 去重：同名引用只计一次
    let stream3 = b"/Fm0 Do /Fm0 Do";
    let c3 = CollectedResources::from_content(&PdeContent::parse(stream3).unwrap());
    assert_eq!(c3.xobject, vec![b"Fm0".to_vec()]);
}

/// 未引用资源不进写回产物（Adobe：resources 只含 contents 引用到的
/// 资源——合并面按引用过滤）。
#[test]
fn merge_collected_filters_unreferenced() {
    let res = page_resources();
    let stream = b"q /GS0 gs BT /F1 12 Tf 72 720 Td (Hi) Tj ET /Fm0 Do Q";
    let content = PdeContent::parse(stream).unwrap();
    let c = CollectedResources::from_content(&content);
    let merged = res.merge_collected(&c);
    // 引用到的三件进产物
    assert_eq!(merged.xobject.len(), 1);
    assert_eq!(merged.font.len(), 1);
    assert_eq!(merged.extgstate.len(), 1);
    // 未引用的 Shading 不进产物
    assert_eq!(merged.shading.len(), 0);
    // 引用面本体完整：无缺失
    assert!(c.missing_from(&res).is_empty());
}

/// 引用面缺失本体探针（内容流引用了资源表没有的名字）。
#[test]
fn missing_from_probe() {
    let res = page_resources();
    let stream = b"/Fm0 Do /Ghost Do";
    let content = PdeContent::parse(stream).unwrap();
    let c = CollectedResources::from_content(&content);
    let missing = c.missing_from(&res);
    assert_eq!(missing, vec![b"Ghost".to_vec()]);
    // 缺失不阻断合并（静默跳过——完整性裁决归调用方）
    let merged = res.merge_collected(&c);
    assert_eq!(merged.xobject.len(), 1);
}

/// 资源字典序列化结构（写回产物面——名字清单形态）。
#[test]
fn resources_to_bytes_shape() {
    let stream = b"q /GS0 gs BT /F1 12 Tf 72 720 Td (Hi) Tj ET /Fm0 Do Q";
    let content = PdeContent::parse(stream).unwrap();
    let c = CollectedResources::from_content(&content);
    let bytes = c.to_bytes();
    let s = String::from_utf8(bytes.clone()).unwrap();
    assert!(s.starts_with("<< /ExtGState << /GS0 >> /Font << /F1 >> /XObject << /Fm0 >> >>"));
    // 空引用面 → 空字典形态
    let empty = CollectedResources::new();
    assert_eq!(empty.to_bytes(), b"<< >>");
    assert!(empty.is_empty());
}

// ===== 5. Container 嵌套解引用 =====

/// Container（marked-content）内的 Do 引用同样解引用（form 的
/// display list 里可以再 Do——Adobe 嵌套 form 同构）。
#[test]
fn container_nested_deref() {
    let res = page_resources();
    let stream = b"/P <</ActualText (x)>> BDC /Im0 Do EMC";
    let content = PdeContent::parse(stream).unwrap();
    let (resolved, kinds) = resolve_content(&content, &res).unwrap();
    assert_eq!(kinds.len(), 1);
    match resolved.elem(0).unwrap() {
        PdeElement::Container(c) => {
            assert_eq!(c.tag, b"P");
            match c.content.elem(0).unwrap() {
                PdeElement::Image(img) => {
                    assert_eq!(image_attr_num(img, b"Width"), Some(2));
                }
                _ => panic!("Container 内应为 Image"),
            }
        }
        _ => panic!("应为 Container 形态"),
    }
}

/// 单元素解析入口（resolve_xobject_element）的 form 面保持名字引用
/// 形态（写回 Do 原样发出）+ 解引用产物交调用方。
#[test]
fn form_keeps_name_reference() {
    let res = page_resources();
    let content = PdeContent::parse(b"q /Fm0 Do Q").unwrap();
    let (elem, kind) = resolve_xobject_element(content.elem(0).unwrap(), &res).unwrap();
    match (&elem, &kind) {
        (PdeElement::XObject(x), Some(_)) => {
            assert_eq!(x.name, b"Fm0");
        }
        _ => panic!("form 应保持 XObject 名字引用形态"),
    }
    // 写回：名字引用原样（Do 面）
    let out = PdeContent {
        elements: vec![elem],
    }
    .write();
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("/Fm0 Do"), "写回应含 /Fm0 Do：{s}");
}

// ===== #180/#181/#182 修复轮验收（round143 双向验收面：反例全灭） =====

/// #181：同一容器里两个 Form——解引用产物**全部**返回（旧实现只回
/// 第一个，第二个 form 的编辑/写回材料整体丢失——Lead 反例）。
#[test]
fn container_double_form_all_kinds() {
    let root = ResValue::Dict(vec![(
        b"XObject".to_vec(),
        ResValue::Dict(vec![
            (
                b"FmA".to_vec(),
                form_stream(b"0 0 1 rg 0 0 10 10 re f", None, None),
            ),
            (
                b"FmB".to_vec(),
                form_stream(b"0 1 0 rg 0 0 20 20 re f", None, None),
            ),
        ]),
    )]);
    let res = ResourceDict::from_res_value(&root);
    let content = PdeContent::parse(b"/P0 BDC q /FmA Do Q q /FmB Do Q EMC").unwrap();
    let (resolved, kinds) = resolve_content(&content, &res).unwrap();
    // 修复面：容器内 2 个 Do → kinds 全收（旧实现只回第一个）
    assert_eq!(kinds.len(), 2, "同一容器两个 Form 必须都返回");
    // 两个 kind 都是 Form 且 content 各自独立
    assert!(kinds.iter().all(|k| matches!(k, XObjectKind::Form { .. })));
    // 容器内元素仍为名字引用形态（写回 Do 原样）
    match resolved.elem(0).unwrap() {
        PdeElement::Container(c) => {
            let xos: Vec<&Vec<u8>> = c
                .content
                .elements
                .iter()
                .filter_map(|e| match e {
                    PdeElement::XObject(x) => Some(&x.name),
                    _ => None,
                })
                .collect();
            assert_eq!(xos, vec![&b"FmA".to_vec(), &b"FmB".to_vec()]);
        }
        _ => panic!("应为 Container"),
    }
}

/// #181：named image 的 `/Decode [1 0]`（灰度反色）全量透传进 attrs
/// （旧实现五项白名单——Decode 丢失，写回后图像颜色静默反转）。
#[test]
fn named_image_decode_array_passthrough() {
    let stream = ResValue::Stream {
        dict: vec![
            (b"Type".to_vec(), ResValue::Name(b"XObject".to_vec())),
            (b"Subtype".to_vec(), ResValue::Name(b"Image".to_vec())),
            (b"Width".to_vec(), ResValue::Int(2)),
            (b"Height".to_vec(), ResValue::Int(1)),
            (b"BitsPerComponent".to_vec(), ResValue::Int(8)),
            (
                b"ColorSpace".to_vec(),
                ResValue::Name(b"DeviceGray".to_vec()),
            ),
            // 反色表：0→255（Decode [1 0]——灰度分量反转）
            (
                b"Decode".to_vec(),
                ResValue::Array(vec![ResValue::Int(1), ResValue::Int(0)]),
            ),
            // 顺带验证其他键同收（Interpolate）
            (b"Interpolate".to_vec(), ResValue::Bool(true)),
        ],
        data: vec![0x10, 0x80],
    };
    let root = ResValue::Dict(vec![(
        b"XObject".to_vec(),
        ResValue::Dict(vec![(b"Im9".to_vec(), stream)]),
    )]);
    let res = ResourceDict::from_res_value(&root);
    let content = PdeContent::parse(b"q /Im9 Do Q").unwrap();
    let (resolved, _) = resolve_content(&content, &res).unwrap();
    match resolved.elem(0).unwrap() {
        PdeElement::Image(img) => {
            // Decode 反色表透传（数组形态保留）
            let decode = img.attrs.iter().find(|(k, _)| k == b"Decode");
            let (k, v) = decode.expect("Decode 必须透传进 attrs");
            assert_eq!(k, b"Decode");
            match v {
                Operand::Array(items) => {
                    assert_eq!(items, &vec![Operand::Int(1), Operand::Int(0)]);
                }
                _ => panic!("Decode 应为数组形态"),
            }
            // Interpolate 同收
            assert!(
                img.attrs.iter().any(|(k, _)| k == b"Interpolate"),
                "非白名单键全量透传"
            );
            // 基础五项仍在（缩写键）
            assert!(image_attr_num(img, b"Width") == Some(2));
        }
        _ => panic!("应为 Image 形态"),
    }
}

/// #181：form 递归 content 的引用收集（`from_content_with_forms`）——
/// Lead 实测：Do 进 form，form 内部引用 GS0/CS0/F0，页级收集全空，
/// 写回资源表丢引用。
#[test]
fn form_inner_refs_collected() {
    // form 内部：gs GS0 + cs CS0 + Tf F0
    let inner = b"/GS0 gs /CS0 cs 0.5 scn /F0 12 Tf BT 5 5 Td (Hi) Tj ET";
    let root = ResValue::Dict(vec![(
        b"XObject".to_vec(),
        ResValue::Dict(vec![(b"Fm0".to_vec(), form_stream(inner, None, None))]),
    )]);
    let res = ResourceDict::from_res_value(&root);
    let content = PdeContent::parse(b"q /Fm0 Do Q").unwrap();
    let (_resolved, kinds) = resolve_content(&content, &res).unwrap();
    assert_eq!(kinds.len(), 1);

    // 旧收集面：页级只有 Fm0——form 内部引用不可见（Lead 反例）
    let page_only = CollectedResources::from_content(&content);
    assert!(page_only.extgstate.is_empty());
    assert!(page_only.colorspace.is_empty());
    assert!(page_only.font.is_empty());

    // #181 修复面：from_content_with_forms 收 form 内部引用
    let with_forms = CollectedResources::from_content_with_forms(&content, &kinds);
    assert!(
        with_forms.extgstate.iter().any(|n| n == b"GS0"),
        "form 内部 gs 引用必须收集：{:?}",
        with_forms.extgstate
    );
    assert!(
        with_forms.colorspace.iter().any(|n| n == b"CS0"),
        "form 内部 cs 引用必须收集：{:?}",
        with_forms.colorspace
    );
    assert!(
        with_forms.font.iter().any(|n| n == b"F0"),
        "form 内部 Tf 引用必须收集：{:?}",
        with_forms.font
    );
    // Do 名字（Fm0）双收（页级 + form 收集入口的页级遍历）
    assert!(with_forms.xobject.iter().any(|n| n == b"Fm0"));
}

/// #180 集成面：链式解码预算（image 数据面入口）。
///
/// Lead 反例形态：11 字节输入三层链膨胀 47,662 倍。等价最小炸弹：
/// A85 z 组（4 倍/字节）+ 小预算 → 拒绝。
#[test]
fn image_chain_budget_rejects_bomb() {
    let mut attrs = vec![
        (b"W".to_vec(), Operand::Int(4)),
        (b"H".to_vec(), Operand::Int(4)),
        (b"BPC".to_vec(), Operand::Int(8)),
        (b"CS".to_vec(), Operand::Name(b"DeviceGray".to_vec())),
        (b"F".to_vec(), Operand::Name(b"A85".to_vec())),
    ];
    // z×5000 + EOD：5002 输入 → 20000 输出；几何预算 4×4×8bpc/8+1 = 17
    let data: Vec<u8> = vec![b'z'; 5000];
    let mut enc = data;
    enc.extend_from_slice(b"~>");
    let img = PdeImage {
        gstate: GState::default(),
        attrs: std::mem::take(&mut attrs),
        data: enc,
    };
    let r = decode_image_data(&img);
    assert!(r.is_err(), "z 组炸弹必须被几何预算拒绝（17 字节上限）");
    let msg = format!("{:?}", r.unwrap_err());
    assert!(msg.contains("超预算"), "错误信息带预算语义：{msg}");
}
