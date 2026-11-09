//! #420：PDF 页面缩略图侧栏——低 dpi 光栅化 + 虚拟化懒加载 + 跳页/高亮联动。
//!
//! ## 交付背景（恢复交付范围）
//!
//! PR #448 空关（0 commits/0 files）无可恢复实现，本模块按 issue #420
//! 原方案全新开发。Acrobat 左侧栏同款：打开 PDF 页视图即有缩略图导航。
//!
//! ## 架构（高内聚 / 低耦合）
//!
//! ```text
//! shell.rs ──h_flex──┬─ PdfThumbnailsView（本模块——左侧栏 148px）
//!                    └─ PdfPagesView（主视图——既有页面渲染）
//!        联动经 PdfThumbsHooks 双向钩子（壳注入，本模块零 shell 依赖）：
//!        · jump_to_page：点击缩略行 → shell → PdfPagesView::scroll_to_page
//!        · current_page：主视图 top_visible_page → 侧栏高亮 + Nearest 跟随
//! ```
//!
//! - **渲染**：自有低 dpi [`RenderWorker`]（128px 光栅——与主视图工人
//!   独立、互不抢优先级）；字节快照与主视图**同源 Arc 共享**（零拷贝
//!   零重复内存），看门狗换代同纪律。
//! - **懒加载**（验收「大文件不卡」）：uniform_list 虚拟化——只对真实
//!   可见行提交光栅作业；≤ [`EAGER_MAX_PAGES`] 页小文档首批**全量**
//!   （issue 原方案「60 页以内全生成」），以上只染可见窗。缓存 32MB
//!   LRU（128px BGRA ≈ 92KB/页——千页级长文档靠预算逐出 + 可见自愈）。
//! - **联动**：主视图滚动 → [`PdfPagesView::top_visible_page`]（#420
//!   新读数面）→ 侧栏高亮行 + `scroll_to_item(Nearest)` 就近带回（已
//!   可视不动——不与用户侧栏滚动打架）；点击缩略行 → 跳页。
//!
//! ## 环境门（AGENTS 诚实边界）
//!
//! 无 libpdfium 环境构造期引擎绑定失败 → `error` 态（面板明示不白屏；
//! shell 据主视图 error 态不挂侧栏）。真渲染测试在缺件环境 SKIP 横幅
//! ——CI（windows，MDC_PDFIUM_LIB 在位）与本机（真库路径导出）真跑。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use gpui::{AnyElement, App, ImageSource, RenderImage, UniformListScrollHandle, Window, px};
use mdc_pdf::engine::DocumentEngine;
use mdc_pdf::engine_pdfium::PdfiumEngine;
use mdc_pdf::page::PageIndex;
use mdc_pdf::render::RenderRequest;
use mdc_pdf::worker::{RenderWorker, WorkerReply};
use ui::prelude::*;

use crate::pdf_pages::rendered_page_to_image;

/// 侧栏光栅宽（px）——64 量子对齐（128 = 2×64，主视图 WIDTH_QUANTUM
/// 同口径下无需再量化）；issue 原方案 120px 档就近取整。
pub const THUMB_WIDTH: u32 = 128;
/// 侧栏列宽（px）——光栅宽 + 左右留白 + 滚动条余量。
pub const SIDEBAR_W: f32 = 148.;
/// 页码说明行高（px）。
const CAPTION_H: f32 = 18.;
/// 缩略行间垂直留白（px）。
const ROW_GAP: f32 = 6.;
/// 小文档首批全量阈值（页）——issue 原方案「60 页以内全生成」。
pub const EAGER_MAX_PAGES: usize = 60;
/// 缩略缓存字节预算（100 页 ≈ 9MB；千页级 LRU 逐出自愈）。
pub const THUMB_CACHE_BUDGET: usize = 32 * 1024 * 1024;
/// 渲染看门狗（pdf_pages 主工人同款 20s——敌意页换代不炸侧栏）。
const THUMB_WATCHDOG: Duration = Duration::from_secs(20);
/// 回执轮询周期（pdf_pages POLL_INTERVAL 同款 16ms——待办清空退出）。
const THUMB_POLL_MS: Duration = Duration::from_millis(16);

