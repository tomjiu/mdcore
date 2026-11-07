//! PDF 页面位图视图（#289-④——「快捷键打开文件 pdf 显示却是只有文字
//! 没有格式像 md」的修复面）。
//!
//! ## 定位
//!
//! 旧版 pdf 标签 = 文本行模型（[`crate::editor::pdf_doc`]——pdfium
//! `extract_text` → 行 + span，查找/书签/光标全复用 Editor 管线）。
//! 用户实测反馈：**PDF 必须长得像 PDF**——表格/图片/字体/版面要原样
//! 呈现，纯文本列表糊弄不过去。
//!
//! 本模块 = 真·页面渲染视图：mdc-pdf [`Rasterizer`]（pdfium）把每页
//! 光栅化为 BGRA 位图 → gpui `img`（`ImageSource::Render`）按页竖排。
//! **文本行模型不废弃**——两视图并存可切（状态栏「页面视图 / 文本
//! 视图」钮；Ctrl+F 在页面视图下自动切回文本视图再开查找条）：
//!
//! ```text
//! PdfPagesView（本模块——页面位图，主显示面）
//!      │  engine-pdfium 光栅化（RenderWorker 专用线程 + 看门狗换代）
//!      │  PageCache（RenderKey 键控 → RenderImage，LRU 字节预算）
//!      │  右键拖框批注：高亮/下划线/便签三手势（#302④ 验收 1：
//!      │    PdfAnnotateController 增量写面；退化单击 = 工具切换）
//!      └─ Editor（既有 pdf 文本模型——查找/书签锚/属性面板/字数）
//! ```
//!
//! ## 渲染管线（懒加载）
//!
//! - **uniform_list 虚拟化**：页槽等高（首页纵横比 × 适宽），只有
//!   可见页（±1 页过扫描）进渲染——百页档不爆内存；
//! - **懒光栅**：render 期按滚动偏移算可见区间，未缓存页经
//!   [`RenderWorker::submit`] 派单（专用渲染线程——UI 不卡帧）；
//! - **轮询收账**：提交时起一个 16ms 定时循环 `try_recv` 收回执，
//!   `Done` → `PageCache::put` + `cx.notify()`（下一帧换上真图），
//!   待办清空即退出循环（空闲零开销）；
//! - **宽度量子化**：缓存键的宽取 64px 倍数（窗口拖动时连续 resize
//!   不至于页页重光栅）；
//! - **#305 RenderKey 键控**：缓存键含 `annotation_rev`——批注
//!   flush（#302④）后旧位图自然 miss，重渲染即见新批注；旧 rev
//!   条目经 `invalidate_stale_revisions` 即刻回收预算。
//!
//! ## 批注交互（#302 阶段 2·④——验收 1 三手势）
//!
//! **右键拖框 = 按当前工具提交批注**（右键不占列表滚动）：按下记录
//! 页内起点 → 松开由 [`PdfAnnotateController`] 走完整写路径（#304
//! 坐标变换 → `AnnotateAdd` → 增量 flush）→ 本视图 `reload_after_flush`
//! （换字节快照 + 重建渲染工人 + 缓存失效）→ 下一帧重光栅即见批注。
//! 控制器首次使用时懒初始化（open_scan + 写器门一次性成本）。
//!
//! ## 缩放（#421——四态缩放域）
//!
//! - **状态**：[`ZoomMode`]——FitWidth（v1 现状适宽，默认零变化）/
//!   FitPage（整页可见）/ ActualSize（1pt:1px，百分比基准）/ Custom
//!   （相对实际大小的倍率——滚轮/键盘步进/预设档落此态）；
//! - **口径单点**：[`PdfPagesView::raster_width`] 是光栅宽的唯一解析点
//!   ——submit_visible（可见区间）/ render_page_slot（槽宽）/ commit_
//!   annotation（DPI）三消费点同源调用：**缩放后批注坐标正确性的
//!   关键**（issue 验收 3——#304 坐标变换的 DPI 基准随缩放同步）；
//! - **渲染关联**：RenderKey 键含（页, 宽）——缩放即宽变即新键，旧位
//!   图自然 miss 重光栅（无需改键结构；缓存预算不变 LRU 回收）；
//! - **交互**：Ctrl+滚轮（capture 期窗口级拦截——止冒泡防列表随缩
//!   放滚动；annotate_overlay paint 注册先例同款）+ 键域（shell 键
//!   分派前置分支，canvas 态键域同构——Ctrl+0 适应页面 / Ctrl+1 实
//!   际大小 / Ctrl+Shift+0 适应宽度 / Ctrl+=·Ctrl+- 步进）+ 右下角
//!   百分比 chip（[-][+] 步进钮；chip 点击弹预设菜单，backdrop 模态）；
//! - **滚动锚**：缩放后 offset × 槽高比例（首页纵横比恒定——页锚即
//!   内容锚，缩放前后视口内容位置不跳页）。
//!
//! **退化右键单击（无拖框意图）= 循环切换工具**：高亮 → 下划线 →
//! 便签 → 高亮；底部横幅明示当前工具（下一次手势提交时清零——不
//! 永久占屏）。三手势分别落 `/Highlight`、`/Underline`（QuadPoints 面）
//! 与 `/Text`（锚定 Rect 面）——引擎写面同一条命令链，签名保持语义
//! 同锁。
//!
//! ## 已知边界（v1 登记）
//!
//! - **页尺寸不均的档**：uniform_list 等高槽——异形页按首页纵横比
//!   定槽高，真图内接呈现（`ObjectFit::Contain` 缺省），不裁不拉；
//!   异形页上的右键拖框坐标按等比槽近似（拖框落点在异形页上有
//!   letterbox 偏移——与「异形页内接呈现」同源边界，登记待后续轮）；
//! - **页面视图下无文本查找**：Ctrl+F 自动切文本视图（提示横幅说明）
//!   ——跨视图查找/选区属后续轮；
//! - **#421 缩放已接**：FitWidth（默认——v1 适宽行为零变化）/
//!   FitPage / ActualSize / Custom 四态——Ctrl+滚轮（capture 期拦截）
//!   + 键域 Ctrl+0/Ctrl+1/Ctrl+Shift+0/Ctrl+=·Ctrl+-（shell 键分派
//!   前置分支——canvas 态键域同构）+ 右下角百分比 chip（可点开预设
//!   菜单）。横向平移（超宽裁边导航）属后续轮；
//! - 扫描件（无文本层）在页面视图下**正是正确呈现**（位图就是内容）
//!   ——文本视图的「OCR 不支持」提示不再挡路；
//! - **沙箱无 GUI**：右键拖框交互层只有编译面验证（本机 cargo check），
//!   手势体验待真机；写路径逻辑面（坐标变换/命令/flush/落盘/签名
//!   保持）在 `tests/pdf_annotate_e2e.rs` 全量真跑（无 GUI 依赖）。
//!
//! ## 引擎绑定纪律
//!
//! 与 [`crate::editor::pdf_doc`] 同源：libpdfium.so 定位序
//! `MDC_PDFIUM_LIB` → 编译期候选 → 系统；`PdfiumEngine::open` 触发
//! 进程级单绑定。无 libpdfium 环境（CI）下 `PdfPagesView::new` 走
//! `error` 态（面板明示「pdfium 未绑定」）——不崩不白屏。

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, App, ClickEvent, Context, DispatchPhase, FocusHandle, Focusable, ImageSource,
    MouseButton, MouseDownEvent, MouseUpEvent, Pixels, ScrollDelta, ScrollEvent,
    UniformListScrollHandle, Window, px,
};
use image::{Frame, ImageBuffer};
use mdc_pdf::engine::DocumentEngine;
use mdc_pdf::engine_pdfium::PdfiumEngine;
use mdc_pdf::page::PageIndex;
use mdc_pdf::render::{RenderKey, RenderRequest, RenderedPage};
use mdc_pdf::worker::{RenderWorker, WorkerReply};
use smallvec::SmallVec;
use ui::prelude::*;

