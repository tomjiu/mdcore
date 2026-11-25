//! #47 巨石拆解批④：原 docx_pages.rs 内嵌 `tests` 纯迁出
//!（零行为变化——字节原样剥壳，挂载 #[cfg(test)] + #[path] 在宿主文件）。

use super::*;
use crate::editor::docx_lok_session::TileStats;

/// Boot 判据：内核缺失阶段名（宿主回流口径）。
#[test]
fn boot_no_runtime_gate() {
    assert!(
        Boot::Failed {
            stage: "discover",
            message: "未发现 LibreOffice 运行时".into()
        }
        .is_no_runtime()
    );
    assert!(
        !Boot::Failed {
            stage: "load",
            message: String::new()
        }
        .is_no_runtime()
    );
    assert!(!Boot::Starting.is_no_runtime());
}

/// BGRA → RenderImage：载荷不符拒绝（不 panic）——C1 回归门（几何
/// 收拢 docx_geom 时保留在此，与 `bgra_to_image` 同文件）。
#[test]
fn bgra_to_image_validates() {
    assert!(bgra_to_image(&[], 0, 0).is_none());
    assert!(bgra_to_image(&[0; 4], 2, 2).is_none()); // 长度不符
    assert!(bgra_to_image(&[0u8; 16], 2, 2).is_some());
}

/// 视图态机（无运行时恒绿）：坏路径 → 打开失败态（load 段）；轮询
/// 收尾后停表。真实布局验证失败面板呈现（不白屏）。
#[gpui::test]
fn bad_path_lands_in_failed_boot(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let bad = std::env::temp_dir().join(format!("mdc-docx-missing-{}.docx", std::process::id()));
    let view = cx.update(|app| app.new(|cx| DocxPagesView::new(bad.clone(), cx)));
    // 先真实渲染一次——轮询循环在首次 render 起表（App 面无法
    // spawn 实体任务，pdf_pages 同源纪律），不先渲染则 drain 永不
    // 发生。
    struct Host(gpui::Entity<DocxPagesView>);
    impl gpui::Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
            gpui::div().size_full().child(self.0.clone())
        }
    }
    let window = cx.open_window(gpui::size(px(600.), px(480.)), {
        let view = view.clone();
        move |_, _| Host(view)
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, app| window.draw(app).clear(app));
    // 泵轮询：打开线程（真实墙钟）+ 16ms 拍（测试时钟）并进。
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    loop {
        cx.background_executor
            .advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(2));
        if matches!(
            view.read_with(cx, |v, _| v.boot().clone()),
            Boot::Failed { .. }
        ) {
            break;
        }
        let now = std::time::Instant::now();
        let current = view.read_with(cx, |v, _| v.boot().clone());
        assert!(now < deadline, "8s 内必须到达失败态（当前：{current:?}）");
    }
    let stage = match view.read_with(cx, |v, _| v.boot().clone()) {
        Boot::Failed { stage, .. } => stage,
        other => panic!("必须是失败态：{other:?}"),
    };
    assert_eq!(
        stage, "load",
        "不存在的路径必须落在 load 段（canonicalize 门）"
    );
    visual.update(|window, app| window.draw(app).clear(app));
    assert!(
        visual.debug_bounds("docx-pages-state").is_some(),
        "失败态必须呈现状态面板（不白屏）"
    );
}

/// C2：缩放键域——Ctrl+0 适宽 / Ctrl+1 实际大小 / Ctrl+=/- 步进；
/// 未消费键（无 Ctrl）返回 false；换档后光栅宽按档变化。
#[gpui::test]
fn zoom_key_domain_and_raster_widths(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let path = std::env::temp_dir().join(format!("mdc-docx-zoom-{}.docx", std::process::id()));
    let view = cx.update(|app| app.new(|cx| DocxPagesView::new(path, cx)));
    // 直接注入 Ready 态 + A4 几何 + 视口（视图态机面——显示链真窗口
    // 回归见 cold_start_renders_tiles_via_real_probe；LO 真链由
    // docx_lok_session::real_runtime_* 锁定）。
    view.update(cx, |v, _| {
        v.doc_size = Some((11906, 16838));
        v.boot = Boot::Ready;
        v.viewport_size = (800., 600.);
    });
    struct Host(gpui::Entity<DocxPagesView>);
    impl gpui::Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
            gpui::div().size_full().child(self.0.clone())
        }
    }
    let window = cx.open_window(gpui::size(px(800.), px(600.)), {
        let view = view.clone();
        move |_, _| Host(view)
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, app| window.draw(app).clear(app));
    // 键域：无 Ctrl 不消费。
    view.update(cx, |v, cx| {
        assert!(!v.dispatch_zoom_key("0", false, false, cx));
        assert!(v.dispatch_zoom_key("1", true, false, cx), "Ctrl+1 消费");
        assert_eq!(v.zoom_mode_for_test(), ZoomMode::ActualSize);
        let rw_actual = v.raster_width(800.);
        assert_eq!(
            rw_actual,
            quantize_width((11906 / 15) as u32),
            "实际大小光栅宽"
        );
        // Ctrl+= 步进 → Custom 档起算 = 当前生效倍率（不跳档）。
        assert!(v.dispatch_zoom_key("=", true, false, cx));
        assert!(rw_actual < v.raster_width(800.), "步进放大必须增光栅宽");
        let stepped_pct = v.zoom_pct(800.);
        // 连续两次步进 ×1.25²。
        assert!(v.dispatch_zoom_key("=", true, false, cx));
        assert!(v.zoom_pct(800.) > stepped_pct, "连续步进递增");
        // Ctrl+0 回适宽。
        assert!(v.dispatch_zoom_key("0", true, false, cx));
        assert_eq!(v.zoom_mode_for_test(), ZoomMode::FitWidth);
        assert_eq!(v.raster_width(800.), quantize_width(800));
    });
    visual.update(|window, app| window.draw(app).clear(app));
    assert!(
        visual.debug_bounds("docx-zoom-chip").is_some(),
        "就绪态必须呈现缩放 chip"
    );
}

/// C2：INVALIDATE_TILES 失效链——矩形载荷逐出命中条带（跨光栅宽），
/// 不可解析载荷全量逐出（含在途/失败登记）。
#[gpui::test]
fn invalidate_tiles_evicts_hit_bands(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let path = std::env::temp_dir().join(format!("mdc-docx-inval-{}.docx", std::process::id()));
    let view = cx.update(|app| app.new(|cx| DocxPagesView::new(path, cx)));
    view.update(cx, |v, _| {
        v.doc_size = Some((11906, 16838));
        v.boot = Boot::Ready;
        v.viewport_size = (1024., 600.);
    });
    // 注入缓存/失败/在途登记（两档光栅宽 × 三条带）。
    let img = || Arc::new(bgra_to_image(&[0u8; 16], 2, 2).expect("2x2 载荷必须可成像"));
    view.update(cx, |v, _| {
        for band in [0u32, 1, 2] {
            v.cache_insert((band, 1024), img(), 16);
            v.cache_insert((band, 512), img(), 16);
        }
        v.failed.insert((1, 1024));
        v.pending
            .insert(999, ((2, 1024), std::time::Instant::now()));
    });
    // 条带 1 区域的 twips 矩形（1024px 档 A4：条带 0 ≈ [0, 5907)）。
    view.update(cx, |v, cx| {
        v.handle_callback("INVALIDATE_TILES", "0, 6000, 11906, 100, 0, 0", cx);
    });
    view.read_with(cx, |v, _| {
        assert!(!v.cache.contains_key(&(1, 1024)), "命中条带 1024 档逐出");
        assert!(
            !v.cache.contains_key(&(1, 512)),
            "命中条带 512 档同逐（跨档）"
        );
        assert!(v.cache.contains_key(&(0, 1024)), "未命中条带保留");
        assert!(v.cache.contains_key(&(2, 1024)), "未命中条带保留");
        assert!(
            !v.failed.contains(&(1, 1024)),
            "失败登记随失效清除（可重试）"
        );
        assert!(
            v.pending.values().any(|(k, _)| *k == (2, 1024)),
            "条带 2 不在矩形内——在途保留"
        );
    });
    // 全量失效（不可解析载荷）：连在途一并清（迟到 tile 机械丢弃）。
    view.update(cx, |v, cx| {
        v.handle_callback("INVALIDATE_TILES", "EMPTY, EMPTY, 0, 0, 0, 0", cx);
    });
    view.read_with(cx, |v, _| {
        assert_eq!(v.cache_stats_for_test().1, 0, "全量逐出缓存清零");
        assert_eq!(v.failed_for_test(), 0);
        assert_eq!(v.pending_for_test(), 0, "全量失效必须清在途");
    });
}

/// C2：DOCUMENT_SIZE_CHANGED → 几何更新 + 全量失效 + 滚动钳制
///（无会话态：request_size 无害空转——会话层真链另有锁定）。
#[gpui::test]
fn document_size_changed_updates_geometry(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let path = std::env::temp_dir().join(format!("mdc-docx-size-{}.docx", std::process::id()));
    let view = cx.update(|app| app.new(|cx| DocxPagesView::new(path, cx)));
    view.update(cx, |v, _| {
        v.doc_size = Some((11906, 16838));
        v.boot = Boot::Ready;
        v.viewport_size = (1024., 600.);
        v.cache_insert(
            (0, 1024),
            Arc::new(bgra_to_image(&[0u8; 16], 2, 2).unwrap()),
            16,
        );
    });
    // 无会话：回调不得 panic（request_size 空转）。
    view.update(cx, |v, cx| {
        v.handle_callback("DOCUMENT_SIZE_CHANGED", "11906, 20000", cx);
    });
    // 有会话：注入哑会话？会话不可构造（open_blocking 需真链）——
    // 几何更新路径经 Size 回执口直测：
    view.update(cx, |v, cx| {
        v.apply_size_reply((11906, 20168), cx);
    });
    view.read_with(cx, |v, _| {
        assert_eq!(v.doc_size, Some((11906, 20168)), "几何必须更新");
        assert_eq!(v.cache_stats_for_test().1, 0, "几何变化 → 全量失效");
    });
}

