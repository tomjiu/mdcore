//! PDF-r 显示矩阵 —— #634 PDFium 页面正确显示专项（引擎层验收面）。
//!
//! #634 的验收问题是「PDFium 页面在产品里显示正确吗」——本文件把
//! 它拆成引擎层可自动判定的六面，全部基于**真实文件**（十类显示
//! 矩阵夹具，`fixtures/display_matrix/`，`gen_display_matrix.py`
//! 确定性生成）：
//!
//! 1. [`geometry_contract_matches_pypdf_manifest`]——十类夹具逐页
//!    几何契约：MediaBox / CropBox（含偏移原点真裁剪）/ `/Rotate`
//!    / 显示域尺寸，与 pypdf 独立解析的 manifest 逐值对照（跨引擎
//!    断言基准——不信任 pdfium 自报）；
//! 2. [`parity_vs_poppler_ncc_and_ink_iou`]——渲染像素对照：pdfium
//!    光栅 vs `pdftoppm -cropbox -gray` 同 DPI 光栅，各自 box 降采样
//!    96 宽网格后做 NCC（归一化互相关）+ 墨区 IoU 双指标（阈值分
//!    级：非嵌入 CID 字体低阈值单列——两引擎字体回退的形态漂移是
//!    已知差异源，实测校准）；
//! 3. [`rotation_orientation_pixels`]——`/Rotate 90/180` 方向特征
//!    像素断言：夹具在用户空间顶部/左缘画方向条，渲染后位置必须
//!    随旋转迁移到正确的显示边（旋转正确显示的字面证明）；
//! 4. [`multipage_render_determinism`]——120 页档两轮渲染逐字节
//!    一致 + 页间内容互异（防「恒渲同页」假绿）；
//! 5. [`cross_document_render_and_aspect`]——全部夹具渲染成功 +
//!    位图纵横比 = 显示域纵横比（±2%——显示链几何一致性的跨文档
//!    面）；
//! 6. [`tiny_extreme_widths_render_stability`]——40×720 极窄页
//!    （aspect 18）：1px / 96px / 2048px 三档宽渲染稳定（含总像素
//!    预算收缩路径），纵横比全程保持。
//! 7. [`lopdf_runtime_independent_geometry_contract`]——#51 D1 落地
//!    面：**运行时** lopdf 独立解析对照（MediaBox/CropBox//Rotate 含
//!    页树继承，与面 1 的 pypdf manifest **生成期**对照互补——不
//!    依赖 python 前置步骤、不用 pdfium 自证的第三独立面）；
//!
//! ## 环境门
//!
//! - **pdfium 库**：走 [`PdfiumEngine::open`] 定位序（env
//!   `MDC_PDFIUM_LIB` → 编译期候选清单 → 系统）；缺席 = panic 打印
//!   全部候选（同 `pdfc_engine` 环境合同——不静默 skip）；
//! - **poppler 对照**：`pdftoppm` 缺席时本面降级打印横幅跳过；
//!   设 `MDC_634_REQUIRE_PARITY=1` 转硬门（缺席即 FAIL——CI 防假绿，
//!   对齐 windows-test.yml engine 面的 poppler 安装步骤）；
//! - `MDC_634_MEASURE=1`：校准模式——打印每面实测 NCC/IoU 不判阈值
//!   （阈值标定用；正常门禁不设）。
//!
//! ## 证据落盘
//!
//! `test-artifacts/634-evidence/`：`report-{face}.json`（带运行时头：
//! pdfium 库实际路径 + SHA256（engine-lopdf 同开时）+ poppler 版本）
//! 与 `pdfium-*.pgm` / `poppler-*.pgm` 网格缩略图对（96 宽灰度）。
//! 落盘失败不判红（eprintln 留痕）——证据是副产品，门禁是断言。

// feature 门（默认构建零引擎依赖——本文件整体退化为空目标，跨环境
// 可构建纪律保持；CI 命令恒 `--features engine-lopdf,engine-pdfium`）。
#![cfg(feature = "engine-pdfium")]

use std::path::{Path, PathBuf};
use std::process::Command;

use mdc_pdf::command::CommandError;
use mdc_pdf::engine::DocumentEngine;
use mdc_pdf::engine_pdfium::PdfiumEngine;
use mdc_pdf::page::{PageIndex, Rotation};
use mdc_pdf::render::{Rasterizer, RenderRequest};
use mdc_pdf::transform::PageTransform;
use serde_json::{json, Value};

// ---------------------------------------------------------------- 定位序

/// fixtures 目录 env 覆盖位（同 `pdfc_engine` 惯例）。
const FIXTURES_ENV: &str = "MDC_PDF_TEST_FIXTURES";
/// poppler 对照硬门开关（见模块文档「环境门」）。
const PARITY_REQUIRE_ENV: &str = "MDC_634_REQUIRE_PARITY";
/// 校准模式开关（打印实测不判阈值）。
const MEASURE_ENV: &str = "MDC_634_MEASURE";

/// parity 网格宽（格）——96 在「形态可辨」与「运行时长」间平衡
/// （实测十类夹具 × 探针页全对照 < 20s）。
const GRID_W: usize = 96;
/// 超采样因子：渲染宽 = 网格宽 × 4（384px），box 降采样到 96 网格。
/// 两引擎的抗锯齿 / hinting 形态差在字形边缘 1-2px 量级——直采 96px
/// 会把该差异放大成「字形形状不同」（实测中文页 NCC 掉到 0.77）；
/// 4×4 邻域平均后差异被稀释到亚格级（网格值是几何覆盖率的函数，
/// 而非抗锯齿实现的函数）。
const GRID_SS: usize = 4;

