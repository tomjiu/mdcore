//! #485 web e2e 夹具生成器：带脚注的样例 DOCX → `web/tests/fixtures/
//! import_footnote.docx`（docx_footnote.py 与 mdcore-wasm 原生漂移守卫
//! 测试共用）。产线 = office-io `DocxWriter`（单一真源
//! `build_footnote_sample_docx`，与原生测试同源——零双份漂移）。
//!
//! 用法（仓库根）：`cargo run -p mdcore-wasm --example gen_docx_footnote_fixture`
//!（可选显式路径参数；默认 web/tests/fixtures/import_footnote.docx）。

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "web/tests/fixtures/import_footnote.docx".to_string());
    let bytes = mdcore_wasm::docx_import::build_footnote_sample_docx();
    let path = std::path::Path::new(&out);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .unwrap_or_else(|e| panic!("create fixture dir {}: {e}", dir.display()));
    }
    std::fs::write(path, &bytes).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    println!(
        "fixture written: {} ({} bytes)",
        path.display(),
        bytes.len()
    );
}
