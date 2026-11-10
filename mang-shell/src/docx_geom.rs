//! #50 C2：DOCX 页面几何单源——twips↔px 换算、缩放档、条带几何、
//! 内核失效区间映射（全纯函数，零 gpui 依赖）。
//!
//! **定位**：C1 的几何函数从 docx_pages.rs 收拢于此并扩为缩放档模型
//! （pdf_pages #421 `ZoomMode` 同构——键位/菜单/步进/滚轮语义与 PDF
//! 视图一致，用户心智零迁移）。**twips↔px 单一真源**：`TWIPS_PER_PX`
//!（96 DPI 名义：1px = 15tw）此前住在 docx_lok_session，本模块收拢
//! 后 docx_lok_session re-export（既有消费面零改动）——「实际大小
//! 100%」= 1px:15tw 名义口径（与 worker `paint` 的画布像素契约一致：
//! 内核按画布尺寸缩放渲染，宿主换算只此一家）。
//!
//! **DPI 边界（如实）**：本批光栅 = 逻辑 px（与 main 上 pdf_pages 现状
//! 同口径）；设备像素密度光栅（HiDPI 按 scale_factor 请求高密度位图）
//! 待 D 线 #51 D2 的 device_raster_width 方案合入 main 后按同款接法
//! 收编——不在此预造第二套（「不并发整块重写共享文件」纪律）。
//!
//! **条带模型回顾**：LOK Writer 文档是连续画布（`size()` twips）。
//! 光栅宽 `rw`（px）决定显示缩放：`scale = rw / doc_w_px`，两轴同缩放。
//! 画布纵向切 512px 条带，边界 = floor(i·512·tw_per_px)——同一边界点
//! 两侧同取 floor（相邻条带首尾相接零重叠；去重消除高倍缩放下不足
//! 1 twip 的空条带）。缓存键 = (条带序号, rw)——缩放换代天然分键。
//!
//! **失效区间映射**：内核 INVALIDATE_TILES 回调载荷（twips 矩形）→
//! 受影响条带序号区间（条带按 y 单调有序 → 区间可二段扫描）。
//! 载荷不可解析/非正尺寸（LOK 以 `-1,-1,0,0`、`EMPTY` 表全量失效）
//! → 全量失效（诚实放大不静默丢弃）。

use crate::docx_geom::ZoomMode::{ActualSize, Custom, FitWidth};

/// twips → 像素换算（96 DPI 名义，与 crates/lok/src/smoke.rs
/// `TWIPS_PER_PX` 同源；worker 渲染面 DPI 无关，宿主侧换算单源）。
pub const TWIPS_PER_PX: i64 = 15;

/// 条带显示高（px）——画布纵向切片粒度（渲染中占位/缓存键基准）。
pub const BAND_PX: u32 = 512;
/// 光栅宽量化步进（pdf_pages `WIDTH_QUANTUM` 同款——视口微抖不换缓存键）。
pub const WIDTH_QUANTUM: u32 = 64;
/// 光栅宽下/上限（px）——与 pdf_pages 同值（跨视图一致的光栅域护栏）。
pub const WIDTH_MIN: u32 = 256;
pub const WIDTH_MAX: u32 = 4096;

/// 缩放下限/上限（相对实际大小的倍率——pdf `PDF_ZOOM_MIN/MAX` 同值）。
pub const DOCX_ZOOM_MIN: f32 = 0.25;
pub const DOCX_ZOOM_MAX: f32 = 8.0;
/// 键盘步进倍率（Ctrl+= / Ctrl+-——pdf `PDF_ZOOM_STEP` 同值）。
pub const DOCX_ZOOM_STEP: f32 = 1.25;

// ---------------------------------------------------------------------------
// 缩放档模型（pdf_pages #421 `ZoomMode` 同构）
// ---------------------------------------------------------------------------

/// #50 C2：页面视图缩放模式。
///
/// 与 PDF 视图的差异（如实）：DOCX 连续画布无「适应页面」档——单页
/// 适页需要页边界（内核页面几何 op，已登记 #49 缺口），不宿主自算。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoomMode {
    /// 适应宽度（v1 默认——光栅宽 = 视口宽量化，行为零变化）。
    FitWidth,
    /// 实际大小（100% = 96 DPI 名义：`doc_w_px = doc_w_tw / 15`）。
    ActualSize,
    /// 自定义倍率（相对实际大小；Ctrl+=/- 步进、滚轮、预设档落此态）。
    Custom(f32),
}

