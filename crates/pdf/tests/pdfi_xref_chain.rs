//! PDF-i — xref 链完整性防线（2026-10-31 行尾污染事故的读写双守卫 +
//! 夹具金丝雀；feature `engine-lopdf`）。
//!
//! # 事故档案（issue #113 评论链，用户实测 5 红的根因）
//!
//! Windows 开发机默认 `core.autocrlf=true`；仓内 `fixtures/small.pdf`
//! 是**零 NUL 字节**的未压缩 PDF，git 文本启发式（前 8000 字节无 NUL
//! 即判文本）将其判为文本 → 检出时 LF→CRLF → 全部字节偏移漂移、xref
//! 条目 EOL 形态破坏。因果链：
//!
//! ```text
//! lopdf 装载退化为全文件扫描重建（xref_start=0，页可读——写路径因此「活」）
//!   → 增量写 new_from_prev 无 /Prev 可指（lopdf 对重建档刻意省略）
//!   → 产物 xref 只覆盖本轮新对象，目录/页树经链不可达
//!   → read_annotations 装载成功但 get_pages()=0 → 静默 parsed:0
//!   → pdff2_annotate 5 测试红：「写入成功但读不回来」
//! ```
//!
//! # 锁清单
//!
//! 1. `fixture_canary_small_pdf_pristine`——夹具金丝雀：small.pdf 检出
//!    逐位未污染（零 CR）+ xref 经正常路径解析（xref_start≠0）+
//!    `.gitattributes` 在位标 PDF binary（根因三件套的环境侧触发器，
//!    任何平台检出污染即刻红并附修复指引）；
//! 2. `writer_rejects_reconstructed_input`——写入口守卫（Guard A）：
//!    CRLF 篡改档（扫描重建装载态）经 `LopdfWriter::from_bytes` /
//!    `PdfAnnotator::open` 必须**响亮拒绝**（Engine），不得静默接受并
//!    产出链断裂产物；同一夹具未篡改形态必须可开（守卫无误伤）；
//! 3. `read_rejects_unreachable_page_tree`——读面零页守卫（Guard B）：
//!    手工构造「装载成功但页树不可达」的链断裂档（镜像事故产物形态：
//!    追加仅覆盖新对象的 xref、trailer 无 /Prev）→ `read_annotations`
//!    必须响亮失败，不得静默返回 parsed:0；
//! 4. `mangled_product_shape_is_exactly_the_incident`——事故形态钉定：
//!    Guard A 拦截的输入若被旧版写器接受，其产物正是锁 3 的链断裂
//!    形态（前缀逐位=篡改输入、trailer 无 /Prev、页树不可达）——
//!    把「为什么入口拒绝是对的」钉成可执行断言；
//! 5. `writer_rejects_offset_corrected_input`——偏移修正档守卫
//!    （#573：lopdf 0.46 `xref_recovery` 两档扩面）：startxref 写错的
//!    档（装载改用邻近节，`xref_start != 0`——旧守卫盲区形态）在
//!    写器/门面/匿名化三入口必须响亮拒绝（决策 B 保守拒绝），
//!    零修复装载不受误伤。
//!
//! 环境合同同 pdff2_annotate：fixtures 仓内随包；测试内**只在内存**
//! 构造篡改形态（盘上夹具永不落污染字节）。

// #469:本文件消费特性门后模块——裸 crate 调用编译门(CI 恒带特性,零行为变化)。
#![cfg(feature = "engine-lopdf")]
#![cfg(feature = "engine-lopdf")]

use std::path::Path;

use mdc_pdf::annotate::{read_annotations, PdfAnnotator};
use mdc_pdf::anonymize::anonymize_pdf_info;
use mdc_pdf::command::CommandError;
use mdc_pdf::security::Limits;
use mdc_pdf::writer_lopdf::{lopdf, LopdfWriter};

use lopdf::dictionary;

fn fixture_bytes(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture {name} 读取失败：{e}"))
}

/// 模拟 Windows autocrlf 检出污染（每个 LF 前插 CR——git 的文本行尾
/// 规范化语义）。
fn crlf_mangled(data: &[u8]) -> Vec<u8> {
    data.iter()
        .flat_map(|&b| {
            if b == b'\n' {
                vec![b'\r', b'\n']
            } else {
                vec![b]
            }
        })
        .collect()
}