use crate::canvas_view::WHEEL_LINE_PX;
use crate::pdf_annotate::{AnnotateTool, PdfAnnotateController};

/// 光栅宽量子化步长（px）——缓存键稳定性 vs 重光栅频率的折中。
pub const WIDTH_QUANTUM: u32 = 64;
/// 光栅宽下限（px）——再窄不可读，光栅成本也不降。
pub const WIDTH_MIN: u32 = 256;
/// 光栅宽上限（px）——4K 屏适宽 + 余量；引擎侧 S3-L2 仍有独立钳制。
pub const WIDTH_MAX: u32 = 4096;
/// 页面缓存字节预算（192MB——A4 适宽 BGRA ≈ 1.4MB/页，可容百页级
/// 工作集；超预算 LRU 逐出，回滚再光栅）。
pub const CACHE_BUDGET_BYTES: usize = 192 * 1024 * 1024;
/// #421：缩放域下限（相对实际大小——canvas_view ZOOM_MIN 同款护栏
/// 思想；实际光栅宽另受 WIDTH_MIN 钳制，两道护栏取严者生效）。
pub const PDF_ZOOM_MIN: f32 = 0.25;
/// #421：缩放域上限（相对实际大小）。
pub const PDF_ZOOM_MAX: f32 = 8.0;
/// #421：键盘步进倍率（Ctrl+= / Ctrl+-——canvas_view ZOOM_STEP 同款
/// 全应用手感一致）。
pub const PDF_ZOOM_STEP: f32 = 1.25;
/// #421：滚轮步进倍率（Ctrl+滚轮每「行」×1.1 ≈ issue 步进 10%）。
const PDF_ZOOM_WHEEL_STEP: f32 = 1.1;
/// #421：几何预读失败时 ActualSize 百分比基准兜底（A4 纵向 595pt）。
const PAGE_PT_W_FALLBACK: f32 = 595.0;
/// 看门狗预算（单页光栅超时——与 mdc-pdf worker 文档的 20s 预算同源）。
const RENDER_WATCHDOG: Duration = Duration::from_secs(20);
/// 回执轮询间隔（渲染中 ~60fps 足够；空闲零轮询）。
const POLL_INTERVAL: Duration = Duration::from_millis(16);
/// 页左右留白（px）——页面与视口缘的呼吸。
const PAGE_MARGIN_X: Pixels = px(12.);

/// 预渲染身份键：（0 基页号, 量子化光栅宽）——渲染前不知引擎侧
/// 实际高（页各自纵横比），缓存全键（[`RenderKey`]）待回执落地后
/// 才能组装（高度由引擎报告）。
type PageKey = (u32, u32);

/// #421：页面视图缩放模式（issue 交互面三预设 + 自定义档）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoomMode {
    /// 适应宽度（v1 现状默认——光栅宽 = 适宽，行为零变化）。
    FitWidth,
    /// 适应页面（整页可见：宽 = min(适宽, 视口高/纵横比)）。
    FitPage,
    /// 实际大小（1pt = 1px——百分比换算基准）。
    ActualSize,
    /// 自定义倍率（相对实际大小；Ctrl+滚轮/键盘步进/预设档落此态）。
    Custom(f32),
}

/// #421：缩放模式 → 光栅宽（px）——纯函数（单测锚点；[`PdfPagesView::
/// raster_width`] 的判定核心）。量化 + 域钳制与适宽路径同款护栏。
fn zoom_raster_width(mode: ZoomMode, aspect: f32, page_pt_w: f32, vw: f32, vh: f32) -> u32 {
    let raw = match mode {
        ZoomMode::FitWidth => PdfPagesView::fit_width(vw) as f32,
        // 适页：高受限时按视口高/纵横比，宽受限时不超过适宽。
        ZoomMode::FitPage => (vh / aspect).min(PdfPagesView::fit_width(vw) as f32),
        ZoomMode::ActualSize => page_pt_w,
        ZoomMode::Custom(z) => page_pt_w * z.clamp(PDF_ZOOM_MIN, PDF_ZOOM_MAX),
    };
    quantize_width(raw.max(1.) as u32)
}

/// #421：缩放百分比（相对实际大小）——Custom 档直接报倍率（量子化
/// 只影响光栅缓存键，不折进来——菜单预设 50% 报 50 不报 54）；预设
/// 档从光栅宽派生（适宽随视口变化，百分比如实反映）。纯函数（单测
/// 锚点）。
fn zoom_mode_percent(mode: ZoomMode, page_pt_w: f32, vw: f32, vh: f32) -> u32 {
    if let ZoomMode::Custom(z) = mode {
        return (z * 100.).round().clamp(10., 800.) as u32;
    }
    if page_pt_w <= 0. {
        return 100;
    }
    let w = zoom_raster_width(mode, 1.414, page_pt_w, vw, vh) as f32;
    (w / page_pt_w * 100.).round().clamp(10., 800.) as u32
}

/// #421：缩放预设菜单行（label, 目标模式, 键域提示——空串 = 无键位）。
const ZOOM_PRESETS: &[(&str, ZoomMode, &str)] = &[
    ("适应宽度", ZoomMode::FitWidth, "Ctrl+Shift+0"),
    ("适应页面", ZoomMode::FitPage, "Ctrl+0"),
    ("实际大小", ZoomMode::ActualSize, "Ctrl+1"),
    ("50%", ZoomMode::Custom(0.5), ""),
    ("75%", ZoomMode::Custom(0.75), ""),
    ("150%", ZoomMode::Custom(1.5), ""),
    ("200%", ZoomMode::Custom(2.0), ""),
];

