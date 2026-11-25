//! #47 巨石拆解批④：原 pdf_pages.rs 内嵌 `tests` 纯迁出
//!（零行为变化——字节原样剥壳，挂载 #[cfg(test)] + #[path] 在宿主文件）。

use super::*;
use mdc_pdf::page::Rotation;
use std::path::PathBuf;

/// #643：设备宽量化护栏（上限 = 引擎 S3-L2 钳制值）。
#[test]
fn device_quantize_width_clamps_and_rounds_up() {
    assert_eq!(device_quantize_width(256), 256);
    // 64 步长向上对齐（与逻辑宽同款量子化护栏思想）。
    assert_eq!(device_quantize_width(300), 320);
    // 上限 = 引擎单边钳制值；更高请求壳侧先钳（键不失控）。
    assert_eq!(device_quantize_width(u32::MAX), WIDTH_MAX_DEVICE);
    assert_eq!(device_quantize_width(0), WIDTH_MIN);
}

/// #643：HiDPI 密度接线不变量——请求密度随 sf 升档，显示几何
/// （槽高）保持逻辑域；单倍屏与 v1 完全同值（缓存键零迁移）。
#[gpui::test]
fn hidpi_device_width_scales_display_stays_logical(cx: &mut gpui::TestAppContext) {
    let view = cx.update(|app| {
        app.new(|cx| {
            PdfPagesView::error_state(
                Arc::new(Vec::new()),
                cx.focus_handle(),
                cx.focus_handle(),
                "测试态：引擎缺席（几何口径面不依赖 worker）".into(),
            )
        })
    });
    cx.update(|app| {
        view.update(app, |v, _| {
            v.viewport_size = (1000., 800.);
            let logical = v.raster_width(1000., 800.);
            // 单倍屏：设备宽 = 逻辑宽——v1 行为与缓存键完全兼容。
            assert_eq!(v.device_raster_width(1000., 800.), logical);
            // 2x 屏：请求密度翻倍（64 步长量化对齐）。
            v.set_scale_factor_for_test(2.0);
            assert_eq!(
                v.device_raster_width(1000., 800.),
                device_quantize_width(logical * 2)
            );
            // 显示几何不随 sf 变：槽高仍按逻辑宽（uniform_list
            // 行高稳定——HiDPI 换档不重排）。
            let slot_h = PdfPagesView::slot_height(logical, v.aspect);
            v.set_scale_factor_for_test(1.0);
            assert_eq!(slot_h, PdfPagesView::slot_height(logical, v.aspect));
            // sf 域防御：异常低报不降密度（逻辑宽即保底清晰度）。
            v.set_scale_factor_for_test(0.5);
            assert_eq!(v.device_raster_width(1000., 800.), logical);
        })
    });
}

#[test]
fn quantize_width_clamps_and_rounds_up() {
    assert_eq!(quantize_width(0), WIDTH_MIN, "下限兜底");
    assert_eq!(quantize_width(100), WIDTH_MIN);
    assert_eq!(quantize_width(300), 320, "向上取 64 倍数");
    assert_eq!(quantize_width(320), 320, "已是倍数不抖动");
    assert_eq!(quantize_width(321), 384);
    assert_eq!(quantize_width(100_000), WIDTH_MAX, "上限钳制");
    // 上限对齐：4096 本身是 64 倍数，钳后不变。
    assert_eq!(quantize_width(WIDTH_MAX), WIDTH_MAX);
}

#[test]
fn slot_height_follows_aspect() {
    assert!((PdfPagesView::slot_height(800, 1.5) - 1200.).abs() < 1e-6);
    // 宽高比 1.415（A4 近似——避开 clippy::approx_constant 对
    // √2 近似值的误报；测试输入参数，非常量计算）：
    // 640 × 1.415 = 905.6 → #634 0.5px 量子化保持 905.5。
    assert!((PdfPagesView::slot_height(640, 1.415) - 905.5).abs() < 0.01);
    // #634：量子化语义——解析值对齐半像素网格（Taffy 行高测量
    // 同网格；uniform_list 测量与滚动换算单源一致）。aspect 用
    // √2 常量（字面近似值触发 clippy::approx_constant）。
    assert_eq!(
        PdfPagesView::slot_height(576, std::f32::consts::SQRT_2),
        814.5
    );
    assert_eq!(
        PdfPagesView::slot_height(577, std::f32::consts::SQRT_2),
        816.0
    );
}

#[test]
fn fit_width_subtracts_margin_and_quantizes() {
    // 1024 视口 - 24 边距 = 1000 → 量子化 1024。
    assert_eq!(PdfPagesView::fit_width(1024.), 1024);
    // 极窄视口 → 下限。
    assert_eq!(PdfPagesView::fit_width(10.), WIDTH_MIN);
}

/// #51 D2 尾巴：内容 y → 页空间锚点（纯函数——异尺寸档的换算
/// 正确性面；fit_w=576 × aspect [0.5/1.5/1.0] → 槽高 [288/864/576]）。
#[test]
fn content_offset_maps_y_to_page_space_anchor() {
    let aspects = [0.5, 1.5, 1.0];
    let off = PdfPagesView::content_offset_to_page(&aspects, 576, 0.);
    assert_eq!(off.item_ix, 0, "y=0 → 首页顶");
    assert_eq!(f32::from(off.offset_in_item), 0.);

    let off = PdfPagesView::content_offset_to_page(&aspects, 576, 287.5);
    assert_eq!(off.item_ix, 0, "页 0 内最后半像素（287.5 < 288）");
    assert!((f32::from(off.offset_in_item) - 287.5).abs() < 0.01);

    // 页边界：288 恰落页 1 顶（半开区间 [0,288) 归页 0）。
    let off = PdfPagesView::content_offset_to_page(&aspects, 576, 288.);
    assert_eq!(off.item_ix, 1);
    assert_eq!(f32::from(off.offset_in_item), 0.);

    // 页 1 中部：累计 288 + 712 = 1000。
    let off = PdfPagesView::content_offset_to_page(&aspects, 576, 1000.);
    assert_eq!(off.item_ix, 1);
    assert!((f32::from(off.offset_in_item) - 712.).abs() < 0.01);

    // 越界钳末页顶（总高 288+864+576=1728 之后 → 末页锚）。
    let off = PdfPagesView::content_offset_to_page(&aspects, 576, 5000.);
    assert_eq!(off.item_ix, 2);
    assert_eq!(f32::from(off.offset_in_item), 0.);

    // 空表防御：零页档钳 item_ix=0（error 态不进本路径，防御面）。
    let off = PdfPagesView::content_offset_to_page(&[], 576, 10.);
    assert_eq!(off.item_ix, 0);
}