/// 光栅宽量化：钳制区间 + 向上取整到步进倍数（单一口径：派单/槽渲染/
/// 缩放档全部经此——缓存键稳定的第一道门）。
pub fn quantize_width(w: u32) -> u32 {
    let clamped = w.clamp(WIDTH_MIN, WIDTH_MAX);
    clamped.div_ceil(WIDTH_QUANTUM) * WIDTH_QUANTUM
}

/// 缩放模式 → 光栅宽（px）——纯函数（单测锚点；视口宽仅 FitWidth 用）。
pub fn zoom_raster_width(mode: ZoomMode, doc_w_tw: i64, vw: f32) -> u32 {
    let doc_w_px = (doc_w_tw.max(1) as f64 / TWIPS_PER_PX as f64) as f32;
    let raw = match mode {
        FitWidth => vw.max(1.),
        ActualSize => doc_w_px,
        Custom(z) => doc_w_px * z.clamp(DOCX_ZOOM_MIN, DOCX_ZOOM_MAX),
    };
    quantize_width(raw.max(1.) as u32)
}

/// 缩放百分比（相对实际大小；Custom 档直报倍率——量化只影响光栅缓存
/// 键不折进来，菜单预设 50% 报 50 不报 54；预设档从光栅宽如实派生）。
pub fn zoom_percent(mode: ZoomMode, doc_w_tw: i64, vw: f32) -> u32 {
    if let Custom(z) = mode {
        return (z * 100.).round().clamp(10., 800.) as u32;
    }
    let doc_w_px = (doc_w_tw.max(1) as f64 / TWIPS_PER_PX as f64) as f32;
    if doc_w_px <= 0. {
        return 100;
    }
    let w = zoom_raster_width(mode, doc_w_tw, vw) as f32;
    (w / doc_w_px * 100.).round().clamp(10., 800.) as u32
}

// ---------------------------------------------------------------------------
// 连续画布几何（C1 收拢——签名 view_w → rw 语义显名）
// ---------------------------------------------------------------------------

/// 显示缩放（显示 px / 文档名义 px）。文档宽必须 > 0。
pub fn raster_scale(doc_w_tw: i64, rw: u32) -> f64 {
    let doc_w_px = doc_w_tw as f64 / TWIPS_PER_PX as f64;
    debug_assert!(doc_w_px > 0.);
    rw as f64 / doc_w_px.max(1.)
}

/// 总显示高（px）：`doc_h_px * scale`（两轴同缩放）。
pub fn content_height_px(doc_w_tw: i64, doc_h_tw: i64, rw: u32) -> f64 {
    let doc_h_px = doc_h_tw as f64 / TWIPS_PER_PX as f64;
    doc_h_px * raster_scale(doc_w_tw, rw)
}

/// 条带边界（twips，严格递增，首恒 0、末恒 `doc_h_tw`）：第 i 条带显示
/// 区 `[i*BAND_PX, (i+1)*BAND_PX)` 换算 `floor(i*BAND_PX*tw_per_px)`——
/// 同一边界点两侧同取 floor（相邻条带首尾相接零重叠；去重消除高倍
/// 缩放下不足 1 twip 的空条带）。
pub fn band_boundaries(doc_w_tw: i64, doc_h_tw: i64, rw: u32) -> Vec<i64> {
    if doc_w_tw <= 0 || doc_h_tw <= 0 || rw == 0 {
        return Vec::new();
    }
    let tw_per_disp_px = TWIPS_PER_PX as f64 / raster_scale(doc_w_tw, rw);
    let total_disp = content_height_px(doc_w_tw, doc_h_tw, rw);
    let mut bounds = vec![0i64];
    let max_i = (total_disp / BAND_PX as f64).ceil() as i64 + 1;
    for i in 1..=max_i {
        let b = (i as f64 * BAND_PX as f64 * tw_per_disp_px).floor() as i64;
        if b > *bounds.last().unwrap() && b < doc_h_tw {
            bounds.push(b);
        }
        if b >= doc_h_tw {
            break;
        }
    }
    if *bounds.last().unwrap() < doc_h_tw {
        bounds.push(doc_h_tw);
    }
    bounds
}

