//! #634 PDFium 页面正确显示专项——真实文件对照验收（测试专用模块，
//! lib.rs 以 `#[cfg(test)]` 声明——不进产线编译面，不进 bin 树）。
//!
//! 三层核验（真实 pdfium 全链；缺库环境 SKIP 横幅，不伪装通过）：
//!
//! 1. [`engine_content_matches_poppler_reference`]（纯引擎 `#[test]`）：
//!    15 文件 × 25 探针页矩阵——
//!    - **几何**：引擎 `page_geometry` 的 effective_width/height（含
//!      /Rotate 与可见域——pdfium 口径）对照 manifest 的 **pypdf 独立
//!      解析**值（±0.75pt）；
//!    - **内容**：`RenderWorker`（生产同款看门狗工人）640px 渲染 →
//!      24×24 网格双指标（灰度均值 / 墨迹覆盖率）对照 **poppler 25.03
//!      参照渲染**（`reference_manifest.json` 分页容差 = 本机观测最大
//!      逐格差 ×1.3，下限 0.02/0.04——跨引擎抗锯齿差异在网格级平均化
//!      后的余量；容差校准过程见 scripts/display_corpus/build_corpus.py）；
//!    - **健壮**：坏文件（截断/垃圾）typed 拒绝、报错信息非空，不崩
//!      不挂（pypdf/poppler/pdfium 三方对照行为在 manifest 登记）。
//! 2. [`view_chain_first_screen_scroll_zoom_close_reopen`]（`#[gpui::test]`）：
//!    视图链 open → 首屏位图就绪 → 跳页 60（top_visible_page 读回 +
//!    位图就绪）→ 缩放档（Ctrl+1 实际大小——公开 `dispatch_zoom_key`
//!    真路径，光栅宽 576→640 换档重渲）→ 关闭重开（同字节新会话，
//!    位图再现）。
//! 3. [`robustness_damaged_oversized_multi_tab`]（`#[gpui::test]`）：
//!    坏文件视图 error 态明示（不白屏不 panic）；多视图并存快速切换/
//!    关闭后回来（多会话互不锁死）；超常尺寸页正常渲染（2000×2800pt）。
//!
//! 参照语义要点（语料构建实测的跨引擎差异，已在校准吸收）：
//! - poppler CLI 默认渲染 MediaBox、PDFium 默认 CropBox——参照命令
//!   固化 `-cropbox`；
//! - poppler 对 /Rotate 页默认先缩放后旋转（A4+90° 得 906×640）——
//!   参照命令固化 `-scale-dimension-before-rotation` 得 640×453。
//!
//! 网格度量的数值口径与本仓 `scripts/display_corpus/build_corpus.py`
//! **逐位对齐**：PIL L 灰度（整数 L24 公式）、floor 网格边界、
//! 均值/255（白=1.0）、luma<200 判墨、±2px 高度差中心对齐（高者裁
//! 中段/矮者白补——更大差异是几何级发现，断言失败不吸收）。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mdc_pdf::engine::DocumentEngine;
use mdc_pdf::engine_pdfium::PdfiumEngine;
use mdc_pdf::page::PageIndex;
use mdc_pdf::render::{RenderRequest, RenderedPage};
use mdc_pdf::worker::{RenderWorker, WorkerReply};

use gpui::AppContext;
use ui::prelude::*;

use crate::pdf_pages::PdfPagesView;

/// 网格边数（与 manifest `grid` 一致——24×24）。
const GRID_N: usize = 24;
/// 墨迹阈值（luma < 200 判墨——与 manifest `ink_threshold` 一致）。
const INK_THRESHOLD: u8 = 200;
/// 测试光栅宽（与 manifest `raster_width_px` 一致——poppler 参照同宽）。
const RASTER_W: u32 = 640;

