//! #50 C1/C2：DOCX 页面视图——LOK 内核真源渲染（连续画布条带模型）。
//!
//! **定位**：docx 的「页面模式」主显示面（对照 pdf 的
//! [`crate::pdf_pages::PdfPagesView`]——同一视图槽位纪律：壳层
//! `Option<(PathBuf, Entity)>` 按标签路径持有，状态栏「页面/文本」
//! 钮切换）。编辑真源 = LOK 内核（外部 worker 进程，[`crate::editor::
//! docx_lok_session`] 会话层）；行投影编辑器与页面视图经壳层互斥
//! （切换卫：文本态有未保存编辑拒切页面态——单一编辑真源）。
//!
//! **渲染模型（C1）**：LOK Writer 文档是连续画布（`size()` twips）。
//! 光栅宽决定缩放，画布纵向切 **512px 条带**，每条带一次 `paint`
//! （twips 区域 → 画布像素；排版/图片/页眉页脚全交内核）。条带
//! 几何纯函数收拢 [`crate::docx_geom`]（单测锚点：覆盖性/末带钳制/
//! 无重叠）。可见区 ±1 观测余量派单，LRU 预算缓存，16ms 轮询回收执
//! （pdf_pages 搜索索引/OCR 同款纪律）。
//!
//! **C2 面（本批）**：
//! - **缩放档**：`ZoomMode`（适应宽度/实际大小/自定义——pdf #421
//!   同构模型；键位 Ctrl+0/Ctrl+1/Ctrl+=·-，chip + 预设菜单，Ctrl+
//!   滚轮 capture 拦截）。换档滚动锚定：视口中心的内容 twips 纵
//!   坐标不变（互逆纯函数——`docx_geom::center_twips`）。
//! - **缓存失效（内核事件驱动）**：INVALIDATE_TILES 载荷（twips
//!   矩形）→ 受影响条带区间 → 跨光栅宽逐出（内容变更与缩放无关）；
//!   不可解析/非正尺寸 → 全量逐出（含在途/失败登记——陈旧像素是
//!   产品缺陷，诚实放大）。DOCUMENT_SIZE_CHANGED → `size` 重查 →
//!   几何更新 + 全量逐出 + 滚动钳制。
//! - **视口真尺寸**：C1 的每带覆盖 canvas 记录的是条带尺寸，C2 改
//!   根级探针（pdf 同款）——可见窗口推导用真视口高。
//!
//! **边界（如实）**：只读渲染（键鼠输入 C3、保存 C4）；跳页待 B 线
//! 页面几何 op（#49 已登记）。内核缺失/装载失败 → 明确错误面板 +
//! 文本视图后备提示（不白屏、不假绿）。

use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use gpui::{
    AnyElement, App, ClickEvent, Context, DispatchPhase, FocusHandle, Focusable, ImageSource,
    ScrollDelta, ScrollEvent, UniformListScrollHandle, Window, px,
};
use image::{Frame, ImageBuffer};
use smallvec::SmallVec;
use ui::prelude::*;

use crate::canvas_view::WHEEL_LINE_PX;
use crate::docx_geom::{
    BAND_PX, DOCX_ZOOM_MAX, DOCX_ZOOM_MIN, DOCX_ZOOM_STEP, InvalScope, ZoomMode, band_count,
    band_region, center_twips, content_height_px, offset_for_center_twips, parse_invalidate_payload,
    quantize_width, zoom_percent, zoom_raster_width,
};
use crate::editor::docx_lok_session::{DocxLokSession, SessionError, SessionReply};

/// 条带位图缓存预算（字节）——LRU 逐出（512px 条带约 2MB/条常态）。
const CACHE_BUDGET_BYTES: usize = 64 * 1024 * 1024;
/// 回执轮询周期（16ms——pdf_pages `POLL_INTERVAL` 同款）。
const POLL_INTERVAL: Duration = Duration::from_millis(16);
/// 滚轮缩放步进倍率（每「行」——pdf `PDF_ZOOM_WHEEL_STEP` 同值）。
const DOCX_ZOOM_WHEEL_STEP: f32 = 1.1;