/// PDF 页面位图视图实体（每 pdf 标签一份，由 ShellApp 持有/切换）。
pub struct PdfPagesView {
    /// 源字节（看门狗换代工厂重开引擎的原料——Arc 免拷贝）。
    bytes: Arc<Vec<u8>>,
    /// 总页数（构造期主线程一次性读取；0 = 引擎面失败走 error 态）。
    page_count: usize,
    /// 首页纵横比（高/宽，含旋转归一）——uniform 槽高的基准。
    aspect: f32,
    /// 渲染工人（专用线程 + 看门狗换代；error 态为 None）。
    worker: Option<RenderWorker<PdfiumEngine>>,
    /// 页面缓存（RenderKey 键控——含 annotation_rev；LRU 字节预算）。
    cache: mdc_pdf::cache::PageCache<RenderKey, Arc<gpui::RenderImage>>,
    /// 在途作业：job_id → 缓存键（回执对账 + 失败不再无限重提）。
    pending: HashMap<u64, PageKey>,
    /// 失败页（Done(Err)/Timeout 一次性登记——不无限重试敌意页）。
    failed: HashSet<PageKey>,
    /// 引擎侧实际光栅高（页各自纵横比）：(页, 宽) → 高——RenderKey
    /// 组装素材（渲染前未知，回执落地后记录；同键重复渲染零成本）。
    rendered_heights: HashMap<PageKey, u32>,
    /// #302④：标注写会话（首次右键拖框时懒初始化——open_scan +
    /// 写器门的一次性成本不落在纯阅读路径上）。
    annotate: Option<PdfAnnotateController>,
    /// 标注修订号镜像（控制器 flush 后同步——RenderKey 键控源；
    /// 0 = 无批注会话/从未 flush）。
    annotate_rev: u64,
    /// 右键拖框会话（按下时的页号 + 页内像素起点；None = 非拖框中）。
    drag_start: Option<(u32, f32, f32)>,
    /// #421：缩放模式（FitWidth = v1 现状默认——行为零变化）。
    zoom_mode: ZoomMode,
    /// #421：首页宽（pt）——ActualSize 基准 + 百分比换算（构造期几何
    /// 预读与 aspect 同源捕获；全前 3 页无有效几何时兜底 A4 595pt）。
    page_pt_w: f32,
    /// #421：缩放预设菜单开（右下角百分比 chip 点击弹出——backdrop
    /// 模态；点选项/点空白即关，不再占屏）。
    zoom_menu_open: bool,
    /// #302 验收 1：当前批注工具（三手势——高亮/下划线/便签；退化
    /// 右键单击循环切换，见 [`Self::finish_drag_annotation`]）。
    annotate_tool: AnnotateTool,
    /// 批注路径错误（最近一次——面板明示，不静默丢；用户可重试）。
    annotate_error: Option<String>,
    /// 工具切换提示（最近一次退化单击——底部横幅明示当前工具与切换
    /// 手势；下一次手势提交时清零——不永久占屏）。
    annotate_hint: Option<String>,
    /// 回执轮询循环进行中（防重复起循环）。
    polling: bool,
    /// 虚拟列表滚动句柄（scroll_to_page / 可见区间推算共用）。
    scroll: UniformListScrollHandle,
    /// 焦点域（键盘域占位——v1 无键位面，滚轮/拖滚动条不依赖焦点）。
    focus_handle: FocusHandle,
    /// 视口尺寸（宽, 高）——最近一次消费值（覆盖 canvas 每帧回写
    /// `viewport` Cell，render 期取用；首帧 0 = 未记录，下帧起算）。
    viewport_size: (f32, f32),
    /// 视口尺寸回写通道（覆盖 canvas paint 回调持有同款 Rc——零实体
    /// 更新零重入；不注册命中区，不挡列表滚轮）。
    viewport: Rc<Cell<Option<(f32, f32)>>>,
    /// 引擎面错误（构造期 open 失败——面板明示，不白屏）。
    error: Option<String>,
}

impl PdfPagesView {
    /// 构造：主线程一次性 open 读页数/几何（毫秒级），随即把引擎交
    /// 给看门狗工人在渲染线程重建（工厂持字节快照——换代同源）。
    pub fn new(bytes: Vec<u8>, cx: &mut App) -> Self {
        let bytes = Arc::new(bytes);
        let focus_handle = cx.focus_handle();
        // 几何预读（一次性）：页数 + 首页纵横比 + 首页 pt 宽（前 3 页
        // 取首个有效几何——封面页偶有异形，首页族比单首页稳；#421：
        // pt 宽 = ActualSize/百分比基准）。失败 → error 态面板。
        let (page_count, aspect, page_pt_w) = match PdfiumEngine::open((*bytes).clone()) {
            Ok(engine) => {
                let count = engine.page_count() as usize;
                let geo = (0..count.min(3))
                    .filter_map(|i| engine.page_geometry(PageIndex(i as u32)).ok())
                    .find(|g| g.effective_width() > 0. && g.effective_height() > 0.);
                let (aspect, pt_w) = match geo {
                    Some(g) => (
                        g.effective_height() / g.effective_width(),
                        g.effective_width(),
                    ),
                    None => (1.414, PAGE_PT_W_FALLBACK),
                };
                (count, aspect, pt_w)
            }
            Err(err) => {
                return Self::error_state(
                    bytes,
                    focus_handle,
                    format!("pdfium 引擎打开失败：{err:?}"),
                );
            }
        };
        // 看门狗工人：工厂持字节快照，首代/替代代都在渲染线程内构造
        // （工厂 panic 不炸宿主帧——worker.rs 既有纪律）。
        let factory_bytes = bytes.clone();
        let worker = RenderWorker::watchdog(
            move || {
                PdfiumEngine::open((*factory_bytes).clone())
                    .expect("pdfium 看门狗工厂：字节快照重开失败（几何预读已验证可开）")
            },
            RENDER_WATCHDOG,
        );
        Self {
            bytes,
            page_count,
            aspect,
            page_pt_w,
            zoom_mode: ZoomMode::FitWidth,
            zoom_menu_open: false,
            worker: Some(worker),
            cache: mdc_pdf::cache::PageCache::new(CACHE_BUDGET_BYTES),
            pending: HashMap::new(),
            failed: HashSet::new(),
            rendered_heights: HashMap::new(),
            annotate: None,
            annotate_rev: 0,
            drag_start: None,
            annotate_tool: AnnotateTool::Highlight,
            annotate_error: None,
            annotate_hint: None,
            polling: false,
            scroll: UniformListScrollHandle::new(),
            focus_handle,
            viewport_size: (0., 0.),
            viewport: Rc::new(Cell::new(None)),
            error: None,
        }
    }

