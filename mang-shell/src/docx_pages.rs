//! #50 C1：DOCX 页面视图——LOK 内核真源渲染（连续画布条带模型）。
//!
//! **定位**：docx 的「页面模式」主显示面（对照 pdf 的
//! [`crate::pdf_pages::PdfPagesView`]——同一视图槽位纪律：壳层
//! `Option<(PathBuf, Entity)>` 按标签路径持有，状态栏「页面/文本」
//! 钮切换）。编辑真源 = LOK 内核（外部 worker 进程，[`crate::editor::
//! docx_lok_session`] 会话层）；行投影编辑器与页面视图经壳层互斥
//! （切换卫：文本态有未保存编辑拒切页面态——单一编辑真源）。
//!
//! **渲染模型（C1）**：LOK Writer 文档是连续画布（`size()` twips）。
//! 适宽（fit-width）单缩放：整文档宽缩进视口宽，画布纵向切 **512px
//! 条带**，每条带一次 `paint`（twips 区域 → 画布像素；排版/图片/
//! 页眉页脚全交内核）。条带几何纯函数（单测锚点：覆盖性/末带钳制/
//! 无重叠）。可见区 ±1 观测余量派单，LRU 预算缓存，16ms 轮询回收执
//! （pdf_pages 搜索索引/OCR 同款纪律）。
//!
//! **C1 边界（如实）**：只读渲染（键鼠输入 C3、保存 C4、缩放档 C2、
//! 回调缓存失效 C2）。内核缺失/装载失败 → 明确错误面板 + 文本视图
//! 后备提示（不白屏、不假绿）。

use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use gpui::{
    AnyElement, App, Context, FocusHandle, Focusable, ImageSource, UniformListScrollHandle, Window,
    px,
};
use image::{Frame, ImageBuffer};
use smallvec::SmallVec;
use ui::prelude::*;

use crate::editor::docx_lok_session::{DocxLokSession, SessionError, SessionReply, TWIPS_PER_PX};

/// 条带显示高（px）——画布纵向切片粒度（渲染中占位/缓存键基准）。
const BAND_PX: u32 = 512;
/// 适宽光栅宽量化步进（pdf_pages `WIDTH_QUANTUM` 同款——视口微抖不换缓存键）。
const WIDTH_QUANTUM: u32 = 64;
/// 适宽光栅宽下/上限（px）。
const WIDTH_MIN: u32 = 256;
const WIDTH_MAX: u32 = 4096;
/// 条带位图缓存预算（字节）——LRU 逐出（512px 条带约 2MB/条常态）。
const CACHE_BUDGET_BYTES: usize = 64 * 1024 * 1024;
/// 回执轮询周期（16ms——pdf_pages `POLL_INTERVAL` 同款）。
const POLL_INTERVAL: Duration = Duration::from_millis(16);

// ---------------------------------------------------------------------------
// 条带几何（纯函数——单测锚点）
// ---------------------------------------------------------------------------

/// 适宽光栅宽：视口宽 → 量化钳制区间（单一口径：派单/槽渲染同源）。
pub fn quantize_width(w: u32) -> u32 {
    let clamped = w.clamp(WIDTH_MIN, WIDTH_MAX);
    clamped.div_ceil(WIDTH_QUANTUM) * WIDTH_QUANTUM
}

/// 适宽缩放（显示 px / 文档 px）。文档宽必须 > 0。
fn fit_scale(doc_w_tw: i64, view_w: u32) -> f64 {
    let doc_w_px = doc_w_tw as f64 / TWIPS_PER_PX as f64;
    debug_assert!(doc_w_px > 0.);
    view_w as f64 / doc_w_px.max(1.)
}

/// 总显示高（px）：`doc_h_px * s`（两轴同缩放——适宽语义）。
fn content_height_px(doc_w_tw: i64, doc_h_tw: i64, view_w: u32) -> f64 {
    let doc_h_px = doc_h_tw as f64 / TWIPS_PER_PX as f64;
    doc_h_px * fit_scale(doc_w_tw, view_w)
}

