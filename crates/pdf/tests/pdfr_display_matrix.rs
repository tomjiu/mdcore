//! #634 D1（#51 工作线 D）——PDFium 引擎层**内容×几何显示矩阵**
//! （feature `engine-pdfium` + `engine-lopdf`）。
//!
//! # 定位（D1 收口的三方对照之引擎层）
//!
//! mang-shell 层 [`pdf_display_verify`]（活参照 pdftoppm + pypdf manifest）
//! 回答「显示链与独立阅读器像不像」；本文件回答**引擎几何/旋转/渲染
//! 尺寸链自身与第三方解析器（lopdf——与 pdfium 互不相干的 PDF 对象
//! 语义实现）是否一致**：
//!
//! 1. `page_geometry` 显示域宽高 vs lopdf 独立解析（MediaBox/CropBox/
//!    /Rotate，含页树继承——spec 7.7.3.3）——**不用 pdfium 自证 pdfium**；
//! 2. 旋转标记（`/Rotate` → `Rotation` 枚举）与 lopdf 逐页一致；
//! 3. 渲染位图尺寸与显示域纵横比一致（640px 宽请求 → 高 = 显示域
//!    高/宽 × 640，ceil/round 取整差 ±1px 口径带）；
//! 4. 坏文件 typed 拒绝（与 manifest 三方对照登记一致——lopdf 的宽容/
//!    拒绝行为不入断言：宽松解析器对损坏文件可开可拒，属三方对照
//!    观测面，不是引擎语义）。
//!
//! # 口径与阈值（D1：不放宽、不删除断言）
//!
//! - 几何容差 ±0.75pt（与 mang-shell 层 pypdf 对照同款——独立解析器
//!   的数值格式差（Real/Integer）在亚像素级）；
//! - 位图高：|our_h − 640 × eff_h/eff_w| ≤ 1.5px（poppler/pypdfium2
//!   ceil vs 本仓 round 的 ±1px 取整差 + 浮点累积）；
//! - 探针页 = `reference_manifest.json` 的 `pages`（15 文件 × 25 探针
//!   页与显示层矩阵同面，跨层同覆盖可互查）。
//!
//! # 环境合同（不静默 skip）
//!
//! - 库定位走 `PdfiumEngine::open` 定位序（`MDC_PDFIUM_LIB` → 编译期
//!   候选 → 系统）；
//! - 语料 = mang-shell/tests/fixtures/display_corpus（env
//!   `MDC_PDF_TEST_FIXTURES` 直指 → 编译期候选清单）；
//! - `MDC_634_REQUIRE_PARITY=1`：**活对照门**——缺库/缺语料时本文件
//!   **panic**（显示矩阵要求独立对照在场，不以 SKIP 计通过）；未设置：
//!   缺件环境显式 SKIP（banner 不伪装通过）。
//!
//! 命令（#51 验证清单 1/3）：
//!
//! ```text
//! cargo test -p mdc-pdf --features engine-lopdf,engine-pdfium \
//!   --test pdfr_display_matrix -- --nocapture --test-threads=1
//! ```

// 特性门（默认构建零引擎依赖——本文件退化为空目标，跨环境可构建纪律）。
#![cfg(feature = "engine-pdfium")]
#![cfg(feature = "engine-lopdf")]

use std::path::{Path, PathBuf};

use mdc_pdf::engine::DocumentEngine;
use mdc_pdf::engine_pdfium::PdfiumEngine;
use mdc_pdf::page::{PageIndex, Rotation};
use mdc_pdf::render::{Rasterizer, RenderRequest};

// ---------------------------------------------------------------------------
// 环境合同：语料定位（env → 编译期候选）+ PARITY 门
// ---------------------------------------------------------------------------

/// PARITY 环境变量（与 mang-shell 层 `pdf_display_verify` 同名同义——
/// `=1` = 独立对照必须在场，缺件 = fail）。
const PARITY_ENV: &str = "MDC_634_REQUIRE_PARITY";

/// 语料定位环境变量（目录形态——直指 display_corpus）。
const FIXTURES_ENV: &str = "MDC_PDF_TEST_FIXTURES";

fn parity_required() -> bool {
    std::env::var(PARITY_ENV).map(|v| v == "1").unwrap_or(false)
}

/// display_corpus 编译期候选（crate manifest 相对）：mang-shell 语料位
/// 在前（#634 成果入库位——fresh clone 即可复现）。
fn corpus_candidates() -> Vec<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    vec![
        // mang-shell 显示语料（#634 成果位）。
        manifest.join("../../mang-shell/tests/fixtures/display_corpus"),
        // crate 本地 fixtures（同目录兜底——测试面素材位约定）。
        manifest.join("fixtures/display_corpus"),
    ]
}