/// ±1px 高度取整口径差（本仓引擎 round vs poppler/pypdfium2 ceil——
/// A4@640px：905 vs 906，显示面不可见：位图拉伸入槽）经线性重采样
/// 对齐后的墨迹阈值跨越余量：亚像素内容位移使部分抗锯齿边缘像素
/// 跨过 luma<200 判墨线（实测 ≤0.004 量级）；内容级错误（缺页/错位/
/// 错内容）的典型逐格差 ≥0.1（6 倍以上隔离带）。良性口径差不修引擎
/// （round 为 crates/pdf 合同测试锁定的合法口径，非显示缺陷）。
const RESAMPLE_INK_ALLOWANCE: f64 = 0.015;

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join("display_corpus")
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    let p = corpus_dir().join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("语料不可读（{}）：{e}", p.display()))
}

fn load_manifest() -> serde_json::Value {
    let p = corpus_dir().join("reference_manifest.json");
    let raw = std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("参照清单不可读（{}）：{e}", p.display()));
    serde_json::from_str(&raw).expect("参照清单必须是合法 JSON")
}

/// 缺库环境判定（引擎/视图错误信息含部署提示片段——与数据格式错误
/// 区分：坏文件的拒绝信息不含 `libpdfium`，不会误判 SKIP）。
fn is_missing_library_error(msg: &str) -> bool {
    msg.contains("libpdfium") || msg.contains("PDF 引擎不可用")
}

fn skip_banner(msg: &str, face: &str) {
    eprintln!(
        "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
         本机无 libpdfium——#634 {face} 未真跑。\n  \
         绑定/打开错误全文：{msg}\n  \
         有 libpdfium 的环境（MDC_PDFIUM_LIB=<库路径> \
         cargo test）全量执行。\n═══════════════════════════\n"
    );
}

// ---------------------------------------------------------------------------
// 网格度量（与 scripts/display_corpus/build_corpus.py 逐位对齐）
// ---------------------------------------------------------------------------

/// PIL `convert("L")` 的 L24 整数公式（亮度权重 299/587/114 + 半值
/// 进位——精确镜像，不是近似）。
#[inline]
fn pil_luma(bgra: &[u8]) -> u8 {
    let (b, g, r) = (bgra[0] as u32, bgra[1] as u32, bgra[2] as u32);
    ((r * 19595 + g * 38470 + b * 7471 + 0x8000) >> 16) as u8
}

/// BGRA 位图 → luma 行阵（行 0 = 渲染页顶部——与参照 row-major 一致）。
fn luma_rows(bgra: &[u8], w: usize, h: usize) -> Vec<Vec<u8>> {
    assert_eq!(bgra.len(), w * h * 4, "BGRA 缓冲长度合同（w×h×4）");
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| {
                    let i = (y * w + x) * 4;
                    pil_luma(&bgra[i..i + 4])
                })
                .collect()
        })
        .collect()
}

/// 高度对齐到参照高（|Δ|≤2px 差异带——垂直线性重采样；更大差异由
/// 调用方先行断言拦截）。为什么不用裁/补：poppler/pypdfium2 的高度
/// 取整为 ceil、本仓引擎为 round（A4@640px：906 vs 905——良性口径差，
/// 显示面位图拉伸入槽无视觉影响）；1px 白补行在高频内容（扫描噪点）
/// 上引入 0.05+ 逐格伪差，线性重采样保持内容连续，仅 ~0.1% 尺度差
/// 进入网格平均（像素中心映射 + 双行线性插值）。
fn fit_height_to_reference(rows: Vec<Vec<u8>>, target_h: usize) -> Vec<Vec<u8>> {
    let h = rows.len();
    if h == 0 || h == target_h {
        return rows;
    }
    let w = rows[0].len();
    let mut out = vec![vec![255u8; w]; target_h];
    for (j, row) in out.iter_mut().enumerate() {
        let sy = (j as f64 + 0.5) * h as f64 / target_h as f64 - 0.5;
        let y0 = sy.floor().max(0.).min((h - 1) as f64) as usize;
        let y1 = (y0 + 1).min(h - 1);
        let frac = (sy - y0 as f64).clamp(0., 1.);
        for (x, cell) in row.iter_mut().enumerate() {
            let v0 = rows[y0][x] as f64;
            let v1 = rows[y1][x] as f64;
            *cell = (v0 + (v1 - v0) * frac).round().clamp(0., 255.) as u8;
        }
    }
    out
}

