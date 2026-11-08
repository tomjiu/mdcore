//! PDF-p (#411) —— DocMDP（文档修改检测策略）三写面接线验收。
//!
//! 语义基线：ISO 32000-2 §12.8.2.2（P=1 全拒 / P=2 表单填写+签署 /
//! P=3 另加标注增删改；页内容重写任何 P 级都不放）。检测面纯函数
//! 矩阵（含 fail-closed 与间接引用形态）在 `src/docmdp.rs` 单测——
//! 本文件锁**接线面**：每张写脸在真实夹具上按 P 纳入/拒写。
//!
//! 夹具纪律（与 pdfh_stamp.rs #408 同款）：small.pdf /
//! form_bid_letter.pdf 无 DocMDP 基座，测试内 lopdf 注入
//! `/Perms /DocMDP` 逐级构造（AC-EXP-03 样张在研究仓不在本仓——
//! 构造形态与 spec §12.8.2.2 的变换参数结构逐字段一致，如实登记）。
//!
//! **环境合同**：仅需 feature `engine-lopdf`（无 pdfium 依赖——
//! 三写面全是 lopdf 面）。

// #469:本文件消费特性门后模块——裸 crate 调用编译门(CI 恒带特性,零行为变化)。
#![cfg(feature = "engine-lopdf")]
#![cfg(feature = "engine-lopdf")]

use std::path::PathBuf;

use mdc_pdf::annotation::{AnnotId, Annotation, AnnotationKind, Origin, Point, Quad, Rgb};
use mdc_pdf::command::CommandError;
use mdc_pdf::docmdp::{DocMdpLevel, WriteOp};
use mdc_pdf::forms::{FormError, PdfForm};
use mdc_pdf::page::PageIndex;
use mdc_pdf::stamp::{stamp_image, StampError, StampOptions};
use mdc_pdf::writer::AnnotationWriter;
use mdc_pdf::writer_lopdf::LopdfWriter;

use lopdf::{dictionary, Object};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn small_pdf() -> Vec<u8> {
    std::fs::read(fixtures().join("small.pdf")).expect("small.pdf 缺失")
}

fn form_pdf() -> Vec<u8> {
    std::fs::read(fixtures().join("form_bid_letter.pdf")).expect("form_bid_letter.pdf 缺失")
}

/// 注入 DocMDP 认证签名标记（spec §12.8.2.2 变换参数结构）。
fn with_doc_mdp(bytes: &[u8], p: i64) -> Vec<u8> {
    let mut doc = lopdf::Document::load_mem(bytes).expect("夹具应可装载");
    doc.catalog_mut().unwrap().set(
        "Perms",
        dictionary! {
            "DocMDP" => dictionary! {
                "Type" => "Sig",
                "Reference" => Object::Array(vec![Object::Dictionary(
                    dictionary! {
                        "Type" => "SigRef",
                        "DigestMethod" => "SHA256",
                        "TransformParams" => dictionary! { "P" => p, "V" => "R" },
                    },
                )]),
            },
        },
    );
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("注入后应可保存");
    out
}

/// 最小合法标注（高亮——writer 面无需 pdfium）。
fn sample_highlight() -> Annotation {
    Annotation {
        id: AnnotId(1),
        page: PageIndex(0),
        kind: AnnotationKind::Highlight {
            quads: vec![Quad {
                points: [
                    Point { x: 10.0, y: 60.0 },
                    Point { x: 100.0, y: 60.0 },
                    Point { x: 10.0, y: 50.0 },
                    Point { x: 100.0, y: 50.0 },
                ],
            }],
            color: Rgb {
                r: 255,
                g: 255,
                b: 0,
            },
        },
        author: String::new(),
        origin: Origin::Human,
        created_ms: 0,
        modified_ms: 0,
    }
}

// ------------------------------------------------------------- writer 面