fn corpus_dir() -> PathBuf {
    if let Ok(dir) = std::env::var(FIXTURES_ENV) {
        return PathBuf::from(dir);
    }
    for dir in corpus_candidates() {
        if dir.join("reference_manifest.json").is_file() {
            return dir;
        }
    }
    panic!(
        "display_corpus 定位失败（定位序：{FIXTURES_ENV} → 编译期候选{}）",
        corpus_candidates()
            .iter()
            .map(|p| format!(" {}", p.display()))
            .collect::<String>()
    );
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(corpus_dir().join(name)).unwrap_or_else(|e| {
        panic!(
            "语料 {name} 读取失败：{e}（目录 {}）",
            corpus_dir().display()
        )
    })
}

/// 缺库环境判定（与 mang-shell 层 `pdf_display_verify` 同款文案口径）。
fn is_missing_library_error(msg: &str) -> bool {
    msg.contains("libpdfium") || msg.contains("PDF 引擎不可用")
}

fn skip_banner(msg: &str) {
    eprintln!(
        "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
         本机无 libpdfium——#634 引擎显示矩阵未真跑。\n  \
         绑定/打开错误全文：{msg}\n  \
         有 libpdfium 的环境（MDC_PDFIUM_LIB=<库路径> \
         cargo test）全量执行。\n═══════════════════════════\n"
    );
}

/// 缺件守卫：PARITY=1 → fail；否则 SKIP（banner）并返回 None。
fn missing_guard(msg: &str, face: &str) -> Option<()> {
    if parity_required() {
        panic!(
            "MDC_634_REQUIRE_PARITY=1 而 {face} 缺件：独立对照门要求真跑，\
             缺库不可 SKIP 计通过。错误全文：{msg}"
        );
    }
    skip_banner(msg);
    None
}

// ---------------------------------------------------------------------------
// lopdf 独立几何解析（第三方对照器——不共享 pdfium 任何代码路径）
// ---------------------------------------------------------------------------

/// lopdf 独立解析的单页显示域几何（显示域 = CropBox 优先、缺省回落
/// MediaBox；有效宽高 = 90°/270° 旋转交换后的**渲染显示**口径——与
/// pypdf manifest 的 render_w_pt/render_h_pt 同义）。
#[derive(Debug, Clone, Copy, PartialEq)]
struct LopdfGeometry {
    media: (f64, f64, f64, f64),
    crop: Option<(f64, f64, f64, f64)>,
    rotation_deg: i64,
}

impl LopdfGeometry {
    /// 显示域矩形（CropBox 优先；None = MediaBox 形态回落）。
    fn display_rect(&self) -> (f64, f64, f64, f64) {
        self.crop.unwrap_or(self.media)
    }
    /// 未旋转显示域宽高。
    fn raw_w(&self) -> f64 {
        let (x0, _, x1, _) = self.display_rect();
        x1 - x0
    }
    fn raw_h(&self) -> f64 {
        let (_, y0, _, y1) = self.display_rect();
        y1 - y0
    }
    /// 旋转后渲染显示宽高（90/270 交换——spec /Rotate 页字典）。
    fn render_w(&self) -> f64 {
        if self.rotation_deg % 180 == 90 {
            self.raw_h()
        } else {
            self.raw_w()
        }
    }
    fn render_h(&self) -> f64 {
        if self.rotation_deg % 180 == 90 {
            self.raw_w()
        } else {
            self.raw_h()
        }
    }
}

/// lopdf `Object` → f64（Integer/Real 双格式；其他形态 = None——
/// 对照器不做猜测）。
fn obj_as_f64(obj: &lopdf::Object) -> Option<f64> {
    match obj {
        lopdf::Object::Integer(v) => Some(*v as f64),
        lopdf::Object::Real(v) => Some(f64::from(*v)),
        _ => None,
    }
}

/// 间接引用解到实对象（Reference → `get_object`；其余原样）。
fn derefed<'a>(doc: &'a lopdf::Document, obj: &'a lopdf::Object) -> Option<&'a lopdf::Object> {
    match obj {
        lopdf::Object::Reference(id) => doc.get_object(*id).ok(),
        other => Some(other),
    }
}

/// 矩形 Object → (x0,y0,x1,y1)。
fn obj_as_rect(obj: &lopdf::Object) -> Option<(f64, f64, f64, f64)> {
    let arr = obj.as_array().ok()?;
    if arr.len() != 4 {
        return None;
    }
    Some((
        obj_as_f64(&arr[0])?,
        obj_as_f64(&arr[1])?,
        obj_as_f64(&arr[2])?,
        obj_as_f64(&arr[3])?,
    ))
}