// -----------------------------------------------------------------
// #186：C2 显示链真实窗口回归（探针测量 → 自排帧 → 派单 → 回执
// 缓存 → 位图入树；MockSession 注桩——不手工塞 viewport_size、
// 不依赖 LO 运行时）
// -----------------------------------------------------------------

/// 派单参数记录（canvas_w, canvas_h, tile_x, tile_y, tile_w, tile_h）。
type TileRequest = (usize, usize, i32, i32, i32, i32);

// ======================= #177 像素级验收基线 =======================
// Lead 定性：「现有断言只验几何数值不验像素——C2 缺像素级验收基线」。
// 本组把 MockSession 的哑像素（2×2 全零）升级为**程序化合成文档条带**，
// 并对 cache 里 RenderImage 的 BGRA 原始字节做像素级断言——
// 「渲染出来的像素像不像一份文档」首次有可自动执行的判据。

/// 合成「像文档」的条带 BGRA（自上而下行序，bgra_to_image 契约）。
///
/// 内容清单（生成器即规格——render_fidelity 同款纪律）：
/// - 白底（0xFFFFFFFF 近似——白=±16 口径同 worker stats_bgra）
/// - 顶部标题墨迹带（高 h/12 的 0xFF101010 深墨矩形，居中 70% 宽）
/// - N 行文字行（行高 h/(N+2) 的 80%、行内左侧 60% 为墨迹条纹——
///   行投影呈峰谷的「文字行」形态）
/// - 一条彩色元素行（0xFFD030D0 品红——验彩色通道不丢）
/// - 页缘四周 w/16 留白（对称——文档页边距形态）
///
/// 返回 (bgra, 行峰坐标清单)——峰坐标供断言行数。
fn synthetic_band_bgra(w: usize, h: usize, n_text_rows: usize) -> (Vec<u8>, Vec<usize>) {
    assert!(w > 32 && h > 32, "合成条带需要非退化尺寸");
    let mut px = vec![0xFFu8; w * h * 4]; // 白底 BGRA(B=255)
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            px[i] = 0xFF; // A
            px[i + 1] = 0xFF; // R
            px[i + 2] = 0xFF; // G
            px[i + 3] = 0xFF; // B
        }
    }
    let margin = w / 16;
    let title_h = (h / 12).max(4);
    let mut peaks: Vec<usize> = Vec::new();
    // 标题带（峰 0）——BGRA 布局 [B,G,R,A]：墨三槽 = i..i+2。
    for y in 0..title_h {
        for x in margin..w - margin {
            let i = (y * w + x) * 4;
            px[i] = 0x10;
            px[i + 1] = 0x10;
            px[i + 2] = 0x10;
        }
    }
    peaks.push(0usize);
    // 文字行：峰谷结构（行块间留隔）
    let row_block = (h - title_h - h / 8) / n_text_rows.max(1);
    let ink_h = (row_block * 4 / 5).max(2);
    for r in 0..n_text_rows {
        let y0 = title_h + h / 8 + r * row_block;
        peaks.push(y0);
        for y in y0..(y0 + ink_h).min(h) {
            for x in margin..w - margin {
                // 左侧 60% 文字条纹（斜纹模拟字形墨迹密度）
                if (x / 3) % 2 == 0 {
                    let i = (y * w + x) * 4;
                    px[i] = 0x1A;
                    px[i + 1] = 0x1A;
                    px[i + 2] = 0x1A;
                }
            }
        }
    }
    // 彩色元素行（底部）
    let y0c = h - (h / 16).max(2);
    for y in y0c..h {
        for x in margin..w - margin {
            let i = (y * w + x) * 4;
            px[i] = 0xD0; // B
            px[i + 1] = 0x30; // G
            px[i + 2] = 0xD0; // R → BGRA 显示品红
        }
    }
    (px, peaks)
}

/// 行投影峰检测：对 BGRA 做逐行墨迹密度（非白像素占比），
/// 返回密度序列（0..h）——文字行结构应呈「峰谷交替」。
fn row_ink_profile(bgra: &[u8], w: usize, h: usize) -> Vec<f32> {
    let mut prof = vec![0f32; h];
    for y in 0..h {
        let mut ink = 0usize;
        for x in 0..w {
            let i = (y * w + x) * 4 + 1; // R
            let g = bgra[i + 1];
            let b = bgra[i + 2];
            if bgra[i] < 0xF0 || g < 0xF0 || b < 0xF0 {
                ink += 1;
            }
        }
        prof[y] = ink as f32 / w as f32;
    }
    prof
}

/// 峰计数：profile 中「高于阈值且前后为谷」的峰段数。
fn count_row_peaks(profile: &[f32]) -> usize {
    let thr = 0.25f32;
    let mut peaks = 0usize;
    let mut in_peak = false;
    for &v in profile {
        if v > thr && !in_peak {
            peaks += 1;
            in_peak = true;
        } else if v <= thr && in_peak {
            in_peak = false;
        }
    }
    peaks
}

/// 非白占比（白=±16 近似——worker stats_bgra 同口径）。
fn non_white_ratio(bgra: &[u8]) -> f32 {
    let n = bgra.len() / 4;
    if n == 0 {
        return 0.;
    }
    let mut ink = 0usize;
    for px4 in bgra.chunks_exact(4) {
        // BGRA：b=px4[0] g=px4[1] r=px4[2]（合成器写位在 +1/+2/+3——
        // 与 bgra_to_image 直通语义对齐，此处按布局 BGR 读）
        let (b, g, r) = (px4[0], px4[1], px4[2]);
        if b < 0xF0 || g < 0xF0 || r < 0xF0 {
            ink += 1;
        }
    }
    ink as f32 / n as f32
}

/// #186 测试注桩会话（TileSession seam）：记录派单参数（对账 seq
/// = 记录序）；预置回执队列逐条弹出；`auto_reply` = 派单即回 2×2
/// tile（全绿路径）。Arc 字段共享——clone 后状态同源。
#[derive(Clone)]
struct MockSession {
    requests: Arc<std::sync::Mutex<Vec<TileRequest>>>,
    replies: Arc<std::sync::Mutex<VecDeque<SessionReply>>>,
    /// #50 C3：编辑命令录制（dispatch 断言面——命令串序）。
    edits: Arc<std::sync::Mutex<Vec<String>>>,
    auto_reply: bool,
    /// #177：true = auto 回执带**合成文档像素**（synthetic_band_bgra
    /// 按派单 canvas_w×canvas_h 生成）；false 保持旧哑像素
    /// （2×2 全零——既有状态机测试语义不变）。
    synthetic: bool,
    /// #177：合成行数（文字行峰数——L1 断言对账源）。
    synthetic_rows: usize,
}

impl MockSession {
    fn new(auto_reply: bool) -> Self {
        Self {
            requests: Arc::default(),
            replies: Arc::default(),
            edits: Arc::default(),
            auto_reply,
            synthetic: false,
            synthetic_rows: 4,
        }
    }

    /// #177：合成像素模式（L1/L2 像素基线测试用）。
    fn new_synthetic(rows: usize) -> Self {
        Self {
            requests: Arc::default(),
            replies: Arc::default(),
            edits: Arc::default(),
            auto_reply: true,
            synthetic: true,
            synthetic_rows: rows,
        }
    }
}

impl TileSession for MockSession {
    fn request_tile(
        &self,
        canvas_w: usize,
        canvas_h: usize,
        tile_x: i32,
        tile_y: i32,
        tile_w: i32,
        tile_h: i32,
    ) -> u64 {
        let mut reqs = self.requests.lock().unwrap();
        reqs.push((canvas_w, canvas_h, tile_x, tile_y, tile_w, tile_h));
        let seq = reqs.len() as u64;
        if self.auto_reply {
            let (w, h, bgra, non_white, colored) = if self.synthetic {
                // #177 合成文档像素：行投影峰谷 + 彩色元素（「像不像
                // 一份文档」判据的数据源）。
                let (px, _) = synthetic_band_bgra(canvas_w, canvas_h, self.synthetic_rows);
                let nw = non_white_ratio(&px) as f64;
                let colored = 0.02f64; // 品红元素带实测约 1/16 高度
                (canvas_w, canvas_h, px, nw, colored)
            } else {
                (2usize, 2usize, vec![0u8; 16], 0.5f64, 0.5f64)
            };
            self.replies.lock().unwrap().push_back(SessionReply::Tile {
                seq,
                width: w,
                height: h,
                bgra: Arc::new(bgra),
                sha256: "mock".into(),
                stats: TileStats {
                    unique_colors: 1,
                    non_white_ratio: non_white,
                    colored_ratio: colored,
                },
                elapsed_ms: 1,
            });
        }
        seq
    }
    fn request_size(&self) {}
    fn try_recv(&self) -> Option<SessionReply> {
        self.replies.lock().unwrap().pop_front()
    }
    // #50 C3/C4：编辑命令 mock——录制（命令串）+ 受控回执注入
    //（测试显式 push EditDone/TextDone/SaveDone——对账链可逐步驱动）。
    fn edit_type_text(&mut self, seq: u64, text: &str) -> bool {
        self.edits.lock().unwrap().push(format!("type:{text}"));
        let _ = seq;
        true
    }
    fn edit_uno(&mut self, seq: u64, command: &str, args: Option<&str>, query: bool) -> bool {
        self.edits.lock().unwrap().push(format!(
            "uno:{command}{}{}",
            args.unwrap_or_default(),
            if query { "?q" } else { "" }
        ));
        let _ = seq;
        true
    }
    fn edit_text_of(&mut self, seq: u64, _mime: &str) -> bool {
        self.edits.lock().unwrap().push("text_of".into());
        let _ = seq;
        true
    }
    fn edit_save_as(&mut self, seq: u64, url: &str, format: &str) -> bool {
        self.edits
            .lock()
            .unwrap()
            .push(format!("save:{url}({format})"));
        let _ = seq;
        true
    }
}