#[test]
fn writer_open_allowed_but_write_denied_on_docmdp() {
    // 开面（from_bytes）不拒——只读打开 + list 在受限档上合法；
    // 拒的是第一个写手势（write_annotation）。
    for (p, expect_reason, level) in [
        (1, "docmdp-p1-no-changes", DocMdpLevel::P1),
        (2, "docmdp-p2-annotation-denied", DocMdpLevel::P2),
    ] {
        let pdf = with_doc_mdp(&small_pdf(), p);
        let mut writer = LopdfWriter::from_bytes(pdf).expect("受限档开面（只读会话装载）应放行");
        let err = writer
            .write_annotation(PageIndex(0), &sample_highlight())
            .expect_err("P={p} 写标注应被拒");
        assert!(
            matches!(&err, CommandError::SecurityDeny { reason } if *reason == expect_reason),
            "P={p} 策略码应冻结为 {expect_reason}，实测 {err:?}"
        );
        // 门先于一切写入：staged 空 + pending 保持 false（零污染）。
        assert!(!matches!(err, CommandError::Engine(_)));
        let _ = level;
    }
}

#[test]
fn writer_write_allowed_on_docmdp_p3() {
    let pdf = with_doc_mdp(&small_pdf(), 3);
    let mut writer = LopdfWriter::from_bytes(pdf).expect("P=3 开面应放行");
    writer
        .write_annotation(PageIndex(0), &sample_highlight())
        .expect("P=3 标注增删改是放行面（spec §12.8.2.2）");
    let out = writer.into_bytes();
    assert!(!out.is_empty());
}

#[test]
fn writer_write_allowed_without_docmdp() {
    // 控制组：无策略档语义不变（门是增量防线，不是存量负担）。
    let mut writer = LopdfWriter::from_bytes(small_pdf()).unwrap();
    writer
        .write_annotation(PageIndex(0), &sample_highlight())
        .expect("无 DocMDP 档写标注不受影响");
}

#[test]
fn writer_flush_on_restricted_doc_is_zero_append() {
    // 被拒写手势后的保存：零脏幂等（bytes_appended = 0）——门拒了
    // 写入，产物与原字节逐位一致（无半成品状态）。
    let pdf = with_doc_mdp(&small_pdf(), 1);
    let mut writer = LopdfWriter::from_bytes(pdf).unwrap();
    let _ = writer
        .write_annotation(PageIndex(0), &sample_highlight())
        .is_err();
    let save = writer.save_incremental().expect("零脏 flush 应幂等成功");
    assert_eq!(save.bytes_appended, 0, "拒写后产物必须零追加");
}

// ------------------------------------------------------------- forms 面

#[test]
fn forms_fill_denied_on_p1_allowed_on_p2() {
    // P=1：填写写回被拒（typed DocMdp——零脏早退之后才拦，真实写回面）。
    let pdf = with_doc_mdp(&form_pdf(), 1);
    let mut form = PdfForm::read(&pdf).expect("P=1 读表单应放行（读非违约）");
    form.set_field("project_name", "智慧园区弱电工程").unwrap();
    let err = form.save().expect_err("P=1 填写写回应被拒");
    assert!(
        matches!(&err, FormError::DocMdp(v) if v.level() == DocMdpLevel::P1
            && v.op() == WriteOp::FormFill),
        "P=1 应拒 FormFill（typed），实测 {err:?}"
    );

    // P=2：表单填写是明文放行面——写回成功且增量前缀保持。
    let pdf = with_doc_mdp(&form_pdf(), 2);
    let mut form = PdfForm::read(&pdf).expect("P=2 读表单应放行");
    form.set_field("project_name", "智慧园区弱电工程").unwrap();
    let out = form.save().expect("P=2 表单填写写回应放行");
    assert!(out.len() > pdf.len(), "增量修订应追加字节");
}

#[test]
fn forms_clean_save_on_p1_is_noop_allowed() {
    // 零脏 save（没改任何域）在 P=1 档放行——无写即无违约（幂等早退
    // 先于门）。
    let pdf = with_doc_mdp(&form_pdf(), 1);
    let form = PdfForm::read(&pdf).unwrap();
    let out = form.save().expect("零脏 save 不触发 DocMDP 门");
    assert_eq!(out, pdf, "零脏 save 返回原字节");
}

