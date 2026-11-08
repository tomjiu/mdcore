//! #480（#161 P1-4 缺口）：合成敌意输入健壮性横切面——lopdf 侧三入口。
//!
//! 与 `pdf_corpus.rs` 的分工：那边是**真实语料** broken/ 面（环境门控，
//! CI 缺语料即 SKIP）；本文件是**合成形态**（空/截断/坏 xref/悬空引用/
//! 巨数值——零外部依赖，CI 恒跑）。
//!
//! 断言口径（pdf_corpus 第 3 条纪律同款）：
//! - typed `Err` 或**有界** `Ok`（页数/标注数有限值，不吞 0xFFFFFFFF）；
//! - **永不 panic**——全输入过 `catch_unwind` 兜底（panic = 测试失败）。
//!
//! 纪律：只加测试不改产线（发现真缺陷 → 开新 issue 报 Lead——规则 17）。

#![cfg(feature = "engine-lopdf")]

use std::panic::{catch_unwind, AssertUnwindSafe};

use mdc_pdf::annotate::read_annotations;
use mdc_pdf::security::Limits;
use mdc_pdf::security_scan::open_scan;
use mdc_pdf::writer_lopdf::LopdfWriter;

/// 最小合法单页 PDF（对象 1=Catalog 2=Pages 3=Page 4=Font 5=流）——
/// 敌意变体全部从它派生（变异点单一可归因）。
fn minimal_pdf() -> Vec<u8> {
    let mut out = Vec::new();
    let mut offsets = [0usize; 5];
    let mut obj = |out: &mut Vec<u8>, num: usize, offsets: &mut [usize; 5], body: &[u8]| {
        offsets[num - 1] = out.len();
        out.extend_from_slice(format!("{} 0 obj\n", num).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    out.extend_from_slice(b"%PDF-1.4\n");
    obj(
        &mut out,
        1,
        &mut offsets,
        b"<< /Type /Catalog /Pages 2 0 R >>",
    );
    obj(
        &mut out,
        2,
        &mut offsets,
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    );
    obj(
        &mut out,
        3,
        &mut offsets,
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R >>",
    );
    obj(
        &mut out,
        4,
        &mut offsets,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    );
    obj(
        &mut out,
        5,
        &mut offsets,
        b"<< /Length 44 >>\nstream\nBT /F1 12 Tf 72 720 Td (hello) Tj ET\nendstream",
    );
    let xref_at = out.len();
    out.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for off in offsets {
        out.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer << /Size 6 /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            xref_at
        )
        .as_bytes(),
    );
    out
}

/// 三入口横切：typed Err / 有界 Ok,永不 panic。
/// 返回 (扫描结果字符串, 写器结果, 标注读结果) 的简码用于断言。
fn sweep(label: &str, input: &[u8]) -> (String, String, String) {
    let limits = Limits::default();

    // catch_unwind 兜底：任一入口 panic = 测试失败（敌意输入永不 panic 合同）。
    let scan = catch_unwind(AssertUnwindSafe(|| match open_scan(input, &limits) {
        Ok(a) => format!("ok(enc={},warn={})", a.encrypted, a.warnings.len()),
        Err(e) => format!("err({:?})", e),
    }))
    .unwrap_or_else(|_| panic!("[{label}] open_scan panic"));
    let writer = catch_unwind(AssertUnwindSafe(|| {
        match LopdfWriter::from_bytes_with_limits(input.to_vec(), limits) {
            Ok(_) => "ok".to_string(),
            Err(e) => format!("err({:?})", e),
        }
    }))
    .unwrap_or_else(|_| panic!("[{label}] from_bytes_with_limits panic"));
    let annots = catch_unwind(AssertUnwindSafe(|| {
        match read_annotations(input.to_vec(), limits) {
            Ok((annots, _report)) => format!("ok({})", annots.len()),
            Err(e) => format!("err({:?})", e),
        }
    }))
    .unwrap_or_else(|_| panic!("[{label}] read_annotations panic"));
    (scan, writer, annots)
}

#[test]
fn empty_and_header_only_inputs_never_panic() {
    // 空 / 1B / 仅 %PDF / 完整头无对象——最浅敌意族。
    for (label, bytes) in [
        ("empty", b"".as_slice()),
        ("one-byte", b"%".as_slice()),
        ("header-only", b"%PDF".as_slice()),
        ("header-newline", b"%PDF-1.4\n".as_slice()),
    ] {
        let (scan, writer, annots) = sweep(label, bytes);
        // 合同:typed 结果(Ok/Err 皆可——形态决定),但永不 panic 已由
        // sweep 内部断言;写器面装载必然失败(无对象)。
        assert!(
            writer.starts_with("err("),
            "[{label}] 无对象输入写器必须 typed 拒绝,got {writer}"
        );
        assert!(!scan.is_empty() && !annots.is_empty(), "[{label}] 结果缺位");
    }
}

#[test]
fn truncated_before_startxref_is_typed_reject() {
    // 截断:去掉尾部 startxref 段(对象体在场,xref 不可达)。
    let full = minimal_pdf();
    let cut = &full[..full.len() * 7 / 10];
    let (scan, writer, annots) = sweep("truncated", cut);
    assert!(
        writer.starts_with("err("),
        "截断档写器必须 typed 拒绝,got {writer}"
    );
    let _ = (scan, annots); // typed 即可(形态决定 Ok/Err)
}

#[test]
fn bogus_xref_offsets_are_typed_reject() {
    // 坏 xref:偏移全部指向文件尾之后(lopdf 重扫或拒——两者皆 typed)。
    let mut pdf = minimal_pdf();
    let pos = pdf.windows(4).position(|w| w == b"xref").expect("xref 段");
    // 把 xref 后第一行条目改成 9999999999(越界偏移)。
    let entry_at = pos + 4 + "\n0 6\n0000000000 65535 f \n".len();
    let bogus = b"9999999999 00000 n \n";
    pdf[entry_at..entry_at + bogus.len()].copy_from_slice(bogus);
    let (scan, writer, annots) = sweep("bogus-xref", &pdf);
    let _ = (scan, annots);
    assert!(
        writer.starts_with("err(") || writer == "ok",
        "坏 xref 必须有界 typed 结果,got {writer}"
    );
}

#[test]
fn dangling_root_ref_is_typed_reject() {
    // 悬空引用:trailer /Root 指向不存在对象(9 0 R)。
    let mut pdf = minimal_pdf();
    let root_at = pdf
        .windows(7)
        .position(|w| w == b"/Root 1")
        .expect("trailer root");
    pdf[root_at..root_at + 7].copy_from_slice(b"/Root 9");
    let (scan, writer, annots) = sweep("dangling-root", &pdf);
    // 合同（纵深防御在案,gates.md Guard A/B/C）：装载面 Ok（xref 有效
    // ——Guard A 不触发）；保存期 Guard C 是第二道闸；本锁锁 Guard B:
    // 无可达页 → 标注读必须 Engine 拒（页树不可达守卫——零页档非合法
    // 形态,拒绝静默返回空标注集）。
    assert!(
        writer == "ok" || writer.starts_with("err("),
        "悬空 /Root 写器必须有界 typed 结果,got {writer}"
    );
    assert!(
        annots.starts_with("err(") && annots.contains("页树不可达"),
        "悬空 /Root 必须命中 Guard B 页树守卫,got {annots}"
    );
    let _ = scan;
}

#[test]
fn huge_declared_count_stays_bounded() {
    // 巨数值:/Count 宣称 0xFFFFFFFF(真实对象只有 1 页)。
    let mut pdf = minimal_pdf();
    let count_at = pdf
        .windows(9)
        .position(|w| w == b"/Count 1 ")
        .expect("pages count");
    pdf.splice(
        count_at..count_at + 9,
        b"/Count 4294967295 ".to_vec(),
    );
    let (scan, writer, annots) = sweep("huge-count", &pdf);
    // 合同:typed 结果 + **有界**——任何 Ok 面的计数不得吞下宣称值
    // (三入口的计数口径 = 实际遍历对象,非声明值;此处锁「不 panic +
    // 结果字符串为有限格式」)。
    assert!(!scan.contains("4294967295"), "扫描面吞下巨 Count:{scan}");
    assert!(!annots.contains("4294967295"), "标注面吞下巨 Count:{annots}");
    let _ = writer;
}

#[test]
fn garbage_after_header_is_typed() {
    // 头后全随机字节(含不可打印与假对象语法碎片)。
    let mut pdf = b"%PDF-1.7\n".to_vec();
    pdf.extend_from_slice(&[0x00, 0xff, 0xfe, 0x89, 0x50, 0x4b, 0x03, 0x04]);
    pdf.extend_from_slice(b"obj endobj xref trailer startxref %%EOF random");
    let (scan, writer, annots) = sweep("garbage", &pdf);
    assert!(
        writer.starts_with("err("),
        "垃圾流写器必须 typed 拒绝,got {writer}"
    );
    let _ = (scan, annots);
}

#[test]
fn deep_nested_dictionary_is_bounded() {
    // 深嵌套字典(1000 层 << >>)——递归解析器的栈压力面
    // (lopdf 自带深度限,此处锁「typed 不 panic」)。
    let depth = 1000usize;
    let mut body = String::from("<< ");
    for _ in 0..depth {
        body.push_str("/K << ");
    }
    body.push('>');
    for _ in 0..depth {
        body.push_str(" >>");
    }
    let mut pdf = minimal_pdf();
    // 注入进 catalog 对象体(替换 /Catalog 键段)。
    let cat_at = pdf.windows(9).position(|w| w == b"/Catalog ").expect("catalog");
    let injected = format!("/Deep {}", body);
    pdf.splice(cat_at..cat_at + 9, injected.into_bytes());
    let (scan, writer, annots) = sweep("deep-nest", &pdf);
    // typed 即可(Ok/Err 由 lopdf 深度限决定)——锁「永不 panic」由 sweep 保证。
    assert!(!scan.is_empty() && !writer.is_empty() && !annots.is_empty());
}