/// 条带数（边界链长度-1；退化输入 0）。
pub fn band_count(doc_w_tw: i64, doc_h_tw: i64, rw: u32) -> usize {
    band_boundaries(doc_w_tw, doc_h_tw, rw)
        .len()
        .saturating_sub(1)
}

/// 条带几何：显示条带 `band` 的内核请求区域（twips）与画布（px）。
#[derive(Debug, Clone, PartialEq)]
pub struct BandGeom {
    /// 画布宽（px）＝光栅宽。
    pub canvas_w: u32,
    /// 画布高（px）＝条带显示高（末带余量）。
    pub canvas_h: u32,
    /// 区域 x（twips，恒 0——适宽全幅）。
    pub tile_x: i32,
    /// 区域 y（twips）。
    pub tile_y: i32,
    /// 区域宽（twips，恒全文档宽）。
    pub tile_w: i32,
    /// 区域高（twips，末带钳制到文末）。
    pub tile_h: i32,
}

/// 第 `band` 条带几何（越界 = None）。边界与 [`band_boundaries`] 同源
/// ——相邻条带首尾相接、无重叠、末带终于文末（单测锁）。
pub fn band_region(doc_w_tw: i64, doc_h_tw: i64, rw: u32, band: u32) -> Option<BandGeom> {
    let bounds = band_boundaries(doc_w_tw, doc_h_tw, rw);
    if bounds.len() < 2 || band as usize + 1 >= bounds.len() {
        return None;
    }
    let tw_per_disp_px = TWIPS_PER_PX as f64 / raster_scale(doc_w_tw, rw);
    let tile_y = bounds[band as usize];
    let tile_end = bounds[band as usize + 1];
    debug_assert!(tile_end > tile_y);
    let tile_h = tile_end - tile_y;
    let canvas_h = ((tile_h as f64 / tw_per_disp_px).ceil() as u32).max(1);
    Some(BandGeom {
        canvas_w: rw,
        canvas_h,
        tile_x: 0,
        tile_y: tile_y.min(i32::MAX as i64) as i32,
        tile_w: doc_w_tw.min(i32::MAX as i64) as i32,
        tile_h: tile_h.min(i32::MAX as i64) as i32,
    })
}

// ---------------------------------------------------------------------------
// 滚动锚定（缩放换档——内容点不跳）
// ---------------------------------------------------------------------------

/// 视口中心的内容 twips 纵坐标（缩放换档锚点——换档前后取值应相等）。
pub fn center_twips(offset_y_px: f64, viewport_h_px: f64, doc_w_tw: i64, rw: u32) -> f64 {
    let center_px = (offset_y_px.max(0.) + viewport_h_px.max(0.) / 2.).max(0.);
    let tw_per_px = TWIPS_PER_PX as f64 / raster_scale(doc_w_tw, rw);
    center_px * tw_per_px
}

/// 中心 twips → 新光栅宽下的滚动偏移（换档回写：`center_twips -
/// vh/2`——与取锚同式互逆，量化舍入差 ≤ 半像素级）。
pub fn offset_for_center_twips(
    center_tw: f64,
    viewport_h_px: f64,
    doc_w_tw: i64,
    rw: u32,
) -> f64 {
    let px_per_tw = raster_scale(doc_w_tw, rw) / TWIPS_PER_PX as f64;
    (center_tw * px_per_tw - viewport_h_px.max(0.) / 2.).max(0.)
}

// ---------------------------------------------------------------------------
// 内核失效区间映射（INVALIDATE_TILES 载荷 → 条带序号区间）
// ---------------------------------------------------------------------------

/// 失效范围：twips 矩形（部分失效）或全量（载荷不可解析/非正尺寸——
/// LOK 以 `-1,-1,0,0`、`EMPTY` 字段表全量；诚实放大不静默丢弃）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InvalScope {
    /// 全量失效（换尺寸/未知矩形）。
    Full,
    /// 矩形（twips）：`(x, y, w, h)`。
    Rect { x: i64, y: i64, w: i64, h: i64 },
}