/// 条带边界（twips，严格递增，首恒 0、末恒 `doc_h_tw`）：第 i 条带显示
/// 区 `[i*BAND_PX, (i+1)*BAND_PX)` 换算为 `floor(i*BAND_PX*tw/px)`——
/// **同一边界点两侧同取 floor**（相邻条带首尾相接零重叠；去重消除
/// 高倍缩放下不足 1 twip 的空条带）。
fn band_boundaries(doc_w_tw: i64, doc_h_tw: i64, view_w: u32) -> Vec<i64> {
    if doc_w_tw <= 0 || doc_h_tw <= 0 || view_w == 0 {
        return Vec::new();
    }
    let tw_per_disp_px = TWIPS_PER_PX as f64 / fit_scale(doc_w_tw, view_w);
    let total_disp = content_height_px(doc_w_tw, doc_h_tw, view_w);
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
pub fn band_count(doc_w_tw: i64, doc_h_tw: i64, view_w: u32) -> usize {
    band_boundaries(doc_w_tw, doc_h_tw, view_w)
        .len()
        .saturating_sub(1)
}

/// 条带几何：显示条带 `band` 的内核请求区域（twips）与画布（px）。
pub struct BandGeom {
    /// 画布宽（px）＝适宽光栅宽。
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
pub fn band_region(doc_w_tw: i64, doc_h_tw: i64, view_w: u32, band: u32) -> Option<BandGeom> {
    let bounds = band_boundaries(doc_w_tw, doc_h_tw, view_w);
    if bounds.len() < 2 || band as usize + 1 >= bounds.len() {
        return None;
    }
    let tw_per_disp_px = TWIPS_PER_PX as f64 / fit_scale(doc_w_tw, view_w);
    let tile_y = bounds[band as usize];
    let tile_end = bounds[band as usize + 1];
    debug_assert!(tile_end > tile_y);
    let tile_h = tile_end - tile_y;
    let canvas_h = ((tile_h as f64 / tw_per_disp_px).ceil() as u32).max(1);
    Some(BandGeom {
        canvas_w: view_w,
        canvas_h,
        tile_x: 0,
        tile_y: tile_y.min(i32::MAX as i64) as i32,
        tile_w: doc_w_tw.min(i32::MAX as i64) as i32,
        tile_h: tile_h.min(i32::MAX as i64) as i32,
    })
}

/// BGRA（自上而下）→ `gpui::RenderImage`（字节序直配——LO Linux cairo
/// tile 缓冲即 BGRA；`pdf_pages::rendered_page_to_image` 同款搬运）。
fn bgra_to_image(bgra: &[u8], width: usize, height: usize) -> Option<gpui::RenderImage> {
    if bgra.len() != width * height * 4 || width == 0 || height == 0 {
        return None;
    }
    let buffer = ImageBuffer::from_raw(width as u32, height as u32, bgra.to_vec())?;
    Some(gpui::RenderImage::new(SmallVec::from_elem(
        Frame::new(buffer),
        1,
    )))
}

// ---------------------------------------------------------------------------
// 视图
// ---------------------------------------------------------------------------

/// 会话启动状态（C1 门：加载错误/内核缺失明确呈现）。
#[derive(Debug, Clone, PartialEq)]
pub enum Boot {
    /// 打开链进行中（worker boot + load——首次可达数十秒）。
    Starting,
    /// 就绪（文档几何已知，条带可派单）。
    Ready,
    /// 终态失败（阶段名 + 原文；`stage == "discover"` = 内核缺失）。
    Failed {
        stage: &'static str,
        message: String,
    },
}

impl Boot {
    /// 内核缺失判据（宿主回流文本视图口径——`SessionError::NoRuntime`）。
    pub fn is_no_runtime(&self) -> bool {
        matches!(
            self,
            Self::Failed {
                stage: "discover",
                ..
            }
        )
    }
}

/// docx 页面视图实体（LOK 连续画布条带渲染——见模块头）。
pub struct DocxPagesView {
    /// 源文件绝对路径（标签路径绑定单一真源）。
    path: PathBuf,
    boot: Boot,
    /// 打开线程回传通道（会话句柄/打开错误；终态置 None）。
    open_rx: Option<Receiver<Result<DocxLokSession, SessionError>>>,
    session: Option<DocxLokSession>,
    /// 文档几何（twips——Ready 后有效）。
    doc_size: Option<(i64, i64)>,
    /// 条带位图缓存（键 = (条带, 适宽光栅宽)）。
    cache: HashMap<(u32, u32), Arc<gpui::RenderImage>>,
    /// 缓存成本记账（键 → 字节——LRU 逐出扣减同源）。
    cache_costs: HashMap<(u32, u32), usize>,
    /// LRU 序（队首冷——逐出依据）。
    cache_order: VecDeque<(u32, u32)>,
    cache_bytes: usize,
    /// 派单在途（seq → 条带键——回执 seq 对账）。
    pending: HashMap<u64, (u32, u32)>,
    /// 单次失败登记（同键不重派——诚实失败面）。
    failed: HashSet<(u32, u32)>,
    polling: bool,
    scroll: UniformListScrollHandle,
    focus_handle: FocusHandle,
    /// 视口尺寸回写（覆盖 canvas paint 期写、render 期读——pdf_pages
    /// 同款：避免每帧实体更新）。
    viewport: Rc<Cell<Option<(f32, f32)>>>,
    viewport_size: (f32, f32),
}

impl DocxPagesView {
    /// 新建视图：后台线程起会话（打开链零 UI 阻塞）。文件不存在/
    /// 装载失败经 `Boot::Failed` 呈现（不 panic——用户文件不可打开是
    /// 产品态，非程序错误）。
    pub fn new(path: PathBuf, cx: &mut App) -> Self {
        let focus_handle = cx.focus_handle();
        let (open_tx, open_rx) = channel();
        let thread_path = path.clone();
        std::thread::Builder::new()
            .name("docx-lok-open".into())
            .spawn(move || {
                // 会话层自带线程跑打开链——本线程只等句柄回传（秒内）。
                let _ = open_tx.send(DocxLokSession::open_blocking(&thread_path, None));
            })
            .expect("打开线程");
        let view = Self {
            path,
            boot: Boot::Starting,
            open_rx: Some(open_rx),
            session: None,
            doc_size: None,
            cache: HashMap::new(),
            cache_costs: HashMap::new(),
            cache_order: VecDeque::new(),
            cache_bytes: 0,
            pending: HashMap::new(),
            failed: HashSet::new(),
            // 轮询循环在首次 render 起表（Context 语义——App 面无法
            // spawn 实体任务；渲染即驱动，pdf_pages 同源纪律）。
            polling: false,
            scroll: UniformListScrollHandle::default(),
            focus_handle,
            viewport: Rc::new(Cell::new(None)),
            viewport_size: (0., 0.),
        };
        view
    }

    /// 渲染期起表卫（首次 render 启动轮询——仅一次；终态自停，pdf
    /// _pages 同源纪律）。
    fn ensure_polling(&mut self, cx: &mut Context<Self>) {
        if !self.polling {
            self.polling = true;
            self.spawn_poll_loop(cx);
        }
    }

    /// 源路径（壳层槽位对账）。
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// 启动状态读数（壳层/测试面）。
    pub fn boot(&self) -> &Boot {
        &self.boot
    }

    /// C1 只读渲染：恒不脏（保存面 C4；文本态切换卫保证行投影不会与
    /// 会话同时编辑同一文件）。
    pub fn is_dirty(&self) -> bool {
        false
    }

    /// 条带缓存读数（测试面：`(已用字节, 条目数)`）。
    #[cfg(test)]
    pub fn cache_stats_for_test(&self) -> (usize, usize) {
        (self.cache_bytes, self.cache.len())
    }

    /// 派单在途数（测试面）。
    #[cfg(test)]
    pub fn pending_for_test(&self) -> usize {
        self.pending.len()
    }

    /// 回执轮询循环（16ms drain；打开/会话双通道终态即停——pdf_pages
    /// 搜索索引轮询同款纪律。实体释放（关标签/切文本态 Drop）时弱句柄
    /// 升级失败即自然止——防循环泄漏）。
    fn spawn_poll_loop(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                if this.upgrade().is_none() {
                    break; // 实体已释放——自然止。
                }
                let mut done = false;
                let _ = this.update(cx, |view, cx| {
                    view.drain(cx);
                    if view.open_rx.is_none() && view.session.is_none() {
                        view.polling = false;
                        done = true;
                    }
                });
                if done {
                    break;
                }
            }
        })
        .detach();
    }

    /// 双通道 drain：打开句柄回传 + 会话回执（一次 16ms 拍全消费）。
    /// 回执先集后处理（借用纪律：会话不可变借用不跨自修改）。
    fn drain(&mut self, cx: &mut Context<Self>) {
        // ---- 1) 打开结果（一次性；take/放回避免同字段借用冲突）----
        if let Some(rx) = self.open_rx.take() {
            match rx.try_recv() {
                Ok(Ok(session)) => {
                    eprintln!(
                        "docx-pages: 会话句柄就绪（{}）——等待 load/Ready…",
                        session.path().display()
                    );
                    self.session = Some(session);
                    self.open_rx = Some(rx);
                }
                Ok(Err(err)) => {
                    eprintln!("docx-pages: 打开失败（{}）：{}", err.stage(), err.message());
                    self.boot = Boot::Failed {
                        stage: err.stage(),
                        message: err.message().to_string(),
                    };
                }
                Err(TryRecvError::Empty) => {
                    self.open_rx = Some(rx);
                }
                Err(TryRecvError::Disconnected) => {
                    self.boot = Boot::Failed {
                        stage: "spawn",
                        message: "打开线程异常退出".into(),
                    };
                }
            }
        }

        // ---- 2) 会话回执（先集后处理）----
        let Some(session) = &self.session else {
            return;
        };
        let mut replies = Vec::new();
        while let Some(reply) = session.try_recv() {
            replies.push(reply);
            if replies.len() >= 64 {
                break; // 单拍上限——防极端回调风暴卡帧（下一拍继续）
            }
        }
        for reply in replies {
            match reply {
                SessionReply::Ready {
                    width_twips,
                    height_twips,
                    kernel_sha256,
                    version_info,
                    elapsed_ms,
                } => {
                    eprintln!(
                        "docx-pages: ready size={width_twips}x{height_twips}tw \
                         kernel={kernel_sha256} version={version_info} open_ms={elapsed_ms}"
                    );
                    self.doc_size = Some((width_twips, height_twips));
                    self.boot = Boot::Ready;
                    cx.notify();
                }
                SessionReply::Failed { stage, message } => {
                    eprintln!("docx-pages: 会话失败（{stage}）：{message}");
                    self.session = None;
                    self.pending.clear();
                    self.boot = Boot::Failed { stage, message };
                    cx.notify();
                }
                SessionReply::Died { detail } => {
                    eprintln!("docx-pages: worker 死亡：{detail}");
                    self.session = None;
                    self.pending.clear();
                    self.boot = Boot::Failed {
                        stage: "died",
                        message: detail,
                    };
                    cx.notify();
                }
                SessionReply::Tile {
                    seq,
                    width,
                    height,
                    bgra,
                    sha256,
                    stats,
                    elapsed_ms,
                } => {
                    let Some(key) = self.pending.remove(&seq) else {
                        continue; // 迟到 tile（换代竞态）——机械丢弃。
                    };
                    eprintln!(
                        "docx-pages: tile band={} w={} {width}x{height}px sha={sha256} \
                         paint_ms={elapsed_ms} non_white={:.4} colored={:.4}",
                        key.0, key.1, stats.non_white_ratio, stats.colored_ratio,
                    );
                    if let Some(image) = bgra_to_image(&bgra, width, height) {
                        self.cache_insert(key, Arc::new(image), width * height * 4);
                    } else {
                        self.failed.insert(key);
                    }
                    cx.notify();
                }
                SessionReply::Callback { name, payload } => {
                    // C1 仅登记（C2 接缓存失效：INVALIDATE_TILES /
                    // DOCUMENT_SIZE_CHANGED 驱动重派/换尺寸）。
                    eprintln!("docx-pages: callback name={name} payload={payload}");
                }
                SessionReply::Pong { rtt_ms } => {
                    eprintln!("docx-pages: pong rtt_ms={rtt_ms}");
                }
            }
        }
    }

    /// LRU 插入（预算内逐出最旧；同键换代先清旧账）。
    fn cache_insert(&mut self, key: (u32, u32), image: Arc<gpui::RenderImage>, cost: usize) {
        if let Some(old_cost) = self.cache_costs.remove(&key) {
            self.cache_bytes = self.cache_bytes.saturating_sub(old_cost);
            self.cache.remove(&key);
            self.cache_order.retain(|k| *k != key);
        }
        self.cache.insert(key, image);
        self.cache_costs.insert(key, cost);
        self.cache_bytes += cost;
        self.cache_order.push_back(key);
        while self.cache_bytes > CACHE_BUDGET_BYTES && self.cache_order.len() > 1 {
            let Some(oldest) = self.cache_order.pop_front() else {
                break;
            };
            if let Some(cost) = self.cache_costs.remove(&oldest) {
                self.cache_bytes = self.cache_bytes.saturating_sub(cost);
            }
            self.cache.remove(&oldest);
        }
    }

    /// 可见条带派单（render 期调用——读覆盖 canvas 上帧记录的视口；
    /// 首帧视口未记录则等下一帧，pdf_pages 同款门）。
    fn submit_visible(&mut self, _cx: &mut Context<Self>) {
        if !matches!(self.boot, Boot::Ready) {
            return;
        }
        if let Some((w, h)) = self.viewport.get() {
            if w > 0. && h > 0. {
                self.viewport_size = (w, h);
            }
        }
        let (vw, vh) = self.viewport_size;
        if vw <= 0. || vh <= 0. {
            return;
        }
        let Some((w_tw, h_tw)) = self.doc_size else {
            return;
        };
        let view_w = quantize_width(vw as u32);
        let count = band_count(w_tw, h_tw, view_w);
        if count == 0 {
            return;
        }
        // 滚动偏移（gpui 负向约定——pdf_pages 同款取负归正）。
        let offset_y = (-f32::from(self.scroll.0.borrow().base_handle.offset().y)).max(0.);
        let first = (offset_y as usize / BAND_PX as usize).saturating_sub(1);
        let visible_bands = (vh as usize / BAND_PX as usize + 2).max(1);
        let last = (first + visible_bands).min(count);
        // 相 1（无会话借用）：算键集。
        let mut todo = Vec::new();
        for band in first..last {
            let key = (band as u32, view_w);
            if self.cache.contains_key(&key)
                || self.failed.contains(&key)
                || self.pending.values().any(|k| *k == key)
            {
                continue;
            }
            if let Some(geom) = band_region(w_tw, h_tw, view_w, key.0) {
                todo.push((key, geom));
            }
        }
        // 相 2（会话借用 + pending 写入——字段不相交）。
        if !todo.is_empty()
            && let Some(session) = &self.session
        {
            for (key, geom) in todo {
                let seq = session.request_tile(
                    geom.canvas_w as usize,
                    geom.canvas_h as usize,
                    geom.tile_x,
                    geom.tile_y,
                    geom.tile_w,
                    geom.tile_h,
                );
                self.pending.insert(seq, key);
            }
        }
    }

    /// 条带槽元素（uniform_list 回调——缓存出位图，未就绪出占位框）。
    fn render_band_slot(&mut self, idx: usize, cx: &App) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let (vw, _) = self.viewport_size;
        let view_w = quantize_width(if vw > 0. { vw as u32 } else { WIDTH_MIN });
        let key = (idx as u32, view_w);
        let Some((w_tw, h_tw)) = self.doc_size else {
            return div().size_full().into_any_element();
        };
        let Some(geom) = band_region(w_tw, h_tw, view_w, key.0) else {
            return div().size_full().into_any_element();
        };
        let slot_w = geom.canvas_w as f32;
        let slot_h = geom.canvas_h as f32;
        let band_no = idx + 1;
        let image = self.cache.get(&key).cloned();
        match image {
            Some(image) => gpui::img(ImageSource::Render(image))
                .id(gpui::ElementId::NamedInteger(
                    "docx-band".into(),
                    idx as u64,
                ))
                .w(px(slot_w))
                .h(px(slot_h))
                .flex_none()
                .into_any_element(),
            None => {
                let failed = self.failed.contains(&key);
                div()
                    .id(gpui::SharedString::from(format!("docx-band-pending-{idx}")))
                    .w(px(slot_w))
                    .h(px(slot_h))
                    .flex_none()
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.panel_background)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        ui::Label::new(if failed {
                            format!("条带 {band_no} 渲染失败")
                        } else {
                            format!("条带 {band_no} 渲染中…")
                        })
                        .size(ui::LabelSize::Small)
                        .color(if failed {
                            ui::Color::Warning
                        } else {
                            ui::Color::Muted
                        }),
                    )
                    .into_any_element()
            }
        }
    }

    /// 启动/失败面板（明示——不白屏；失败态常驻文本视图后备提示）。
    fn render_state_panel(&self, cx: &App) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let failed = matches!(self.boot, Boot::Failed { .. });
        let (title, detail) = match &self.boot {
            Boot::Starting => (
                "文档打开中…（LOK 内核首次启动可能需要数十秒）".to_string(),
                format!("文件：{}", self.path.display()),
            ),
            Boot::Failed { stage, message } => (
                format!("LOK 页面视图不可用（阶段：{stage}）"),
                message.clone(),
            ),
            Boot::Ready => ("文档已就绪".to_string(), String::new()),
        };
        v_flex()
            .id("docx-pages-state")
            .debug_selector(|| "docx-pages-state".into())
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .bg(colors.background)
            .child(
                ui::Label::new(title)
                    .size(ui::LabelSize::Large)
                    .color(if failed {
                        ui::Color::Warning
                    } else {
                        ui::Color::Muted
                    }),
            )
            .child(
                ui::Label::new(detail)
                    .size(ui::LabelSize::Small)
                    .color(ui::Color::Muted),
            )
            .when(failed, |el| {
                el.child(
                    ui::Label::new(
                        "文本视图不受影响——状态栏「切换到文本视图」可回到行模型阅读/编辑",
                    )
                    .size(ui::LabelSize::Small)
                    .color(ui::Color::Muted),
                )
            })
            .into_any_element()
    }
}