/// 几何断言容差（pt）——pdfium `FPDFPage_GetMediaBox` 的 f32 精度兜底。
const GEOM_EPS_PT: f32 = 0.6;

fn matrix_candidates() -> Vec<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    vec![
        // crate 本地夹具（生成器产物入库位——CI 直取）。
        manifest_dir.join("fixtures/display_matrix"),
        // env 覆盖位（分析仓 / 本地开发取件位）。
        std::env::var(FIXTURES_ENV)
            .map(|d| PathBuf::from(d).join("display_matrix"))
            .unwrap_or_else(|_| manifest_dir.join("../../../display_matrix")),
    ]
}

fn matrix_dir() -> PathBuf {
    let mut tried = Vec::new();
    for dir in matrix_candidates() {
        if dir.join("manifest.json").is_file() {
            return dir;
        }
        tried.push(dir.display().to_string());
    }
    panic!(
        "display_matrix 夹具目录定位失败（需 manifest.json；生成：crates/pdf/fixtures/display_matrix/gen_display_matrix.py）；已尝试：{}",
        tried.join("；")
    );
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    let dir = matrix_dir();
    match std::fs::read(dir.join(name)) {
        Ok(bytes) => bytes,
        Err(why) => panic!("fixture {name} 读取失败（{}）：{why}", dir.display()),
    }
}

/// 打开夹具引擎（绑定失败 = panic 打印定位序——门禁可诊断，同
/// `pdfc_engine::open_small` 合同）。
fn open_engine(name: &str) -> PdfiumEngine {
    match PdfiumEngine::open(fixture_bytes(name)) {
        Ok(engine) => engine,
        Err(CommandError::Engine(detail)) => panic!(
            "pdfium 绑定/打开失败（{name}；库定位序：MDC_PDFIUM_LIB → 候选 {} → 系统）：{detail}",
            PdfiumEngine::library_candidates()
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(" → ")
        ),
        Err(other) => panic!("打开 {name} 应成功或 Engine 错误，得 {other:?}"),
    }
}

fn manifest_fixtures() -> Vec<Value> {
    let raw = std::fs::read_to_string(matrix_dir().join("manifest.json"))
        .expect("manifest.json 可读（生成器同目录产出）");
    let v: Value = serde_json::from_str(&raw).expect("manifest.json 合法 JSON");
    v["fixtures"]
        .as_array()
        .expect("manifest.fixtures 数组")
        .clone()
}

// ---------------------------------------------------------------- 像素工具

/// 灰度位图（渲染 / PGM 解析的统一载体）。
struct Gray {
    w: usize,
    h: usize,
    data: Vec<u8>,
}

/// pdfium 渲染单页 → 灰度（BGRA → luma 601）。
fn render_gray(engine: &mut PdfiumEngine, page0: u32, width_px: u32) -> Gray {
    let page = engine
        .rasterize(RenderRequest {
            page: PageIndex(page0),
            width_px,
        })
        .unwrap_or_else(|e| panic!("rasterize(page {page0}, {width_px}px) 应成功：{e:?}"));
    let (w, h) = (page.width_px as usize, page.height_px as usize);
    assert_eq!(
        page.bgra.len(),
        w * h * 4,
        "位图缓冲合同（w*h*4）——engine_pdfium 语义锁"
    );
    let mut data = vec![0u8; w * h];
    for (dst, px) in data.iter_mut().zip(page.bgra.as_chunks::<4>().0) {
        // BGRA 字节序：[B, G, R, A]——Rec. 601 luma。
        *dst = ((px[2] as u32 * 299 + px[1] as u32 * 587 + px[0] as u32 * 114) / 1000) as u8;
    }
    Gray { w, h, data }
}

/// 解析 P5 PGM（pdftoppm 输出）。
///
/// 头部**四字段**（`P5` / 宽 / 高 / maxval——`-gray` 恒 255），字段间
/// 任意空白、`#` 到行尾是注释；maxval 后恰一个空白字节即像素数据。
fn parse_pgm(bytes: &[u8]) -> Option<Gray> {
    let mut tokens: Vec<String> = Vec::with_capacity(4);
    let mut token = String::new();
    let mut pos = 0usize;
    while tokens.len() < 4 {
        let b = *bytes.get(pos)?;
        if b == b'#' {
            while pos < bytes.len() && bytes[pos] != b'\n' {
                pos += 1;
            }
        } else if b.is_ascii_whitespace() {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
            pos += 1;
        } else {
            token.push(b as char);
            pos += 1;
        }
    }
    // 第 4 token 的终止空白已在循环内消费——pos 即像素数据起点
    // （P5 规定 maxval 后恰一个空白分隔符；pdftoppm 实测一致）。
    if tokens[0] != "P5" {
        return None;
    }
    let (w, h, maxval): (usize, usize, u32) = (
        tokens[1].parse().ok()?,
        tokens[2].parse().ok()?,
        tokens[3].parse().ok()?,
    );
    if maxval != 255 || w == 0 || h == 0 {
        return None;
    }
    let data = bytes.get(pos..)?.to_vec();
    if data.len() < w * h {
        return None;
    }
    Some(Gray {
        w,
        h,
        data: data[..w * h].to_vec(),
    })
}

/// pdftoppm 版本串（缺席 = None——调用方决定降级或硬门）。
fn poppler_version() -> Option<String> {
    let out = Command::new("pdftoppm")
        .arg("-v")
        .output()
        .ok()
        .filter(|o| o.status.success() || !o.stderr.is_empty())?;
    let raw = String::from_utf8_lossy(if out.stderr.is_empty() {
        &out.stdout
    } else {
        &out.stderr
    })
    .lines()
    .next()?
    .trim()
    .to_string();
    if raw.is_empty() {
        None
    } else {
        Some(raw)
    }
}

