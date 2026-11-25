//! #109 批②③：项目搜索/替换**面板实体**（批①纯逻辑层之上的呈现/执行面）。
//!
//! 挂左栏 dock（与文件树同域——#106 dock 报告 D1 归属裁决；Panel trait
//! 全套含换侧）。搜索输入框复用编辑器 `FormInput` 的自绘模式（非编辑
//! 器实体承载——单行输入不需要 Buffer；光标/选区渲染同款，IME 经面板
//! 级 `EntityInputHandler` + paint 期 canvas `handle_input` 接线）。
//!
//! 后台执行（报告 D2 批②立项注记）：
//! - `cx.background_executor().spawn`（smol 线程池——文件收集/正则扫描
//!   全离主线程；与 #306 OCR 轮询同模式）；
//! - `search_id` 自增失效：新搜索/输入变更即作废在途结果（旧结果不
//!   覆盖新态——面板每次响应输入重扫，防「陈旧结果当新结果」）；
//! - 120ms 轮询收结果（`POLL_INTERVAL`——Q-IDE R8 口径；轮询而非纯
//!   await：进度（已扫文件数）在扫描中途实时呈现）；
//! - 取消：`AtomicBool` 置位后后台扫描早退（旧 search_id 的结果落
//!   地前还会再过一道 id 门——双保险）。
//!
//! 批③替换链（报告 D4 反超点：有确认而非参照的无确认直接写盘）：
//! - 预览数据面走批① `preview_replace`（文件清单 + 处数）；
//! - 确认对话框/执行/善后全部经 `SearchPanelHooks` 注入宿主（lib 树
//!   纪律——不引 shell；FileTree/GitPanel 钩子同模式第四次应用）；
//! - 打开标签重载善后由宿主承担（#134 `reload_tab` + mtime 检测——
//!   覆盖「替换写盘后打开标签还显示旧内容」的真机症状）。
//!
//! 测试面：合成树 + capturing hooks（git_panel keyboard_tests 同款
//! 无头形态——渲染/键盘/结果分组/替换计数不依赖真窗口）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::project_search::{self, SearchHit, SearchOptions, SearchReport};
use gpui::{
    App, AsyncApp, Bounds, Context, ElementInputHandler, FocusHandle, Pixels, ScrollHandle,
    SharedString, WeakEntity, Window, canvas,
};
use theme::ThemeColors;
use ui::prelude::*;
use ui::{Icon, IconName, IconSize, Label, LabelSize, List, ListHeader};

/// #109 批②③：搜索面板的 dock 身份键（布局序列化——发布后不可改，
/// 蓝本 dock.rs:37 警告同款）。
pub const SEARCH_PANEL_KEY: &str = "search";

/// 面板输入/结果字体（editor::EDITOR_FONT 同值本地常量——私有面不跨
/// 模块导；两处任一改须同步注释）。
const EDITOR_FONT: &str = "Sarasa Mono SC";

/// 后台扫描执行体（collect + 逐文件扫；取消旗每文件间隙查——早退
/// 返回已扫部分结果（零命中有效——取消后丢弃由 id 门再拦一道））。
fn run_search(
    root: &Path,
    pattern: &str,
    options: &SearchOptions,
    cancel: &AtomicBool,
    scanned: &AtomicU64,
) -> Result<SearchReport, String> {
    // 收集阶段取消检查（大库 collect 也可能耗时）。
    if cancel.load(Ordering::SeqCst) {
        return Ok(SearchReport::default());
    }
    let (files, truncated_files) = project_search::collect_files(root, options.scope.as_deref());
    let mut report = SearchReport {
        truncated_files,
        ..Default::default()
    };
    let re = build_regex(pattern, options)?;
    let mut total_bytes: u64 = 0;
    for path in files {
        if cancel.load(Ordering::SeqCst) {
            return Ok(report);
        }
        scanned.fetch_add(1, Ordering::Relaxed);
        match scan_file(&path, &re, &mut total_bytes) {
            Ok(Some(mut hits)) => report.hits.append(&mut hits),
            Ok(None) => {}
            Err(kind) => match kind {
                ScanSkip::TooBig => report.skipped_big += 1,
                ScanSkip::Binary => report.skipped_binary += 1,
            },
        }
        if report.hits.len() >= project_search::MAX_HITS {
            report.truncated = true;
            break;
        }
        if total_bytes > project_search::MAX_TOTAL_BYTES {
            report.truncated = true;
            break;
        }
    }
    Ok(report)
}

/// 正则装配（批① build_pattern 同口径——本面板独立装配避免跨模块私有
/// 面；两处任一改口径须同步注释）。
fn build_regex(pattern: &str, options: &SearchOptions) -> Result<regex::Regex, String> {
    if pattern.is_empty() {
        return Err("空模式".into());
    }
    let mut source = String::new();
    if !options.case_sensitive {
        source.push_str("(?i)");
    }
    if options.whole_word {
        source.push_str(r"\b(?:");
        source.push_str(pattern);
        source.push_str(r")\b");
    } else {
        source.push_str(pattern);
    }
    regex::Regex::new(&source).map_err(|e| format!("正则无效：{e}"))
}

enum ScanSkip {
    TooBig,
    Binary,
}

/// 单文件扫描（读 + 分类 + 行匹配——批① search_workspace 内联同逻辑
/// 的文件级拆分；取消粒度在文件间）。Ok(None) = 跳过（预算/二进制之外的
/// 读失败静默跳过）。
fn scan_file(
    path: &Path,
    re: &regex::Regex,
    total_bytes: &mut u64,
) -> Result<Option<Vec<SearchHit>>, ScanSkip> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return Ok(None),
    };
    if bytes.len() as u64 > project_search::MAX_FILE_BYTES {
        return Err(ScanSkip::TooBig);
    }
    let sniff = &bytes[..bytes.len().min(8 * 1024)];
    if sniff.contains(&0) {
        return Err(ScanSkip::Binary);
    }
    let text = match String::from_utf8(bytes) {
        Ok(t) => t,
        Err(_) => return Err(ScanSkip::Binary),
    };
    *total_bytes += text.len() as u64;
    let mut hits = Vec::new();
    for (li, line) in text.split('\n').enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        for m in re.find_iter(line) {
            let col = crate::editor::buffer::byte_to_char(line, m.start());
            let end = crate::editor::buffer::byte_to_char(line, m.end());
            hits.push(SearchHit {
                path: path.to_path_buf(),
                line_no: li + 1,
                col,
                len: end - col,
                line_text: line.to_string(),
            });
        }
    }
    Ok(Some(hits))
}

