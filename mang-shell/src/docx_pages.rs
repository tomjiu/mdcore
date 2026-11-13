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
//! - **首帧派单链（#186）**：根级探针 paint 量得视口 → `on_next_
//!   frame` 自排一帧消费（pdf_pages 同款纪律）——Ready 后无任何
//!   输入/回调也必须开始派单（缺此通知 = 条带永远「渲染中」）；
//!   视口变更（缩放/窗口调整）同径再消费。在途 tile 超时（15s）
//!   重派一次，再超转失败面——无进展不无限「渲染中」。
//! - **会话面 seam（#186）**：`TileSession` trait 收拢 tile 派单/几何
//!   重查/回执拉取——生产 `DocxLokSession`，测试注桩（真实窗口
//!   绘制链回归，不依赖 LO 运行时、不手工塞 viewport_size）。
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
use std::time::{Duration, Instant};

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
    band_region, center_twips, content_height_px, offset_for_center_twips,
    parse_invalidate_payload, quantize_width, zoom_percent, zoom_raster_width,
};
use crate::editor::docx_lok_session::{DocxLokSession, SessionError, SessionRecv, SessionReply};

/// 条带位图缓存预算（字节）——LRU 逐出（512px 条带约 2MB/条常态）。
const CACHE_BUDGET_BYTES: usize = 64 * 1024 * 1024;
/// 回执轮询周期（16ms——pdf_pages `POLL_INTERVAL` 同款）。
const POLL_INTERVAL: Duration = Duration::from_millis(16);
/// #186 tile 在途超时（无进展不无限「渲染中」——超时清出在途，未过
/// 尝试上限由下一拍 `submit_visible` 重派；较全局看门狗精确到键——
/// 单带悬死不被其余回执掩盖）。
const TILE_PENDING_TIMEOUT: Duration = Duration::from_secs(15);
/// #186 同键最大尝试数（首次 + 重派 1 次；再超时 → 失败面明示）。
const TILE_MAX_ATTEMPTS: u32 = 2;
/// #186 派单看门狗拍数（≈2 秒无任何 tile 进账 → 一次全量重派兜底——
/// 与每键超时并存：键级超时管单带悬死，全局看门狗管整链零回执）。
const WATCHDOG_TICKS: u32 = 125;
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

/// #179：panic 载荷 → 可读文本（`catch_unwind` 的 `Box<dyn Any +
/// Send>` 下钻 `&str`/`String`；其余给占位——「有 panic」这一事实
/// 本身不丢失）。
fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "非字符串 panic 载荷".to_string()
    }
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
// 会话面（#186 显示链测试 seam）
// ---------------------------------------------------------------------------

/// 会话面：tile 派单/几何重查/回执拉取。生产 = [`DocxLokSession`]（线程
/// 演员包装，`open_blocking` 真链）；测试注桩（tests 模块 `MockSession`）
/// ——真实窗口绘制链回归不依赖 LO 运行时、不手工塞 viewport_size。
pub(crate) trait TileSession: Send {
    /// 提交 tile 渲染（画布 px + 区域 twips——`DocSession::paint` 契约）。
    /// 返回对账 seq（回执 `SessionReply::Tile` 对号入座）。
    fn request_tile(
        &self,
        canvas_w: usize,
        canvas_h: usize,
        tile_x: i32,
        tile_y: i32,
        tile_w: i32,
        tile_h: i32,
    ) -> u64;
    /// 文档几何重查（DOCUMENT_SIZE_CHANGED 时机）。
    fn request_size(&self);
    /// 非阻塞收回执（16ms drain 拉取）。
    fn try_recv(&self) -> Option<SessionReply>;
    /// #179 三分收回执：`Reply` / `Empty` / `Disconnected`——通道
    /// 断开与暂时无消息分流（旧 `try_recv` 同貌 None——会话线程无终态
    /// 回执退出时视图永挂，无从诊断）。默认实现 = `try_recv` 的
    /// 二分包装（不区分断开——MockSession 语义即「通道恒活」）。
    fn try_recv_detailed(&self) -> crate::editor::docx_lok_session::SessionRecv {
        match self.try_recv() {
            Some(reply) => crate::editor::docx_lok_session::SessionRecv::Reply(reply),
            None => crate::editor::docx_lok_session::SessionRecv::Empty,
        }
    }
}