/// #50 C3：MockSession 编辑面辅助——受控回执注入（drain 消费链）。
impl MockSession {
    fn push_edit_done(&self, seq: u64, ok: bool, dirty: bool, revision: u64) {
        self.replies
            .lock()
            .unwrap()
            .push_back(SessionReply::EditDone {
                seq,
                ok,
                dirty,
                revision,
                error: if ok { None } else { Some("mock 失败".into()) },
            });
    }
    fn push_text_done(&self, seq: u64, text: &str) {
        self.replies
            .lock()
            .unwrap()
            .push_back(SessionReply::TextDone {
                seq,
                text: text.into(),
                error: None,
            });
    }
    fn push_save_done(&self, seq: u64, ok: bool, revision_saved: u64) {
        self.replies
            .lock()
            .unwrap()
            .push_back(SessionReply::SaveDone {
                seq,
                ok,
                revision_saved,
                error: if ok {
                    None
                } else {
                    Some("mock 保存失败".into())
                },
            });
    }
}

/// 测试宿主（视图满窗——探针量得真实窗口尺寸）。
struct ChainHost(gpui::Entity<DocxPagesView>);

impl gpui::Render for ChainHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div().size_full().child(self.0.clone())
    }
}

/// 注桩：清打开通道（真实 open 线程对不存在路径快速失败——本组
/// 测试聚焦显示链，不与 Starting→Failed 竞态）；mock 会话接管
/// 派单/回执。Ready 经 mock 回执驱动（冷启动真实转换面）。
fn inject_mock(
    view: &gpui::Entity<DocxPagesView>,
    cx: &mut gpui::TestAppContext,
    mock: &MockSession,
) {
    view.update(cx, |v, _| {
        v.open_rx = None;
        v.session = Some(Box::new(mock.clone()));
    });
}

/// 泵到首笔派单（冷链：mock Ready 回执 → Ready 树（探针入树）→
/// paint 量得视口 → 自排帧 → submit_visible 真派单）。返回首笔
/// 派单后的总派单数。
fn pump_until_dispatched(
    cx: &mut gpui::TestAppContext,
    visual: &mut gpui::VisualTestContext,
    view: &gpui::Entity<DocxPagesView>,
    mock: &MockSession,
) -> usize {
    // 帧 1：起表（轮询循环首次 render 起表——App 面无法 spawn）。
    visual.update(|window, app| window.draw(app).clear(app));
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        cx.background_executor
            .advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(2));
        if view.read_with(cx, |v, _| matches!(v.boot(), Boot::Ready)) {
            break;
        }
    }
    assert!(
        view.read_with(cx, |v, _| matches!(v.boot(), Boot::Ready)),
        "mock Ready 回执必须到达（冷启动转换面）"
    );
    // 帧 2：Ready 树入树——本帧 submit_visible 仍无量得视口（守卫
    // 早退）；探针 paint 量得真实窗口尺寸并自排一帧。
    visual.update(|window, app| window.draw(app).clear(app));
    assert!(
        view.read_with(cx, |v, _| v
            .viewport
            .get()
            .is_some_and(|(w, h)| w > 0. && h > 0.)),
        "真实 layout/paint 必须量得非空 docx 视口（canvas 无尺寸假说在此否证/证实）"
    );
    assert!(
        visual.update(|window, app| window.simulate_next_frame(app)) > 0,
        "首帧量得视口必须自排后续帧——等输入才派单 = #186 条带永远「渲染中」"
    );
    // 帧 3：submit_visible 消费视口 → 真派单。
    visual.update(|window, app| window.draw(app).clear(app));
    let n = mock.requests.lock().unwrap().len();
    assert!(n > 0, "Ready + 量得视口后必须已派单（零输入驱动）");
    n
}

/// #186 核心：真实窗口冷启动全链——Starting →（mock Ready 回执）
/// → Ready 树（探针入树）→ paint 量得视口 → on_next_frame 自排
/// → submit_visible 真派单（FitWidth 光栅宽 = 量化视口）→（mock
/// tile 回执）→ 缓存命中 → 位图入真实渲染树（占位框让位）。全
/// 程零输入事件、零手工 viewport 注入；稳态不自排帧（无永久动
/// 画环）。
#[gpui::test]
fn cold_start_renders_tiles_via_real_probe(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let path = std::env::temp_dir().join(format!("mdc-docx-chain-{}.docx", std::process::id()));
    let mock = MockSession::new(true);
    let view = cx.update(|app| app.new(|cx| DocxPagesView::new(path, cx)));
    inject_mock(&view, cx, &mock);
    mock.replies.lock().unwrap().push_back(SessionReply::Ready {
        width_twips: 11906,
        height_twips: 16838,
        kernel_sha256: "mock".into(),
        elapsed_ms: 1,
    });
    let window = cx.open_window(gpui::size(px(800.), px(600.)), {
        let view = view.clone();
        move |_, _| ChainHost(view)
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let n0 = pump_until_dispatched(cx, &mut visual, &view, &mock);
    // 派单口径：FitWidth 光栅宽 = 量化(视口 800)（真探针读数，非手工值）。
    let first_rw = mock.requests.lock().unwrap()[0].0;
    assert_eq!(
        first_rw,
        quantize_width(800) as usize,
        "首笔派单光栅宽必须走真探针口径（FitWidth = 量化视口宽）"
    );
    // 泵轮询：auto tile 回执 → 缓存命中。
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        cx.background_executor
            .advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(2));
        if view.read_with(cx, |v, _| v.cache_stats_for_test().1 > 0) {
            break;
        }
    }
    assert!(
        view.read_with(cx, |v, _| v.cache_stats_for_test().1 > 0),
        "mock tile 回执必须入缓存（在途 {}笔，已派 {}笔）",
        view.read_with(cx, |v, _| v.pending_for_test()),
        n0
    );
    // 帧 4：位图入真实渲染树（占位框让位）。
    visual.update(|window, app| window.draw(app).clear(app));
    let band = visual
        .debug_bounds("docx-visible-band-0")
        .expect("缓存命中的条带必须进入真实渲染树");
    assert!(
        band.size.width > px(100.) && band.size.height > px(100.),
        "条带槽必须占可见尺寸（512px 条带满宽）"
    );
    assert!(
        visual.debug_bounds("docx-band-pending-0").is_none(),
        "缓存命中后占位框必须让位给位图"
    );
    // 稳态：视口不变 → 不再自排帧（无永久动画环）。
    assert_eq!(
        visual.update(|window, app| window.simulate_next_frame(app)),
        0,
        "视口不变不得形成常驻动画环"
    );
}

// ---------------- #177 L1：合成文档像素 → 缓存像素级断言 ----------------

/// L1 主体：mock 合成文档条带（行投影峰谷 + 彩色元素）→ drain →
/// cache RenderImage 读出 → 四断言（尺寸承接 / 逐字节保真 /
/// 非白占比 / 行峰数）。「渲染出来的像素像不像一份文档」首次
/// 有恒绿自动判据（headless——无 LO 依赖）。
#[gpui::test]
fn pixel_baseline_synthetic_band_l1(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let path = std::env::temp_dir().join(format!("mdc-177-l1-{}.docx", std::process::id()));
    let mock = MockSession::new_synthetic(4);
    let view = cx.update(|app| app.new(|cx| DocxPagesView::new(path, cx)));
    inject_mock(&view, cx, &mock);
    mock.replies.lock().unwrap().push_back(SessionReply::Ready {
        width_twips: 11906,
        height_twips: 16838,
        kernel_sha256: "mock".into(),
        elapsed_ms: 1,
    });
    let window = cx.open_window(gpui::size(px(800.), px(600.)), {
        let view = view.clone();
        move |_, _| ChainHost(view)
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    pump_until_dispatched(cx, &mut visual, &view, &mock);
    // 泵到首笔缓存命中（auto 合成像素回执）。
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        cx.background_executor
            .advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(2));
        if view.read_with(cx, |v, _| v.cache_stats_for_test().1 > 0) {
            break;
        }
    }
    assert!(
        view.read_with(cx, |v, _| v.cache_stats_for_test().1 > 0),
        "合成 tile 回执必须入缓存"
    );
    // ---- 像素级断言（#177 核心——对 cache BGRA 原始字节）----
    let band_key = view.read_with(cx, |v, _| {
        v.cache_order
            .iter()
            .next()
            .copied()
            .expect("至少一个缓存条目")
    });
    let (w, h, bytes) = view
        .read_with(cx, |v, _| v.cached_image_for_test(band_key))
        .expect("缓存像素必须可读出（RenderImage::as_bytes）");
    // 断言 1：尺寸承接——派单 canvas_w×canvas_h == 缓存像素尺寸。
    let (req_w, req_h) = {
        let r = mock.requests.lock().unwrap();
        r.iter()
            .find(|(_, _, _, _, _, _)| true)
            .map(|(cw, ch, _, _, _, _)| (*cw, *ch))
            .expect("派单记录非空")
    };
    assert_eq!(
        (w, h),
        (req_w, req_h),
        "尺寸承接：缓存像素尺寸必须等于派单画布尺寸"
    );
    // 断言 2：逐字节保真——读出 BGRA 与合成源全等（直通零变换）。
    let (src_px, _) = synthetic_band_bgra(w, h, 4);
    assert_eq!(
        bytes, src_px,
        "逐字节保真：bgra_to_image 直通——注入像素必须原样到达缓存消费终点"
    );
    // 断言 3：非白占比 ∈ [0.10, 0.85]（「像文档」= 有墨但不糊死）。
    let nw = non_white_ratio(&bytes);
    assert!(
        nw > 0.10 && nw < 0.85,
        "非白占比 {nw:.3} 必须在 (0.10, 0.85)——文字文档形态（非空白也非整页黑）"
    );
    // 断言 4：行投影峰数 == 注入行数 + 1（标题）+ 1（彩带并入末峰或独立峰）。
    let prof = row_ink_profile(&bytes, w, h);
    let peaks = count_row_peaks(&prof);
    assert!(
        (peaks >= 5 && peaks <= 7),
        "行投影峰数 {peaks} 必须接近注入结构（标题 1 + 文字 4 + 彩带 1 = 5~7 峰）——\
             像素里能看到「一行行文字」的文档结构，而非噪声/整块"
    );
    // 断言 5：彩色元素在场（品红 B=R≈0xD0 高于 G）。
    let mut magenta_px = 0usize;
    for px4 in bytes.chunks_exact(4) {
        if px4[0] > 0xC0 && px4[1] < 0x60 && px4[2] > 0xC0 {
            magenta_px += 1;
        }
    }
    assert!(
        magenta_px > (w * h) / 64,
        "彩色元素必须出现在缓存像素（≥1/64 像素）——彩色通道不被管线丢弃"
    );
    drop(visual);
    window.update(cx, |_, _, _| {}).ok();
}

