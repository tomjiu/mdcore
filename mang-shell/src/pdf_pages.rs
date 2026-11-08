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
//! 视图」钮；两视图各自有查找面——页面视图内查找 #422 已接，文本视图
//! 沿用 Editor 查找条）：
//!
//! ```text
//! PdfPagesView（本模块——页面位图，主显示面）
//!      │  engine-pdfium 光栅化（RenderWorker 专用线程 + 看门狗换代）
//!      │  PageCache（RenderKey 键控 → RenderImage，LRU 字节预算）
//!      │  右键拖框批注：高亮/下划线/便签三手势（#302④ 验收 1：
//!      │    PdfAnnotateController 增量写面；退化单击 = 工具切换）
//!      │  页面视图内查找（#422）：positioned_chars 文本层索引线程 +
//!      │    黄色高亮叠加 + Enter/Shift+Enter 导航 + 「n/m」计数
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
//!   #422 查找高亮每页独立取几何换算（拉伸页按轴独立缩放，同源边界）；
//! - **页面视图内查找（#422）**：Ctrl+F 不切视图直接开页内查找条——
//!   文本层后台线程索引 + 页上黄色高亮 + Enter/Shift+Enter（Ctrl+G）
//!   导航 + 「n/m」计数；扫描件无文本层 → 如实「无匹配」（不虚装）；
//!   无 libpdfium 环境下页视图本就 error 态（面板明示），查找不另设面；
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
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use gpui::{
    AnyElement, App, ClickEvent, Context, DispatchPhase, FocusHandle, Focusable, ImageSource,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, ScrollDelta, ScrollEvent,
    StyledText, UniformListScrollHandle, Window, px,
};
use image::{Frame, ImageBuffer};
use mdc_pdf::engine::DocumentEngine;
use mdc_pdf::engine_pdfium::PdfiumEngine;
use mdc_pdf::page::PageIndex;
use mdc_pdf::render::{RenderKey, RenderRequest, RenderedPage};
use mdc_pdf::table::PositionedChar;
use mdc_pdf::worker::{RenderWorker, WorkerReply};
use mdc_pdf::writer::IncrementalSave;
use smallvec::SmallVec;
use ui::Tooltip;
use ui::prelude::*;

use crate::canvas_view::WHEEL_LINE_PX;
use crate::editor::shape_text_width;
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
    /// #386：有未落盘批注（首次 [`Self::commit_annotation`] 成功后
    /// 置位，[`Self::save_annotations`] 写盘成功清零）。Shell 据此
    /// 驱动关闭确认与标题栏脏标。
    dirty: bool,
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
    /// #422：页内查找态（None = 查找条未开；开条期间常驻——层索引
    /// 跨查询复用，关条丢弃）。
    search: Option<PdfSearch>,
    /// #422：查找输入域焦点（查找条 query 行 track_focus）。
    search_focus: FocusHandle,
    /// #422：索引线程回流通道（open_search 换新；关闭即 drop →
    /// 发送端 send 失败线程自然退出）。
    search_rx: Option<Receiver<SearchIndexMsg>>,
    /// #422：索引回流轮询循环在跑（防重复起循环；search 关闭即止）。
    search_polling: bool,
}

// ---------------------------------------------------------------------------
// #422：页面视图内文本查找——文本层索引 + 匹配 + 高亮 + 导航
// ---------------------------------------------------------------------------

/// 查找命中（一处）：页 + 行矩形族（用户空间 pt——命中跨行时一行
/// 一个 quad，与 pdf 高亮批注的 QuadPoints 同语义）。
#[derive(Clone, Debug, PartialEq)]
struct SearchMatch {
    /// 0 基页号。
    page: u32,
    /// 行矩形（阅读序；用户空间 pt，`top > bottom`——原点左下，
    /// 与 [`PositionedChar`] 同向）。
    quads: SmallVec<[SearchQuad; 2]>,
}

/// 行矩形（用户空间 pt）。
#[derive(Clone, Copy, Debug, PartialEq)]
struct SearchQuad {
    left: f32,
    bottom: f32,
    right: f32,
    top: f32,
}

