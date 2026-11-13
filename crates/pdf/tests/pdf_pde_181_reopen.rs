//! #181 重开面回归——XObject/Shading **自身** gstate 资源收集。
//!
//! Lead 复验（#185 评论，main d8cd5bd）：原反例
//! `PdeContent::parse(b"/GS0 gs /CS0 cs 0.5 scn /Fm0 Do")` 的 write 仍含
//! `/GS0 gs`、`/CS0 cs`，但 `CollectedResources::from_content` 返回
//! extgstate=[]、colorspace=[]——引用与收集不一致，`merge_collected`
//! 会把正文仍引用的资源删掉（写回即坏）。
//!
//! 根因：`collect_elem` 的 `PdeElement::XObject`/`PdeElement::Shading`
//! 分支只收 name 不走 `collect_gstate`（parser 快照 gstate 进元素，
//! 收集面却丢弃——Text/Path/Image 都收，唯这两支漏）。
//! 既有测试 `collect_resources_roundtrip` 因流含 BT/Tj（Text 分支代收）
//! 而绿灯——遮住真实路径（与 #187 的 read_only 手动设置同型假绿）。
//!
//! 本文件收 Lead 指定的**原最小反例**（不得用重新构造的 Form 内部引用
//! 测试替代——#185 评论原文要求），外加 Shading 同形静态缺口与写回
//! 一致性核对（不只数元素——验收原文）。

use mdc_pdf::pde::model::PdeContent;
use mdc_pdf::pde::resources::{resolve_content, CollectedResources, ResValue, ResourceDict};

