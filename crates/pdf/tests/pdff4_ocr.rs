//! PDF-f4 — OCR 接入语义锁（feature `engine-pdfium` + tesseract 环境门）。
//!
//! 环境合同（issue 指令「沙盒缺就 SKIP——参照 bid-parse」）：
//! - tesseract 二进制缺失 → 全部测试**跳过并打印**（不失败）；
//! - 语言包：`MDC_OCR_TESSDATA` 注入（本仓 `test-artifacts/ocr-tessdata/`
//!   携带 chi_sim——取件脚本见该目录 README）；chi_sim 缺失 → 中文面
//!   跳过并打印，英文面照常（eng 系统包）；
//! - pdfium 库定位序同 pdfc_engine（MDC_PDFIUM_LIB → 候选 → 系统）。
//!
//! 锁清单：
//! 1. `scanned_mixed_ocr_english`——扫描件（PIL 真字体渲染 → JPEG →
//!    DCTDecode 嵌入，零文本层）→ ocr_page(eng) → 英文关键词条
//!    （"Specification" / "Requirements"）；
//! 2. `scanned_mixed_ocr_chinese_mixed`——中英混合（chi_sim+eng）→
//!    中文短语锚点（「招标文件」/「验收标准」）——issue 验收
//!    「OCR 出文本（中英混合）」的字面证明；
//! 3. `text_pdf_ocr_sanity`——真文本 PDF（small.pdf）经同一管线
//!    OCR——渲染→识别全链健康度（英文词 "Spike"）；
//! 4. `ocr_env_probe_semantics`——探针面（二进制/语言清单/has_lang）；
//! 5. `missing_lang_typed_error`——缺失语言包 → typed Engine 错误
//!    （产品面不静默）；页越界 → InvalidPage；
//! 6. `scanned_ocr_words_bbox_user_space`（#306 ①）——词级识别：词框
//!    在页内（Letter 用户空间）+ 行锚分组连续 + 中文词识别锚点；
//! 7. `scanned_searchable_writeback_dual_engine`（#306 ②）——**主锁**：
//!    识别词 → write_searchable_pdf → **双引擎提取**（pdfium
//!    extract_text + lopdf extract_text）均命中中文锚点——「导出的
//!    PDF 在任何阅读器里都能搜」的仓内双引擎证明（Acrobat 真机面
//!    另归 #270/#554 平台验收，仓内以双引擎 + CI 环境门诚实标注）；
//! 8. `writeback_report_and_zero_word_noop`（#306 ②）——报告面：
//!    零词空操作（产物可解析、报告零注入）+ 报告计数与注入词一致。

// #469:本文件消费特性门后模块——裸 crate 调用编译门(CI 恒带特性,零行为变化)。
// #306 新增锁 6-8 消费 ocr_writeback（engine-lopdf）——逐测试门控
// （文件级门会丢 pdfium-only 配置的既有锁 1-5 覆盖）。
#![cfg(feature = "engine-pdfium")]

use std::path::Path;

use mdc_pdf::command::CommandError;
use mdc_pdf::ocr::{ocr_available, ocr_page, ocr_page_words};
use mdc_pdf::page::PageIndex;

use mdc_pdf::engine::DocumentEngine;
use mdc_pdf::engine_pdfium::PdfiumEngine;
#[cfg(feature = "engine-lopdf")]
use mdc_pdf::ocr_writeback::write_searchable_pdf;
use mdc_pdf::page::PageSpan;
#[cfg(feature = "engine-lopdf")]
use mdc_pdf::writer_lopdf::lopdf;

/// tessdata 注入（本仓携带目录存在时）。
fn setup_tessdata() {
    let local = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-artifacts/ocr-tessdata");
    if std::env::var_os("MDC_OCR_TESSDATA").is_none() && local.is_dir() {
        // 仅当本地目录存在且含 chi_sim（eng/osd 系统自带）。
        if local.join("chi_sim.traineddata").exists() {
            std::env::set_var("MDC_OCR_TESSDATA", local);
        }
    }
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture {name} 读取失败：{e}"))
}

#[test]
fn scanned_mixed_ocr_english() {
    setup_tessdata();
    let env = ocr_available();
    if !env.binary_ok {
        eprintln!("[pdff4] tesseract 缺失 — SKIP（环境门：issue 指令沙盒缺就 SKIP）");
        return;
    }
    if !env.has_lang("eng") {
        eprintln!("[pdff4] eng 语言包缺失 — SKIP");
        return;
    }
    let scanned = fixture_bytes("scanned_mixed.pdf");
    let text = ocr_page(&scanned, 0, "eng").unwrap();
    eprintln!(
        "[pdff4] eng OCR 输出前 3 行：{}",
        text.lines().take(3).collect::<Vec<_>>().join(" ⏎ ")
    );
    // 关键词条（OCR 非确定性——词条级断言非全文相等）。
    let lower = text.to_lowercase();
    assert!(
        lower.contains("specification"),
        "英文锚点 Specification 须识别（得：{text:?}）"
    );
    assert!(
        lower.contains("requirements"),
        "英文锚点 Requirements 须识别"
    );
}

