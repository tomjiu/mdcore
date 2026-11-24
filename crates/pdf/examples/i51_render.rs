//! D1 归因实验工具（#51 工作线 D）——单页 BGRA 渲染落盘（原始字节 + 元数据
//! 打印到 stdout）。供 `scripts/display_corpus/d1_attribution.py` 做 poppler
//! 25.03 / 26.09 / PDFium(Linux) / PDFium(Windows 附件) 四端点两两网格对照。
//!
//! 例程仅依赖公开 API（`PdfiumEngine::open` + `Rasterizer::rasterize`）；
//! 产物为 `.bgra` 原始缓冲（w×h×4）——灰度化/网格度量在 Python 侧单点
//! 复刻（PIL L24 公式），度量的数值口径与 `mang-shell/src/pdf_display_verify.rs`
//! 逐位对齐（该测试的 `pil_luma` / `grids` / `fit_height_to_reference` 同式）。
//!
//! 用法：`cargo run -p mdc-pdf --features engine-lopdf,engine-pdfium --example
//! i51_render -- <pdf> <page_0based> <out.bgra>`
#![cfg(feature = "engine-pdfium")]

use mdc_pdf::engine::DocumentEngine;
use mdc_pdf::engine_pdfium::PdfiumEngine;
use mdc_pdf::page::PageIndex;
use mdc_pdf::render::{Rasterizer, RenderRequest};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("用法：i51_render <pdf> <page_0based> <out.bgra>");
        std::process::exit(2);
    }
    let pdf = &args[1];
    let page: u32 = args[2].parse().expect("页号必须是 0 基数字");
    let out = &args[3];

    let bytes = std::fs::read(pdf).unwrap_or_else(|e| panic!("读取 {pdf} 失败：{e}"));
    let mut engine = PdfiumEngine::open(bytes).unwrap_or_else(|e| {
        panic!("打开失败（库定位序：env MDC_PDFIUM_LIB → 候选清单 → 系统）：{e:?}")
    });
    let geo = engine
        .page_geometry(PageIndex(page))
        .unwrap_or_else(|e| panic!("几何读取失败：{e:?}"));
    let rendered = engine
        .rasterize(RenderRequest {
            page: PageIndex(page),
            width_px: 640,
        })
        .unwrap_or_else(|e| panic!("渲染失败：{e:?}"));

    // 元数据行（Python 侧解析）：宽 高 有效宽pt 有效高pt 旋转
    println!(
        "{} {} {} {} {}",
        rendered.width_px,
        rendered.height_px,
        geo.effective_width(),
        geo.effective_height(),
        rotation_deg(geo.rotation)
    );
    std::fs::write(out, &rendered.bgra).unwrap_or_else(|e| panic!("写 {out} 失败：{e}"));
}

fn rotation_deg(r: mdc_pdf::page::Rotation) -> u32 {
    match r {
        mdc_pdf::page::Rotation::Deg0 => 0,
        mdc_pdf::page::Rotation::Deg90 => 90,
        mdc_pdf::page::Rotation::Deg180 => 180,
        mdc_pdf::page::Rotation::Deg270 => 270,
    }
}
