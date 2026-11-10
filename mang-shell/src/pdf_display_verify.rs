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
//! # D1（#51 工作线 D）：活参照独立对照（PARITY 门控）
//!
//! 静态 manifest 网格由 Linux/poppler 25.03 预生成——无嵌入字体的
//! 文本页（rotated_90 等）在异平台字体栈下与静态网格存在系统级漂移
//! （Windows/poppler 26.09 实测 mean 0.2569 > 固定容差 0.24——#51
//! D1 登记）。`MDC_634_REQUIRE_PARITY=1` 切换为**本机 pdftoppm 活参照**
//! （命令与 manifest `poppler_command` 逐项一致；PPM P6 与参照 PNG
//! 同渲染路径实测 100% 逐像素一致）：同机同版本同字体栈的独立对照
//! ——排除字体栈差异，**阈值不放宽、断言不删减**；同时记录网格
//! NCC / 像素级墨迹 IoU / 静态网格漂移（跨平台差异的量化面）。
//! PARITY=1 而 pdftoppm 缺件 = panic（独立对照门要求真跑，SKIP 不计
//! 通过）。跨平台版本差异由此以活参照证明（26.09 机器用 26.09 自证，
//! 25.03 机器用 25.03 自证——poppler ≠ pdfium 的独立性不变）。
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
use mdc_pdf::page::{PageIndex, Rotation};
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

/// D1（#51）：缺件守卫统一出口——PARITY=1 时任何环境缺件（无库/
/// 无独立参照）= **panic**（显示矩阵要求真跑与独立对照在场，SKIP
/// 不计通过）；未设 PARITY 才走显式 SKIP 横幅（缺件环境可诊断）。
/// 三个测试的库门与测试 1 的活参照门共用此出口——门语义单点，不漂移。
fn missing_guard(msg: &str, face: &str) {
    if parity_required() {
        panic!(
            "MDC_634_REQUIRE_PARITY=1 而 {face} 缺件：独立对照门要求真跑，\
         缺库/缺参照不可 SKIP 计通过。错误全文：{msg}"
        );
    }
    skip_banner(msg, face);
}

// ---------------------------------------------------------------------------
// 网格度量（与 scripts/display_corpus/build_corpus.py 逐位对齐）
// ---------------------------------------------------------------------------

/// PIL `convert("L")` 的 L24 整数公式（亮度权重 299/587/114 + 半值
/// 进位——精确镜像，不是近似）。RGB 入参形态（活参照 PPM 与引擎
/// BGRA 两调用方共享同一数值口径）。
#[inline]
fn pil_luma_rgb(r: u8, g: u8, b: u8) -> u8 {
    ((r as u32 * 19595 + g as u32 * 38470 + b as u32 * 7471 + 0x8000) >> 16) as u8
}