/// 解析 LOK `INVALIDATE_TILES` 载荷（格式：`"x, y, w, h[, part, mode]"`
/// ——逗号分隔、空格容忍；LibreOfficeKit 回调契约）。**不可解析/非正
/// 尺寸一律 Full**（宁可多失效不少失效——陈旧像素是产品缺陷，多画
/// 一次只是开销）。
pub fn parse_invalidate_payload(payload: &str) -> InvalScope {
    let fields: Vec<&str> = payload.split(',').map(str::trim).collect();
    let Some(Ok(x)) = fields.first().map(|s| s.parse::<i64>()) else {
        return InvalScope::Full;
    };
    let (Some(Ok(y)), Some(Ok(w)), Some(Ok(h))) = (
        fields.get(1).map(|s| s.parse::<i64>()),
        fields.get(2).map(|s| s.parse::<i64>()),
        fields.get(3).map(|s| s.parse::<i64>()),
    ) else {
        return InvalScope::Full;
    };
    if w <= 0 || h <= 0 {
        return InvalScope::Full;
    }
    InvalScope::Rect { x, y, w, h }
}

/// 矩形 → 受影响条带区间 `[start, end)`（空区间 = 无条带受影响——
/// 矩形在文档域外/横向零交叠）。条带按 y 单调有序，区间连续。
///
/// 横向交叠判据：条带恒全幅（tile_x=0, tile_w=doc_w_tw），故矩形
/// `[x, x+w)` 与 `[0, doc_w_tw)` 有交即可。
pub fn bands_in_rect(
    doc_w_tw: i64,
    doc_h_tw: i64,
    rw: u32,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
) -> std::ops::Range<u32> {
    let bounds = band_boundaries(doc_w_tw, doc_h_tw, rw);
    if bounds.len() < 2 {
        return 0..0;
    }
    // 横向零交叠或纵向零交叠（文档域外）。
    if x >= doc_w_tw || x + w <= 0 || y >= doc_h_tw || y + h <= 0 {
        return 0..0;
    }
    let rect_end = (y + h).min(doc_h_tw);
    let rect_start = y.max(0);
    let mut start = None;
    let mut end = 0u32;
    for (i, win) in bounds.windows(2).enumerate() {
        let (b0, b1) = (win[0], win[1]);
        let overlaps = b0 < rect_end && b1 > rect_start;
        if overlaps {
            if start.is_none() {
                start = Some(i as u32);
            }
            end = i as u32 + 1;
        }
    }
    start.unwrap_or(0)..end
}