impl TileSession for DocxLokSession {
    fn request_tile(
        &self,
        canvas_w: usize,
        canvas_h: usize,
        tile_x: i32,
        tile_y: i32,
        tile_w: i32,
        tile_h: i32,
    ) -> u64 {
        Self::request_tile(self, canvas_w, canvas_h, tile_x, tile_y, tile_w, tile_h)
    }
    fn request_size(&self) {
        Self::request_size(self);
    }
    fn try_recv(&self) -> Option<SessionReply> {
        // inherent `try_recv` 已被 #179 改名 `try_recv_detailed`（三分
        // 返回）——此处若写 `Self::try_recv(self)` 会解析回本 trait 方法
        // 自身（inherent 不存在）→ 无限递归栈溢出。显式投影三分到
        // 二分：Disconnected 与 Empty 同为 None（二分语义）。
        match Self::try_recv_detailed(self) {
            crate::editor::docx_lok_session::SessionRecv::Reply(reply) => Some(reply),
            crate::editor::docx_lok_session::SessionRecv::Empty => None,
            crate::editor::docx_lok_session::SessionRecv::Disconnected => None,
        }
    }
    fn try_recv_detailed(&self) -> crate::editor::docx_lok_session::SessionRecv {
        Self::try_recv_detailed(self)
    }
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
    /// 会话（TileSession 面持有——生产 DocxLokSession；测试注桩）。
    session: Option<Box<dyn TileSession>>,
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
    /// 派单在途（seq → (条带键, 派单时刻)——回执 seq 对账 + 超时巡检）。
    pending: HashMap<u64, ((u32, u32), Instant)>,
    /// 同键尝试计数（超时重派次数——过 `TILE_MAX_ATTEMPTS` 转失败面；
    /// 失效/换尺寸/回执成功即清零）。
    attempts: HashMap<(u32, u32), u32>,
    /// 单次失败登记（同键不重派——诚实失败面；失效/换档即清）。
    failed: HashSet<(u32, u32)>,
    polling: bool,
    scroll: UniformListScrollHandle,
    focus_handle: FocusHandle,
    /// 视口尺寸回写（根级探针 canvas paint 期写、render 期读——
    /// pdf_pages 同款：避免每帧实体更新）。
    viewport: Rc<Cell<Option<(f32, f32)>>>,
    viewport_size: (f32, f32),
    /// #186：视口探针已 paint 到的尺寸（`refresh_viewport` 的变化检测
    /// 基线——探针首帧写 Cell 后由轮询拍接管派单，交互不再前置）。
    viewport_seen: (f32, f32),
    /// #186 看门狗：Ready 后经历过的轮询拍数（无 tile 进账时计数）——
    /// 超过 [`WATCHDOG_TICKS`] 触发一次全量重派；再超则条带转失败
    /// （不无限「渲染中」）。
    watchdog_ticks: u32,
    /// #186 看门狗：已触发过一次重派（仅一次——诚实失败面）。
    watchdog_retried: bool,
}