/// pdftoppm 渲染单页（`-cropbox -gray`——显示域口径与 pdfium 对齐：
/// poppler CLI 默认 MediaBox，pdfium 默认 CropBox，必须显式对齐）。
fn pdftoppm_gray(pdf: &Path, page1: u32, dpi: f64, tag: &str) -> Option<Gray> {
    let out_dir = std::env::temp_dir().join("mdc634-parity");
    std::fs::create_dir_all(&out_dir).ok()?;
    // 清同 tag 旧产物（重跑 / 页码零填充变体——`tag` 必须全局唯一：
    // 调用方以「夹具名-页号」构造，不同夹具同页号不得共用临时名）。
    if let Ok(entries) = std::fs::read_dir(&out_dir) {
        for e in entries.flatten() {
            if let Some(n) = e.file_name().to_str() {
                if n.starts_with(tag) && n.ends_with(".pgm") {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
    let prefix = out_dir.join(tag);
    let status = Command::new("pdftoppm")
        .arg("-cropbox")
        .arg("-gray")
        .arg("-r")
        .arg(format!("{dpi:.4}"))
        .arg("-f")
        .arg(page1.to_string())
        .arg("-l")
        .arg(page1.to_string())
        .arg(pdf)
        .arg(&prefix)
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    // 输出名带页码零填充（页数 > 9 / > 99 的档 pdftoppm 用 %02d / %03d
    // ——120 页档单页渲染输出 prefix-001.pgm）：扫描 tag 前缀取首个
    // 命中（单页渲染只产一个文件）。
    let entries = std::fs::read_dir(&out_dir).ok()?;
    let mut hits: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with(&format!("{tag}-")) && n.ends_with(".pgm"))
                .unwrap_or(false)
        })
        .collect();
    hits.sort();
    let path = hits.into_iter().next()?;
    let bytes = std::fs::read(path).ok()?;
    parse_pgm(&bytes)
}

/// box 降采样到 `gw × gh` 网格（行主序；两引擎位图尺寸 ±1px 差异的
/// 归一基准——按显示域 pt 比例定网格，各自独立降采样）。
fn box_grid(g: &Gray, gw: usize, gh: usize) -> Vec<f32> {
    assert!(gw > 0 && gh > 0);
    let (sw, sh) = (g.w, g.h);
    let mut out = vec![0f32; gw * gh];
    for gy in 0..gh {
        let y0 = gy * sh / gh;
        let y1 = (((gy + 1) * sh / gh).max(y0 + 1)).min(sh);
        for gx in 0..gw {
            let x0 = gx * sw / gw;
            let x1 = (((gx + 1) * sw / gw).max(x0 + 1)).min(sw);
            let mut sum = 0u64;
            for y in y0..y1 {
                let row = y * sw;
                for x in x0..x1 {
                    sum += g.data[row + x] as u64;
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as f32;
            out[gy * gw + gx] = sum as f32 / n;
        }
    }
    out
}

/// 归一化互相关（[-1, 1]——形态一致性；1 = 逐格线性一致）。
fn ncc(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    let n = a.len() as f32;
    let (ma, mb) = (a.iter().sum::<f32>() / n, b.iter().sum::<f32>() / n);
    let mut num = 0f32;
    let mut da = 0f32;
    let mut db = 0f32;
    for (&x, &y) in a.iter().zip(b.iter()) {
        let (x, y) = (x - ma, y - mb);
        num += x * y;
        da += x * x;
        db += y * y;
    }
    num / (da.sqrt() * db.sqrt()).max(1e-9)
}

/// 墨量 IoU（**软墨量**交并）：格内墨覆盖率 = (255 - 灰度)/255，交 =
/// 逐格 min，并 = 逐格 max。硬阈值判定的边缘格（抗锯齿灰度在阈值
/// 附近）会因两引擎 ±10 灰度差翻转整格墨/非墨——实测中文页 IoU 掉到
/// 0.63；软墨量把边缘格的部分墨量平滑计入，是阈值不敏感的布局锚
/// （NCC 之外的墨迹落点一致性而二指标）。
fn ink_iou(a: &[f32], b: &[f32]) -> f32 {
    let (mut inter, mut union) = (0f32, 0f32);
    for (&x, &y) in a.iter().zip(b.iter()) {
        let (ia, ib) = ((255.0 - x) / 255.0, (255.0 - y) / 255.0);
        inter += ia.min(ib);
        union += ia.max(ib);
    }
    if union < 1e-6 {
        1.0 // 双白页——墨区语义上完全一致
    } else {
        inter / union
    }
}

/// 网格 → P5 PGM（证据缩略图）。
fn write_grid_pgm(path: &Path, grid: &[f32], gw: usize, gh: usize) {
    let mut out = format!("P5\n{gw} {gh}\n255\n").into_bytes();
    out.extend(grid.iter().map(|&v| v.clamp(0.0, 255.0) as u8));
    if let Err(why) = std::fs::write(path, out) {
        eprintln!("证据 PGM 写失败（{}）：{why}", path.display());
    }
}

/// 侧边带均值（0..=255）——方向断言的观测面。`frac` = 带宽占比。
fn band_mean(g: &Gray, side: &str, frac: f32) -> f32 {
    let bw = ((g.w as f32 * frac).ceil() as usize).clamp(1, g.w);
    let bh = ((g.h as f32 * frac).ceil() as usize).clamp(1, g.h);
    let (x0, x1, y0, y1) = match side {
        "left" => (0, bw, 0, g.h),
        "right" => (g.w - bw, g.w, 0, g.h),
        "top" => (0, g.w, 0, bh),
        "bottom" => (0, g.w, g.h - bh, g.h),
        _ => unreachable!("side ∈ left/right/top/bottom"),
    };
    let mut sum = 0u64;
    for y in y0..y1 {
        let row = y * g.w;
        for x in x0..x1 {
            sum += g.data[row + x] as u64;
        }
    }
    sum as f32 / (((x1 - x0) * (y1 - y0)) as f32)
}

// ---------------------------------------------------------------- 证据落盘

fn evidence_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-artifacts/634-evidence")
}

/// pdfium 库 SHA256（engine-lopdf 同开时可用——CI 双特性面全量；
/// 单 engine-pdfium 面报告 None 不阻断）。
#[cfg(feature = "engine-lopdf")]
fn sha256_hex(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).ok()?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Some(
        h.finalize()
            .into_iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    )
}

#[cfg(not(feature = "engine-lopdf"))]
fn sha256_hex(_path: &Path) -> Option<String> {
    None
}

/// 运行时头（报告溯源：pdfium 库实路径 + SHA256 + poppler 版本）。
fn runtime_head(poppler: Option<&str>) -> Value {
    let lib = PdfiumEngine::library_candidate_roles()
        .into_iter()
        .find(|(_, p)| p.exists())
        .map(|(role, p)| {
            json!({
                "role": role,
                "path": p.display().to_string(),
                "sha256": sha256_hex(&p),
                "provenance": "chromium/8086（CI ci-deps-v1 同轨铆定）",
            })
        });
    json!({
        "pdfium_lib": lib,
        "mdc_pdfium_lib_env": std::env::var("MDC_PDFIUM_LIB").ok(),
        "poppler": poppler,
    })
}

fn write_report(name: &str, value: Value) {
    let dir = evidence_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        eprintln!("证据目录创建失败（{}）", dir.display());
        return;
    }
    if let Err(why) = std::fs::write(
        dir.join(name),
        serde_json::to_string_pretty(&value).unwrap_or_default() + "\n",
    ) {
        eprintln!("证据报告写失败（{name}）：{why}");
    }
}