/// 结果轮询间隔（报告 D2 批②口径——进度呈现 + 收结果的折中）。
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(120);

/// 命中点击 → 宿主打开文件（`path` + 1 基行号——宿主 open_document_tab
/// + jump_to_line 收口）。
pub type OpenHitHook = Arc<dyn Fn(&Path, usize, &mut Window, &mut App)>;

/// 全部替换请求 → 宿主确认 + 执行 + 善后（模式/替换串/选项随行——
/// 宿主侧拼预览计数并弹确认框）。
pub type ReplaceAllHook = Arc<dyn Fn(&str, &str, &SearchOptions, &mut Window, &mut App)>;

/// Esc 宿主回调（焦点还编辑器——FileTree/GitPanel 同语义）。
pub type EscapeHook = Arc<dyn Fn(&mut Window, &mut App)>;

/// 宿主面收口（lib 树纪律：不引 shell——依赖倒置，file_tree 同型）。
/// 渲染期捕获 Arc（clone 廉价——闭包共享）。
#[derive(Clone)]
pub struct SearchPanelHooks {
    pub open_hit: OpenHitHook,
    pub replace_all: ReplaceAllHook,
    pub escape: EscapeHook,
}

/// 面板态机（输入/选项/结果）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// 无查询（初始/清空）。
    Idle,
    /// 后台扫描中。
    Running,
    /// 结果就绪（含零命中——零命中是有效结果）。
    Done,
}

/// 单行输入框的模型（编辑器 FormInput 自绘同款的最小面——文本/选区/
/// 光标/IME marked；无历史无撤销——面板输入不需要）。
///
/// UTF16 索引选区（与 FormInput 同口径——IME 平台通道一致性）。
#[derive(Clone, Debug, Default)]
struct LineInput {
    text: String,
    anchor: usize,
    head: usize,
    marked: Option<std::ops::Range<usize>>,
}

impl LineInput {
    fn len_utf16(&self) -> usize {
        self.text.encode_utf16().count()
    }

    fn set_text(&mut self, text: &str) {
        self.text = text.to_owned();
        self.head = self.len_utf16();
        self.anchor = self.head;
        self.marked = None;
    }

    /// UTF16 偏移吸附到 char 边界（代理对不劈开）。
    fn boundary(&self, offset: usize, ceil: bool) -> usize {
        let mut u = 0;
        for ch in self.text.chars() {
            let end = u + ch.len_utf16();
            if offset < end {
                return if ceil && offset > u { end } else { u };
            }
            u = end;
        }
        u
    }

    /// UTF16 范围 → 字节范围（replace 用）。
    fn byte_range(&self, range: std::ops::Range<usize>) -> std::ops::Range<usize> {
        let a = self.boundary(range.start, false);
        let b = self.boundary(range.end, true);
        let byte_a = self
            .text
            .char_indices()
            .position(|_| {
                let _ = a;
                false
            })
            .unwrap_or(0);
        let _ = byte_a;
        let to_byte = |offset: usize| -> usize {
            let mut u = 0;
            for (b, ch) in self.text.char_indices() {
                if u >= offset {
                    return b;
                }
                u += ch.len_utf16();
            }
            self.text.len()
        };
        to_byte(a)..to_byte(b)
    }

    /// 替换（IME 提交/键入统一入口——range None = 选区或光标处插入）。
    fn replace(&mut self, range: Option<std::ops::Range<usize>>, text: &str) {
        let r = range.unwrap_or_else(|| {
            let a = self.anchor.min(self.head);
            let b = self.anchor.max(self.head);
            if a == b { a..a } else { a..b }
        });
        let bytes = self.byte_range(r.clone());
        self.text.replace_range(bytes, text);
        let new_head = r.start + text.encode_utf16().count();
        self.head = new_head;
        self.anchor = new_head;
        self.marked = None;
    }

    /// IME 预编辑（replace_and_mark 同语义——FormInput::mark 对齐）。
    fn mark(
        &mut self,
        range: Option<std::ops::Range<usize>>,
        new_text: &str,
        new_selected: Option<std::ops::Range<usize>>,
    ) {
        let base = range.or_else(|| self.marked.clone()).unwrap_or_else(|| {
            let a = self.anchor.min(self.head);
            a..a
        });
        let bytes = self.byte_range(base.clone());
        self.text.replace_range(bytes, new_text);
        let start = base.start;
        self.marked = Some(start..start + new_text.chars().map(|c| c.len_utf16()).sum::<usize>());
        if let Some(sel) = new_selected {
            let a = self.boundary(start + sel.start, false);
            let b = self.boundary(start + sel.end, true);
            self.anchor = a;
            self.head = b;
        } else {
            self.head = self.marked.as_ref().map(|m| m.end).unwrap_or(start);
            self.anchor = self.head;
        }
    }

    fn select(&mut self, range: std::ops::Range<usize>) {
        let a = self.boundary(range.start, false);
        let b = self.boundary(range.end, true);
        (self.anchor, self.head) = if range.start > range.end {
            (a, b)
        } else if range.start > range.end {
            (b, a)
        } else {
            (a, b)
        };
    }

    /// 普通键编辑（字母/退格/删除/Home/End/左右——Enter/Escape/Tab
    /// 等功能键由面板层收，不入这里；返回是否有文本变更）。
    fn key(&mut self, ks: &gpui::Keystroke) -> bool {
        let ctrl = ks.modifiers.control || ks.modifiers.platform;
        let selection = self.anchor.min(self.head)..self.anchor.max(self.head);
        if ctrl && ks.key.eq_ignore_ascii_case("a") {
            self.select(0..self.len_utf16());
            return false;
        }
        if ctrl {
            return false;
        }
        match ks.key.as_str() {
            "backspace" | "delete" => {
                let r = if !selection.is_empty() {
                    selection
                } else if ks.key == "backspace" {
                    self.boundary(self.head.saturating_sub(1), false)..self.head
                } else {
                    self.head..self.boundary(self.head + 1, true)
                };
                let changed = !r.is_empty();
                self.replace(Some(r), "");
                changed
            }
            "left" | "right" => {
                let d = if ks.key == "left" { 1 } else { usize::MAX };
                let target = if d == 1 {
                    self.boundary(self.head.saturating_sub(1), false)
                } else {
                    self.boundary(self.head + 1, true)
                };
                self.anchor = target;
                self.head = target;
                false
            }
            "home" => {
                self.anchor = 0;
                self.head = 0;
                false
            }
            "end" => {
                let n = self.len_utf16();
                self.anchor = n;
                self.head = n;
                false
            }
            _ => {
                // 单字面文本键（含 CJK——IME 未启用平台直插）。
                if ks.key.chars().count() == 1 {
                    let ch = ks.key.chars().next().unwrap();
                    self.replace(None, &ch.to_string());
                    true
                } else {
                    false
                }
            }
        }
    }
}