/// #50 C2：缩放预设菜单行（label, 目标模式, 键域提示——空串 = 无键位）。
/// 与 PDF 预设的差异（如实）：无「适应页面」档——连续画布单页适页需
/// 页边界（内核页面几何 op，#49 缺口已登记），不宿主自算。
const ZOOM_PRESETS: &[(&str, ZoomMode, &str)] = &[
    ("适应宽度", ZoomMode::FitWidth, "Ctrl+0"),
    ("实际大小", ZoomMode::ActualSize, "Ctrl+1"),
    ("50%", ZoomMode::Custom(0.5), ""),
    ("75%", ZoomMode::Custom(0.75), ""),
    ("150%", ZoomMode::Custom(1.5), ""),
    ("200%", ZoomMode::Custom(2.0), ""),
];

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
    /// 缩放档（C2——pdf #421 同构；默认适宽 = v1 行为零变化）。
    zoom_mode: ZoomMode,
    /// 缩放预设菜单开态（chip 点击开/backdrop 关）。
    zoom_menu_open: bool,
    /// 条带位图缓存（键 = (条带, 光栅宽)）。
    cache: HashMap<(u32, u32), Arc<gpui::RenderImage>>,
    /// 缓存成本记账（键 → 字节——LRU 逐出扣减同源）。
    cache_costs: HashMap<(u32, u32), usize>,
    /// LRU 序（队首冷——逐出依据）。
    cache_order: VecDeque<(u32, u32)>,
    cache_bytes: usize,
    /// 派单在途（seq → 条带键——回执 seq 对账）。
    pending: HashMap<u64, (u32, u32)>,
    /// 单次失败登记（同键不重派——诚实失败面；失效/换档即清）。
    failed: HashSet<(u32, u32)>,
    polling: bool,
    scroll: UniformListScrollHandle,
    focus_handle: FocusHandle,
    /// 视口尺寸回写（根级探针 canvas paint 期写、render 期读——
    /// pdf_pages 同款：避免每帧实体更新）。
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
            zoom_mode: ZoomMode::FitWidth,
            zoom_menu_open: false,
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

    /// C1/C2 只读渲染：恒不脏（保存面 C4；文本态切换卫保证行投影不会
    /// 与会话同时编辑同一文件）。
    pub fn is_dirty(&self) -> bool {
        false
    }

    /// 当前缩放档（测试面）。
    #[cfg(test)]
    pub fn zoom_mode_for_test(&self) -> ZoomMode {
        self.zoom_mode
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

    /// 失效面（测试面：`(失败登记数, 缓存条目数)`——失效链断言用）。
    #[cfg(test)]
    pub fn failed_for_test(&self) -> usize {
        self.failed.len()
    }

    // -----------------------------------------------------------------
    // 缩放档（C2——pdf #421 同构模型）
    // -----------------------------------------------------------------

    /// 光栅宽单点口径（派单/槽渲染/可见窗口全部经此——缩放一致性关键）。
    fn raster_width(&self, vw: f32) -> u32 {
        match self.doc_size {
            Some((w_tw, _)) => zoom_raster_width(self.zoom_mode, w_tw, vw),
            // 几何未知（Starting）：退 v1 适宽口径（不派单，仅测高）。
            None => quantize_width(vw.max(1.) as u32),
        }
    }

    /// 当前缩放百分比（相对实际大小——chip 显示）。
    fn zoom_pct(&self, vw: f32) -> u32 {
        match self.doc_size {
            Some((w_tw, _)) => zoom_percent(self.zoom_mode, w_tw, vw),
            None => 100,
        }
    }

    /// 当前生效倍率（任意档 → 光栅宽/文档名义宽；步进起算基准——
    /// 预设态从当前生效百分比继续，不从 100% 跳）。
    fn effective_zoom(&self, vw: f32) -> f32 {
        match self.doc_size {
            Some((w_tw, _)) => {
                let doc_w_px = (w_tw.max(1) as f64 / crate::docx_geom::TWIPS_PER_PX as f64) as f32;
                self.raster_width(vw) as f32 / doc_w_px
            }
            None => 1.,
        }
    }

    /// 切换缩放档（滚动锚定：视口中心的内容 twips 纵坐标不变——
    /// `docx_geom` 互逆纯函数；预设菜单点击/键位同走此口）。
    fn set_zoom_mode(&mut self, mode: ZoomMode, cx: &mut Context<Self>) {
        let (vw, vh) = self.viewport_size;
        let Some((w_tw, _)) = self.doc_size else {
            self.zoom_mode = mode;
            cx.notify();
            return;
        };
        let old_rw = self.raster_width(vw) as f64;
        let offset_y = (-f32::from(self.scroll.0.borrow().base_handle.offset().y)).max(0.) as f64;
        let anchor = center_twips(offset_y, vh as f64, w_tw, old_rw as u32);
        self.zoom_mode = mode;
        self.zoom_menu_open = false;
        let new_rw = self.raster_width(vw);
        let new_offset = offset_for_center_twips(anchor, vh as f64, w_tw, new_rw);
        self.scroll
            .0
            .borrow()
            .base_handle
            .set_offset(gpui::point(px(0.), px(-new_offset as f32)));
        cx.notify();
    }

    /// 键盘步进缩放（Ctrl+= / Ctrl+-：×`DOCX_ZOOM_STEP`）。
    fn zoom_step(&mut self, bigger: bool, cx: &mut Context<Self>) {
        let factor = if bigger {
            DOCX_ZOOM_STEP
        } else {
            1. / DOCX_ZOOM_STEP
        };
        self.zoom_by_factor(factor, cx);
    }

    /// 滚轮缩放（每「行」×`DOCX_ZOOM_WHEEL_STEP`；steps 正 = 放大
    /// ——pdf zoom_wheel 同款方向语义）。
    fn zoom_wheel(&mut self, steps: f32, cx: &mut Context<Self>) {
        let factor = DOCX_ZOOM_WHEEL_STEP.powf(steps);
        self.zoom_by_factor(factor, cx);
    }

    /// 相对当前生效倍率缩放（统一收口：落 Custom 档；连续步进不跳档）。
    fn zoom_by_factor(&mut self, factor: f32, cx: &mut Context<Self>) {
        let (vw, _) = self.viewport_size;
        let next = (self.effective_zoom(vw) * factor).clamp(DOCX_ZOOM_MIN, DOCX_ZOOM_MAX);
        self.set_zoom_mode(ZoomMode::Custom(next), cx);
    }

    /// 预设行是否为当前态（Custom 档比倍率近似——0.5% 内视为同档）。
    fn zoom_preset_active(&self, target: ZoomMode) -> bool {
        match (target, self.zoom_mode) {
            (ZoomMode::Custom(a), ZoomMode::Custom(b)) => (a - b).abs() < 0.005,
            (a, b) => a == b,
        }
    }

    /// #50 C2：docx 页面视图键域（视口缩放命令——pdf #421 `dispatch_
    /// zoom_key` 同构先例：视口命令在 shell keymap 表之前分流，消费
    /// 后 shell return 防字号键二次响应）。键位：
    ///
    /// - Ctrl+0：适应宽度（连续画布无「适应页面」档——见模块头）
    /// - Ctrl+1：实际大小 100%
    /// - Ctrl+= / Ctrl++：步进放大（shift 兼容）
    /// - Ctrl+-：步进缩小
    ///
    /// 返回 false = 未消费（键继续走 shell keymap 表，非页面态零变化）。
    pub fn dispatch_zoom_key(
        &mut self,
        key: &str,
        ctrl: bool,
        _shift: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if !ctrl {
            return false;
        }
        match key {
            "0" => self.set_zoom_mode(ZoomMode::FitWidth, cx),
            "1" => self.set_zoom_mode(ZoomMode::ActualSize, cx),
            "=" | "+" => self.zoom_step(true, cx),
            "-" => self.zoom_step(false, cx),
            _ => return false,
        }
        true
    }

    // -----------------------------------------------------------------
    // 回执轮询（C2：Size 重查 + 事件驱动失效）
    // -----------------------------------------------------------------

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
                        continue; // 迟到 tile（失效/换代竞态）——机械丢弃。
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
                SessionReply::Size {
                    width_twips,
                    height_twips,
                } => {
                    // DOCUMENT_SIZE_CHANGED 后的重查回执：几何变化 →
                    // 全量失效 + 滚动钳制（不变则幂等）。
                    self.apply_size_reply((width_twips, height_twips), cx);
                }
                SessionReply::Callback { name, payload } => {
                    self.handle_callback(&name, &payload, cx);
                }
                SessionReply::Pong { rtt_ms } => {
                    eprintln!("docx-pages: pong rtt_ms={rtt_ms}");
                }
            }
        }
    }

    /// 内核回调分派（C2 面：缓存失效/几何重查；其余登记——C3 命令态）。
    fn handle_callback(&mut self, name: &str, payload: &str, cx: &mut Context<Self>) {
        match name {
            "INVALIDATE_TILES" => {
                eprintln!("docx-pages: INVALIDATE_TILES payload={payload}");
                let scope = parse_invalidate_payload(payload);
                let evicted = self.apply_invalidate(scope);
                if evicted > 0 {
                    cx.notify();
                }
            }
            "DOCUMENT_SIZE_CHANGED" => {
                eprintln!("docx-pages: DOCUMENT_SIZE_CHANGED payload={payload}");
                // 几何单一真源 = worker `size` op（载荷仅日志参考）。
                if let Some(session) = &self.session {
                    session.request_size();
                }
            }
            other => {
                eprintln!("docx-pages: callback name={other} payload={payload}");
            }
        }
    }

    /// 失效区间逐出（跨光栅宽：内容变更与缩放无关——同条带全档逐出；
    /// 在途/失败登记同步清除——陈旧像素宁可重画不可误显）。返回逐出
    /// 缓存条数。
    fn apply_invalidate(&mut self, scope: InvalScope) -> usize {
        let Some((w_tw, h_tw)) = self.doc_size else {
            return 0;
        };
        match scope {
            InvalScope::Full => {
                let n = self.cache.len();
                self.invalidate_all();
                n
            }
            InvalScope::Rect { x, y, w, h } => {
                let (vw, _) = self.viewport_size;
                // 命中判定用当前光栅宽的条带边界（条带序号空间一致
                // ——任何档下条带 i 覆盖的文档区间与当前档对齐）。
                let rw = self.raster_width(vw);
                let range = crate::docx_geom::bands_in_rect(w_tw, h_tw, rw, x, y, w, h);
                if range.is_empty() {
                    return 0;
                }
                let mut n = 0;
                self.cache.retain(|(band, _), _| {
                    let hit = range.contains(band);
                    n += hit as usize;
                    !hit
                });
                self.cache_costs.retain(|(band, _), _| !range.contains(band));
                self.cache_order.retain(|(band, _)| !range.contains(band));
                self.cache_bytes = self.cache_costs.values().sum();
                self.failed.retain(|(band, _)| !range.contains(band));
                self.pending.retain(|_, (band, _)| !range.contains(band));
                n
            }
        }
    }

    /// 全量失效：缓存/成本/LRU/在途/失败登记全清（换尺寸/不可解析失效
    /// 载荷——诚实放大）。
    fn invalidate_all(&mut self) {
        self.cache.clear();
        self.cache_costs.clear();
        self.cache_order.clear();
        self.cache_bytes = 0;
        self.pending.clear();
        self.failed.clear();
    }

    /// 几何更新（DOCUMENT_SIZE_CHANGED → `size` 重查回执）：变化 →
    /// 全量失效 + 滚动钳制；不变则幂等（如实——只读会话不应静默重建）。
    fn apply_size_reply(&mut self, size: (i64, i64), cx: &mut Context<Self>) {
        if self.doc_size == Some(size) {
            return;
        }
        eprintln!(
            "docx-pages: 几何更新 {}x{}tw → {}x{}tw（全量失效）",
            self.doc_size.map(|s| s.0).unwrap_or(0),
            self.doc_size.map(|s| s.1).unwrap_or(0),
            size.0,
            size.1
        );
        self.doc_size = Some(size);
        self.invalidate_all();
        self.clamp_scroll_to_content();
        cx.notify();
    }

    /// 几何更新后滚动钳制：offset 不得超过新内容高（负向约定——取负
    /// 归正钳制回写，pdf #634 修正同源口径）。
    fn clamp_scroll_to_content(&mut self) {
        let (vw, vh) = self.viewport_size;
        let Some((w_tw, h_tw)) = self.doc_size else {
            return;
        };
        let rw = self.raster_width(vw);
        let total = content_height_px(w_tw, h_tw, rw).max(0.) as f32;
        if vh <= 0. || total <= 0. {
            return;
        }
        let max_offset = (total - vh).max(0.);
        let offset_y = -f32::from(self.scroll.0.borrow().base_handle.offset().y);
        if offset_y > max_offset {
            self.scroll
                .0
                .borrow()
                .base_handle
                .set_offset(gpui::point(px(0.), px(-max_offset)));
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

    /// 可见条带派单（render 期调用——读根级探针上帧记录的视口）。
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
        let rw = self.raster_width(vw);
        let count = band_count(w_tw, h_tw, rw);
        if count == 0 {
            return;
        }
        // 滚动偏移（gpui 负向约定——取负归正；uniform_list 以首项测高，
        // 末带偏短属固有近似——条带步进恒 BAND_PX）。
        let offset_y = (-f32::from(self.scroll.0.borrow().base_handle.offset().y)).max(0.);
        let first = (offset_y as usize / BAND_PX as usize).saturating_sub(1);
        let visible_bands = (vh as usize / BAND_PX as usize + 2).max(1);
        let last = (first + visible_bands).min(count);
        // 相 1（无会话借用）：算键集。
        let mut todo = Vec::new();
        for band in first..last {
            let key = (band as u32, rw);
            if self.cache.contains_key(&key)
                || self.failed.contains(&key)
                || self.pending.values().any(|k| *k == key)
            {
                continue;
            }
            if let Some(geom) = band_region(w_tw, h_tw, rw, key.0) {
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
        let rw = self.raster_width(if vw > 0. { vw } else { 256. });
        let key = (idx as u32, rw);
        let Some((w_tw, h_tw)) = self.doc_size else {
            return div().size_full().into_any_element();
        };
        let Some(geom) = band_region(w_tw, h_tw, rw, key.0) else {
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

    /// #50 C2：缩放预设菜单（backdrop 模态——pdf #421 同款形态）。
    fn render_zoom_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.zoom_menu_open {
            return None;
        }
        let colors = cx.theme().colors().clone();
        let hover = colors.element_hover;
        let rows: Vec<AnyElement> = ZOOM_PRESETS
            .iter()
            .map(|(label, target, shortcut)| {
                let target = *target;
                let active = self.zoom_preset_active(target);
                let row_hover = hover;
                let label = *label;
                let shortcut = *shortcut;
                div()
                    .id(gpui::SharedString::from(format!("docx-zoom-preset-{label}")))
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .px_3()
                    .py_1()
                    .rounded_sm()
                    .when(active, |el| el.bg(colors.element_selected))
                    .hover(move |el| el.bg(row_hover))
                    .on_click(cx.listener(move |this, _ev: &ClickEvent, _window, cx| {
                        this.set_zoom_mode(target, cx);
                    }))
                    .child(
                        ui::Label::new(label)
                            .size(ui::LabelSize::Small)
                            .color(if active {
                                ui::Color::Default
                            } else {
                                ui::Color::Muted
                            }),
                    )
                    .child(
                        ui::Label::new(shortcut)
                            .size(ui::LabelSize::Small)
                            .color(ui::Color::Muted),
                    )
                    .into_any_element()
            })
            .collect();
        Some(
            div()
                .id("docx-zoom-backdrop")
                .absolute()
                .size_full()
                .top_0()
                .left_0()
                .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                    this.zoom_menu_open = false;
                    cx.notify();
                }))
                .child(
                    v_flex()
                        .id("docx-zoom-menu")
                        .debug_selector(|| "docx-zoom-menu".into())
                        .absolute()
                        .bottom_12()
                        .right_4()
                        .rounded_md()
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.panel_background)
                        .opacity(0.97)
                        .py_1()
                        .min_w(px(160.))
                        .children(rows),
                )
                .into_any_element(),
        )
    }

    /// #50 C2：缩放控制条（常驻右下角——chip：步进钮 + 百分比 + 步进钮，
    /// 百分比点击开预设菜单；pdf #421 同款形态）。
    fn render_zoom_chip(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let hover = colors.element_hover;
        let (vw, _) = self.viewport_size;
        let zoom_pct = self.zoom_pct(if vw > 0. { vw } else { 320. });
        div()
            .id("docx-zoom-chip")
            .debug_selector(|| "docx-zoom-chip".into())
            .absolute()
            .bottom_4()
            .right_4()
            .flex()
            .items_center()
            .gap_1()
            .rounded_md()
            .border_1()
            .border_color(colors.border)
            .bg(colors.panel_background)
            .opacity(0.95)
            .px_1()
            .py_1()
            .child(
                div()
                    .id("docx-zoom-out")
                    .px_2()
                    .rounded_sm()
                    .hover(move |el| el.bg(hover))
                    .on_click(cx.listener(|this, _ev: &ClickEvent, _window, cx| {
                        this.zoom_step(false, cx);
                    }))
                    .child(
                        ui::Label::new("-")
                            .size(ui::LabelSize::Small)
                            .color(ui::Color::Muted),
                    ),
            )
            .child(
                div()
                    .id("docx-zoom-pct")
                    .px_2()
                    .rounded_sm()
                    .hover(move |el| el.bg(hover))
                    .on_click(cx.listener(|this, _ev: &ClickEvent, _window, cx| {
                        this.zoom_menu_open = !this.zoom_menu_open;
                        cx.notify();
                    }))
                    .child(
                        ui::Label::new(format!("{zoom_pct}%"))
                            .size(ui::LabelSize::Small)
                            .color(ui::Color::Muted),
                    ),
            )
            .child(
                div()
                    .id("docx-zoom-in")
                    .px_2()
                    .rounded_sm()
                    .hover(move |el| el.bg(hover))
                    .on_click(cx.listener(|this, _ev: &ClickEvent, _window, cx| {
                        this.zoom_step(true, cx);
                    }))
                    .child(
                        ui::Label::new("+")
                            .size(ui::LabelSize::Small)
                            .color(ui::Color::Muted),
                    ),
            )
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
        let zoom_entity = entity.clone();
        let (vw, _) = self.viewport_size;
        let rw = self.raster_width(if vw > 0. { vw } else { 256. });
        let count = band_count(w_tw, h_tw, rw);
        if count == 0 {
            return self.render_state_panel(cx);
        }
        let zoom_menu = self.render_zoom_menu(cx);
        let zoom_chip = self.render_zoom_chip(cx);
        let colors = cx.theme().colors().clone();
        let list = gpui::uniform_list(
            "docx-pages",
            count,
            move |visible: std::ops::Range<usize>, _window: &mut Window, cx: &mut App| {
                visible
                    .clone()
                    .filter_map(|idx| {
                        let slot = entity
                            .update(cx, |view, cx| view.render_band_slot(idx, cx))
                            .ok()?;
                        Some(band_wrapper(slot, idx))
                    })
                    .collect::<Vec<AnyElement>>()
            },
        )
        .track_scroll(&self.scroll)
        .debug_selector(|| "docx-pages-list".into())
        .w_full()
        .h_full();
        div()
            .id("docx-pages-root")
            .debug_selector(|| "docx-pages-root".into())
            .relative()
            .size_full()
            .min_w_0()
            .bg(colors.background)
            .flex()
            .flex_col()
            .items_center()
            .child(list)
            // #50 C2：缩放预设菜单（backdrop + 面板——在位时叠加）。
            .children(zoom_menu)
            // #50 C2：缩放控制条（常驻右下角）。
            .child(zoom_chip)
            // 根级视口探针：透明、绝对铺满——paint 期回写真视口尺寸
            //（C1 的每带探针记录的是条带尺寸，可见窗口推导被矮化——
            // 本批改根级口径）+ Ctrl+滚轮 capture 期拦截（缩放——
            // 止冒泡防列表滚动，pdf #421 同款）。
            .child(
                div().id("docx-viewport-probe").absolute().size_full().child(
                    gpui::canvas(
                        move |bounds, _window, _cx| bounds,
                        move |bounds: gpui::Bounds<gpui::Pixels>,
                              _t,
                              window: &mut Window,
                              _cx: &mut App| {
                            let measured =
                                (f32::from(bounds.size.width), f32::from(bounds.size.height));
                            if measured.0 > 0. && measured.1 > 0. {
                                viewport_cell.set(Some(measured));
                            }
                            // Ctrl+滚轮缩放——capture 期窗口级拦截
                            //（先于 uniform_list 滚动消费；横向手势不拦
                            //——冒泡照常滚动；control||platform 归一）。
                            window.on_mouse_event(
                                move |event: &ScrollEvent,
                                      phase: DispatchPhase,
                                      _window: &mut Window,
                                      cx: &mut App| {
                                    if phase != DispatchPhase::Capture {
                                        return;
                                    }
                                    if !(event.modifiers.control
                                        || event.modifiers.platform)
                                    {
                                        return;
                                    }
                                    if !bounds.contains(&event.position) {
                                        return;
                                    }
                                    let (dx, dy) = match event.delta {
                                        ScrollDelta::Pixels(p) => {
                                            (p.x.as_f32(), p.y.as_f32())
                                        }
                                        ScrollDelta::Lines(l) => {
                                            (l.x * WHEEL_LINE_PX, l.y * WHEEL_LINE_PX)
                                        }
                                    };
                                    if dy.abs() < dx.abs() {
                                        return; // 横向手势不缩放——冒泡照旧。
                                    }
                                    let steps = dy / WHEEL_LINE_PX;
                                    let _ = zoom_entity.update(cx, |view, cx| {
                                        view.zoom_wheel(steps, cx);
                                    });
                                    cx.stop_propagation();
                                },
                            );
                        },
                    ),
                ),
            )
            .into_any_element()
    }
}