#[test]
fn forms_flatten_denied_on_any_docmdp_level() {
    // 扁平化 = 页内容重写 + widget 删除——超出「表单填写」放行面，
    // 任何 P 级都拒（含 P=3）。
    for p in [1, 2, 3] {
        let pdf = with_doc_mdp(&form_pdf(), p);
        let form = PdfForm::read(&pdf).unwrap();
        let err = form.flatten().expect_err("P={p} 扁平化应被拒");
        assert!(
            matches!(&err, FormError::DocMdp(v) if v.op() == WriteOp::ContentRewrite),
            "P={p} flatten 应按 ContentRewrite 拒，实测 {err:?}"
        );
    }
    // 控制组：无策略档扁平化不受影响（form_bid_letter 无已签域，
    // ASCII 值免字体——应真实产出）。
    let form = PdfForm::read(&form_pdf()).unwrap();
    let out = form.flatten().expect("无 DocMDP 档 flatten 不受影响");
    assert!(!out.is_empty());
}

// ------------------------------------------------------------- stamp 面

#[test]
fn stamp_denied_on_docmdp_all_levels_with_typed_error() {
    let seal = std::fs::read(fixtures().join("stamp_seal.png")).expect("stamp_seal.png 缺失");
    for (p, level) in [
        (1, DocMdpLevel::P1),
        (2, DocMdpLevel::P2),
        (3, DocMdpLevel::P3),
    ] {
        let pdf = with_doc_mdp(&small_pdf(), p);
        let err = stamp_image(
            pdf,
            0,
            [50.0, 50.0, 150.0, 150.0],
            &seal,
            StampOptions::default(),
        )
        .expect_err("P={p} 盖章应被拒（页内容重写）");
        assert_eq!(
            err,
            StampError::DocMdpRestricted(level),
            "P={p} 错误归因到 P 级"
        );
        assert!(
            err.to_string().contains("DocMDP"),
            "错误文案可指向 UI 提示：{err}"
        );
    }
}

#[test]
fn stamp_docmdp_allow_break_contract_preserved() {
    // #408 契约不回退：allow_break_signature = true 时 DocMDP 档放行
    // （该开关即「违反签名/策略纪律」的显式知情确认）。
    let seal = std::fs::read(fixtures().join("stamp_seal.png")).expect("stamp_seal.png 缺失");
    let pdf = with_doc_mdp(&small_pdf(), 2);
    let opts = StampOptions {
        allow_break_signature: true,
        ..Default::default()
    };
    let out = stamp_image(pdf.clone(), 0, [50.0, 50.0, 150.0, 150.0], &seal, opts)
        .expect("显式放行应可盖（#408 契约）");
    assert!(out.len() > pdf.len(), "产物应含新增图对象");
}

#[test]
fn stamp_plain_signed_doc_still_signed_document_error() {
    // 普通（非 DocMDP）签名档走 #408 原路：SignedDocument——两形态
    // 错误可区分（SigFlags 注入构造，DocMDP 缺席）。
    let seal = std::fs::read(fixtures().join("stamp_seal.png")).expect("stamp_seal.png 缺失");
    let mut doc = lopdf::Document::load_mem(&small_pdf()).unwrap();
    doc.catalog_mut()
        .unwrap()
        .set("AcroForm", dictionary! { "SigFlags" => 3 });
    let mut pdf = Vec::new();
    doc.save_to(&mut pdf).unwrap();
    let err = stamp_image(
        pdf,
        0,
        [50.0, 50.0, 150.0, 150.0],
        &seal,
        StampOptions::default(),
    )
    .expect_err("普通签名档盖章应拒");
    assert_eq!(
        err,
        StampError::SignedDocument,
        "非 DocMDP 签名档保持原错误面"
    );
}