/// 搜索面板实体。
pub struct SearchPanel {
    position: mdc_dock::DockPosition,
    /// 面板根焦点（dock 激活协议落点）。
    panel_focus: FocusHandle,
    /// 查询输入（单行自绘——见 [`LineInput`]）。
    query: LineInput,
    /// 替换输入（批③——空 + 非 replace_mode 时不显示行）。
    replacement: LineInput,
    /// 替换行展开态（面板内切换钮驱动；Enter 替换需展开）。
    replace_mode: bool,
    options: SearchOptions,
    /// 工作区根（搜索域——宿主打开工作区时经 [`Self::set_root`] 换绑）。
    root: Option<PathBuf>,
    phase: Phase,
    report: SearchReport,
    /// 后台扫描的失效号（新搜索/输入变更自增——旧结果落地前对账）。
    search_id: Arc<AtomicU64>,
    /// 取消旗（True = 在途扫描早退）。
    cancel: Arc<AtomicBool>,
    /// 后台已扫文件数（轮询期呈现——进度面的轻量数据）。
    scanned: Arc<AtomicU64>,
    hooks: SearchPanelHooks,
    /// 结果体滚动句柄（点击跳行后焦点回面板用——不自动滚）。
    scroll: ScrollHandle,
    /// 查询框焦点（激活面板时的缺省落点——Panel::activation_focus_handle）。
    query_focus: FocusHandle,
    /// 替换框焦点（replace_mode 展开后可聚焦）。
    replace_focus: FocusHandle,
    /// 非法正则等扫描错误（Done 态呈现——清空查询/新扫描时重置）。
    error: Option<String>,
}

impl SearchPanel {
    /// 构造（无根 Idle 态——宿主开工作区后 `set_root`）。
    pub fn new(hooks: SearchPanelHooks, cx: &mut Context<Self>) -> Self {
        Self {
            position: mdc_dock::DockPosition::Left,
            panel_focus: cx.focus_handle(),
            query_focus: cx.focus_handle(),
            replace_focus: cx.focus_handle(),
            query: LineInput::default(),
            replacement: LineInput::default(),
            replace_mode: false,
            options: SearchOptions::default(),
            root: None,
            phase: Phase::Idle,
            report: SearchReport::default(),
            search_id: Arc::new(AtomicU64::new(0)),
            cancel: Arc::new(AtomicBool::new(false)),
            scanned: Arc::new(AtomicU64::new(0)),
            hooks,
            scroll: ScrollHandle::new(),
            error: None,
        }
    }

    /// 工作区换绑（打开/更换工作区——旧结果清空回 Idle）。
    pub fn set_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        self.root = root;
        self.report = SearchReport::default();
        self.error = None;
        self.phase = Phase::Idle;
        // 在途扫描失效（root 变了——旧域结果不落地）。
        self.search_id.fetch_add(1, Ordering::SeqCst);
        self.cancel.store(true, Ordering::SeqCst);
        cx.notify();
    }

    /// 只读面（测试断言用——结果分组/计数）。
    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn hit_count(&self) -> usize {
        self.report.hits.len()
    }

    pub fn hits(&self) -> &[SearchHit] {
        &self.report.hits
    }

    /// 结果分组序（文件 → 该文件命中——渲染同序；测试断言同源）。
    pub fn grouped(&self) -> Vec<(&Path, &[SearchHit])> {
        let mut out: Vec<(&Path, &[SearchHit])> = Vec::new();
        let mut i = 0;
        while i < self.report.hits.len() {
            let path = self.report.hits[i].path.as_path();
            let start = i;
            while i < self.report.hits.len() && self.report.hits[i].path.as_path() == path {
                i += 1;
            }
            out.push((path, &self.report.hits[start..i]));
        }
        out
    }
    /// 大小写敏感切换（Aa 钮——立即重扫）。
    pub fn toggle_case_sensitive(&mut self, cx: &mut Context<Self>) {
        self.options.case_sensitive = !self.options.case_sensitive;
        self.rescan(cx);
    }

    /// 全词切换（[ab] 钮——立即重扫）。
    pub fn toggle_whole_word(&mut self, cx: &mut Context<Self>) {
        self.options.whole_word = !self.options.whole_word;
        self.rescan(cx);
    }

    /// 替换执行后的结果重扫（宿主善后链尾调用——面板内容自刷新）。
    pub fn rerun_after_replace(&mut self, cx: &mut Context<Self>) {
        self.rescan(cx);
    }

    /// 替换行展开切换（右上钮）。
    pub fn toggle_replace_mode(&mut self, cx: &mut Context<Self>) {
        self.replace_mode = !self.replace_mode;
        cx.notify();
    }

    /// Enter（查询框）：重扫。替换框：发起全部替换（经宿主钩子——
    /// 确认/执行/善后在 shell 侧）。
    pub fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.replacement_focus_active(window) {
            let pattern = self.query.text.clone();
            let replacement = self.replacement.text.clone();
            let options = self.options.clone();
            if !pattern.is_empty() && self.root.is_some() {
                (self.hooks.replace_all)(&pattern, &replacement, &options, window, cx);
            }
            return;
        }
        self.rescan(cx);
    }

    /// 重扫（输入变更/选项切换/Enter 共用——search_id 自增 + 后台spawn）。
    fn rescan(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let pattern = self.query.text.clone();
        if pattern.is_empty() {
            self.phase = Phase::Idle;
            self.report = SearchReport::default();
            self.error = None;
            cx.notify();
            return;
        }
        // 失效旧扫描 + 旧取消旗退役（新旗新号）。
        self.cancel.store(true, Ordering::SeqCst);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        let scanned = Arc::new(AtomicU64::new(0));
        self.scanned = scanned.clone();
        let id = self.search_id.fetch_add(1, Ordering::SeqCst) + 1;
        self.phase = Phase::Running;
        self.error = None;
        cx.notify();

        let options = self.options.clone();
        // 后台执行（collect + 扫描全离主线程；取消旗每文件间隙查 +
        // 进度计数——见 run_search）。
        let handle = cx
            .background_executor()
            .spawn(async move { run_search(&root, &pattern, &options, &cancel, &scanned) });
        // 结果共享槽（后台写 / 轮询读——Task 无 is_finished 面，完成
        // 经槽位 Option 标记；120ms 轮询 OCR 先例同型）。
        let slot = Arc::new(Mutex::new(None));
        let slot_writer = slot.clone();
        cx.spawn(async move |_, cx| {
            let result = handle.await;
            // 落槽即可——轮询链收（写槽不需要 app 上下文，cx 仅保活链）。
            let _ = cx;
            *slot_writer.lock().unwrap() = Some(result);
        })
        .detach();
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_this, cx| {
            poll_loop(weak, &slot, id, cx).await;
        })
        .detach();
    }

    /// 输入键分发（查询/替换两框——文本键/编辑键入模型，Enter/Esc/
    /// Tab 功能键面板层收）。
    fn input_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let in_replace = self.replacement_focus_active(window);
        let input = if in_replace {
            &mut self.replacement
        } else {
            &mut self.query
        };
        let ks = &event.keystroke;
        let text_changed = input.key(ks);
        if !in_replace && text_changed {
            self.rescan(cx);
        } else {
            cx.notify();
        }
    }

    /// 替换框是否持焦（焦点面：查询/替换两框各持句柄——渲染期 track_focus）。
    fn replacement_focus_active(&self, window: &Window) -> bool {
        self.replace_focus.is_focused(window)
    }
}