/// 镜像事故产物形态：在合法档上追加一个**只覆盖新对象**的 xref，
/// trailer 无 /Prev——前序修订的全部对象经链不可达（目录/页树失联）。
fn append_revision_without_prev(mut pdf: Vec<u8>) -> Vec<u8> {
    let obj_offset = pdf.len();
    pdf.extend_from_slice(b"9 0 obj\n<</Type/Annot/Subtype/Text/Rect[0 0 10 10]>>\nendobj\n");
    let xref_offset = pdf.len();
    pdf.extend_from_slice(
        format!(
            "xref\n9 1\n{obj_offset:010} 00000 n \ntrailer\n<</Size 10/Root 1 0 \
             R>>\nstartxref\n{xref_offset}\n%%EOF\n"
        )
        .as_bytes(),
    );
    pdf
}

/// 构造偏移修正档（#573 锁 5 专用）：把尾部 `startxref` 的信写错
/// `delta` 字节（+4 = 指到 `xref` 关键字后——镜像 lopdf 上游自测
/// `recovers_from_miswritten_startxref_offset` 的真实生成器缺陷形态：
/// 有些生成器把 startxref 指到 xref 关键字的下一行）。装载侧效果：
/// lopdf 在 ±64 字节窗口内找回真实 xref 节——
/// `xref_recovery = Some(OffsetCorrected)` 且 `xref_start != 0`
/// （能读——旧守卫仅查 `xref_start == 0`，此档被放行 = 盲区形态）。
/// 档身其余字节逐位不动（对象偏移/xref 表本体均未改）。
fn miswritten_startxref(data: &[u8], delta: usize) -> Vec<u8> {
    let marker = b"startxref\n";
    let pos = data
        .windows(marker.len())
        .rposition(|w| w == marker)
        .expect("夹具应有 startxref 尾");
    let num_start = pos + marker.len();
    let num_end = data[num_start..]
        .iter()
        .position(|&b| b == b'\n')
        .expect("startxref 后应有换行")
        + num_start;
    let claimed: usize = std::str::from_utf8(&data[num_start..num_end])
        .expect("startxref 值应为 ASCII 数字")
        .parse()
        .expect("startxref 值应可解析");
    let mut out = data.to_vec();
    out.truncate(num_start);
    out.extend_from_slice(format!("{}\n%%EOF\n", claimed + delta).as_bytes());
    out
}

// ---------------------------------------------------------------------------
// 1. 夹具金丝雀（环境侧触发器——污染检出即刻红并附修复指引）
// ---------------------------------------------------------------------------

#[test]
fn fixture_canary_small_pdf_pristine() {
    let bytes = fixture_bytes("small.pdf");
    assert!(
        !bytes.contains(&b'\r'),
        "small.pdf 含 CR——检出被行尾转换污染（Windows git autocrlf 所致，\
         全部字节偏移已漂移）。修复：仓库根执行 `git checkout -f -- \
         crates/pdf/fixtures/small.pdf`（.gitattributes 已标 *.pdf binary，\
         修复后不再复发）"
    );
    let doc = lopdf::Document::load_mem(&bytes).expect("small.pdf 装载（未污染形态）");
    assert_ne!(
        doc.xref_start, 0,
        "small.pdf xref 不可正常解析（lopdf 退化为扫描重建装载）——疑似\
         偏移漂移污染。修复：`git checkout -f -- crates/pdf/fixtures/small.pdf`"
    );
    // 根因修复在位性：.gitattributes 标 PDF binary（缺失 = 防线退化为
    // 仅金丝雀告警）。
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.gitattributes");
    let attrs = std::fs::read_to_string(&repo_root)
        .unwrap_or_else(|e| panic!(".gitattributes 读取失败（应在仓库根）：{e}"));
    assert!(
        attrs.contains("*.pdf") && attrs.contains("binary"),
        ".gitattributes 缺少 *.pdf binary 标记——行尾污染防线不完整"
    );
}

// ---------------------------------------------------------------------------
// 2. 写入口守卫（Guard A：重建装载档响亮拒绝）
// ---------------------------------------------------------------------------