#[test]
fn scanned_mixed_ocr_chinese_mixed() {
    setup_tessdata();
    let env = ocr_available();
    if !env.binary_ok {
        eprintln!("[pdff4] tesseract 缺失 — SKIP");
        return;
    }
    if !env.has_lang("chi_sim") {
        eprintln!(
            "[pdff4] chi_sim 语言包缺失 — SKIP（取件：test-artifacts/ocr-tessdata/README.md）"
        );
        return;
    }
    let scanned = fixture_bytes("scanned_mixed.pdf");
    // 中英混合（issue 验收字面）。
    let text = ocr_page(&scanned, 0, "chi_sim+eng").unwrap();
    eprintln!(
        "[pdff4] 中英混合 OCR 输出前 4 行：{}",
        text.lines().take(4).collect::<Vec<_>>().join(" ⏎ ")
    );
    assert!(
        text.contains("招标文件"),
        "中文锚点「招标文件」须识别（得：{text:?}）"
    );
    assert!(
        text.contains("验收标准") || text.contains("总体要求"),
        "中文锚点（第二章标题）须识别"
    );
    assert!(
        text.to_lowercase().contains("specification"),
        "混合模式下英文锚点须识别"
    );
}

#[test]
fn text_pdf_ocr_sanity() {
    setup_tessdata();
    let env = ocr_available();
    if !env.binary_ok || !env.has_lang("eng") {
        eprintln!("[pdff4] tesseract/eng 缺失 — SKIP");
        return;
    }
    let text = ocr_page(&fixture_bytes("small.pdf"), 0, "eng").unwrap();
    eprintln!(
        "[pdff4] small.pdf OCR：{:?}",
        text.lines().take(2).collect::<Vec<_>>()
    );
    assert!(
        text.to_lowercase().contains("spike"),
        "small.pdf 页 0 的 'Spike' 须识别——渲染→识别全链健康度"
    );
}

#[test]
fn ocr_env_probe_semantics() {
    setup_tessdata();
    let env = ocr_available();
    eprintln!(
        "[pdff4] OCR 环境：binary={} langs={:?}",
        env.binary_ok, env.languages
    );
    if !env.binary_ok {
        eprintln!("[pdff4] tesseract 缺失 — 探针语义按缺席面断言");
        assert!(env.languages.is_empty());
        assert!(!env.has_lang("eng"));
        return;
    }
    // 沙盒实测面：eng 系统包恒在（测试环境合同）；chi_sim 经注入目录。
    assert!(env.has_lang("eng"));
    assert!(!env.has_lang("nonexistent_lang"));
}