// ---------------------------------------------------------------- 测试面 1

/// **面 1 · 几何契约**：十类夹具逐页 MediaBox / CropBox（偏移原点真
/// 裁剪）/ `/Rotate` / 显示域尺寸，对照 pypdf 独立解析的 manifest。
///
/// 这里的「正确显示」= 显示链的几何事实单源：`PageGeometry` 的盒与
/// 旋转、`PageTransform::display_size` 的显示域（CropBox 优先 + 旋转
/// 交换），与独立解析器一致——后续所有像素面的几何前提。
#[test]
fn geometry_contract_matches_pypdf_manifest() {
    let fixtures = manifest_fixtures();
    assert!(fixtures.len() >= 10, "十类显示矩阵夹具在位");
    let mut report = Vec::new();
    for fx in &fixtures {
        let file = fx["file"].as_str().unwrap();
        let engine = open_engine(file); // 几何查询 &self——无渲染无需 mut
        let pages = fx["pages"].as_array().unwrap();
        let count = engine.page_count();
        assert_eq!(
            count as usize,
            pages.len(),
            "{file}：页数与 manifest 不符（pdfium {count} vs pypdf {}）",
            pages.len()
        );
        for (i, spec) in pages.iter().enumerate() {
            let geo = engine
                .page_geometry(PageIndex(i as u32))
                .unwrap_or_else(|e| panic!("{file} 页 {}: 几何查询失败 {e:?}", i + 1));
            let t = PageTransform::from_geometry(&geo)
                .unwrap_or_else(|e| panic!("{file} 页 {}: 变换构造失败 {e:?}", i + 1));

            // MediaBox 四值。
            let media: Vec<f64> = spec["media_box"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect();
            for (got, want, axis) in [
                (geo.media_box.x0, media[0], "x0"),
                (geo.media_box.y0, media[1], "y0"),
                (geo.media_box.x1, media[2], "x1"),
                (geo.media_box.y1, media[3], "y1"),
            ] {
                assert!(
                    (got - want as f32).abs() <= GEOM_EPS_PT,
                    "{file} 页 {} MediaBox.{axis}：pdfium {got} vs pypdf {want}",
                    i + 1
                );
            }

            // CropBox（None 语义 = 页无 /CropBox——两边必须同形）。
            match (geo.crop_box, spec["crop_box"].as_array()) {
                (None, None) => {}
                (Some(c), Some(want)) => {
                    let want: Vec<f64> = want.iter().map(|v| v.as_f64().unwrap()).collect();
                    let got = [c.x0, c.y0, c.x1, c.y1];
                    for (g, w) in got.iter().zip(want.iter()) {
                        assert!(
                            (g - *w as f32).abs() <= GEOM_EPS_PT,
                            "{file} 页 {} CropBox：pdfium {got:?} vs pypdf {want:?}",
                            i + 1
                        );
                    }
                }
                (a, b) => panic!(
                    "{file} 页 {} CropBox 形态不符：pdfium {a:?} vs pypdf {b:?}",
                    i + 1
                ),
            }

            // /Rotate。
            let want_rot = spec["rotate"].as_u64().unwrap();
            let got_rot: u32 = match geo.rotation {
                Rotation::Deg0 => 0,
                Rotation::Deg90 => 90,
                Rotation::Deg180 => 180,
                Rotation::Deg270 => 270,
            };
            assert_eq!(got_rot, want_rot as u32, "{file} 页 {} /Rotate", i + 1);

            // 显示域（CropBox 优先 + 旋转交换——渲染位图的期望宽高）。
            let ds = t.display_size();
            let want: Vec<f64> = spec["display_size_pt"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect();
            assert!(
                (ds.width - want[0] as f32).abs() <= GEOM_EPS_PT
                    && (ds.height - want[1] as f32).abs() <= GEOM_EPS_PT,
                "{file} 页 {} 显示域：pdfium {:.2}×{:.2} vs pypdf {:.2}×{:.2}",
                i + 1,
                ds.width,
                ds.height,
                want[0],
                want[1]
            );
            report.push(json!({
                "file": file, "page": i + 1,
                "media_box": [geo.media_box.x0, geo.media_box.y0, geo.media_box.x1, geo.media_box.y1],
                "crop_box": geo.crop_box.map(|c| [c.x0, c.y0, c.x1, c.y1]),
                "rotate": got_rot,
                "display_size_pt": [ds.width, ds.height],
            }));
        }
    }
    write_report(
        "report-geometry.json",
        json!({"face": "geometry_contract", "head": runtime_head(None), "pages": report}),
    );
}

// ---------------------------------------------------------------- 测试面 2

/// **面 2 · 渲染像素对照（parity）**：pdfium vs poppler（pdftoppm
/// `-cropbox -gray` 同 DPI）——96 宽网格 NCC + 墨区 IoU。
///
/// 阈值分级：非嵌入 CID 字体（chinese_missing_font）0.80/0.60 低阈值
/// 单列（两引擎字体回退的形态漂移是已知差异源）；其余 0.90/0.72
/// （实测基线最差留 >5% 裕量）。
#[test]
fn parity_vs_poppler_ncc_and_ink_iou() {
    let measure = std::env::var(MEASURE_ENV).ok().as_deref() == Some("1");
    let Some(poppler) = poppler_version() else {
        if std::env::var(PARITY_REQUIRE_ENV).ok().as_deref() == Some("1") {
            panic!(
                "{PARITY_REQUIRE_ENV}=1：pdftoppm 缺席——parity 硬门 FAIL \
                 （安装 poppler-utils，或确认放弃像素对照后解除该 env）"
            );
        }
        eprintln!(
            "=== SKIP parity_vs_poppler（pdftoppm 缺席；设 {PARITY_REQUIRE_ENV}=1 转硬门）==="
        );
        return;
    };
    let evidence = evidence_dir();
    let _ = std::fs::create_dir_all(&evidence);
    let mut rows = Vec::new();
    for fx in &fixtures_all() {
        let file = fx["file"].as_str().unwrap();
        let ncc_min = fx["parity"]["ncc_min"].as_f64().unwrap() as f32;
        let iou_min = fx["parity"]["ink_iou_min"].as_f64().unwrap() as f32;
        let mut engine = open_engine(file);
        let pdf_path = matrix_dir().join(file);
        for p1 in fx["probe_pages"].as_array().unwrap() {
            let p1 = p1.as_u64().unwrap() as u32;
            let spec = &fx["pages"].as_array().unwrap()[(p1 - 1) as usize];
            let dw = spec["display_size_pt"][0].as_f64().unwrap();
            let dh = spec["display_size_pt"][1].as_f64().unwrap();
            let raster_w = (GRID_W * GRID_SS) as f64;
            let dpi = raster_w / dw * 72.0;
            let gh = ((GRID_W as f64 * dh / dw).round() as usize).max(1);

            let a = render_gray(&mut engine, p1 - 1, raster_w as u32);
            // 临时名全局唯一（夹具名+页号——不同夹具同页号不共用）。
            let stem = file.trim_end_matches(".pdf");
            let b = pdftoppm_gray(&pdf_path, p1, dpi, &format!("{stem}-p{p1}"))
                .unwrap_or_else(|| panic!("{file} 页 {p1}：pdftoppm 渲染失败（dpi {dpi:.2}）"));
            let (ga, gb) = (box_grid(&a, GRID_W, gh), box_grid(&b, GRID_W, gh));
            let (n, i) = (ncc(&ga, &gb), ink_iou(&ga, &gb));

            // 证据缩略图对（网格灰度 PGM）。
            write_grid_pgm(
                &evidence.join(format!("pdfium-{file}-p{p1}.pgm")),
                &ga,
                GRID_W,
                gh,
            );
            write_grid_pgm(
                &evidence.join(format!("poppler-{file}-p{p1}.pgm")),
                &gb,
                GRID_W,
                gh,
            );
            rows.push(json!({
                "file": file, "page": p1, "dpi": (dpi * 100.0).round() / 100.0,
                "grid": [GRID_W, gh],
                "supersample": GRID_SS,
                "ncc": (n * 10000.0).round() / 10000.0,
                "ink_iou": (i * 10000.0).round() / 10000.0,
                "thresholds": {"ncc_min": ncc_min, "ink_iou_min": iou_min},
                "pdfium_bitmap": [a.w, a.h], "poppler_bitmap": [b.w, b.h],
            }));
            if measure {
                println!(
                    "[MEASURE] {file} p{p1}: NCC={n:.4} IoU={i:.4}（位图 {}×{} vs {}×{}）",
                    a.w, a.h, b.w, b.h
                );
                continue;
            }
            assert!(
                n >= ncc_min,
                "{file} 页 {p1} NCC {n:.4} < {ncc_min}——pdfium 与 poppler 像素形态失配",
            );
            assert!(
                i >= iou_min,
                "{file} 页 {p1} 墨区 IoU {i:.4} < {iou_min}——墨迹落点失配",
            );
        }
    }
    write_report(
        "report-parity.json",
        json!({
            "face": "parity_vs_poppler",
            "head": runtime_head(Some(&poppler)),
            "measure_mode": measure,
            "rows": rows,
        }),
    );
}

fn fixtures_all() -> Vec<Value> {
    manifest_fixtures()
}

// ---------------------------------------------------------------- 测试面 3

/// **面 3 · 旋转方向像素断言**：`/Rotate 90`（页 1）与 `/Rotate 180`
/// （页 2）——夹具在用户空间画方向条（顶部粗黑条 + 左缘竖条），渲染
/// 后必须出现在旋转语义要求的显示边。
///
/// 推导（Acrobat 同款 `/Rotate` 语义——顺时针旋转显示）：
/// - R90：用户顶部（+y）→ 显示右边；用户左缘（-x）→ 显示上边；
/// - R180：用户顶部 → 显示下边；用户左缘 → 显示右边。
#[test]
fn rotation_orientation_pixels() {
    const BAND: f32 = 0.08; // 侧带宽 8%
    const DARK_DELTA: f32 = 30.0; // 带均值差下限（实测 67-101，裕量充足）
    let mut engine = open_engine("rotation_matrix.pdf");

    // 页 1（/Rotate 90——显示域 841.9×595.3 横）。
    let g = render_gray(&mut engine, 0, 240);
    assert!(g.w > g.h, "R90 页显示域应为横向（得 {}×{}）", g.w, g.h);
    let (l, r, t, b) = (
        band_mean(&g, "left", BAND),
        band_mean(&g, "right", BAND),
        band_mean(&g, "top", BAND),
        band_mean(&g, "bottom", BAND),
    );
    assert!(
        r < l - DARK_DELTA,
        "R90：用户顶部黑条应落在显示右边（右带 {r:.0} 应显著暗于左带 {l:.0}）"
    );
    assert!(
        t < b - DARK_DELTA,
        "R90：用户左缘竖条应落在显示上边（上带 {t:.0} 应显著暗于下带 {b:.0}）"
    );

    // 页 2（/Rotate 180——显示域 595.3×841.9 纵）。
    let g = render_gray(&mut engine, 1, 240);
    assert!(g.h > g.w, "R180 页显示域应为纵向（得 {}×{}）", g.w, g.h);
    let (l, r, t, b) = (
        band_mean(&g, "left", BAND),
        band_mean(&g, "right", BAND),
        band_mean(&g, "top", BAND),
        band_mean(&g, "bottom", BAND),
    );
    assert!(
        b < t - DARK_DELTA,
        "R180：用户顶部黑条应落在显示下边（下带 {b:.0} 应显著暗于上带 {t:.0}）"
    );
    assert!(
        r < l - DARK_DELTA,
        "R180：用户左缘竖条应落在显示右边（右带 {r:.0} 应显著暗于左带 {l:.0}）"
    );

    write_report(
        "report-rotation.json",
        json!({
            "face": "rotation_orientation_pixels",
            "head": runtime_head(None),
            "fixtures": "rotation_matrix.pdf（/Rotate 90 + /Rotate 180）",
            "probes": {
                "R90": {"display": [841.9, 595.3], "asserts": ["right_dark", "top_dark"]},
                "R180": {"display": [595.3, 841.9], "asserts": ["bottom_dark", "right_dark"]},
            },
        }),
    );
}

// ---------------------------------------------------------------- 测试面 4

/// **面 4 · 多页确定性**：120 页档页 1/60/120 两轮渲染逐字节一致；
/// 页间内容互异（生成器页号比例条——防「恒渲同页」假绿）。
#[test]
fn multipage_render_determinism() {
    let mut engine = open_engine("multipage_120.pdf");
    assert_eq!(engine.page_count(), 120, "120 页档");
    let probes = [0u32, 59, 119];
    let mut first = Vec::new();
    for &p in &probes {
        let page = engine
            .rasterize(RenderRequest {
                page: PageIndex(p),
                width_px: 96,
            })
            .unwrap_or_else(|e| panic!("页 {} 渲染失败：{e:?}", p + 1));
        first.push(page);
    }
    for (round, &p) in probes.iter().enumerate() {
        let again = engine
            .rasterize(RenderRequest {
                page: PageIndex(p),
                width_px: 96,
            })
            .unwrap_or_else(|e| panic!("第 2 轮页 {} 渲染失败：{e:?}", p + 1));
        assert_eq!(
            first[round],
            again,
            "页 {} 两轮渲染字节不一致——多页渲染非确定",
            p + 1
        );
    }
    assert_ne!(
        first[0], first[1],
        "页 1 与页 60 位图应互异（页号比例条内容不同）"
    );
    assert_ne!(first[1], first[2], "页 60 与页 120 位图应互异");
    write_report(
        "report-multipage.json",
        json!({
            "face": "multipage_render_determinism",
            "head": runtime_head(None),
            "pages": 120, "probe_pages_1based": [1, 60, 120],
            "deterministic": true, "pages_distinct": true,
        }),
    );
}

// ---------------------------------------------------------------- 测试面 5

/// **面 5 · 跨文档渲染与纵横比**：全部夹具 × 探针页渲染成功，位图
/// 纵横比 = 显示域纵横比（±2%——显示链几何一致性的跨文档面； CropBox
/// 裁剪 / 旋转交换 / 极窄页全覆盖）。
#[test]
fn cross_document_render_and_aspect() {
    let mut rows = Vec::new();
    for fx in &fixtures_all() {
        let file = fx["file"].as_str().unwrap();
        let mut engine = open_engine(file);
        for p1 in fx["probe_pages"].as_array().unwrap() {
            let p1 = p1.as_u64().unwrap() as u32;
            let spec = &fx["pages"].as_array().unwrap()[(p1 - 1) as usize];
            let dw = spec["display_size_pt"][0].as_f64().unwrap();
            let dh = spec["display_size_pt"][1].as_f64().unwrap();
            let g = render_gray(&mut engine, p1 - 1, 120);
            let want = dh / dw;
            let got = g.h as f64 / g.w as f64;
            assert!(
                (got - want).abs() / want < 0.02,
                "{file} 页 {p1}：位图纵横比 {got:.4} 与显示域 {want:.4} 偏差 > 2%"
            );
            rows.push(json!({
                "file": file, "page": p1,
                "bitmap": [g.w, g.h], "display_size_pt": [dw, dh],
                "aspect_delta_pct": ((got - want).abs() / want * 100.0 * 100.0).round() / 100.0,
            }));
        }
    }
    write_report(
        "report-crossdoc.json",
        json!({"face": "cross_document_render_and_aspect", "head": runtime_head(None), "rows": rows}),
    );
}

// ---------------------------------------------------------------- 测试面 6

/// **面 6 · 极端宽渲染稳定**：40×720 极窄页（aspect 18）四档宽：
/// 1px（下限钳制）/ 96px（parity 常规档）/ 400px（高 7200——不触
/// 钳制的最大常规档）/ 2048px（高 36864 触发 S3-L2 单边 8192 钳制）
/// ——前三档纵横比保持 ±2%；2048 档按钳制语义如实断言（8192 上限）。
#[test]
fn tiny_extreme_widths_render_stability() {
    let mut engine = open_engine("tiny_note.pdf");
    let want = 720.0f64 / 40.0; // 18.0
    for width in [1u32, 96, 400] {
        let page = engine
            .rasterize(RenderRequest {
                page: PageIndex(0),
                width_px: width,
            })
            .unwrap_or_else(|e| panic!("tiny_note {width}px 渲染失败：{e:?}"));
        let got = page.height_px as f64 / page.width_px as f64;
        assert!(
            (got - want).abs() / want < 0.02,
            "tiny_note {width}px 档：纵横比 {got:.4} ≠ {want}（±2%）"
        );
    }
    // 2048px 档：等比高 36864 超单边 8192 上限——S3-L2 钳制语义
    // （单边收敛不拒绝）：高收敛到 8192，宽保持请求值。
    let clamped = engine
        .rasterize(RenderRequest {
            page: PageIndex(0),
            width_px: 2048,
        })
        .unwrap_or_else(|e| panic!("tiny_note 2048px 渲染失败：{e:?}"));
    assert_eq!(
        (clamped.width_px, clamped.height_px),
        (2048, 8192),
        "2048px 档应触发单边 8192 钳制（等比高 36864 → 8192）"
    );
    write_report(
        "report-edges.json",
        json!({
            "face": "tiny_extreme_widths_render_stability",
            "head": runtime_head(None),
            "fixture": "tiny_note.pdf（40×720pt，aspect 18）",
            "widths_px": [1, 96, 2048],
            "note": "2048px 档触发 S3-L2 总像素预算等比收缩（收缩后纵横比保持）",
        }),
    );
}

// ---------------------------------------------------------------- 测试面 7

/// lopdf `Object` → f64（Integer/Real 双格式；其他形态 = None——
/// 对照器不做猜测）。
fn lopdf_obj_as_f64(obj: &lopdf::Object) -> Option<f64> {
    match obj {
        lopdf::Object::Integer(v) => Some(*v as f64),
        lopdf::Object::Real(v) => Some(f64::from(*v)),
        _ => None,
    }
}

/// 间接引用解到实对象（Reference → `get_object`；其余原样）。
fn lopdf_derefed<'a>(
    doc: &'a lopdf::Document,
    obj: &'a lopdf::Object,
) -> Option<&'a lopdf::Object> {
    match obj {
        lopdf::Object::Reference(id) => doc.get_object(*id).ok(),
        other => Some(other),
    }
}