fn contains_sub(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn form_stream(content: &[u8]) -> ResValue {
    let dict = vec![
        (b"Type".to_vec(), ResValue::Name(b"XObject".to_vec())),
        (b"Subtype".to_vec(), ResValue::Name(b"Form".to_vec())),
        (b"Length".to_vec(), ResValue::Int(content.len() as i64)),
    ];
    ResValue::Stream {
        dict,
        data: content.to_vec(),
    }
}

/// 原反例（#181 证据 1 原文）：无 BT/无 q——gs/cs 只能附着在 Do 元素
/// 自身 gstate 上。期望集合含 GS0/CS0。
#[test]
fn xobject_own_gstate_refs_collected() {
    let stream = b"/GS0 gs /CS0 cs 0.5 scn /Fm0 Do";
    let content = PdeContent::parse(stream).unwrap();
    let c = CollectedResources::from_content(&content);
    assert_eq!(c.xobject, vec![b"Fm0".to_vec()]);
    assert_eq!(
        c.extgstate,
        vec![b"GS0".to_vec()],
        "原反例（#181 证据1）：Do 元素自身 gstate 的 gs 引用必须收集"
    );
    assert_eq!(
        c.colorspace,
        vec![b"CS0".to_vec()],
        "原反例（#181 证据1）：Do 元素自身 gstate 的 cs 引用必须收集"
    );
}

/// Shading 同形（#181 证据 1「Shading同形静态缺口」）：`sh` 元素自身
/// gstate 的 gs/cs 引用必须收集。
#[test]
fn shading_own_gstate_refs_collected() {
    let stream = b"/GS2 gs /CS2 cs 0.5 scn /S1 sh";
    let content = PdeContent::parse(stream).unwrap();
    let c = CollectedResources::from_content(&content);
    assert_eq!(c.shading, vec![b"S1".to_vec()]);
    assert_eq!(c.extgstate, vec![b"GS2".to_vec()], "sh 元素自身 gs 引用");
    assert_eq!(c.colorspace, vec![b"CS2".to_vec()], "sh 元素自身 cs 引用");
}

/// 写回一致性（验收原文「不只数元素」）：write 仍引用的资源在
/// merge_collected 产物里必须在——正文引用被删 = 写回即坏。
#[test]
fn merge_collected_keeps_own_gstate_refs() {
    let stream = b"/GS0 gs /CS0 cs 0.5 scn /Fm0 Do";
    let content = PdeContent::parse(stream).unwrap();
    // 前置条件：写回正文确实仍引用（若 write 丢了引用那是另一缺陷）
    let w = content.write();
    assert!(contains_sub(&w, b"/GS0 gs"), "write 仍引用 /GS0 gs");
    assert!(contains_sub(&w, b"/CS0 cs"), "write 仍引用 /CS0 cs");

    // 注入表含本体 → 合并产物必须含（收集漏收则 merge 删除仍引用资源）
    let mut res = ResourceDict::new();
    res.extgstate
        .push((b"GS0".to_vec(), ResValue::Dict(vec![])));
    res.colorspace
        .push((b"CS0".to_vec(), ResValue::Dict(vec![])));
    let c = CollectedResources::from_content(&content);
    let merged = res.merge_collected(&c);
    assert!(
        merged.extgstate.iter().any(|(n, _)| n == b"GS0"),
        "merge 不得删除正文仍引用的 ExtGState（收集漏收 → merge 静默删）"
    );
    assert!(
        merged.colorspace.iter().any(|(n, _)| n == b"CS0"),
        "merge 不得删除正文仍引用的 ColorSpace"
    );
}

/// 验收原文「补真实 PDF 写回后独立解析/渲染核对，不只数元素」：
/// 完整编辑链（parse → resolve → collect → merge → write）后，把
/// **写回产物正文**重新独立 parse 并收集——引用面必须与首轮一致
/// （GS0/CS0 双向在位）。这锁的是「写回不因收集面丢引用而坏」的
/// 端到端语义，而不是集合的元素个数。
#[test]
fn writeback_reparse_collect_roundtrip() {
    // 注入表：Fm0 = form（含内容流本体），GS0/CS0 本体在位
    let root = ResValue::Dict(vec![
        (
            b"XObject".to_vec(),
            ResValue::Dict(vec![(b"Fm0".to_vec(), form_stream(b"0 0 100 50 re S"))]),
        ),
        (
            b"ExtGState".to_vec(),
            ResValue::Dict(vec![(
                b"GS0".to_vec(),
                ResValue::Dict(vec![(b"CA".to_vec(), ResValue::Real(0.5))]),
            )]),
        ),
        (
            b"ColorSpace".to_vec(),
            ResValue::Dict(vec![(
                b"CS0".to_vec(),
                ResValue::Array(vec![ResValue::Name(b"ICCBased".to_vec()), ResValue::Int(5)]),
            )]),
        ),
    ]);
    let res = ResourceDict::from_res_value(&root);

    // 首轮：parse → resolve（form 解引用）→ 收集（含 form 面）
    let content = PdeContent::parse(b"/GS0 gs /CS0 cs 0.5 scn /Fm0 Do").unwrap();
    let (resolved, kinds) = resolve_content(&content, &res).unwrap();
    let c1 = CollectedResources::from_content_with_forms(&resolved, &kinds);
    assert!(c1.extgstate.iter().any(|n| n == b"GS0"), "首轮收集含 GS0");
    assert!(c1.colorspace.iter().any(|n| n == b"CS0"), "首轮收集含 CS0");
    assert!(c1.xobject.iter().any(|n| n == b"Fm0"), "首轮收集含 Fm0");

    // 写回正文仍引用（前置——write 不丢引用）
    let w = resolved.write();
    assert!(contains_sub(&w, b"/GS0 gs"));
    assert!(contains_sub(&w, b"/CS0 cs"));
    assert!(contains_sub(&w, b"/Fm0 Do"));

    // merge 产物含全部三本体（收集漏收则 merge 静默删）
    let merged = res.merge_collected(&c1);
    assert!(merged.extgstate.iter().any(|(n, _)| n == b"GS0"));
    assert!(merged.colorspace.iter().any(|(n, _)| n == b"CS0"));
    assert!(merged.xobject.iter().any(|(n, _)| n == b"Fm0"));

    // 独立解析核对：写回正文重新 parse + from_content 收集——
    // Do 元素自身 gstate 的 GS0/CS0 引用必须再次出现在收集面
    // （即本修复的端到端语义：写回产物自洽，无「引用了却不在
    // 资源表」的悬空状态）。
    let re = PdeContent::parse(&w).unwrap();
    let c2 = CollectedResources::from_content(&re);
    assert!(
        c2.extgstate.iter().any(|n| n == b"GS0"),
        "独立解析核对：写回产物再收集必须含 GS0（悬空引用 = 写回即坏）"
    );
    assert!(
        c2.colorspace.iter().any(|n| n == b"CS0"),
        "独立解析核对：写回产物再收集必须含 CS0"
    );
}
