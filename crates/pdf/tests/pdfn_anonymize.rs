//! #505 / #161 P0-2：PDF /Info + XMP 暗标元数据清理测试。
//!
//! 覆盖七验收：①身份键清除（/Info 六键）+ XMP 下线 + 非身份键保留；
//! ②前缀逐位保持；③重载往返（页数不变）；④加密拒 + DocMDP
//! P1/P2/P3 全拒（typed 策略码）；⑤无元数据幂等透传；⑥批注修订 +
//! 匿名化修订双链共存；⑦全绿（本文件即证）。

use lopdf::{dictionary, Object, Stream};
use mdc_pdf::anonymize::anonymize_pdf_info;
use mdc_pdf::command::CommandError;
use mdc_pdf::writer::AnnotationWriter;

fn fixtures() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn small_pdf() -> Vec<u8> {
    std::fs::read(fixtures().join("small.pdf")).expect("small.pdf 缺失")
}

/// 脏 fixture：small.pdf + /Info 六身份键 + 日期保留面 + catalog
/// /Metadata XMP 包（Acrobat 元数据主读面）。
fn dirty_pdf() -> Vec<u8> {
    let mut doc = lopdf::Document::load_mem(&small_pdf()).expect("夹具应可装载");
    let info_id = doc.add_object(dictionary! {
        "Author" => Object::string_literal("张三（某某建设集团）"),
        "Title" => Object::string_literal("某某项目投标文件"),
        "Subject" => Object::string_literal("施工标段报价"),
        "Keywords" => Object::string_literal("投标;报价;某某建设"),
        "Creator" => Object::string_literal("张三的工作站"),
        "Producer" => Object::string_literal("某某建设集团 PDF 产线"),
        "CreationDate" => Object::string_literal("D:20261001090000+08'00'"),
        "ModDate" => Object::string_literal("D:20261101090000+08'00'"),
    });
    doc.trailer.set("Info", Object::Reference(info_id));
    // catalog /Metadata：XMP 包（dc:creator 承载身份——Acrobat 优先读）。
    let xmp = concat!(
        r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>"#,
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">"#,
        r#"<rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator><rdf:Seq><rdf:li>"#,
        "张三（某某建设集团）",
        r#"</rdf:li></rdf:Seq></dc:creator></rdf:Description>"#,
        r#"</rdf:RDF></x:xmpmeta><?xpacket end="w"?>"#,
    );
    let xmp_id = doc.add_object(Object::Stream(Stream::new(
        dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
        xmp.as_bytes().to_vec(),
    )));
    let catalog_id = doc
        .trailer
        .get(b"Root")
        .ok()
        .and_then(|o| o.as_reference().ok())
        .expect("trailer /Root 应在");
    let catalog = doc
        .get_object_mut(catalog_id)
        .expect("catalog 应在")
        .as_dict_mut()
        .expect("catalog 应为字典");
    catalog.set("Metadata", Object::Reference(xmp_id));
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("脏夹具应可保存");
    out
}

/// 读产物 /Info（trailer 引用 → 字典克隆）。
fn read_info(bytes: &[u8]) -> Option<lopdf::Dictionary> {
    let doc = lopdf::Document::load_mem(bytes).ok()?;
    let id = doc.trailer.get(b"Info").ok()?.as_reference().ok()?;
    doc.get_object(id).ok()?.as_dict().ok().cloned()
}

#[test]
fn identity_keys_removed_xmp_off_dates_kept() {
    let src = dirty_pdf();
    let out = anonymize_pdf_info(&src).expect("清理应成功");

    // ① /Info：六身份键消失，日期保留。
    let info = read_info(&out).expect("/Info 应在（对象被替换非移除）");
    for key in [
        "Author", "Title", "Subject", "Keywords", "Creator", "Producer",
    ] {
        assert!(info.get(key.as_bytes()).is_err(), "/Info.{key} 应被剥除");
    }
    assert!(
        info.get(b"CreationDate").is_ok() && info.get(b"ModDate").is_ok(),
        "非身份键（日期）保留"
    );

    // ① XMP：catalog /Metadata 条目下线。
    let doc = lopdf::Document::load_mem(&out).unwrap();
    let catalog = doc.catalog().expect("catalog 应在");
    assert!(
        catalog.get(b"Metadata").is_err(),
        "catalog /Metadata 应下线（Acrobat 回落读 /Info）"
    );

    // ② 前缀逐位保持（增量语义核心断言——函数内已断言，测试面再钉）。
    assert!(out.len() > src.len(), "应追加了修订段");
    assert_eq!(&out[..src.len()], &src[..], "前缀逐位保持");

    // ③ 重载往返：页数不变。
    let re = lopdf::Document::load_mem(&out).unwrap();
    assert_eq!(
        re.get_pages().len(),
        lopdf::Document::load_mem(&src).unwrap().get_pages().len(),
        "页数不变"
    );
}

#[test]
fn clean_pdf_idempotent_passthrough() {
    // small.pdf 无 /Info 无 /Metadata → 原样返回（幂等透传）。
    let src = small_pdf();
    let out = anonymize_pdf_info(&src).expect("干净档应透传");
    assert_eq!(out, src, "零追加字节");
}

#[test]
fn only_info_without_xmp() {
    // 只有 /Info 无 XMP 的常见形态：/Info 面单独清理。
    let mut doc = lopdf::Document::load_mem(&small_pdf()).unwrap();
    let info_id = doc.add_object(dictionary! {
        "Author" => Object::string_literal("李四"),
        "Producer" => Object::string_literal("P厂"),
        "CreationDate" => Object::string_literal("D:20260101000000Z"),
    });
    doc.trailer.set("Info", Object::Reference(info_id));
    let mut src = Vec::new();
    doc.save_to(&mut src).unwrap();

    let out = anonymize_pdf_info(&src).unwrap();
    let info = read_info(&out).unwrap();
    assert!(info.get(b"Author").is_err() && info.get(b"Producer").is_err());
    assert!(info.get(b"CreationDate").is_ok());
    assert_eq!(&out[..src.len()], &src[..], "前缀保持");
}