    /// error 态构造（引擎绑定/打开失败——面板明示）。
    fn error_state(bytes: Arc<Vec<u8>>, focus_handle: FocusHandle, message: String) -> Self {
        Self {
            bytes,
            page_count: 0,
            aspect: 1.414,
            page_pt_w: PAGE_PT_W_FALLBACK,
            zoom_mode: ZoomMode::FitWidth,
            zoom_menu_open: false,
            worker: None,
            cache: mdc_pdf::cache::PageCache::new(CACHE_BUDGET_BYTES),
            pending: HashMap::new(),
            failed: HashSet::new(),
            rendered_heights: HashMap::new(),
            annotate: None,
            annotate_rev: 0,
            drag_start: None,
            annotate_tool: AnnotateTool::Highlight,
            annotate_error: None,
            annotate_hint: None,
            polling: false,
            scroll: UniformListScrollHandle::new(),
            focus_handle,
            viewport_size: (0., 0.),
            viewport: Rc::new(Cell::new(None)),
            error: Some(message),
        }
    }

    /// 适宽（px）——视口宽 - 页边距；量子化 + 域钳制（纯函数面，
    /// [`quantize_width`] 单测锚点）。
    fn fit_width(viewport_w: f32) -> u32 {
        let w = (viewport_w - 2. * f32::from(PAGE_MARGIN_X)).max(WIDTH_MIN as f32);
        quantize_width(w as u32)
    }

    /// 槽高（px）：适宽 × 首页纵横比（纯函数——单测锚点）。
    fn slot_height(fit_w: u32, aspect: f32) -> f32 {
        fit_w as f32 * aspect
    }

    /// 滚到指定页（0 基——书签跳转锚；越界钳到末页）。
    pub fn scroll_to_page(&self, page: usize) {
        if self.page_count == 0 {
            return;
        }
        let idx = page.min(self.page_count - 1);
        self.scroll.scroll_to_item(idx, gpui::ScrollStrategy::Top);
    }

    // ------------------------------------------------------------------
    // #421：缩放域（状态面——交互面在 render/键分派两处接入）
    // ------------------------------------------------------------------

    /// #421：当前光栅宽（px）——缩放模式解析 + 量化钳制。**三个消费点
    /// （submit_visible 可见区间 / render_page_slot 槽宽 / commit_
    /// annotation DPI）必须同源调用本方法**——缩放后批注坐标正确性的
    /// 关键（issue 验收 3：#304 坐标变换的 DPI 基准随缩放同步）。
    fn raster_width(&self, vw: f32, vh: f32) -> u32 {
        zoom_raster_width(self.zoom_mode, self.aspect, self.page_pt_w, vw, vh)
    }

    /// #421：当前缩放百分比（相对实际大小——chip 显示与菜单基准）。
    fn zoom_percent(&self, vw: f32, vh: f32) -> u32 {
        zoom_mode_percent(self.zoom_mode, self.page_pt_w, vw, vh)
    }

    /// 当前生效倍率（任意模式 → 光栅宽/pt 宽；预设态步进缩放的起算
    /// 基准——从当前生效百分比继续，不从 100% 跳）。
    fn effective_zoom(&self, vw: f32, vh: f32) -> f32 {
        if self.page_pt_w <= 0. {
            return 1.0;
        }
        self.raster_width(vw, vh) as f32 / self.page_pt_w
    }

    /// #421：切换缩放模式（滚动锚保持——内容 offset 按槽高比例缩放，
    /// 缩放前后视口内容位置不跳页；槽高刷新由下一帧 uniform_list 重测
    /// 自然完成）。预设菜单点击亦走此画口（菜单即关）。
    fn set_zoom_mode(&mut self, mode: ZoomMode, cx: &mut Context<Self>) {
        let (vw, vh) = self.viewport_size;
        let old_w = self.raster_width(vw, vh) as f32;
        self.zoom_mode = mode;
        let new_w = self.raster_width(vw, vh) as f32;
        self.rescale_scroll_offset(old_w, new_w);
        self.zoom_menu_open = false;
        cx.notify();
    }

    /// #421：步进缩放（键盘 Ctrl+= / Ctrl+-：×PDF_ZOOM_STEP）。
    fn zoom_step(&mut self, bigger: bool, cx: &mut Context<Self>) {
        let factor = if bigger {
            PDF_ZOOM_STEP
        } else {
            1. / PDF_ZOOM_STEP
        };
        self.zoom_by_factor(factor, cx);
    }

    /// #421：滚轮缩放（每「行」×PDF_ZOOM_WHEEL_STEP ≈ issue 步进 10%；
    /// steps 正 = 放大——canvas_view::zoom_wheel 同款方向语义）。
    fn zoom_wheel(&mut self, steps: f32, cx: &mut Context<Self>) {
        let factor = PDF_ZOOM_WHEEL_STEP.powf(steps);
        self.zoom_by_factor(factor, cx);
    }

    /// 相对当前生效倍率缩放（统一收口：落 Custom 档——预设态起算 =
    /// 当前光栅宽/pt 宽，滚轮/键盘连续步进不跳档）。
    fn zoom_by_factor(&mut self, factor: f32, cx: &mut Context<Self>) {
        let (vw, vh) = self.viewport_size;
        let next = (self.effective_zoom(vw, vh) * factor).clamp(PDF_ZOOM_MIN, PDF_ZOOM_MAX);
        let old_w = self.raster_width(vw, vh) as f32;
        self.zoom_mode = ZoomMode::Custom(next);
        let new_w = self.raster_width(vw, vh) as f32;
        self.rescale_scroll_offset(old_w, new_w);
        cx.notify();
    }

    /// 滚动锚保持：内容坐标 offset ×（新槽高/旧槽高）——首页纵横比
    /// 恒定，槽高与光栅宽同比，页锚即内容锚（issue 验收 4：切换页面/
    /// 缩放后视口内容位置不跳）。首帧视口未记录（0×n=0）与同宽零变
    /// 无操作。
    fn rescale_scroll_offset(&mut self, old_w: f32, new_w: f32) {
        if old_w <= 0. || (new_w - old_w).abs() < 0.5 {
            return;
        }
        let ratio = new_w / old_w;
        let state = self.scroll.0.borrow();
        let offset = state.base_handle.offset();
        let y = f32::from(offset.y).max(0.) * ratio;
        state.base_handle.set_offset(gpui::point(px(0.), px(y)));
    }

    /// #421：预设行是否为当前态（Custom 档比倍率近似——0.5% 内视为
    /// 同档；预设三态精确等值）。
    fn zoom_preset_active(&self, target: ZoomMode) -> bool {
        match (target, self.zoom_mode) {
            (ZoomMode::Custom(a), ZoomMode::Custom(b)) => (a - b).abs() < 0.005,
            (a, b) => a == b,
        }
    }

