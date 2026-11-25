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
//!   重派一次，再超转失败面——无进展不无限「渲染中」。#193 统一
//!   请求级超时：看门狗只审计「覆盖缺口」（可见条带既未缓存、未
//!   失败、也无在途——派单丢失补派一次），在途一律归键级 15s 管
//!   辖；健康覆盖静置零动作、零日志、零 notify。
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
/// #193 派单看门狗拍数（≈2 秒覆盖缺口未消 → 补派一次兜底）。只管
/// 「可见条带既未缓存、未失败、也无在途」的覆盖缺口（派单丢失自
/// 愈——失效清理后无重派路径等）；在途请求一律归键级 15s 超时管辖，
/// 看门狗不清在途、不标失败、不绕过 15s×2 键级策略。健康覆盖
/// （缺口为空）= 静置零动作（#193：健康缓存静置 ≥60s 无重派/失败/
/// notify 风暴）。
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
    /// #50 C3：文本输入（默认未接线返回 false——mock 未实现编辑面时
    /// 键盘面如实「不可编辑」，不假绿）。
    fn edit_type_text(&mut self, _seq: u64, _text: &str) -> bool {
        false
    }
    /// #50 C3：UNO 命令（同上——默认不可用）。
    fn edit_uno(&mut self, _seq: u64, _command: &str, _args: Option<&str>, _query: bool) -> bool {
        false
    }
    /// #50 C3：剪贴板读（同上）。
    fn edit_text_of(&mut self, _seq: u64, _mime: &str) -> bool {
        false
    }
    /// #50 C4：保存（同上）。
    fn edit_save_as(&mut self, _seq: u64, _url: &str, _format: &str) -> bool {
        false
    }
    /// #50 C5：页几何查询（`getPartPageRectangles`——跳页/翻页 tile 的
    /// 页矩形坐标源；默认未接线返回 false，mock 未实现时调用面如实
    /// 「不可跳页」不假绿。真链消费 = real_chain_c5_matrix_parity；
    /// 产品翻页呈现接后续批次）。
    fn edit_page_geometry(&mut self, _seq: u64) -> bool {
        false
    }
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
    /// #50 C3：编辑命令——`DocxLokSession` 的 request_* 面转发（seq
    /// 由本视图统一分配，回执经同一 `try_recv` 通道）。
    fn edit_type_text(&mut self, seq: u64, text: &str) -> bool {
        self.request_type_text(seq, text);
        true
    }
    fn edit_uno(&mut self, seq: u64, command: &str, args: Option<&str>, query: bool) -> bool {
        self.request_uno(seq, command, args, query);
        true
    }
    fn edit_text_of(&mut self, seq: u64, mime: &str) -> bool {
        self.request_text_of(seq, mime);
        true
    }
    /// #50 C4：保存（file_url 交内核——B 线 profile::file_url_of 同源
    /// 语义由会话线程内部承担，宿主传 OS 路径）。
    fn edit_save_as(&mut self, seq: u64, url: &str, format: &str) -> bool {
        self.request_save_as(seq, url, format);
        true
    }
    /// #50 C5：页几何转发（真链通道——C5 表格跨页样本已实证必需：
    /// 页矩形是跳页 tile 的唯一正确坐标锚）。
    fn edit_page_geometry(&mut self, _seq: u64) -> bool {
        // 自分 seq（request_page_geometry 无参形态——trait 臂收 _seq
        // 只为签名对齐；回执 PageGeometry{seq} 由调用面 drain 对账）。
        self.request_page_geometry();
        true
    }
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
/// #50 C3：编辑命令序号（独立静态域——与 tile SEQ 不共空间；回执
/// 路由按 `edit_pending` 对账）。
static EDIT_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_edit_seq() -> u64 {
    EDIT_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// #50 C3：编辑面命令类别（回执路由——`EditDone`/`TextDone`/
/// `SaveDone` 三回执共 seq 空间，按类别分流）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EditKind {
    Type,
    Uno,
    TextOf,
    Save,
}

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
    /// #50 C3/C4：内核侧脏位（`EditDone{ok,dirty}` 置位——成功编辑
    /// 即脏；失败回执保持原值）。
    dirty: bool,
    /// #50 C4：宿主侧修订号（`EditDone{revision}` 更新——保存对账的
    /// 「当前」侧；`SaveDone{revision_saved}` 相等才清 dirty，防延迟
    /// 回执清掉新编辑）。
    revision: u64,
    /// #50 C3：编辑/保存/剪贴板命令的回执在途（seq → 命令类别——回执
    /// 路由；切标签 Drop 后自然作废=旧回执不落新文档）。
    edit_pending: HashMap<u64, EditKind>,
    /// #50 C3：IME preedit 组合串（宿主渲染——契约 §10 口径：组合期
    /// 仅宿主显示，定稿 `type_text` 逐字符下发；空 = 无组合）。
    preedit: String,
    /// 视口尺寸回写（根级探针 canvas paint 期写、render 期读——
    /// pdf_pages 同款：避免每帧实体更新）。
    viewport: Rc<Cell<Option<(f32, f32)>>>,
    viewport_size: (f32, f32),
    /// #186：视口探针已 paint 到的尺寸（`refresh_viewport` 的变化检测
    /// 基线——探针首帧写 Cell 后由轮询拍接管派单，交互不再前置）。
    viewport_seen: (f32, f32),
    /// #193 看门狗：Ready 后「覆盖缺口」持续拍数（缺口 = 既未缓存、
    /// 未失败、也无在途派单的可见条带非空）——缺口为空即复位（健康
    /// 静置零动作）；持续超 [`WATCHDOG_TICKS`] 补派一次。
    watchdog_ticks: u32,
    /// #193 看门狗：本故障周期已补派过一次（有界——补派后仍缺覆盖
    /// 才登记失败面；缺口闭合或视口/内容代际切换时复位预算）。
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
            // #50 C3/C4：编辑状态面初始零值（会话建立后由回执驱动）。
            dirty: false,
            revision: 0,
            edit_pending: HashMap::new(),
            preedit: String::new(),
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

    /// 测试辅助：preedit 域读出（IME 测试断言面）。
    #[cfg(test)]
    pub(crate) fn preedit_for_test(&self) -> String {
        self.preedit.clone()
    }

    /// 测试辅助：编辑/保存在途命令数（终态出清断言面）。
    #[cfg(test)]
    pub(crate) fn edit_pending_len_for_test(&self) -> usize {
        self.edit_pending.len()
    }

    /// #50 C3/C4：真实脏位（`EditDone{ok,dirty}` 回执置位；`SaveDone`
    /// 对账清零——见 [`Self::handle_save_done`]；查询面给关闭确认链/
    /// save_tab 分流消费）。
    pub fn is_dirty(&self) -> bool {
        self.dirty
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

    /// #177 像素级基线：缓存条带的原始 BGRA 读出（`(w, h, 字节)`）。
    ///
    /// `gpui::RenderImage::as_bytes` 公开 API（third_party/zed/crates/
    /// gpui/src/assets.rs:76）——缓存消费终点即解码像素，测试在此层
    /// 做「渲染出来的像素像不像一份文档」断言（零产线改动）。
    #[cfg(test)]
    pub fn cached_image_for_test(&self, key: (u32, u32)) -> Option<(usize, usize, Vec<u8>)> {
        let img = self.cache.get(&key)?;
        let size = img.size(0);
        let bytes = img.as_bytes(0)?.to_vec();
        Some((
            u32::from(size.width) as usize,
            u32::from(size.height) as usize,
            bytes,
        ))
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

    /// #193 测试面：看门狗故障周期状态（覆盖缺口持续拍数，本周期已
    /// 补派标记）。
    #[cfg(test)]
    pub fn watchdog_state_for_test(&self) -> (u32, bool) {
        (self.watchdog_ticks, self.watchdog_retried)
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
    /// `docx_geom` 互逆纯函数；预设菜单点击/键位同走此口）。换档 =
    /// 视口代际切换——复位 #193 看门狗故障周期（新光栅宽键域重新
    /// 获得补派预算）。
    fn set_zoom_mode(&mut self, mode: ZoomMode, cx: &mut Context<Self>) {
        self.watchdog_cycle_reset();
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

    /// #50 C3：编辑键域分发（docx 页面态键入主入口——shell 键钩子在
    /// 缩放键分流后调用；`false` = 未消费继续走 keymap 表）。
    ///
    /// 键位面（Q-IDE/ElyOffice 键语义——QWERTY）：
    /// - 字符键（含 CJK——无修饰）：`type_text` 逐字符下发（选区被
    ///   替换的标准编辑语义由内核承担）；
    /// - Backspace/Delete：`.uno:Delete` / `.uno:DelToEndOfPara`（保守
    ///   两键——Delete 视同 Backspace 同 `.uno:Delete`，不猜造未实证
    ///   的 toEndOfWord 细分）；
    /// - Enter：`.uno:InsertPara`；Tab：`.uno:InsertPara` 不对——Tab
    ///   是 `.uno:InsertTab`；
    /// - 方向/Home/End/PgUp/PgDn：光标移动 UNO（`GoLeft/GoRight/
    ///   GoUp/GoDown/StartOfLine/EndOfLine/PageUp/PageDown`——查询
    ///   类不置脏）；
    /// - Ctrl+Z/Y（Ctrl+Shift+Z）：Undo/Redo；
    /// - Ctrl+A：SelectAll（查询类）；Ctrl+C：TextOf+剪贴板写；
    ///   Ctrl+X：TextOf + 剪贴板写 + `.uno:Cut`；Ctrl+V：剪贴板读 +
    ///   `type_text`（富文本面后续批次——纯文本保守口径）；
    /// - Ctrl+B/I/U：粗体/斜体/下划线（`.uno:Bold` 等——编辑类）。
    ///
    /// IME（契约 §10 口径）：preedit 宿主渲染（[`Self::preedit`]——
    /// render 期状态面板叠加组合串），定稿/提交经 GPUI `EntityInputHandler`
    /// 的 replace_text_in_range 走同一 `type_text` 通道（见 impl 块）。
    pub fn dispatch_edit_key(
        &mut self,
        key: &str,
        ctrl: bool,
        _shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        // 就绪卫：Starting/Failed 态键入零动作（不假绿——不可编辑态如实
        // 未消费，落 keymap/shell 兜底）。
        if !matches!(self.boot, Boot::Ready) || self.session.is_none() {
            return false;
        }
        let ks = gpui::Keystroke {
            modifiers: gpui::Modifiers {
                control: ctrl,
                ..Default::default()
            },
            key: key.to_owned(),
            key_char: None,
        };
        let _ = (&ks, window);
        // 单字符文本键（无修饰——ctrl 组合是命令域）。
        if !ctrl && key.chars().count() == 1 {
            let ch = key.chars().next().unwrap();
            self.send_edit(
                EditKind::Type,
                |session, seq| session.edit_type_text(seq, &ch.to_string()),
                cx,
            );
            return true;
        }
        if ctrl {
            return self.dispatch_ctrl_edit(key, cx);
        }
        // 无 ctrl 的功能键。
        match key {
            "backspace" | "delete" => {
                self.send_uno_edit(".uno:Delete", None, false, cx);
                true
            }
            "enter" => {
                self.send_uno_edit(".uno:InsertPara", None, false, cx);
                true
            }
            "tab" => {
                self.send_uno_edit(".uno:InsertTab", None, false, cx);
                true
            }
            "left" | "right" | "up" | "down" | "home" | "end" | "pageup" | "pagedown" => {
                let command = match key {
                    "left" => ".uno:GoLeft",
                    "right" => ".uno:GoRight",
                    "up" => ".uno:GoUp",
                    "down" => ".uno:GoDown",
                    "home" => ".uno:StartOfLine",
                    "end" => ".uno:EndOfLine",
                    "pageup" => ".uno:PageUp",
                    _ => ".uno:PageDown",
                };
                self.send_uno_edit(command, None, true, cx);
                true
            }
            _ => false,
        }
    }

    /// Ctrl 组合的编辑命令分发（`dispatch_edit_key` 的 ctrl 分支）。
    fn dispatch_ctrl_edit(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        match key {
            "z" | "y" => {
                // Ctrl+Shift+Z = Redo（y 与 shift+z 双键位——QWERTY 通行）。
                let command = if key == "z" { ".uno:Undo" } else { ".uno:Redo" };
                self.send_uno_edit(command, None, false, cx);
                true
            }
            "a" => {
                self.send_uno_edit(".uno:SelectAll", None, true, cx);
                true
            }
            "c" => {
                // copy：读选区 → TextDone 回执写系统剪贴板。
                self.send_edit(
                    EditKind::TextOf,
                    |session, seq| session.edit_text_of(seq, "text/plain;charset=utf-8"),
                    cx,
                );
                true
            }
            "x" => {
                // cut：读选区（回执写剪贴板）+ Delete（读回执后独立的
                // 删除命令——两命令独立 seq，内核序即所见序）。
                self.send_edit(
                    EditKind::TextOf,
                    |session, seq| session.edit_text_of(seq, "text/plain;charset=utf-8"),
                    cx,
                );
                self.send_uno_edit(".uno:Cut", None, false, cx);
                true
            }
            "v" => {
                // paste：剪贴板读 + type_text（纯文本保守口径——富文本
                // 面 B 线 mime 通道后续批次）。
                if let Some(text) = self.clipboard_read(cx) {
                    self.send_edit(
                        EditKind::Type,
                        |session, seq| session.edit_type_text(seq, &text),
                        cx,
                    );
                }
                true
            }
            "b" | "i" | "u" => {
                let command = match key {
                    "b" => ".uno:Bold",
                    "i" => ".uno:Italic",
                    _ => ".uno:Underline",
                };
                self.send_uno_edit(command, None, false, cx);
                true
            }
            _ => false,
        }
    }

    /// 编辑命令发送（seq 分配 + 在途登记 + trait 面派发——mock/生产
    /// 同一调用面；未接线（trait 默认 false）时如实清在途不假绿）。
    fn send_edit(
        &mut self,
        kind: EditKind,
        f: impl FnOnce(&mut dyn TileSession, u64) -> bool,
        cx: &mut Context<Self>,
    ) {
        let seq = next_edit_seq();
        let sent = match self.session.as_mut() {
            Some(session) => f(session.as_mut(), seq),
            None => false,
        };
        if sent {
            self.edit_pending.insert(seq, kind);
        }
        cx.notify();
    }

    /// UNO 编辑命令便捷面（`send_edit` 的 Uno 特化）。
    fn send_uno_edit(
        &mut self,
        command: &str,
        args: Option<&str>,
        query: bool,
        cx: &mut Context<Self>,
    ) {
        self.send_edit(
            EditKind::Uno,
            |session, seq| session.edit_uno(seq, command, args, query),
            cx,
        );
    }

    /// #50 C4：保存请求（shell save_tab 分流消费面——回源路径
    /// `file://` + 修订号对账清 dirty）。
    pub fn request_save(&mut self, cx: &mut Context<Self>) -> bool {
        if !matches!(self.boot, Boot::Ready) || self.session.is_none() {
            return false;
        }
        let path = self.path.clone();
        self.send_edit(
            EditKind::Save,
            |session, seq| {
                // url 形态：内核 `profile::file_url_of` 同源语义（POSIX 路径
                // 直拼 `file://`——生产路径已 canonicalize）。
                let url = format!("file://{}", path.display());
                session.edit_save_as(seq, &url, "docx")
            },
            cx,
        );
        true
    }

    /// #50 C3：剪贴板写（TextDone 回执消费面——`ClipboardItem::
    /// new_string` 编辑器同款；`Context<Self>` 的 AppContext 面直调）。
    fn clipboard_write(&mut self, cx: &mut Context<Self>, text: &str) {
        use gpui::ClipboardItem;
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
    }

    /// #50 C3：剪贴板读（paste 派发面——`item.text()` 编辑器
    /// EditorAction::Paste 同款）。
    fn clipboard_read(&self, cx: &App) -> Option<String> {
        cx.read_from_clipboard().and_then(|i| i.text())
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
                    // #50 C3：编辑/保存回执通道同亡——在途命令出清
                    //（终态后 EditDone/TextDone/SaveDone 永不再来，「在途」
                    // 永挂会让保存发起面误判进行中）。
                    self.edit_pending.clear();
                    self.boot = Boot::Failed { stage, message };
                    cx.notify();
                }
                SessionReply::Died { detail } => {
                    terminal_seen = true;
                    eprintln!("docx-pages: worker 死亡：{detail}");
                    self.session = None;
                    self.pending.clear();
                    self.attempts.clear();
                    // 同上（Failed 分支注释）。
                    self.edit_pending.clear();
                    self.boot = Boot::Failed {
                        stage: "died",
                        message: detail,
                    };
                    cx.notify();
                }
                SessionReply::PageGeometry { seq: _, pages: _ } => {
                    // #50 C5：页几何回执（跳页坐标源）。本批消费者 =
                    // 测试真链（real_chain_c5_matrix_parity）；产品
                    // docx_pages 翻页呈现接后续批次（数据面已通——此
                    // 处显式分支不静默吞，防将来误当未知回执报警）。
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
                // #50 C3：编辑命令完成——dirty/revision 状态面落位
                //（成功编辑置脏；失败保持原值——「编辑不可用」的根因
                // 由 error 串在 stderr 留痕，不弹横幅打断输入流）。
                SessionReply::EditDone {
                    seq,
                    ok,
                    dirty,
                    revision,
                    error,
                } => {
                    self.edit_pending.remove(&seq);
                    if ok {
                        self.dirty = dirty;
                        self.revision = revision;
                    } else if let Some(err) = error {
                        eprintln!("docx-pages: edit failed: {err}");
                    }
                    cx.notify();
                }
                // #50 C3：剪贴板读回执——宿主写系统剪贴板（copy 面
                // 完成即写；cut 由先 TextOf 后 Uno 的调用序在
                // dispatch_edit_key 内编排）。
                SessionReply::TextDone { seq, text, error } => {
                    self.edit_pending.remove(&seq);
                    if let Some(err) = error {
                        eprintln!("docx-pages: clipboard read failed: {err}");
                    } else {
                        self.clipboard_write(cx, &text);
                    }
                }
                // #50 C4：保存完成——revision 对账（相等才清 dirty：
                // 延迟回执 revision_saved 落后当前 revision = 期间又有
                // 新编辑，不清——「保存回执清除新编辑 dirty」的病灶根除）。
                SessionReply::SaveDone {
                    seq,
                    ok,
                    revision_saved,
                    error,
                } => {
                    self.edit_pending.remove(&seq);
                    if let Some(err) = error {
                        eprintln!("docx-pages: save failed: {err}");
                    }
                    if ok && revision_saved == self.revision {
                        self.dirty = false;
                    }
                    cx.notify();
                }
            }
        }

        // ---- 4) #179 会话通道断开分流（断开且本批无终态回执 = 异常
        // 退出，落可定位失败——正常收尾误报由 terminal_seen 卫）----
        self.handle_session_disconnect(!channel_alive && !terminal_seen, cx);

        // ---- 5) #193 派单看门狗（每拍收口——统一请求级超时策略）----
        // 键级 15s 超时（expire_stale_pending）是唯一的在途过期/重试/
        // 失败判定权威；本看门狗只审计「覆盖缺口」：存在既未缓存、未
        // 失败、也无在途的可见条带（失效清理后无重派路径、派单丢失
        // 等）持续 [`WATCHDOG_TICKS`] 拍 → 补派一次（submit_visible 对
        // 已覆盖键幂等）；补派后仍缺覆盖 → 缺带登记失败面一次。健康
        // 覆盖（缺口为空）= 静置零动作、零日志、零 notify（#193：健
        // 康缓存静置 ≥60s 无重派/失败/notify 风暴——旧 known_missing
        // 把「缓存非空」当「缺货」的误判面在此根除）。任何回执不复
        // 位计数——晚到/他键回执不能误复位新代际；缺口闭合或代际切
        // 换（视口变更/失效/换档）才复位补派预算。
        if matches!(self.boot, Boot::Ready) {
            self.refresh_viewport_size_from_probe();
            let missing = self.uncovered_visible_keys();
            if missing.is_empty() {
                if self.watchdog_ticks != 0 || self.watchdog_retried {
                    // 故障周期闭合（缺口消解）——复位补派预算。
                    self.watchdog_ticks = 0;
                    self.watchdog_retried = false;
                }
            } else {
                self.watchdog_ticks = self.watchdog_ticks.saturating_add(1);
                if self.watchdog_ticks >= WATCHDOG_TICKS {
                    if !self.watchdog_retried {
                        eprintln!(
                            "docx-pages: #193 看门狗——{WATCHDOG_TICKS} 拍覆盖缺口未消（缺 {} 带）——补派一次（pending={} cache={} failed={}）",
                            missing.len(),
                            self.pending.len(),
                            self.cache.len(),
                            self.failed.len()
                        );
                        self.watchdog_retried = true;
                        self.watchdog_ticks = 0;
                        self.submit_visible(cx);
                    } else {
                        eprintln!(
                            "docx-pages: #193 看门狗——补派后仍缺覆盖，缺带登记失败面（{} 带，失效/换档可重试）",
                            missing.len()
                        );
                        for key in missing {
                            self.failed.insert(key);
                        }
                        self.watchdog_ticks = 0;
                        cx.notify();
                    }
                }
            }
        }
    }

    /// #193：视口探针读数→派单（轮询拍接管——探针 paint 写 Cell 后由
    /// 本方法收口，变化才 notify：视口稳定后零额外渲染）。视口代际
    /// 切换同时复位看门狗故障周期（新代际重新获得补派预算）。
    fn refresh_viewport(&mut self, cx: &mut Context<Self>) {
        if let Some((w, h)) = self.viewport.get() {
            if w > 0. && h > 0. && (w, h) != self.viewport_seen {
                eprintln!("docx-pages: #186 视口探针 {w:.0}x{h:.0}px（测量→派单接管）");
                self.viewport_seen = (w, h);
                self.watchdog_cycle_reset();
                self.submit_visible(cx);
                cx.notify();
            }
        }
    }

    /// #193 看门狗故障周期复位（视口/内容代际切换：新代际重新获得补
    /// 派预算）。晚到旧代际回执本就被 seq 对账机械丢弃——不复位、不
    /// 污染，无需额外代际号。
    fn watchdog_cycle_reset(&mut self) {
        self.watchdog_ticks = 0;
        self.watchdog_retried = false;
    }

    /// 探针读数回写视口尺寸（submit_visible 与 #193 覆盖审计共用的
    /// 口径收口——读 Cell 写 viewport_size，零值守卫）。
    fn refresh_viewport_size_from_probe(&mut self) {
        if let Some((w, h)) = self.viewport.get()
            && w > 0.
            && h > 0.
        {
            self.viewport_size = (w, h);
        }
    }

    /// 可见条带区间（`first..last`，光栅宽）——submit_visible 派单与
    /// #193 覆盖审计共用的单一口径（滚动偏移 → 起始带，视口高 → 带
    /// 数，±1 观测余量）。几何/视口未知 → None（无从派单/审计）。
    fn visible_band_range(&self) -> Option<(usize, usize, u32)> {
        let (vw, vh) = self.viewport_size;
        if vw <= 0. || vh <= 0. {
            return None;
        }
        let (w_tw, h_tw) = self.doc_size?;
        let rw = self.raster_width(vw);
        let count = band_count(w_tw, h_tw, rw);
        if count == 0 {
            return None;
        }
        // 滚动偏移（gpui 负向约定——取负归正；uniform_list 以首项测高，
        // 末带偏短属固有近似——条带步进恒 BAND_PX）。
        let offset_y = (-f32::from(self.scroll.0.borrow().base_handle.offset().y)).max(0.);
        let first = (offset_y as usize / BAND_PX as usize).saturating_sub(1);
        let visible_bands = (vh as usize / BAND_PX as usize + 2).max(1);
        let last = (first + visible_bands).min(count);
        Some((first, last, rw))
    }

    /// #193 覆盖审计：当前视口下既未缓存、未失败、也无在途派单的
    /// 可见条带键集（空 = 健康覆盖——静置零动作；非空 = 派单链缺
    /// 口，看门狗补派兜底）。键口径与 submit_visible 的 todo 同源
    /// （同一区间单源），不重复派单已覆盖键。
    fn uncovered_visible_keys(&self) -> Vec<(u32, u32)> {
        let Some((first, last, rw)) = self.visible_band_range() else {
            return Vec::new();
        };
        (first..last)
            .filter(|band| {
                let key = (*band as u32, rw);
                !self.cache.contains_key(&key)
                    && !self.failed.contains(&key)
                    && !self.pending.values().any(|(k, _)| *k == key)
            })
            .map(|band| (band as u32, rw))
            .collect()
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
    /// 在途/失败登记同步清除——陈旧像素宁可重画不可误显）。内容代际
    /// 切换——复位 #193 看门狗故障周期。返回逐出缓存条数。
    fn apply_invalidate(&mut self, scope: InvalScope) -> usize {
        self.watchdog_cycle_reset();
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

    /// 可见条带派单（render 期调用——读根级探针上帧记录的视口；
    /// 区间口径与 #193 覆盖审计同源——visible_band_range 单源）。
    fn submit_visible(&mut self, _cx: &mut Context<Self>) {
        if !matches!(self.boot, Boot::Ready) {
            return;
        }
        self.refresh_viewport_size_from_probe();
        let Some((first, last, rw)) = self.visible_band_range() else {
            return;
        };
        let Some((w_tw, h_tw)) = self.doc_size else {
            return;
        };
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
            let offset_y = (-f32::from(self.scroll.0.borrow().base_handle.offset().y)).max(0.);
            eprintln!(
                "docx-pages: 派单 +{dispatched}（视口 {}x{} 偏移 y={offset_y:.0} \
                 rw={rw} band {first}..{last}）→ 在途共 {}",
                self.viewport_size.0,
                self.viewport_size.1,
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
        // #176：状态面板同样接焦点域（focus_active 在 Starting/Failed
        // 态也落本视图句柄——不接则句柄游离，键事件死在合成根，
        // #604 同款病灶；Ctrl+S 的明确提示在错误面板态同样要可达）。
        let focus = self.focus_handle.clone();
        v_flex()
            .id("docx-pages-state")
            .debug_selector(|| "docx-pages-state".into())
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .bg(colors.background)
            .track_focus(&focus)
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

/// #50 C3：IME 输入通道（gpui 平台 → 视图实体——契约 §10 口径：
/// preedit 宿主渲染不进内核；提交文本经 `type_text` 逐字符下发）。
/// 「哪个视图持焦点哪个实体收 IME」——gpui 的 `ElementInputHandler`
/// 在视图根的 canvas paint 期注册（与编辑器 ime-hook 同模式）。
impl gpui::EntityInputHandler for DocxPagesView {
    fn set_selected_text_range(
        &mut self,
        _range: std::ops::Range<usize>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        // 选区归内核管理（`getSelection` 面后续批次）——IME 通道不接受
        // 宿主侧选区写。
    }

    fn text_length_utf16(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.preedit.encode_utf16().count())
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        // 就绪会话才可编辑（Starting/Failed 态 IME 平台通道关闭——
        // 平台不会为其弹输入法）。
        matches!(self.boot, Boot::Ready)
    }

    fn text_for_range(
        &mut self,
        range: std::ops::Range<usize>,
        adjusted: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let text: String = self
            .preedit
            .chars()
            .take(range.end)
            .skip(range.start)
            .collect();
        *adjusted = Some(range);
        Some(text)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<gpui::UTF16Selection> {
        let len = self.preedit.encode_utf16().count();
        Some(gpui::UTF16Selection {
            range: len..len,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        if self.preedit.is_empty() {
            None
        } else {
            Some(0..self.preedit.encode_utf16().count())
        }
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.preedit.clear();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<std::ops::Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // #50 C3 修正（编辑器 EntityInputHandler 同构语义）：提交 =
        // 覆写替换 + **立即下发内核**。两形态：
        // - `Some(range)`：preedit 域内子段替换（拉丁快速键入的裸插入
        //   兜底——不经 marked 的直接文本）；
        // - `None`：Windows IME 提交/取消形态——替换整个 marked 域
        //   （取消 = 空串替换，语义自然正确）。
        // 提交后清 preedit（`marked = None` 同构）——组合串出清、浮层
        // 消失、光标域归零。
        let replacement = match range {
            Some(r) => {
                let start = utf16_to_char_idx(&self.preedit, r.start);
                let end = utf16_to_char_idx(&self.preedit, r.end);
                self.preedit.replace_range(start..end, text);
                text.to_owned()
            }
            None => {
                // 提交形态：text 即定稿全文。**取消（空串）= 出清组合
                // 串 + 零命令**（取消不是编辑——无替换、无 dirty）；非空
                // = 定稿新串（旧组合串出清弃用，以提交串为准）。
                let _ = std::mem::take(&mut self.preedit);
                text.to_owned()
            }
        };
        // 定稿非空 → type_text 下发内核（逐字符 post_key——契约 §10
        // 产品 IME 通道；取消（空串）零命令）。就绪卫内含（未就绪
        // accepts_text_input=false，平台不会派发到此处——防御位仍查）。
        if !replacement.is_empty() && matches!(self.boot, Boot::Ready) {
            if self.session.is_some() {
                self.send_edit(
                    EditKind::Type,
                    |session, seq| session.edit_type_text(seq, &replacement),
                    cx,
                );
            }
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<std::ops::Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 预编辑：preedit 域替换（无 preedit 时光标位=空段插入）。
        match range {
            Some(r) => {
                let start = utf16_to_char_idx(&self.preedit, r.start);
                let end = utf16_to_char_idx(&self.preedit, r.end);
                self.preedit.replace_range(start..end, new_text);
            }
            None => self.preedit = new_text.to_owned(),
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _element_bounds: gpui::Bounds<gpui::Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<gpui::Bounds<gpui::Pixels>> {
        None
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<gpui::Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl DocxPagesView {
    /// #50 C3 IME 提交通道说明（供后续批次读码）：提交语义已并入
    /// [`Self::replace_text_in_range`]（编辑器 EntityInputHandler
    /// 同构：替换 + `preedit` 出清 + 定稿 `type_text` 下发——本方法
    /// 保留为文档锚点，不再承担提交判定（marked_text_range/unmark
    /// 组合时序已由平台协议保证）。
    fn preedit_commit_if_standalone(&mut self, cx: &mut Context<Self>) {
        let _ = cx;
    }
}

/// UTF16 → char 边界索引（preedit 域换算——中文组合串安全）。
fn utf16_to_char_idx(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (idx, ch) in text.char_indices() {
        if units >= utf16 {
            return idx;
        }
        units += ch.len_utf16();
    }
    text.len()
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
        // #50 C3：IME 注册句柄（探针 paint 期 handle_input——焦点在
        // 视图时平台输入法路由到本实体）。
        let ime_focus = self.focus_handle.clone();
        let ime_entity = cx.entity();
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
        // #176：视图焦点域（focus_active 在页面态落本视图句柄——
        // 此前从不 track_focus，句柄游离，键事件死在合成根：#50 C2
        // 缩放键分支与 #176 键守卫实际不可达。pdf #422 / xlsx #389
        // 同款接线，三个替换视图归一）。#50 C3：on_key_down 编辑键域
        //（字符/功能/ctrl 命令经 dispatch_edit_key——消费即止冒泡，
        // 未消费键继续 shell 兜底链；视图在渲染树内时此入口优先于
        // shell 键钩子——焦点在视图上的键直收）。
        let focus = self.focus_handle.clone();
        let preedit = self.preedit.clone();
        let has_preedit = !preedit.is_empty();
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
            .track_focus(&focus)
            // #50 C3：编辑键域（焦点在视图时键直收——pdf/xlsx 同位
            // 键处理纪律；zoom 键 shell 分流在前，此处键序 = 缩放
            // （shell 已 return）→ 编辑（此处）→ 未消费冒泡 shell）。
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                let ks = &event.keystroke;
                let ctrl = ks.modifiers.control || ks.modifiers.platform;
                let key = ks.key.to_lowercase();
                // zoom 域先让 shell 分流——此处只接编辑域（Ctrl+0/1/=/
                // - 是视口命令非编辑，缩放不可达（shell 已 return 消费）；
                // 防御：显式跳过视口键族防双路径）。
                if ctrl && matches!(key.as_str(), "0" | "1" | "=" | "+" | "-") {
                    return;
                }
                if this.dispatch_edit_key(&key, ctrl, ks.modifiers.shift, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(list)
            // #50 C3：IME 接线（编辑器 ime-hook 同构——独立 size_0 canvas
            // 借 paint 时机注册 ElementInputHandler；不占布局。焦点在
            // 视图时平台输入法（中文组合/提交）路由到本实体）。
            .child(
                div()
                    .id("docx-ime-hook")
                    .absolute()
                    .size_0()
                    .child(gpui::canvas(
                        move |bounds, _, _| bounds,
                        move |bounds, _, window, cx| {
                            window.handle_input(
                                &ime_focus,
                                gpui::ElementInputHandler::new(bounds, ime_entity.clone()),
                                cx,
                            );
                        },
                    )),
            )
            // #50 C3：IME preedit 组合串（宿主渲染——契约 §10 口径：
            // 组合期仅宿主显示不进内核；定位跟随文本光标面暂以左上
            // 浮层保守呈现——精确光标矩形 B 线 `getSelectionRect` 通道
            // 接入后升级）。
            .when(has_preedit, |el| {
                el.child(
                    div()
                        .id("docx-preedit")
                        .debug_selector(|| "docx-preedit".into())
                        .absolute()
                        .top_8()
                        .left_8()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(colors.panel_background)
                        .border_1()
                        .border_color(colors.border)
                        .text_size(gpui::px(14.))
                        .text_color(colors.text)
                        .child(preedit),
                )
            })
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
#[path = "docx_pages_tests.rs"]
mod tests;