/// #51 D2 尾巴：**异尺寸合成件**的每页几何捕获 + 每页槽高——
/// 混合横竖/大小页不再被首页等高槽统一（真引擎开档；缺库环境
/// 按 first_painted 同款 SKIP 门跳过）。
#[gpui::test]
fn mixed_size_pages_capture_per_page_aspects(cx: &mut gpui::TestAppContext) {
    let view =
        cx.update(|app| app.new(|cx| PdfPagesView::new(mixed_size_pdf_bytes_for_tests(), cx)));
    if let Some(err) = view.read_with(cx, |v, _| v.error.clone()) {
        if std::env::var_os("MDC_PDFIUM_LIB").is_none() && err.contains("pdfium") {
            eprintln!("SKIP: 每页几何捕获需要 PDFium 库：{err}");
            return;
        }
        panic!("异尺寸夹具必须打开成功（配齐 PDFium）：{err}");
    }
    view.read_with(cx, |v, _| {
        assert_eq!(v.page_count(), 3, "三页异尺寸夹具");
        // 每页纵横比：宽扁 0.5 / 高瘦 1.5 / 正方 1.0（MediaBox 实测）。
        for (idx, want) in [(0usize, 0.5f32), (1, 1.5), (2, 1.0)] {
            let got = v.page_aspect_of(idx);
            assert!(
                (got - want).abs() < 0.01,
                "页 {idx} 纵横比 {got} ≠ 夹具值 {want}（每页几何捕获）"
            );
        }
        // 每页槽高（fit_w=576）：288 / 864 / 576——异尺寸档各页各高。
        for (idx, want) in [(0usize, 288f32), (1, 864.), (2, 576.)] {
            let got = v.page_slot_height(576, idx);
            assert!((got - want).abs() < 0.5, "页 {idx} 槽高 {got} ≠ {want}");
        }
        // 页顶累计偏移：0 / 288 / 1152（跳转 y 的页级定位基准）。
        for (idx, want) in [(0usize, 0f32), (1, 288.), (2, 1152.)] {
            let got = v.page_top_offset(576, idx);
            assert!((got - want).abs() < 0.5, "页 {idx} 顶偏移 {got} ≠ {want}");
        }
    });
}

/// #637：回退键选择器——同页多宽择最近、rev 优先、异页不取、
/// 空缓存 None（四例齐）。
#[test]
fn nearest_fallback_key_picks_same_page_nearest_width() {
    let mk = |page: u32, w: u32, rev: u64| RenderKey::new(page, w, w * 2, rev);
    let keys = vec![mk(0, 320, 1), mk(0, 800, 1), mk(1, 640, 1)];
    // 同页两条（320/800）目标 700 → 800 更近。
    let pick = PdfPagesView::nearest_fallback_key(&keys, 0, 700).unwrap();
    assert_eq!(pick.width_px, 800, "同页择宽最近");
    // 异页不取：页 2 无条目 → None。
    assert!(
        PdfPagesView::nearest_fallback_key(&keys, 2, 700).is_none(),
        "异页不取"
    );
    // 空缓存 → None。
    assert!(
        PdfPagesView::nearest_fallback_key(&[], 0, 700).is_none(),
        "空缓存 None"
    );
}

#[test]
fn nearest_fallback_key_rev_wins_over_width() {
    let mk = |page: u32, w: u32, rev: u64| RenderKey::new(page, w, w * 2, rev);
    // rev=2 的 320 宽 vs rev=1 的 800 宽：目标 700——宽近者 800 但
    // rev 旧；rev 最大者优先（防闪回旧批注态）。
    let keys = vec![mk(0, 320, 2), mk(0, 800, 1)];
    let pick = PdfPagesView::nearest_fallback_key(&keys, 0, 700).unwrap();
    assert_eq!(
        (pick.width_px, pick.annotation_rev),
        (320, 2),
        "rev 优先于宽近"
    );
}

/// 尺寸/缓冲不一致 → None（failed 面，不 panic）。
#[test]
fn rendered_page_to_image_rejects_mismatched_buffer() {
    let bad = RenderedPage {
        width_px: 4,
        height_px: 4,
        bgra: vec![0u8; 15], // 应为 64 字节。
    };
    assert!(rendered_page_to_image(&bad).is_none());
}

/// 合法缓冲 → Some（尺寸承接到 RenderImage frame）。
#[test]
fn rendered_page_to_image_accepts_valid_buffer() {
    let good = RenderedPage {
        width_px: 2,
        height_px: 2,
        bgra: vec![1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255],
    };
    assert!(rendered_page_to_image(&good).is_some());
}

// -----------------------------------------------------------------
// #422：页内查找——纯函数面（quads 归行 / 层匹配 / 计数条文案）
// -----------------------------------------------------------------

/// 合成字符（同基线 y 行，字号 10；用户空间 pt）。
fn ch(c: char, x: f32, y: f32) -> PositionedChar {
    PositionedChar {
        ch: c,
        left: x,
        right: x + 10.,
        bottom: y,
        top: y + 10.,
        font_size: 10.,
    }
}

/// 同行连续字符 → 单 quad 包围盒；跨行（中心距 > 容差）→ 每行一
/// quad——高亮跨行命中的行矩形语义。
#[test]
fn match_quads_groups_lines() {
    // 同一行三字符（y=100 基线）。
    let same_line = vec![ch('a', 10., 100.), ch('b', 25., 100.), ch('c', 40., 100.)];
    let quads = match_quads(&same_line);
    assert_eq!(quads.len(), 1, "同行命中归一 quad");
    assert_eq!(quads[0].left, 10.);
    assert_eq!(quads[0].right, 50., "右缘取端点极值");
    // 跨行（第二行基线 y=140——中心距 40 > 容差）。
    let two_lines = vec![ch('a', 10., 100.), ch('b', 25., 100.), ch('c', 10., 140.)];
    let quads = match_quads(&two_lines);
    assert_eq!(quads.len(), 2, "跨行命中两 quad（一行一个）");
    assert_eq!(quads[0].bottom, 100.);
    assert_eq!(quads[1].top, 150., "第二行 quad 自带上缘");
    // 空段 → 零 quad。
    assert!(match_quads(&[]).is_empty());
}