    /// #421：pdf 页面视图键域（视口缩放命令——canvas 态键域同构先例
    /// editor::dispatch_canvas_key：视口命令在 keymap 表之前分流，消
    /// 费后 shell return 防字号键二次响应）。键位（issue #421）：
    ///
    /// - Ctrl+0：适应页面
    /// - Ctrl+1：实际大小 100%
    /// - Ctrl+Shift+0：适应宽度
    /// - Ctrl+= / Ctrl++：步进放大（shift 兼容——"+" 多为 shift+= 产生）
    /// - Ctrl+-：步进缩小
    ///
    /// 返回 false = 未消费（非本域键——键继续走 shell keymap 表，非
    /// pdf 态行为零变化）。
    pub fn dispatch_zoom_key(
        &mut self,
        key: &str,
        ctrl: bool,
        shift: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if !ctrl {
            return false;
        }
        match (key, shift) {
            ("0", false) => self.set_zoom_mode(ZoomMode::FitPage, cx),
            ("0", true) => self.set_zoom_mode(ZoomMode::FitWidth, cx),
            ("1", false) => self.set_zoom_mode(ZoomMode::ActualSize, cx),
            ("=" | "+", _) => self.zoom_step(true, cx),
            ("-", _) => self.zoom_step(false, cx),
            _ => return false,
        }
        true
    }

    /// render 期派单：按滚动偏移 + 视口高推算可见区间（±1 页过扫描），
    /// 未缓存且未在途且未失败的页逐个 submit。先收上一帧视口尺寸。
    fn submit_visible(&mut self, cx: &mut Context<Self>) {
        if self.worker.is_none() || self.page_count == 0 {
            return;
        }
        // 消费覆盖 canvas 上帧记录的视口尺寸。
        if let Some((w, h)) = self.viewport.get() {
            if w > 0. && h > 0. {
                self.viewport_size = (w, h);
            }
        }
        let (vw, vh) = self.viewport_size;
        if vw <= 0. {
            return; // 首帧视口未记录——覆盖 canvas 本帧落位，下一帧起算。
        }
        // #421：光栅宽单点口径（缩放模式解析——FitWidth 与 v1 同值）。
        let fit_w = self.raster_width(vw, vh);
        let slot_h = Self::slot_height(fit_w, self.aspect).max(1.);
        // 滚动偏移（内容坐标——base_handle.offset）。
        let offset_y = f32::from(self.scroll.0.borrow().base_handle.offset().y).max(0.);
        let first = (((offset_y / slot_h) as i64).saturating_sub(1)).max(0) as u32;
        let visible_rows = (((vh / slot_h).ceil() as i64) + 2).max(1) as u32;
        let last = (first + visible_rows).min(self.page_count as u32);
        let mut submitted = false;
        for page in first..last {
            let key = (page, fit_w);
            if self
                .render_key(&key)
                .is_some_and(|k| self.cache.contains(&k))
                || self.failed.contains(&key)
                || self.pending.values().any(|k| *k == key)
            {
                continue;
            }
            let id = self.worker.as_mut().unwrap().submit(RenderRequest {
                page: PageIndex(key.0),
                width_px: key.1,
            });
            self.pending.insert(id, key);
            submitted = true;
        }
        if submitted && !self.polling {
            self.polling = true;
            Self::spawn_poll_loop(cx);
        }
    }

    /// 预渲染键 → 缓存全键（#305 RenderKey）：高度取引擎侧实测
    /// （`rendered_heights`——首次渲染前无记录 → None = 必 miss，
    /// 渲染回执落地后同键即命中）。rev = 标注修订号（flush 推进）。
    fn render_key(&self, key: &PageKey) -> Option<RenderKey> {
        let height = *self.rendered_heights.get(key)?;
        Some(RenderKey::new(key.0, key.1, height, self.annotate_rev))
    }