/// 引擎 BGRA 像素 → luma（内部走 [`pil_luma_rgb`] 同一公式）。
#[inline]
fn pil_luma(bgra: &[u8]) -> u8 {
    pil_luma_rgb(bgra[2], bgra[1], bgra[0])
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
fn grid_worst_diff(ours: &[Vec<f64>], reference: &[Vec<f64>]) -> f64 {
    let mut worst = 0f64;
    for (row_ours, row_ref) in ours.iter().zip(reference) {
        assert_eq!(row_ours.len(), row_ref.len(), "参照网格列数合同");
        for (v, rv) in row_ours.iter().zip(row_ref) {
            worst = worst.max((v - rv).abs());
        }
    }
    worst
}

// ---------------------------------------------------------------------------
// D1（#51 工作线 D）：活参照独立对照（PARITY 门控）
// ---------------------------------------------------------------------------

/// PARITY 环境变量（=1 时矩阵对照必须用本机 pdftoppm 活参照——
/// 独立对照门要求真跑；缺 pdftoppm = panic，SKIP 不计通过）。
const PARITY_ENV: &str = "MDC_634_REQUIRE_PARITY";

fn parity_required() -> bool {
    std::env::var(PARITY_ENV).map(|v| v == "1").unwrap_or(false)
}

/// PATH 中 pdftoppm 版本（无 → None；stderr 形如 "pdftoppm version 25.03.0"）。
fn pdftoppm_version() -> Option<String> {
    let out = std::process::Command::new("pdftoppm")
        .arg("-v")
        .output()
        .ok()?;
    let txt = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    txt.lines()
        .find_map(|l| l.split_once("version ").map(|(_, v)| v.trim().to_string()))
}

/// PPM P6 解析（pdftoppm 默认输出——与参照 PNG 同渲染路径仅编码不同，
/// 实测 100% 逐像素一致：同页 PNG/PPM 双渲对拍 289920/289920 像素全同）。
/// 返回 (w, h, RGB 字节)。头部：magic + 宽/高/maxval 三整数，`#` 行注释。
fn parse_ppm_p6(data: &[u8]) -> (usize, usize, Vec<u8>) {
    let mut pos = 0usize;
    let mut tokens: Vec<String> = Vec::with_capacity(4);
    while tokens.len() < 4 {
        while pos < data.len() && data[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos < data.len() && data[pos] == b'#' {
            while pos < data.len() && data[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        let start = pos;
        while pos < data.len() && !data[pos].is_ascii_whitespace() {
            pos += 1;
        }
        assert!(
            pos > start,
            "PPM P6 头不完整（第 {} 个 token 处截断）",
            tokens.len()
        );
        tokens.push(String::from_utf8_lossy(&data[start..pos]).into_owned());
    }
    pos += 1; // maxval 后恰一个空白分隔，其后为原始像素字节
    assert_eq!(
        tokens[0], "P6",
        "pdftoppm 输出必须是 P6（实得 {}）",
        tokens[0]
    );
    let w: usize = tokens[1].parse().expect("PPM 宽");
    let h: usize = tokens[2].parse().expect("PPM 高");
    let maxval: u16 = tokens[3].parse().expect("PPM maxval");
    assert_eq!(maxval, 255, "pdftoppm P6 maxval 必须 255（实得 {maxval}）");
    let rgb = data[pos..].to_vec();
    assert_eq!(rgb.len(), w * h * 3, "PPM 像素缓冲长度（w×h×3）");
    (w, h, rgb)
}

/// 活参照：本机 pdftoppm 渲染单页 → 灰度行阵（命令与 manifest
/// `poppler_command` 逐项对齐——同宽 640/同 -cropbox/同旋转口径）。
/// 返回行阵的行数即参照高（poppler ceil 口径，与静态 PNG 名义高一致）。
/// 临时目录读后即清，不留测试残留面。
fn live_reference_luma(bytes: &[u8], page: u32, raster_w: u32) -> Vec<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!(
        "mdc634-live-{}-p{page}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("活参照临时目录创建");
    let pdf_path = dir.join("probe.pdf");
    std::fs::write(&pdf_path, bytes).expect("活参照探针写入");
    let page1 = page + 1; // pdftoppm 1 基页号
    let out = std::process::Command::new("pdftoppm")
        .args([
            "-r",
            "96",
            "-f",
            &page1.to_string(),
            "-l",
            &page1.to_string(),
            "-scale-to-x",
            &raster_w.to_string(),
            "-scale-to-y",
            "-1",
            "-cropbox",
            "-scale-dimension-before-rotation",
        ])
        .arg(&pdf_path)
        .arg(dir.join("ref"))
        .output()
        .expect("pdftoppm 调起（活参照门已在测试头验证在场）");
    let ppm_data = std::fs::read_dir(&dir)
        .expect("活参照目录读取")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().map(|x| x == "ppm").unwrap_or(false))
        .map(|p| std::fs::read(p))
        .expect("活参照 PPM 产出（唯一单页）");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "pdftoppm 活参照渲染失败：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (w, h, rgb) = parse_ppm_p6(&ppm_data.expect("PPM 字节读回"));
    assert_eq!(w as u32, raster_w, "活参照宽必须精确 {raster_w}px");
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| {
                    let i = (y * w + x) * 3;
                    pil_luma_rgb(rgb[i], rgb[i + 1], rgb[i + 2])
                })
                .collect()
        })
        .collect()
}