/// 层匹配：大小写折叠（Kafka ← kafka）+ CJK byte 偏移 → char 下标
/// 换算（中文混排不偏）+ 多处命中阅读序。
#[test]
fn find_in_layer_casefold_and_cjk_mapping() {
    let text = "消息中间件采用Kafka三副本，元";
    let chars: Vec<PositionedChar> = text
        .chars()
        .enumerate()
        .map(|(i, c)| ch(c, i as f32 * 10., 100.))
        .collect();
    let layer = PageTextLayer {
        lower: text.to_lowercase(),
        chars,
        transform: PageTransform::from_geometry(&PageGeometry::simple(595., 842., Rotation::Deg0))
            .expect("A4 简单几何必可构造变换"),
    };
    // 大小写折叠：小写查询命中大写原文。
    let hits = find_in_layer(&layer, "kafka");
    assert_eq!(hits.len(), 1, "Kafka 一处（大小写折叠）");
    assert_eq!(hits[0], (7, 5), "起始 char 下标 + 长度（7 = 「采用」后）");
    // CJK 多字节：byte 偏移换算 char 下标不错位（元 = 第 16 char——
    // 消息中间件采用 7 + Kafka 5 + 三副本 3 + 逗号 1 之后）。
    let hits = find_in_layer(&layer, "元");
    assert_eq!(hits, vec![(16, 1)], "中文单字命中 char 下标直落");
    // 多处命中阅读序 + 空查询零命中。
    let hits = find_in_layer(&layer, "三");
    assert_eq!(hits.len(), 1);
    assert!(find_in_layer(&layer, "").is_empty());
}

// -----------------------------------------------------------------
// #51 D4：文本选择——纯函数面（矩形→字符 / 行间断点 / 坐标变换）
// -----------------------------------------------------------------

/// 合成两行层（用户空间 pt：行 1 基线 y=600「Alpha」，行 2 基线
/// y=560「Beta Gamma」——中心差 40pt 远超 30% 行容差）。
fn two_line_layer() -> PageTextLayer {
    let mut chars = Vec::new();
    for (i, c) in "Alpha".chars().enumerate() {
        chars.push(ch(c, i as f32 * 10., 600.));
    }
    for (i, c) in "Beta Gamma".chars().enumerate() {
        chars.push(ch(c, i as f32 * 10., 560.));
    }
    PageTextLayer {
        lower: "alpha beta gamma".into(),
        chars,
        transform: PageTransform::from_geometry(&PageGeometry::simple(595., 842., Rotation::Deg0))
            .expect("A4 简单几何必可构造变换"),
    }
}

/// 矩形命中：整页矩形 → 全字符（阅读序）；窄矩形 → 只命中覆盖列
/// 的字符（相交判据）；行间断点 → 复制文本换行。
#[test]
fn selection_chars_and_line_breaks() {
    let layer = two_line_layer();
    // 整页矩形（用户空间全覆盖）→ 两行全选。
    let full = (0., 0., 595., 842.);
    let chars = selection_chars(&layer, full);
    assert_eq!(chars.len(), 15, "两行 5 + 10 字符全命中");
    let text = selection_text(&layer, full);
    assert_eq!(text, "Alpha\nBeta Gamma", "跨行选择 = 行间换行");
    // 窄矩形（x∈[51, 91] 只覆盖行 2 的「Gamma」列——左界过空格
    // 右缘（50）避带空格；用户空间 y∈[550, 575] 只碰行 2）。
    let narrow = (51., 550., 91., 575.);
    let text = selection_text(&layer, narrow);
    assert_eq!(text, "Gamma", "窄矩形只选覆盖列（相交判据）");
    // 空白区矩形（页边留白）→ 零命中零文本。
    let blank = (0., 800., 100., 842.);
    assert!(selection_chars(&layer, blank).is_empty());
    assert_eq!(selection_text(&layer, blank), "", "空白区选择 = 空文本");
}

/// 槽域 → 用户空间变换链（letterbox 偏移扣除 + DPI + 组盒）：A4
/// 适宽 576px 槽（纵横比恰好 → 零留白偏移）下，屏幕矩形映射 =
/// 用户空间等比矩形（576/595 ≈ 0.968 缩放，y 翻转，y 底对齐 842）。
#[test]
fn selection_user_rect_identity_chain() {
    let layer = two_line_layer();
    // fit_w=576、slot_h=842×576/595 ≈ 815.7——纵横比与页一致
    //（contain 无 letterbox 偏移），内容盒即整槽。
    let fit_w = 576.;
    let slot_h = 815.7;
    // 屏幕矩形（左上原点 y 向下）：x∈[0, 576]、y∈[0, 407.8]（半页）
    // → 用户空间 = y∈[842/2, 842]（下半页）。
    let (x0, y0, x1, y1) = selection_user_rect(&layer, fit_w, slot_h, (0., 0., 576., 407.8));
    let sx = 595. / 576.; // 用户 pt / 屏幕 px
    assert!((x0 - 0.).abs() < 0.5, "左缘对齐（实得 {x0}）");
    assert!((x1 - 595.).abs() < 1., "右缘对齐（实得 {x1}）");
    assert!((y0 - 421.).abs() < 1., "下缘半页（实得 {y0}）");
    assert!((y1 - 842.).abs() < 1., "上缘页顶（实得 {y1}）");
    let _ = (y0, y1, sx);
    // 反向对角序（拖选从右下往左上）→ 同一矩形（归一化）。
    let (r_x0, r_y0, r_x1, r_y1) =
        selection_user_rect(&layer, fit_w, slot_h, (576., 407.8, 0., 0.));
    assert_eq!((x0, y0, x1, y1), (r_x0, r_y0, r_x1, r_y1), "对角序无关");
}

/// #51 D4d：跨页拖选 → 各触达页槽域矩形——前进/后退/单页退化
/// 三面。角域语义：低页端点→页底右角、高页页顶左角→端点、中间
/// 页全页；哨兵端 = 开放边（下游内容盒钳制收边）。
#[test]
fn cross_page_slot_rects_forward_backward_and_degenerate() {
    let s = SEL_EDGE_SENTINEL;
    // 前进：页 0 锚点 (10, 20) → 页 2 游标 (30, 40)。
    let fwd = SelectionDrag {
        anchor: (0, 10., 20.),
        cursor: (2, 30., 40.),
    };
    let rects = cross_page_slot_rects(&fwd);
    assert_eq!(rects.len(), 3, "触达页段 [0, 2] 恰三页");
    assert_eq!(rects[&0], (10., 20., s, s), "低页 = 锚点端 → 页底右角");
    assert_eq!(rects[&1], (-s, -s, s, s), "中间页 = 全页");
    assert_eq!(rects[&2], (-s, -s, 30., 40.), "高页 = 页顶左角 → 游标端");

    // 后退：页 2 锚点 (10, 20) → 页 0 游标 (30, 40)——低页端 = 游标，
    // 高页端 = 锚点（倒拖不倒文本——读序组装恒 lo → hi）。
    let bwd = SelectionDrag {
        anchor: (2, 10., 20.),
        cursor: (0, 30., 40.),
    };
    let rects = cross_page_slot_rects(&bwd);
    assert_eq!(rects.len(), 3, "触达页段 [0, 2] 恰三页");
    assert_eq!(rects[&0], (30., 40., s, s), "低页 = 游标端 → 页底右角");
    assert_eq!(rects[&1], (-s, -s, s, s), "中间页 = 全页");
    assert_eq!(rects[&2], (-s, -s, 10., 20.), "高页 = 页顶左角 → 锚点端");

    // 单页退化（D4a 原语义不动）：锚点 = 游标页——页内矩形原样。
    let same = SelectionDrag {
        anchor: (1, 10., 20.),
        cursor: (1, 30., 40.),
    };
    let rects = cross_page_slot_rects(&same);
    assert_eq!(rects.len(), 1, "单页段");
    assert_eq!(rects[&1], (10., 20., 30., 40.), "页内矩形（任意对角序）");

    // 跳页快拖（页 0 → 页 4，中间页无 move 事件）：两端点推导整段。
    let skip = SelectionDrag {
        anchor: (0, 5., 5.),
        cursor: (4, 6., 7.),
    };
    let rects = cross_page_slot_rects(&skip);
    assert_eq!(rects.len(), 5, "触达页段 [0, 4] 恰五页（推导不看事件流）");
    assert_eq!(rects[&3], (-s, -s, s, s), "未经过的中间页也全页覆盖");
}