/// 页文本层（索引线程产物——`positioned_chars` 一次提取，跨查询
/// 复用；`lower` 是匹配面，`chars` 是坐标面，两面对同一字符序）。
#[derive(Debug)]
struct PageTextLayer {
    /// 字符流（阅读序）——与 `lower` 的字符一一对应。
    chars: Vec<PositionedChar>,
    /// 小写拼接文本（匹配面——大小写不敏感查询的单一事实源）。
    lower: String,
    /// 页几何（pt；高亮换算的轴缩放基准——用户空间 → 槽像素）。
    width_pt: f32,
    height_pt: f32,
}

/// 索引线程 → 视图回流消息（通道关闭 = 查找条已关，线程自然退出）。
enum SearchIndexMsg {
    /// 一页文本层落地（页序推进）。
    Layer(Box<PageTextLayer>),
    /// 索引完成（全部页处理尽；`failed_pages` = 提取失败页数——
    /// 引擎打不开 = 全部页计入，如实呈现在计数条脚注）。
    Done { failed_pages: u32 },
}

/// 查找状态机（[`PdfPagesView::search`] 的载荷）。
struct PdfSearch {
    /// 查询词（原样保留——匹配在 `lower` 面做）。
    query: String,
    /// 光标（char 计——输入行编辑位）。
    cursor: usize,
    /// 总页数（计数条「索引中 x/n」的分母）。
    total: u32,
    /// 已建索引的页文本层（0 基页号键控；索引进度 = len）。
    layers: BTreeMap<u32, Arc<PageTextLayer>>,
    /// 索引完成旗（完成后的计数条不再显「索引中」）。
    indexed_done: bool,
    /// 引擎面提取失败的页数（如实计数——不计匹配面；0 = 全部健康）。
    failed_pages: u32,
    /// 当前查询的命中（页升序 + 页内阅读序；每层落地/查询变更重算）。
    matches: Vec<SearchMatch>,
    /// 当前命中位（0 基；导航环回）。
    current: usize,
}

impl PdfSearch {
    fn new(total: u32) -> Self {
        Self {
            query: String::new(),
            cursor: 0,
            total,
            layers: BTreeMap::new(),
            indexed_done: false,
            failed_pages: 0,
            matches: Vec::new(),
            current: 0,
        }
    }

    /// 计数条文案（「n/m」主面 + 索引进度/无匹配副面）。
    fn match_label(&self) -> String {
        if self.query.chars().all(|c| c.is_whitespace()) || self.query.is_empty() {
            return "输入关键词".into();
        }
        let progress = if self.indexed_done {
            String::new()
        } else {
            format!("（索引中 {}/{} 页）", self.layers.len(), self.total)
        };
        if self.matches.is_empty() {
            if self.indexed_done {
                format!("无匹配{}", self.zero_note())
            } else {
                format!("…{progress}")
            }
        } else {
            format!("{}/{} 处{progress}", self.current + 1, self.matches.len())
        }
    }

    /// 「无匹配」的诚实脚注：有提取失败页 = 结论不完整（非「全文无」）。
    fn zero_note(&self) -> String {
        if self.failed_pages > 0 {
            format!("（{} 页文本层不可读，未计入）", self.failed_pages)
        } else {
            String::new()
        }
    }
}

/// 一处命中的行矩形分组（纯函数——单测锚点）：连续字符按「垂直中心
/// 距 ≤ 两字符高度均值 × 30%」归行；同 quad 取包围盒（左右取端点
/// 极值——行内字符 x 序与阅读序一致，min/max 双保险兼容 RTL 字符）。
fn match_quads(chars: &[PositionedChar]) -> SmallVec<[SearchQuad; 2]> {
    let mut quads: SmallVec<[SearchQuad; 2]> = SmallVec::new();
    let mut cur: Option<SearchQuad> = None;
    for ch in chars {
        let quad = SearchQuad {
            left: ch.left,
            bottom: ch.bottom,
            right: ch.right,
            top: ch.top,
        };
        let center_y = (ch.top + ch.bottom) / 2.;
        match cur.as_mut() {
            None => cur = Some(quad),
            Some(c) => {
                let prev_center = (c.top + c.bottom) / 2.;
                let tolerance = (((c.top - c.bottom) + (ch.top - ch.bottom)).max(1.) / 2.) * 0.6;
                if (center_y - prev_center).abs() <= tolerance {
                    c.left = c.left.min(quad.left);
                    c.right = c.right.max(quad.right);
                    c.bottom = c.bottom.min(quad.bottom);
                    c.top = c.top.max(quad.top);
                } else {
                    quads.push(*c);
                    cur = Some(quad);
                }
            }
        }
    }
    if let Some(c) = cur {
        quads.push(c);
    }
    quads
}