/// 24×24 网格双指标（floor 边界；mean = 均值/255 白=1.0；ink = luma
/// < 200 像素占比）——与语料构建 `grids_from_gray` 同式。
fn grids(rows: &[Vec<u8>]) -> (Vec<Vec<f64>>, Vec<Vec<f64>>) {
    let h = rows.len();
    let w = rows.first().map_or(0, |r| r.len());
    let xs: Vec<usize> = (0..=GRID_N).map(|i| i * w / GRID_N).collect();
    let ys: Vec<usize> = (0..=GRID_N).map(|j| j * h / GRID_N).collect();
    let mut mean = vec![vec![0f64; GRID_N]; GRID_N];
    let mut ink = vec![vec![0f64; GRID_N]; GRID_N];
    for j in 0..GRID_N {
        for i in 0..GRID_N {
            let (mut sum, mut ink_n, mut n) = (0u64, 0u64, 0u64);
            for y in ys[j]..ys[j + 1] {
                for x in xs[i]..xs[i + 1] {
                    let v = rows[y][x] as u64;
                    sum += v;
                    ink_n += (v < INK_THRESHOLD as u64) as u64;
                    n += 1;
                }
            }
            if n > 0 {
                mean[j][i] = sum as f64 / (n as f64 * 255.);
                ink[j][i] = ink_n as f64 / n as f64;
            }
        }
    }
    (mean, ink)
}

/// 参照 PNG 文件名尾段 `<w>x<h>.png` → 参照高（几何对齐基准）。
fn ref_height(png: &str) -> usize {
    png.strip_suffix(".png")
        .unwrap_or(png)
        .rsplit_once('x')
        .and_then(|(_, h)| h.parse().ok())
        .unwrap_or_else(|| panic!("参照 PNG 名缺尺寸段：{png}"))
}

/// 单作业回执等待（真实渲染线程跑完；30s 限期——看门狗 20s 换代
/// 前必达）。
fn drain_one(
    worker: &mut RenderWorker<PdfiumEngine>,
    job: u64,
    name: &str,
    page: u32,
) -> RenderedPage {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        while let Some(reply) = worker.try_recv() {
            match reply {
                WorkerReply::Done { id, result } => {
                    assert_eq!(id, job, "{name} p{page} 回执序错（单作业串行）");
                    return result.unwrap_or_else(|e| panic!("{name} p{page} 渲染失败：{e:?}"));
                }
                other => panic!("{name} p{page} 意外回执（单作业应为 Done）：{other:?}"),
            }
        }
        assert!(
            Instant::now() < deadline,
            "{name} p{page} 渲染 30s 未回执（真缺陷——看门狗应换代重试或超时回报）"
        );
        std::thread::sleep(Duration::from_millis(4));
    }
}

/// 网格对照：返回最大逐格差（供证据行；越差即断言失败——报文件/页/
/// 格坐标与两侧值）。
fn grid_worst_diff(ours: &[Vec<f64>], reference: &serde_json::Value) -> f64 {
    let mut worst = 0f64;
    let rows = reference
        .as_array()
        .expect("参照网格必须是二维数组（row-major）");
    for (j, row) in rows.iter().enumerate() {
        for (i, cell) in row.as_array().expect("参照网格行").iter().enumerate() {
            let rv = cell.as_f64().expect("参照格值必须是数字");
            worst = worst.max((ours[j][i] - rv).abs());
        }
    }
    worst
}

// ---------------------------------------------------------------------------
// 测试 1：引擎内容×几何矩阵（15 文件 × 25 探针页 vs poppler 参照）
// ---------------------------------------------------------------------------