/// #51 D4d：页间断行拼接——恰一条（不叠不缺）；空文本页整页跳
/// 过（不生空行不生连续换行）；空入参 = 空串。
#[test]
fn join_page_texts_page_break_lines() {
    use super::join_page_texts;
    // 基本页界：两页各一段 → 中间恰一条换行。
    assert_eq!(
        join_page_texts(&["第一页尾".into(), "第二页头".into()]),
        "第一页尾\n第二页头"
    );
    // 前页尾已是行尾 → 不重复补。
    assert_eq!(
        join_page_texts(&["第一页尾\n".into(), "第二页头".into()]),
        "第一页尾\n第二页头"
    );
    // 空文本页整页跳过——不生空行。
    assert_eq!(
        join_page_texts(&["首页".into(), "".into(), "末页".into()]),
        "首页\n末页"
    );
    // 全空段 = 空串（无选择语义——消费点另判空）。
    assert_eq!(join_page_texts(&[]), "");
    assert_eq!(join_page_texts(&["".into(), "".into()]), "");
    // 三页链：页界各恰一条。
    assert_eq!(
        join_page_texts(&["甲".into(), "乙".into(), "丙".into()]),
        "甲\n乙\n丙"
    );
}

/// #421 尾：量子门控溢出面——FitWidth div_ceil 上量化伪溢出
/// （< WIDTH_QUANTUM=64）归零（平移面不启用——右缘 ≤63px 裁剪与
/// v1 同款非产品溢出）；≥ 64px 真溢出如实通过；负值（光栅窄于
/// 视口）同归零。
#[test]
fn gated_pan_overflow_quantum_threshold() {
    use super::PdfPagesView;
    let q = WIDTH_QUANTUM as f32;
    // 伪溢出区（0 < raw < 64）→ 0。
    assert_eq!(PdfPagesView::gated_pan_overflow(0.), 0., "零溢出");
    assert_eq!(
        PdfPagesView::gated_pan_overflow(32.),
        0.,
        "FitWidth 量子化过冲 32px = 伪溢出归零"
    );
    assert_eq!(
        PdfPagesView::gated_pan_overflow(63.9),
        0.,
        "量子梯下沿仍伪溢出"
    );
    // 真溢出（≥ 64）→ 原值通过。
    assert_eq!(PdfPagesView::gated_pan_overflow(q), q, "恰一梯 = 真溢出");
    assert_eq!(
        PdfPagesView::gated_pan_overflow(280.),
        280.,
        "125% 档溢出如实"
    );
    // 负值（光栅窄于视口——FitPage/小档）→ 0。
    assert_eq!(
        PdfPagesView::gated_pan_overflow(-120.),
        0.,
        "无溢出归零（负钳）"
    );
}

/// 计数条文案四态：空查询 / 索引中 / 完成无匹配（含失败页脚注）/
/// 完成有命中「n/m」——计数条是唯一展示源，文案漂移即 UI 回归。
#[test]
fn search_label_states() {
    let mut s = PdfSearch::new(2);
    assert_eq!(s.match_label(), "输入关键词", "空查询");
    s.query = "不存在词".into();
    assert!(
        s.match_label().contains("索引中"),
        "索引未完成显进度：{}",
        s.match_label()
    );
    s.indexed_done = true;
    assert_eq!(s.match_label(), "无匹配", "完成零命中");
    s.failed_pages = 1;
    assert!(
        s.match_label().contains("1 页文本层不可读"),
        "提取失败页如实脚注：{}",
        s.match_label()
    );
    s.failed_pages = 0;
    s.matches = vec![
        SearchMatch {
            page: 0,
            quads: smallvec::smallvec![],
        },
        SearchMatch {
            page: 1,
            quads: smallvec::smallvec![],
        },
    ];
    s.current = 1;
    assert_eq!(s.match_label(), "2/2 处", "命中态「n/m」");
}

// -----------------------------------------------------------------
// #422：页内查找——视图全链（引擎门控：无 libpdfium SKIP 横幅）
// -----------------------------------------------------------------

/// smoke.pdf 字节（与 #336/#386 同源夹具）。
fn smoke_pdf_bytes() -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.pdf");
    std::fs::read(&p).unwrap_or_else(|e| panic!("夹具不可读（{}）：{e}", p.display()))
}

/// No fonts or text layer: a visible filled rectangle must still render.
fn bitmap_only_pdf_bytes() -> Vec<u8> {
    let content = "q 1 0 0 rg 20 30 70 50 re f Q\n";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 320 240] /Resources << >> /Contents 4 0 R >>"
            .to_string(),
        format!(
            "<< /Length {} >>\nstream\n{content}endstream",
            content.len()
        ),
    ];
    let mut pdf = "%PDF-1.4\n".to_string();
    let mut offsets = vec![0];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{object}\nendobj\n", i + 1));
    }
    let xref = pdf.len();
    pdf.push_str("xref\n0 5\n0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
    ));
    pdf.into_bytes()
}

struct BitmapHost(gpui::Entity<PdfPagesView>);

impl gpui::Render for BitmapHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div().size_full().child(self.0.clone())
    }
}