/// 矩形 Object → (x0,y0,x1,y1)。
fn lopdf_obj_as_rect(obj: &lopdf::Object) -> Option<(f64, f64, f64, f64)> {
    let arr = obj.as_array().ok()?;
    if arr.len() != 4 {
        return None;
    }
    Some((
        lopdf_obj_as_f64(&arr[0])?,
        lopdf_obj_as_f64(&arr[1])?,
        lopdf_obj_as_f64(&arr[2])?,
        lopdf_obj_as_f64(&arr[3])?,
    ))
}

/// 页字典键沿 /Parent 链继承查找（spec 7.7.3.3：MediaBox/CropBox/
/// Rotate 可继承）。
fn lopdf_inherited<'a>(
    doc: &'a lopdf::Document,
    dict: &'a lopdf::Dictionary,
    key: &[u8],
) -> Option<&'a lopdf::Object> {
    let mut cur = dict;
    loop {
        // lopdf 0.46：`Dictionary::get` → Result<&Object, Error>。
        if let Ok(found) = cur.get(key) {
            return lopdf_derefed(doc, found);
        }
        let parent_id = cur.get(b"Parent").ok()?;
        let parent = lopdf_derefed(doc, parent_id)?;
        cur = parent.as_dict().ok()?;
    }
}

/// 矩形四元组（lopdf 独立解析的返回形态——type 别名化避免签名
/// clippy::type_complexity）。
type Rect4 = (f64, f64, f64, f64);