#[test]
fn engine_content_matches_poppler_reference() {
    let manifest = load_manifest();
    eprintln!(
        "#634 运行时证据：poppler {} | 参照 pdfium build {} | 测试光栅宽 {}px | 网格 {}×{} | 墨迹阈值 <{}",
        manifest["poppler_version"],
        manifest["pdfium_build"],
        manifest["raster_width_px"],
        manifest["grid"],
        manifest["grid"],
        manifest["ink_threshold"],
    );
    match std::env::var("MDC_PDFIUM_LIB") {
        Ok(lib) => eprintln!("#634 运行时证据：MDC_PDFIUM_LIB={lib}"),
        Err(_) => eprintln!(
            "#634 运行时证据：MDC_PDFIUM_LIB 未设（定位序：env → exe 同目录 → 候选清单 → 系统）"
        ),
    }
    // 运行时发现方式走查（deploy_check 生产诊断面——绑定路径与结果入证据）
    eprintln!("#634 运行时证据（deploy_check 走查）：");
    eprintln!(
        "{}",
        mdc_pdf::deploy_check::render_report(&mdc_pdf::deploy_check::run_check())
    );

    // 环境门先行（缺库 → 全测 SKIP；不让坏文件断言空转在库错误上）。
    if let Err(err) = PdfiumEngine::open(fixture_bytes("zh_embedded_font.pdf")) {
        let msg = format!("{err:?}");
        if is_missing_library_error(&msg) {
            skip_banner(&msg, "引擎内容×几何矩阵");
            return;
        }
        panic!("完好探针文件打开失败（真缺陷，非环境缺件）：{msg}");
    }

    let mut checked_files = 0usize;
    let mut checked_pages = 0usize;
    for (name, entry) in manifest["files"]
        .as_object()
        .expect("manifest.files 必须是对象")
    {
        let bytes = fixture_bytes(name);

        // 坏文件分支：typed 拒绝 + 报错信息非空（不崩不挂不空白成功）。
        if entry["expected"] == "error" {
            match PdfiumEngine::open(bytes) {
                Ok(_) => panic!("{name} 是坏文件——引擎必须拒绝打开（三方对照见 manifest）"),
                Err(err) => {
                    let msg = format!("{err:?}");
                    assert!(
                        !is_missing_library_error(&msg),
                        "{name} 拒绝原因不可是环境缺件（库已在位——环境门已过）"
                    );
                    assert!(msg.trim().len() > 8, "{name} 拒绝信息不可为空");
                    eprintln!("#634 健壮 ✓ {name} → typed 拒绝（与 pypdf/poppler 三方同判）");
                    continue;
                }
            }
        }

        // 完好文件：生产同构——主线程引擎几何预读 + 看门狗工人（字节
        // 快照重开——pdf_pages/pdf_thumbs 构造同款）。
        let engine = PdfiumEngine::open(bytes.clone())
            .unwrap_or_else(|e| panic!("{name}（完好文件）打开失败：{e:?}"));
        let snapshot = Arc::new(bytes);
        let mut worker = RenderWorker::watchdog(
            move || {
                PdfiumEngine::open((*snapshot).clone())
                    .expect("pdfium 看门狗工厂：字节快照重开失败（构造期已验证可开）")
            },
            Duration::from_secs(20),
        );

        let pages = entry["pages"]
            .as_object()
            .unwrap_or_else(|| panic!("{name} 完好文件必须有探针页"));
        for (page_str, page_entry) in pages {
            let page: u32 = page_str.parse().expect("探针页键必须是 0 基数字");

            // ① 几何（pypdf 独立解析对照——含 /Rotate 与可见域口径）。
            let geo = engine
                .page_geometry(PageIndex(page))
                .unwrap_or_else(|e| panic!("{name} p{page} 几何读取失败：{e:?}"));
            let exp_w = page_entry["pypdf"]["render_w_pt"]
                .as_f64()
                .expect("render_w_pt");
            let exp_h = page_entry["pypdf"]["render_h_pt"]
                .as_f64()
                .expect("render_h_pt");
            assert!(
                (f64::from(geo.effective_width()) - exp_w).abs() <= 0.75,
                "{name} p{page} 有效宽 {}pt ≠ pypdf 独立解析 {exp_w}pt（±0.75 容差）",
                geo.effective_width()
            );
            assert!(
                (f64::from(geo.effective_height()) - exp_h).abs() <= 0.75,
                "{name} p{page} 有效高 {}pt ≠ pypdf 独立解析 {exp_h}pt（±0.75 容差）",
                geo.effective_height()
            );

            // ② 内容（640px 渲染 → 24×24 网格双指标 vs poppler 参照）。
            let job = worker.submit(RenderRequest {
                page: PageIndex(page),
                width_px: RASTER_W,
            });
            let rendered = drain_one(&mut worker, job, name, page);
            assert_eq!(
                rendered.width_px, RASTER_W,
                "{name} p{page} 光栅宽必须按请求精确回显"
            );
            let ref_h = ref_height(page_entry["ref_png"].as_str().expect("ref_png"));
            let our_h = rendered.height_px as usize;
            assert!(
                our_h.abs_diff(ref_h) <= 2,
                "{name} p{page} 引擎高 {our_h}px ≠ poppler 参照高 {ref_h}px（差 >2px = \
                 几何级发现，不在中心对齐带内吸收——报 Lead，不静默）"
            );
            let rows =
                fit_height_to_reference(luma_rows(&rendered.bgra, RASTER_W as usize, our_h), ref_h);
            let (mean, ink) = grids(&rows);
            let tol_mean = page_entry["tol_mean_max_cell"]
                .as_f64()
                .expect("tol_mean_max_cell");
            let tol_ink = page_entry["tol_ink_max_cell"]
                .as_f64()
                .expect("tol_ink_max_cell");
            let worst_mean = grid_worst_diff(&mean, &page_entry["grid_mean"]);
            let worst_ink = grid_worst_diff(&ink, &page_entry["grid_ink"]);
            assert!(
                worst_mean <= tol_mean + 1e-3,
                "{name} p{page} mean 网格越差：最大逐格差 {worst_mean:.4} > 容差 {tol_mean}\
                 （对照 poppler 参照——抗锯齿级差异应在校准余量内）"
            );
            assert!(
                worst_ink <= tol_ink + RESAMPLE_INK_ALLOWANCE + 1e-3,
                "{name} p{page} ink 网格越差：最大逐格差 {worst_ink:.4} > 容差 {tol_ink}+{RESAMPLE_INK_ALLOWANCE}\
                 （对照 poppler 参照——内容/几何级差异才会越出）"
            );
            eprintln!(
                "#634 对照 ✓ {name} p{page}：几何 {}×{}pt | mean {worst_mean:.4}/{tol_mean:.3} ink {worst_ink:.4}/{:.3}",
                exp_w,
                exp_h,
                tol_ink + RESAMPLE_INK_ALLOWANCE
            );
            checked_pages += 1;
        }
        checked_files += 1;
        // 工人收束（下一文件换新会话——逐文件自证多会话不互锁的串行面；
        // 并发面在 robustness 测试）。
        drop(worker);
    }

    assert!(
        checked_files >= 13 && checked_pages >= 25,
        "矩阵覆盖面坍缩（{checked_files} 文件 / {checked_pages} 页——清单不得静默缩水）"
    );
    eprintln!(
        "#634 矩阵核验 ✓ {checked_files} 文件 × {checked_pages} 探针页（poppler {} vs pdfium，分页容差=观测×1.3）",
        manifest["poppler_version"]
    );
}