/// The first painted viewport must schedule its own follow-up frame. No
/// mouse/key/resize event is injected to accidentally start the worker.
#[gpui::test]
fn first_painted_viewport_starts_bitmap_render_without_input(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let view = cx.update(|app| app.new(|cx| PdfPagesView::new(bitmap_only_pdf_bytes(), cx)));
    if let Some(err) = view.read_with(cx, |v, _| v.error.clone()) {
        if std::env::var_os("MDC_PDFIUM_LIB").is_none() && err.contains("pdfium") {
            eprintln!("SKIP: first-frame bitmap rendering requires a PDFium library: {err}");
            return;
        }
        panic!("bitmap-only PDF must open with configured PDFium: {err}");
    }
    let window = cx.open_window(gpui::size(px(600.), px(480.)), {
        let view = view.clone();
        move |_, _| BitmapHost(view)
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, app| window.draw(app).clear(app));
    assert!(
        view.read_with(&visual, |v, _| v
            .viewport
            .get()
            .is_some_and(|(w, h)| w > 0. && h > 0.)),
        "real layout/paint must measure a nonempty PDF viewport"
    );
    let scheduled = visual.update(|window, app| window.simulate_next_frame(app));
    assert!(
        scheduled > 0,
        "first painted viewport must request a follow-up frame; waiting for user input leaves PDF unrendered"
    );
    visual.update(|window, app| window.draw(app).clear(app));
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        cx.background_executor
            .advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(2));
        if view.read_with(&visual, |v, _| {
            // #643：键口径 = 设备像素宽（与 submit_visible 同源）。
            let width = v.device_raster_width(v.viewport_size.0, v.viewport_size.1);
            v.render_key(&(0, width))
                .is_some_and(|key| v.cache.contains(&key))
        }) {
            visual.update(|window, app| window.draw(app).clear(app));
            let list = visual
                .debug_bounds("pdf-pages-list")
                .expect("page list is in the real rendered tree");
            assert!(
                list.size.width > px(200.),
                "rendered list must have visible width, not only a cache entry"
            );
            let page = visual
                .debug_bounds("pdf-visible-page-0")
                .expect("first rendered page reaches the layout tree");
            assert!(
                page.size.width > px(100.) && page.size.height > px(100.),
                "cached PDF image must occupy a visible page slot"
            );
            assert_eq!(
                visual.update(|window, app| window.simulate_next_frame(app)),
                0,
                "unchanged viewport must not create a permanent animation loop"
            );
            return;
        }
    }
    panic!("first visible bitmap did not reach the real worker/cache within 10s");
}

/// 合成按键（main.rs 烟雾派发同构）。
fn key_event(key: &str, key_char: Option<&str>) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: gpui::Keystroke {
            modifiers: Default::default(),
            key: key.to_string(),
            key_char: key_char.map(|c| c.to_string()),
        },
        is_held: false,
        prefer_character_input: false,
    }
}

/// ctrl 按键变体（Enter/Shift 组合导航面）。
fn key_event_mods(key: &str, shift: bool, key_char: Option<&str>) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: gpui::Keystroke {
            modifiers: gpui::Modifiers {
                shift,
                ..Default::default()
            },
            key: key.to_string(),
            key_char: key_char.map(|c| c.to_string()),
        },
        is_held: false,
        prefer_character_input: false,
    }
}

/// 泵测试时钟直到索引完成。真实索引线程吃**真实墙钟**（首档
/// 文本层冷加载——字体表装载可达秒级），测试时钟推进只驱动轮询
/// 循环；两者并进（每拍实睡 2ms 让线程跑），墙钟 15s 限期。
fn pump_until_indexed(cx: &mut gpui::TestAppContext, view: &gpui::Entity<PdfPagesView>) {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        cx.background_executor
            .advance_clock(Duration::from_millis(20));
        cx.background_executor.run_until_parked();
        std::thread::sleep(Duration::from_millis(2));
        let done = cx.read(|app| {
            view.read(app)
                .search
                .as_ref()
                .is_some_and(|s| s.indexed_done)
        });
        if done {
            return;
        }
    }
    panic!("索引 15s 墙钟未完成（真缺陷不放过——CI 慢机另行调阈）");
}

/// #422 全链：开条 → 索引（线程/通道/轮询三件套）→ 键入（大小写
/// 折叠）→ 计数「n/m」→ 导航环回 → 退格改查询 → Esc 关条（层丢
/// 弃）。libpdfium 环境全量真跑；无库环境 SKIP 横幅（#386 门序同
/// 款——不伪装通过）。
/// 测试宿主（PdfPagesView 不作根视图——与 shell 测试同款空 div
/// 宿主；实体经 slot 持有，WindowHandle::update 内 update 不重入）。
#[allow(dead_code)]
struct SearchHost(gpui::Entity<PdfPagesView>);

impl gpui::Render for SearchHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div()
    }
}