/// 跳页回调签名（0 基页号；壳接主视图 scroll_to_page）。
pub type PageJumpFn = Arc<dyn Fn(usize, &mut Window, &mut App)>;
/// 当前页读数回调签名（壳接主视图 top_visible_page）。
pub type CurrentPageFn = Arc<dyn Fn(&App) -> usize>;

/// #420 联动钩子（壳注入——本模块零 shell/主视图依赖；测试同面注入）。
pub struct PdfThumbsHooks {
    /// 点击缩略行（0 基页号）→ 跳页（壳接
    /// [`crate::pdf_pages::PdfPagesView::scroll_to_page`]）。
    pub jump_to_page: PageJumpFn,
    /// 当前顶部可见页读数（壳接
    /// [`crate::pdf_pages::PdfPagesView::top_visible_page`]）。
    pub current_page: CurrentPageFn,
}

/// 缩略图侧栏实体（左栏——点击跳页 + 高亮跟随）。
pub struct PdfThumbnailsView {
    page_count: usize,
    /// 首页纵横比（与主视图同源——uniform 槽高基准）。
    aspect: f32,
    /// 低 dpi 渲染工人（独立线程 + 看门狗换代；error 态为 None）。
    worker: Option<RenderWorker<PdfiumEngine>>,
    /// 缩略缓存（页号键控——预算 LRU）。
    cache: mdc_pdf::cache::PageCache<u32, Arc<RenderImage>>,
    /// 在途作业：job_id → 页号（回执对账）。
    pending: HashMap<u64, u32>,
    /// 失败页（敌意页一次性登记，不无限重试）。
    failed: HashSet<u32>,
    hooks: PdfThumbsHooks,
    /// 上次高亮页（变更时侧栏 Nearest 跟随去重）。
    highlighted: usize,
    /// 小文档首批全量已发（每实体一次）。
    eager_done: bool,
    /// 回执轮询循环在跑（防重复起）。
    polling: bool,
    /// 虚拟列表滚动句柄（点击命中 + Nearest 跟随）。
    scroll: UniformListScrollHandle,
    /// 引擎面错误（构造期 open 失败——面板明示，不白屏）。
    error: Option<String>,
}

impl PdfThumbnailsView {
    /// 构造：主线程一次性 open 读页数/纵横比（与主视图几何预读同源
    /// 口径——aspect 入参为主视图同源值，字节自证优先），随即把引擎
    /// 交给看门狗工人在渲染线程重建（字节快照 Arc 共享）。
    pub fn new(bytes: Arc<Vec<u8>>, aspect: f32, hooks: PdfThumbsHooks, _cx: &mut App) -> Self {
        let (page_count, aspect) = match PdfiumEngine::open((*bytes).clone()) {
            Ok(engine) => {
                let count = engine.page_count() as usize;
                let geo = (0..count.min(3))
                    .filter_map(|i| engine.page_geometry(PageIndex(i as u32)).ok())
                    .find(|g| g.effective_width() > 0. && g.effective_height() > 0.);
                let aspect = geo
                    .map(|g| g.effective_height() / g.effective_width())
                    .unwrap_or(aspect);
                (count, aspect)
            }
            Err(err) => {
                return Self::error_state(aspect, hooks, format!("pdfium 引擎打开失败：{err:?}"));
            }
        };
        let factory_bytes = bytes.clone();
        let worker = RenderWorker::watchdog(
            move || {
                PdfiumEngine::open((*factory_bytes).clone())
                    .expect("pdfium 看门狗工厂：字节快照重开失败（构造期已验证可开）")
            },
            THUMB_WATCHDOG,
        );
        Self {
            page_count,
            aspect,
            worker: Some(worker),
            cache: mdc_pdf::cache::PageCache::new(THUMB_CACHE_BUDGET),
            pending: HashMap::new(),
            failed: HashSet::new(),
            hooks,
            highlighted: 0,
            eager_done: false,
            polling: false,
            scroll: UniformListScrollHandle::new(),
            error: None,
        }
    }