// ---------------------------------------------------------------------------
// 测试面（全纯函数——lib 树恒绿）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A4 纵向夹具（twips）。
    const A4: (i64, i64) = (11906, 16838);

    /// 量化：钳制 + 向上取整到 64 倍数（v1 口径回归）。
    #[test]
    fn quantize_width_clamps_and_rounds() {
        assert_eq!(quantize_width(0), WIDTH_MIN);
        assert_eq!(quantize_width(100), WIDTH_MIN);
        assert_eq!(quantize_width(257), WIDTH_QUANTUM * 5);
        assert_eq!(quantize_width(320), 320);
        assert_eq!(quantize_width(10_000), WIDTH_MAX);
    }

    /// FitWidth 档 = v1 适宽（量化视口宽）——零行为变化门。
    #[test]
    fn zoom_raster_width_fit_width_matches_v1() {
        assert_eq!(zoom_raster_width(ZoomMode::FitWidth, A4.0, 800.), quantize_width(800));
        assert_eq!(zoom_raster_width(ZoomMode::FitWidth, A4.0, 79.), WIDTH_MIN);
    }

    /// ActualSize 档 = 名义文档宽（A4 → 11906/15 ≈ 794px）。
    #[test]
    fn zoom_raster_width_actual_size_is_nominal() {
        let w = zoom_raster_width(ZoomMode::ActualSize, A4.0, 800.);
        assert_eq!(w, quantize_width((A4.0 / TWIPS_PER_PX) as u32));
        // 100% 档换回 twips 不失真（单源口径）。
        assert!((w as f64) - (A4.0 as f64 / TWIPS_PER_PX as f64) < WIDTH_QUANTUM as f64);
    }

    /// Custom 档：线性缩放 + 域钳制（0.25×A4=198px → WIDTH_MIN 256）。
    #[test]
    fn zoom_raster_width_custom_scales_and_clamps() {
        let half = zoom_raster_width(ZoomMode::Custom(0.5), A4.0, 800.);
        assert_eq!(half, quantize_width((A4.0 as f32 * 0.5 / TWIPS_PER_PX as f32) as u32));
        let tiny = zoom_raster_width(ZoomMode::Custom(DOCX_ZOOM_MIN), A4.0, 800.);
        assert_eq!(tiny, WIDTH_MIN, "下限档必须钳到光栅下限");
        let big = zoom_raster_width(ZoomMode::Custom(DOCX_ZOOM_MAX), A4.0, 800.);
        assert!(big <= WIDTH_MAX);
    }

    /// 百分比上报：Custom 直报倍率；预设档从**量化后**光栅宽如实派生
    ///（量化向上取整 → 「实际大小」在 chip 上报 ≥100——与 pdf #421
    /// 同口径：显示的是真实生效倍率而非名义档位，漂移上界 = 一步量化）。
    #[test]
    fn zoom_percent_reporting() {
        assert_eq!(zoom_percent(ZoomMode::Custom(0.5), A4.0, 800.), 50);
        assert_eq!(zoom_percent(ZoomMode::Custom(1.5), A4.0, 800.), 150);
        // 实际大小：量化漂移上界 = WIDTH_QUANTUM/doc_w_px（A4 ≈ 8%）。
        let actual_pct = zoom_percent(ZoomMode::ActualSize, A4.0, 800.);
        let doc_w_px = A4.0 as f64 / TWIPS_PER_PX as f64;
        let drift = (WIDTH_QUANTUM as f64 / doc_w_px * 100.).ceil() as u32;
        assert!(
            actual_pct >= 100 && actual_pct <= 100 + drift,
            "实际大小百分比必须落在 [100, {}]（实际 {actual_pct}）",
            100 + drift
        );
        // 适宽随视口变化——百分比如实反映（800px 视口 → 832/794 ≈ 105%）。
        let fit_pct = zoom_percent(ZoomMode::FitWidth, A4.0, 800.);
        assert!(fit_pct >= 95 && fit_pct <= 110, "适宽百分比 = {fit_pct}");
    }

    /// 条带族无缝覆盖 `[0, doc_h_tw)`、无重叠、末带钳制文末（C1 回归
    /// 门——多档光栅宽）。
    #[test]
    fn band_regions_tile_document_exactly() {
        for rw in [WIDTH_MIN, 512, 1024, 4096] {
            let count = band_count(A4.0, A4.1, rw);
            assert!(count >= 1, "条带数必须 ≥1（rw={rw}）");
            let mut prev_end = 0i64;
            for band in 0..count as u32 {
                let geom = band_region(A4.0, A4.1, rw, band)
                    .unwrap_or_else(|| panic!("条带 {band} 必须有几何"));
                assert_eq!(geom.tile_x, 0);
                assert_eq!(geom.tile_w as i64, A4.0, "适宽全幅（rw={rw}）");
                assert!(geom.tile_h > 0, "区域高必须 > 0");
                assert!(geom.tile_y as i64 + geom.tile_h as i64 <= A4.1);
                assert!(
                    geom.tile_y as i64 == prev_end,
                    "条带必须首尾相接（band={band}）"
                );
                prev_end = geom.tile_y as i64 + geom.tile_h as i64;
                assert!(geom.canvas_h >= 1);
            }
            assert_eq!(prev_end, A4.1, "条带族必须覆盖到文末（rw={rw}）");
        }
    }

    /// 越界条带 = None；退化尺寸 = 0 条带。
    #[test]
    fn band_region_bounds() {
        assert!(band_region(100, 100, 512, 99).is_none());
        assert_eq!(band_count(0, 100, 512), 0);
        assert_eq!(band_count(100, 0, 512), 0);
        assert_eq!(band_count(100, 100, 0), 0);
    }

    /// 窄文档（s>1 放大）：显示高按比例放大、条带数随之增。
    #[test]
    fn narrow_doc_scales_up() {
        // 文档 3000tw 宽（200px 名义）、6000tw 高（400px 名义）。
        let rw = 1024u32; // s = 1024/200 = 5.12
        let total = content_height_px(3000, 6000, rw);
        assert!((total - 2048.).abs() < 1., "两轴同缩放：400*5.12≈{total:.0}");
        let count = band_count(3000, 6000, rw);
        assert!((count as f64 * BAND_PX as f64) >= total);
        assert!(total > BAND_PX as f64);
    }

    /// 锚定互逆：center_twips → offset_for_center_twips 还原（±1px——
    /// 量化舍入级）；换档前后中心 twips 不变（缩放不跳内容）。
    #[test]
    fn scroll_anchor_roundtrip_and_invariance() {
        let (offset, vh) = (1234.5f64, 600.0f64);
        let c = center_twips(offset, vh, A4.0, 1024);
        let back = offset_for_center_twips(c, vh, A4.0, 1024);
        assert!((back - offset).abs() < 1., "互易还原：{back} vs {offset}");
        // 换档：同中心 twips 在新档下的偏移按比例缩放（视口中心内容点
        // 不跳页——#421 验收 4 同款语义）。
        let off_512 = offset_for_center_twips(c, vh, A4.0, 512);
        let c_512 = center_twips(off_512, vh, A4.0, 512);
        assert!((c_512 - c).abs() < TWIPS_PER_PX as f64, "中心锚不漂移");
    }

    /// 失效载荷解析：标准 6 字段 LOK 格式 / 空格容忍 / 部分字段。
    #[test]
    fn parse_invalidate_standard_forms() {
        assert_eq!(
            parse_invalidate_payload("0, 0, 11906, 16838, 0, 0"),
            InvalScope::Rect { x: 0, y: 0, w: 11906, h: 16838 }
        );
        assert_eq!(
            parse_invalidate_payload("100,200,300,400"),
            InvalScope::Rect { x: 100, y: 200, w: 300, h: 400 }
        );
        // 负坐标合法（LO 可发部分越界矩形）。
        assert_eq!(
            parse_invalidate_payload("-50, -60, 100, 200, 1, 2"),
            InvalScope::Rect { x: -50, y: -60, w: 100, h: 200 }
        );
    }

    /// 全量失效口径：非正尺寸（LOK `-1,-1,0,0`）、EMPTY 字段、空/垃圾
    /// 载荷——一律 Full（诚实放大）。
    #[test]
    fn parse_invalidate_full_scope_forms() {
        assert_eq!(parse_invalidate_payload("-1, -1, 0, 0"), InvalScope::Full);
        assert_eq!(parse_invalidate_payload("EMPTY, EMPTY, 0, 0, 0, 0"), InvalScope::Full);
        assert_eq!(parse_invalidate_payload(""), InvalScope::Full);
        assert_eq!(parse_invalidate_payload("garbage"), InvalScope::Full);
        assert_eq!(parse_invalidate_payload("1,2,3"), InvalScope::Full); // 缺字段
    }

    /// 矩形 → 条带区间：单带命中 / 跨带区间 / 域外空区间 / 全文档。
    #[test]
    fn bands_in_rect_maps_band_ranges() {
        // 1024px 光栅 A4：显示高 ≈ 1448px → 3 条带（512×2 + 余量）。
        let count = band_count(A4.0, A4.1, 1024);
        assert!(count >= 3, "夹具前提：≥3 条带（实际 {count}）");
        let bounds = band_boundaries(A4.0, A4.1, 1024);
        // 第 1 条带内部小矩形 → 恰命中条带 0。
        let r0 = bands_in_rect(
            A4.0,
            A4.1,
            1024,
            0,
            bounds[1] - 10,
            A4.0,
            10,
        );
        assert_eq!(r0, 0..1, "条带 0 内矩形");
        // 跨第 2-3 条带的纵向长矩形 → 区间 [1,3)。
        let span = bands_in_rect(
            A4.0,
            A4.1,
            1024,
            0,
            bounds[1] + 10,
            A4.0,
            bounds[3] - bounds[1] - 20,
        );
        assert_eq!(span, 1..3, "跨带矩形 → 连续区间");
        // 文档下方域外 → 空区间。
        let below = bands_in_rect(A4.0, A4.1, 1024, 0, A4.1 + 500, A4.0, 1000);
        assert_eq!(below, 0..0, "纵向域外");
        // 横向域外 → 空区间。
        let beside = bands_in_rect(A4.0, A4.1, 1024, A4.0 + 500, 0, 1000, A4.1);
        assert_eq!(beside, 0..0, "横向域外");
        // 全文档矩形 → 全部条带。
        let all = bands_in_rect(A4.0, A4.1, 1024, 0, 0, A4.0, A4.1);
        assert_eq!(all, 0..count as u32);
    }
}