#[test]
fn writer_rejects_reconstructed_input() {
    let pristine = fixture_bytes("small.pdf");
    let mangled = crlf_mangled(&pristine);

    // 前置钉定：篡改档 lopdf 仍可装载且页可读（宽容重建）——这正是
    // 事故的隐蔽性所在（写路径此前「一切正常」）。
    let doc = lopdf::Document::load_mem(&mangled).expect("篡改档可装载（扫描重建）");
    assert_eq!(doc.xref_start, 0, "篡改档应为扫描重建装载态");
    assert_eq!(doc.get_pages().len(), 3, "宽容重建下页仍可读——事故隐蔽面");

    // 写入口必须响亮拒绝（不得静默接受并产出链断裂产物）。
    let err = LopdfWriter::from_bytes(mangled.clone()).expect_err("写入口必须拒绝扫描重建装载档");
    match err {
        CommandError::Engine(msg) => assert!(msg.contains("xref"), "拒绝理由应指向 xref 链：{msg}"),
        other => panic!("应为 Engine 错误，得 {other:?}"),
    }

    // 门面同款（单一咽喉点——PdfAnnotator::open 内部即 from_bytes）。
    let err2 = PdfAnnotator::open(mangled).expect_err("PdfAnnotator::open 必须拒绝扫描重建装载档");
    match err2 {
        CommandError::Engine(msg) => {
            assert!(msg.contains("xref"), "门面拒绝理由应指向 xref 链：{msg}")
        }
        other => panic!("应为 Engine 错误，得 {other:?}"),
    }

    // 守卫无误伤：未污染原件照常可开。
    LopdfWriter::from_bytes(pristine.clone()).expect("未污染原件不受守卫影响");
    PdfAnnotator::open(pristine).expect("未污染原件门面照常可开");
}

// ---------------------------------------------------------------------------
// 3. 读面零页守卫（Guard B：页树不可达响亮失败）
// ---------------------------------------------------------------------------

#[test]
fn read_rejects_unreachable_page_tree() {
    let pristine = fixture_bytes("small.pdf");
    let broken = append_revision_without_prev(pristine);

    // 前置钉定：该形态**装载成功**且零页（静默产毒的隐蔽面——事故中
    // 旧版 read_annotations 对此返回 Ok(parsed:0)）。
    let doc = lopdf::Document::load_mem(&broken).expect("链断裂档可装载（xref 自身有效）");
    assert!(
        doc.get_pages().is_empty(),
        "链断裂档页树应不可达（/Root 经 xref 无从解析）"
    );

    // 读面必须响亮失败。
    let err = read_annotations(broken, Limits::default())
        .expect_err("read_annotations 必须拒绝页树不可达档");
    match err {
        CommandError::Engine(msg) => assert!(
            msg.contains("页树不可达"),
            "拒绝理由应指明页树不可达：{msg}"
        ),
        other => panic!("应为 Engine 错误，得 {other:?}"),
    }

    // 对照：合法原件读面照常（零标注 = Ok + parsed:0 + 页在）。
    let (annots, report) = read_annotations(fixture_bytes("small.pdf"), Limits::default()).unwrap();
    assert_eq!(report.parsed, 0, "合法档零标注是正常态");
    assert!(annots.is_empty());
}

// ---------------------------------------------------------------------------
// 5. 偏移修正档守卫（#573：lopdf 0.46 `xref_recovery` 两档扩面，决策 B）
// ---------------------------------------------------------------------------