/// 网格 NCC（归一化互相关——D1 记录指标：1.0 = 网格完全同形；
/// 零方差面（双侧纯白）无定义按同形计 1.0——内容正确性由网格双指标
/// 断言承担，NCC 仅证据记录面）。
fn grid_ncc(a: &[Vec<f64>], b: &[Vec<f64>]) -> f64 {
    let flat_len = |g: &[Vec<f64>]| g.iter().map(|r| r.len()).sum::<usize>() as f64;
    let (na, nb) = (flat_len(a), flat_len(b));
    let (ma, mb) = (
        a.iter().flatten().sum::<f64>() / na,
        b.iter().flatten().sum::<f64>() / nb,
    );
    let (mut sa, mut sb, mut sab) = (0f64, 0f64, 0f64);
    for (ra, rb) in a.iter().zip(b) {
        for (va, vb) in ra.iter().zip(rb) {
            let (da, db) = (va - ma, vb - mb);
            sa += da * da;
            sb += db * db;
            sab += da * db;
        }
    }
    if sa <= f64::EPSILON || sb <= f64::EPSILON {
        return 1.0;
    }
    sab / (sa * sb).sqrt()
}

/// 像素级墨迹掩码 IoU（luma<200 判墨；D1 记录指标——掩码交并比）。
fn ink_iou_px(a: &[Vec<u8>], b: &[Vec<u8>]) -> f64 {
    let (mut inter, mut union) = (0u64, 0u64);
    for (ra, rb) in a.iter().zip(b) {
        for (va, vb) in ra.iter().zip(rb) {
            let (ia, ib) = (*va < INK_THRESHOLD, *vb < INK_THRESHOLD);
            inter += (ia && ib) as u64;
            union += (ia || ib) as u64;
        }
    }
    if union == 0 {
        1.0 // 双侧无墨（纯白页）——按同形计
    } else {
        inter as f64 / union as f64
    }
}

/// manifest 网格（serde Value）→ 行阵（统一对照函数消费）。
fn grid_from_manifest(v: &serde_json::Value) -> Vec<Vec<f64>> {
    v.as_array()
        .expect("参照网格必须是二维数组（row-major）")
        .iter()
        .map(|row| {
            row.as_array()
                .expect("参照网格行")
                .iter()
                .map(|c| c.as_f64().expect("参照格值必须是数字"))
                .collect()
        })
        .collect()
}