#[gpui::test]
fn pdf_view_search_full_flow(cx: &mut gpui::TestAppContext) {
    let bytes = smoke_pdf_bytes();
    let mut slot: Option<gpui::Entity<PdfPagesView>> = None;
    let window = cx.add_window(|_window, cx| {
        let v = cx.new(|cx| PdfPagesView::new(bytes, cx));
        slot = Some(v.clone());
        SearchHost(v)
    });
    let view = slot.expect("宿主构造即置");
    // 环境门控：error 态且文案含 pdfium = 缺库（SKIP 横幅 + return）；
    // 其他 error = 夹具/真缺陷（panic 不放过）。
    let gate_err = cx.read(|app| view.read(app).error.clone());
    if let Some(err) = gate_err {
        if err.contains("pdfium") {
            eprintln!(
                "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                     本机无 libpdfium——#422 页内查找全链 e2e 未真跑。\n  \
                     有 libpdfium 的环境（MDC_PDFIUM_LIB=<库路径> \
                     cargo test）全量执行。\n═══════════════════════════\n"
            );
            return;
        }
        panic!("页视图构造失败（夹具/真缺陷，非环境缺件）：{err}");
    }
    // ① 开条（Ctrl+F 落点——Shell 接线面另有测试）。
    window
        .update(cx, |_host, window, cx| {
            view.update(cx, |v, cx| v.open_search(window, cx));
        })
        .expect("开条");
    // ② 索引完成（真实线程 + 通道回流 + 16ms 轮询 drain——测试
    // 时钟推进驱动）。
    pump_until_indexed(cx, &view);
    cx.read(|app| {
        let v = view.read(app);
        let s = v.search.as_ref().expect("索引期查找态常驻");
        assert!(s.indexed_done, "两页夹具索引应完成");
        assert_eq!(s.layers.len(), 2, "2 页文本层全落地");
        assert_eq!(s.failed_pages, 0, "健康夹具零提取失败");
    });
    // ③ 键入 kafka（小写）——大小写折叠命中原文「Kafka」（页 0）。
    window
        .update(cx, |_host, window, cx| {
            view.update(cx, |v, cx| {
                for (k, c) in [
                    ("k", Some("k")),
                    ("a", Some("a")),
                    ("f", Some("f")),
                    ("k", Some("k")),
                    ("a", Some("a")),
                ] {
                    v.dispatch_search_key(&key_event(k, c), window, cx);
                }
            });
        })
        .expect("键入");
    cx.read(|app| {
        let s = view.read(app).search.as_ref().expect("在位");
        assert_eq!(s.query, "kafka", "五键入全插入");
        assert_eq!(s.cursor, 5, "光标随键入推进");
        assert_eq!(s.matches.len(), 1, "Kafka 一处（大小写折叠）");
        assert_eq!(s.matches[0].page, 0, "命中在页 0（第二章段）");
        assert!(!s.matches[0].quads.is_empty(), "命中带行矩形");
        assert_eq!(s.match_label(), "1/1 处", "计数条「n/m」");
    });
    // ④ 导航：单命中环回（next → 回到自身）；视口未记录时零偏移
    // 分支不 panic（viewport_size 0,0 的测试态安全面）。
    window
        .update(cx, |_host, window, cx| {
            view.update(cx, |v, cx| {
                v.dispatch_search_key(&key_event("enter", None), window, cx);
            });
        })
        .expect("导航");
    cx.read(|app| {
        assert_eq!(
            view.read(app).search.as_ref().unwrap().current,
            0,
            "单命中环回不动"
        );
    });
    // ⑤ 退格改查询：kafka → kafk（前缀子串仍命中——匹配面是页
    // 文本不是查询历史；计数如实跟随）。
    window
        .update(cx, |_host, window, cx| {
            view.update(cx, |v, cx| {
                v.dispatch_search_key(&key_event("backspace", None), window, cx);
            });
        })
        .expect("退格");
    cx.read(|app| {
        let s = view.read(app).search.as_ref().unwrap();
        assert_eq!(s.query, "kafk");
        assert_eq!(s.matches.len(), 1, "kafk 是页内 Kafka 的前缀——仍命中");
        assert_eq!(s.match_label(), "1/1 处");
    });
    // ⑥ 中文查询跨页多命中：元（页 0 × 1 + 页 1 × 3 = 4——
    // 第三章三行价目 + 页 0 机柜行）。
    window
        .update(cx, |_host, window, cx| {
            view.update(cx, |v, cx| {
                for _ in 0..4 {
                    v.dispatch_search_key(&key_event("backspace", None), window, cx);
                }
                v.dispatch_search_key(&key_event("元", Some("元")), window, cx);
            });
        })
        .expect("改查询");
    cx.read(|app| {
        let s = view.read(app).search.as_ref().unwrap();
        assert_eq!(s.query, "元");
        assert_eq!(s.matches.len(), 4, "页 0 × 1 + 页 1 × 3");
        let pages: Vec<u32> = s.matches.iter().map(|m| m.page).collect();
        assert_eq!(pages, vec![0, 1, 1, 1], "页升序 + 页内阅读序");
        assert_eq!(s.match_label(), "1/4 处");
    });
    // ⑦ Enter 推进 + Shift+Enter 回退（环回导航）。
    window
        .update(cx, |_host, window, cx| {
            view.update(cx, |v, cx| {
                v.dispatch_search_key(&key_event("enter", None), window, cx);
                v.dispatch_search_key(&key_event("enter", None), window, cx);
                v.dispatch_search_key(&key_event_mods("enter", true, None), window, cx);
            });
        })
        .expect("导航×3");
    cx.read(|app| {
        assert_eq!(
            view.read(app).search.as_ref().unwrap().current,
            1,
            "两次前进一次回退 = 1"
        );
    });
    // ⑧ Esc 关条：态与层全丢（短命会话不留全档字符面）。
    window
        .update(cx, |_host, window, cx| {
            view.update(cx, |v, cx| {
                v.dispatch_search_key(&key_event("escape", None), window, cx);
            });
        })
        .expect("关条");
    cx.read(|app| {
        assert!(view.read(app).search.is_none(), "Esc 关条即清态");
    });
}

// ─── #421：缩放域纯函数面（零引擎依赖——同款纪律真跑） ──────────

/// FitWidth 档与 v1 适宽口径逐值一致（行为零变化锚点）。
#[test]
fn zoom_raster_width_fit_width_matches_v1_fit_width() {
    for vw in [10., 320., 640., 1024., 1366., 1920., 2560., 4096., 8192.] {
        assert_eq!(
            zoom_raster_width(ZoomMode::FitWidth, 1.414, 595., vw, 768.),
            PdfPagesView::fit_width(vw),
            "vw={vw}"
        );
    }
}

/// 适页档：高受限（矮宽视口）按视口高/纵横比；宽受限（窄高视口）
/// 不超过适宽；全程量化钳制。
#[test]
fn zoom_raster_width_fit_page_bounds() {
    // 矮宽视口（高受限）：vh/aspect = 600/1.5 = 400 → 量化 448
    //（适宽 1024 不封顶——高才是约束）。
    assert_eq!(
        zoom_raster_width(ZoomMode::FitPage, 1.5, 595., 1024., 600.),
        448
    );
    // 窄高视口（宽受限）：vh/aspect = 1200/1.5 = 800 > 适宽 320
    //→ 取适宽 320。
    assert_eq!(
        zoom_raster_width(ZoomMode::FitPage, 1.5, 595., 344., 1200.),
        320
    );
    // 极矮视口 → WIDTH_MIN 兜底。
    assert_eq!(
        zoom_raster_width(ZoomMode::FitPage, 1.5, 595., 1024., 100.),
        WIDTH_MIN
    );
}

/// 实际大小档：光栅宽 = 页 pt 宽（1pt:1px）经量化；pt 宽出域走
/// WIDTH_MIN/MAX 钳制；零视口不依赖（首帧即准）。
#[test]
fn zoom_raster_width_actual_size_is_pt_width() {
    // A4 595pt → 量化 640。
    assert_eq!(
        zoom_raster_width(ZoomMode::ActualSize, 1.414, 595., 0., 0.),
        640
    );
    // letter 612pt → 640。
    assert_eq!(
        zoom_raster_width(ZoomMode::ActualSize, 1.414, 612., 1920., 1080.),
        640
    );
    // pt 宽低于下限 → 256。
    assert_eq!(
        zoom_raster_width(ZoomMode::ActualSize, 1.414, 100., 1920., 1080.),
        WIDTH_MIN
    );
    // pt 宽超上限（海报横版）→ 4096。
    assert_eq!(
        zoom_raster_width(ZoomMode::ActualSize, 0.5, 5000., 1920., 1080.),
        WIDTH_MAX
    );
}