    /// error 态构造（引擎绑定/打开失败——面板明示；壳据主视图态不挂）。
    fn error_state(aspect: f32, hooks: PdfThumbsHooks, message: String) -> Self {
        Self {
            page_count: 0,
            aspect,
            worker: None,
            cache: mdc_pdf::cache::PageCache::new(THUMB_CACHE_BUDGET),
            pending: HashMap::new(),
            failed: HashSet::new(),
            hooks,
            highlighted: 0,
            eager_done: true,
            polling: false,
            scroll: UniformListScrollHandle::new(),
            error: Some(message),
        }
    }

    /// 总页数（行数与跳页钳制同源）。
    pub fn page_count(&self) -> usize {
        self.page_count
    }

    /// 槽高（px）：光栅宽 × 纵横比 + 说明行 + 行间留白（uniform_list
    /// 行高单点口径——点击命中测试坐标推导亦用）。
    pub fn row_height(aspect: f32) -> f32 {
        THUMB_WIDTH as f32 * aspect + CAPTION_H + ROW_GAP
    }

    /// 就绪判定（懒加载断言读面——测试与 shell 同源）。
    pub fn thumb_ready(&self, page: usize) -> bool {
        self.cache.contains(&(page as u32))
    }

    /// 当前高亮行（联动断言读面）。
    pub fn highlighted_page(&self) -> usize {
        self.highlighted
    }

    /// 提交一页缩略光栅（已缓存/在途/失败幂等跳过）。
    fn submit_thumb(&mut self, page: u32, cx: &mut gpui::Context<Self>) {
        let Some(worker) = self.worker.as_mut() else {
            return;
        };
        if self.cache.contains(&page) || self.failed.contains(&page) {
            return;
        }
        if self.pending.values().any(|&p| p == page) {
            return;
        }
        let id = worker.submit(RenderRequest {
            page: PageIndex(page),
            width_px: THUMB_WIDTH,
        });
        self.pending.insert(id, page);
        if !self.polling {
            self.polling = true;
            Self::spawn_poll_loop(cx);
        }
    }