/// 页字典键沿 /Parent 链继承查找（spec 7.7.3.3：MediaBox/CropBox/
/// Rotate 可继承）。
fn inherited<'a>(
    doc: &'a lopdf::Document,
    dict: &'a lopdf::Dictionary,
    key: &[u8],
) -> Option<&'a lopdf::Object> {
    let mut cur = dict;
    loop {
        // lopdf 0.46：`Dictionary::get` → Result<&Object, Error>。
        if let Ok(found) = cur.get(key) {
            return derefed(doc, found);
        }
        let parent_id = cur.get(b"Parent").ok()?;
        let parent = derefed(doc, parent_id)?;
        cur = parent.as_dict().ok()?;
    }
}

/// lopdf 独立解析全页几何（页树遍历——与 pdfium 互不相干的语义实现）。
/// 失败（结构损坏/对象缺失）= `None`：对照器对无法解析的文件不作
/// 猜测（坏文件在矩阵中以 pdfium typed 拒绝为准，lopdf 行为入证据行）。
fn lopdf_page_geometries(bytes: &[u8]) -> Option<Vec<LopdfGeometry>> {
    let doc = lopdf::Document::load_mem(bytes).ok()?;
    let pages = doc.get_pages(); // BTreeMap<u32(1 基), ObjectId>
    let mut out = Vec::with_capacity(pages.len());
    for (_, page_id) in pages {
        let page = doc.get_object(page_id).ok()?;
        let dict = page.as_dict().ok()?;
        let media = inherited(&doc, dict, b"MediaBox")
            .and_then(obj_as_rect)
            .unwrap_or((0., 0., 612., 792.)); // spec 缺省 Letter——仅显示域回退口径
        let crop = inherited(&doc, dict, b"CropBox")
            .and_then(obj_as_rect)
            .filter(|c| {
                // 恒等形态（crop ≡ media，容差 0.5pt）归 None——与引擎
                // `engine_pdfium.rs` 的归档语义同款（「真裁剪」vs「缺省」）。
                (c.0 - media.0).abs() > 0.5
                    || (c.1 - media.1).abs() > 0.5
                    || (c.2 - media.2).abs() > 0.5
                    || (c.3 - media.3).abs() > 0.5
            });
        let rotation_deg = inherited(&doc, dict, b"Rotate")
            .and_then(|o| match o {
                lopdf::Object::Integer(v) => Some(*v),
                _ => None,
            })
            .unwrap_or(0);
        out.push(LopdfGeometry {
            media,
            crop,
            rotation_deg,
        });
    }
    Some(out)
}

fn rotation_deg(r: Rotation) -> i64 {
    match r {
        Rotation::Deg0 => 0,
        Rotation::Deg90 => 90,
        Rotation::Deg180 => 180,
        Rotation::Deg270 => 270,
    }
}

// ---------------------------------------------------------------------------
// 矩阵测试
// ---------------------------------------------------------------------------