// ---------------------------------------------------------------------------
// 测试 2：视图链（open → 首屏 → 跳页 → 缩放 → 关闭重开）
// ---------------------------------------------------------------------------

/// 视图宿主（pdf_pages 既有测试 BitmapHost 同款：整树 + size_full）。
struct PagesHost(gpui::Entity<PdfPagesView>);

impl gpui::Render for PagesHost {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        gpui::div().size_full().child(self.0.clone())
    }
}

/// 泵：推进测试时钟 + 真实渲染线程跑光栅 + 16ms 轮询 drain，每轮
/// 重绘一帧（渲染期读数面 top_page/键宽随帧刷新——#420 同教训）。
fn pump_ready(
    cx: &mut gpui::TestAppContext,
    visual: &mut gpui::VisualTestContext,
    view: &gpui::Entity<PdfPagesView>,
    secs: u64,
    what: &str,
    ready: impl Fn(&PdfPagesView) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        cx.background_executor
            .advance_clock(Duration::from_millis(20));
        cx.background_executor.run_until_parked();
        std::thread::sleep(Duration::from_millis(2));
        visual.update(|window, app| window.draw(app).clear(app));
        if view.read_with(visual, |v, _| ready(v)) {
            return;
        }
        assert!(Instant::now() < deadline, "{what} 泵超时（真缺陷不放过）");
    }
}