/// 自定义档：pt 宽 × 倍率（钳制在缩放域），量化照走；倍率出域
/// 钳到 PDF_ZOOM_MIN/MAX。
#[test]
fn zoom_raster_width_custom_scales_and_clamps() {
    // 595 × 1.5 = 892.5 → 量化 896。
    assert_eq!(
        zoom_raster_width(ZoomMode::Custom(1.5), 1.414, 595., 0., 0.),
        896
    );
    // 超上限倍率 → 钳 PDF_ZOOM_MAX：595×8 = 4760 → WIDTH_MAX 4096。
    assert_eq!(
        zoom_raster_width(ZoomMode::Custom(99.), 1.414, 595., 0., 0.),
        zoom_raster_width(ZoomMode::Custom(PDF_ZOOM_MAX), 1.414, 595., 0., 0.)
    );
    assert_eq!(
        zoom_raster_width(ZoomMode::Custom(PDF_ZOOM_MAX), 1.414, 595., 0., 0.),
        WIDTH_MAX
    );
    // 低于下限倍率 → 钳 PDF_ZOOM_MIN：595×0.25 = 148.75 → WIDTH_MIN。
    assert_eq!(
        zoom_raster_width(ZoomMode::Custom(0.01), 1.414, 595., 0., 0.),
        WIDTH_MIN
    );
}

/// 百分比换算：Custom 档直报倍率（量子化不折入——预设 50% 报 50
/// 不报 54）；预设档从光栅宽派生（适宽随视口如实反映）。
#[test]
fn zoom_mode_percent_custom_exact_and_preset_derived() {
    // Custom：直报倍率——量子化只影响缓存键不影响报数。
    assert_eq!(
        zoom_mode_percent(ZoomMode::Custom(0.5), 1.414, 595., 1024., 768.),
        50
    );
    assert_eq!(
        zoom_mode_percent(ZoomMode::Custom(1.0), 1.414, 595., 1024., 768.),
        100
    );
    assert_eq!(
        zoom_mode_percent(ZoomMode::Custom(2.0), 1.414, 595., 1024., 768.),
        200
    );
    // ActualSize：光栅 640 / 595 = 107.6% → 108%（量化折入——如
    // 实报当前生效光栅比例）。
    assert_eq!(
        zoom_mode_percent(ZoomMode::ActualSize, 1.414, 595., 1024., 768.),
        108
    );
    // FitWidth：适宽 1024 / 595 = 172.1% → 172%。
    assert_eq!(
        zoom_mode_percent(ZoomMode::FitWidth, 1.414, 595., 1024., 768.),
        172
    );
    // pt 宽无效（兜底路径防 0 除）→ 100。
    assert_eq!(
        zoom_mode_percent(ZoomMode::FitWidth, 1.414, 0., 1024., 768.),
        100
    );
    // #634：非 A4 档——横 A4（aspect 0.707）FitPage 光栅宽由
    // vh/aspect 派生，百分比与 A4 魔数口径可分辨（aspect 实参化
    // 的行为面：非 A4 档预设百分比不再按 A4 折算）。
    let landscape = zoom_mode_percent(ZoomMode::FitPage, 0.707, 842., 1024., 768.);
    let a4magic = zoom_mode_percent(ZoomMode::FitPage, 1.414, 842., 1024., 768.);
    assert_ne!(
        landscape, a4magic,
        "横 A4 与 A4 魔数口径的 FitPage 百分比应可分辨"
    );
}

/// 缩放域护栏常量自洽（PDF_ZOOM_MIN/MAX 与 issue 交互面一致）。
#[test]
fn zoom_domain_constants() {
    assert!(PDF_ZOOM_MIN < 1.0 && PDF_ZOOM_MAX > 1.0);
    assert!((PDF_ZOOM_WHEEL_STEP - 1.1).abs() < 1e-6, "issue 步进 10%");
    assert!((PDF_ZOOM_STEP - 1.25).abs() < 1e-6, "canvas ZOOM_STEP 同款");
}

// -----------------------------------------------------------------------
// #386：批注落盘——手势级 e2e（从用户手势开始，不跳过视图层）
// -----------------------------------------------------------------------
// 与 tests/pdf_annotate_e2e.rs（#336 控制器面）的分工：那批证明
// 「控制器**能**存盘」，本组证明「用户操作（右键拖框手势 → 视图
// 脏标 → 保存）**最终调到**存盘」——#386 的验收缺口正是「视图 →
// 文件」这一环。断言基线与 #336 同源：盘上前缀保持（增量非重写）
// + mdc-pdf 读面独立第二意见（与写面无共享实现路径）。
//
// 环境门控（render_170_pdf 同口径）：`PdfPagesView::new` 的几何
// 预读需 pdfium——无库环境 SKIP 横幅（不伪装通过），有则全量
// 真跑。批注链本身纯 Rust（lopdf），但本组钉的是页视图在位的
// 完整产品面。
// -----------------------------------------------------------------------