// =============================================================================
// 后台轮询收口（rescan 的另一半——120ms 周期查收/失效/取消/落地）
// =============================================================================

/// 轮询循环（rescan 起链）：每 POLL_INTERVAL 查三态——
/// - 面板 search_id 已前进（用户又输入/换根）→ 旧链终止（新链在跑）；
/// - 取消旗置位 → 终止；
/// - 后台完成（JoinHandle 内部 Future 已 ready）→ id 门通过后落地
///   报告回 Done（非法正则错误也在此落地——Err 呈现为错误行）。
async fn poll_loop(
    this: WeakEntity<SearchPanel>,
    slot: &Arc<Mutex<Option<Result<SearchReport, String>>>>,
    id: u64,
    cx: &mut AsyncApp,
) {
    loop {
        cx.background_executor().timer(POLL_INTERVAL).await;
        // 存活 + 失效门合一（WeakEntity::update 返回 Result——释放即 Err）：
        // 面板 search_id 已前进 = 用户又输入/换根——旧链终止，新链在跑。
        let stale = this
            .update(cx, |panel, _| panel.search_id.load(Ordering::SeqCst) != id)
            .unwrap_or(true);
        if stale {
            return;
        }
        // 完成探测（共享槽 Some = 后台已写——未完继续下一周期）。
        let result = slot.lock().unwrap().take();
        let Some(result) = result else {
            continue;
        };
        // 落地前最后一道 id 门（扫描期间可能又 rescan 了）。
        let _ = this.update(cx, |panel, cx| {
            if panel.search_id.load(Ordering::SeqCst) != id {
                return;
            }
            match result {
                Ok(report) => {
                    panel.report = report;
                    panel.phase = Phase::Done;
                }
                Err(message) => {
                    panel.error = Some(message);
                    panel.phase = Phase::Done;
                }
            }
            cx.notify();
        });
        return;
    }
}

// =============================================================================
// Panel trait / Render / EntityInputHandler（dock 协议 + 呈现 + IME）
// =============================================================================

impl gpui::EventEmitter<mdc_dock::PanelEvent> for SearchPanel {}

impl gpui::Focusable for SearchPanel {
    fn focus_handle(&self, _cx: &gpui::App) -> gpui::FocusHandle {
        self.panel_focus.clone()
    }
}

impl mdc_dock::Panel for SearchPanel {
    fn persistent_name() -> &'static str {
        SEARCH_PANEL_KEY
    }

    fn panel_key() -> &'static str {
        SEARCH_PANEL_KEY
    }

    /// 激活焦点 = 查询框（过滤面优先——蓝本 dock.rs:44 覆写语义）。
    fn activation_focus_handle(&self, _cx: &App) -> FocusHandle {
        self.query_focus.clone()
    }

    fn position(&self) -> mdc_dock::DockPosition {
        self.position
    }

    fn position_is_valid(&self, position: mdc_dock::DockPosition) -> bool {
        // 与 FileTree/GitPanel 同口径：左栏域面板（左/右可换侧）。
        matches!(
            position,
            mdc_dock::DockPosition::Left | mdc_dock::DockPosition::Right
        )
    }

    fn set_position(&mut self, position: mdc_dock::DockPosition, _cx: &mut Context<Self>) {
        self.position = position;
    }
}