/// L2：INVALIDATE（内容换代）后缓存像素必须换代——旧像素残留
/// = 陈旧渲染（「内容变更必须换像素」的像素级版）。
#[gpui::test]
fn pixel_baseline_invalidate_swaps_pixels_l2(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let path = std::env::temp_dir().join(format!("mdc-177-l2-{}.docx", std::process::id()));
    let mock = MockSession::new_synthetic(4);
    let view = cx.update(|app| app.new(|cx| DocxPagesView::new(path, cx)));
    inject_mock(&view, cx, &mock);
    mock.replies.lock().unwrap().push_back(SessionReply::Ready {
        width_twips: 11906,
        height_twips: 16838,
        kernel_sha256: "mock".into(),
        elapsed_ms: 1,
    });
    let window = cx.open_window(gpui::size(px(800.), px(600.)), {
        let view = view.clone();
        move |_, _| ChainHost(view)
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    pump_until_dispatched(cx, &mut visual, &view, &mock);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        cx.background_executor
            .advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(2));
        if view.read_with(cx, |v, _| v.cache_stats_for_test().1 > 0) {
            break;
        }
    }
    let band_key = view.read_with(cx, |v, _| {
        v.cache_order
            .iter()
            .next()
            .copied()
            .expect("至少一个缓存条目")
    });
    let before = view
        .read_with(cx, |v, _| v.cached_image_for_test(band_key))
        .expect("换代前像素");
    // 内核失效通知（内容变更面——INVALIDATE_TILES 真回调口径，
    // 同 invalidate_tiles_evicts_hit_bands 的 handle_callback 用法）。
    view.update(cx, |v, cx| {
        v.handle_callback("INVALIDATE_TILES", "0, 6000, 11906, 100, 0, 0", cx);
    });
    // 失效即逐出（既有 invalidate_tiles_evicts_hit_bands 断言过）；
    // 再派单 → 同键新合成像素 → 像素内容换代。
    visual.update(|window, app| window.draw(app).clear(app));
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        cx.background_executor
            .advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(2));
        if view
            .read_with(cx, |v, _| v.cached_image_for_test(band_key))
            .is_some()
        {
            break;
        }
    }
    let after = view
        .read_with(cx, |v, _| v.cached_image_for_test(band_key))
        .expect("换代后像素必须重建（失效→再派单→新回执）");
    assert_eq!(
        before.0, after.0,
        "换代后条带尺寸口径不变（同视口同光栅宽）"
    );
    // 换代判据：内容字节必须重建过（同合成器确定性下，至少不得
    // 是逐出前的旧实例残留——用「非白占比重算一致 + 峰结构在」
    // 钉「新像素仍像文档」，并断言换代前像素确实被逐出过）。
    let nw_after = non_white_ratio(&after.2);
    let peaks_after = count_row_peaks(&row_ink_profile(&after.2, after.0, after.1));
    assert!(
        nw_after > 0.10 && nw_after < 0.85 && (5..=7).contains(&peaks_after),
        "换代后像素仍须呈文档形态（非白 {nw_after:.3}，峰 {peaks_after}）——\
             失效重建不得退化为空白/噪声"
    );
    drop(visual);
    window.update(cx, |_, _, _| {}).ok();
}