impl DocxPagesView {
    /// 新建视图：后台线程起会话（打开链零 UI 阻塞）。文件不存在/
    /// 装载失败经 `Boot::Failed` 呈现（不 panic——用户文件不可打开是
    /// 产品态，非程序错误）。
    pub fn new(path: PathBuf, cx: &mut App) -> Self {
        let focus_handle = cx.focus_handle();
        let (open_tx, open_rx) = channel();
        let thread_path = path.clone();
        // #179：打开链 panic 捕获回传——线程静默死时通道只剩通用
        // Disconnected，真实原因（panic 原文）丢失；线程创建失败
        //（资源耗尽等）不再 `.expect` 崩 UI 线程，构造期直落
        // `Boot::Failed`（错误面板明示）。
        let spawned = std::thread::Builder::new()
            .name("docx-lok-open".into())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    DocxLokSession::open_blocking(&thread_path)
                }));
                let reply = outcome.unwrap_or_else(|panic| {
                    Err(SessionError::Spawn(format!(
                        "打开线程 panic：{}",
                        panic_text(&panic)
                    )))
                });
                let _ = open_tx.send(reply);
            });
        let mut view = Self::empty(focus_handle, path, Some(open_rx));
        if let Err(e) = spawned {
            eprintln!("docx-pages: 打开线程创建失败：{e}");
            view.open_rx = None;
            view.boot = Boot::Failed {
                stage: "spawn",
                message: format!("打开线程创建失败：{e}"),
            };
        }
        view
    }

    /// 字段单源构造（`new` 真链与 #179 受控时序测试共用）。
    fn empty(
        focus_handle: FocusHandle,
        path: PathBuf,
        open_rx: Option<Receiver<Result<DocxLokSession, SessionError>>>,
    ) -> Self {
        Self {
            path,
            boot: Boot::Starting,
            open_rx,
            session: None,
            doc_size: None,
            zoom_mode: ZoomMode::FitWidth,
            zoom_menu_open: false,
            cache: HashMap::new(),
            cache_costs: HashMap::new(),
            cache_order: VecDeque::new(),
            cache_bytes: 0,
            pending: HashMap::new(),
            attempts: HashMap::new(),
            failed: HashSet::new(),
            // 轮询循环在首次 render 起表（Context 语义——App 面无法
            // spawn 实体任务；渲染即驱动，pdf_pages 同源纪律）。
            polling: false,
            scroll: UniformListScrollHandle::default(),
            focus_handle,
            viewport: Rc::new(Cell::new(None)),
            viewport_size: (0., 0.),
            viewport_seen: (0., 0.),
            watchdog_ticks: 0,
            watchdog_retried: false,
        }
    }

    /// #179 受控时序回归面：注入打开通道（无真实线程——结果与时序
    /// 由测试驱动；聚焦句柄走真实 `App` 面，字段与 `new` 同源
    ///（`empty`））。
    #[cfg(test)]
    fn new_with_open_rx_for_test(
        cx: &mut App,
        open_rx: Option<Receiver<Result<DocxLokSession, SessionError>>>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        Self::empty(focus_handle, PathBuf::from("test://179.docx"), open_rx)
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

    /// #186：测试面轮询拍（真实 drain 的单拍语义——含 refresh_viewport
    /// 与看门狗收口；gpui::test 里直接驱动而无需等 16ms 定时器）。
    #[cfg(test)]
    pub fn drain_for_test(&mut self, cx: &mut Context<Self>) {
        self.drain(cx);
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
        // ---- 1) 打开结果（一次性通道）----
        // #179：消费即终态——Ok/Err 都不放回 `open_rx`。旧实现在
        // `Ok(Ok(session))` 后放回，打开线程正常退出的通道关闭在
        // 下一拍被 `Disconnected` 分支当成异常，把已就绪的 Ready 或
        // 会话通道回传的具体失败覆盖成通用「打开线程异常退出」。
        if let Some(rx) = self.open_rx.take() {
            match rx.try_recv() {
                Ok(Ok(session)) => {
                    eprintln!(
                        "docx-pages: 会话句柄就绪（{}）——等待 load/Ready…",
                        session.path().display()
                    );
                    self.session = Some(Box::new(session));
                    // rx 即弃（不放回）：一次性结果已消费，通道此后的
                    // 正常关闭与会话状态无关（#179 根因修复点——TileSession
                    // 化后装箱）。
                }
                Ok(Err(err)) => {
                    eprintln!("docx-pages: 打开失败（{}）：{}", err.stage(), err.message());
                    self.boot = Boot::Failed {
                        stage: err.stage(),
                        message: err.message().to_string(),
                    };
                    // #179：状态迁移补 notify（旧实现 417-420 缺失——
                    // 错误面板显示时点依赖后续重绘，不能保证）。
                    cx.notify();
                }
                Err(TryRecvError::Empty) => {
                    self.open_rx = Some(rx);
                }
                Err(TryRecvError::Disconnected) => {
                    // 打开线程未回传任何结果就退出。panic 已被
                    // catch_unwind 转为 Err 回传（见 new）——走到这
                    // = 线程被强制终止/无法创建，非「回传后正常关闭」。
                    // 终态卫：Starting 以外不覆盖（防御——消费路径
                    // 已不放回 rx，正常流程此处只可能是 Starting）。
                    eprintln!("docx-pages: 打开通道断开（未收到任何结果）");
                    if matches!(self.boot, Boot::Starting) {
                        self.boot = Boot::Failed {
                            stage: "spawn",
                            message: "打开线程异常退出（未回传结果——线程被终止或无法创建）".into(),
                        };
                    }
                    cx.notify();
                }
            }
        }

        // ---- 1.5) 在途超时巡检（#186：无进展不无限「渲染中」——键级
        // 超时清出在途并计次，未过尝试上限由下一拍 submit_visible 重派）----
        self.expire_stale_pending(cx);

        // ---- 2) 视口探针收口（#186——置于会话检查前：Ready 但会话尚未
        // 建立的窗口期同样需要视口进位，且无会话时 submit_visible 自身
        // 会空转，语义不受影响）----
        self.refresh_viewport(cx);

        // ---- 3) 会话回执（先集后处理）----
        let Some(session) = &self.session else {
            return;
        };
        let mut replies = Vec::new();
        // #179：通道断开与暂时无消息分流（旧 try_recv 同貌 None——
        // 会话线程无终态回执退出时视图永挂，无从诊断）。
        let mut channel_alive = true;
        while replies.len() < 64 {
            match session.try_recv_detailed() {
                SessionRecv::Reply(reply) => replies.push(reply),
                SessionRecv::Empty => break,
                SessionRecv::Disconnected => {
                    channel_alive = false;
                    break;
                }
            }
        }
        let had_tile = replies
            .iter()
            .any(|r| matches!(r, SessionReply::Tile { .. }));
        let mut terminal_seen = false;
        for reply in replies {
            match reply {
                SessionReply::Ready {
                    width_twips,
                    height_twips,
                    kernel_sha256,
                    elapsed_ms,
                } => {
                    eprintln!(
                        "docx-pages: ready size={width_twips}x{height_twips}tw \
                         kernel={kernel_sha256} open_ms={elapsed_ms}"
                    );
                    self.doc_size = Some((width_twips, height_twips));
                    self.boot = Boot::Ready;
                    cx.notify();
                }
                SessionReply::Failed { stage, message } => {
                    terminal_seen = true;
                    eprintln!("docx-pages: 会话失败（{stage}）：{message}");
                    self.session = None;
                    self.pending.clear();
                    self.attempts.clear();
                    self.boot = Boot::Failed { stage, message };
                    cx.notify();
                }
                SessionReply::Died { detail } => {
                    terminal_seen = true;
                    eprintln!("docx-pages: worker 死亡：{detail}");
                    self.session = None;
                    self.pending.clear();
                    self.attempts.clear();
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
                    let Some((key, _)) = self.pending.remove(&seq) else {
                        continue; // 迟到 tile（失效/换代竞态）——机械丢弃。
                    };
                    self.attempts.remove(&key);
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
            }
        }

        // ---- 4) #179 会话通道断开分流（断开且本批无终态回执 = 异常
        // 退出，落可定位失败——正常收尾误报由 terminal_seen 卫）----
        self.handle_session_disconnect(!channel_alive && !terminal_seen, cx);

        // ---- 5) #186 派单看门狗（每拍收口）----
        // 看门狗：Ready 且视口已知但长时间无 tile 进账——一次全量重派
        // （pending 悬死/派单丢失的自愈）；重派后仍无进账则把在途条带
        // 转失败呈现（诚实失败面，不无限「渲染中」）。
        if had_tile {
            self.watchdog_ticks = 0;
        } else if matches!(self.boot, Boot::Ready) && self.viewport_size.0 > 0. {
            self.watchdog_ticks = self.watchdog_ticks.saturating_add(1);
            let known_missing =
                !self.cache.is_empty() || !self.pending.is_empty() || !self.failed.is_empty();
            if self.watchdog_ticks >= WATCHDOG_TICKS && known_missing {
                if !self.watchdog_retried {
                    eprintln!(
                        "docx-pages: #186 看门狗——{WATCHDOG_TICKS} 拍无 tile 进账，\
                         全量重派（pending={} cache={} failed={}）",
                        self.pending.len(),
                        self.cache.len(),
                        self.failed.len()
                    );
                    self.pending.clear();
                    self.watchdog_retried = true;
                    self.watchdog_ticks = 0;
                    self.submit_visible(cx);
                } else {
                    eprintln!(
                        "docx-pages: #186 看门狗——重派后仍无 tile 进账，在途条带\
                         转失败（不无限渲染中）"
                    );
                    let bands: Vec<(u32, u32)> =
                        self.pending.values().map(|(key, _)| *key).collect();
                    self.pending.clear();
                    for key in bands {
                        self.failed.insert(key);
                    }
                    cx.notify();
                }
            }
        }
    }

    /// #186：视口探针读数→派单（轮询拍接管——探针 paint 写 Cell 后由
    /// 本方法收口，变化才 notify：视口稳定后零额外渲染）。
    fn refresh_viewport(&mut self, cx: &mut Context<Self>) {
        if let Some((w, h)) = self.viewport.get() {
            if w > 0. && h > 0. && (w, h) != self.viewport_seen {
                eprintln!("docx-pages: #186 视口探针 {w:.0}x{h:.0}px（测量→派单接管）");
                self.viewport_seen = (w, h);
                self.submit_visible(cx);
                cx.notify();
            }
        }
    }

    /// 在途超时巡检（#186：每 16ms 拍随 drain）。超时条目离在途：未过
    /// 尝试上限 → 下一帧 `submit_visible` 重派（本巡检 notify 驱动那一
    /// 帧）；过限 → 失败面（占位框明示「渲染失败」——invalidate/换档
    /// 换光栅宽可重试）。巡检后到的迟到 tile 对不上号——机械丢弃。
    fn expire_stale_pending(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let expired: Vec<u64> = self
            .pending
            .iter()
            .filter(|(_, (_, at))| now.duration_since(*at) >= TILE_PENDING_TIMEOUT)
            .map(|(seq, _)| *seq)
            .collect();
        if expired.is_empty() {
            return;
        }
        for seq in expired {
            let Some((key, _)) = self.pending.remove(&seq) else {
                continue;
            };
            let attempts = self.attempts.entry(key).or_insert(0);
            *attempts += 1;
            if *attempts >= TILE_MAX_ATTEMPTS {
                eprintln!(
                    "docx-pages: tile band={} rw={} 重派后仍 {}s 无回执——转失败面",
                    key.0,
                    key.1,
                    TILE_PENDING_TIMEOUT.as_secs()
                );
                self.failed.insert(key);
            } else {
                eprintln!(
                    "docx-pages: tile band={} rw={} 在途 {}s 无回执——清出在途待重派",
                    key.0,
                    key.1,
                    TILE_PENDING_TIMEOUT.as_secs()
                );
            }
        }
        cx.notify();
    }

    /// #179：会话通道断开且本批未含终态回执（Failed/Died）——会话
    /// 线程异常退出（panic/被杀），与「暂时无消息」分流：落可定位
    /// 失败 + 停轮询（session 清空），不再永挂 Starting/Ready。
    /// 正常收尾（终态回执后线程退出）不误报——terminal_seen 卫。
    /// `disconnected_abnormal` = 通道断开**且**本批无终态回执（#179 分流
    /// 语义——正常收尾（终态后退出）不误报）。
    fn handle_session_disconnect(&mut self, disconnected_abnormal: bool, cx: &mut Context<Self>) {
        if disconnected_abnormal {
            eprintln!("docx-pages: 会话通道断开（未收到终态回执——会话线程异常退出）");
            self.session = None;
            self.pending.clear();
            if matches!(self.boot, Boot::Starting | Boot::Ready) {
                self.boot = Boot::Failed {
                    stage: "session",
                    message: "会话线程异常退出（通道断开且无终态回执）".into(),
                };
            }
            cx.notify();
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
                self.cache_costs
                    .retain(|(band, _), _| !range.contains(band));
                self.cache_order.retain(|(band, _)| !range.contains(band));
                self.cache_bytes = self.cache_costs.values().sum();
                self.failed.retain(|(band, _)| !range.contains(band));
                self.pending
                    .retain(|_, ((band, _), _)| !range.contains(band));
                self.attempts.retain(|(band, _), _| !range.contains(band));
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
        self.attempts.clear();
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
                || self.pending.values().any(|(k, _)| *k == key)
            {
                continue;
            }
            if let Some(geom) = band_region(w_tw, h_tw, rw, key.0) {
                todo.push((key, geom));
            }
        }
        // 相 2（会话借用 + pending 写入——字段不相交）。派单一批一条
        // 汇总日志（#186 同次可追踪证据：视口/偏移/光栅宽/带区间/在途）。
        if !todo.is_empty()
            && let Some(session) = &self.session
        {
            let mut dispatched = 0usize;
            for (key, geom) in todo {
                let seq = session.request_tile(
                    geom.canvas_w as usize,
                    geom.canvas_h as usize,
                    geom.tile_x,
                    geom.tile_y,
                    geom.tile_w,
                    geom.tile_h,
                );
                self.pending.insert(seq, (key, Instant::now()));
                dispatched += 1;
            }
            eprintln!(
                "docx-pages: 派单 +{dispatched}（视口 {vw:.0}x{vh:.0} 偏移 y={offset_y:.0} \
                 rw={rw} band {first}..{last}）→ 在途共 {}",
                self.pending.len()
            );
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
                // 在途已派单时长（无进展可见——#186：不无限「渲染中」；
                // 超时面见 expire_stale_pending）。
                let pending_secs = self
                    .pending
                    .values()
                    .find(|(k, _)| *k == key)
                    .map(|(_, at)| at.elapsed().as_secs());
                div()
                    .id(gpui::SharedString::from(format!("docx-band-pending-{idx}")))
                    .debug_selector(move || format!("docx-band-pending-{idx}"))
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
                            format!(
                                "条带 {band_no} 渲染失败（{}s×{} 无回执）",
                                TILE_PENDING_TIMEOUT.as_secs(),
                                TILE_MAX_ATTEMPTS
                            )
                        } else if let Some(secs) = pending_secs {
                            format!("条带 {band_no} 渲染中…（已 {secs}s）")
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
                    .id(gpui::SharedString::from(format!(
                        "docx-zoom-preset-{label}"
                    )))
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
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.zoom_menu_open = false;
                        cx.notify();
                    }),
                )
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
        let probe_entity = entity.clone();
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
            // 本批改根级口径）+ 量得变更即自排一帧（#186：Ready 后
            // 无输入也必须派单——pdf_pages 同款纪律）+ Ctrl+滚轮
            // capture 期拦截（缩放——止冒泡防列表滚动，pdf #421 同款）。
            .child(
                div()
                    .id("docx-viewport-probe")
                    .debug_selector(|| "docx-viewport-probe".into())
                    .absolute()
                    .inset_0()
                    .size_full()
                    .child(
                        // canvas 显式满尺寸：`Style::default()` 的块级子元素
                        // height:auto 会塌成 0（测试窗实测 800×0——#186 定位
                        // 线索「canvas 未显式设置尺寸」证实）。显式 size_full
                        // 确定性铺满探针 div——全平台一致，不赌平台布局差异。
                        gpui::canvas(
                            move |bounds, _window, _cx| bounds,
                            move |bounds: gpui::Bounds<gpui::Pixels>,
                                  _t,
                                  window: &mut Window,
                                  _cx: &mut App| {
                                let measured =
                                    (f32::from(bounds.size.width), f32::from(bounds.size.height));
                                if measured.0 > 0.
                                    && measured.1 > 0.
                                    && viewport_cell.get() != Some(measured)
                                {
                                    viewport_cell.set(Some(measured));
                                    eprintln!(
                                        "docx-pages: viewport 探针 {:.0}x{:.0}px（变更——下一帧消费）",
                                        measured.0, measured.1
                                    );
                                    // Paint 不能更新被借用的视图——量得的视口
                                    // 要到下一帧才进 submit_visible 派单，必须
                                    // 自排一帧后续帧（pdf_pages 同款纪律）；
                                    // 缺此通知 = #186：Ready 后永不派单，
                                    // 条带永远「渲染中」。refresh_viewport
                                    // 轮询拍收口是第二保险（不依赖帧时机）。
                                    window.on_next_frame(move |_, cx| {
                                        let _ = probe_entity.update(cx, |_, cx| cx.notify());
                                    });
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
                                        if !(event.modifiers.control || event.modifiers.platform)
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
                        )
                        .size_full(),
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

    /// #186 测试注桩会话（TileSession seam）：记录派单参数（对账 seq
    /// = 记录序）；预置回执队列逐条弹出；`auto_reply` = 派单即回 2×2
    /// tile（全绿路径）。Arc 字段共享——clone 后状态同源。
    #[derive(Clone)]
    struct MockSession {
        requests: Arc<std::sync::Mutex<Vec<TileRequest>>>,
        replies: Arc<std::sync::Mutex<VecDeque<SessionReply>>>,
        auto_reply: bool,
    }

    impl MockSession {
        fn new(auto_reply: bool) -> Self {
            Self {
                requests: Arc::default(),
                replies: Arc::default(),
                auto_reply,
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
                self.replies.lock().unwrap().push_back(SessionReply::Tile {
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
                    elapsed_ms: 1,
                });
            }
            seq
        }
        fn request_size(&self) {}
        fn try_recv(&self) -> Option<SessionReply> {
            self.replies.lock().unwrap().pop_front()
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

    /// #186：窗口调整 → 探针重量（变更守卫）→ 自排帧 → 新视口口径
    /// 再派单（800×600 → 1000×700：光栅宽 832 → 1024）。验收面
    /// 「窗口调整后能首屏派单」。
    #[gpui::test]
    fn resize_remeasures_and_redispatches(cx: &mut gpui::TestAppContext) {
        cx.update(|app| {
            theme::init(theme::LoadThemes::JustBase, app);
            theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
        });
        let path =
            std::env::temp_dir().join(format!("mdc-docx-resize-{}.docx", std::process::id()));
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
        let path =
            std::env::temp_dir().join(format!("mdc-docx-zoomrd-{}.docx", std::process::id()));
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
        let view = cx.update(|app| {
            app.new(|cx| DocxPagesView::new_with_open_rx_for_test(cx, Some(open_rx)))
        });
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
        let view = cx.update(|app| {
            app.new(|cx| DocxPagesView::new_with_open_rx_for_test(cx, Some(open_rx)))
        });
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
        let view = cx.update(|app| {
            app.new(|cx| DocxPagesView::new_with_open_rx_for_test(cx, Some(open_rx)))
        });
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
        let view = cx.update(|app| {
            app.new(|cx| DocxPagesView::new_with_open_rx_for_test(cx, Some(open_rx)))
        });
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
}