impl SearchPanel {
    /// 输入框元素（自绘单行——find_bar 同款视觉：光标/选区/文本/IME
    /// canvas。宽度自适应：flex_1）。
    fn render_line_input(
        &self,
        which: InputField,
        id: &'static str,
        placeholder: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (input, focus): (&LineInput, &FocusHandle) = match which {
            InputField::Query => (&self.query, &self.query_focus),
            InputField::Replacement => (&self.replacement, &self.replace_focus),
        };
        let focused = focus.is_focused(window);
        let colors = cx.theme().colors().clone();
        // 光标/选区定位（shape_text_width——编辑器同款；面板自建样式）。
        let style = panel_input_style(&colors);
        let sel = input.anchor.min(input.head)..input.anchor.max(input.head);
        let text_a =
            crate::editor::shape_text_width(window, &utf16_prefix(input, sel.start), &style);
        let text_b = crate::editor::shape_text_width(window, &utf16_prefix(input, sel.end), &style);
        let caret =
            crate::editor::shape_text_width(window, &utf16_prefix(input, input.head), &style);
        let entity = cx.entity();
        let focus_clone = focus.clone();
        let which_clone = which;
        let show_text = if input.text.is_empty() && !focused {
            SharedString::from(placeholder)
        } else {
            SharedString::from(input.text.as_str())
        };
        let marked = input.marked.clone();

        div()
            .id(id)
            .track_focus(focus)
            .flex_1()
            .min_w_0()
            .h(px(28.))
            .px_1()
            .rounded_sm()
            .bg(colors.element_active)
            .overflow_hidden()
            .relative()
            .on_key_down(
                cx.listener(move |this, event: &gpui::KeyDownEvent, window, cx| {
                    this.input_key(event, window, cx);
                }),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _: &gpui::MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    // 点击聚焦（查询/替换两框路由）。
                    let handle = match which_clone {
                        InputField::Query => this.query_focus.clone(),
                        InputField::Replacement => this.replace_focus.clone(),
                    };
                    window.focus(&handle, cx);
                    cx.notify();
                }),
            )
            // 选区高亮（聚焦且有选区时）。
            .when(focused && !sel.is_empty(), |el| {
                el.child(
                    div()
                        .absolute()
                        .top(px(5.))
                        .left(text_a)
                        .w(text_b - text_a)
                        .h(px(16.))
                        .bg(colors.element_selected),
                )
            })
            // 文本（占位灰——未聚焦且空时）。
            .child(
                div()
                    .absolute()
                    .top(px(5.))
                    .text_size(px(13.))
                    .line_height(px(16.))
                    .font_family(EDITOR_FONT)
                    .when(input.text.is_empty() && !focused, |el| {
                        el.text_color(colors.text_muted)
                    })
                    .when(!input.text.is_empty(), |el| el.text_color(colors.text))
                    .child(show_text)
                    // IME 预编辑下划线（marked 区间）。
                    .when(marked.is_some(), |el| {
                        el.child(
                            div()
                                .absolute()
                                .top(px(5.))
                                .left(px(0.))
                                .w_full()
                                .h(px(1.))
                                .bg(colors.text),
                        )
                    }),
            )
            // 光标（聚焦时 1px 竖线）。
            .when(focused, |el| {
                el.child(
                    div()
                        .absolute()
                        .top(px(5.))
                        .left(caret)
                        .w(px(1.))
                        .h(px(16.))
                        .bg(colors.text),
                )
            })
            // IME 接线（paint 期 handle_input——编辑器 ime-hook 同模式）。
            .child(
                canvas(
                    move |bounds: Bounds<Pixels>, _, _cx| bounds,
                    move |bounds: Bounds<Pixels>, _, window: &mut Window, cx: &mut App| {
                        window.handle_input(
                            &focus_clone,
                            ElementInputHandler::new(bounds, entity.clone()),
                            cx,
                        );
                    },
                )
                .absolute()
                .size_full(),
            )
    }

    /// 状态行文案（Running 进度 / Done 计数 / 截断与跳过标记）。
    fn status_line(&self) -> String {
        match self.phase {
            Phase::Idle => String::new(),
            Phase::Running => {
                let n = self.scanned.load(Ordering::Relaxed);
                format!("搜索中…（已扫 {n} 文件）")
            }
            Phase::Done => {
                if let Some(err) = &self.error {
                    return err.clone();
                }
                let r = &self.report;
                let mut parts = vec![format!(
                    "{} 个文件中的 {} 处结果",
                    r.hits
                        .iter()
                        .map(|h| h.path.as_path())
                        .collect::<std::collections::HashSet<_>>()
                        .len(),
                    r.hits.len()
                )];
                if r.truncated {
                    parts.push("已达 500 上限（收窄模式）".into());
                }
                if r.skipped_big > 0 {
                    parts.push(format!("{} 个超大文件跳过", r.skipped_big));
                }
                if r.skipped_binary > 0 {
                    parts.push(format!("{} 个二进制跳过", r.skipped_binary));
                }
                if r.truncated_files {
                    parts.push("文件数达上限截断".into());
                }
                parts.join(" · ")
            }
        }
    }
}

/// 输入框位（查询/替换——焦点路由与渲染取数共用）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum InputField {
    Query,
    Replacement,
}

/// UTF16 前缀取文本（光标/选区 shaping 用——面板输入的 UTF16 语义对齐）。
fn utf16_prefix(input: &LineInput, upto: usize) -> String {
    let mut out = String::new();
    let mut u: usize = 0;
    for ch in input.text.chars() {
        if u >= upto {
            break;
        }
        out.push(ch);
        u += ch.len_utf16();
    }
    out
}

/// 面板输入 TextStyle（find_text_style 同规格自建——私面不跨模块导）。
fn panel_input_style(colors: &ThemeColors) -> gpui::TextStyle {
    gpui::TextStyle {
        color: colors.text,
        font_family: "Sarasa Mono SC".into(),
        font_features: Default::default(),
        font_fallbacks: None,
        font_size: px(13.).into(),
        line_height: px(16.).into(),
        font_weight: gpui::FontWeight::default(),
        font_style: gpui::FontStyle::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
        white_space: Default::default(),
        text_overflow: None,
        text_align: Default::default(),
        line_clamp: None,
    }
}