/// #186：窗口调整 → 探针重量（变更守卫）→ 自排帧 → 新视口口径
/// 再派单（800×600 → 1000×700：光栅宽 832 → 1024）。验收面
/// 「窗口调整后能首屏派单」。
#[gpui::test]
fn resize_remeasures_and_redispatches(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let path = std::env::temp_dir().join(format!("mdc-docx-resize-{}.docx", std::process::id()));
    let mock = MockSession::new(false); // 无回执——聚焦再派单口径
    let view = cx.update(|app| app.new(|cx| DocxPagesView::new(path, cx)));
    inject_mock(&view, cx, &mock);
    mock.replies.lock().unwrap().push_back(SessionReply::Ready {
        width_twips: 11906,
        height_twips: 16838,
        kernel_sha256: "mock".into(),
        elapsed_ms: 1,
    });
    let window = cx.open_window(gpui::size(px(800.), px(600.)), {
        let view = view.clone();
        move |_, _| ChainHost(view)
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let n0 = pump_until_dispatched(cx, &mut visual, &view, &mock);
    // 窗口调整（模拟平台 resize 回调）。
    visual.simulate_resize(gpui::size(px(1000.), px(700.)));
    visual.update(|window, app| window.draw(app).clear(app));
    assert!(
        view.read_with(cx, |v, _| v.viewport.get() == Some((1000., 700.))),
        "resize 后探针必须重量新视口（变更守卫更新 Cell）"
    );
    assert!(
        visual.update(|window, app| window.simulate_next_frame(app)) > 0,
        "视口变更必须自排后续帧（消费新口径）"
    );
    visual.update(|window, app| window.draw(app).clear(app));
    let reqs = mock.requests.lock().unwrap().clone();
    assert!(
        reqs.len() > n0,
        "新视口必须触发再派单（旧键在途/缓存不计入 todo——新光栅宽新键）"
    );
    assert_eq!(
        reqs.last().unwrap().0,
        quantize_width(1000) as usize,
        "再派单光栅宽 = 量化(新视口 1000)"
    );
}

/// #186：缩放换档 → 新光栅宽键域再派单（热启动面；1200 宽窗口
/// FitWidth 1216 ≠ ActualSize 832——口径可区分）。验收面「缩放后
/// 能首屏派单」。
#[gpui::test]
fn zoom_change_redispatches_new_raster(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let path = std::env::temp_dir().join(format!("mdc-docx-zoomrd-{}.docx", std::process::id()));
    let mock = MockSession::new(false);
    let view = cx.update(|app| app.new(|cx| DocxPagesView::new(path, cx)));
    inject_mock(&view, cx, &mock);
    mock.replies.lock().unwrap().push_back(SessionReply::Ready {
        width_twips: 11906,
        height_twips: 16838,
        kernel_sha256: "mock".into(),
        elapsed_ms: 1,
    });
    let window = cx.open_window(gpui::size(px(1200.), px(800.)), {
        let view = view.clone();
        move |_, _| ChainHost(view)
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let n0 = pump_until_dispatched(cx, &mut visual, &view, &mock);
    assert_eq!(
        mock.requests.lock().unwrap()[0].0,
        quantize_width(1200) as usize,
        "1200 宽窗口 FitWidth 光栅宽 = 量化(1200)"
    );
    // Ctrl+1 实际大小（视口命令键域同源入口）。
    view.update(cx, |v, cx| {
        assert!(v.dispatch_zoom_key("1", true, false, cx));
    });
    visual.update(|window, app| window.draw(app).clear(app));
    let reqs = mock.requests.lock().unwrap().clone();
    assert!(reqs.len() > n0, "换档后必须再派单（新光栅宽新键域）");
    assert_eq!(
        reqs.last().unwrap().0,
        quantize_width((11906 / 15) as u32) as usize,
        "实际大小光栅宽 = 量化(文档名义宽 793)"
    );
}

/// #186：在途超时——重派一次，再超转失败面（不无限「渲染中」）。
/// 回拨派单时刻（真实 Instant）绕开 15s 等待；drain 直驱巡检。
#[gpui::test]
fn pending_timeout_redispatches_then_fails(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let path = std::env::temp_dir().join(format!("mdc-docx-tmo-{}.docx", std::process::id()));
    let mock = MockSession::new(false); // 永不回执——卡在途面
    let view = cx.update(|app| app.new(|cx| DocxPagesView::new(path, cx)));
    inject_mock(&view, cx, &mock);
    mock.replies.lock().unwrap().push_back(SessionReply::Ready {
        width_twips: 11906,
        height_twips: 16838,
        kernel_sha256: "mock".into(),
        elapsed_ms: 1,
    });
    let window = cx.open_window(gpui::size(px(800.), px(600.)), {
        let view = view.clone();
        move |_, _| ChainHost(view)
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let n0 = pump_until_dispatched(cx, &mut visual, &view, &mock);
    // 回拨：全部在途条目判超时 → drain 巡检（未过限——清出待重派）。
    // 注：drain 的 notify 会触发测试窗自动重绘——重派在返回前即发生，
    // 可观测面取确定性状态（attempts/failed/派单计数），不赌在途瞬时值。
    view.update(cx, |v, _| {
        for (_, at) in v.pending.values_mut() {
            *at = Instant::now() - TILE_PENDING_TIMEOUT - Duration::from_secs(1);
        }
    });
    view.update(cx, |v, cx| v.drain(cx));
    view.read_with(cx, |v, _| {
        assert_eq!(v.attempts.len(), n0, "全部可见带登记尝试（{n0} 笔）");
        assert!(
            v.attempts.values().all(|a| *a == 1),
            "首次超时 = 尝试 1（重派机会，不转失败）"
        );
        assert_eq!(v.failed_for_test(), 0, "首次超时不转失败");
    });
    // 重派发生（自动重绘或显式帧任一驱动）。
    visual.update(|window, app| window.draw(app).clear(app));
    let after_retry = mock.requests.lock().unwrap().len();
    assert!(
        after_retry > n0,
        "超时后必须重派（首次 {n0} 笔 → 重派后 {after_retry} 笔）"
    );
    // 再回拨 + drain：过限 → 失败面（同键不再派）。
    view.update(cx, |v, _| {
        for (_, at) in v.pending.values_mut() {
            *at = Instant::now() - TILE_PENDING_TIMEOUT - Duration::from_secs(1);
        }
    });
    view.update(cx, |v, cx| v.drain(cx));
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.failed_for_test(),
            n0,
            "二次超时必须转失败面（不无限「渲染中」）"
        );
        assert!(v.attempts.values().all(|a| *a == 2), "尝试计数到上限 2");
    });
    visual.update(|window, app| window.draw(app).clear(app));
    assert_eq!(
        mock.requests.lock().unwrap().len(),
        after_retry,
        "失败面同键不重派"
    );
}

// ------------------------------------------------------------------
// #193：统一请求级超时/重试状态机（受控时间回归——健康静置、慢
// tile、派单丢失、真悬死、晚到回执卫生、代际复位；Linux 本地
// 可全绿，Windows 真机静置证据归 Lead 复验）
// ------------------------------------------------------------------

/// #193 核心回归：健康缓存静置（cache 非空、pending=0、failed=0）
/// ——≥60s 静置无重派、无失败登记、无逐拍 notify（自排帧零）。真机
/// 100s 3118 行 stderr（2850 行看门狗）病灶锁定：旧 known_missing
/// 把「缓存非空」当「缺货」，健康态每拍 log+notify；旧 else 分支
/// 无退出守卫 → 每拍「在途条带转失败」。
#[gpui::test]
fn healthy_cache_quiescent_no_storm(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let path = std::env::temp_dir().join(format!("mdc-docx-quiet-{}.docx", std::process::id()));
    let mock = MockSession::new(true);
    let view = cx.update(|app| app.new(|cx| DocxPagesView::new(path, cx)));
    inject_mock(&view, cx, &mock);
    mock.replies.lock().unwrap().push_back(SessionReply::Ready {
        width_twips: 11906,
        height_twips: 16838,
        kernel_sha256: "mock".into(),
        elapsed_ms: 1,
    });
    let window = cx.open_window(gpui::size(px(800.), px(600.)), {
        let view = view.clone();
        move |_, _| ChainHost(view)
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let n0 = pump_until_dispatched(cx, &mut visual, &view, &mock);
    // 泵到全部可见带入缓存（auto tile 回执全入账——健康态到位）。
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        cx.background_executor
            .advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(1));
        if view.read_with(cx, |v, _| {
            v.cache_stats_for_test().1 > 0 && v.pending_for_test() == 0
        }) {
            break;
        }
    }
    let (_, cache_n) = view.read_with(cx, |v, _| v.cache_stats_for_test());
    assert!(
        cache_n > 0,
        "前置：健康缓存必须非空（auto 回执入账，首派 {n0} 笔）"
    );
    assert_eq!(view.read_with(cx, |v, _| v.pending_for_test()), 0);
    assert_eq!(view.read_with(cx, |v, _| v.failed_for_test()), 0);
    let requests_at_quiesce = mock.requests.lock().unwrap().len();
    // 静置 ≥60s：测试时钟推进 64s（16ms 拍 × 4000），驱动真实轮询
    // 循环（含定时器链全链——非直呼 drain）。
    for _ in 0..3200 {
        cx.background_executor
            .advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
    }
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.cache_stats_for_test().1,
            cache_n,
            "健康静置缓存条目不得增删"
        );
        assert_eq!(v.pending_for_test(), 0, "健康静置不得有新在途");
        assert_eq!(v.failed_for_test(), 0, "健康静置不得登记失败");
        assert_eq!(
            v.watchdog_state_for_test(),
            (0, false),
            "健康覆盖：看门狗零计拍、零补派标记"
        );
    });
    assert_eq!(
        mock.requests.lock().unwrap().len(),
        requests_at_quiesce,
        "健康静置 ≥60s 不得重派（旧实现 125 拍清 pending 全量重派）"
    );
    // 无逐拍 notify：静置期不排任何新帧。
    assert_eq!(
        visual.update(|window, app| window.simulate_next_frame(app)),
        0,
        "静置期零自排帧——旧实现 else 分支逐拍 notify"
    );
}