/// lopdf 独立解析单页几何三元组（media/crop/rotate——渲染显示域由
/// 调用方按 90/270 交换换算；与面 1 的 pypdf manifest 对照互补）。
fn lopdf_page_rects(bytes: &[u8], page0: u32) -> Option<(Rect4, Option<Rect4>, i64)> {
    let doc = lopdf::Document::load_mem(bytes).ok()?;
    let page_id = doc.get_pages().get(&(page0 + 1))?.to_owned();
    let page = doc.get_object(page_id).ok()?;
    let dict = page.as_dict().ok()?;
    let media = lopdf_inherited(&doc, dict, b"MediaBox")
        .and_then(lopdf_obj_as_rect)
        .unwrap_or((0., 0., 612., 792.)); // spec 缺省 Letter——仅回退口径
    let crop = lopdf_inherited(&doc, dict, b"CropBox")
        .and_then(lopdf_obj_as_rect)
        .filter(|c| {
            // 恒等形态（crop ≡ media，容差 0.5pt）归 None——与引擎
            // `engine_pdfium.rs` 的归档语义同款（「真裁剪」vs「缺省」）。
            (c.0 - media.0).abs() > 0.5
                || (c.1 - media.1).abs() > 0.5
                || (c.2 - media.2).abs() > 0.5
                || (c.3 - media.3).abs() > 0.5
        });
    let rotate = lopdf_inherited(&doc, dict, b"Rotate")
        .and_then(|o| match o {
            lopdf::Object::Integer(v) => Some(*v),
            _ => None,
        })
        .unwrap_or(0);
    Some((media, crop, rotate))
}