/// 条带包装（uniform_list 槽位——debug_selector 供视觉测试定位）。
fn band_wrapper(slot: AnyElement, idx: usize) -> AnyElement {
    div()
        .id(gpui::SharedString::from(format!("docx-band-overlay-{idx}")))
        .debug_selector(move || format!("docx-visible-band-{idx}"))
        .relative()
        .flex_none()
        .child(slot)
        .into_any_element()
}

// ---------------------------------------------------------------------------
// 测试面（几何纯函数已收拢 docx_geom；本文件：视图态机 + 缩放/失效链）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
        // 直接注入 Ready 态 + A4 几何 + 视口（视图态机面——真链由
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
            assert_eq!(rw_actual, quantize_width((11906 / 15) as u32), "实际大小光栅宽");
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
        let img = || {
            Arc::new(
                bgra_to_image(&[0u8; 16], 2, 2).expect("2x2 载荷必须可成像"),
            )
        };
        view.update(cx, |v, _| {
            for band in [0u32, 1, 2] {
                v.cache_insert((band, 1024), img(), 16);
                v.cache_insert((band, 512), img(), 16);
            }
            v.failed.insert((1, 1024));
            v.pending.insert(999, (2, 1024));
        });
        // 条带 1 区域的 twips 矩形（1024px 档 A4：条带 0 ≈ [0, 5907)）。
        view.update(cx, |v, cx| {
            v.handle_callback(
                "INVALIDATE_TILES",
                "0, 6000, 11906, 100, 0, 0",
                cx,
            );
        });
        view.read_with(cx, |v, _| {
            assert!(!v.cache.contains_key(&(1, 1024)), "命中条带 1024 档逐出");
            assert!(!v.cache.contains_key(&(1, 512)), "命中条带 512 档同逐（跨档）");
            assert!(v.cache.contains_key(&(0, 1024)), "未命中条带保留");
            assert!(v.cache.contains_key(&(2, 1024)), "未命中条带保留");
            assert!(!v.failed.contains(&(1, 1024)), "失败登记随失效清除（可重试）");
            assert!(
                v.pending.values().any(|k| *k == (2, 1024)),
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
            v.cache_insert((0, 1024), Arc::new(bgra_to_image(&[0u8; 16], 2, 2).unwrap()), 16);
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
}