/// 层文本上的全量匹配（纯函数——大小写不敏感；返回每处命中的起始
/// char 下标 + 长度，页内阅读序）。`layer.lower` 与 `layer.chars`
/// 的字符序一一对应（构造不变量），byte 偏移经 char 计数换算。
fn find_in_layer(layer: &PageTextLayer, query_lower: &str) -> Vec<(usize, usize)> {
    if query_lower.is_empty() {
        return Vec::new();
    }
    layer
        .lower
        .match_indices(query_lower)
        .map(|(byte_off, slice)| {
            let head = layer.lower[..byte_off].chars().count();
            (head, slice.chars().count())
        })
        .collect()
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
                    cx.focus_handle(),
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
            dirty: false,
            polling: false,
            scroll: UniformListScrollHandle::new(),
            focus_handle,
            viewport_size: (0., 0.),
            viewport: Rc::new(Cell::new(None)),
            error: None,
            search: None,
            search_focus: cx.focus_handle(),
            search_rx: None,
            search_polling: false,
        }
    }

    /// error 态构造（引擎绑定/打开失败——面板明示）。
    fn error_state(
        bytes: Arc<Vec<u8>>,
        focus_handle: FocusHandle,
        search_focus: FocusHandle,
        message: String,
    ) -> Self {
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
            dirty: false,
            polling: false,
            scroll: UniformListScrollHandle::new(),
            focus_handle,
            viewport_size: (0., 0.),
            viewport: Rc::new(Cell::new(None)),
            error: Some(message),
            search: None,
            search_focus,
            search_rx: None,
            search_polling: false,
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
        // #422：本页查找高亮层（页级换算——层几何 → 槽像素，轴独立
        // 缩放；当前命中橙色加描边，其余黄色）。
        let highlights = self.search_highlights(idx as u32, fit_w as f32, slot_h);
        match image {
            Some(image) => {
                let img = gpui::img(ImageSource::Render(image))
                    .id(gpui::ElementId::NamedInteger("pdf-page".into(), idx as u64))
                    .w(px(fit_w as f32))
                    .h(px(slot_h))
                    .flex_none()
                    .rounded_sm()
                    .shadow_md();
                if highlights.is_empty() {
                    return img.into_any_element();
                }
                div()
                    .id(gpui::SharedString::from(format!("pdf-page-hit-{idx}")))
                    .relative()
                    .flex_none()
                    .child(img)
                    .children(highlights)
                    .into_any_element()
            }
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

    /// #422：本页查找高亮元素族（用户空间 pt → 槽像素；空 = 本页无
    /// 命中或查找未开）。当前命中 = 橙色 + 描边；其余 = 黄色。
    fn search_highlights(&self, page: u32, fit_w: f32, slot_h: f32) -> Vec<AnyElement> {
        let Some(search) = self.search.as_ref() else {
            return Vec::new();
        };
        if search.matches.is_empty() {
            return Vec::new();
        }
        let layer = search.layers.get(&page);
        let (sx, sy) = layer
            .map(|l| (fit_w / l.width_pt.max(1.), slot_h / l.height_pt.max(1.)))
            .unwrap_or((1., 1.));
        let mut out = Vec::new();
        for (mi, m) in search.matches.iter().enumerate() {
            if m.page != page {
                continue;
            }
            let is_current = mi == search.current;
            for (qi, q) in m.quads.iter().enumerate() {
                let x = q.left * sx;
                let y = slot_h - q.top * sy;
                let w = (q.right - q.left) * sx;
                let h = (q.top - q.bottom) * sy;
                let mut el = div()
                    .id(gpui::SharedString::from(format!(
                        "pdf-hit-{page}-{mi}-{qi}"
                    )))
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(w))
                    .h(px(h.max(2.)))
                    .when(is_current, |el| {
                        el.border_1().border_color(gpui::rgba(0xc2410cff))
                    });
                el = if is_current {
                    el.bg(gpui::rgba(0xf59e0bff))
                } else {
                    el.bg(gpui::rgba(0xfde047d9))
                };
                out.push(el.into_any_element());
            }
        }
        out
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
    /// #386：`pub(crate)`——e2e 测试从手势提交入口驱动（与右键拖框
    /// 收尾 [`Self::finish_drag_annotation`] 同一调用面）。
    pub(crate) fn commit_annotation(
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
        // #386：内存已刷新但盘上未落——脏标记（Shell 据此驱动关闭
        // 确认与保存分流；save_annotations 清零）。
        self.dirty = true;
        Ok(())
    }

    // ------------------------------------------------------------------
    // #422：页面视图内文本查找——开条 / 索引 / 匹配 / 导航
    // ------------------------------------------------------------------

    /// 开查找条（Shell 派发的 Ctrl+F / 菜单「查找」在页面视图下的
    /// 落点）：首次开起后台索引线程（全档逐页 `positioned_chars`），
    /// 焦点入查询行。重复调用幂等（已开只聚焦）。
    pub fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.error.is_some() || self.worker.is_none() {
            // error 态视图本就不渲染页——调用方（Shell）保底分流，
            // 此处防御性 no-op。
            return;
        }
        if self.search.is_none() {
            self.search = Some(PdfSearch::new(self.page_count as u32));
            self.spawn_search_index(cx);
        }
        window.focus(&self.search_focus, cx);
        cx.notify();
    }

    /// #422：查找条开态观测（Shell 接线测试/诊断面——状态机私有，
    /// 开闭经行为面观察）。
    pub fn search_open(&self) -> bool {
        self.search.is_some()
    }

    /// 关查找条（Esc / × 钮）：态与层索引一并丢弃（下次开条重建——
    /// 查找是短命会话，不为常驻内存留 500 页字符面）；焦点回视图。
    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search = None;
        self.search_rx = None; // 通道 drop → 索引线程 send 失败自然退出。
        self.search_polling = false;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// 起索引线程 + 16ms 轮询循环（RenderWorker 同款纪律：专用线程
    /// 一次开引擎逐页提取，通道回流，前台定时 drain——UI 零阻塞面）。
    fn spawn_search_index(&mut self, cx: &mut Context<Self>) {
        let (tx, rx) = std::sync::mpsc::channel::<SearchIndexMsg>();
        let bytes = self.bytes.clone();
        let page_count = self.page_count as u32;
        self.search_rx = Some(rx);
        std::thread::spawn(move || index_document(bytes, page_count, tx));
        self.search_polling = true;
        Self::spawn_search_poll_loop(cx);
    }

    /// 索引回流轮询循环（16ms 定时 drain；Done 消费当拍即止——
    /// 通道 FIFO，Done 后无层可来；查找条提前关即 search None 止）。
    fn spawn_search_poll_loop(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                let mut stop = false;
                let _ = this.update(cx, |view, cx| {
                    view.drain_search_index(cx);
                    if view.search.is_none() || view.search.as_ref().is_some_and(|s| s.indexed_done)
                    {
                        view.search_polling = false;
                        stop = true;
                    }
                });
                if stop {
                    break;
                }
            }
        })
        .detach();
    }

    /// 收索引回流：层落地 → 匹配重算 + notify（Done 落完成旗）。
    fn drain_search_index(&mut self, cx: &mut Context<Self>) {
        let Some(rx) = self.search_rx.take() else {
            return;
        };
        let mut touched = false;
        let mut done_failed: Option<u32> = None;
        // 借出循环 drain（self.search 可变借用与 rx 解耦——rx 先取）。
        while let Ok(msg) = rx.try_recv() {
            match msg {
                SearchIndexMsg::Layer(layer) => {
                    if let Some(search) = self.search.as_mut() {
                        let page = search.layers.len() as u32;
                        search.layers.insert(page, Arc::new(*layer));
                        touched = true;
                    }
                }
                SearchIndexMsg::Done { failed_pages } => {
                    done_failed = Some(failed_pages);
                }
            }
        }
        self.search_rx = Some(rx);
        if let Some(failed) = done_failed {
            if let Some(search) = self.search.as_mut() {
                search.indexed_done = true;
                search.failed_pages = failed;
            }
            touched = true;
        }
        if touched {
            self.rematch();
            cx.notify();
        }
    }

    /// 匹配重算（层落地 / 查询变更 / 选项变更共用）：页升序逐层
    /// `find_in_layer` → 命中字符段 `match_quads` 归行。纯内存面
    /// （500 页 × 千字符级 ≈ 毫秒），键入即重算不卡帧。
    fn rematch(&mut self) {
        let Some(search) = self.search.as_mut() else {
            return;
        };
        let query_lower = search.query.to_lowercase();
        search.matches.clear();
        if !query_lower.is_empty() {
            for (page, layer) in &search.layers {
                for (head, len) in find_in_layer(layer, &query_lower) {
                    let chars = layer.chars.get(head..head + len).unwrap_or(&[]);
                    if chars.is_empty() {
                        continue;
                    }
                    search.matches.push(SearchMatch {
                        page: *page,
                        quads: match_quads(chars),
                    });
                }
            }
        }
        search.current = search.current.min(search.matches.len().saturating_sub(1));
    }

    /// 命中导航（Enter / Shift+Enter / Ctrl+G / 计数条箭头钮）：
    /// 环回推进 + 滚动到当前命中（页级 uniform 槽 + 页内像素偏移
    /// 直算——目标置于视口上部 30% 处）。
    fn jump_search(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(search) = self.search.as_mut() else {
            return;
        };
        if search.matches.is_empty() {
            return;
        }
        let len = search.matches.len();
        search.current = if forward {
            (search.current + 1) % len
        } else {
            (search.current + len - 1) % len
        };
        let m = search.matches[search.current].clone();
        // 槽几何（与 submit_visible 同源算式）。
        let (vw, vh) = self.viewport_size;
        if vw <= 0. {
            cx.notify();
            return;
        }
        let fit_w = Self::fit_width(vw);
        let slot_h = Self::slot_height(fit_w, self.aspect).max(1.);
        // 页内 y：层几何轴缩放（拉伸页独立 y 轴——同源边界注释见模块头）。
        let layer = self
            .search
            .as_ref()
            .and_then(|s| s.layers.get(&m.page))
            .cloned();
        let y_in_slot = layer
            .as_ref()
            .filter(|l| l.height_pt > 0.)
            .and_then(|l| m.quads.first().map(|q| (q, slot_h / l.height_pt)))
            .map(|(q, sy)| slot_h - q.top * sy)
            .unwrap_or(0.);
        let target = (m.page as f32 * slot_h + y_in_slot - vh * 0.3).max(0.);
        // uniform_list 滚动偏移直设（负值向下——gpui 滚动语义）。
        let max_offset = (self.page_count as f32 * slot_h - vh).max(0.);
        let clamped = target.min(max_offset);
        self.scroll
            .0
            .borrow_mut()
            .base_handle
            .set_offset(gpui::Point::new(px(0.), px(-clamped)));
        cx.notify();
    }

    /// 查找条按键分发（`search_focus` 聚焦时——find_bar 同款输入行
    /// 语义：可打印插入 / 退格删除 / 光标移动 / Enter 跳转 / Esc 关）。
    fn dispatch_search_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ks = &event.keystroke;
        let ctrl = ks.modifiers.control || ks.modifiers.platform;
        let key = ks.key.to_lowercase();
        // 命令键族（与编辑无编辑语义冲突——find_bar 同序）。
        match key.as_str() {
            "escape" => {
                self.close_search(window, cx);
                return;
            }
            "enter" => {
                self.jump_search(!ks.modifiers.shift, cx);
                return;
            }
            "g" if ctrl => {
                self.jump_search(!ks.modifiers.shift, cx);
                return;
            }
            _ => {}
        }
        // 编辑键族（query 行内编辑——光标语义与 find_bar 一致）。
        let mut edited = false;
        let mut moved = false;
        let Some(search) = self.search.as_mut() else {
            return;
        };
        match key.as_str() {
            "backspace" if !ctrl => {
                if search.cursor > 0 {
                    let cut = search
                        .query
                        .char_indices()
                        .nth(search.cursor - 1)
                        .map(|(i, _)| i);
                    if let Some(at) = cut {
                        let end = search.query[search
                            .query
                            .char_indices()
                            .nth(search.cursor)
                            .map(|(i, _)| i)
                            .unwrap_or(search.query.len())..]
                            .to_string();
                        search.query.truncate(at);
                        search.query.push_str(&end);
                        search.cursor -= 1;
                        edited = true;
                    }
                }
            }
            "delete" if !ctrl => {
                let total = search.query.chars().count();
                if search.cursor < total {
                    let start = search
                        .query
                        .char_indices()
                        .nth(search.cursor)
                        .map(|(i, _)| i)
                        .unwrap_or(search.query.len());
                    let end = search
                        .query
                        .char_indices()
                        .nth(search.cursor + 1)
                        .map(|(i, _)| i)
                        .unwrap_or(search.query.len());
                    search.query.replace_range(start..end, "");
                    edited = true;
                }
            }
            "left" if !ctrl => {
                search.cursor = search.cursor.saturating_sub(1);
                moved = true;
            }
            "right" if !ctrl => {
                search.cursor = (search.cursor + 1).min(search.query.chars().count());
                moved = true;
            }
            "home" if !ctrl => {
                search.cursor = 0;
                moved = true;
            }
            "end" if !ctrl => {
                search.cursor = search.query.chars().count();
                moved = true;
            }
            _ => {
                // 可打印字符插入（无修饰或仅 shift；IME 未接管直输路径
                // ——与 editor 输入行同口径，控制字符过滤）。
                if !ctrl
                    && !ks.modifiers.alt
                    && let Some(c) = &ks.key_char
                    && !c.is_empty()
                    && c.chars().all(|ch| !ch.is_control())
                {
                    let at = search
                        .query
                        .char_indices()
                        .nth(search.cursor)
                        .map(|(i, _)| i)
                        .unwrap_or(search.query.len());
                    search.query.insert_str(at, c);
                    search.cursor += 1;
                    edited = true;
                }
            }
        }
        if edited {
            self.rematch();
        }
        if edited || moved {
            cx.notify();
        }
    }

    /// #422：视图态按键（视图焦点面）：Ctrl+F（换绑跟随）开页内查找
    /// 条。仅解析 OpenFind——其余键不吞不拦（滚轮/拖拽不依赖焦点，
    /// 本层零副作用）。
    fn dispatch_view_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ks = &event.keystroke;
        let ctrl = ks.modifiers.control || ks.modifiers.platform;
        let kq = crate::keymap::KeyQuery {
            key: ks.key.to_lowercase(),
            ctrl,
            shift: ks.modifiers.shift,
            alt: ks.modifiers.alt,
        };
        if let crate::keymap::EditorOutcome::Matched(crate::keymap::EditorAction::OpenFind) =
            crate::keymap::store().resolve_editor_main(&kq, None)
        {
            self.open_search(window, cx);
        }
    }

    /// #386：是否有未落盘批注（Shell 据此驱动关闭确认与标题栏脏标；
    /// e2e 断言面）。
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// #386 e2e 断言面：构造期引擎错误（None = 页视图在位可渲染；
    /// Some = error 态面板文案）。测试据此在无 pdfium 环境 SKIP
    /// （render_170_pdf 同口径——不伪装通过）。
    pub(crate) fn engine_error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// #386：批注落盘（增量——产物字节以原文件为逐位前缀，签名保持
    /// #227/#308 语义）。路径由 Shell 传入（单一真源：Shell 的
    /// `pdf_pages` 槽持有 `PathBuf`，视图不重复存）。**写盘成功才清
    /// 脏标**（原子语义）；错误透传（调用方必须显式示警，不许静默）。
    ///
    /// - `Ok(None)`：无未落盘批注（干净 no-op）；
    /// - `Ok(Some(stats))`：已增量写盘（`stats` = 追加修订统计）。
    pub fn save_annotations(
        &mut self,
        path: &Path,
    ) -> Result<Option<IncrementalSave>, crate::pdf_annotate::AnnotateError> {
        if !self.dirty {
            return Ok(None);
        }
        let Some(controller) = self.annotate.as_mut() else {
            // 脏标在位而控制器不在（不可达——dirty 只经
            // commit_annotation 成功路径置位，控制器随之在位）。防御
            // 态：清标按干净处理，不虚构写盘。
            self.dirty = false;
            return Ok(None);
        };
        let stats = controller.save(path)?;
        self.dirty = false;
        eprintln!("pdf-annot-save: {}（增量落盘）", path.display());
        Ok(Some(stats))
    }

    /// flush 后刷新：换字节快照 + 重建渲染工人（引擎重开见追加修订）
    /// + 旧 rev 缓存即刻回收（#305）+ rev 镜像同步（RenderKey 键控）。
    /// 几何未变（批注不改页树）——`rendered_heights` 保留，同键新 rev
    /// 重光栅后即命中。#422：批注刷新不废查找索引（文本层不受批注
    /// 追加影响——提取面读的是文本对象，高亮/下划线/便签不改文本层）。
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
        let focus = self.focus_handle.clone();
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
        // #422：页内查找条（open_search 后在位——右上角悬浮；输入行
        // 焦点域 + 计数 + 前后导航钮 + 关闭钮）。
        let search_bar = self.render_search_bar(_window, cx);
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
            // #422：视图焦点域（页视图态主显示面——焦点从隐藏的文本
            // 编辑器迁至此，键事件不再游离；Ctrl+F 视图级开查找条）。
            .track_focus(&focus)
            .on_key_down(cx.listener(Self::dispatch_view_key))
            .child(list)
            // #302④：批注错误横幅（在位时叠加顶部）。
            .children(error_banner)
            // #302 验收 1：工具提示横幅（在位时叠加底部）。
            .children(hint_banner)
            // #422：页内查找条（在位时叠加右上角）。
            .children(search_bar)
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