/// 旋转枚举 → 度（pypdf manifest 对照形态）。
fn rotation_deg(r: Rotation) -> i64 {
    match r {
        Rotation::Deg0 => 0,
        Rotation::Deg90 => 90,
        Rotation::Deg180 => 180,
        Rotation::Deg270 => 270,
    }
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

    // D1（#51 工作线 D）：PARITY 门控——活参照独立对照必须在场。
    let parity = parity_required();
    let live_ver = pdftoppm_version();
    if parity {
        let ver = live_ver.as_deref().unwrap_or_else(|| {
            panic!(
                "MDC_634_REQUIRE_PARITY=1：活对照门要求 PATH 中有 pdftoppm（独立参照），\
             缺件不可 SKIP 计通过（#51 D1：不以 SKIP/尺寸非空代替页面正确）"
            )
        });
        eprintln!(
            "#634 D1 活参照：本机 pdftoppm {ver}（参照命令与 manifest 逐项一致；\
             对照阈值仍用 manifest 分页容差，不放宽）"
        );
    } else {
        eprintln!(
            "#634 D1：MDC_634_REQUIRE_PARITY 未设——参照 = 静态 manifest（poppler {}）；\
             设 1 时切换为本机 pdftoppm 活参照（pdftoppm 在场：{}）",
            manifest["poppler_version"],
            live_ver.as_deref().unwrap_or("无"),
        );
    }

    // 环境门先行（缺库：PARITY=1 → fail；否则 SKIP——不让坏文件断言
    // 空转在库错误上）。
    if let Err(err) = PdfiumEngine::open(fixture_bytes("zh_embedded_font.pdf")) {
        let msg = format!("{err:?}");
        if is_missing_library_error(&msg) {
            missing_guard(&msg, "引擎内容×几何矩阵");
            return;
        }
        panic!("完好探针文件打开失败（真缺陷，非环境缺件）：{msg}");
    }

    let mut checked_files = 0usize;
    let mut checked_pages = 0usize;
    // D1 记录面汇总（PARITY 模式——矩阵尾证据行：旋转/极端尺寸已逐页
    // 断言，跨文档 NCC/IoU/漂移在此汇总）。
    let (mut min_ncc, mut min_iou, mut max_static_drift) = (1f64, 1f64, 0f64);
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
        let snapshot = Arc::new(bytes.clone()); // D1：bytes 保留给活参照渲染（clone 一次，测试面）
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
            // D1：旋转标记逐页断言（离散语义零容差——rotated_90 p0=90°/
            // p1=270°、rotated_180=180°、混合页随页登记；引擎枚举必须
            // 与 pypdf /Rotate 逐位一致，容差断言不吸收错向旋转）。
            let exp_rot = page_entry["pypdf"]["rotation"]
                .as_i64()
                .expect("pypdf rotation 字段");
            assert_eq!(
                rotation_deg(geo.rotation),
                exp_rot,
                "{name} p{page} 旋转标记：引擎 {:?} ≠ pypdf /Rotate {exp_rot}°",
                geo.rotation
            );

            // ② 内容（640px 渲染 → 24×24 网格双指标 vs 独立参照）。
            //    D1（#51）：PARITY=1 → 本机 pdftoppm 活参照（同机同版本
            //    同字体栈的独立对照——rotated_90 等无嵌入字体页的跨平台
            //    字体栈差异被排除，阈值不放宽）；默认 → 静态 manifest
            //    网格（poppler 25.03 观测口径，缺件环境可跑）。
            let job = worker.submit(RenderRequest {
                page: PageIndex(page),
                width_px: RASTER_W,
            });
            let rendered = drain_one(&mut worker, job, name, page);
            assert_eq!(
                rendered.width_px, RASTER_W,
                "{name} p{page} 光栅宽必须按请求精确回显"
            );
            let our_h = rendered.height_px as usize;
            // D1 规约（#51）：活参照双侧**同规约**——引擎位图与活参照
            // PPM 都线性 fit 到 H_ref = round(640 × 高/宽)（pypdf 几何
            // 基准——与 build_corpus.py 网格规约同基）。实测：规约不
            // 匹配（引擎 fit 到 PPM 原生高、参照原生直出网格）引入
            // ~0.02 级伪差（zh_embedded_font p1 ink 0.0916 > 0.085）；
            // 同规约后 0.0468/0.07 双绿——**阈值不放宽**，差值即为纯
            // 渲染差（poppler ≠ pdfium 的独立性不变）。静态模式沿用
            // PNG 名义高（ceil）现状——13×25 已绿口径不动。
            let (rows, live_fit) = if parity {
                let live = live_reference_luma(&bytes, page, RASTER_W);
                let live_h = live.len();
                let h_ref = ((RASTER_W as f64) * exp_h / exp_w).round() as usize;
                assert!(
                    our_h.abs_diff(live_h) <= 2,
                    "{name} p{page} 引擎高 {our_h}px ≠ 活参照高 {live_h}px（差 >2px = \
                     几何级发现，不在中心对齐带内吸收——报 Lead，不静默）"
                );
                let live_fit = fit_height_to_reference(live, h_ref);
                let rows = fit_height_to_reference(
                    luma_rows(&rendered.bgra, RASTER_W as usize, our_h),
                    h_ref,
                );
                (rows, Some(live_fit))
            } else {
                let ref_h = ref_height(page_entry["ref_png"].as_str().expect("ref_png"));
                assert!(
                    our_h.abs_diff(ref_h) <= 2,
                    "{name} p{page} 引擎高 {our_h}px ≠ 参照高 {ref_h}px（差 >2px = \
                     几何级发现，不在中心对齐带内吸收——报 Lead，不静默）"
                );
                (
                    fit_height_to_reference(
                        luma_rows(&rendered.bgra, RASTER_W as usize, our_h),
                        ref_h,
                    ),
                    None,
                )
            };
            let (mean, ink) = grids(&rows);
            let (ref_mean, ref_ink) = if let Some(live) = &live_fit {
                grids(live)
            } else {
                (
                    grid_from_manifest(&page_entry["grid_mean"]),
                    grid_from_manifest(&page_entry["grid_ink"]),
                )
            };
            let tol_mean = page_entry["tol_mean_max_cell"]
                .as_f64()
                .expect("tol_mean_max_cell");
            let tol_ink = page_entry["tol_ink_max_cell"]
                .as_f64()
                .expect("tol_ink_max_cell");
            let worst_mean = grid_worst_diff(&mean, &ref_mean);
            let worst_ink = grid_worst_diff(&ink, &ref_ink);
            assert!(
                worst_mean <= tol_mean + 1e-3,
                "{name} p{page} mean 网格越差：最大逐格差 {worst_mean:.4} > 容差 {tol_mean}\
                 （对照独立参照——抗锯齿级差异应在校准余量内）"
            );
            assert!(
                worst_ink <= tol_ink + RESAMPLE_INK_ALLOWANCE + 1e-3,
                "{name} p{page} ink 网格越差：最大逐格差 {worst_ink:.4} > 容差 {tol_ink}+{RESAMPLE_INK_ALLOWANCE}\
                 （对照独立参照——内容/几何级差异才会越出）"
            );
            if parity {
                // D1 记录面：NCC / IoU / 静态网格漂移（跨平台字体栈差异
                // 的量化——Windows/poppler 26.09 上 rotated_90 实测漂移
                // 0.2569 的根因即在字体栈，活参照消解后漂移应归零量级）。
                let ncc = grid_ncc(&mean, &ref_mean);
                let iou = ink_iou_px(&rows, live_fit.as_ref().expect("活参照行阵"));
                let static_drift_mean =
                    grid_worst_diff(&mean, &grid_from_manifest(&page_entry["grid_mean"]));
                let static_drift_ink =
                    grid_worst_diff(&ink, &grid_from_manifest(&page_entry["grid_ink"]));
                eprintln!(
                    "#634 D1 活参照 ✓ {name} p{page}：mean {worst_mean:.4}/{tol_mean:.3} \
                     ink {worst_ink:.4}/{:.3} | NCC {ncc:.4} | IoU {iou:.4} | \
                     静态漂移 mean {static_drift_mean:.4} / ink {static_drift_ink:.4} \
                     （活 poppler {} vs 静态 {}）",
                    tol_ink + RESAMPLE_INK_ALLOWANCE,
                    live_ver.as_deref().unwrap_or("?"),
                    manifest["poppler_version"],
                );
                min_ncc = min_ncc.min(ncc);
                min_iou = min_iou.min(iou);
                max_static_drift = max_static_drift.max(static_drift_mean);
            } else {
                eprintln!(
                    "#634 对照 ✓ {name} p{page}：几何 {}×{}pt | mean {worst_mean:.4}/{tol_mean:.3} ink {worst_ink:.4}/{:.3}",
                    exp_w,
                    exp_h,
                    tol_ink + RESAMPLE_INK_ALLOWANCE
                );
            }
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
    if parity {
        eprintln!(
            "#634 D1 矩阵核验 ✓ {checked_files} 文件 × {checked_pages} 探针页（活参照 pdftoppm {}：\
             阈值不放宽）| 跨文档汇总：min NCC {min_ncc:.4} / min IoU {min_iou:.4} / \
             max 静态漂移 {max_static_drift:.4}（静态源 poppler {}）",
            live_ver.as_deref().unwrap_or("?"),
            manifest["poppler_version"],
        );
    } else {
        eprintln!(
            "#634 矩阵核验 ✓ {checked_files} 文件 × {checked_pages} 探针页（poppler {} vs pdfium，分页容差=观测×1.3）",
            manifest["poppler_version"]
        );
    }
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
            missing_guard(&err, "视图链核验");
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

    // 环境门先行（缺库：PARITY=1 → fail；否则 SKIP——坏文件视图态
    // 不该空转在库错误上）。
    let probe =
        cx.update(|app| app.new(|cx| PdfPagesView::new(fixture_bytes("zh_embedded_font.pdf"), cx)));
    if let Some(err) = probe.read_with(cx, |v, _| v.error_for_test()) {
        if is_missing_library_error(&err) {
            missing_guard(&err, "健壮性核验");
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