/// 临时落盘路径守卫（Drop 清理——panic 也清）。
struct TempPdf(PathBuf);
impl Drop for TempPdf {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// #386 核心闭环：手势 → 脏标 → 落盘 → 独立读回在档 → 脏标清零
/// → 干净重存 no-op（验收 1/2/5/6 的视图层全链）。
#[gpui::test]
fn gesture_marks_dirty_and_save_roundtrips(cx: &mut gpui::TestAppContext) {
    let original = smoke_pdf_bytes();
    let entity = cx.update(|cx| cx.new(|cx| PdfPagesView::new(original.clone(), cx)));
    // 环境门控：构造期引擎失败（无 libpdfium）→ SKIP；其他失败 =
    // 夹具/真缺陷（panic，不放过）。
    if let Some(e) = cx.update(|cx| entity.read(cx).error.clone()) {
        if e.contains("pdfium") {
            eprintln!(
                "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                     本机无 libpdfium——#386 视图级手势 e2e 未真跑。\n  \
                     有 libpdfium 的环境（MDC_PDFIUM_LIB=<库路径> \
                     cargo test）全量执行。\n═══════════════════════════\n"
            );
            return;
        }
        panic!("页视图构造失败（夹具/真缺陷，非环境缺件）：{e}");
    }
    // ① 手势（生产入口 = finish_drag_annotation 收尾同款
    //    commit_annotation；视口未记录 → fit_w=WIDTH_MIN=256，
    //    屏幕坐标按 256px 宽视图取页 0 上部——同 #336 的 96DPI
    //    量级意图）。
    cx.update(|cx| {
        entity.update(cx, |v, _| {
            v.commit_annotation(0, 40., 60., 200., 120.)
                .expect("#386 起点：用户手势提交");
        })
    });
    // ② 视图脏标（未落盘前必真——Shell 据此驱动关闭确认）。
    assert!(
        cx.update(|cx| entity.read(cx).is_dirty()),
        "#386 验收：手势后视图必脏"
    );
    // ③ 保存（Shell 传入路径——单一真源；此处直驱同款调用面）。
    let file =
        TempPdf(std::env::temp_dir().join(format!("mang-386-gesture-{}.pdf", std::process::id())));
    let stats = cx
        .update(|cx| entity.update(cx, |v, _| v.save_annotations(&file.0)))
        .expect("落盘应成功")
        .expect("脏视图保存 = Ok(Some)");
    // 手势内 commit_annotation 已 flush（产出修订段）——save 的
    // flush 零脏幂等（零追加，不产空修订——#336 语义锁同源）。
    assert_eq!(
        stats.bytes_appended, 0,
        "手势已 flush → save 零脏幂等：{stats:?}"
    );
    // ④ 盘上字节真变了（≠原文件）且保持原前缀（增量非重写——
    // 验收 2 机制面，全量重写在此必红）。
    let on_disk = std::fs::read(&file.0).expect("盘上文件");
    assert!(
        on_disk.starts_with(&original),
        "#386 验收 2：盘上保持原字节前缀（增量）"
    );
    assert!(
        on_disk.len() > original.len(),
        "批注修订段非空（得 {} → {}）",
        original.len(),
        on_disk.len()
    );
    // ⑤ 独立第二意见（mdc-pdf 读面）：1 条高亮在档 + 页号往返。
    let (annots, report) =
        mdc_pdf::annotate::read_annotations(on_disk.clone(), mdc_pdf::security::Limits::default())
            .expect("独立读回");
    assert_eq!(report.parsed, 1, "重开应见 1 条批注：{report:?}");
    assert_eq!(annots.len(), 1);
    assert_eq!(annots[0].page.0, 0, "页号往返");
    // ⑥ 脏标清零（写盘成功才清——原子语义）。
    assert!(
        !cx.update(|cx| entity.read(cx).is_dirty()),
        "保存成功后脏标必清"
    );
    // ⑦ 干净重存 = no-op（Ok(None)，盘上不动）。
    let noop = cx
        .update(|cx| entity.update(cx, |v, _| v.save_annotations(&file.0)))
        .expect("干净保存不报错");
    assert!(noop.is_none(), "干净视图保存 = no-op");
    assert_eq!(
        std::fs::read(&file.0).unwrap(),
        on_disk,
        "no-op 不动盘上字节"
    );
}

/// #386 连续会话：保存后再画 → 再脏 → 再存 → 修订累积（两次手势
/// 两条批注在档——「保存后继续工作」的常见流不丢不重置）。
#[gpui::test]
fn second_gesture_after_save_accumulates(cx: &mut gpui::TestAppContext) {
    let original = smoke_pdf_bytes();
    let entity = cx.update(|cx| cx.new(|cx| PdfPagesView::new(original.clone(), cx)));
    if let Some(e) = cx.update(|cx| entity.read(cx).error.clone()) {
        if e.contains("pdfium") {
            eprintln!(
                "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                     本机无 libpdfium——#386 视图级手势 e2e 未真跑。\n═════\
                     ════════════════════════════\n"
            );
            return;
        }
        panic!("页视图构造失败（夹具/真缺陷，非环境缺件）：{e}");
    }
    let file =
        TempPdf(std::env::temp_dir().join(format!("mang-386-accum-{}.pdf", std::process::id())));
    // 第一笔：手势 → 落盘。
    cx.update(|cx| {
        entity.update(cx, |v, _| {
            v.commit_annotation(0, 40., 60., 200., 120.)
                .expect("手势一");
        })
    });
    cx.update(|cx| entity.update(cx, |v, _| v.save_annotations(&file.0)))
        .expect("落盘一")
        .expect("脏视图必写");
    let after_first = std::fs::read(&file.0).unwrap();
    // 第二笔：再手势（保存后的新改动）→ 再脏 → 再落盘。
    cx.update(|cx| {
        entity.update(cx, |v, _| {
            v.commit_annotation(0, 60., 200., 220., 260.)
                .expect("手势二");
        })
    });
    assert!(
        cx.update(|cx| entity.read(cx).is_dirty()),
        "保存后再画必再脏"
    );
    cx.update(|cx| entity.update(cx, |v, _| v.save_annotations(&file.0)))
        .expect("落盘二")
        .expect("脏视图必写");
    let after_second = std::fs::read(&file.0).unwrap();
    // 增量链：二跳仍以一跳为前缀（连续修订不重写全文）。
    assert!(
        after_second.starts_with(&after_first),
        "第二跳以第一跳产物为前缀（增量链）"
    );
    assert!(after_second.len() > after_first.len());
    // 独立读回：两条批注都在档（不重置不丢）。
    let (annots, report) =
        mdc_pdf::annotate::read_annotations(after_second, mdc_pdf::security::Limits::default())
            .expect("独立读回");
    assert_eq!(report.parsed, 2, "两笔手势 = 两条批注：{report:?}");
    assert_eq!(annots.len(), 2);
    assert!(
        !cx.update(|cx| entity.read(cx).is_dirty()),
        "二次保存后脏标清零"
    );
}

/// #386 干净视图：从未画过 → 保存是显式 no-op（Ok(None)）且**不
/// 创建文件**（干净路径零写盘——「没东西可存」不许产生空动作痕
/// 迹）。
#[gpui::test]
fn clean_view_save_is_explicit_noop(cx: &mut gpui::TestAppContext) {
    let _original = smoke_pdf_bytes();
    let entity = cx.update(|cx| cx.new(|cx| PdfPagesView::new(_original.clone(), cx)));
    if let Some(e) = cx.update(|cx| entity.read(cx).error.clone()) {
        if e.contains("pdfium") {
            eprintln!(
                "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                     本机无 libpdfium——#386 视图级手势 e2e 未真跑。\n═════\
                     ════════════════════════════\n"
            );
            return;
        }
        panic!("页视图构造失败（夹具/真缺陷，非环境缺件）：{e}");
    }
    let file =
        TempPdf(std::env::temp_dir().join(format!("mang-386-noop-{}.pdf", std::process::id())));
    let outcome = cx.update(|cx| entity.update(cx, |v, _| v.save_annotations(&file.0)));
    assert!(
        matches!(&outcome, Ok(None)),
        "干净视图保存 = Ok(None) no-op（得 {outcome:?}）"
    );
    assert!(!file.0.exists(), "no-op 不得创建文件（零写盘）");
    assert!(
        !cx.update(|cx| entity.read(cx).is_dirty()),
        "干净视图恒不脏"
    );
}