/// #193 统一超时策略：在途慢 tile（看门狗窗 ×2 过去、键级 15s 未
/// 到）不得被全局看门狗抢先清在途/标失败——旧实现 125 拍清
/// pending 重派、250 拍转失败，绕过 15s×2 键级策略。慢 tile 晚
/// 正常回执后入缓存。
#[gpui::test]
fn slow_tile_survives_watchdog_window(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let path = std::env::temp_dir().join(format!("mdc-docx-slow-{}.docx", std::process::id()));
    let mock = MockSession::new(false); // 不自动回执——tile 「慢」
    let view = cx.update(|app| app.new(|cx| DocxPagesView::new(path, cx)));
    inject_mock(&view, cx, &mock);
    mock.replies.lock().unwrap().push_back(SessionReply::Ready {
        width_twips: 11906,
        height_twips: 16838,
        kernel_sha256: "mock".into(),
        elapsed_ms: 1,
    });
    let window = cx.open_window(gpui::size(px(800.), px(600.)), {
        let view = view.clone();
        move |_, _| ChainHost(view)
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let n0 = pump_until_dispatched(cx, &mut visual, &view, &mock);
    let pending_n = view.read_with(cx, |v, _| v.pending_for_test());
    assert_eq!(pending_n, n0, "前置：首派 {n0} 笔全部在途");
    // 静置 ≈4.8s（300 拍 > 2× 看门狗窗 2s，< 键级 15s）：在途必须
    // 原样保留——不得清在途、不得重派、不得失败。
    for _ in 0..300 {
        cx.background_executor
            .advance_clock(Duration::from_millis(16));
        cx.run_until_parked();
    }
    view.read_with(cx, |v, _| {
        assert_eq!(v.pending_for_test(), n0, "慢 tile 在途不得被看门狗清除");
        assert_eq!(v.failed_for_test(), 0, "键级 15s 未到不得失败");
        assert!(
            v.attempts.is_empty(),
            "键级 15s 未到不得计尝试（旧实现 2s 即抢先）"
        );
    });
    assert_eq!(
        mock.requests.lock().unwrap().len(),
        n0,
        "在途期间不得重派（旧实现 125 拍全量重派）"
    );
    // 慢 tile 回执（seq 对账入账）→ 缓存命中、在途清空。
    for seq in 1..=n0 as u64 {
        mock.replies.lock().unwrap().push_back(SessionReply::Tile {
            seq,
            width: 2,
            height: 2,
            bgra: Arc::new(vec![0u8; 16]),
            sha256: "mock".into(),
            stats: TileStats {
                unique_colors: 1,
                non_white_ratio: 0.5,
                colored_ratio: 0.5,
            },
            elapsed_ms: 4800,
        });
    }
    for _ in 0..10 {
        cx.background_executor
            .advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
    }
    view.read_with(cx, |v, _| {
        assert_eq!(v.cache_stats_for_test().1, n0, "慢 tile 回执必须入缓存");
        assert_eq!(v.pending_for_test(), 0);
        assert_eq!(v.failed_for_test(), 0);
    });
}

/// #193 受控状态机（无窗口直驱 drain——不依赖渲染重派路径）：
/// 覆盖缺口（失效清理后无重派路径）→ 看门狗窗口后补派一次（有
/// 界）→ 真悬死走键级 15s×2（超时→重派→再超时→失败面）→ 失败
/// 后静置无限拍无重派/无失败增长（日志/notify 有界的可观测面）。
#[gpui::test]
fn dispatch_loss_and_hang_unified_strategy(cx: &mut gpui::TestAppContext) {
    let view = cx.update(|app| app.new(|cx| DocxPagesView::new_with_open_rx_for_test(cx, None)));
    let mock = MockSession::new(false); // 永不回执——悬死面
    inject_mock(&view, cx, &mock);
    view.update(cx, |v, _| {
        v.doc_size = Some((11906, 16838));
        v.boot = Boot::Ready;
        v.viewport.set(Some((800., 600.)));
        v.viewport_size = (800., 600.);
        v.viewport_seen = (800., 600.); // refresh_viewport 视为无变更
    });
    // 拍 1..=124：覆盖缺口计拍，不派单（无渲染路径——补派归看门狗）。
    for _ in 0..124 {
        view.update(cx, |v, cx| v.drain_for_test(cx));
    }
    assert_eq!(mock.requests.lock().unwrap().len(), 0, "窗口内不得提前派单");
    assert_eq!(
        view.read_with(cx, |v, _| v.watchdog_state_for_test()),
        (124, false)
    );
    // 第 125 拍：补派一次（缺口闭合——在途覆盖）。
    view.update(cx, |v, cx| v.drain_for_test(cx));
    let n0 = mock.requests.lock().unwrap().len();
    assert!(n0 > 0, "覆盖缺口持续 {WATCHDOG_TICKS} 拍必须补派");
    assert_eq!(
        view.read_with(cx, |v, _| v.watchdog_state_for_test()),
        (0, true),
        "补派后计拍清零、标记已补派"
    );
    assert_eq!(view.read_with(cx, |v, _| v.pending_for_test()), n0);
    // 缺口闭合 → 后续拍静置（补派有界——不得循环重派）。
    for _ in 0..200 {
        view.update(cx, |v, cx| v.drain_for_test(cx));
    }
    assert_eq!(
        mock.requests.lock().unwrap().len(),
        n0,
        "在途覆盖期间不得重复补派（旧实现每 125 拍全量重派）"
    );
    // 真悬死①：键级 15s 超时（回拨）→ 尝试 1、清出在途待重派。
    view.update(cx, |v, _| {
        for (_, at) in v.pending.values_mut() {
            *at = Instant::now() - TILE_PENDING_TIMEOUT - Duration::from_secs(1);
        }
    });
    view.update(cx, |v, cx| v.drain_for_test(cx));
    view.read_with(cx, |v, _| {
        assert!(
            v.attempts.values().all(|a| *a == 1),
            "首次键级超时 = 尝试 1"
        );
        assert_eq!(v.failed_for_test(), 0, "首次超时不转失败");
        assert_eq!(v.pending_for_test(), 0);
    });
    // 无渲染路径：缺口再现 → 看门狗窗口后补派（键级第 2 次尝试）。
    let mut ticks = 0u32;
    while mock.requests.lock().unwrap().len() == n0 {
        view.update(cx, |v, cx| v.drain_for_test(cx));
        ticks += 1;
        assert!(ticks < 200, "悬死重派必须在看门狗窗口内发生");
    }
    let n1 = mock.requests.lock().unwrap().len();
    assert_eq!(n1, n0 * 2, "第二次补派 = 键级重试（15s×2 策略）");
    // 真悬死②：再超时 → 尝试 2 = 上限 → 失败面（诚实呈现）。
    view.update(cx, |v, _| {
        for (_, at) in v.pending.values_mut() {
            *at = Instant::now() - TILE_PENDING_TIMEOUT - Duration::from_secs(1);
        }
    });
    view.update(cx, |v, cx| v.drain_for_test(cx));
    view.read_with(cx, |v, _| {
        assert_eq!(v.failed_for_test(), n0, "二次键级超时必须转失败面");
        assert!(v.attempts.values().all(|a| *a == 2));
    });
    // 失败面静置：无限拍无重派、无失败增长（旧实现 else 分支每拍
    // log+notify 的病灶在此锁死——失败面是稳态）。
    for _ in 0..500 {
        view.update(cx, |v, cx| v.drain_for_test(cx));
    }
    assert_eq!(
        mock.requests.lock().unwrap().len(),
        n1,
        "失败面静置不得重派（失效/换档才可重试）"
    );
    assert_eq!(view.read_with(cx, |v, _| v.failed_for_test()), n0);
    assert_eq!(
        view.read_with(cx, |v, _| v.watchdog_state_for_test()),
        (0, false),
        "失败面覆盖完整——故障周期闭合复位"
    );
}

/// #193 回执卫生：晚到旧回执（seq 不在在途——换代后残留）机械
/// 丢弃——不入缓存、不误失败，也不得复位看门狗计拍（旧 had_tile
/// 把任何 Tile 回执当进账逐拍清零计数 → 新代际缺口永不补派）；
/// 补派后的新 seq 回执正常入账。
#[gpui::test]
fn late_receipts_neither_pollute_nor_reset(cx: &mut gpui::TestAppContext) {
    let view = cx.update(|app| app.new(|cx| DocxPagesView::new_with_open_rx_for_test(cx, None)));
    let mock = MockSession::new(false);
    inject_mock(&view, cx, &mock);
    view.update(cx, |v, _| {
        v.doc_size = Some((11906, 16838));
        v.boot = Boot::Ready;
        v.viewport.set(Some((800., 600.)));
        v.viewport_size = (800., 600.);
        v.viewport_seen = (800., 600.);
    });
    // 看门狗补派 3 带（seq 1..=3）。
    for _ in 0..125 {
        view.update(cx, |v, cx| v.drain_for_test(cx));
    }
    let n0 = mock.requests.lock().unwrap().len();
    assert!(n0 > 0, "前置：缺口补派已发生");
    // 换代：全量失效（清在途/缓存/尝试——晚到回执全部失配）。
    view.update(cx, |v, cx| {
        v.handle_callback("INVALIDATE_TILES", "EMPTY, EMPTY, 0, 0, 0, 0", cx)
    });
    // 每拍塞一笔晚到旧回执 + drain：看门狗必须照常计拍（晚到回执
    // 不复位）——直到窗口满补派新代际。
    let mut fired_at = None;
    for i in 0..150 {
        mock.replies.lock().unwrap().push_back(SessionReply::Tile {
            seq: (i % n0 as u64) + 1, // 旧代际 seq——必然失配
            width: 2,
            height: 2,
            bgra: Arc::new(vec![0u8; 16]),
            sha256: "late".into(),
            stats: TileStats {
                unique_colors: 1,
                non_white_ratio: 0.5,
                colored_ratio: 0.5,
            },
            elapsed_ms: 1,
        });
        view.update(cx, |v, cx| v.drain_for_test(cx));
        if mock.requests.lock().unwrap().len() > n0 {
            fired_at = Some(i);
            break;
        }
    }
    let fired_at = fired_at.expect("晚到回执不得阻止看门狗补派（旧 had_tile 病灶）");
    assert!(
        fired_at + 1 >= WATCHDOG_TICKS as u64,
        "补派必须发生在完整看门狗窗口之后（第 {} 拍）",
        fired_at + 1
    );
    // 晚到回执不污染：缓存仍空、失败仍空（机械丢弃，不入账）。
    view.read_with(cx, |v, _| {
        assert_eq!(v.cache_stats_for_test().1, 0, "晚到回执不得入缓存");
        assert_eq!(v.failed_for_test(), 0, "晚到回执不得误失败");
    });
    // 新代际回执正常入账：补派产生新 seq，推对应回执 → 入缓存。
    let n1 = mock.requests.lock().unwrap().len();
    for seq in (n0 as u64 + 1)..=(n1 as u64) {
        mock.replies.lock().unwrap().push_back(SessionReply::Tile {
            seq,
            width: 2,
            height: 2,
            bgra: Arc::new(vec![0u8; 16]),
            sha256: "new-gen".into(),
            stats: TileStats {
                unique_colors: 1,
                non_white_ratio: 0.5,
                colored_ratio: 0.5,
            },
            elapsed_ms: 1,
        });
    }
    view.update(cx, |v, cx| v.drain_for_test(cx));
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.cache_stats_for_test().1,
            n1 - n0,
            "新代际回执必须正常入账"
        );
        assert_eq!(v.pending_for_test(), 0);
    });
}

/// #193 代际切换复位：视口变更（缩放/窗口调整同源路径）复位看门
/// 狗故障周期——新代际重新获得补派预算，且新视口口径立即派单。
#[gpui::test]
fn viewport_generation_resets_watchdog_cycle(cx: &mut gpui::TestAppContext) {
    let view = cx.update(|app| app.new(|cx| DocxPagesView::new_with_open_rx_for_test(cx, None)));
    let mock = MockSession::new(false);
    inject_mock(&view, cx, &mock);
    view.update(cx, |v, _| {
        v.doc_size = Some((11906, 16838));
        v.boot = Boot::Ready;
        v.viewport.set(Some((800., 600.)));
        v.viewport_size = (800., 600.);
        v.viewport_seen = (800., 600.);
    });
    // 模拟故障周期中途（计拍过半、已补派）。
    view.update(cx, |v, _| {
        v.watchdog_ticks = WATCHDOG_TICKS - 1;
        v.watchdog_retried = true;
    });
    // 视口变更（探针量得新尺寸 → refresh_viewport 变更分支收口）。
    view.update(cx, |v, cx| {
        v.viewport.set(Some((1000., 700.)));
        v.drain_for_test(cx);
    });
    assert_eq!(
        view.read_with(cx, |v, _| v.watchdog_state_for_test()),
        (0, false),
        "视口代际切换必须复位故障周期"
    );
    assert!(
        !mock.requests.lock().unwrap().is_empty(),
        "新视口口径必须立即派单（不等待看门狗窗口）"
    );
    // 补派预算已复位：新代际缺口同样享有一次补派（隐含于上——
    // 若未复位，retried=true 会令下一窗口直接登记失败面）。
    view.read_with(cx, |v, _| {
        assert_eq!(v.failed_for_test(), 0, "代际复位后不得直接登记失败面");
    });
}