impl Focusable for DocxPagesView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DocxPagesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 轮询起表（首次渲染启动——App 面无法 spawn；终态自停）。
        self.ensure_polling(cx);
        // 派单（render 期直接改 self——uniform_list 回调期只读）。
        self.submit_visible(cx);
        let ready = matches!(self.boot, Boot::Ready);
        let doc_size = self.doc_size;
        if !ready || doc_size.is_none() {
            return self.render_state_panel(cx);
        }
        let (w_tw, h_tw) = doc_size.unwrap();
        let entity = cx.entity().downgrade();
        let viewport_cell = self.viewport.clone();
        let count = {
            let (vw, _) = self.viewport_size;
            let view_w = quantize_width(if vw > 0. { vw as u32 } else { WIDTH_MIN });
            band_count(w_tw, h_tw, view_w)
        };
        if count == 0 {
            return self.render_state_panel(cx);
        }
        gpui::uniform_list(
            "docx-pages",
            count,
            move |visible: std::ops::Range<usize>, _window: &mut Window, cx: &mut App| {
                visible
                    .clone()
                    .filter_map(|idx| {
                        let slot = entity
                            .update(cx, |view, cx| view.render_band_slot(idx, cx))
                            .ok()?;
                        Some(band_overlay(slot, idx, viewport_cell.clone()))
                    })
                    .collect::<Vec<AnyElement>>()
            },
        )
        .track_scroll(&self.scroll)
        .debug_selector(|| "docx-pages-list".into())
        .w_full()
        .h_full()
        .into_any_element()
    }
}