#[gpui::test]
fn view_chain_first_screen_scroll_zoom_close_reopen(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let bytes = fixture_bytes("long_doc_120p.pdf");
    let view = cx.update(|app| app.new(|cx| PdfPagesView::new(bytes.clone(), cx)));
    if let Some(err) = view.read_with(cx, |v, _| v.error_for_test()) {
        if is_missing_library_error(&err) {
            skip_banner(&err, "视图链核验");
            return;
        }
        panic!("百页长文档（完好文件）打开失败：{err}");
    }
    assert_eq!(view.read_with(cx, |v, _| v.page_count()), 120, "百页夹具");

    // ① open + 首屏：真实视口 probe + 看门狗工人 + 16ms 轮询全链。
    let window = cx.open_window(gpui::size(gpui::px(600.), gpui::px(480.)), {
        let view = view.clone();
        move |_, _| PagesHost(view)
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, app| window.draw(app).clear(app));
    pump_ready(cx, &mut visual, &view, 30, "首屏位图", |v| {
        v.page_ready_for_test(0)
    });
    eprintln!("#634 视图链 ✓ 首屏位图就绪（120 页长文档，真实视口 probe）");

    // ② 跳页 60（0 基）：top_visible_page 读回 + 目标页位图就绪。
    cx.update(|app| view.update(app, |v, _| v.scroll_to_page(60)));
    pump_ready(cx, &mut visual, &view, 30, "跳页读回", |v| {
        v.top_visible_page() == 60
    });
    pump_ready(cx, &mut visual, &view, 30, "跳页位图", |v| {
        v.page_ready_for_test(60)
    });
    eprintln!("#634 视图链 ✓ 跳页 60：top_visible_page 读回 60 + 位图就绪");

    // ③ 缩放换档：Ctrl+1 实际大小（公开 dispatch_zoom_key 真路径）——
    //    A4 595.28pt → quantize 640px（FitWidth 576px → 640px 换档）。
    let consumed =
        cx.update(|app| view.update(app, |v, cx| v.dispatch_zoom_key("1", true, false, cx)));
    assert!(consumed, "Ctrl+1 必须被 pdf 键域消费");
    assert_eq!(
        view.read_with(&visual, |v, _| v.raster_width_for_test()),
        640,
        "实际大小档光栅宽（A4 595pt → quantize 640）"
    );
    pump_ready(cx, &mut visual, &view, 30, "换档重渲", |v| {
        v.page_ready_for_test(60)
    });
    eprintln!("#634 视图链 ✓ 缩放换档：576→640px 新键重渲就绪");

    // ④ 缓存预算不变量（受限缓存接线面——LRU 逐出语义在 crates/pdf
    //    cache 单测；此处断言视图面持有的缓存恒在预算内）。
    let (bytes_used, entries) = view.read_with(&visual, |v, _| v.cache_stats_for_test());
    assert!(
        bytes_used <= crate::pdf_pages::CACHE_BUDGET_BYTES,
        "缓存 {bytes_used}B 越预算 {}B",
        crate::pdf_pages::CACHE_BUDGET_BYTES
    );
    eprintln!(
        "#634 视图链 ✓ 缓存受限：{bytes_used}B / {}B（{entries} 条）",
        crate::pdf_pages::CACHE_BUDGET_BYTES
    );

    // ⑤ 关闭重开（remove_window 真关窗 → 宿主释放 → 实体释放 →
    //    工人线程收束；同字节新会话无残留锁，位图再现）。
    visual.update(|window, _| window.remove_window());
    drop(visual);
    let _ = window; // WindowHandle 是 Copy——真实关闭在 remove_window，句柄丢弃非关闭面
    drop(view); // 实体末引用释放 → 工人线程收束（重开前完成真关闭链）
    let view2 = cx.update(|app| app.new(|cx| PdfPagesView::new(bytes.clone(), cx)));
    assert!(
        view2.read_with(cx, |v, _| v.error_for_test()).is_none(),
        "重开必须成功（同字节——上会话不残留锁）"
    );
    let window2 = cx.open_window(gpui::size(gpui::px(600.), gpui::px(480.)), {
        let view2 = view2.clone();
        move |_, _| PagesHost(view2)
    });
    let mut visual2 = gpui::VisualTestContext::from_window(window2.into(), cx);
    visual2.update(|window, app| window.draw(app).clear(app));
    pump_ready(cx, &mut visual2, &view2, 30, "重开首屏", |v| {
        v.page_ready_for_test(0)
    });
    eprintln!("#634 视图链 ✓ 关闭重开：同字节新会话位图再现");
}

// ---------------------------------------------------------------------------
// 测试 3：健壮性（坏文件 / 超常尺寸 / 多标签并存切换）
// ---------------------------------------------------------------------------

#[gpui::test]
fn robustness_damaged_oversized_multi_tab(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });

    // 环境门先行（缺库 → 全测 SKIP；坏文件视图态不该空转在库错误上）。
    let probe =
        cx.update(|app| app.new(|cx| PdfPagesView::new(fixture_bytes("zh_embedded_font.pdf"), cx)));
    if let Some(err) = probe.read_with(cx, |v, _| v.error_for_test()) {
        if is_missing_library_error(&err) {
            skip_banner(&err, "健壮性核验");
            return;
        }
        panic!("完好探针文件视图打开失败（真缺陷）：{err}");
    }
    drop(probe);

    // ① 坏文件视图态：error 明示 + 零页 + 不 panic（不白屏不闪退）。
    for name in ["damaged_truncated.pdf", "damaged_garbage.pdf"] {
        let view = cx.update(|app| app.new(|cx| PdfPagesView::new(fixture_bytes(name), cx)));
        let err = view
            .read_with(cx, |v, _| v.error_for_test())
            .unwrap_or_else(|| panic!("{name} 视图必须进 error 态"));
        assert!(
            !is_missing_library_error(&err),
            "{name} 错误不可是环境缺件（库已在位）"
        );
        assert!(!err.trim().is_empty(), "{name} 错误明示不可为空");
        assert_eq!(view.read_with(cx, |v, _| v.page_count()), 0, "{name} 零页");
        eprintln!("#634 健壮 ✓ {name} 视图 error 态明示（不白屏不闪退）");
    }

    // ② 多标签并存 + 快速切换 + 关闭后回来（两文档两工人并发）。
    let view_a =
        cx.update(|app| app.new(|cx| PdfPagesView::new(fixture_bytes("zh_embedded_font.pdf"), cx)));
    let view_b =
        cx.update(|app| app.new(|cx| PdfPagesView::new(fixture_bytes("scanned_page.pdf"), cx)));
    let bytes_a = view_a.read_with(cx, |v, _| v.bytes());
    let window_a = cx.open_window(gpui::size(gpui::px(600.), gpui::px(480.)), {
        let view_a = view_a.clone();
        move |_, _| PagesHost(view_a)
    });
    let mut visual_a = gpui::VisualTestContext::from_window(window_a.into(), cx);
    let window_b = cx.open_window(gpui::size(gpui::px(600.), gpui::px(480.)), {
        let view_b = view_b.clone();
        move |_, _| PagesHost(view_b)
    });
    let mut visual_b = gpui::VisualTestContext::from_window(window_b.into(), cx);
    visual_a.update(|window, app| window.draw(app).clear(app));
    visual_b.update(|window, app| window.draw(app).clear(app));

    // 快速切换（交替泵 A/B——两工人并发渲染互不阻塞）。
    for round in 0..40 {
        cx.background_executor
            .advance_clock(Duration::from_millis(20));
        cx.background_executor.run_until_parked();
        std::thread::sleep(Duration::from_millis(2));
        visual_a.update(|window, app| window.draw(app).clear(app));
        visual_b.update(|window, app| window.draw(app).clear(app));
        let (a_ready, b_ready) = (
            view_a.read_with(&visual_a, |v, _| v.page_ready_for_test(0)),
            view_b.read_with(&visual_b, |v, _| v.page_ready_for_test(0)),
        );
        if a_ready && b_ready {
            eprintln!("#634 健壮 ✓ 多标签并存：双文档双工人并发就绪（切换泵 {round} 轮内）");
            break;
        }
        assert!(round < 39, "多标签并发泵 40 轮未双就绪（互锁嫌疑——真缺陷）");
    }

    // 关闭 B（remove_window 真关窗 → 宿主/实体/工人线程整树释放）后
    // A 继续可用 + B 重开（关闭后回来）。
    visual_b.update(|window, _| window.remove_window());
    drop(visual_b);
    let _ = window_b; // Copy 句柄——真实关闭在 remove_window
    drop(view_b); // 实体末引用释放 → 工人线程收束
    cx.update(|app| view_a.update(app, |v, _| v.scroll_to_page(1)));
    pump_ready(cx, &mut visual_a, &view_a, 30, "关 B 后 A 跳页", |v| {
        v.top_visible_page() == 1 && v.page_ready_for_test(1)
    });
    eprintln!("#634 健壮 ✓ 关闭 B 后 A 继续跳页就绪（会话不互锁）");
    let view_b2 =
        cx.update(|app| app.new(|cx| PdfPagesView::new(fixture_bytes("scanned_page.pdf"), cx)));
    let window_b2 = cx.open_window(gpui::size(gpui::px(600.), gpui::px(480.)), {
        let view_b2 = view_b2.clone();
        move |_, _| PagesHost(view_b2)
    });
    let mut visual_b2 = gpui::VisualTestContext::from_window(window_b2.into(), cx);
    visual_b2.update(|window, app| window.draw(app).clear(app));
    pump_ready(cx, &mut visual_b2, &view_b2, 30, "B 重开首屏", |v| {
        v.page_ready_for_test(0)
    });
    eprintln!("#634 健壮 ✓ 关闭后回来：B 同字节重开位图再现");
    let _ = bytes_a; // ⑤ 重开链在测试 2 覆盖；此处仅保 A 字节所有权干净。

    // ③ 超常尺寸页（2000×2800pt——quantize 域内正常渲染，不拒不开）。
    let view_c =
        cx.update(|app| app.new(|cx| PdfPagesView::new(fixture_bytes("oversized_page.pdf"), cx)));
    assert!(
        view_c.read_with(cx, |v, _| v.error_for_test()).is_none(),
        "超常尺寸（域内）必须正常打开"
    );
    let window_c = cx.open_window(gpui::size(gpui::px(600.), gpui::px(480.)), {
        let view_c = view_c.clone();
        move |_, _| PagesHost(view_c)
    });
    let mut visual_c = gpui::VisualTestContext::from_window(window_c.into(), cx);
    visual_c.update(|window, app| window.draw(app).clear(app));
    pump_ready(cx, &mut visual_c, &view_c, 30, "超常尺寸首屏", |v| {
        v.page_ready_for_test(0)
    });
    eprintln!("#634 健壮 ✓ 超常尺寸页（2000×2800pt）首屏位图就绪");
}