// ------------------------------------------------------------------
// #179：打开通道/会话通道消费语义（受控时序回归——无真实线程，
// `drain` 逐拍直驱；覆盖验收场景 1/2/3 与「会话通道异常断开」
// 补充场景。真实线程 panic 捕获链见 new 的 catch_unwind——真链
// 行为由 bad_path_lands_in_failed_boot 锁定）。
// ------------------------------------------------------------------

/// 场景 1：句柄与 Ready 同拍到达，发送端随后正常关闭——后续多拍
/// 最终仍是 Ready。旧实现在 `Ok(Ok(session))` 后放回 `open_rx`，
/// 打开线程正常退出的通道关闭在下一拍被 `Disconnected` 当成异常，
/// 把 Ready 覆盖成通用「打开线程异常退出」。
#[gpui::test]
fn open_handle_and_ready_same_tick_then_close_stays_ready(cx: &mut gpui::TestAppContext) {
    let (open_tx, open_rx) = channel();
    let (reply_tx, reply_rx) = channel();
    reply_tx
        .send(SessionReply::Ready {
            width_twips: 11906,
            height_twips: 16838,
            kernel_sha256: "test-sha".into(),
            elapsed_ms: 1,
        })
        .unwrap();
    open_tx
        .send(Ok(DocxLokSession::from_reply_channel_for_test(reply_rx)))
        .unwrap();
    drop(open_tx); // 打开线程「回传后正常退出」——通道随之关闭。
    let view =
        cx.update(|app| app.new(|cx| DocxPagesView::new_with_open_rx_for_test(cx, Some(open_rx))));
    // 拍 1：句柄 + Ready 同拍。
    view.update(cx, |v, cx| v.drain(cx));
    assert_eq!(view.read_with(cx, |v, _| v.boot.clone()), Boot::Ready);
    assert_eq!(
        view.read_with(cx, |v, _| v.doc_size),
        Some((11906, 16838)),
        "Ready 几何落位"
    );
    // 后续多拍（旧实现在此被 Disconnected 覆盖成 Failed(spawn)）。
    for _ in 0..5 {
        view.update(cx, |v, cx| v.drain(cx));
    }
    assert_eq!(
        view.read_with(cx, |v, _| v.boot.clone()),
        Boot::Ready,
        "回传后的正常通道关闭不得覆盖 Ready（#179）"
    );
    // 会话通道仍在（reply_tx 存活）——会话不被误拆。
    assert!(view.read_with(cx, |v, _| v.session.is_some()));
}

/// 场景 2：打开线程发送结果前异常退出（通道断开且无结果）——
/// 落 `Failed(spawn)` 且文案明确「未回传结果」，不报成正常就绪；
/// 多拍幂等。
#[gpui::test]
fn open_thread_exit_before_result_lands_failed_spawn(cx: &mut gpui::TestAppContext) {
    let (open_tx, open_rx) = channel();
    drop(open_tx); // 线程未回传任何结果即退出。
    let view =
        cx.update(|app| app.new(|cx| DocxPagesView::new_with_open_rx_for_test(cx, Some(open_rx))));
    view.update(cx, |v, cx| v.drain(cx));
    match view.read_with(cx, |v, _| v.boot.clone()) {
        Boot::Failed { stage, message } => {
            assert_eq!(stage, "spawn", "阶段可定位");
            assert!(message.contains("未回传结果"), "文案可定位：{message}");
        }
        other => panic!("必须是 spawn 段失败：{other:?}"),
    }
    // 多拍幂等（不反复迁移/不假就绪）。
    for _ in 0..3 {
        view.update(cx, |v, cx| v.drain(cx));
    }
    assert!(matches!(
        view.read_with(cx, |v, _| v.boot.clone()),
        Boot::Failed { stage: "spawn", .. }
    ));
}

/// 场景 3：句柄与真实 Failed 同拍到达，打开通道随后正常关闭——
/// 最初的具体失败阶段/原因保留，不被通用 spawn 文案覆盖；同拍
/// 断开的会话通道亦不误报 session 段（终态回执已见）。
#[gpui::test]
fn open_handle_and_concrete_failed_same_tick_preserves_stage(cx: &mut gpui::TestAppContext) {
    let (open_tx, open_rx) = channel();
    let (reply_tx, reply_rx) = channel();
    reply_tx
        .send(SessionReply::Failed {
            stage: "load",
            message: "documentLoadWithOptions 失败：0xC0000005".into(),
        })
        .unwrap();
    open_tx
        .send(Ok(DocxLokSession::from_reply_channel_for_test(reply_rx)))
        .unwrap();
    drop(open_tx);
    drop(reply_tx); // 打开/会话双通道随后关闭。
    let view =
        cx.update(|app| app.new(|cx| DocxPagesView::new_with_open_rx_for_test(cx, Some(open_rx))));
    // 拍 1：句柄 + 具体 Failed 同拍。
    view.update(cx, |v, cx| v.drain(cx));
    match view.read_with(cx, |v, _| v.boot.clone()) {
        Boot::Failed { stage, message } => {
            assert_eq!(stage, "load", "具体阶段保留");
            assert!(
                message.contains("documentLoadWithOptions"),
                "具体原因保留：{message}"
            );
        }
        other => panic!("必须是 load 段失败：{other:?}"),
    }
    // 拍 2+：双通道已断——具体失败不得被通用文案覆盖。
    for _ in 0..3 {
        view.update(cx, |v, cx| v.drain(cx));
    }
    match view.read_with(cx, |v, _| v.boot.clone()) {
        Boot::Failed { stage: "load", .. } => {}
        other => panic!("具体失败不得被覆盖：{other:?}"),
    }
}

/// 补充场景：会话通道异常断开（无终态回执）——与「暂时无消息」
/// 分流：先证 Empty 不动状态/不拆会话，再断开 → `Failed(session)`
/// 可定位 + 停轮询（会话拆），不永挂 Starting。
#[gpui::test]
fn session_channel_disconnect_without_terminal_lands_failed(cx: &mut gpui::TestAppContext) {
    let (open_tx, open_rx) = channel();
    let (reply_tx, reply_rx) = channel();
    open_tx
        .send(Ok(DocxLokSession::from_reply_channel_for_test(reply_rx)))
        .unwrap();
    drop(open_tx);
    let view =
        cx.update(|app| app.new(|cx| DocxPagesView::new_with_open_rx_for_test(cx, Some(open_rx))));
    // 拍 1：仅句柄（会话通道 Empty——「暂时无消息」不动状态）。
    view.update(cx, |v, cx| v.drain(cx));
    assert_eq!(
        view.read_with(cx, |v, _| v.boot.clone()),
        Boot::Starting,
        "Empty 不得误判"
    );
    assert!(
        view.read_with(cx, |v, _| v.session.is_some()),
        "Empty 不拆会话"
    );
    // 会话线程异常退出：未发终态回执即断开。
    drop(reply_tx);
    view.update(cx, |v, cx| v.drain(cx));
    match view.read_with(cx, |v, _| v.boot.clone()) {
        Boot::Failed { stage, message } => {
            assert_eq!(stage, "session", "可定位到会话段");
            assert!(message.contains("会话线程"), "文案可定位：{message}");
        }
        other => panic!("必须是 session 段失败：{other:?}"),
    }
    assert!(
        view.read_with(cx, |v, _| v.session.is_none()),
        "停轮询（会话拆）"
    );
}

// -----------------------------------------------------------------
// #50 C3/C4：编辑链测试（mock 命令录制 + 受控回执驱动状态面）
// -----------------------------------------------------------------

/// 编辑链就绪视图 + 宿主窗（Ready + mock 会话——真实 window 域内
/// dispatch；ChainHost 同款）。
fn edit_ready_view(
    cx: &mut gpui::TestAppContext,
) -> (
    gpui::Entity<DocxPagesView>,
    MockSession,
    gpui::VisualTestContext,
) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let path = std::env::temp_dir().join(format!("mdc-docx-edit-{}.docx", std::process::id()));
    let mock = MockSession::new(false);
    let view = cx.update(|app| app.new(|cx| DocxPagesView::new(path, cx)));
    inject_mock(&view, cx, &mock);
    mock.replies.lock().unwrap().push_back(SessionReply::Ready {
        width_twips: 11906,
        height_twips: 16838,
        kernel_sha256: "mock".into(),
        elapsed_ms: 1,
    });
    let window = cx.open_window(gpui::size(gpui::px(800.), gpui::px(600.)), {
        let view = view.clone();
        move |_, _| ChainHost(view)
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, cx| window.draw(cx).clear(cx));
    (view, mock, visual)
}

/// 单字符键入：dispatch_edit_key → type 命令 + EditDone 回执 → dirty 置位。
#[gpui::test]
fn edit_key_types_and_lands_dirty(cx: &mut gpui::TestAppContext) {
    let (view, mock, mut visual) = edit_ready_view(cx);
    view.update(cx, |v, cx| v.drain_for_test(cx));
    visual.update(|window, cx| {
        view.update(cx, |v, cx| {
            assert!(v.dispatch_edit_key("a", false, false, window, cx));
        });
    });
    let edits = mock.edits.lock().unwrap().clone();
    assert_eq!(edits, vec!["type:a".to_string()], "字符键 → type 命令");
    // 受控回执：成功编辑（dirty=true, rev=1）。
    let seq = next_edit_seq_for_test();
    mock.push_edit_done(seq - 1, true, true, 1);
    view.update(cx, |v, cx| v.drain_for_test(cx));
    view.read_with(cx, |v, _| {
        assert!(v.is_dirty(), "EditDone(ok,dirty) 回执置位脏位");
    });
}