/// 条带包装：叠加透明 canvas——paint 期把视口尺寸写进 `Rc<Cell>`（每
/// 帧注册、零实体更新——pdf_pages 覆盖 canvas 同款）。
fn band_overlay(
    slot: AnyElement,
    idx: usize,
    viewport: Rc<Cell<Option<(f32, f32)>>>,
) -> AnyElement {
    div()
        .id(gpui::SharedString::from(format!("docx-band-overlay-{idx}")))
        .debug_selector(move || format!("docx-visible-band-{idx}"))
        .relative()
        .flex_none()
        .child(slot)
        .child(
            div()
                .id(gpui::SharedString::from(format!("docx-band-canvas-{idx}")))
                .absolute()
                .size_full()
                .child(gpui::canvas(
                    move |bounds, _window, _cx| bounds,
                    move |bounds: gpui::Bounds<gpui::Pixels>, _t, _window, _cx| {
                        let w = f32::from(bounds.size.width);
                        let h = f32::from(bounds.size.height);
                        viewport.set(Some((w, h)));
                    },
                )),
        )
        .into_any_element()
}

// ---------------------------------------------------------------------------
// 测试面（几何纯函数恒绿；GPUI 态机无运行时可测；真链见会话层测试）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 适宽量化：钳制 + 向上取整到 64 倍数。
    #[test]
    fn quantize_width_clamps_and_rounds() {
        assert_eq!(quantize_width(0), WIDTH_MIN);
        assert_eq!(quantize_width(100), WIDTH_MIN);
        assert_eq!(quantize_width(257), WIDTH_QUANTUM * 5); // 257 → 320
        assert_eq!(quantize_width(320), 320);
        assert_eq!(quantize_width(10_000), WIDTH_MAX);
    }

    /// A4 纵向（≈ 11906 × 16838 twips）：多档适宽下条带族无缝覆盖
    /// `[0, doc_h_tw)`、无重叠、末带钳制文末。
    #[test]
    fn band_regions_tile_document_exactly() {
        let (w_tw, h_tw) = (11906i64, 16838i64);
        for view_w in [WIDTH_MIN, 512, 1024, 4096] {
            let count = band_count(w_tw, h_tw, view_w);
            assert!(count >= 1, "条带数必须 ≥1（view_w={view_w}）");
            let mut prev_end = 0i64;
            for band in 0..count as u32 {
                let geom = band_region(w_tw, h_tw, view_w, band)
                    .unwrap_or_else(|| panic!("条带 {band} 必须有几何"));
                assert_eq!(geom.tile_x, 0);
                assert_eq!(geom.tile_w as i64, w_tw, "适宽全幅（view_w={view_w}）");
                assert!(geom.tile_h > 0, "区域高必须 > 0");
                assert!(
                    geom.tile_y as i64 + geom.tile_h as i64 <= h_tw,
                    "区域不得越文末（band={band}）"
                );
                assert!(
                    geom.tile_y as i64 == prev_end,
                    "条带必须首尾相接（band={band}：y={} ≠ prev_end={prev_end}）",
                    geom.tile_y
                );
                prev_end = geom.tile_y as i64 + geom.tile_h as i64;
                assert!(geom.canvas_h >= 1);
            }
            assert_eq!(prev_end, h_tw, "条带族必须覆盖到文末（view_w={view_w}）");
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
        let view_w = 1024u32; // s = 1024/200 = 5.12
        let total = content_height_px(3000, 6000, view_w);
        assert!(
            (total - 2048.).abs() < 1.,
            "两轴同缩放：400*5.12≈{total:.0}"
        );
        let count = band_count(3000, 6000, view_w);
        assert!(
            (count as f64 * BAND_PX as f64) >= total,
            "条带数必须覆盖显示高（{count}*512 >= {total:.0}）"
        );
        assert!(total > BAND_PX as f64, "放大后显示高应超单带（{total:.0}）");
    }

    /// BGRA → RenderImage：载荷不符拒绝（不 panic）。
    #[test]
    fn bgra_to_image_validates() {
        assert!(bgra_to_image(&[], 0, 0).is_none());
        assert!(bgra_to_image(&[0; 4], 2, 2).is_none()); // 长度不符
        assert!(bgra_to_image(&[0u8; 16], 2, 2).is_some());
    }

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

    /// 视图态机（无运行时恒绿）：坏路径 → 打开失败态（load 段）；轮询
    /// 收尾后停表。真实布局验证失败面板呈现（不白屏）。
    #[gpui::test]
    fn bad_path_lands_in_failed_boot(cx: &mut gpui::TestAppContext) {
        cx.update(|app| {
            theme::init(theme::LoadThemes::JustBase, app);
            theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
        });
        let bad =
            std::env::temp_dir().join(format!("mdc-docx-missing-{}.docx", std::process::id()));
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
}