#[test]
fn writer_rejects_offset_corrected_input() {
    let pristine = fixture_bytes("small.pdf");
    let miswritten = miswritten_startxref(&pristine, 4);

    // 前置钉定（盲区形态）：装载改用邻近节成功且页可读、
    // xref_start != 0——旧守卫（仅查 xref_start == 0）放行此档。
    let doc = lopdf::Document::load_mem(&miswritten)
        .expect("偏移修正档可装载（宽容修正——这正是盲区的隐蔽性）");
    assert_eq!(
        doc.xref_recovery,
        Some(lopdf::xref::XrefRecovery::OffsetCorrected),
        "构造面应为偏移修正档（startxref 写错 +4，邻近节找回）"
    );
    assert_ne!(
        doc.xref_start, 0,
        "偏移修正档 xref_start 非零——旧守卫盲区形态的关键特征"
    );
    assert_eq!(doc.get_pages().len(), 3, "宽容修正下页仍可读——隐蔽面");

    // 写入口必须响亮拒绝（决策 B：装载期修复动作 = 不可信——不押注
    // 「修正后的偏移恰好正确」）。
    let err = LopdfWriter::from_bytes(miswritten.clone())
        .expect_err("写入口必须拒绝偏移修正档（#573 盲区收口）");
    match err {
        CommandError::Engine(msg) => {
            assert!(msg.contains("xref"), "拒绝理由应指向 xref 链：{msg}");
            assert!(
                msg.contains("OffsetCorrected"),
                "拒绝理由应名档位（偏移修正）：{msg}"
            );
        }
        other => panic!("应为 Engine 错误，得 {other:?}"),
    }

    // 门面同款（PdfAnnotator::open 内部即 from_bytes——单一咽喉点）。
    let err2 =
        PdfAnnotator::open(miswritten.clone()).expect_err("PdfAnnotator::open 必须拒绝偏移修正档");
    match err2 {
        CommandError::Engine(msg) => {
            assert!(
                msg.contains("OffsetCorrected"),
                "门面拒绝理由应名档位：{msg}"
            )
        }
        other => panic!("应为 Engine 错误，得 {other:?}"),
    }

    // 匿名化面同款（三写面同一纪律——共享判定单点）。
    let err3 = anonymize_pdf_info(&miswritten).expect_err("anonymize_pdf_info 必须拒绝偏移修正档");
    match err3 {
        CommandError::Engine(msg) => {
            assert!(
                msg.contains("OffsetCorrected"),
                "匿名化拒绝理由应名档位：{msg}"
            )
        }
        other => panic!("应为 Engine 错误，得 {other:?}"),
    }

    // 守卫无误伤：零修复装载（xref_recovery = None）照常可开。
    let clean = lopdf::Document::load_mem(&pristine).unwrap();
    assert_eq!(
        clean.xref_recovery, None,
        "未篡改原件应为零修复装载（#573 守卫的故行面基线）"
    );
    LopdfWriter::from_bytes(pristine.clone()).expect("零修复装载不受守卫影响");
    PdfAnnotator::open(pristine.clone()).expect("零修复装载门面照常可开");
    anonymize_pdf_info(&pristine).expect("零修复装载匿名化面照常可开");
}

// ---------------------------------------------------------------------------
// 4. 事故形态钉定（Guard A 拦截的输入 × 旧版行为 = 锁 3 的链断裂产物）
// ---------------------------------------------------------------------------

#[test]
fn mangled_product_shape_is_exactly_the_incident() {
    let pristine = fixture_bytes("small.pdf");
    let mangled = crlf_mangled(&pristine);

    // Guard A 生效后本路径不可达；此处绕过产品入口，直接用 lopdf
    // 原语复刻旧版写器在篡改档上的增量保存（对象 + 页挂载 + 保存），
    // 钉定「若不拦截，产物即链断裂形态」——入口拒绝的正当性证明。
    let doc = lopdf::Document::load_mem(&mangled).unwrap();
    let mut inc = lopdf::IncrementalDocument::create_from(mangled.clone(), doc);
    let page_id = *inc
        .get_prev_documents()
        .get_pages()
        .get(&1)
        .expect("重建装载下页对象可达（写面此前正常的原因）");
    inc.opt_clone_object_to_new_document(page_id).unwrap();
    let annot_id = inc
        .new_document
        .add_object(lopdf::Object::Dictionary(dictionary! {
            "Type" => lopdf::Object::Name(b"Annot".to_vec()),
            "Subtype" => lopdf::Object::Name(b"Text".to_vec()),
            "Rect" => lopdf::Object::Array(vec![
                lopdf::Object::Integer(0),
                lopdf::Object::Integer(0),
                lopdf::Object::Integer(10),
                lopdf::Object::Integer(10),
            ]),
        }));
    // 页 /Annots 挂载（Direct 形态直改——与产品 mount_on_page 同义）。
    let page = inc
        .new_document
        .get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    page.set(
        "Annots",
        lopdf::Object::Array(vec![lopdf::Object::Reference(annot_id)]),
    );
    let mut out = Vec::new();
    inc.save_to(&mut out).unwrap();

    // 产物 = 事故形态：前缀逐位 = 篡改输入 + 追加修订无 /Prev + 页树不可达。
    assert!(out.starts_with(&mangled), "前缀逐位保持（篡改输入）");
    let tail = String::from_utf8_lossy(&out[out.len() - 300..]);
    assert!(
        !tail.contains("/Prev"),
        "产物 trailer 应无 /Prev（重建装载态无从回指——事故核心）"
    );
    let reloaded = lopdf::Document::load_mem(&out).unwrap();
    assert!(
        reloaded.get_pages().is_empty(),
        "产物页树不可达——正是锁 3 拒绝的形态（入口拦截的正当性）"
    );
}