    /// 回执轮询循环（16ms try_recv——待办清空退出；实体释放自然止）。
    fn spawn_poll_loop(cx: &mut gpui::Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(THUMB_POLL_MS).await;
                let mut done = false;
                // #634：`update` Err（实体释放）即 break——`let _ =` 忽略
                // 释放错误会让循环持死定时器永不退（与 pdf_pages 同款
                // 泄漏修复）。
                match this.update(cx, |view, cx| {
                    view.drain_replies(cx);
                    if view.pending.is_empty() {
                        view.polling = false;
                        done = true;
                    }
                }) {
                    Ok(()) => {}
                    Err(_) => break, // 实体已释放——轮询链随退。
                }
                if done {
                    break;
                }
            }
        })
        .detach();
    }

    /// 收回执（Done → 缓存 + notify；Cancelled/Timeout/Err → failed）。
    fn drain_replies(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(worker) = self.worker.as_mut() else {
            return;
        };
        while let Some(reply) = worker.try_recv() {
            match reply {
                WorkerReply::Done { id, result } => {
                    let Some(page) = self.pending.remove(&id) else {
                        continue; // 迟到回执（换代竞态）——机械丢弃。
                    };
                    match result {
                        Ok(rendered) => {
                            if let Some(image) = rendered_page_to_image(&rendered) {
                                let cost = rendered.bgra.len();
                                let _evicted = self.cache.put(page, Arc::new(image), cost);
                                cx.notify();
                            } else {
                                self.failed.insert(page);
                                cx.notify();
                            }
                        }
                        Err(_) => {
                            self.failed.insert(page);
                            cx.notify();
                        }
                    }
                }
                WorkerReply::Cancelled { id } | WorkerReply::Timeout { id } => {
                    if let Some(page) = self.pending.remove(&id) {
                        self.failed.insert(page);
                        cx.notify();
                    }
                }
            }
        }
    }

    /// 可见缩略行（uniform_list 回调期——懒加载提交点 + 三态渲染 +
    /// 点击跳页）。uniform_list 虚拟化只回调真实可见行——窗外页零
    /// 作业零内存（大文件不卡的机制点）。
    fn render_thumb_row(&mut self, idx: usize, cx: &mut gpui::Context<Self>) -> AnyElement {
        // 懒加载提交点。
        self.submit_thumb(idx as u32, cx);
        let colors = cx.theme().colors().clone();
        let aspect = self.aspect;
        let highlighted = self.highlighted.min(self.page_count.saturating_sub(1));
        let jump = self.hooks.jump_to_page.clone();
        let img_h = THUMB_WIDTH as f32 * aspect;
        let row_h = Self::row_height(aspect);

        // 三态：Ready（缓存位图）/ Failed（敌意页）/ Pending（占位）。
        let thumb: gpui::AnyElement = if self.failed.contains(&(idx as u32)) {
            failed_slot(idx, colors.border, img_h)
        } else if let Some(image) = self.cache.get(&(idx as u32)) {
            gpui::img(ImageSource::Render(image.clone()))
                .id(gpui::ElementId::NamedInteger(
                    "pdf-thumb-img".into(),
                    idx as u64,
                ))
                .w(px(THUMB_WIDTH as f32))
                .h(px(img_h))
                .flex_none()
                .rounded_sm()
                .into_any_element()
        } else {
            pending_slot(idx, colors.border, img_h)
        };

        div()
            .id(gpui::SharedString::from(format!("pdf-thumb-row-{idx}")))
            .flex_none()
            .h(px(row_h))
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            // 当前页高亮：行描边 + 说明行强调（主视图滚动联动面）。
            .when(highlighted == idx, |row| {
                row.border_1().border_color(colors.border)
            })
            .child(thumb)
            .child(
                Label::new(format!("第 {} 页", idx + 1))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted)
                    .when(highlighted == idx, |label| label.color(Color::Accent)),
            )
            // 点击跳页：真实 GPUI 命中（stateless handler——壳注入钩子
            // 直接消费，无实体重入面）。
            .on_click(move |_, window, cx| {
                jump(idx, window, cx);
            })
            .into_any_element()
    }
}

/// 未就绪占位（渲染中）——不闪结构、不产空白假图。
fn pending_slot(idx: usize, border: gpui::Hsla, img_h: f32) -> AnyElement {
    div()
        .id(gpui::SharedString::from(format!("pdf-thumb-pending-{idx}")))
        .w(px(THUMB_WIDTH as f32))
        .h(px(img_h))
        .flex_none()
        .rounded_sm()
        .border_1()
        .border_color(border)
        .flex()
        .items_center()
        .justify_center()
        .child(Label::new("…").size(LabelSize::XSmall).color(Color::Muted))
        .into_any_element()
}