    /// 回执轮询循环（16ms 定时 try_recv——待办清空退出；实体释放自然止）。
    fn spawn_poll_loop(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                let mut done = false;
                let _ = this.update(cx, |view, cx| {
                    view.drain_replies(cx);
                    if view.pending.is_empty() {
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

    /// 收回执（Done → 缓存 + notify；Cancelled/Timeout/Err → failed 登记）。
    fn drain_replies(&mut self, cx: &mut Context<Self>) {
        let Some(worker) = self.worker.as_mut() else {
            return;
        };
        while let Some(reply) = worker.try_recv() {
            match reply {
                WorkerReply::Done { id, result } => {
                    let Some(key) = self.pending.remove(&id) else {
                        continue; // 迟到回执（换代竞态）——机械丢弃。
                    };
                    match result {
                        Ok(page) => {
                            if let Some(image) = rendered_page_to_image(&page) {
                                let cost = page.bgra.len();
                                // #305：全键落地（引擎侧实测高 + 当前 rev）
                                // + 高度记录（后续帧组装键的素材）。
                                self.rendered_heights.insert(key, page.height_px);
                                let full =
                                    RenderKey::new(key.0, key.1, page.height_px, self.annotate_rev);
                                let _evicted = self.cache.put(full, Arc::new(image), cost);
                                cx.notify();
                            } else {
                                self.failed.insert(key);
                                cx.notify();
                            }
                        }
                        Err(_) => {
                            self.failed.insert(key);
                            cx.notify();
                        }
                    }
                }
                WorkerReply::Cancelled { id } | WorkerReply::Timeout { id } => {
                    if let Some(key) = self.pending.remove(&id) {
                        self.failed.insert(key);
                        cx.notify();
                    }
                }
            }
        }
    }

    /// 可见页元素（uniform_list 回调——读缓存出位图，未就绪出占位框：
    /// 渲染中 / 失败两态）。#302④：页面槽上叠透明 canvas——paint 期
    /// 注册右键拖框观察器（窗口级 on_mouse_event + 自身 bounds 过滤，
    /// 与 shell 面板拖拽收尾钩同模式；无命中区不挡滚动）。
    fn render_page_slot(&mut self, idx: usize, cx: &App) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let (vw, vh) = self.viewport_size;
        // #421：光栅宽单点口径（首帧视口未记录时各模式均落 WIDTH_MIN
        // ——与 v1 兜底同值；ActualSize/Custom 档不依赖视口，首帧即准）。
        let fit_w = self.raster_width(vw, vh);
        let key = (idx as u32, fit_w);
        let slot_h = Self::slot_height(fit_w, self.aspect);
        let page_no = idx + 1;
        let image = self
            .render_key(&key)
            .and_then(|k| self.cache.get(&k).cloned());
        match image {
            Some(image) => gpui::img(ImageSource::Render(image))
                .id(gpui::ElementId::NamedInteger("pdf-page".into(), idx as u64))
                .w(px(fit_w as f32))
                .h(px(slot_h))
                .flex_none()
                .rounded_sm()
                .shadow_md()
                .into_any_element(),
            None => {
                let failed = self.failed.contains(&key);
                div()
                    .id(gpui::SharedString::from(format!("pdf-page-pending-{idx}")))
                    .w(px(fit_w as f32))
                    .h(px(slot_h))
                    .flex_none()
                    .rounded_sm()
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.panel_background)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        ui::Label::new(if failed {
                            format!("第 {page_no} 页渲染失败")
                        } else {
                            format!("第 {page_no} 页渲染中…")
                        })
                        .size(ui::LabelSize::Small)
                        .color(ui::Color::Muted),
                    )
                    .into_any_element()
            }
        }
    }

    // ------------------------------------------------------------------
    // #302 阶段 2·④：右键拖框批注（三手势）→ 增量写 → 渲染刷新
    // ------------------------------------------------------------------

    /// 右键拖框收尾：页内拖框（跨页丢弃）→ 按当前工具提交完整写路径
    /// → flush 后视图刷新。**退化单击 = 循环切换工具**（高亮→下划线
    /// →便签→高亮——无拖框意图的单击不当零面积批注，改当工具选择；
    /// 底部横幅明示当前工具，下一次手势提交时清零）。错误不静默丢
    /// （面板明示，用户可重试）。
    fn finish_drag_annotation(&mut self, page: u32, x: f32, y: f32, cx: &mut Context<Self>) {
        let Some((drag_page, x0, y0)) = self.drag_start.take() else {
            return; // 非拖框会话（拖框起点不在本页——按下观察器未命中）。
        };
        if drag_page != page {
            return; // 跨页拖框（v1 页内语义——丢弃不猜）。
        }
        if (x - x0).abs() < 3.0 && (y - y0).abs() < 3.0 {
            // 退化单击：无框选意图——循环切换批注工具并明示。
            self.annotate_tool = self.annotate_tool.next();
            self.annotate_hint = Some(format!(
                "批注工具：{}（右键拖框使用；单击继续切换：高亮→下划线→便签）",
                self.annotate_tool.label()
            ));
            cx.notify();
            return;
        }
        match self.commit_annotation(page, x0, y0, x, y) {
            Ok(()) => {
                self.annotate_error = None;
                // 提交成功 → 工具提示使命完成（不永久占屏）。
                self.annotate_hint = None;
            }
            Err(e) => self.annotate_error = Some(e.to_string()),
        }
        cx.notify();
    }

    /// 提交批注（三手势派发）：懒初始化控制器 → #304 坐标变换（屏幕
    /// 拖框→用户空间）→ AnnotateAdd → 增量 flush → [`Self::reload_after_flush`]。
    fn commit_annotation(
        &mut self,
        page: u32,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
    ) -> Result<(), crate::pdf_annotate::AnnotateError> {
        if self.annotate.is_none() {
            let controller = PdfAnnotateController::open((*self.bytes).clone())?;
            self.annotate = Some(controller);
        }
        // #421：光栅宽先行（controller 可变借用前算好——DPI 基准与
        // submit/render 同源）。
        let (vw, vh) = self.viewport_size;
        let fit_w = self.raster_width(vw, vh);
        let controller = self.annotate.as_mut().expect("刚初始化");
        // DPI：位图 px / 显示域 pt × 72（等比槽近似——异形页 letterbox
        // 边界见模块文档 v1 登记）。#421：基准 = 当前光栅宽（缩放后拖
        // 框坐标与位图像素 1:1 同域——验收 3 坐标正确性的关键；视口
        // 未记录时各模式均落 WIDTH_MIN 兜底）。
        let transform = controller.page_transform(page)?;
        let display_w = transform.display_size().width.max(1.0);
        let dpi = fit_w as f32 / display_w * 72.0;
        // 三手势派发（issue #302 验收 1：高亮/下划线/便签）。
        match self.annotate_tool {
            AnnotateTool::Highlight => {
                controller.add_highlight_from_screen(
                    page,
                    x0,
                    y0,
                    x1,
                    y1,
                    dpi,
                    "mang-shell 用户",
                )?;
            }
            AnnotateTool::Underline => {
                controller.add_underline_from_screen(
                    page,
                    x0,
                    y0,
                    x1,
                    y1,
                    dpi,
                    "mang-shell 用户",
                )?;
            }
            AnnotateTool::Note => {
                controller.add_note_from_screen(page, x0, y0, x1, y1, dpi, "mang-shell 用户")?;
            }
        }
        let (bytes, _stats) = controller.flush()?;
        self.reload_after_flush(bytes);
        Ok(())
    }

    /// flush 后刷新：换字节快照 + 重建渲染工人（引擎重开见追加修订）
    /// + 旧 rev 缓存即刻回收（#305）+ rev 镜像同步（RenderKey 键控）。
    /// 几何未变（批注不改页树）——`rendered_heights` 保留，同键新 rev
    /// 重光栅后即命中。
    fn reload_after_flush(&mut self, bytes: Vec<u8>) {
        self.bytes = Arc::new(bytes);
        self.annotate_rev = self
            .annotate
            .as_ref()
            .map(|c| c.annotation_rev())
            .unwrap_or(0);
        // 在途/失败清零：旧工人即将退役，其回执不再落地（换代竞态的
        // 机械丢弃路径）；新工人重提时这些键不在位即会重派。
        self.pending.clear();
        self.failed.clear();
        // #305：旧 rev 条目即刻回收（不等 LRU 驱逐——预算即时归位）。
        let _evicted = self.cache.invalidate_stale_revisions(self.annotate_rev);
        // 重建工人：工厂持新字节快照（原字节前缀 + 追加修订——增量
        // 产物结构有效，与打开路径同构）。
        if self.worker.is_some() {
            let factory_bytes = self.bytes.clone();
            self.worker = Some(RenderWorker::watchdog(
                move || {
                    PdfiumEngine::open((*factory_bytes).clone())
                        .expect("pdfium 看门狗工厂：批注后字节快照重开失败（增量产物结构有效）")
                },
                RENDER_WATCHDOG,
            ));
        }
    }
}

impl Focusable for PdfPagesView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PdfPagesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 派单（render 期直接改 self——uniform_list 回调期只读，无重入面）。
        self.submit_visible(cx);
        let colors = cx.theme().colors().clone();
        // error 态：明示面板（不白屏）。
        if self.error.is_some() || self.worker.is_none() {
            let message = self.error.clone().unwrap_or_else(|| "pdfium 未绑定".into());
            return v_flex()
                .id("pdf-pages-error")
                .size_full()
                .items_center()
                .justify_center()
                .bg(colors.background)
                .child(ui::Label::new(message).color(ui::Color::Warning))
                .child(
                    ui::Label::new("页面渲染不可用——文本视图仍可查看（状态栏切换）")
                        .size(ui::LabelSize::Small)
                        .color(ui::Color::Muted),
                )
                .into_any_element();
        }
        let entity = cx.entity().downgrade();
        let viewport_cell = self.viewport.clone();
        let zoom_entity = entity.clone();
        let page_count = self.page_count;
        // #421：缩放百分比（首帧视口未记录时用下限兜底——与 v1 适宽
        // hint 同款下限口径，不闪屏错值）。
        let zoom_pct = self.zoom_percent(
            self.viewport_size.0.max(320.),
            self.viewport_size.1.max(240.),
        );
        let zoom_menu_open = self.zoom_menu_open;
        let annotate_error = self.annotate_error.clone();
        let annotate_hint = self.annotate_hint.clone();
        let list = gpui::uniform_list(
            "pdf-pages",
            page_count,
            move |visible: std::ops::Range<usize>, _window: &mut Window, cx: &mut App| {
                visible
                    .clone()
                    .filter_map(|idx| {
                        let slot = entity
                            .update(cx, |view, cx| view.render_page_slot(idx, cx))
                            .ok()?;
                        Some(annotate_overlay(slot, idx, entity.clone()))
                    })
                    .collect::<Vec<AnyElement>>()
            },
        )
        .track_scroll(&self.scroll)
        .h_full();
        // #302④：批注路径错误横幅（最近一次——不静默丢，用户可重试）。
        let error_banner = annotate_error.map(|message| {
            div()
                .id("pdf-annotate-error")
                .absolute()
                .top_4()
                .left_12()
                .right_12()
                .opacity(0.95)
                .rounded_md()
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_background)
                .px_3()
                .py_2()
                .child(
                    ui::Label::new(format!("批注失败：{message}"))
                        .size(ui::LabelSize::Small)
                        .color(ui::Color::Warning),
                )
        });
        // #302 验收 1：工具提示横幅（退化右键单击后——当前工具明示；
        // 底部与错误横幅分位，提交成功即清零）。
        let hint_banner = annotate_hint.map(|message| {
            div()
                .id("pdf-annotate-tool-hint")
                .absolute()
                .bottom_4()
                .left_12()
                .right_12()
                .opacity(0.95)
                .rounded_md()
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_background)
                .px_3()
                .py_2()
                .child(
                    ui::Label::new(message)
                        .size(ui::LabelSize::Small)
                        .color(ui::Color::Muted),
                )
        });
        // #421：缩放预设菜单（backdrop 模态——透明全幅捕获左键关菜单；
        // 菜单面板与 chip 在 backdrop 之后入树 = 盖在其上，选项/chip 仍可
        // 点；右键不拦截——批注拖框窗口级监听不受影响）。
        let hover = colors.element_hover;
        let zoom_menu = zoom_menu_open.then(|| {
            let rows: Vec<AnyElement> =
                ZOOM_PRESETS
                    .iter()
                    .map(|(label, target, shortcut)| {
                        let target = *target;
                        let active = self.zoom_preset_active(target);
                        let row_hover = hover;
                        let label = *label;
                        let shortcut = *shortcut;
                        div()
                            .id(gpui::SharedString::from(format!("pdf-zoom-preset-{label}")))
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
                            .child(ui::Label::new(label).size(ui::LabelSize::Small).color(
                                if active {
                                    ui::Color::Default
                                } else {
                                    ui::Color::Muted
                                },
                            ))
                            .when(!shortcut.is_empty(), |el| {
                                el.child(
                                    ui::Label::new(shortcut)
                                        .size(ui::LabelSize::XSmall)
                                        .color(ui::Color::Muted),
                                )
                            })
                            .into_any_element()
                    })
                    .collect();
            v_flex()
                // backdrop：透明全幅左键捕获（点空白关菜单——chip 与
                // 菜单行盖在其上，点击不落此）。
                .child(
                    div()
                        .id("pdf-zoom-menu-backdrop")
                        .absolute()
                        .size_full()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _ev: &MouseDownEvent, _window, cx| {
                                this.zoom_menu_open = false;
                                cx.notify();
                            }),
                        ),
                )
                // 菜单面板：chip 上方弹出（bottom_16 = chip 高度让位）。
                .child(
                    v_flex()
                        .id("pdf-zoom-menu")
                        .absolute()
                        .bottom_16()
                        .right_4()
                        .w(px(176.))
                        .rounded_md()
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.panel_background)
                        .opacity(0.97)
                        .py_1()
                        .gap_0()
                        .children(rows),
                )
                .into_any_element()
        });
        // #421：缩放控制条（右下角——[−] 百分比 [+ ]；百分比点击弹/
        // 收预设菜单）。
        let zoom_chip = div()
            .id("pdf-zoom-chip")
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
                    .id("pdf-zoom-out")
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
                    .id("pdf-zoom-pct")
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
                    .id("pdf-zoom-in")
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
            );
        div()
            .id("pdf-pages-root")
            .relative()
            .size_full()
            .min_w_0()
            .bg(colors.background)
            .flex()
            .flex_col()
            .items_center()
            .child(list)
            // #302④：批注错误横幅（在位时叠加顶部）。
            .children(error_banner)
            // #302 验收 1：工具提示横幅（在位时叠加底部）。
            .children(hint_banner)
            // #421：缩放预设菜单（backdrop + 面板——在位时叠加；菜单
            // 开时盖住列表左键（点空白即关），chip/菜单行在其上仍可点）。
            .children(zoom_menu)
            // #421：缩放控制条（常驻右下角）。
            .child(zoom_chip)
            // 覆盖 canvas：透明、绝对铺满——paint 期回写视口尺寸（Cell
            // 共享，零实体更新零重入；不注册命中区，不挡列表滚轮）+
            // #421 Ctrl+滚轮 capture 期拦截（缩放——止冒泡防列表滚动）。
            .child(
                div()
                    .id("pdf-viewport-probe")
                    .absolute()
                    .size_full()
                    .child(gpui::canvas(
                        move |bounds, _window, _cx| bounds,
                        move |bounds: gpui::Bounds<gpui::Pixels>,
                              _t,
                              window: &mut Window,
                              _cx: &mut App| {
                            viewport_cell.set(Some((
                                f32::from(bounds.size.width),
                                f32::from(bounds.size.height),
                            )));
                            // #421：Ctrl+滚轮缩放——capture 期窗口级拦截
                            //（annotate_overlay paint 注册先例同款；capture
                            // 先于 bubble = 先于 uniform_list 滚动消费，
                            // stop_propagation 后列表不随缩放滚动）。横向
                            // 分量占优（触控板双指横扫）不拦截——让事件
                            // 冒泡照常滚动。修饰键 control||platform 归一
                            //（与 shell 键分派同口径）。
                            window.on_mouse_event(
                                move |event: &ScrollEvent,
                                      phase: DispatchPhase,
                                      _window: &mut Window,
                                      cx: &mut App| {
                                    if phase != DispatchPhase::Capture {
                                        return;
                                    }
                                    if !(event.modifiers.control || event.modifiers.platform) {
                                        return;
                                    }
                                    if !bounds.contains(&event.position) {
                                        return;
                                    }
                                    let (dx, dy) = match event.delta {
                                        ScrollDelta::Pixels(p) => (p.x.as_f32(), p.y.as_f32()),
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
                    )),
            )
            .into_any_element()
    }
}

/// 光栅产物 → gpui 图像（BGRA 直通——`RenderedPage::bgra` 与
/// `RenderImage` 同为 BGRA 字节序，无通道交换；canvas_view 的 RGBA
/// 面才需要换 R↔B）。尺寸乘积不一致（防御）→ None 走 failed 面。
fn rendered_page_to_image(page: &RenderedPage) -> Option<gpui::RenderImage> {
    let expected = (page.width_px as usize) * (page.height_px as usize) * 4;
    if page.bgra.len() != expected || page.width_px == 0 || page.height_px == 0 {
        return None;
    }
    let buffer = ImageBuffer::from_raw(page.width_px, page.height_px, page.bgra.clone())?;
    Some(gpui::RenderImage::new(SmallVec::from_elem(
        Frame::new(buffer),
        1,
    )))
}

// ---------------------------------------------------------------------------
// #302④：右键拖框观察器（页槽叠加透明 canvas——paint 期窗口级注册）
// ---------------------------------------------------------------------------

/// 页槽包装：叠加透明 canvas，paint 期注册右键按下/松开观察器
/// （窗口级 `on_mouse_event` + 自身 bounds 过滤——与 shell 面板拖拽
/// 收尾钩同模式：每帧重注册，事件只落在自己 bounds 内才动作；
/// 无命中区不挡列表滚轮，右键不占左键交互）。
fn annotate_overlay(
    slot: AnyElement,
    idx: usize,
    entity: gpui::WeakEntity<PdfPagesView>,
) -> AnyElement {
    let page = idx as u32;
    div()
        .id(gpui::SharedString::from(format!("pdf-annot-overlay-{idx}")))
        .relative()
        .flex_none()
        .child(slot)
        .child(
            div()
                .id(gpui::SharedString::from(format!("pdf-annot-canvas-{idx}")))
                .absolute()
                .size_full()
                .child(gpui::canvas(
                    move |bounds, _window, _cx| bounds,
                    move |bounds: gpui::Bounds<gpui::Pixels>,
                          _t,
                          window: &mut Window,
                          _cx: &mut App| {
                        // 右键按下：记录页内像素起点。
                        let ent_down = entity.clone();
                        window.on_mouse_event(
                            move |event: &MouseDownEvent,
                                  phase: DispatchPhase,
                                  _w: &mut Window,
                                  cx: &mut App| {
                                if phase != DispatchPhase::Bubble
                                    || event.button != MouseButton::Right
                                {
                                    return;
                                }
                                let Some((x, y)) = local_point(event.position, bounds) else {
                                    return;
                                };
                                let _ = ent_down.update(cx, |view, _cx| {
                                    view.drag_start = Some((page, x, y));
                                });
                            },
                        );
                        // 右键松开：拖框收尾（跨页/退化单击在收尾面丢弃）。
                        let ent_up = entity.clone();
                        window.on_mouse_event(
                            move |event: &MouseUpEvent,
                                  phase: DispatchPhase,
                                  _w: &mut Window,
                                  cx: &mut App| {
                                if phase != DispatchPhase::Bubble
                                    || event.button != MouseButton::Right
                                {
                                    return;
                                }
                                let Some((x, y)) = local_point(event.position, bounds) else {
                                    return;
                                };
                                let _ = ent_up.update(cx, |view, cx| {
                                    view.finish_drag_annotation(page, x, y, cx);
                                });
                            },
                        );
                    },
                )),
        )
        .into_any_element()
}

/// 窗口坐标 → 元素内局部像素（bounds 外 → None）。
fn local_point(position: gpui::Point<Pixels>, bounds: gpui::Bounds<Pixels>) -> Option<(f32, f32)> {
    let in_x = position.x >= bounds.origin.x && position.x <= bounds.origin.x + bounds.size.width;
    let in_y = position.y >= bounds.origin.y && position.y <= bounds.origin.y + bounds.size.height;
    (in_x && in_y).then_some((
        f32::from(position.x - bounds.origin.x),
        f32::from(position.y - bounds.origin.y),
    ))
}

/// 光栅宽量子化 + 域钳制（纯函数——单测锚点）。
pub fn quantize_width(w: u32) -> u32 {
    let clamped = w.clamp(WIDTH_MIN, WIDTH_MAX);
    (clamped.div_ceil(WIDTH_QUANTUM) * WIDTH_QUANTUM).min(WIDTH_MAX)
}

// ─── 纯函数面单测（零引擎依赖——CI 无 libpdfium 同样真跑） ──────────

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!((PdfPagesView::slot_height(640, 1.4142) - 905.088).abs() < 0.01);
    }

    #[test]
    fn fit_width_subtracts_margin_and_quantizes() {
        // 1024 视口 - 24 边距 = 1000 → 量子化 1024。
        assert_eq!(PdfPagesView::fit_width(1024.), 1024);
        // 极窄视口 → 下限。
        assert_eq!(PdfPagesView::fit_width(10.), WIDTH_MIN);
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
            zoom_mode_percent(ZoomMode::Custom(0.5), 595., 1024., 768.),
            50
        );
        assert_eq!(
            zoom_mode_percent(ZoomMode::Custom(1.0), 595., 1024., 768.),
            100
        );
        assert_eq!(
            zoom_mode_percent(ZoomMode::Custom(2.0), 595., 1024., 768.),
            200
        );
        // ActualSize：光栅 640 / 595 = 107.6% → 108%（量化折入——如
        // 实报当前生效光栅比例）。
        assert_eq!(
            zoom_mode_percent(ZoomMode::ActualSize, 595., 1024., 768.),
            108
        );
        // FitWidth：适宽 1024 / 595 = 172.1% → 172%。
        assert_eq!(
            zoom_mode_percent(ZoomMode::FitWidth, 595., 1024., 768.),
            172
        );
        // pt 宽无效（兜底路径防 0 除）→ 100。
        assert_eq!(zoom_mode_percent(ZoomMode::FitWidth, 0., 1024., 768.), 100);
    }

    /// 缩放域护栏常量自洽（PDF_ZOOM_MIN/MAX 与 issue 交互面一致）。
    #[test]
    fn zoom_domain_constants() {
        assert!(PDF_ZOOM_MIN < 1.0 && PDF_ZOOM_MAX > 1.0);
        assert!((PDF_ZOOM_WHEEL_STEP - 1.1).abs() < 1e-6, "issue 步进 10%");
        assert!((PDF_ZOOM_STEP - 1.25).abs() < 1e-6, "canvas ZOOM_STEP 同款");
    }
}