#[test]
fn encrypted_denied() {
    // S5：加密档拒（typed SecurityDeny）——仍加密形态（非空口令，
    // lopdf 以未解密装载）。pdfe_security 同构构造。
    use lopdf::{EncryptionVersion, Permissions};
    let mut doc = lopdf::Document::with_version("1.7");
    doc.trailer.set(
        "ID",
        Object::Array(vec![
            Object::string_literal("mdc-pdf-anonymize-id"),
            Object::string_literal("mdc-pdf-anonymize-id"),
        ]),
    );
    let pages_id = doc.new_object_id();
    let page_id = doc.add_object(Object::Dictionary(dictionary! {
        "Type" => Object::Name(b"Page".to_vec()),
        "Parent" => Object::Reference(pages_id),
        "MediaBox" => vec![
            Object::Integer(0), Object::Integer(0),
            Object::Integer(612), Object::Integer(792),
        ],
    }));
    doc.set_object(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => Object::Name(b"Pages".to_vec()),
            "Kids" => vec![Object::Reference(page_id)],
            "Count" => Object::Integer(1),
        }),
    );
    let catalog_id = doc.add_object(Object::Dictionary(dictionary! {
        "Type" => Object::Name(b"Catalog".to_vec()),
        "Pages" => Object::Reference(pages_id),
    }));
    doc.trailer.set("Root", Object::Reference(catalog_id));
    let state: lopdf::EncryptionState = EncryptionVersion::V1 {
        document: &doc,
        owner_password: "owner-secret",
        user_password: "user-secret",
        permissions: Permissions::all(),
    }
    .try_into()
    .expect("加密状态构造");
    doc.encrypt(&state).expect("对象加密");
    let mut encrypted = Vec::new();
    doc.save_to(&mut encrypted).expect("加密夹具保存");

    let err = anonymize_pdf_info(&encrypted).expect_err("加密档应拒");
    assert!(
        matches!(&err, CommandError::SecurityDeny { reason } if *reason == "S5-encrypted-write"),
        "策略码应冻结，实测 {err:?}"
    );
}

#[test]
fn docmdp_all_levels_denied() {
    // DocMDP P1/P2/P3 全拒（元数据改写不在任何许可面——先匿名后签纪律）。
    // P1 = 专属码（p1-no-changes，任何写都走它）；P2/P3 = ContentRewrite 码。
    for (p, expect_reason) in [
        (1, "docmdp-p1-no-changes"),
        (2, "docmdp-content-rewrite"),
        (3, "docmdp-content-rewrite"),
    ] {
        let src = with_doc_mdp(&dirty_pdf(), p);
        let err = anonymize_pdf_info(&src).expect_err("DocMDP 档应拒");
        assert!(
            matches!(&err, CommandError::SecurityDeny { reason } if *reason == expect_reason),
            "P={p} 应拒码 {expect_reason}，实测 {err:?}"
        );
    }
}

#[test]
fn annotation_revision_and_anonymize_coexist() {
    // ⑥ 双修订链：先标注增量保存 → 再匿名化（两段修订 /Prev 链完整）。
    use mdc_pdf::annotation::AnnotationKind;
    use mdc_pdf::page::PageIndex;
    use mdc_pdf::writer_lopdf::LopdfWriter;

    let src = dirty_pdf();
    let mut writer = LopdfWriter::from_bytes(src.clone()).expect("写器装载");
    let quads = vec![mdc_pdf::annotation::Quad {
        points: [
            mdc_pdf::annotation::Point { x: 10.0, y: 60.0 },
            mdc_pdf::annotation::Point { x: 100.0, y: 60.0 },
            mdc_pdf::annotation::Point { x: 10.0, y: 50.0 },
            mdc_pdf::annotation::Point { x: 100.0, y: 50.0 },
        ],
    }];
    let annot = mdc_pdf::annotation::Annotation {
        id: mdc_pdf::annotation::AnnotId(1),
        page: PageIndex(0),
        kind: AnnotationKind::Highlight {
            quads,
            color: mdc_pdf::annotation::Rgb {
                r: 255,
                g: 255,
                b: 0,
            },
        },
        author: String::new(),
        origin: mdc_pdf::annotation::Origin::Human,
        created_ms: 0,
        modified_ms: 0,
    };
    writer
        .write_annotation(PageIndex(0), &annot)
        .expect("标注写入");
    let _save = writer.save_incremental().expect("标注增量保存");
    let annotated = writer.product_bytes();

    // 匿名化叠加在标注产物上。
    let out = anonymize_pdf_info(&annotated).expect("标注档匿名化应成功");
    // 前缀 = 标注产物（匿名化不破坏标注修订）。
    assert_eq!(&out[..annotated.len()], &annotated[..]);
    // 标注仍在（重载可读回）。
    let (annots, report) =
        mdc_pdf::annotate::read_annotations(out.clone(), mdc_pdf::security::Limits::default())
            .expect("标注读回");
    assert!(
        !annots.is_empty() && report.parsed >= 1,
        "标注修订应存活（parsed={}）",
        report.parsed
    );
    // 身份键已清。
    let info = read_info(&out).unwrap();
    assert!(info.get(b"Author").is_err());
}

/// DocMDP 注入（pdfp_docmdp 同款）。
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