/// Ctrl+Z/Y → uno Undo/Redo 命令（录制断言）。
#[gpui::test]
fn edit_undo_redo_dispatch_uno(cx: &mut gpui::TestAppContext) {
    let (view, mock, mut visual) = edit_ready_view(cx);
    view.update(cx, |v, cx| v.drain_for_test(cx));
    visual.update(|window, cx| {
        view.update(cx, |v, cx| {
            assert!(v.dispatch_edit_key("z", true, false, window, cx));
            assert!(v.dispatch_edit_key("y", true, false, window, cx));
        });
    });
    let edits = mock.edits.lock().unwrap().clone();
    assert_eq!(
        edits,
        vec!["uno:.uno:Undo".to_string(), "uno:.uno:Redo".to_string()],
        "Ctrl+Z/Y → undo/redo uno 命令"
    );
}

/// Ctrl+C → text_of 读选区 + TextDone 回执写剪贴板（读回断言）。
#[gpui::test]
fn edit_copy_reads_selection_to_clipboard(cx: &mut gpui::TestAppContext) {
    let (view, mock, mut visual) = edit_ready_view(cx);
    view.update(cx, |v, cx| v.drain_for_test(cx));
    visual.update(|window, cx| {
        view.update(cx, |v, cx| {
            assert!(v.dispatch_edit_key("c", true, false, window, cx));
        });
    });
    let seq = next_edit_seq_for_test();
    mock.push_text_done(seq - 1, "选中文本");
    view.update(cx, |v, cx| v.drain_for_test(cx));
    // 剪贴板读回（写进系统剪贴板——测试面读回验证）。
    let pasted = view.read_with(cx, |v, cx| v.clipboard_read(cx));
    assert_eq!(pasted.as_deref(), Some("选中文本"), "TextDone → 剪贴板写");
}

/// C4 保存对账：SaveDone.revision_saved == 当前 revision 才清 dirty；
/// 落后（期间又有新编辑）不清——「延迟回执清新 dirty」病灶钉死。
#[gpui::test]
fn save_revision_reconciliation_guards_stale_receipt(cx: &mut gpui::TestAppContext) {
    let (view, mock, mut visual) = edit_ready_view(cx);
    view.update(cx, |v, cx| v.drain_for_test(cx));
    // 编辑 → dirty（rev 1）。
    let seq = next_edit_seq_for_test();
    mock.push_edit_done(seq - 1, true, true, 1);
    // 保存发起（save 命令 + SaveDone rev=1——与当前相等 → 清）。
    view.update(cx, |v, cx| {
        assert!(v.request_save(cx), "保存发起");
    });
    view.update(cx, |v, cx| v.drain_for_test(cx));
    let save_seq = next_edit_seq_for_test();
    mock.push_save_done(save_seq - 1, true, 1);
    view.update(cx, |v, cx| v.drain_for_test(cx));
    view.read_with(cx, |v, _| {
        assert!(!v.is_dirty(), "revision 相等的保存回执清 dirty");
    });

    // 再编辑（rev 2）+ 陈旧保存回执（rev=1 落后）——不清。
    let seq2 = next_edit_seq_for_test();
    mock.push_edit_done(seq2 - 1, true, true, 2);
    view.update(cx, |v, cx| v.drain_for_test(cx));
    let stale_seq = next_edit_seq_for_test();
    mock.push_save_done(stale_seq - 1, true, 1);
    view.update(cx, |v, cx| v.drain_for_test(cx));
    view.read_with(cx, |v, _| {
        assert!(v.is_dirty(), "落后 revision 的保存回执不清新编辑的 dirty");
    });
    let _ = &mut visual;
}

/// 未就绪态（Starting）保存如实不发起——不可编辑不假绿（键面卫
/// 同源：`Boot` 非 Ready 时 dispatch_edit_key 恒 false——就绪卫与
/// request_save 同一判据）。
#[gpui::test]
fn edit_keys_not_consumed_before_ready(cx: &mut gpui::TestAppContext) {
    let path = std::env::temp_dir().join("mdc-docx-nr.docx");
    let mock = MockSession::new(false);
    let view = cx.update(|app| app.new(|cx| DocxPagesView::new(path, cx)));
    inject_mock(&view, cx, &mock);
    // 无 Ready 回执——Boot::Starting（request_save 无 window 面：
    // 就绪卫直接断言；dispatch 键面同卫同判据）。
    view.update(cx, |v, cx| {
        assert!(!v.request_save(cx), "Starting 态保存不发起（就绪卫）");
    });
    view.read_with(cx, |v, _| {
        assert!(
            !matches!(v.boot, Boot::Ready),
            "未就绪态（boot 卫同判据——键面同 false）"
        );
    });
}

/// #50 C3 修正：IME 全链——mark 组合（preedit 宿主域）→ None 提交
/// （出清 + type_text 下发）→ EditDone 置 dirty。修前病灶：提交
/// 文本只堆 preedit 字符串、永不到内核——本测试钉死。
#[gpui::test]
fn ime_mark_commit_lands_in_kernel(cx: &mut gpui::TestAppContext) {
    let (view, mock, mut visual) = edit_ready_view(cx);
    view.update(cx, |v, cx| v.drain_for_test(cx));
    // 组合：replace_and_mark（「中文」进 preedit——不下发）。
    visual.update(|window, cx| {
        view.update(cx, |v, cx| {
            gpui::EntityInputHandler::replace_and_mark_text_in_range(
                v, None, "中文", None, window, cx,
            );
        });
    });
    view.read_with(cx, |v, _| assert_eq!(v.preedit_for_test(), "中文"));
    assert!(
        mock.edits.lock().unwrap().is_empty(),
        "组合期零命令（preedit 宿主渲染——契约 §10）"
    );
    // 提交：replace_text_in_range(None, 定稿)。
    visual.update(|window, cx| {
        view.update(cx, |v, cx| {
            gpui::EntityInputHandler::replace_text_in_range(v, None, "中文", window, cx);
        });
    });
    view.read_with(cx, |v, _| {
        assert!(
            v.preedit_for_test().is_empty(),
            "提交后 preedit 出清（浮层消失）"
        );
    });
    assert_eq!(
        mock.edits.lock().unwrap()[0],
        "type:中文",
        "提交定稿经 type_text 下发内核（修前永不到内核的病灶）"
    );
    // EditDone 回执 → dirty。
    let seq = next_edit_seq_for_test();
    mock.push_edit_done(seq - 1, true, true, 1);
    view.update(cx, |v, cx| v.drain_for_test(cx));
    view.read_with(cx, |v, _| assert!(v.is_dirty()));
}

/// #50 C3 修正：IME 取消（None + 空串）——出清 preedit、零命令
/// （取消不是编辑：无选区替换、无 dirty 置位）。
#[gpui::test]
fn ime_cancel_clears_preedit_without_commands(cx: &mut gpui::TestAppContext) {
    let (view, mock, mut visual) = edit_ready_view(cx);
    view.update(cx, |v, cx| v.drain_for_test(cx));
    visual.update(|window, cx| {
        view.update(cx, |v, cx| {
            gpui::EntityInputHandler::replace_and_mark_text_in_range(
                v, None, "组合", None, window, cx,
            );
        });
    });
    // 取消（Windows IME 协议形态：range=None + 空串）。
    visual.update(|window, cx| {
        view.update(cx, |v, cx| {
            gpui::EntityInputHandler::replace_text_in_range(v, None, "", window, cx);
        });
    });
    view.read_with(cx, |v, _| {
        assert!(v.preedit_for_test().is_empty(), "取消出清组合串");
    });
    assert!(
        mock.edits.lock().unwrap().is_empty(),
        "取消零命令（不 type_text 不置 dirty）"
    );
    view.read_with(cx, |v, _| assert!(!v.is_dirty()));
}

/// #50 C3 终态清理：Died 回执 → edit_pending 出清（在途命令面
/// 归零——保存/编辑「进行中」误判根除）+ 后续键入零 panic。
#[gpui::test]
fn session_died_clears_edit_pending(cx: &mut gpui::TestAppContext) {
    let (view, mock, mut visual) = edit_ready_view(cx);
    view.update(cx, |v, cx| v.drain_for_test(cx));
    // 编辑在途（命令已发、回执未回）。
    visual.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.dispatch_edit_key("a", false, false, window, cx);
        });
    });
    assert_eq!(view.read_with(cx, |v, _| v.edit_pending_len_for_test()), 1);
    // worker 死亡终态（会话通道回执）。
    mock.replies.lock().unwrap().push_back(SessionReply::Died {
        detail: "mock 死亡".into(),
    });
    view.update(cx, |v, cx| v.drain_for_test(cx));
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.edit_pending_len_for_test(),
            0,
            "终态出清在途编辑命令（回执永不再来）"
        );
        assert!(matches!(v.boot, Boot::Failed { .. }), "终态落 Failed 呈现");
        assert!(!v.is_dirty(), "未确认的编辑不虚报脏位");
    });
    // 死后键入：就绪卫拒收（不 panic、不假绿）。
    visual.update(|window, cx| {
        view.update(cx, |v, cx| {
            assert!(!v.dispatch_edit_key("a", false, false, window, cx));
        });
    });
}

/// 测试辅助：编辑 seq 探针（EDIT_SEQ 静态域——只读非破坏）。
fn next_edit_seq_for_test() -> u64 {
    EDIT_SEQ.load(std::sync::atomic::Ordering::Relaxed) + 1
}
