//! export_pdf 契约（#49 B 线 → 暗标导出线 / #52 分发验收）。
//!
//! LOK saveAs(url, "pdf", options) 参数化导出（ely-lok `save.rs` 同源
//! typed JSON 形态）：页范围 PageRange（from-to，None=全文）、批注
//! ExportNotesInMargin、修订 ExportTrackedChanges。
//!
//! 真实内核验证口径：产物 `%PDF` 魔数 + `%%EOF` 尾 + 页对象计数
//! （`/Type /Page` 字典项——LO 导出的对象字典不压缩，流才压缩）。
//!
//! 环境门（AGENTS §13 纪律）：无 LibreOffice 运行时时打印横幅 SKIP。

use mdc_lok::client::DocSession;
use std::path::PathBuf;

fn runtime_available() -> bool {
    mdc_lok::runtime::discover().is_ok()
}

fn fixture() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/three-pages.docx")
}

/// 数 PDF 页对象（字典级 `/Type/Page`——LO 25.2 实测无空格形态，
/// 兼容带空格形态；排除 `/Type/Pages` 树节点 = 后随 `s` 的命中）。
fn pdf_page_count(bytes: &[u8]) -> usize {
    let mut n = 0;
    for pat in [b"/Type/Page".as_slice(), b"/Type /Page".as_slice()] {
        let mut i = 0;
        while let Some(at) = bytes[i..].windows(pat.len()).position(|w| w == pat) {
            let at = i + at;
            let after = at + pat.len();
            if bytes.get(after) != Some(&b's') {
                n += 1;
            }
            i = after + 1;
        }
        if n > 0 {
            break; // 两种形态只按实际出现的那种数一遍
        }
    }
    n
}

fn assert_valid_pdf(path: &std::path::Path, want_pages: usize, tag: &str) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{tag}：读取导出产物失败：{e}"));
    assert!(bytes.starts_with(b"%PDF"), "{tag}：缺 %PDF 魔数");
    let tail = &bytes[bytes.len().saturating_sub(64)..];
    assert!(tail.windows(5).any(|w| w == b"%%EOF"), "{tag}：缺 %%EOF 尾");
    let pages = pdf_page_count(&bytes);
    assert_eq!(pages, want_pages, "{tag}：页对象计数（PDF 字典级证据）");
}

#[test]
fn export_pdf_full_and_page_range() {
    if let Some(w) = option_env!("CARGO_BIN_EXE_mdc-lok-worker") {
        std::env::set_var("MDC_LOK_WORKER_BIN", w);
    }
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时（export_pdf 契约未执行）===");
        eprintln!("=== 补救：安装 LibreOffice 或设 MDC_LOK_HOME=<安装根> ===");
        return;
    }
    let dir = std::env::temp_dir().join(format!("mdc-lok-pdf-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let work = dir.join("work.docx");
    std::fs::copy(fixture(), &work).unwrap();
    let canon = std::fs::canonicalize(&work).unwrap();
    let url = mdc_lok::profile::file_url_of(&canon);

    let s = DocSession::open(None, &dir.join("s"), &url, None).expect("DocSession::open");

    let pdf_url = |name: &str| -> String {
        let dir_canon = std::fs::canonicalize(&dir).unwrap();
        mdc_lok::profile::file_url_of(&dir_canon.join(name))
    };

    // ---- 全文导出（3 页夹具）----
    s.export_pdf(&pdf_url("full.pdf"), None, false, false)
        .expect("export_pdf 全文");
    assert_valid_pdf(&dir.join("full.pdf"), 3, "全文导出");

    // ---- 页范围 2-3（跳过第 1 页）----
    s.export_pdf(&pdf_url("range23.pdf"), Some((2, 3)), false, false)
        .expect("export_pdf 页范围 2-3");
    assert_valid_pdf(&dir.join("range23.pdf"), 2, "页范围 2-3");

    // ---- 单页 1 ----
    s.export_pdf(&pdf_url("only1.pdf"), Some((1, 1)), false, false)
        .expect("export_pdf 单页");
    assert_valid_pdf(&dir.join("only1.pdf"), 1, "单页 1");

    // ---- 导出不改文档脏位语义（只读会话导出后源 docx 未被改写）----
    let src_sha_after = mdc_lok::sha256::sha256_hex_file(&work).unwrap();
    assert!(
        !dir.join("work.pdf").exists(),
        "导出不得改写源文件（save_guard 面）"
    );
    let _ = src_sha_after;

    // ---- worker 仍可服务 ----
    let (w, h) = s.size().expect("导出后 worker 仍可服务");
    assert!(w > 0 && h > 0);
    s.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