/// 矩阵：15 语料 × manifest 探针页——pdfium 显示几何/旋转/渲染尺寸 vs
/// lopdf 独立解析。三方断言不放宽（±0.75pt / 旋转枚举 / ±1.5px）。
#[test]
fn engine_geometry_matrix_matches_lopdf_independent_parse() {
    let manifest_raw = std::fs::read_to_string(corpus_dir().join("reference_manifest.json"))
        .expect("reference_manifest.json 可读");
    let manifest: serde_json::Value = serde_json::from_str(&manifest_raw).expect("manifest JSON");
    eprintln!(
        "#634 引擎矩阵证据：poppler {}（manifest 登记）| 参照 pdfium build {} | PARITY={}",
        manifest["poppler_version"],
        manifest["pdfium_build"],
        std::env::var(PARITY_ENV).unwrap_or_else(|_| "<未设>".into()),
    );

    // 环境门先行（缺库：PARITY=1 → fail；否则 SKIP）。
    let probe = fixture_bytes("zh_embedded_font.pdf");
    if let Err(err) = PdfiumEngine::open(probe) {
        let msg = format!("{err:?}");
        assert!(
            is_missing_library_error(&msg),
            "完好探针文件打开失败（真缺陷，非环境缺件）：{msg}"
        );
        let _ = missing_guard(&msg, "引擎显示矩阵");
        return;
    }

    let mut checked = 0usize;
    let mut worst_geo_pt = 0f64;
    let mut worst_bitmap_px = 0f64;
    for (name, entry) in manifest["files"].as_object().expect("manifest.files 对象") {
        let bytes = fixture_bytes(name);

        // 坏文件：pdfium typed 拒绝（三方对照见 manifest 登记；lopdf 的
        // 宽容/拒绝行为仅入证据行，不入断言）。
        if entry["expected"] == "error" {
            match PdfiumEngine::open(bytes.clone()) {
                Ok(_) => panic!("{name} 是坏文件——引擎必须拒绝打开"),
                Err(err) => {
                    let msg = format!("{err:?}");
                    assert!(
                        !is_missing_library_error(&msg),
                        "{name} 拒绝原因不可是环境缺件（环境门已过）"
                    );
                    let lopdf_face = match lopdf_page_geometries(&bytes) {
                        Some(g) => {
                            format!("lopdf 可解析（{} 页——宽容解析面，三方对照已登记）", g.len())
                        }
                        None => "lopdf 亦拒绝/不可解析".to_string(),
                    };
                    eprintln!("#634 引擎矩阵 ✓ {name} → typed 拒绝；独立面：{lopdf_face}");
                    continue;
                }
            }
        }

        // 完好文件：pdfium 几何 vs lopdf 独立解析逐页矩阵。
        let mut engine = PdfiumEngine::open(bytes.clone())
            .unwrap_or_else(|e| panic!("{name}（完好文件）打开失败：{e:?}"));
        let lopdf_geo = lopdf_page_geometries(&bytes)
            .unwrap_or_else(|| panic!("{name} lopdf 独立解析失败（完好文件必须可解析）"));

        let pages = entry["pages"]
            .as_object()
            .unwrap_or_else(|| panic!("{name} 完好文件必须有探针页"));
        for (page_str, _page_entry) in pages {
            let page: u32 = page_str.parse().expect("探针页键 0 基数字");
            let geo = engine
                .page_geometry(PageIndex(page))
                .unwrap_or_else(|e| panic!("{name} p{page} 几何读取失败：{e:?}"));
            let indep = &lopdf_geo[page as usize];

            // ① 旋转标记（枚举级一致——/Rotate 是离散语义，无容差）。
            assert_eq!(
                rotation_deg(geo.rotation),
                indep.rotation_deg,
                "{name} p{page} 旋转标记：pdfium {:?} ≠ lopdf /Rotate {}",
                geo.rotation,
                indep.rotation_deg,
            );

            // ② 显示域宽高（±0.75pt——与 pypdf 对照同容差；90/270 已按
            // 渲染显示口径交换）。
            let (exp_w, exp_h) = (indep.render_w(), indep.render_h());
            let dw = (f64::from(geo.effective_width()) - exp_w).abs();
            let dh = (f64::from(geo.effective_height()) - exp_h).abs();
            worst_geo_pt = worst_geo_pt.max(dw).max(dh);
            assert!(
                dw <= 0.75,
                "{name} p{page} 有效宽 {:.2}pt ≠ lopdf 独立解析 {exp_w:.2}pt（±0.75）",
                geo.effective_width()
            );
            assert!(
                dh <= 0.75,
                "{name} p{page} 有效高 {:.2}pt ≠ lopdf 独立解析 {exp_h:.2}pt（±0.75）",
                geo.effective_height()
            );

            // ③ 渲染位图尺寸链（640px 宽请求 → 高按显示域纵横比；
            //    round vs ceil 取整差 ≤1px + 浮点累积 → 1.5px 带）。
            let rendered = engine
                .rasterize(RenderRequest {
                    page: PageIndex(page),
                    width_px: 640,
                })
                .unwrap_or_else(|e| panic!("{name} p{page} 渲染失败：{e:?}"));
            assert_eq!(rendered.width_px, 640, "{name} p{page} 光栅宽精确回显");
            let expect_h = 640. * exp_h / exp_w;
            let dev = (rendered.height_px as f64 - expect_h).abs();
            worst_bitmap_px = worst_bitmap_px.max(dev);
            assert!(
                dev <= 1.5,
                "{name} p{page} 位图高 {}px 偏离显示域纵横比 {expect_h:.2}px（>1.5px）",
                rendered.height_px
            );

            // ④ 极端尺寸/旋转标记证据行（rotated_90/oversized_page 等
            //    特征面在 D1 要求中点名记录——逐页输出供 PR 证据引用）。
            eprintln!(
                "#634 引擎矩阵 ✓ {name} p{page}：显示域 {:.2}×{:.2}pt | /Rotate {} | 位图 640×{}（纵横比偏离 {dev:.2}px）",
                geo.effective_width(),
                geo.effective_height(),
                indep.rotation_deg,
                rendered.height_px,
            );
            checked += 1;
        }
    }

    assert!(
        checked >= 25,
        "矩阵覆盖面坍缩（{checked} 探针页——清单不得静默缩水）"
    );
    eprintln!(
        "#634 引擎矩阵核验 ✓ {checked} 探针页（pdfium vs lopdf 独立解析；最差几何差 {worst_geo_pt:.3}pt / 位图比差 {worst_bitmap_px:.2}px）"
    );
}