impl gpui::Render for SearchPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let hooks = self.hooks.clone();
        let status = self.status_line();
        let case_on = self.options.case_sensitive;
        let word_on = self.options.whole_word;
        let replace_mode = self.replace_mode;
        let has_root = self.root.is_some();

        // 选项 chip（find_bar 同款视觉：激活高亮/未激活灰；cx.listener
        // 包面板回调——与 git_panel 行点击同型）。
        let chip = |id: &'static str,
                    text: &'static str,
                    active: bool,
                    on_click: fn(&mut SearchPanel, &mut Context<SearchPanel>),
                    cx: &mut Context<Self>| {
            div()
                .id(id)
                .px_1()
                .rounded_sm()
                .text_size(px(11.))
                .line_height(px(16.))
                .font_family(EDITOR_FONT)
                .when(active, |el| {
                    el.bg(colors.element_active).text_color(colors.text)
                })
                .when(!active, |el| el.text_color(colors.text_muted))
                .hover(|el| el.bg(colors.element_hover))
                .child(text)
                .on_click(cx.listener(move |this, _, _, cx| {
                    on_click(this, cx);
                }))
        };

        let grouped = self.grouped();
        let root = self.root.clone();

        v_flex()
            .id("search-panel")
            .flex_1()
            .min_h_0()
            .size_full()
            .bg(colors.panel_background)
            .p_2()
            .gap_1()
            // 未开工作区：引导文案（与 git 面板同口径）。
            .when(!has_root, |el| {
                el.child(
                    Label::new("未打开工作区（Ctrl+K Ctrl+O）")
                        .size(LabelSize::Small)
                        .color(ui::Color::Muted),
                )
            })
            .when(has_root, |el| {
                el.child(
                    List::new()
                        .header(
                            ListHeader::new("搜索").toggle(true).start_slot(
                                Icon::new(IconName::MagnifyingGlass)
                                    .size(IconSize::XSmall)
                                    .color(ui::Color::Muted),
                            ),
                        )
                        // 查询行（输入 + 选项 chip）。
                        .child(
                            h_flex()
                                .id("search-query-row")
                                .w_full()
                                .gap_1()
                                .child(self.render_line_input(
                                    InputField::Query,
                                    "search-query-input",
                                    "搜索（支持正则）",
                                    _window,
                                    cx,
                                ))
                                .child(chip(
                                    "search-case-chip",
                                    "Aa",
                                    case_on,
                                    SearchPanel::toggle_case_sensitive,
                                    cx,
                                ))
                                .child(chip(
                                    "search-word-chip",
                                    "[ab]",
                                    word_on,
                                    SearchPanel::toggle_whole_word,
                                    cx,
                                )),
                        )
                        // 替换行（replace_mode 展开时）。
                        .when(replace_mode, |el| {
                            el.child(
                                h_flex().id("search-replace-row").w_full().gap_1().child(
                                    self.render_line_input(
                                        InputField::Replacement,
                                        "search-replace-input",
                                        "替换为",
                                        _window,
                                        cx,
                                    )
                                    .when(false, |el| el),
                                ),
                            )
                        })
                        // 替换展开切换钮（左侧箭头形）。
                        .child(
                            div()
                                .id("search-replace-toggle")
                                .text_size(px(11.))
                                .text_color(colors.text_muted)
                                .hover(|el| el.bg(colors.element_hover))
                                .child(if replace_mode {
                                    "▾ 收起替换"
                                } else {
                                    "▸ 展开替换"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.toggle_replace_mode(cx);
                                })),
                        )
                        // 状态行。
                        .when(!status.is_empty(), |el| {
                            el.child(
                                Label::new(status)
                                    .size(LabelSize::XSmall)
                                    .color(ui::Color::Muted),
                            )
                        })
                        // 结果体：文件分组（文件头 + 命中行）。
                        .child(
                            v_flex()
                                .id("search-results")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .track_scroll(&self.scroll)
                                .children(grouped.iter().map(|(path, hits)| {
                                    let rel = root
                                        .as_ref()
                                        .and_then(|r| path.strip_prefix(r).ok())
                                        .map(|p| p.display().to_string())
                                        .unwrap_or_else(|| path.display().to_string());
                                    let open = hooks.open_hit.clone();
                                    v_flex()
                                        .id(SharedString::from(format!(
                                            "search-group-{}",
                                            path.display()
                                        )))
                                        .gap_0()
                                        // 文件头（相对路径 + 计数——点击文件头开文件首命中）。
                                        .child(
                                            div()
                                                .id(SharedString::from(format!(
                                                    "search-file-{}",
                                                    path.display()
                                                )))
                                                .py_1()
                                                .text_size(px(11.))
                                                .font_family(EDITOR_FONT)
                                                .text_color(colors.text_muted)
                                                .child(format!("{} ({})", rel, hits.len()))
                                                .on_click({
                                                    let path = (*path).to_path_buf();
                                                    let line = hits
                                                        .first()
                                                        .map(|h| h.line_no)
                                                        .unwrap_or(1);
                                                    move |_, window, cx| {
                                                        open(&path, line, window, cx);
                                                    }
                                                }),
                                        )
                                        // 命中行（1 基行号 + 整行原文——点击跳行）。
                                        .children(hits.iter().map(|hit| {
                                            let open = hooks.open_hit.clone();
                                            let path = hit.path.clone();
                                            let line = hit.line_no;
                                            let line_text = hit.line_text.clone();
                                            div()
                                                .id(SharedString::from(format!(
                                                    "search-hit-{}-{}-{}",
                                                    path.display(),
                                                    line,
                                                    hit.col
                                                )))
                                                .pl_2()
                                                .pr_1()
                                                .py(px(1.))
                                                .text_size(px(12.))
                                                .font_family(EDITOR_FONT)
                                                .text_color(colors.text)
                                                .hover(|el| el.bg(colors.element_hover))
                                                .child(
                                                    h_flex()
                                                        .id("search-hit-row")
                                                        .gap_1()
                                                        .overflow_hidden()
                                                        .child(
                                                            Label::new(line.to_string())
                                                                .size(LabelSize::XSmall)
                                                                .color(ui::Color::Muted),
                                                        )
                                                        .child(
                                                            div()
                                                                .flex_1()
                                                                .min_w_0()
                                                                .overflow_hidden()
                                                                .text_ellipsis()
                                                                .child(SharedString::from(
                                                                    line_text,
                                                                )),
                                                        ),
                                                )
                                                .on_click(move |_, window, cx| {
                                                    open(&path, line, window, cx);
                                                })
                                        }))
                                })),
                        ),
                )
            })
    }
}

impl gpui::EntityInputHandler for SearchPanel {
    fn set_selected_text_range(
        &mut self,
        range: std::ops::Range<usize>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        let (input, _) = self.active_field_mut(window);
        input.select(range);
    }

    fn text_length_utf16(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        let (input, _) = self.active_field_mut(window);
        Some(input.len_utf16())
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        true
    }