/// **面 7 · lopdf 运行时独立几何对照**（#51 D1 落地面——与面 1 的
/// pypdf manifest 互补）：面 1 的断言基准是**生成期**对照（pypdf 在
/// 语料构建时解析、冻结进 manifest——改文件后基准不跟随）；本面在
/// **运行时**用 lopdf（与 pdfium 互不相干的 PDF 对象语义实现）直接
/// 解析夹具字节，覆盖十类夹具逐页 MediaBox/CropBox（含继承与真裁剪
/// 归档语义）//Rotate 枚举与渲染显示域换算——不用 pdfium 自证、
/// 不依赖 python 前置步骤（门禁环境可复现的第三独立面）。
#[test]
fn lopdf_runtime_independent_geometry_contract() {
    let fixtures = manifest_fixtures();
    assert!(fixtures.len() >= 10, "十类显示矩阵夹具在位");
    let mut checked = 0usize;
    for fx in &fixtures {
        let file = fx["file"].as_str().unwrap();
        let bytes = fixture_bytes(file);
        let engine = open_engine(file);
        for i in 0..engine.page_count() {
            let Some((media, crop, rotate)) = lopdf_page_rects(&bytes, i) else {
                panic!(
                    "{file} 页 {}：lopdf 独立解析失败（完好夹具必须可解析）",
                    i + 1
                );
            };
            let geo = engine
                .page_geometry(PageIndex(i))
                .unwrap_or_else(|e| panic!("{file} 页 {}: 几何查询失败 {e:?}", i + 1));

            // MediaBox 四值（±0.6pt——数值格式差在亚像素级）。
            let got_media = [
                geo.media_box.x0,
                geo.media_box.y0,
                geo.media_box.x1,
                geo.media_box.y1,
            ];
            let want_media = [media.0, media.1, media.2, media.3];
            for (g, w) in got_media.iter().zip(want_media) {
                assert!(
                    (g - w as f32).abs() <= GEOM_EPS_PT,
                    "{file} 页 {} MediaBox：pdfium {g} vs lopdf {w}",
                    i + 1
                );
            }

            // CropBox（None 语义同面 1：真裁剪 vs 缺省必须同形）。
            match (geo.crop_box, crop) {
                (None, None) => {}
                (Some(c), Some(w)) => {
                    let got = [c.x0, c.y0, c.x1, c.y1];
                    for (g, w) in got.iter().zip([w.0, w.1, w.2, w.3]) {
                        assert!(
                            (g - w as f32).abs() <= GEOM_EPS_PT,
                            "{file} 页 {} CropBox：pdfium {got:?} vs lopdf {w:?}",
                            i + 1
                        );
                    }
                }
                (a, b) => panic!(
                    "{file} 页 {} CropBox 形态不符：pdfium {a:?} vs lopdf {b:?}",
                    i + 1
                ),
            }

            // /Rotate 枚举。
            let got_rot: u32 = match geo.rotation {
                Rotation::Deg0 => 0,
                Rotation::Deg90 => 90,
                Rotation::Deg180 => 180,
                Rotation::Deg270 => 270,
            };
            assert_eq!(
                got_rot,
                rotate as u32,
                "{file} 页 {} /Rotate：pdfium {got_rot} vs lopdf {rotate}",
                i + 1
            );

            // 渲染显示域（CropBox 优先 + 90/270 交换——与面 1 的
            // display_size 同义）。
            let disp = crop.unwrap_or(media);
            let (rw, rh) = (disp.2 - disp.0, disp.3 - disp.1);
            let (want_w, want_h) = if rotate % 180 == 90 {
                (rh, rw)
            } else {
                (rw, rh)
            };
            let t = PageTransform::from_geometry(&geo)
                .unwrap_or_else(|e| panic!("{file} 页 {}: 变换构造失败 {e:?}", i + 1));
            let ds = t.display_size();
            let (got_w, got_h) = (ds.width, ds.height);
            assert!(
                (got_w - want_w as f32).abs() <= GEOM_EPS_PT
                    && (got_h - want_h as f32).abs() <= GEOM_EPS_PT,
                "{file} 页 {} 显示域：pdfium {got_w}×{got_h} vs lopdf {want_w}×{want_h}",
                i + 1
            );
            checked += 1;
        }
        eprintln!(
            "#634 面7 lopdf 独立对照 ✓ {file}（{} 页）",
            engine.page_count()
        );
    }
    assert!(checked >= 10, "lopdf 对照覆盖面坍缩（{checked} 页）");
    write_report(
        "report-lopdf-face.json",
        json!({
            "face": "lopdf_runtime_independent_geometry_contract",
            "head": runtime_head(None),
            "pages_checked": checked,
            "note": "运行时 lopdf 独立解析（与 pypdf manifest 生成期对照互补——无 python 依赖的第三独立面）",
        }),
    );
}