impl PdfPagesView {
    /// #422：查找条（find_bar 同款输入行——keystroke 驱动 + shape
    /// 前缀宽度的光标竖线；非 gpui text_input——与编辑器查找条同一
    /// 交互范式，键位面见 [`Self::dispatch_search_key`]）。
    fn render_search_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let search = self.search.as_ref()?;
        let colors = cx.theme().colors().clone();
        let label = search.match_label();
        let cursor_col = search.cursor;
        // 输入光标 x：shape query 前缀宽度（编辑器 find_bar 同款算式）。
        let prefix: String = search.query.chars().take(cursor_col).collect();
        let text_style = gpui::TextStyle {
            color: colors.text,
            font_family: "Sarasa Mono SC".into(),
            font_size: px(13.).into(),
            line_height: px(18.).into(),
            ..Default::default()
        };
        let prefix_width = shape_text_width(window, &prefix, &text_style);
        let focus = self.search_focus.clone();
        Some(
            h_flex()
                .id("pdf-search-bar")
                .absolute()
                .top_4()
                .right_12()
                .gap_1()
                .px_2()
                .py_1()
                .rounded_md()
                .opacity(0.97)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_background)
                .shadow_md()
                .child(
                    Icon::new(IconName::MagnifyingGlass)
                        .size(IconSize::Small)
                        .color(ui::Color::Muted),
                )
                // 输入行（焦点域 + 按键面 + 点击聚焦）。
                .child(
                    div()
                        .id("pdf-search-input")
                        .track_focus(&focus)
                        .on_key_down(cx.listener(Self::dispatch_search_key))
                        .on_click(cx.listener(|_, _, _, cx| cx.notify()))
                        .w(px(180.))
                        .py_0p5()
                        .relative()
                        .text_size(px(13.))
                        .font_family("Sarasa Mono SC")
                        .child(StyledText::new(search.query.clone()))
                        .child(
                            div()
                                .id("pdf-search-caret")
                                .absolute()
                                .left(prefix_width)
                                .top(px(2.))
                                .w(px(1.5))
                                .h(px(14.))
                                .bg(colors.text),
                        ),
                )
                // 计数条（「n/m」+ 索引进度——match_label 单一源）。
                .child(
                    div()
                        .id("pdf-search-count")
                        .px_1()
                        .text_size(px(11.))
                        .line_height(px(16.))
                        .text_color(colors.text_muted)
                        .child(label),
                )
                // 前一命中（Shift+Enter 同源）。#565：批 6 登记遗留件
                // 补齐（#445 冲突区已解除）——icon-only 钮 tooltip
                // 统一化（editor find-close 同款）。零行为改动。
                .child(
                    div()
                        .id("pdf-search-prev")
                        .px_1()
                        .rounded_sm()
                        .text_size(px(12.))
                        .line_height(px(16.))
                        .text_color(colors.text_muted)
                        .hover(|el| el.bg(colors.element_hover))
                        .tooltip(Tooltip::text("上一处（Shift+Enter）"))
                        .child("↑")
                        .on_click(cx.listener(|this, _, _, cx| this.jump_search(false, cx))),
                )
                // 下一命中（Enter / Ctrl+G 同源）。#565：tooltip 补齐同上。
                .child(
                    div()
                        .id("pdf-search-next")
                        .px_1()
                        .rounded_sm()
                        .text_size(px(12.))
                        .line_height(px(16.))
                        .text_color(colors.text_muted)
                        .hover(|el| el.bg(colors.element_hover))
                        .tooltip(Tooltip::text("下一处（Enter / Ctrl+G）"))
                        .child("↓")
                        .on_click(cx.listener(|this, _, _, cx| this.jump_search(true, cx))),
                )
                // 关闭（Esc 同源——层索引随之丢弃）。#565：tooltip 补齐同上。
                .child(
                    div()
                        .id("pdf-search-close")
                        .px_1()
                        .rounded_sm()
                        .text_size(px(12.))
                        .line_height(px(16.))
                        .text_color(colors.text_muted)
                        .hover(|el| el.bg(colors.element_hover))
                        .tooltip(Tooltip::text("关闭搜索（Esc）"))
                        .child("×")
                        .on_click(cx.listener(|this, _, window, cx| this.close_search(window, cx))),
                )
                .into_any_element(),
        )
    }
}