    fn text_for_range(
        &mut self,
        range: std::ops::Range<usize>,
        adjusted: &mut Option<std::ops::Range<usize>>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let (input, _) = self.active_field_mut(window);
        let a = input.boundary(range.start, false);
        let b = input.boundary(range.end, true);
        *adjusted = Some(a..b);
        Some(utf16_slice(input, a..b))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled: bool,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<gpui::UTF16Selection> {
        let (input, _) = self.active_field_mut(window);
        Some(gpui::UTF16Selection {
            range: input.anchor.min(input.head)..input.anchor.max(input.head),
            reversed: input.head < input.anchor,
        })
    }

    fn marked_text_range(
        &self,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        let input = if self.replace_focus.is_focused(window) {
            &self.replacement
        } else {
            &self.query
        };
        input.marked.clone()
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (input, _) = self.active_field_mut(window);
        input.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<std::ops::Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let in_replace = self.replacement_focus_active(window);
        let (input, _) = self.active_field_mut(window);
        input.replace(range, text);
        if !in_replace {
            self.rescan(cx);
        } else {
            cx.notify();
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<std::ops::Range<usize>>,
        new_text: &str,
        new_selected_range: Option<std::ops::Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let in_replace = self.replacement_focus_active(window);
        let (input, _) = self.active_field_mut(window);
        input.mark(range, new_text, new_selected_range);
        if !in_replace {
            self.rescan(cx);
        } else {
            cx.notify();
        }
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _element_bounds: gpui::Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<gpui::Bounds<Pixels>> {
        None
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl SearchPanel {
    /// 当前聚焦的输入框（查询/替换——IME 通道取数；无聚焦退查询）。
    fn active_field_mut(&mut self, window: &mut Window) -> (&mut LineInput, InputField) {
        if self.replace_focus.is_focused(window) {
            (&mut self.replacement, InputField::Replacement)
        } else {
            (&mut self.query, InputField::Query)
        }
    }
}

/// UTF16 区间取文本。
fn utf16_slice(input: &LineInput, range: std::ops::Range<usize>) -> String {
    let mut out = String::new();
    let mut u: usize = 0;
    for ch in input.text.chars() {
        let end = u + ch.len_utf16();
        if end > range.start && u < range.end {
            out.push(ch);
        }
        u = end;
    }
    out
}

// =============================================================================
// 单元测试（#109 批②③：面板实体——合成树 + capturing hooks 无头形态）
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn capturing_hooks(
        opened: Arc<Mutex<Vec<(PathBuf, usize)>>>,
        replaced: Arc<Mutex<Vec<String>>>,
    ) -> SearchPanelHooks {
        SearchPanelHooks {
            open_hit: Arc::new(move |path, line, _, _| {
                opened.lock().unwrap().push((path.to_path_buf(), line));
            }),
            replace_all: Arc::new(move |pattern, replacement, _, _, _| {
                replaced
                    .lock()
                    .unwrap()
                    .push(format!("{pattern}→{replacement}"));
            }),
            escape: Arc::new(|_, _| {}),
        }
    }

    fn temp_tree(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mdc-109b3-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 面板 + 宿主窗口（主题 init + 实体挂 Host——git_panel 同款）。
    fn panel_window(
        cx: &mut gpui::TestAppContext,
        root: &Path,
    ) -> (
        gpui::Entity<SearchPanel>,
        gpui::VisualTestContext,
        Arc<Mutex<Vec<(PathBuf, usize)>>>,
        Arc<Mutex<Vec<String>>>,
    ) {
        cx.update(|app| {
            theme::init(theme::LoadThemes::JustBase, app);
            theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
        });
        let opened: Arc<Mutex<Vec<(PathBuf, usize)>>> = Arc::default();
        let replaced: Arc<Mutex<Vec<String>>> = Arc::default();
        let hooks = capturing_hooks(opened.clone(), replaced.clone());
        let root = root.to_path_buf();
        let panel = cx.update(|cx| {
            cx.new(|cx| {
                let mut p = SearchPanel::new(hooks, cx);
                p.set_root(Some(root), cx);
                p
            })
        });
        struct Host(gpui::Entity<SearchPanel>);
        impl gpui::Render for Host {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
                v_flex().size_full().child(self.0.clone())
            }
        }
        let window = cx.open_window(gpui::size(gpui::px(300.), gpui::px(500.)), {
            let panel = panel.clone();
            move |_, _| Host(panel)
        });
        cx.run_until_parked();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        (panel, visual, opened, replaced)
    }

    /// 等待扫描落地（轮询链收口——draw + run_until_parked + 虚拟时钟
    /// 推进；pdf_thumbs 测试同款 advance_clock 先例，多轮确保 120ms
    /// 周期跨过）。
    fn settle(cx: &mut gpui::TestAppContext, visual: &mut gpui::VisualTestContext) {
        for _ in 0..30 {
            visual.update(|window, cx| window.draw(cx).clear(cx));
            visual.run_until_parked();
            cx.background_executor
                .advance_clock(std::time::Duration::from_millis(50));
        }
    }

    #[gpui::test]
    fn search_lands_hits_grouped_by_file(cx: &mut gpui::TestAppContext) {
        let root = temp_tree("group");
        std::fs::write(root.join("a.md"), "alpha beta\nplain\n").unwrap();
        std::fs::write(root.join("b.txt"), "beta tail beta\n").unwrap();
        let (panel, mut visual, _, _) = panel_window(cx, &root);

        // 输入查询（键模型直驱 + 后台落地——绕开真实键盘合成）。
        panel.update(&mut visual, |p, cx| {
            p.query.set_text("beta");
            p.rescan(cx);
        });
        settle(cx, &mut visual);

        panel.read_with(&visual, |p, _| {
            assert_eq!(p.phase(), Phase::Done, "扫描完成落地");
            assert_eq!(p.hit_count(), 3, "三处命中（a 一 + b 两）");
            // 分组断言不依赖目录序（read_dir 原始序平台相关——只断
            // 集合与组内计数、同文件命中相邻）。
            let groups = p.grouped();
            assert_eq!(groups.len(), 2, "两文件分组");
            for (path, hits) in &groups {
                let expect = if path.ends_with("a.md") { 1 } else { 2 };
                assert_eq!(
                    hits.len(),
                    expect,
                    "{path:?} 组内计数（同文件命中相邻分组）"
                );
                assert!(hits.iter().all(|h| h.path.as_path() == *path), "组内同文件");
            }
            // a.md 的首命中定位（1 基行号 / 0 基 char 列 / char 长）。
            let a_hit = p
                .hits()
                .iter()
                .find(|h| h.path.ends_with("a.md"))
                .expect("a.md 命中在案");
            assert_eq!(a_hit.line_no, 1, "1 基行号");
            assert_eq!(a_hit.col, 6, "0 基 char 列");
            assert_eq!(a_hit.len, 4);
            assert_eq!(a_hit.line_text, "alpha beta", "整行原文");
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    #[gpui::test]
    fn query_change_invalidates_stale_results(cx: &mut gpui::TestAppContext) {
        let root = temp_tree("stale");
        std::fs::write(root.join("a.md"), "foo\nfoobar\n").unwrap();
        let (panel, mut visual, _, _) = panel_window(cx, &root);

        panel.update(&mut visual, |p, cx| {
            p.query.set_text("foo");
            p.rescan(cx);
        });
        // 第一个扫描在途时改查询（旧 id 失效——旧结果不落地）。
        panel.update(&mut visual, |p, cx| {
            p.query.set_text("bar");
            p.rescan(cx);
        });
        settle(cx, &mut visual);

        panel.read_with(&visual, |p, _| {
            assert_eq!(p.phase(), Phase::Done);
            // 「bar」只在 foobar 行命中一处——旧「foo」两处的结果被弃。
            assert_eq!(p.hit_count(), 1, "新查询结果（旧结果失效不落地）");
            assert!(p.hits()[0].line_text.contains("foobar"));
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    #[gpui::test]
    fn invalid_regex_lands_error_not_silent(cx: &mut gpui::TestAppContext) {
        let root = temp_tree("badre");
        std::fs::write(root.join("a.md"), "x").unwrap();
        let (panel, mut visual, _, _) = panel_window(cx, &root);

        panel.update(&mut visual, |p, cx| {
            p.query.set_text("(unclosed");
            p.rescan(cx);
        });
        settle(cx, &mut visual);

        panel.read_with(&visual, |p, _| {
            assert_eq!(p.phase(), Phase::Done);
            assert!(
                p.error.as_ref().is_some_and(|e| e.contains("正则无效")),
                "非法正则呈错误（不静默）：{:?}",
                p.error
            );
            assert_eq!(p.hit_count(), 0);
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    #[gpui::test]
    fn line_input_edits_and_utf16_semantics() {
        let ks = |key: &str| gpui::Keystroke {
            modifiers: gpui::Modifiers::default(),
            key: key.into(),
            key_char: None,
        };
        let mut input = LineInput::default();
        input.replace(None, "查找中文");
        assert_eq!(input.text, "查找中文");
        assert_eq!(input.len_utf16(), 4, "CJK 每 char 一 UTF16 单元");
        // 键盘退格删「文」（边界吸附——char 不劈）。
        assert!(input.key(&ks("backspace")), "文本变更");
        assert_eq!(input.text, "查找中");
        // Home/End/左右（无文本变更 false——不触发重扫）。
        assert!(!input.key(&ks("home")));
        assert_eq!(input.head, 0);
        assert!(!input.key(&ks("end")));
        assert_eq!(input.head, 3);
        // 选区 + 删除整选。
        input.select(0..1);
        assert!(input.key(&ks("delete")));
        assert_eq!(input.text, "找中");
    }

    #[gpui::test]
    fn line_input_ime_mark_and_commit() {
        let mut input = LineInput::default();
        // 预编辑「zh」插入光标处（IME 起段——marked 区间在案）。
        input.mark(None, "zh", Some(0..0));
        assert_eq!(input.text, "zh");
        assert!(input.marked.is_some(), "预编辑区间在案");
        // 预编辑更新「中」（替换旧预编辑段——marked 持续）。
        input.mark(None, "中", None);
        assert_eq!(input.text, "中", "预编辑段原位替换");
        assert!(input.marked.is_some(), "预编辑持续（更新不清段）");
        // 提交（replace_text_in_range 通道——range=marked 提交文本）。
        let marked = input.marked.clone().unwrap();
        input.replace(Some(marked), "中");
        assert_eq!(input.text, "中", "提交文本落位");
        assert!(input.marked.is_none(), "提交后预编辑清空");
        assert_eq!(input.head, 1, "光标在提交文本尾");
    }

    #[gpui::test]
    fn toggle_options_flip_and_rescan(cx: &mut gpui::TestAppContext) {
        let root = temp_tree("opts");
        std::fs::write(root.join("a.md"), "Foo foo\n").unwrap();
        let (panel, mut visual, _, _) = panel_window(cx, &root);

        panel.update(&mut visual, |p, cx| {
            p.query.set_text("foo");
            p.rescan(cx);
        });
        settle(cx, &mut visual);
        panel.read_with(&visual, |p, _| {
            assert_eq!(p.hit_count(), 2, "缺省大小写不敏感两处");
        });

        // Aa 钮切敏感——立即重扫。
        panel.update(&mut visual, |p, cx| p.toggle_case_sensitive(cx));
        settle(cx, &mut visual);
        panel.read_with(&visual, |p, _| {
            assert_eq!(p.hit_count(), 1, "敏感一处");
            assert!(p.options.case_sensitive);
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    #[gpui::test]
    fn zero_hits_and_empty_query_are_valid_states(cx: &mut gpui::TestAppContext) {
        let root = temp_tree("zero");
        std::fs::write(root.join("a.md"), "nothing here").unwrap();
        let (panel, mut visual, _, _) = panel_window(cx, &root);

        panel.update(&mut visual, |p, cx| {
            p.query.set_text("zzz");
            p.rescan(cx);
        });
        settle(cx, &mut visual);
        panel.read_with(&visual, |p, _| {
            assert_eq!(p.phase(), Phase::Done, "零命中是有效结果");
            assert_eq!(p.hit_count(), 0);
        });

        // 清空查询 → Idle（无查询态）。
        panel.update(&mut visual, |p, cx| {
            p.query.set_text("");
            p.rescan(cx);
        });
        panel.read_with(&visual, |p, _| {
            assert_eq!(p.phase(), Phase::Idle);
            assert_eq!(p.hit_count(), 0);
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    #[gpui::test]
    fn replace_hook_carries_pattern_replacement_and_options(cx: &mut gpui::TestAppContext) {
        let root = temp_tree("confirm");
        std::fs::write(root.join("a.md"), "needle here\n").unwrap();
        let (panel, mut visual, _, replaced) = panel_window(cx, &root);

        panel.update(&mut visual, |p, cx| {
            p.query.set_text("needle");
            p.replacement.set_text("pin");
            p.replace_mode = true;
            p.rescan(cx);
        });
        settle(cx, &mut visual);

        // 确认路由的可测前置：钩子载荷（模式/替换串/选项）正确携带
        // ——窗口路由层（焦点判据 + menu::Confirm 分派）由真实事件驱动，
        // 此处直接调面板 confirm 的钩子载荷面（replace_focus 聚焦态
        // 判据以 replace_mode 已展开 + 替换串非空为准）。
        let replace_focus = panel.read_with(&visual, |p, _| p.replace_focus.clone());
        visual.update(|window, cx| replace_focus.focus(window, cx));
        visual.update(|window, cx| {
            window.draw(cx).clear(cx);
            // confirm 在 window 域内驱动（聚焦态判据 + 钩子收口同帧）。
            panel.update(cx, |p, cx| p.confirm(window, cx));
        });

        let got = replaced.lock().unwrap().clone();
        assert!(
            got.iter()
                .any(|s| s.contains("needle") && s.contains("pin")),
            "全部替换请求经宿主钩子（模式→替换串携带）：{got:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