#[test]
fn missing_lang_typed_error() {
    setup_tessdata();
    let env = ocr_available();
    if !env.binary_ok {
        eprintln!("[pdff4] tesseract 缺失 — SKIP");
        return;
    }
    let scanned = fixture_bytes("scanned_mixed.pdf");
    // 缺失语言包 → tesseract 退出非零 → typed Engine（产品面不静默）。
    match ocr_page(&scanned, 0, "klingon_fake") {
        Err(CommandError::Engine(msg)) => {
            assert!(
                msg.contains("识别失败") || msg.contains("tesseract"),
                "{msg}"
            )
        }
        other => panic!("缺失语言包应 Engine 错误，得 {other:?}"),
    }
    // 页越界 → InvalidPage。
    match ocr_page(&scanned, 99, "eng") {
        Err(CommandError::InvalidPage(PageIndex(99))) => {}
        other => panic!("页越界应 InvalidPage(99)，得 {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// #306：词级识别 + 可搜索写回（环境门同锁 1-5）
// ---------------------------------------------------------------------------

/// 锁 6（#306 ①）：词级识别——词框在 Letter 页内（用户空间）+ 行锚
/// 分组在位 + 中文词识别锚点（识别非确定性——词条级断言）。
#[test]
fn scanned_ocr_words_bbox_user_space() {
    setup_tessdata();
    let env = ocr_available();
    if !env.binary_ok || !env.has_lang("chi_sim") {
        eprintln!("[pdff4] tesseract/chi_sim 缺失 — SKIP（锁 6）");
        return;
    }
    let scanned = fixture_bytes("scanned_mixed.pdf");
    let words = ocr_page_words(&scanned, 0, "chi_sim+eng").unwrap();
    eprintln!("[pdff4] 词级识别：{} 词", words.len());
    assert!(!words.is_empty(), "扫描件词级识别应产出词");
    // 词框在 Letter 页内（612×792 pt——用户空间换算的几何面锁）。
    for w in &words {
        assert!(w.bbox[0] >= -1.0 && w.bbox[2] <= 613.0, "x 界内：{:?}", w);
        assert!(w.bbox[1] >= -1.0 && w.bbox[3] <= 793.0, "y 界内：{:?}", w);
        assert!(
            w.bbox[0] < w.bbox[2] && w.bbox[1] < w.bbox[3],
            "盒有序：{:?}",
            w
        );
    }
    // 行锚分组在位（首词行 = (1,1,1)——tesseract 结构序）。
    assert_eq!(words[0].line, (1, 1, 1), "行锚透传（首行）");
    // 中文词识别锚点（词条级——「招标」「文件」标题词）。
    let joined: String = words
        .iter()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join("");
    assert!(
        joined.contains("招标"),
        "中文词锚点「招标」（得：{joined:?}）"
    );
    assert!(
        joined.contains("文件") || joined.contains("技术"),
        "中文词锚点（文件/技术）"
    );
    assert!(
        words.iter().any(|w| w.text.contains("Specification")),
        "英文词锚点 Specification"
    );
}

/// 锁 7（#306 ②·主锁）：识别 → 写回 → **双引擎提取**命中——pdfium
/// 与 lopdf 两独立解析面均从产物读到中文锚点（「任何阅读器可搜」的
/// 仓内最强证明；Acrobat 真机面归 #270/#554 平台验收）。
#[test]
fn scanned_searchable_writeback_dual_engine() {
    setup_tessdata();
    let env = ocr_available();
    if !env.binary_ok || !env.has_lang("chi_sim") {
        eprintln!("[pdff4] tesseract/chi_sim 缺失 — SKIP（锁 7）");
        return;
    }
    let scanned = fixture_bytes("scanned_mixed.pdf");
    // 原档零文本层（扫描件前提——写回前基线）。
    let baseline = PdfiumEngine::open(scanned.clone()).unwrap();
    let span = PageSpan::new(PageIndex(0), PageIndex(0)).unwrap();
    let (_, raw_text) = baseline.extract_text(span).unwrap().pop().unwrap();
    assert!(
        raw_text.trim().is_empty(),
        "基线：原档零文本层（得：{raw_text:?}）"
    );

    // 识别 + 写回。
    let words = ocr_page_words(&scanned, 0, "chi_sim+eng").unwrap();
    let (out, report) = write_searchable_pdf(&scanned, &[words]).unwrap();
    eprintln!(
        "[pdff4] 写回：{}→{} 字节，注入 {} 词",
        scanned.len(),
        out.len(),
        report.words_written
    );
    assert!(
        report.words_written >= 10,
        "注入词量级（得 {}）",
        report.words_written
    );

    // 引擎一：pdfium（产品渲染/搜索同源引擎）。
    let reopened = PdfiumEngine::open(out.clone()).unwrap();
    let span = PageSpan::new(PageIndex(0), PageIndex(0)).unwrap();
    let (_, text) = reopened.extract_text(span).unwrap().pop().unwrap();
    eprintln!(
        "[pdff4] pdfium 提取 {} 字符：{:?}",
        text.chars().count(),
        text.chars().take(30).collect::<String>()
    );
    assert!(
        text.contains("招标"),
        "pdfium 提取命中「招标」（得：{text:?}）"
    );
    assert!(
        text.contains("Specification"),
        "pdfium 提取命中 Specification"
    );

    // 引擎二：lopdf（独立解析面——ToUnicode 直射路径）。
    let doc = lopdf::Document::load_mem(&out).unwrap();
    let text2 = doc.extract_text(&[1]).unwrap();
    assert!(
        text2.contains("招标"),
        "lopdf 提取命中「招标」（得：{text2:?}）"
    );
}

/// 锁 8（#306 ②）：报告面——零词空操作（产物可解析 + 报告零注入）；
/// 报告计数与注入词一致（written = 词数 - 跳过）。
#[cfg(feature = "engine-lopdf")]
#[test]
fn writeback_report_and_zero_word_noop() {
    setup_tessdata();
    let scanned = fixture_bytes("scanned_mixed.pdf");
    // 零词空操作。
    let (out, report) = write_searchable_pdf(&scanned, &[Vec::new()]).unwrap();
    assert_eq!(report.words_written, 0, "零词零注入");
    assert!(
        lopdf::Document::load_mem(&out).is_ok(),
        "空操作产物可解析（自检内置）"
    );
    // 报告计数一致（手造词：1 正常 + 1 空词）。
    use mdc_pdf::OcrWord;
    let words = vec![
        OcrWord {
            text: "验收".into(),
            bbox: [50.0, 700.0, 90.0, 730.0],
            line: (1, 1, 1),
        },
        OcrWord {
            text: "  ".into(),
            bbox: [50.0, 600.0, 90.0, 630.0],
            line: (1, 1, 2),
        },
    ];
    let (_, report) = write_searchable_pdf(&scanned, &[words]).unwrap();
    assert_eq!(report.words_written, 1);
    assert_eq!(report.words_skipped, 1);
}