/// 失败占位（敌意页——一次性登记不无限重试）。
fn failed_slot(idx: usize, border: gpui::Hsla, img_h: f32) -> AnyElement {
    div()
        .id(gpui::SharedString::from(format!("pdf-thumb-failed-{idx}")))
        .w(px(THUMB_WIDTH as f32))
        .h(px(img_h))
        .flex_none()
        .rounded_sm()
        .border_1()
        .border_color(border)
        .flex()
        .items_center()
        .justify_center()
        .child(
            Label::new("渲染失败")
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .into_any_element()
}

impl Render for PdfThumbnailsView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        // error 态：明示面板（不白屏；壳据主视图态不挂侧栏，此面为
        // 独立持有实体时的防御呈现）。
        if self.error.is_some() || self.worker.is_none() {
            let message = self.error.clone().unwrap_or_else(|| "pdfium 未绑定".into());
            return v_flex()
                .id("pdf-thumbs-error")
                .size_full()
                .items_center()
                .justify_center()
                .bg(colors.background)
                .child(Label::new(message).color(Color::Warning))
                .into_any_element();
        }
        // 小文档首批全量（≤60 页——issue「60 页以内全生成」；一次性）。
        if !self.eager_done {
            self.eager_done = true;
            if self.page_count <= EAGER_MAX_PAGES {
                for page in 0..self.page_count {
                    self.submit_thumb(page as u32, cx);
                }
            }
        }
        // 高亮联动：主视图顶部页 → 高亮行 + Nearest 跟随（已可视不动——
        // 不与用户侧栏滚动打架）。
        let current = (self.hooks.current_page)(cx).min(self.page_count.saturating_sub(1));
        if current != self.highlighted {
            self.highlighted = current;
            self.scroll
                .scroll_to_item(current, gpui::ScrollStrategy::Nearest);
        }
        let entity = cx.entity().downgrade();
        let page_count = self.page_count;
        let scroll = self.scroll.clone();
        let list = gpui::uniform_list(
            "pdf-thumbs",
            page_count,
            move |visible: std::ops::Range<usize>, _window: &mut Window, cx: &mut App| {
                visible
                    .clone()
                    .filter_map(|idx| {
                        entity
                            .update(cx, |view, cx| view.render_thumb_row(idx, cx))
                            .ok()
                    })
                    .collect::<Vec<AnyElement>>()
            },
        )
        .track_scroll(&scroll)
        .flex_1()
        .min_h_0();
        v_flex()
            .id("pdf-thumbs-sidebar")
            .w(px(SIDEBAR_W))
            .h_full()
            .flex_none()
            .border_r_1()
            .border_color(colors.border)
            .bg(colors.panel_background)
            .child(
                div()
                    .id("pdf-thumbs-header")
                    .flex_none()
                    .w_full()
                    .px_2()
                    .py_1()
                    .child(
                        Label::new("缩略图")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .child(list)
            .into_any_element()
    }
}

// ---------------------------------------------------------------------------
// #420 无头 GPUI 测试（lib 树——CI 可见；真实 pdfium 全链 + 缺件 SKIP 门）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf_pages::PdfPagesView;
    use gpui::{
        Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput, point,
    };
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;

    fn fixture_bytes(name: &str) -> Vec<u8> {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        std::fs::read(&p).unwrap_or_else(|e| panic!("夹具不可读（{}）：{e}", p.display()))
    }

    /// 双视图宿主：侧栏 + 主视图并排（shell h_flex 同构——联动走真实
    /// hooks 接线：点击 → 真主视图 scroll_to_page；高亮 ← 真主视图
    /// top_visible_page。非 index 补偿模拟器）。
    struct ThumbsHost {
        thumbs: Entity<PdfThumbnailsView>,
        pages: Entity<PdfPagesView>,
        // 测试端点击记录：jump 钩子闭包 push，宿主持一份防提前释放；
        // 断言经宿主窗口外句柄读（click 测试）——本字段非直读面。
        #[allow(dead_code)]
        clicked: Rc<RefCell<Vec<usize>>>,
    }

    impl gpui::Render for ThumbsHost {
        fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            // shell.rs 生产链同构：shell-body = h_flex.flex_1().min_h_0()
            // （GROW 解析高度——比百分比稳）；外层 v_flex 列容器提供
            // 明确主轴。主视图包装器 flex_1 + min_w_0。
            gpui::div().size_full().flex().flex_col().child(
                h_flex()
                    .id("pdf-view-with-thumbs")
                    .flex_1()
                    .min_h_0()
                    .child(self.thumbs.clone())
                    .child(div().flex_1().min_w_0().h_full().child(self.pages.clone())),
            )
        }
    }

    /// 主题初始化序列镜像 main.rs（Label/panel 渲染依赖）+ 双实体真实
    /// hooks 接线 + 整树宿主窗口（鼠标命中需要整树 + 命中盒）。
    type Host = (
        Entity<PdfThumbnailsView>,
        Entity<PdfPagesView>,
        gpui::WindowHandle<ThumbsHost>,
        Rc<RefCell<Vec<usize>>>,
    );

    fn host_window(cx: &mut gpui::TestAppContext, doc: &str) -> Option<Host> {
        #![allow(clippy::arc_with_non_send_sync)] // 钩子闭包捕获非 Send 实体引用——钩子只在主线程 App 上下文调用，不跨线程
        cx.update(|app| {
            theme::init(theme::LoadThemes::JustBase, app);
            theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
        });
        let bytes = fixture_bytes(doc);
        let clicked: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));
        let (thumbs, pages) = cx.update(|cx| {
            let pages = cx.new(|cx| PdfPagesView::new(bytes.clone(), cx));
            let pages_jump = pages.clone();
            let pages_read = pages.clone();
            let clicked_hook = clicked.clone();
            let thumbs = cx.new(|cx| {
                PdfThumbnailsView::new(
                    pages.read(cx).bytes(),
                    pages.read(cx).page_aspect(),
                    PdfThumbsHooks {
                        jump_to_page: Arc::new(move |page, _window, cx| {
                            clicked_hook.borrow_mut().push(page);
                            pages_jump.update(cx, |v, _| v.scroll_to_page(page));
                        }),
                        current_page: Arc::new(move |app| pages_read.read(app).top_visible_page()),
                    },
                    cx,
                )
            });
            (thumbs, pages)
        });
        // 环境门（#422 同款）：error 含 pdfium = 缺库 → SKIP 横幅
        // （错误全文随横幅打——绑定失败原因可诊断，不静默）。
        let err = cx.read(|app| thumbs.read(app).error.clone());
        match err {
            Some(e) if e.contains("pdfium") => {
                eprintln!(
                    "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                     本机无 libpdfium——#420 缩略图全链 e2e 未真跑。\n  \
                     绑定/打开错误全文：{e}\n  \
                     有 libpdfium 的环境（MDC_PDFIUM_LIB=<库路径> \
                     cargo test）全量执行。\n═══════════════════════════\n"
                );
                None
            }
            Some(e) => panic!("缩略侧栏构造失败（夹具/真缺陷，非环境缺件）：{e}"),
            None => {
                let (t, p) = (thumbs.clone(), pages.clone());
                let clicked_for_window = clicked.clone();
                // add_window：最大化测试显示器边界（#422 SearchHost 同款
                // ——真实尺寸布局）。
                let window = cx.add_window(move |_window, _cx| ThumbsHost {
                    thumbs: t.clone(),
                    pages: p.clone(),
                    clicked: clicked_for_window.clone(),
                });
                Some((thumbs, pages, window, clicked))
            }
        }
    }

    /// 回执泵：推进测试时钟 + 真线程跑光栅 + 侧栏 16ms 轮询 drain
    /// （#422 pump_until_indexed 同构——真实线程/通道/轮询全链）。
    /// 每轮重绘一帧：渲染期读数面（主视图 top_page / 侧栏高亮）随帧
    /// 刷新——只推进时钟不重绘时这些读数永不更新。
    fn pump_thumbs(
        cx: &mut gpui::TestAppContext,
        window: &gpui::WindowHandle<ThumbsHost>,
        timeout_secs: u64,
        ready: impl Fn(&mut gpui::TestAppContext) -> bool,
    ) {
        let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
        while std::time::Instant::now() < deadline {
            cx.background_executor
                .advance_clock(Duration::from_millis(20));
            cx.background_executor.run_until_parked();
            std::thread::sleep(Duration::from_millis(3));
            redraw(window, cx);
            if ready(cx) {
                return;
            }
        }
        panic!("缩略回执 {timeout_secs}s 墙钟未就绪（真缺陷不放过——CI 慢机另行调阈）");
    }

    /// 真实单击：MouseDown + MouseUp 同点派发（shell.rs b11 click_at 同构）。
    fn click_at(
        window: &gpui::WindowHandle<ThumbsHost>,
        cx: &mut gpui::TestAppContext,
        x: f32,
        y: f32,
    ) {
        let any: gpui::AnyWindowHandle = (*window).into();
        let down = PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position: point(px(x), px(y)),
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        });
        let up = PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position: point(px(x), px(y)),
            modifiers: Modifiers::default(),
            click_count: 1,
        });
        cx.update_window(any, |_, window, cx| {
            window.dispatch_event(down, cx);
            window.dispatch_event(up, cx);
        })
        .expect("单击派发");
    }

    /// 重绘一帧（shell.rs b11 redraw 同构——update_window 口径，不
    /// 双租宿主实体；命中盒随最新状态重注册）。
    fn redraw(window: &gpui::WindowHandle<ThumbsHost>, cx: &mut gpui::TestAppContext) {
        let any: gpui::AnyWindowHandle = (*window).into();
        cx.update_window(any, |_, window, cx| {
            window.draw(cx).clear(cx);
        })
        .expect("重绘一帧");
    }

    /// 验收 1+4：小文档（≤60 页）首批全量就绪；大文档（100 页）只染
    /// 可见窗——窗外页零作业（懒加载），滚入可见即自愈就绪。
    #[gpui::test]
    fn eager_small_doc_and_lazy_big_doc(cx: &mut gpui::TestAppContext) {
        // ① 小文档：首批全量（smoke.pdf——页数 ≤60）。
        let host = host_window(cx, "smoke.pdf");
        let (thumbs, _pages, window, _clicked) = match host {
            None => return, // SKIP 门已打横幅
            Some(h) => h,
        };
        let count = cx.read(|app| thumbs.read(app).page_count());
        assert!(count > 0, "夹具应有页");
        assert!(count <= EAGER_MAX_PAGES, "夹具页数应在全量阈值内");
        redraw(&window, cx);
        pump_thumbs(cx, &window, 15, |cx| {
            (0..count).all(|i| cx.read(|app| thumbs.read(app).thumb_ready(i)))
        });
        for i in 0..count {
            assert!(
                cx.read(|app| thumbs.read(app).thumb_ready(i)),
                "小文档第 {i} 页首批全量就绪"
            );
        }
        let _ = window; // 窗口存活至断言后（显式声明非有意丢弃）

        // ② 大文档（100 页）：>60 → 懒加载——只染可见窗。
        let host = host_window(cx, "thumbs_100p.pdf");
        let (thumbs, _pages, window, _clicked) = match host {
            None => return,
            Some(h) => h,
        };
        let count = cx.read(|app| thumbs.read(app).page_count());
        assert_eq!(count, 100, "百页夹具");
        redraw(&window, cx);
        let aspect = cx.read(|app| thumbs.read(app).aspect);
        let visible_rows = ((800. - 32.) / PdfThumbnailsView::row_height(aspect)).floor() as usize;
        pump_thumbs(cx, &window, 15, |cx| {
            (0..visible_rows.min(count)).all(|i| cx.read(|app| thumbs.read(app).thumb_ready(i)))
        });
        // 窗外页（远离视口）：零作业——懒加载铁证（eager 下这里必然 Ready）。
        for &far in &[visible_rows + 10, 70, 95] {
            assert!(
                !cx.read(|app| thumbs.read(app).thumb_ready(far)),
                "百页文档窗外页 {far} 不应预生成（懒加载——大文件不卡）"
            );
        }
        // ③ 滚入可见窗 → 自愈就绪（uniform_list 虚拟化 + 提交回执）。
        let far = 95;
        cx.update(|cx| {
            thumbs.update(cx, |t, _| {
                t.scroll.scroll_to_item(far, gpui::ScrollStrategy::Top)
            });
        });
        redraw(&window, cx);
        pump_thumbs(cx, &window, 15, |cx| {
            cx.read(|app| thumbs.read(app).thumb_ready(far))
        });
        assert!(
            cx.read(|app| thumbs.read(app).thumb_ready(far)),
            "滚入可见窗的页自愈就绪"
        );
        let _ = window; // 窗口存活至断言后（显式声明非有意丢弃）
    }

    /// 验收 2+3：点击缩略行 → 真实跳页（主视图读回 top_page）；
    /// 主视图滚动 → 侧栏高亮跟随（含 Nearest 滚动跟随）。
    #[gpui::test]
    fn click_jumps_and_scroll_highlight_follows(cx: &mut gpui::TestAppContext) {
        let host = host_window(cx, "thumbs_100p.pdf");
        let (thumbs, pages, window, clicked) = match host {
            None => return,
            Some(h) => h,
        };
        redraw(&window, cx);
        pump_thumbs(cx, &window, 15, |cx| {
            cx.read(|app| thumbs.read(app).thumb_ready(0))
        });
        assert_eq!(
            cx.read(|app| pages.read(app).top_visible_page()),
            0,
            "前置：主视图顶部页 0"
        );

        // 点击第 9 页缩略行：滚到列表顶（scroll_to_item Top → 行顶对齐
        // 视口顶），行内任一点即该行——x=侧栏中线、y=页眉下方 60px。
        let target = 9;
        cx.update(|cx| {
            thumbs.update(cx, |t, _| {
                t.scroll.scroll_to_item(target, gpui::ScrollStrategy::Top)
            });
        });
        redraw(&window, cx);
        click_at(&window, cx, SIDEBAR_W / 2., 60.);
        assert_eq!(
            cx.read(|_| clicked.borrow().len()),
            1,
            "点击恰好触发一次跳页"
        );
        assert_eq!(
            cx.read(|_| *clicked.borrow().first().unwrap()),
            target,
            "点击第 9 行 → 跳第 9 页（0 基）"
        );
        // 主视图真实滚动读回：top_page 到位（跳页链路闭环——pump 每轮
        // 重绘，render 期 submit_visible 的 top_page 读数随帧刷新）。
        pump_thumbs(cx, &window, 15, |cx| {
            cx.read(|app| pages.read(app).top_visible_page()) == target
        });
        assert_eq!(
            cx.read(|app| pages.read(app).top_visible_page()),
            target,
            "点击 → 主视图顶部页读数 9（scroll_to_page 生效）"
        );
        // 侧栏高亮跟随（渲染期 current_page → highlighted + Nearest）。
        redraw(&window, cx);
        assert_eq!(
            cx.read(|app| thumbs.read(app).highlighted_page()),
            target,
            "主视图到第 9 页 → 侧栏高亮行跟随"
        );

        // 主视图程序化滚到第 20 页（书签跳转同链）→ 侧栏高亮跟随。
        let next = 20;
        cx.update(|cx| {
            pages.update(cx, |v, _| v.scroll_to_page(next));
        });
        pump_thumbs(cx, &window, 15, |cx| {
            cx.read(|app| pages.read(app).top_visible_page()) == next
        });
        redraw(&window, cx);
        assert_eq!(
            cx.read(|app| thumbs.read(app).highlighted_page()),
            next,
            "主视图滚到 20 → 侧栏高亮 Nearest 跟随"
        );
        let _ = window; // 窗口存活至断言后（显式声明非有意丢弃）
    }
}