/// #422 索引线程体：一次开引擎 → 逐页 `positioned_chars` + 页几何 →
/// 层回流。提取失败页如实计数（计入 `Done::failed_pages`——计数条
/// 脚注「N 页文本层不可读」）；引擎打不开 = 全部页失败（视图本身
/// 已是 error 态的兄弟场景，防御面）。通道关闭（查找条已关/视图
/// 释放）→ send 失败即止，不空转。
fn index_document(bytes: Arc<Vec<u8>>, page_count: u32, tx: Sender<SearchIndexMsg>) {
    let mut failed = 0u32;
    let engine = match PdfiumEngine::open((*bytes).clone()) {
        Ok(engine) => engine,
        Err(_) => {
            let _ = tx.send(SearchIndexMsg::Done {
                failed_pages: page_count,
            });
            return;
        }
    };
    for page in 0..page_count {
        // 页几何（pt）——高亮换算的轴缩放基准；取不到按 A4 兜底
        //（595×842——只影响高亮落点精度，不影响匹配面）。
        let (width_pt, height_pt) = engine
            .page_geometry(PageIndex(page))
            .map(|g| (g.effective_width(), g.effective_height()))
            .unwrap_or((595., 842.));
        match engine.positioned_chars(PageIndex(page)) {
            Ok(chars) => {
                let lower = chars
                    .iter()
                    .map(|c| c.ch.to_lowercase().collect::<String>())
                    .collect::<String>();
                let layer = PageTextLayer {
                    chars,
                    lower,
                    width_pt,
                    height_pt,
                };
                if tx.send(SearchIndexMsg::Layer(Box::new(layer))).is_err() {
                    return; // 查找条已关——线程即止。
                }
            }
            Err(_) => {
                failed += 1;
            }
        }
    }
    let _ = tx.send(SearchIndexMsg::Done {
        failed_pages: failed,
    });
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
    use std::path::PathBuf;

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
        // 640 × 1.415 = 905.6。
        assert!((PdfPagesView::slot_height(640, 1.415) - 905.6).abs() < 0.01);
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
            width_pt: 595.,
            height_pt: 842.,
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
        let file = TempPdf(
            std::env::temp_dir().join(format!("mang-386-gesture-{}.pdf", std::process::id())),
        );
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
        let (annots, report) = mdc_pdf::annotate::read_annotations(
            on_disk.clone(),
            mdc_pdf::security::Limits::default(),
        )
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
        let file = TempPdf(
            std::env::temp_dir().join(format!("mang-386-accum-{}.pdf", std::process::id())),
        );
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
}
