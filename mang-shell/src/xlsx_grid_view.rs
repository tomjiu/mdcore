//! XLSX 网格视图（#389 阶段 1——真网格渲染骨架；**阶段 2——单元格
//! 编辑接线**；**#555——合并单元格显示与命中**）。
//!
//! #555 面（合并单元格——`xlsx_merge_geometry` 纯函数层 + 本模块接线）：
//!
//! - 模型携带：`GridSheet::merge_spans`（绝对 `Dimensions` → 显示坐标
//!   + guard 裁剪——`from_book` 单点换算）；
//! - 绘制：合并区 = 一个宽格（跨行跨列单视觉格）——覆盖格不独立构建、
//!   内部网格线抑制；跨行 span 每行画一段，选中/编辑态四边框按行拼接
//!   （首行 top / 末行 bottom / 每行 left+right）；滚动只覆盖范围中部
//!   时绝对定位天然承载部分可见；
//! - 命中/编辑：点覆盖格 → `canonical_cell` 映射左上（Excel 语义——
//!   选中/编辑/`selected_a1` 全锚左上）；提交只写左上引用，**禁止复制
//!   值填满合并区域**（issue #555 明文禁令）。
//!
//! # 定位（issue #389 阶段 1 范围声明）
//!
//! 现状：桌面 xlsx = 只读**行模型**（`editor::xlsx_doc` 把表格拍平成
//! 文本行——引擎能改 / wasm 能改 / 桌面只能看）。本模块把显示面换成
//! **真网格**：列字母行 + 行号列边缘固定、单元格点选高亮、双向虚拟
//! 化滚动、工作表切换树保留；**阶段 2 起接编辑**：双击进入内联编辑
//! （键位直输——`keystroke.key_char`，编辑器同源无 IME 路径），
//! Enter 提交 → `XlsxEditor::set_cell` 落账（公式格拒改——引擎面
//! `FormulaCell` 错误明示），Ctrl+S / 菜单保存 → `save_to_vec` 写盘
//! （临时文件 + rename——docx 保存同纪律，写坏不毁原件），脏标记 +
//! 关闭确认由 shell 面统一拦截（`tab_is_modified` / `close_active_tab`）。
//!
//! ```text
//! ┌──────────────────────────────────────────────┐
//! │ [Sheet1] [Sheet2] …   （工作表切换——簿序）    │
//! ├───────┬──────────────────────────────────────┤
//! │corner │   A     B     C     …  （列头·顶部固定）│
//! ├───────┼──────────────────────────────────────┤
//! │   1   │ cell   cell   cell   （行号列·左固定） │
//! │   2   │ cell   …       （uniform_list 纵向虚拟化）│
//! │   ⋮   │     （可见列窗口——横向虚拟化）        │
//! └───────┴──────────────────────────────────────┘
//! ```
//!
//! # 虚拟化纪律（两轴）
//!
//! - **纵向**：`uniform_list`（GPUI 原生——只构建可见行）；
//! - **横向**：`scroll_x` 自管状态 + 可见列窗口（`visible_col_window`
//!   纯函数）——滚轮 deltaX / Shift+滚轮 deltaY 驱动（`clamp_scroll_x`
//!   钳制），窗口外列零构建；底部滚动指示条按 scroll_x / 总宽渲染。
//!
//! # 点选/编辑（命中测试）
//!
//! 覆盖 canvas 的 paint 期注册窗口级鼠标观察器（pdf_pages 同模式：
//! bounds 过滤 + 不占命中区不挡滚动）——`hit_test_cell` 纯函数把局部
//! 坐标 + 滚动状态解析为 (行, 列)，选中格高亮边框 + 行号/列头联动；
//! **双击**（`MouseDownEvent::click_count >= 2`）进入内联编辑，
//! 单击在编辑态时先提交当前编辑再选新格（Excel 同款语义）。
//!
//! # 数据面（单一来源：office-io 读面）
//!
//! `XlsxGridModel` = `io::read_book_with_options` 的网格化转译（与
//! `xlsx_doc::load` 同一读面、同一护栏口径：行 1000 / 列 64 / 单元格宽
//! 24）。编辑会话 = `mdc_office_io::xlsx_edit::XlsxEditor`（同一文件的
//! 独立打开——office-io 门面恒严格）；**不动 `xlsx_doc.rs`**（保留为
//! fallback / 文本视图——状态栏切换钮同 pdf 页面/文本模式先例）。
//!
//! # 已知边界（阶段 2 登记）
//!
//! - 公式格拒改（`set_cell` 引擎面 `FormulaCell`——重算留阶段 3，
//!   issue 明示「阶段 2 先做改值」）；
//! - 编辑限网格可见域（护栏内——越界格不可见亦不可达）；
//! - 文本视图（xlsx_doc 行模型）为打开时快照——保存后重开生效
//!   （两视图内容零漂移口径的已知例外，状态栏可切回网格视图看实况）；
//! - 键盘导航（方向键移选区）不在阶段 2 面——登记待后续打磨轮；
//! - 合并格显示 = 左上单点语义（与行模型同口径——真跨格渲染属阶段 3）；
//! - 超链接/数字格式面外（`CellData::display` 同行模型口径）；
//! - 沙箱无 GUI：GPUI 层编译面验证 + 逻辑面（模型/几何纯函数 + 夹具
//!   往返）全量真跑——`tests/xlsx_grid_model.rs`；真机滚动/点选体验
//!   待开发机（与 #302 pdf_pages 同纪律）。

use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, DispatchPhase, FocusHandle, Focusable, KeyDownEvent, Pixels,
    ScrollEvent, SharedString, UniformListScrollHandle, Window, px,
};
use mdc_office_io::io::{Book, CellData, ReadOptions, read_book_from_bytes_with_options};
use mdc_office_io::xlsx_edit::{CellEdit, XlsxEditor};
use ui::prelude::*;

use crate::xlsx_merge_geometry::{
    MergeSpan, canonical_cell, intersects_window, is_covered, span_anchor_abs, span_edges, span_px,
    spans_in_row,
};

// ---------------------------------------------------------------------------
// 常量（渲染口径——与 xlsx_doc.rs 护栏同源，不另立标准）
// ---------------------------------------------------------------------------

/// 每表渲染行上限（office-io LIMIT_TOTAL_CELLS 的行模型护栏同款）。
pub const MAX_RENDER_ROWS: usize = 1000;
/// 每表渲染列上限（同上）。
pub const MAX_RENDER_COLS: usize = 64;
/// 单元格显示宽上限（East Asian cell 计——超宽截断，列宽钳制吸收）。
pub const CELL_WIDTH_CAP: usize = 24;
/// 列头高（px）。
pub const HEAD_H: f32 = 24.0;
/// 行高（px——uniform_list 等高行的基准）。
pub const ROW_H: f32 = 22.0;
/// 行号槽宽（px）。
pub const GUTTER_W: f32 = 48.0;
/// 列宽下限 / 上限（px——内容自适应 + 域钳制；上限 = CELL_WIDTH_CAP
/// 内容的自然宽：24 cell × 7.5px + 双侧内边距 12px = 192px——两钳
/// 制面串联，上限与内容封顶同源不矛盾）。
pub const COL_W_MIN: f32 = 64.0;
pub const COL_W_MAX: f32 = 192.0;
/// 列间距（px——网格线代替）。
pub const COL_GAP: f32 = 1.0;
/// 单元格内边距（px）。
const CELL_PAD: f32 = 6.0;

// ---------------------------------------------------------------------------
// 数据模型（零 GPUI——全量可测）
// ---------------------------------------------------------------------------

/// 单表网格（显示文本矩阵 + 几何 + 护栏标记）。
#[derive(Clone, Debug, Default)]
pub struct GridSheet {
    /// 表名（簿序）。
    pub name: String,
    /// 数据区起始绝对坐标（0 基——office-io 包围盒）。
    pub start_row: u32,
    pub start_col: u32,
    /// 渲染行/列数（护栏后）。
    pub rows: usize,
    pub cols: usize,
    /// 行主序显示文本（`rows × cols`；Empty = 空串；公式格 = `=公式`）。
    pub cells: Vec<Vec<String>>,
    /// 触发渲染护栏（行 > 1000 或列 > 64——非静默截断标记）。
    pub truncated: bool,
    /// 合并范围数（显示 = 左上单点语义——office-io 原始计数；集成
    /// 测试 `grid_model_geometry_and_guards` 断言消费；bin 视图面读
    /// `merge_spans`——计数字段保留为「原始 vs 可渲染」口径分离面）。
    #[allow(dead_code)]
    pub merges: usize,
    /// #555：合并范围（显示坐标，guard 盒内裁剪后——`from_book` 由
    /// 绝对 `Dimensions` 换算；渲染/命中/编辑全走
    /// `xlsx_merge_geometry` 纯函数层）。
    pub merge_spans: Vec<MergeSpan>,
}

/// 网格载荷（office-io 读面 → 网格模型——单一来源）。
#[derive(Clone, Debug, Default)]
pub struct XlsxGridModel {
    pub sheets: Vec<GridSheet>,
    /// 触发护栏的表数（状态条提示用）。
    pub truncated_sheets: usize,
}

impl XlsxGridModel {
    /// 读盘 + 网格化（`document::open` 的 xlsx 分派同源读面；office-io
    /// 内建输入字节门 + 三门——本层不重复）。
    pub fn load(path: &Path) -> Result<Self, String> {
        let book = mdc_office_io::io::read_book_with_options(
            &path.to_string_lossy(),
            &ReadOptions { compat: true },
        )
        .map_err(|e| format!("xlsx 打开失败：{e}"))?;
        Ok(Self::from_book(&book))
    }

    /// Book → 网格模型（纯函数——转换规则锚点）。
    pub fn from_book(book: &Book) -> Self {
        let mut sheets = Vec::with_capacity(book.sheets.len());
        let mut truncated_sheets = 0;
        for sheet in &book.sheets {
            let grid = &sheet.typed;
            let Some((row0, col0)) = grid.start() else {
                // 空表：零行零列（网格面 = 空态提示；合并范围无渲染面
                // ——guard 盒零维，clip_to_box 全弃）。
                sheets.push(GridSheet {
                    name: sheet.name.clone(),
                    merges: sheet.merges.len(),
                    merge_spans: Vec::new(),
                    ..Default::default()
                });
                continue;
            };
            let height = grid.height();
            let width = grid.width();
            let rows = height.min(MAX_RENDER_ROWS);
            let cols = width.min(MAX_RENDER_COLS);
            let truncated = height > MAX_RENDER_ROWS || width > MAX_RENDER_COLS;
            if truncated {
                truncated_sheets += 1;
            }
            // 显示文本（cell_display 口径 = xlsx_doc 行模型同源：公式
            // 回退 `=公式`、Empty = 空串——两视图内容零漂移）。
            let cells = (0..rows)
                .map(|r| {
                    (0..cols)
                        .map(|c| {
                            let cell = grid.get_abs(row0 + r as u32, col0 + c as u32);
                            let formula = sheet.formula_at(row0 + r as u32, col0 + c as u32);
                            cell_display(cell, formula)
                        })
                        .collect()
                })
                .collect();
            // #555：合并范围携带（绝对 → 显示坐标 + guard 盒裁剪——
            // 部分在护栏内的保留可见段，整段在外丢弃；计数 `merges`
            // 仍记原始值——「有但裁掉」与「没有」语义可区分）。
            let merge_spans = sheet
                .merges
                .iter()
                .filter_map(|d| {
                    MergeSpan::clip_to_box(
                        (d.start.0, d.start.1),
                        (d.end.0, d.end.1),
                        row0,
                        col0,
                        rows,
                        cols,
                    )
                })
                .collect();
            sheets.push(GridSheet {
                name: sheet.name.clone(),
                start_row: row0,
                start_col: col0,
                rows,
                cols,
                cells,
                truncated,
                merges: sheet.merges.len(),
                merge_spans,
            });
        }
        Self {
            sheets,
            truncated_sheets,
        }
    }
}

/// 单元格显示文本（`xlsx_doc::cell_display` 同口径——两视图一致）。
fn cell_display(cell: Option<&CellData>, formula: Option<&str>) -> String {
    match (cell, formula) {
        (Some(CellData::Empty), Some(f)) => format!("={f}"),
        (Some(c), _) => c.display().to_string(),
        (None, Some(f)) => format!("={f}"),
        (None, None) => String::new(),
    }
}

// 合并格值与可见覆盖格分离；仅视图内部使用，不改变公开模型字段。
type MergeValueCache = BTreeMap<(usize, u32, u32), String>;

fn value_at_absolute(sheet: &GridSheet, anchor: (u32, u32)) -> String {
    let Some(row) = anchor.0.checked_sub(sheet.start_row) else {
        return String::new();
    };
    let Some(col) = anchor.1.checked_sub(sheet.start_col) else {
        return String::new();
    };
    sheet
        .cells
        .get(row as usize)
        .and_then(|r| r.get(col as usize))
        .cloned()
        .unwrap_or_default()
}

fn initial_merge_values(model: &XlsxGridModel) -> MergeValueCache {
    let mut values = MergeValueCache::new();
    for (index, sheet) in model.sheets.iter().enumerate() {
        for span in &sheet.merge_spans {
            values.insert(
                (index, span.abs_anchor.0, span.abs_anchor.1),
                value_at_absolute(sheet, span.abs_anchor),
            );
        }
    }
    values
}

fn display_cell_value(
    model: &XlsxGridModel,
    values: &MergeValueCache,
    index: usize,
    row: usize,
    col: usize,
) -> String {
    let Some(sheet) = model.sheets.get(index) else {
        return String::new();
    };
    if let Some(anchor) = span_anchor_abs(&sheet.merge_spans, row, col) {
        return values
            .get(&(index, anchor.0, anchor.1))
            .cloned()
            .unwrap_or_else(|| value_at_absolute(sheet, anchor));
    }
    sheet
        .cells
        .get(row)
        .and_then(|r| r.get(col))
        .cloned()
        .unwrap_or_default()
}

fn refresh_committed_merge_value(
    model: &XlsxGridModel,
    values: &mut MergeValueCache,
    index: usize,
    row: usize,
    col: usize,
    value: &str,
) {
    if let Some(anchor) = model
        .sheets
        .get(index)
        .and_then(|sheet| span_anchor_abs(&sheet.merge_spans, row, col))
    {
        values.insert((index, anchor.0, anchor.1), value.to_string());
    }
}

// 会话字节经过原Office读面，保留格式/公式缓存；无merge不增加工作。
// 回接时读取pending后的真源，不用旧model覆盖尚未落盘的原锚新值。
fn merge_values_from_session(
    model: &XlsxGridModel,
    session: &XlsxEditor,
) -> Result<MergeValueCache, String> {
    let mut values = initial_merge_values(model);
    if values.is_empty() {
        return Ok(values);
    }
    let bytes = session
        .save_to_vec()
        .map_err(|e| format!("合并格会话读取失败：{e}"))?;
    let book = read_book_from_bytes_with_options(&bytes, &ReadOptions { compat: true })
        .map_err(|e| format!("合并格会话解析失败：{e}"))?;
    for (index, sheet) in model.sheets.iter().enumerate() {
        if sheet.merge_spans.is_empty() {
            continue;
        }
        let actual = book
            .sheets
            .get(index)
            .filter(|actual| actual.name == sheet.name)
            .ok_or_else(|| format!("合并格会话表映射失配：{}", sheet.name))?;
        for span in &sheet.merge_spans {
            let (row, col) = span.abs_anchor;
            values.insert(
                (index, row, col),
                cell_display(actual.typed.get_abs(row, col), actual.formula_at(row, col)),
            );
        }
    }
    Ok(values)
}

// ---------------------------------------------------------------------------
// #389 阶段 2：编辑纯函数面（零 GPUI——单测锚点）
// ---------------------------------------------------------------------------

/// 显示坐标 → 绝对 A1 引用（`selected_a1` 同口径——提交/错误面共用）。
fn cell_ref_for(sheet: &GridSheet, row: usize, col: usize) -> String {
    format!(
        "{}{}",
        col_letter(sheet.start_col + col as u32),
        sheet.start_row + row as u32 + 1
    )
}

/// 内联输入 → [`CellEdit`]（Excel 直觉口径）：
/// - 空白（含空白字符）→ `Text("")`（清格——引擎面 validate_text 放行）；
/// - 可解析为有限 f64 → `Number`（前后空白容忍）；
/// - `TRUE`/`FALSE`（大小写不敏感）→ `Boolean`；
/// - 其余 → `Text`（原文——保留内边空白，不加引号/不转义）。
fn parse_cell_input(raw: &str) -> CellEdit {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return CellEdit::Text(String::new());
    }
    if let Ok(n) = trimmed.parse::<f64>() {
        if n.is_finite() {
            return CellEdit::Number(n);
        }
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "true" => CellEdit::Boolean(true),
        "false" => CellEdit::Boolean(false),
        _ => CellEdit::Text(raw.to_string()),
    }
}

/// 提交编辑（引擎落账 + 显示面即时更新——「点击 → 状态变化」的核心
/// 纯函数，单测直接钉定）。
///
/// - 值走 [`XlsxEditor::set_cell`]（公式格/不可编辑表/非法字符 =
///   `Err`——错误文案原样上浮到面板 notice）；
/// - #584 返工：合并区编辑锚**原始绝对左上**（`span_anchor_abs`——
///   左上可在数据包围盒外[空合并格场景]，锚不随可见裁剪漂移），
///   非合并格走显示坐标换算；
/// - 落账成功 → `GridSheet::cells` 即时改写为输入原文（pending 视觉
///   ——与重开读面口径一致：Number 显示输入文本；锚在盒外时
///   显示面无槽位可写，跳过无害——读面以引擎为准）。
fn commit_cell_edit(
    editor: &mut XlsxEditor,
    model: &mut XlsxGridModel,
    sheet_idx: usize,
    row: usize,
    col: usize,
    buffer: &str,
) -> Result<(), String> {
    let sheet = model
        .sheets
        .get(sheet_idx)
        .ok_or_else(|| format!("表下标越界: {sheet_idx}"))?;
    // #584 返工：合并区 → 原始绝对锚（不裁剪）；非合并 → 显示坐标换算。
    let cell_ref = match span_anchor_abs(&sheet.merge_spans, row, col) {
        Some((abs_row, abs_col)) => format!("{}{}", col_letter(abs_col), abs_row + 1),
        None => cell_ref_for(sheet, row, col),
    };
    let value = parse_cell_input(buffer);
    editor
        .set_cell(sheet_idx, &cell_ref, value)
        .map_err(|e| e.to_string())?;
    // 更新真实锚在模型内的槽；锚在盒外时覆盖格仍保留原数据。
    let sheet = &mut model.sheets[sheet_idx];
    let anchor = span_anchor_abs(&sheet.merge_spans, row, col)
        .unwrap_or((sheet.start_row + row as u32, sheet.start_col + col as u32));
    if let (Some(actual_row), Some(actual_col)) = (
        anchor.0.checked_sub(sheet.start_row),
        anchor.1.checked_sub(sheet.start_col),
    ) {
        if let Some(text) = sheet
            .cells
            .get_mut(actual_row as usize)
            .and_then(|r| r.get_mut(actual_col as usize))
        {
            *text = buffer.to_string();
        }
    }
    Ok(())
}

/// 打开编辑会话（`fs::read` → `XlsxEditor::open`——office-io 门面恒
/// 严格；失败返回 (None, Some(原因))——不阻网格渲染，编辑面明示拒因）。
fn open_edit_session(path: &Path) -> (Option<XlsxEditor>, Option<String>) {
    match std::fs::read(path) {
        Ok(bytes) => match XlsxEditor::open(&bytes) {
            Ok(session) => (Some(session), None),
            Err(e) => (None, Some(e.to_string())),
        },
        Err(e) => (None, Some(format!("读取失败：{e}"))),
    }
}

/// 原子写盘（临时文件 + rename——docx 保存同纪律：写坏不毁原件）。
/// `pub`：shell 会话银行直接落盘路径（`save_xlsx_at`）共用同一纪律。
pub fn write_xlsx_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp = path.to_path_buf();
    let tag = format!(
        ".mang-tmp-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    tmp.set_extension(format!(
        "{}.{}",
        path.extension().and_then(|e| e.to_str()).unwrap_or("xlsx"),
        tag
    ));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

// ---------------------------------------------------------------------------
// 几何纯函数（零 GPUI——单测锚点）
// ---------------------------------------------------------------------------

/// 绝对列号（0 基）→ 列字母（A..Z / AA..——xlsx 语义）。
pub fn col_letter(col: u32) -> String {
    let mut n = col;
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (n % 26) as u8);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).expect("ASCII 列字母")
}

/// East Asian 显示宽（CJK 2 cell——与 docx_doc/xlsx_doc 同口径约定）。
pub fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| {
            let cp = c as u32;
            let cjk = (0x1100..=0x115F).contains(&cp) // Hangul Jamo
                || (0x2E80..=0xA4CF).contains(&cp) // CJK 部首..Yi
                || (0xAC00..=0xD7A3).contains(&cp) // Hangul 音节
                || (0xF900..=0xFAFF).contains(&cp) // CJK 兼容表意
                || (0xFE30..=0xFE4F).contains(&cp) // CJK 兼容形式
                || (0xFF00..=0xFF60).contains(&cp) // 全角形式
                || (0xFFE0..=0xFFE6).contains(&cp); // 全角符号
            if cjk { 2 } else { 1 }
        })
        .sum()
}

/// 内容自适应列宽（px）：内容 EA 宽（封顶 CELL_WIDTH_CAP）× 字号近似
/// + 内边距，域钳制 [COL_W_MIN, COL_W_MAX]。
pub fn col_width_px(content_ea: usize) -> f32 {
    let w = content_ea.min(CELL_WIDTH_CAP) as f32 * 7.5 + CELL_PAD * 2.0;
    w.clamp(COL_W_MIN, COL_W_MAX)
}

/// 逐列 x 偏移（px，含 COL_GAP——首列 0 起）。
pub fn col_offsets(widths: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(widths.len());
    let mut x = 0.0;
    for w in widths {
        out.push(x);
        x += w + COL_GAP;
    }
    out
}

/// 可见列窗口：`[first, end)`（scroll_x 起、viewport_w 宽的窗口覆盖的
/// 列区间——窗口外列零构建）。
pub fn visible_col_window(
    offsets: &[f32],
    widths: &[f32],
    scroll_x: f32,
    viewport_w: f32,
) -> (usize, usize) {
    let n = offsets.len().min(widths.len());
    let right = scroll_x + viewport_w.max(0.0);
    let mut first = n;
    let mut end = 0;
    for i in 0..n {
        let col_right = offsets[i] + widths[i];
        let visible = col_right > scroll_x && offsets[i] < right;
        if visible {
            if i < first {
                first = i;
            }
            end = i + 1;
        }
    }
    if first >= end {
        (0, 0) // 空表 / 零列 / 超界 scroll
    } else {
        (first, end)
    }
}

/// 横向滚动钳制：[0, max(0, total_w - viewport_w)]（内容窄于视口 = 0；
/// NaN 防御归 0）。
pub fn clamp_scroll_x(requested: f32, total_w: f32, viewport_w: f32) -> f32 {
    if requested.is_nan() {
        return 0.0;
    }
    let max = (total_w - viewport_w).max(0.0);
    requested.clamp(0.0, max)
}

/// 滚轮事件 → 横向位移（px）：deltaX 直传；Shift+deltaY 转横向
/// （触摸板两指横扫 / 鼠标 Shift+滚轮——两条自然手势都通；纯纵向
/// 返回 0 = 观察器不动作，uniform_list 原生滚动不受扰）。Lines 形态
/// 经 `pixel_delta` 换算（行高基准 = ROW_H——同仓 gpui 惯例）。
pub fn wheel_delta_x(ev: &ScrollEvent) -> f32 {
    let d = ev.delta.pixel_delta(px(ROW_H));
    let dx = f32::from(d.x);
    if dx.abs() > f32::EPSILON {
        return dx;
    }
    if ev.modifiers.shift {
        f32::from(d.y)
    } else {
        0.0
    }
}

/// 网格体命中测试：局部坐标（网格体元素内，px）+ 滚动状态 → (行, 列)。
/// `first_visible_row` = 当前纵向首可见行（uniform_list 滚动偏移换算）。
/// 行号槽（x < GUTTER_W）/ 列间隙 / 越界 = None。
pub fn hit_test_cell(
    offsets: &[f32],
    widths: &[f32],
    scroll_x: f32,
    first_visible_row: usize,
    x: f32,
    y: f32,
) -> Option<(usize, usize)> {
    if x < GUTTER_W || x.is_nan() || y.is_nan() || y < 0.0 {
        return None;
    }
    let content_x = x - GUTTER_W + scroll_x;
    let n = offsets.len().min(widths.len());
    let col = (0..n).find(|&c| content_x >= offsets[c] && content_x < offsets[c] + widths[c])?;
    let row = first_visible_row + (y / ROW_H) as usize;
    Some((row, col))
}

// ---------------------------------------------------------------------------
// 视图（GPUI 层——编译面验证 + 真机登记）
// ---------------------------------------------------------------------------

/// 内联编辑态（双击进入——Enter 提交 / Esc 取消 / Backspace 删 /
/// 可打印字符追加；显示坐标 (行, 列)）。
struct CellEditing {
    row: usize,
    col: usize,
    /// 输入缓冲（未提交原文——提交时经 [`parse_cell_input`] 分型）。
    buffer: String,
}

/// XLSX 网格视图实体（#389 阶段 2：含编辑会话）。
pub struct XlsxGridView {
    model: XlsxGridModel,
    merge_values: MergeValueCache,
    /// 当前表（簿序索引）。
    sheet_idx: usize,
    /// 逐列宽（px——当前表；表切换时重算）。
    col_widths: Vec<f32>,
    /// 逐列 x 偏移（px——`col_offsets(col_widths)`）。
    col_offsets: Vec<f32>,
    /// 横向滚动位置（px——自管状态，`clamp_scroll_x` 钳制）。
    scroll_x: f32,
    /// 视口宽（px——最近一次消费值；首帧 0 = 未记录，下帧起算）。
    viewport_w: f32,
    /// 视口宽回写通道（覆盖 canvas paint 每帧回写——Rc<Cell> 零实体
    /// 更新零重入，pdf_pages viewport 探针同模式）。
    viewport: Rc<Cell<Option<f32>>>,
    /// 选中格（显示坐标 (行, 列)；None = 无选中）。
    selected: Option<(usize, usize)>,
    /// 纵向虚拟化滚动句柄。
    scroll: UniformListScrollHandle,
    /// 焦点域（编辑态键位接收面——根元素 track_focus）。
    focus_handle: FocusHandle,
    /// 引擎面错误（构造期失败——面板明示，不白屏）。
    error: Option<String>,
    /// 源文件路径（保存写回目标；error 态无）。
    path: Option<PathBuf>,
    /// 编辑会话（#389 阶段 2；None = 会话不可用——打开失败/文件不可
    /// 读，双击明示拒因，不静默无反应）。
    editor: Option<XlsxEditor>,
    /// 会话打开失败原因（面板状态条明示）。
    editor_error: Option<String>,
    /// 内联编辑态（双击进入）。
    editing: Option<CellEditing>,
    /// 脏标记（有已提交未落盘编辑——`tab_is_modified`/关闭确认消费）。
    dirty: bool,
    /// 瞬时错误/状态面（提交被拒/保存失败——下一次成功动作清除）。
    notice: Option<String>,
}

impl XlsxGridView {
    /// 构造（模型就绪路径——`XlsxGridModel::load` 由 Shell 侧先行；
    /// 编辑会话从同路径字节独立打开，失败不阻渲染——双击时明示拒因）。
    pub fn new(model: XlsxGridModel, path: PathBuf, cx: &mut App) -> Self {
        let (editor, editor_error) = open_edit_session(&path);
        Self::with_session(model, Some(path), editor, editor_error, cx)
    }

    /// 会话回接构造（#389 阶段 2：切标签/切视图重建实体时，shell 把
    /// 既有会话注入——已提交未落盘编辑跨实体存活，零静默丢失）。
    pub fn new_with_session(
        model: XlsxGridModel,
        path: PathBuf,
        session: XlsxEditor,
        cx: &mut App,
    ) -> Self {
        Self::with_session(model, Some(path), Some(session), None, cx)
    }

    /// 共用构造体（editor = None + editor_error = Some = 会话不可用）。
    fn with_session(
        model: XlsxGridModel,
        path: Option<PathBuf>,
        editor: Option<XlsxEditor>,
        editor_error: Option<String>,
        cx: &mut App,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let merge_values = initial_merge_values(&model);
        let mut this = Self {
            merge_values,
            sheet_idx: 0,
            col_widths: Vec::new(),
            col_offsets: Vec::new(),
            scroll_x: 0.0,
            viewport_w: 0.0,
            viewport: Rc::new(Cell::new(None)),
            selected: None,
            scroll: UniformListScrollHandle::new(),
            focus_handle,
            error: None,
            path,
            editor,
            editor_error,
            editing: None,
            dirty: false,
            notice: None,
            model,
        };
        if let Some(session) = this.editor.as_ref() {
            match merge_values_from_session(&this.model, session) {
                Ok(values) => this.merge_values = values,
                Err(error) => this.error = Some(error),
            }
        }
        this.recompute_col_widths();
        this
    }

    /// error 态构造（模型装载失败——面板明示；无编辑面）。
    pub fn error_state(message: String, cx: &mut App) -> Self {
        Self {
            model: XlsxGridModel::default(),
            merge_values: MergeValueCache::new(),
            sheet_idx: 0,
            col_widths: Vec::new(),
            col_offsets: Vec::new(),
            scroll_x: 0.0,
            viewport_w: 0.0,
            viewport: Rc::new(Cell::new(None)),
            selected: None,
            scroll: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            error: Some(message),
            path: None,
            editor: None,
            editor_error: None,
            editing: None,
            dirty: false,
            notice: None,
        }
    }

    /// 会话交还（shell 切标签/关实体前提取——脏会话进 shell 会话银行
    /// `xlsx_sessions`，重建时经 [`Self::new_with_session`] 注回）。
    pub fn take_session(&mut self) -> Option<XlsxEditor> {
        // 编辑中的缓冲先行提交（切走不丢未按 Enter 的输入——Excel 同款）。
        self.commit_edit();
        self.editor.take()
    }

    /// 脏标记（shell `tab_is_modified` / 关闭确认消费）。
    pub fn is_modified(&self) -> bool {
        self.dirty
    }

    /// 编辑态只读探针（#604 shell 收口回归断言——双击进入/Enter
    /// 提交后确认缓冲生命周期，跨模块只读不触内部结构）。
    pub fn editing_active(&self) -> bool {
        self.editing.is_some()
    }

    /// 状态条文案只读探针（#604 失败面断言——编辑会话拒因/保存错误
    /// 的非静默示警面，跨模块只读）。
    pub fn notice_message(&self) -> Option<String> {
        self.notice.clone()
    }

    /// 脏标记回置（shell 会话银行回接时随行——`ensure_xlsx_grid` 消费）。
    pub fn set_modified(&mut self, dirty: bool) {
        self.dirty = dirty;
    }

    /// 保存（#389 阶段 2：会话 `save_to_vec` → 临时文件 + rename 写盘
    /// ——docx 保存同纪律，写坏不毁原件；成功清脏 + 清 notice，失败
    /// 错误文案进 notice 返回 false）。
    pub fn save(&mut self) -> bool {
        // 编辑中的缓冲先行提交（保存语义 = 落盘全部已输入内容）。
        self.commit_edit();
        let Some(path) = self.path.clone() else {
            self.notice = Some("保存失败：无源路径（error 态视图）".into());
            return false;
        };
        let Some(editor) = self.editor.as_ref() else {
            self.notice = Some("保存失败：编辑会话不可用".into());
            return false;
        };
        let bytes = match editor.save_to_vec() {
            Ok(bytes) => bytes,
            Err(e) => {
                self.notice = Some(format!("保存失败：{e}"));
                return false;
            }
        };
        match write_xlsx_atomically(&path, &bytes) {
            Ok(()) => {
                self.dirty = false;
                self.notice = None;
                eprintln!("saved (xlsx): {}", path.display());
                true
            }
            Err(e) => {
                self.notice = Some(format!("保存失败：{e}"));
                false
            }
        }
    }

    /// 当前表（空簿 = error 态面板路径，不达此处）。
    fn sheet(&self) -> &GridSheet {
        self.model
            .sheets
            .get(self.sheet_idx)
            .or_else(|| self.model.sheets.first())
            .expect("空簿走 error 态（render 门）；构造期 model.sheets 空时同样")
    }

    /// 重算列宽/偏移（构造 + 表切换时——选中/横向滚动随表重置）。
    /// #555：合并区格（含左上）不参与单列宽计算——左上文本由 span
    /// 宽格承载（跨列宽度），撑单列会把无关列顶宽。
    fn recompute_col_widths(&mut self) {
        let sheet = self.sheet();
        let mut widths = Vec::with_capacity(sheet.cols);
        for c in 0..sheet.cols {
            let content = sheet
                .cells
                .iter()
                .enumerate()
                .filter(|(r, row)| {
                    // 覆盖格（含左上）排除；行向量越界的格不覆盖（空尾行）。
                    row.get(c)
                        .is_some_and(|_| !is_covered(&sheet.merge_spans, *r, c))
                })
                .map(|(_, row)| display_width(row.get(c).map(String::as_str).unwrap_or("")))
                .max()
                .unwrap_or(0);
            widths.push(col_width_px(content));
        }
        self.col_offsets = col_offsets(&widths);
        self.col_widths = widths;
        self.selected = None;
        self.scroll_x = 0.0;
    }

    /// 切换工作表（簿序索引；越界/同表 = 无操作）。
    pub fn select_sheet(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.model.sheets.len() || idx == self.sheet_idx {
            return;
        }
        self.sheet_idx = idx;
        self.recompute_col_widths();
        cx.notify();
    }

    /// 当前表索引（状态面）。
    pub fn sheet_index(&self) -> usize {
        self.sheet_idx
    }

    /// 选中格（显示坐标 (行, 列)——阶段 2 编辑接驳锚）。
    pub fn selected_cell(&self) -> Option<(usize, usize)> {
        self.selected
    }

    /// 选中格的绝对 A1 引用（如 `B3`——无选中 = None）。
    /// #584 返工：合并区选中 → **原始绝对锚**（左上可在可见盒外
    /// ——空合并格场景锚 E5 而包围盒从 F6 起，不随裁剪漂移）；
    /// 非合并格显示坐标换算不变。
    pub fn selected_a1(&self) -> Option<String> {
        let sheet = self.sheet();
        let (r, c) = self.selected?;
        if let Some((abs_row, abs_col)) = span_anchor_abs(&sheet.merge_spans, r, c) {
            return Some(format!("{}{}", col_letter(abs_col), abs_row + 1));
        }
        Some(format!(
            "{}{}",
            col_letter(sheet.start_col + c as u32),
            sheet.start_row + r as u32 + 1
        ))
    }

    /// 网格内容总宽（px）。
    fn total_width(&self) -> f32 {
        match (self.col_offsets.last(), self.col_widths.last()) {
            (Some(o), Some(w)) => o + w,
            _ => 0.0,
        }
    }

    /// 纵向首可见行（uniform_list 滚动偏移换算——pdf_pages
    /// `submit_visible` 同口径）。
    fn first_visible_row(&self) -> usize {
        let offset_y = f32::from(self.scroll.0.borrow().base_handle.offset().y).max(0.0);
        (offset_y / ROW_H) as usize
    }

    /// 横向滚动（滚轮 deltaX / Shift+deltaY——观察器透传；钳制域内）。
    fn scroll_horizontal(&mut self, dx: f32, cx: &mut Context<Self>) {
        if dx.abs() < f32::EPSILON {
            return;
        }
        self.scroll_x = clamp_scroll_x(self.scroll_x + dx, self.total_width(), self.viewport_w);
        cx.notify();
    }

    /// 点选收尾（命中测试 → 选中 + 越界钳制；行号槽点击 = 无操作）。
    /// 编辑态先提交当前编辑（Excel 同款：点别的格 = 落定输入）。
    /// #555：命中经 `canonical_cell` 映射——点合并区覆盖格选中整个
    /// 合并区（锚左上——`selected_a1`/列头/行号联动随之锚左上）。
    fn select_at(&mut self, x: f32, y: f32, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            self.commit_edit();
            cx.notify();
        }
        let sheet_rows = self.sheet().rows;
        let hit = hit_test_cell(
            &self.col_offsets,
            &self.col_widths,
            self.scroll_x,
            self.first_visible_row(),
            x,
            y,
        )
        .filter(|(r, _)| *r < sheet_rows)
        .map(|(r, c)| canonical_cell(&self.sheet().merge_spans, r, c).unwrap_or((r, c)));
        if hit.is_some() {
            self.selected = hit;
            cx.notify();
        }
    }

    // ── #389 阶段 2：内联编辑交互 ──────────────────────────────

    /// 双击进入编辑（选中格随双击更新；缓冲以当前格显示文本起步——
    /// Excel「双击改原值」同款）。会话不可用/越界 → notice 明示拒因。
    fn begin_edit_at(&mut self, x: f32, y: f32, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            self.commit_edit();
        }
        let sheet_rows = self.sheet().rows;
        let hit = hit_test_cell(
            &self.col_offsets,
            &self.col_widths,
            self.scroll_x,
            self.first_visible_row(),
            x,
            y,
        )
        .filter(|(r, _)| *r < sheet_rows)
        .map(|(r, c)| {
            // #555：双击覆盖格 = 编辑合并区左上（缓冲以左上原值起步
            // ——Excel「双击改原值」同款；提交走左上引用，覆盖格零写入）。
            canonical_cell(&self.sheet().merge_spans, r, c).unwrap_or((r, c))
        });
        let Some((row, col)) = hit else {
            return;
        };
        self.selected = Some((row, col));
        if self.editor.is_none() {
            self.notice = Some(format!(
                "编辑不可用：{}",
                self.editor_error
                    .clone()
                    .unwrap_or_else(|| "编辑会话未开启".into())
            ));
            cx.notify();
            return;
        }
        let current = display_cell_value(&self.model, &self.merge_values, self.sheet_idx, row, col);
        self.notice = None;
        self.editing = Some(CellEditing {
            row,
            col,
            buffer: current,
        });
        cx.notify();
    }

    /// 提交编辑（Enter / 切选区 / 保存 / 会话交还的统一收口——
    /// `commit_cell_edit` 纯函数核心 + 脏标记 + notice 错误面）。
    fn commit_edit(&mut self) {
        let Some(ed) = self.editing.take() else {
            return;
        };
        if ed.buffer
            == display_cell_value(
                &self.model,
                &self.merge_values,
                self.sheet_idx,
                ed.row,
                ed.col,
            )
        {
            self.notice = None;
            return; // 确认未改：不创造原本不存在的空锚，不置脏。
        }
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        match commit_cell_edit(
            editor,
            &mut self.model,
            self.sheet_idx,
            ed.row,
            ed.col,
            &ed.buffer,
        ) {
            Ok(()) => {
                refresh_committed_merge_value(
                    &self.model,
                    &mut self.merge_values,
                    self.sheet_idx,
                    ed.row,
                    ed.col,
                    &ed.buffer,
                );
                self.dirty = true;
                self.notice = None;
            }
            Err(e) => {
                // 引擎拒改（公式格/非法字符等）——原值不动，错误明示。
                let cell_ref = self
                    .model
                    .sheets
                    .get(self.sheet_idx)
                    .map(|s| cell_ref_for(s, ed.row, ed.col))
                    .unwrap_or_default();
                self.notice = Some(format!("{cell_ref} 未改：{e}"));
            }
        }
    }

    /// 取消编辑（Esc——丢弃缓冲，原值不动）。
    fn cancel_edit(&mut self) {
        self.editing = None;
    }

    /// 键位面（编辑态：Enter 提交 / Esc 取消 / Backspace 删尾 / 可打印
    /// 字符追加；非编辑态：Ctrl+S 保存——菜单路径之外的键位直通车）。
    /// 编辑器同源口径：`keystroke.key_char` 无 IME 直输（find_bar/buffer
    /// 同款已知边界）。
    fn handle_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        let ctrl = ks.modifiers.control || ks.modifiers.platform;
        let key = ks.key.to_lowercase();
        if let Some(ed) = self.editing.as_mut() {
            match key.as_str() {
                "enter" => {
                    self.commit_edit();
                    cx.notify();
                }
                "escape" => {
                    self.cancel_edit();
                    cx.notify();
                }
                "backspace" => {
                    ed.buffer.pop();
                    cx.notify();
                }
                _ => {
                    // 修饰键组合（Ctrl/Cmd/Alt）不进缓冲——快捷键让路。
                    if ctrl || ks.modifiers.alt {
                        return;
                    }
                    // key_char 为 Option<String>（组合键可为空串）——
                    // 控制字符不入缓冲，其余逐字符追加。
                    let text = ks.key_char.as_deref().unwrap_or("");
                    let mut pushed = false;
                    for ch in text.chars().filter(|c| !c.is_control()) {
                        ed.buffer.push(ch);
                        pushed = true;
                    }
                    if pushed {
                        cx.notify();
                    }
                }
            }
            cx.stop_propagation();
            return;
        }
        // 非编辑态：Ctrl+S 直通保存（网格视图持焦点时编辑器键位面
        // 不可达——菜单栏「文件 → 保存」走 shell 拦截，两路同收口）。
        if ctrl && key == "s" {
            let saved = self.save();
            if !saved {
                eprintln!(
                    "mang-shell: xlsx 保存失败（notice 见网格面板）：{}",
                    self.notice.clone().unwrap_or_default()
                );
            }
            cx.notify();
            cx.stop_propagation();
        }
    }

    /// 工作表切换行（簿序 tab——当前表高亮；溢出横滚）。
    fn render_sheet_tabs(&self, cx: &App) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let mut tabs = h_flex()
            .id("xlsx-grid-sheet-tabs")
            .w_full()
            .h(px(28.0))
            .flex_none()
            .gap_1()
            .px_2()
            .bg(colors.background)
            .border_b_1()
            .border_color(colors.border)
            .overflow_x_scroll();
        for (idx, name) in self
            .model
            .sheets
            .iter()
            .map(|s| s.name.as_str())
            .enumerate()
        {
            let is_active = idx == self.sheet_idx;
            let selected = colors.element_selected;
            let hover = colors.element_hover;
            tabs =
                tabs.child(
                    div()
                        .id(SharedString::from(format!("xlsx-grid-tab-{idx}")))
                        .px_2()
                        .h(px(22.0))
                        .flex_none()
                        .rounded_sm()
                        .items_center()
                        .when(is_active, |el| el.bg(selected))
                        .when(!is_active, move |el| el.hover(move |el| el.bg(hover)))
                        .child(ui::Label::new(name).size(ui::LabelSize::XSmall).color(
                            if is_active {
                                ui::Color::Default
                            } else {
                                ui::Color::Muted
                            },
                        )),
                );
        }
        tabs.into_any_element()
    }

    /// 列头行元素（corner + 可见列窗口的字母格——绝对定位于行内，
    /// x = 偏移 - scroll_x；窗口外列零构建；corner 后置 = 盖住左滑
    /// 进 gutter 区的半可见列头（无 z_index——绘制序即层序））。
    fn render_col_header(&self, cx: &App) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let (first, end) = visible_col_window(
            &self.col_offsets,
            &self.col_widths,
            self.scroll_x,
            self.viewport_w,
        );
        let sheet = self.sheet();
        let mut row = h_flex()
            .id("xlsx-grid-col-header")
            .relative()
            .w_full()
            .h(px(HEAD_H))
            .flex_none()
            .bg(colors.panel_background)
            .border_b_1()
            .border_color(colors.border);
        for c in first..end {
            let letter = col_letter(sheet.start_col + c as u32);
            let x = self.col_offsets[c] - self.scroll_x;
            let w = self.col_widths[c];
            let selected_col = self.selected.is_some_and(|(_, sc)| sc == c);
            row = row.child(
                div()
                    .id(SharedString::from(format!("xlsx-grid-col-{c}")))
                    .absolute()
                    .left(px(x + GUTTER_W))
                    .top_0()
                    .w(px(w))
                    .h_full()
                    .flex_none()
                    .items_center()
                    .px(px(CELL_PAD))
                    .when(selected_col, |el| el.bg(colors.element_hover))
                    .child(
                        ui::Label::new(letter)
                            .size(ui::LabelSize::XSmall)
                            .color(ui::Color::Muted),
                    ),
            );
        }
        // corner（左上固定块——后置 = 绘制在列头之上，盖住左滑半可见列）。
        row = row.child(
            div()
                .id("xlsx-grid-corner")
                .w(px(GUTTER_W))
                .h_full()
                .flex_none()
                .bg(colors.panel_background)
                .border_r_1()
                .border_color(colors.border),
        );
        row.into_any_element()
    }

    /// 行元素（uniform_list 回调——可见列窗口的单元格 + 行号槽；行内
    /// 绝对定位，窗口外零构建；选中格高亮边框 + 行号/列头联动底色；
    /// 行号槽后置 = 盖住左滑进槽区的半可见格（无 z_index——绘制序即
    /// 层序，列头 corner 同款）。
    /// #555：合并区切片渲染——覆盖格（含左上）不独立构建，span 宽格
    /// 统一承载（内部网格线抑制 = 合并视觉整体；跨行 span 每行画
    /// 自己那一段，选择/编辑态四边框按行拼接：首行 top / 末行
    /// bottom / 每行 left+right）。
    fn render_row(&self, r: usize, cx: &App) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let sheet = self.sheet();
        let (first, end) = visible_col_window(
            &self.col_offsets,
            &self.col_widths,
            self.scroll_x,
            self.viewport_w,
        );
        let row_no = sheet.start_row + r as u32 + 1; // 1 基显示
        let mut row = h_flex()
            .id(SharedString::from(format!("xlsx-grid-row-{r}")))
            .relative()
            .w_full()
            .h(px(ROW_H))
            .flex_none()
            .overflow_hidden()
            .bg(colors.background);
        // 该行的合并 span（一次取出——格循环跳格 + span 循环绘制共用）。
        let spans = spans_in_row(&sheet.merge_spans, r);
        for c in first..end {
            // #555：合并覆盖格（含左上）跳过独立渲染——span 宽格承载。
            if spans.iter().any(|s| s.contains(r, c)) {
                continue;
            }
            // #389 阶段 2：编辑中的格显示输入缓冲（编辑样式——accent
            // 双线框 + 活动底色，与选中态视觉可区分）。
            let editing_here = self
                .editing
                .as_ref()
                .is_some_and(|ed| ed.row == r && ed.col == c);
            let text = if editing_here {
                self.editing
                    .as_ref()
                    .map(|ed| ed.buffer.clone())
                    .unwrap_or_default()
            } else {
                sheet
                    .cells
                    .get(r)
                    .and_then(|row| row.get(c))
                    .cloned()
                    .unwrap_or_default()
            };
            let x = self.col_offsets[c] - self.scroll_x;
            let w = self.col_widths[c];
            let is_selected = self.selected == Some((r, c));
            let border = colors.border;
            let accent = colors.element_selected;
            let hover = colors.element_hover;
            let active_bg = colors.element_active;
            row = row.child(
                div()
                    .id(SharedString::from(format!("xlsx-grid-cell-{r}-{c}")))
                    .absolute()
                    .left(px(x + GUTTER_W))
                    .top_0()
                    .w(px(w))
                    .h_full()
                    .flex_none()
                    .items_center()
                    .px(px(CELL_PAD))
                    .overflow_hidden()
                    .border_r_1()
                    .border_color(border)
                    .when(editing_here, move |el| {
                        el.border_1().border_color(accent).bg(active_bg)
                    })
                    .when(is_selected && !editing_here, move |el| {
                        el.border_1().border_color(accent).bg(hover)
                    })
                    .child(
                        ui::Label::new(text)
                            .size(ui::LabelSize::XSmall)
                            .color(ui::Color::Default),
                    ),
            );
        }
        // #555：合并 span 宽格（与可见窗口相交才构建——左端滚出窗外/
        // 右端未入窗的部分可见照常渲染：绝对定位天然承载，负 x 由
        // 行号槽后置盖住、右溢由行 overflow_hidden 裁掉）。
        for span in spans {
            if !intersects_window(span.col_start, span.col_end, first, end) {
                continue;
            }
            let Some((x, w)) = span_px(
                &self.col_offsets,
                &self.col_widths,
                span.col_start,
                span.col_end,
            ) else {
                continue; // 脏 span 防御（下标越界）——跳过不 panic
            };
            let Some(edges) = span_edges(span, r) else {
                continue;
            };
            let x = x - self.scroll_x;
            let anchor = (span.row_start, span.col_start);
            // 值只在首行切片显示（Excel 左上单点语义——其余行切片空）。
            let editing_here = self
                .editing
                .as_ref()
                .is_some_and(|ed| (ed.row, ed.col) == anchor && r == span.row_start);
            let text = if r == span.row_start {
                if editing_here {
                    self.editing
                        .as_ref()
                        .map(|ed| ed.buffer.clone())
                        .unwrap_or_default()
                } else {
                    display_cell_value(
                        &self.model,
                        &self.merge_values,
                        self.sheet_idx,
                        span.row_start,
                        span.col_start,
                    )
                }
            } else {
                String::new()
            };
            let is_selected = self.selected == Some(anchor);
            let border = colors.border;
            let accent = colors.element_selected;
            let hover = colors.element_hover;
            let active_bg = colors.element_active;
            // 正常态只画右缘线（与普通格一致——左缘线来自左邻格的
            // border_r）；选中/编辑态画整区外框的该行段（top/bottom
            // 分段 + left/right 全行——拼起来 = 整区四边框，行间无
            // 横线 = 合并视觉整体）。
            let cell = div()
                .id(SharedString::from(format!(
                    "xlsx-grid-merge-{r}-{}-{}",
                    span.col_start, span.col_end
                )))
                .absolute()
                .left(px(x + GUTTER_W))
                .top_0()
                .w(px(w))
                .h_full()
                .flex_none()
                .items_center()
                .px(px(CELL_PAD))
                .overflow_hidden()
                .border_r_1()
                .border_color(border)
                .when(is_selected || editing_here, |el| el.border_l_1())
                .when(is_selected || editing_here, |el| el.border_color(accent))
                .when((is_selected || editing_here) && edges.top, |el| {
                    el.border_t_1()
                })
                .when((is_selected || editing_here) && edges.bottom, |el| {
                    el.border_b_1()
                })
                .when(editing_here, move |el| el.bg(active_bg))
                .when(is_selected && !editing_here, move |el| el.bg(hover))
                .child(
                    ui::Label::new(text)
                        .size(ui::LabelSize::XSmall)
                        .color(ui::Color::Default),
                );
            row = row.child(cell);
        }
        // 行号槽（左固定——后置 = 绘制在单元格之上，盖住左滑半可见格）。
        let selected_row = self.selected.is_some_and(|(sr, _)| sr == r);
        row = row.child(
            div()
                .id(SharedString::from(format!("xlsx-grid-gutter-{r}")))
                .w(px(GUTTER_W))
                .h_full()
                .flex_none()
                .items_center()
                .justify_end()
                .pr_2()
                .bg(if selected_row {
                    colors.element_hover
                } else {
                    colors.panel_background
                })
                .border_r_1()
                .border_color(colors.border)
                .child(
                    ui::Label::new(row_no.to_string())
                        .size(ui::LabelSize::XSmall)
                        .color(ui::Color::Muted),
                ),
        );
        row.into_any_element()
    }
}

impl Focusable for XlsxGridView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for XlsxGridView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 消费覆盖 canvas 上帧记录的视口宽（pdf_pages viewport 探针同
        // 模式——Rc<Cell> 零重入）。
        if let Some(w) = self.viewport.get() {
            if w > 0.0 {
                self.viewport_w = w - GUTTER_W; // 网格体内容宽（扣除行号槽）
            }
        }
        let colors = cx.theme().colors().clone();
        // error 态 / 空簿：明示面板（不白屏——pdf_pages 同纪律）。
        if self.error.is_some() || self.model.sheets.is_empty() {
            let message = self
                .error
                .clone()
                .unwrap_or_else(|| "空工作簿（无工作表）".into());
            return v_flex()
                .id("xlsx-grid-error")
                .size_full()
                .items_center()
                .justify_center()
                .bg(colors.background)
                .child(ui::Label::new(message).color(ui::Color::Warning))
                .child(
                    ui::Label::new("网格视图不可用——文本行视图仍可查看（状态栏切换）")
                        .size(ui::LabelSize::Small)
                        .color(ui::Color::Muted),
                )
                .into_any_element();
        }
        let sheet_tabs = self.render_sheet_tabs(cx);
        let col_header = self.render_col_header(cx);
        let rows = self.sheet().rows;
        let truncated = self.sheet().truncated;
        let sheet_name = self.sheet().name.clone();
        // 行回调（uniform_list 每帧重建可见行——窗口实体 downgrade）。
        let entity = cx.entity().downgrade();
        let list = gpui::uniform_list(
            "xlsx-grid-rows",
            rows,
            move |visible: std::ops::Range<usize>, _window: &mut Window, cx: &mut App| {
                visible
                    .clone()
                    .filter_map(|r| {
                        let slot = entity.update(cx, |view, cx| view.render_row(r, cx)).ok()?;
                        Some(slot)
                    })
                    .collect::<Vec<AnyElement>>()
            },
        )
        .track_scroll(&self.scroll)
        .h_full();
        // 截断护栏提示（非静默——xlsx_doc 同口径）。
        let truncation_banner = truncated.then(|| {
            div()
                .id("xlsx-grid-truncated")
                .flex_none()
                .w_full()
                .px_2()
                .py_1()
                .bg(colors.panel_background)
                .border_t_1()
                .border_color(colors.border)
                .child(
                    ui::Label::new(format!(
                        "「{sheet_name}」超出渲染护栏（行 ≤{MAX_RENDER_ROWS} / 列 ≤{MAX_RENDER_COLS}）——网格视图已截断，完整内容见文本视图"
                    ))
                    .size(ui::LabelSize::XSmall)
                    .color(ui::Color::Warning),
                )
        });
        // 横向滚动指示条（底部——scroll_x / 总宽的比例条；内容窄于
        // 视口不占位；thumb 位置/宽按已记录的视口宽换算 px）。
        let total_w = self.total_width();
        let track_w = self.viewport_w;
        let h_scroll = (total_w > self.viewport_w && track_w > 0.0).then(|| {
            let ratio = (self.viewport_w / total_w).clamp(0.05, 1.0);
            let thumb_w = ratio * track_w;
            let thumb_x = (self.scroll_x / (total_w - self.viewport_w).max(1.0))
                * (track_w - thumb_w).max(0.0);
            div()
                .id("xlsx-grid-h-scroll")
                .relative()
                .flex_none()
                .w_full()
                .h(px(6.0))
                .bg(colors.panel_background)
                .border_t_1()
                .border_color(colors.border)
                .child(
                    div()
                        .id("xlsx-grid-h-thumb")
                        .absolute()
                        .top_0()
                        .left(px(thumb_x))
                        .w(px(thumb_w))
                        .h_full()
                        .rounded_sm()
                        .bg(colors.border),
                )
        });
        // 覆盖 canvas：paint 期注册窗口级滚轮横扫 + 点击命中观察器
        //（bounds 过滤——只落网格体内才动作；不占命中区不挡列表滚轮；
        // pdf_pages annotate_overlay 同模式）+ 视口宽回写（Rc<Cell>）。
        let ent = cx.entity().downgrade();
        let viewport_cell = self.viewport.clone();
        let overlay = div()
            .id("xlsx-grid-overlay")
            .absolute()
            .size_full()
            // #584 返工：canvas 自身 .size_full()——Canvas 默认 auto 尺寸
            //（flex 列容器内主轴高度塌缩为 0，观察器 in_y 边界过滤必然
            // 失败——真实点击全被吞）；铺满才承载命中面。
            .child(
                gpui::canvas(
                    move |bounds, _window, _cx| bounds,
                    move |bounds: gpui::Bounds<Pixels>, _t, window: &mut Window, _cx: &mut App| {
                        // 视口宽回写（本帧 bounds——render 期消费）。
                        viewport_cell.set(Some(f32::from(bounds.size.width)));
                        let ent_wheel = ent.clone();
                        let ent_click = ent.clone();
                        // 滚轮横扫：deltaX / Shift+deltaY → scroll_x（纯纵向
                        // 不动作——uniform_list 原生滚动不受扰）。
                        window.on_mouse_event(
                            move |ev: &ScrollEvent,
                                  phase: DispatchPhase,
                                  _w: &mut Window,
                                  cx: &mut App| {
                                if phase != DispatchPhase::Bubble {
                                    return;
                                }
                                let dx = wheel_delta_x(ev);
                                if dx.abs() < f32::EPSILON {
                                    return;
                                }
                                let _ = ent_wheel.update(cx, |view, cx| {
                                    view.scroll_horizontal(dx, cx);
                                });
                            },
                        );
                        // 点击命中：局部坐标 → (行, 列) 选中（bounds 内才动作）。
                        // #389 阶段 2：双击（click_count >= 2）→ 内联编辑；
                        // 单击在编辑态 → 先提交再选新格（select_at 内收口）。
                        window.on_mouse_event(
                            move |ev: &gpui::MouseDownEvent,
                                  phase: DispatchPhase,
                                  _w: &mut Window,
                                  cx: &mut App| {
                                if phase != DispatchPhase::Bubble
                                    || ev.button != gpui::MouseButton::Left
                                {
                                    return;
                                }
                                let in_x = ev.position.x >= bounds.origin.x
                                    && ev.position.x <= bounds.origin.x + bounds.size.width;
                                let in_y = ev.position.y >= bounds.origin.y
                                    && ev.position.y <= bounds.origin.y + bounds.size.height;
                                if !(in_x && in_y) {
                                    return;
                                }
                                let x = f32::from(ev.position.x - bounds.origin.x);
                                let y = f32::from(ev.position.y - bounds.origin.y);
                                let double_click = ev.click_count >= 2;
                                // 点击即落焦（编辑键位面激活——pdf_pages 点击
                                // 同款纪律；handle 经实体 update 取出后在
                                // 观察器 window 域落焦，F7 教训同源）。
                                let focus = ent_click.update(cx, |view, cx| {
                                    let handle = view.focus_handle(cx).clone();
                                    if double_click {
                                        view.begin_edit_at(x, y, cx);
                                    } else {
                                        view.select_at(x, y, cx);
                                    }
                                    handle
                                });
                                if let Ok(handle) = focus {
                                    _w.focus(&handle, cx);
                                }
                            },
                        );
                    },
                )
                .size_full(),
            );
        // #389 阶段 2：状态条（脏标记 / 编辑会话拒因 / 提交与保存错误
        // ——非静默纪律：每个失败面都有可见文案）。
        let dirty_chip = self.dirty.then(|| {
            div()
                .id("xlsx-grid-dirty")
                .flex_none()
                .px_2()
                .py_0p5()
                .rounded_sm()
                .bg(colors.element_hover)
                .child(
                    ui::Label::new("● 未保存")
                        .size(ui::LabelSize::XSmall)
                        .color(ui::Color::Warning),
                )
        });
        let notice_banner = self.notice.as_ref().map(|msg| {
            div()
                .id("xlsx-grid-notice")
                .flex_none()
                .w_full()
                .px_2()
                .py_1()
                .bg(colors.panel_background)
                .border_t_1()
                .border_color(colors.border)
                .child(
                    ui::Label::new(msg.as_str())
                        .size(ui::LabelSize::XSmall)
                        .color(ui::Color::Warning),
                )
        });
        let editor_error_banner = self.editor_error.as_ref().map(|msg| {
            div()
                .id("xlsx-grid-editor-error")
                .flex_none()
                .w_full()
                .px_2()
                .py_1()
                .bg(colors.panel_background)
                .border_t_1()
                .border_color(colors.border)
                .child(
                    ui::Label::new(format!("编辑不可用（只读浏览）：{msg}"))
                        .size(ui::LabelSize::XSmall)
                        .color(ui::Color::Muted),
                )
        });
        v_flex()
            .id("xlsx-grid-root")
            .relative()
            .size_full()
            .min_w_0()
            .bg(colors.background)
            // #389 阶段 2：焦点域（编辑态键位接收——pdf_pages 同款
            // track_focus 挂根模式）+ 键位分发（编辑/保存）。
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key))
            .child(sheet_tabs)
            .child(col_header)
            .child(
                v_flex()
                    .id("xlsx-grid-body")
                    .flex_1()
                    .min_h_0()
                    .child(list)
                    .children(h_scroll),
            )
            .children(truncation_banner)
            .children(editor_error_banner)
            .children(notice_banner)
            .children(dirty_chip)
            .child(overlay)
            .into_any_element()
    }
}

// ---------------------------------------------------------------------------
// 纯函数面单测（零 GPUI 依赖——CI 无 GUI 同样真跑）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── #389 阶段 2：编辑面（parse/引用/提交/往返） ─────────────

    /// 夹具路径（tests/fixtures/smoke.xlsx——B2=100 数值格、B4=SUM 公式
    /// 格、A2=「一月」共享串格；xlsx_grid_model.rs 同源夹具）。
    fn fixture() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.xlsx")
    }

    #[test]
    fn parse_cell_input_variants() {
        // 数值（前后空白容忍；f64 有限值）。
        assert!(matches!(parse_cell_input("100"), CellEdit::Number(n) if n == 100.0));
        assert!(matches!(parse_cell_input(" 200.5 "), CellEdit::Number(n) if n == 200.5));
        assert!(matches!(parse_cell_input("-3"), CellEdit::Number(n) if n == -3.0));
        // 布尔（大小写不敏感）。
        assert!(matches!(parse_cell_input("TRUE"), CellEdit::Boolean(true)));
        assert!(matches!(
            parse_cell_input("false"),
            CellEdit::Boolean(false)
        ));
        // 文本（原文保留——内边空白不裁剪、无引号转义）。
        assert!(
            matches!(parse_cell_input("hello world"), CellEdit::Text(ref t) if t == "hello world")
        );
        assert!(matches!(parse_cell_input("  pad  "), CellEdit::Text(ref t) if t == "  pad  "));
        // 数值不可解析的近形（日期/分数/科学计数法越界）→ 文本。
        assert!(matches!(parse_cell_input("2026-01-01"), CellEdit::Text(_)));
        // 空白 → 清格（空 Text——引擎面 validate_text 放行）。
        assert!(matches!(parse_cell_input("   "), CellEdit::Text(ref t) if t.is_empty()));
    }

    #[test]
    fn cell_ref_for_maps_display_coords_to_a1() {
        let mut sheet = GridSheet::default();
        sheet.start_row = 0;
        sheet.start_col = 0;
        assert_eq!(cell_ref_for(&sheet, 0, 0), "A1");
        assert_eq!(cell_ref_for(&sheet, 2, 1), "B3");
        // 数据区不从 A1 起的簿（office-io 包围盒偏移）。
        sheet.start_row = 3;
        sheet.start_col = 2;
        assert_eq!(cell_ref_for(&sheet, 0, 0), "C4");
        assert_eq!(cell_ref_for(&sheet, 1, 0), "C5");
    }

    #[test]
    fn commit_cell_edit_updates_model_and_rejects_formula() {
        let bytes = std::fs::read(fixture()).expect("夹具读取");
        let mut editor = XlsxEditor::open(&bytes).expect("编辑会话打开");
        let mut model = XlsxGridModel::load(&fixture()).expect("网格模型装载");

        // B2（数值格，显示坐标 (1,1)——表头行 0）改文本：落账 + 显示面
        // 即时更新。
        commit_cell_edit(&mut editor, &mut model, 0, 1, 1, "三百").expect("B2 值编辑落账");
        assert_eq!(model.sheets[0].cells[1][1], "三百", "pending 显示面更新");

        // B4（公式格，显示坐标 (3,1)）拒改——引擎面 FormulaCell。
        let err =
            commit_cell_edit(&mut editor, &mut model, 0, 3, 1, "999").expect_err("公式格必须拒改");
        assert!(err.contains("公式"), "拒改文案含公式语义：{err}");
        // 拒改后显示面不动（原缓存值 300.5 保持）。
        assert_eq!(model.sheets[0].cells[3][1], "300.5");

        // 保存 → 重开读面：B2 新值在场（真落盘闭环——不是只改内存态）。
        let saved = editor.save_to_vec().expect("保存序列化");
        let mut reopened = XlsxEditor::open(&saved).expect("重开会话");
        let info = reopened.cell_info(0, "B2").expect("B2 在场").expect("非空");
        assert_eq!(info.text, "三百", "落盘后 B2 = 三百");
    }

    #[test]
    fn write_xlsx_atomically_preserves_original_on_failure() {
        // 正常路径：临时文件 + rename 落位（目标可写）。
        let dir = std::env::temp_dir().join(format!("mang-xlsx-atom-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("out.xlsx");
        assert!(write_xlsx_atomically(&target, b"PK\x03\x04bytes").is_ok());
        assert_eq!(std::fs::read(&target).unwrap(), b"PK\x03\x04bytes");
        // 临时文件不残留（rename 收干净）。
        let residue: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("mang-tmp"))
            .collect();
        assert!(residue.is_empty(), "无临时文件残留：{residue:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn col_letter_sequence_and_wrap() {
        assert_eq!(col_letter(0), "A");
        assert_eq!(col_letter(25), "Z");
        assert_eq!(col_letter(26), "AA");
        assert_eq!(col_letter(27), "AB");
        assert_eq!(col_letter(701), "ZZ");
        assert_eq!(col_letter(702), "AAA");
    }

    #[test]
    fn display_width_ea_aware() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("中文"), 4, "CJK 双宽");
        assert_eq!(display_width("a中b"), 4, "混合");
        assert_eq!(display_width(""), 0);
    }

    #[test]
    fn col_width_px_clamps() {
        assert_eq!(col_width_px(0), COL_W_MIN, "空列 = 下限");
        assert_eq!(col_width_px(3), COL_W_MIN, "短内容仍下限");
        // 封顶内容（24 cell）的自然宽 = 24×7.5+12 = 192 = 上限。
        assert_eq!(col_width_px(24), COL_W_MAX, "封顶内容 = 上限");
        assert_eq!(
            col_width_px(200),
            COL_W_MAX,
            "超宽内容 = 上限（CELL_WIDTH_CAP 截断面）"
        );
    }

    #[test]
    fn col_offsets_cumulative_with_gap() {
        let offs = col_offsets(&[100.0, 50.0, 80.0]);
        assert_eq!(offs, vec![0.0, 101.0, 152.0], "含 COL_GAP 累计");
    }

    #[test]
    fn visible_col_window_basic() {
        let widths = vec![100.0; 6];
        let offs = col_offsets(&widths);
        // 视口 300 @ scroll 0 → 列 0..3（列 2 右缘 300 边界算可见）。
        let (f, e) = visible_col_window(&offs, &widths, 0.0, 300.0);
        assert_eq!((f, e), (0, 3), "首窗口");
        // scroll 202（列 2 左缘 202）→ 列 2..5（505 右缘钳在窗口内）。
        let (f, e) = visible_col_window(&offs, &widths, 202.0, 300.0);
        assert_eq!((f, e), (2, 5), "中窗口");
        // 滚到最右：总宽 605，视口 300 → scroll max 305 → 窗口 3..6。
        let (f, e) = visible_col_window(&offs, &widths, 305.0, 300.0);
        assert_eq!((f, e), (3, 6), "末窗口");
        // 超界 scroll = 零窗口（调用方 clamp 前置——契约防御面）。
        let (f, e) = visible_col_window(&offs, &widths, 1e9, 300.0);
        assert_eq!((f, e), (0, 0), "超界 scroll = 零窗口");
        // 空表。
        let (f, e) = visible_col_window(&[], &[], 0.0, 300.0);
        assert_eq!((f, e), (0, 0), "零列 = 零窗口");
    }

    #[test]
    fn clamp_scroll_x_domain() {
        assert_eq!(clamp_scroll_x(-50.0, 1000.0, 300.0), 0.0, "负值钳 0");
        assert_eq!(
            clamp_scroll_x(900.0, 1000.0, 300.0),
            700.0,
            "上限 = 总宽-视口"
        );
        assert_eq!(clamp_scroll_x(50.0, 100.0, 300.0), 0.0, "内容窄于视口 = 0");
        assert_eq!(clamp_scroll_x(f32::NAN, 1000.0, 300.0), 0.0, "NaN 防御");
    }

    #[test]
    fn hit_test_cell_resolves_and_rejects() {
        let widths = vec![100.0, 80.0];
        let offs = col_offsets(&widths);
        // scroll 0：x=GUTTER_W+10 → 列 0；y=45 → 行 2（首可见 0）。
        assert_eq!(
            hit_test_cell(&offs, &widths, 0.0, 0, GUTTER_W + 10.0, 45.0),
            Some((2, 0))
        );
        // 列 1（x = GUTTER_W + 101..180）。
        assert_eq!(
            hit_test_cell(&offs, &widths, 0.0, 0, GUTTER_W + 105.0, 0.0),
            Some((0, 1))
        );
        // 行号槽（x < GUTTER_W）= None。
        assert_eq!(hit_test_cell(&offs, &widths, 0.0, 0, 10.0, 10.0), None);
        // 列间隙（x = GUTTER_W + 100.5 在 100..101 间隙）= None。
        assert_eq!(
            hit_test_cell(&offs, &widths, 0.0, 0, GUTTER_W + 100.5, 10.0),
            None
        );
        // scroll_x=101：列 0 左移出窗 → x=GUTTER_W+10 命中列 1。
        assert_eq!(
            hit_test_cell(&offs, &widths, 101.0, 0, GUTTER_W + 10.0, 0.0),
            Some((0, 1))
        );
        // 首可见行偏移（纵向滚过 3 行）：y=0 → 行 3。
        assert_eq!(
            hit_test_cell(&offs, &widths, 0.0, 3, GUTTER_W + 10.0, 0.0),
            Some((3, 0))
        );
        // 负 y / NaN 防御。
        assert_eq!(
            hit_test_cell(&offs, &widths, 0.0, 0, GUTTER_W + 10.0, -5.0),
            None
        );
        assert_eq!(hit_test_cell(&offs, &widths, 0.0, 0, f32::NAN, 0.0), None);
    }

    #[test]
    fn cell_display_formula_fallback() {
        use mdc_office_io::io::CellData;
        assert_eq!(
            cell_display(Some(&CellData::Empty), Some("SUM(A1:A2)")),
            "=SUM(A1:A2)"
        );
        assert_eq!(cell_display(Some(&CellData::Number(1.5)), None), "1.5");
        assert_eq!(cell_display(None, None), "");
    }

    // ── #555：合并单元格（模型携带 + 编辑落账 + 视图接线） ──────

    /// 表 1 合并 B5:C5 的显示坐标锚（绝对 (4,1)-(4,2)，包围盒 A1 起）。
    fn smoke_merge_span() -> crate::xlsx_merge_geometry::MergeSpan {
        crate::xlsx_merge_geometry::MergeSpan {
            row_start: 4,
            row_end: 4,
            col_start: 1,
            col_end: 2,
            abs_anchor: (4, 1),
        }
    }

    #[test]
    fn from_book_carries_merge_spans() {
        let model = XlsxGridModel::load(&fixture()).expect("网格模型装载");
        // 表 1：B5:C5（绝对 (4,1)-(4,2)）→ 显示坐标同值（A1 起包围盒）。
        assert_eq!(
            model.sheets[0].merge_spans,
            vec![smoke_merge_span()],
            "合并范围全量携带（此前只有计数——范围被丢弃）"
        );
        // 计数与范围口径分离：merges = 原始计数（含裁掉的），spans = 可渲染。
        assert_eq!(model.sheets[0].merges, 1);
        // 表 2：无合并。
        assert!(model.sheets[1].merge_spans.is_empty(), "表 2 无合并范围");
        // 覆盖格 C5（显示 (4,2)）命中映射到左上 B5（4,1）。
        assert_eq!(
            canonical_cell(&model.sheets[0].merge_spans, 4, 2),
            Some((4, 1)),
            "覆盖格 → 左上（Excel 语义）"
        );
        // 普通格不受影响（无映射——单格语义零漂移）。
        assert_eq!(canonical_cell(&model.sheets[0].merge_spans, 1, 1), None);
    }

    #[test]
    fn commit_merge_edit_writes_top_left_only_and_preserves_range() {
        // #555 验收 3（模型/落账面）：双击覆盖格 → 修改左上值 → 保存 →
        // 重读——左上值已改、merge 范围原样保留、覆盖格未写入重复值。
        let bytes = std::fs::read(fixture()).expect("夹具读取");
        let mut editor = XlsxEditor::open(&bytes).expect("编辑会话打开");
        let mut model = XlsxGridModel::load(&fixture()).expect("网格模型装载");

        // 用户手势的模型面等价：点的是覆盖格 C5（显示 (4,2)），命中映射
        // 到左上 B5（4,1）后提交编辑。
        let (r, c) = canonical_cell(&model.sheets[0].merge_spans, 4, 2).expect("覆盖格有映射");
        assert_eq!((r, c), (4, 1), "C5 → B5");
        commit_cell_edit(&mut editor, &mut model, 0, r, c, "改后合并值").expect("左上值编辑落账");
        // pending 显示面：只有左上改（覆盖格不复制值——issue 明文禁令）。
        assert_eq!(model.sheets[0].cells[4][1], "改后合并值");
        assert_eq!(
            model.sheets[0].cells[4][2], "",
            "覆盖格 C5 显示面不写入重复值"
        );

        // 保存 → 重开读面（真落盘闭环）。
        let saved = editor.save_to_vec().expect("保存序列化");
        let mut reopened = XlsxEditor::open(&saved).expect("重开会话");
        let info = reopened.cell_info(0, "B5").expect("B5 在场").expect("非空");
        assert_eq!(info.text, "改后合并值", "落盘后 B5 = 新值");
        // 覆盖格 C5 读回为空（未写入重复值）。
        let covered = reopened.cell_info(0, "C5").expect("C5 可查");
        assert!(
            covered.is_none(),
            "覆盖格 C5 未被写入（得 {:?}）——禁止复制值填满合并区域",
            covered.map(|i| i.text)
        );
        // merge 范围原样保留（保存字节 → 读面比对——不污染仓内夹具）。
        let saved_book = {
            let tmp = std::env::temp_dir()
                .join(format!("mang-merge-roundtrip-{}.xlsx", std::process::id()));
            std::fs::write(&tmp, &saved).expect("临时落盘");
            let b = mdc_office_io::io::read_book_with_options(
                &tmp.to_string_lossy(),
                &ReadOptions { compat: true },
            )
            .expect("保存字节读面");
            let _ = std::fs::remove_file(&tmp);
            b
        };
        assert_eq!(
            saved_book.sheets[0].merges.len(),
            1,
            "merge 范围原样保留（数量）"
        );
        let dm = saved_book.sheets[0].merges[0];
        assert_eq!(
            (dm.start.0, dm.start.1, dm.end.0, dm.end.1),
            (4, 1, 4, 2),
            "merge 范围原样保留（B5:C5 绝对端点）"
        );
    }

    /// #584 返工：测试宿主——视图进**真实渲染树**（空 div 宿主证明
    /// 不了渲染/事件注册——审核要求：真实视图点击/双击/按键回归）。
    struct GridHost(gpui::Entity<XlsxGridView>);

    impl Render for GridHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
            gpui::div().size_full().child(self.0.clone())
        }
    }

    /// #584 返工：真实鼠标事件派发（MouseDown + MouseUp——完整命中
    /// 测试管线，走覆盖 canvas 注册的窗口级观察器 → select_at /
    /// begin_edit_at；main.rs 烟雾 + 批 11 同款 update_window 口径）。
    fn grid_click_at(
        window: &gpui::WindowHandle<GridHost>,
        cx: &mut gpui::TestAppContext,
        x: f32,
        y: f32,
        click_count: usize,
    ) {
        use gpui::{
            Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput, point, px,
        };
        let any: gpui::AnyWindowHandle = window.clone().into();
        let down = PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position: point(px(x), px(y)),
            modifiers: Modifiers::default(),
            click_count,
            first_mouse: false,
        });
        let up = PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position: point(px(x), px(y)),
            modifiers: Modifiers::default(),
            click_count,
        });
        cx.update_window(any, |_, window, cx| {
            window.dispatch_event(down, cx);
            window.dispatch_event(up, cx);
        })
        .expect("鼠标派发");
    }

    /// 合成按键（pdf_pages key_event 同构）。
    fn grid_key_event(key: &str, key_char: Option<&str>, ctrl: bool) -> KeyDownEvent {
        KeyDownEvent {
            keystroke: gpui::Keystroke {
                modifiers: gpui::Modifiers {
                    control: ctrl,
                    ..Default::default()
                },
                key: key.to_string(),
                key_char: key_char.map(|c| c.to_string()),
            },
            is_held: false,
            prefer_character_input: false,
        }
    }

    /// #584 返工：真实键盘事件派发（焦点路径——点击后视图持焦，
    /// KeyDown 经 track_focus 节点到达 handle_key；#422 同款）。
    fn grid_key_down(
        window: &gpui::WindowHandle<GridHost>,
        cx: &mut gpui::TestAppContext,
        ev: KeyDownEvent,
    ) {
        let any: gpui::AnyWindowHandle = window.clone().into();
        cx.update_window(any, |_, window, cx| {
            window.dispatch_event(gpui::PlatformInput::KeyDown(ev), cx);
        })
        .expect("键盘派发");
    }

    /// #584 返工：重绘一帧（命中盒/焦点节点随最新状态注册）。
    fn grid_redraw(window: &gpui::WindowHandle<GridHost>, cx: &mut gpui::TestAppContext) {
        let any: gpui::AnyWindowHandle = window.clone().into();
        cx.update_window(any, |_, window, cx| {
            window.draw(cx).clear(cx);
        })
        .expect("重绘");
    }

    /// #555 验收 3（#584 返工重写——真实渲染树 + 真实事件全链）：
    /// 挂载真实 XlsxGridView（渲染树内）→ **真实点击**覆盖格 C5 →
    /// 选中整区（锚 B5）→ **真实双击**进入编辑（左上原值起步）→
    /// **真实键入** → Enter 提交 → Ctrl+S 落盘 → 重读文件断言
    /// （左上改 / 范围在 / 覆盖格零写入）。
    /// 夹具复制到临时目录——save() 写回源路径，不许污染仓内黄金夹具。
    #[gpui::test]
    fn merge_covered_click_selects_edits_and_saves(cx: &mut gpui::TestAppContext) {
        let src = fixture();
        let tmp = std::env::temp_dir().join(format!("mang-merge-grid-{}.xlsx", std::process::id()));
        std::fs::copy(&src, &tmp).expect("夹具复制到临时目录");
        let model = XlsxGridModel::load(&tmp).expect("网格模型装载");
        assert_eq!(
            model.sheets[0].merge_spans,
            vec![smoke_merge_span()],
            "临时夹具合并范围在档（前置）"
        );

        // #584 返工：真实渲染树挂载需要主题全局（视图 render 读
        // cx.theme()——theme 是 workspace 依赖 crate；渲染期派生还要
        // settings provider 全局，#422 WiringHost 同款双注册）。
        cx.update(|app| {
            theme::init(theme::LoadThemes::JustBase, app);
            theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
        });

        // 真实渲染树挂载（GridHost = size_full + child 视图——渲染/
        // 命中盒/焦点节点全部注册，非空 div 直调可比）。
        let mut slot: Option<gpui::Entity<XlsxGridView>> = None;
        let window = cx.add_window(|_window, cx| {
            let v = cx.new(|cx| XlsxGridView::new(model, tmp.clone(), cx));
            slot = Some(v.clone());
            GridHost(v)
        });
        let view = slot.expect("宿主构造即置");

        // 覆盖格 C5（显示 (4,2)）的窗口坐标：x = 行号槽 + 列 2 偏移
        //（Host 全幅 → 视图原点 = (0,0)，canvas 局部 = 窗口坐标）；
        // y = 行 4 中部（y/ROW_H 整除命中行 4——表格头/表签只遮影行
        // 0-2，行 4 起精确）。
        let (click_x, click_y) = window
            .update(cx, |_host, _window, cx| {
                view.update(cx, |v, _| {
                    let (off2, w2) = (v.col_offsets[2], v.col_widths[2]);
                    (
                        GUTTER_W + off2 + (w2 / 2.0).min(5.0),
                        4.0 * ROW_H + ROW_H / 2.0,
                    )
                })
            })
            .expect("读坐标");

        // ① 真实单击覆盖格 C5 → 选中整区（锚 B5）+ 落焦（点击即焦）。
        grid_redraw(&window, cx);
        grid_click_at(&window, cx, click_x, click_y, 1);

        window
            .update(cx, |_host, _window, cx| {
                view.update(cx, |v, _| {
                    assert_eq!(
                        v.selected_cell(),
                        Some((4, 1)),
                        "真实点击 C5 → 选中锚 B5（整区选择）"
                    );
                    assert_eq!(v.selected_a1().as_deref(), Some("B5"));
                });
            })
            .expect("断言选中");

        // ② 真实双击覆盖格 C5 → 进入编辑（锚左上，缓冲 = 左上原值）。
        grid_redraw(&window, cx);
        grid_click_at(&window, cx, click_x, click_y, 2);
        window
            .update(cx, |_host, _window, cx| {
                view.update(cx, |v, _| {
                    let ed = v.editing.as_ref().expect("真实双击进入编辑态");
                    assert_eq!((ed.row, ed.col), (4, 1), "编辑锚 = 左上 B5");
                    assert_eq!(ed.buffer, "外部资料区", "缓冲以左上原值起步");
                });
            })
            .expect("断言编辑态");

        // ③ 真实键入「新」→ Enter 提交（焦点路径——handle_key 全链）。
        grid_redraw(&window, cx);
        grid_key_down(&window, cx, grid_key_event("新", Some("新"), false));
        window
            .update(cx, |_host, _window, cx| {
                view.update(cx, |v, _| {
                    let ed = v.editing.as_ref().expect("编辑态保持");
                    assert!(ed.buffer.contains("新"), "真实键入进缓冲：{}", ed.buffer);
                });
            })
            .expect("断言键入");
        grid_key_down(&window, cx, grid_key_event("enter", None, false));
        window
            .update(cx, |_host, _window, cx| {
                view.update(cx, |v, _| {
                    assert!(v.editing.is_none(), "Enter 提交退出编辑态");
                    assert!(v.is_modified(), "提交后脏标记");
                });
            })
            .expect("断言提交");

        // ④ 真实 Ctrl+S 落盘。
        grid_redraw(&window, cx);
        grid_key_down(&window, cx, grid_key_event("s", None, true));
        window
            .update(cx, |_host, _window, cx| {
                view.update(cx, |v, _| {
                    assert!(!v.is_modified(), "Ctrl+S 落盘后脏清除");
                });
            })
            .expect("断言保存");

        // ⑤ 重读落盘文件：左上新值 + 范围保留 + 覆盖格零写入。
        let saved_bytes = std::fs::read(&tmp).expect("落盘文件可读");
        let mut reopened = XlsxEditor::open(&saved_bytes).expect("重开会话");
        let info = reopened.cell_info(0, "B5").expect("B5 可查").expect("非空");
        assert!(
            info.text.contains("新") && info.text.contains("外部资料区"),
            "B5 = 键入后的完整值（得 {}）",
            info.text
        );
        assert!(
            reopened.cell_info(0, "C5").expect("C5 可查").is_none(),
            "覆盖格 C5 零写入（禁止复制值填满合并区域）"
        );
        let saved_book = {
            let tmp2 =
                std::env::temp_dir().join(format!("mang-merge-verify-{}.xlsx", std::process::id()));
            std::fs::write(&tmp2, &saved_bytes).expect("验证临时落盘");
            let b = mdc_office_io::io::read_book_with_options(
                &tmp2.to_string_lossy(),
                &ReadOptions { compat: true },
            )
            .expect("读面");
            let _ = std::fs::remove_file(&tmp2);
            b
        };
        let dm = saved_book.sheets[0].merges[0];
        assert_eq!(
            (dm.start.0, dm.start.1, dm.end.0, dm.end.1),
            (4, 1, 4, 2),
            "merge 范围原样保留（B5:C5 绝对端点）"
        );
        let _ = std::fs::remove_file(&tmp);
    }

    /// #584 返工：合并区左上在数据包围盒**外**（审核复现场景——
    /// 空合并格 E5:G7 + 数据 F6/G7 → 包围盒从 F6 起）——真实写面
    /// 构造夹具 → 模型装载（锚 E5 不漂移）→ 编辑覆盖格落账 E5 →
    /// 保存重读（E5 新值 / 覆盖格零写入 / merge 原样）。
    #[test]
    fn anchor_outside_box_edit_writes_original_e5() {
        use mdc_office_io::write::{CellValue, WorkbookWriter};

        // 构造：合并 E5:G7（绝对 (4,4)-(6,6)，文本空——锚在包围盒外）
        // + 数据 F6=1、G7=2（包围盒 (5,5)-(6,6)）。顺序关键：先 merge
        // 后 write——rust_xlsxwriter merge_range 擦除范围内既有写入
        //（先写后合并 = 数据被清空）；先合并后写入 = 覆盖格带值
        //（真实场景：被合并覆盖但仍有值的格——typed 网格过滤空格后
        // 包围盒从 F6 起，与审核复现场景一致）。
        let mut w = WorkbookWriter::new();
        let mut s = w.add_sheet("锚外").expect("建表");
        s.merge(4, 4, 6, 6, "", None).expect("合并 E5:G7（空文本）");
        s.write(5, 5, CellValue::Number(1.), None).expect("F6=1");
        s.write(6, 6, CellValue::Number(2.), None).expect("G7=2");
        let bytes = w.save_to_vec().expect("写盘");
        let tmp = std::env::temp_dir().join(format!("mang-anchor-out-{}.xlsx", std::process::id()));
        std::fs::write(&tmp, &bytes).expect("临时落盘");

        // 模型装载：包围盒 F6 起 + 可见段 (0,0)-(1,1) + 锚 E5 (4,4)。
        let model = XlsxGridModel::load(&tmp).expect("模型装载");
        let sheet = &model.sheets[0];
        assert_eq!(
            (sheet.start_row, sheet.start_col),
            (5, 5),
            "包围盒从 F6 起（merge 空文本不扩盒）"
        );
        let span = sheet.merge_spans.first().expect("合并范围在档");
        assert_eq!((span.row_start, span.col_start), (0, 0), "可见段起点 = F6");
        assert_eq!((span.row_end, span.col_end), (1, 1), "可见段末点 = G7");
        assert_eq!(span.abs_anchor, (4, 4), "原始绝对锚 = E5（不随裁剪漂移）");

        // 编辑覆盖格（可见 (1,1) = G7 位——canonical → 可见锚 (0,0)）：
        // 提交必须落账 **E5**（绝对锚），不是 F6（显示坐标换算的错值）。
        let mut editor = XlsxEditor::open(&bytes).expect("编辑会话");
        let mut model2 = model.clone();
        let (r, c) =
            canonical_cell(&model2.sheets[0].merge_spans, 1, 1).expect("覆盖格有可见锚映射");
        assert_eq!((r, c), (0, 0), "可见锚 = F6 显示坐标");
        // selected_a1 面（A1 引用锚 E5）。
        let a1 = format!("{}{}", col_letter(span.abs_anchor.1), span.abs_anchor.0 + 1);
        assert_eq!(a1, "E5", "A1 引用 = 原始绝对锚");
        commit_cell_edit(&mut editor, &mut model2, 0, r, c, "锚外新值")
            .expect("覆盖格编辑落账（锚 E5）");

        // 保存 → 重读：E5 有新值；F6/G7（覆盖格）保持原值不重写；
        // merge 范围原样。
        let saved = editor.save_to_vec().expect("保存");
        let mut reopened = XlsxEditor::open(&saved).expect("重开");
        let e5 = reopened.cell_info(0, "E5").expect("E5 可查").expect("非空");
        assert_eq!(e5.text, "锚外新值", "落账在原始绝对锚 E5（非 F6）");
        let f6 = reopened.cell_info(0, "F6").expect("F6 可查").expect("非空");
        assert_eq!(f6.text, "1", "覆盖格 F6 保持原值（零写入）");
        let g7 = reopened.cell_info(0, "G7").expect("G7 可查").expect("非空");
        assert_eq!(g7.text, "2", "覆盖格 G7 保持原值（零写入）");
        let book = {
            let tmp2 = std::env::temp_dir()
                .join(format!("mang-anchor-verify-{}.xlsx", std::process::id()));
            std::fs::write(&tmp2, &saved).expect("验证落盘");
            let b = mdc_office_io::io::read_book_with_options(
                &tmp2.to_string_lossy(),
                &ReadOptions { compat: true },
            )
            .expect("读面");
            let _ = std::fs::remove_file(&tmp2);
            b
        };
        let dm = book.sheets[0].merges[0];
        assert_eq!(
            (dm.start.0, dm.start.1, dm.end.0, dm.end.1),
            (4, 4, 6, 6),
            "merge 范围原样保留（E5:G7）"
        );
        let _ = std::fs::remove_file(&tmp);
    }
    mod value_fix_io_tests {
        use super::*;

        pub(super) fn fixture_bytes(visible_top: bool) -> Vec<u8> {
            use mdc_office_io::write::{CellValue, WorkbookWriter};
            let mut book = WorkbookWriter::new();
            {
                let mut sheet = book.add_sheet("Anchor values").unwrap();
                sheet.merge(4, 4, 6, 6, "", None).unwrap();
                if visible_top {
                    sheet
                        .write(0, 5, CellValue::Text("marker".into()), None)
                        .unwrap();
                    sheet
                        .write(4, 5, CellValue::Text("covered-value".into()), None)
                        .unwrap();
                }
                sheet
                    .write(5, 5, CellValue::Text("covered-value".into()), None)
                    .unwrap();
                sheet
                    .write(6, 6, CellValue::Text("covered-tail".into()), None)
                    .unwrap();
            }
            book.save_to_vec().unwrap()
        }

        fn model(bytes: &[u8]) -> XlsxGridModel {
            let book =
                read_book_from_bytes_with_options(bytes, &ReadOptions { compat: true }).unwrap();
            XlsxGridModel::from_book(&book)
        }

        #[test]
        fn empty_original_anchor_is_displayed_and_prefilled_as_empty() {
            let bytes = fixture_bytes(false);
            let model = model(&bytes);
            let editor = XlsxEditor::open(&bytes).unwrap();
            let values = merge_values_from_session(&model, &editor).unwrap();
            assert_eq!(
                (model.sheets[0].start_row, model.sheets[0].start_col),
                (5, 5)
            );
            assert_eq!(display_cell_value(&model, &values, 0, 0, 0), "");
            assert_eq!(model.sheets[0].cells[0][0], "covered-value");
        }

        #[test]
        fn confirming_without_input_does_not_copy_the_covered_value() {
            let bytes = fixture_bytes(false);
            let mut model = model(&bytes);
            let mut editor = XlsxEditor::open(&bytes).unwrap();
            let mut values = merge_values_from_session(&model, &editor).unwrap();
            let value = display_cell_value(&model, &values, 0, 0, 0);
            commit_cell_edit(&mut editor, &mut model, 0, 0, 0, &value).unwrap();
            refresh_committed_merge_value(&model, &mut values, 0, 0, 0, &value);
            let mut reopened = XlsxEditor::open(&editor.save_to_vec().unwrap()).unwrap();
            assert!(
                reopened
                    .cell_info(0, "E5")
                    .unwrap()
                    .map(|c| c.text)
                    .unwrap_or_default()
                    .is_empty()
            );
            assert_eq!(
                reopened.cell_info(0, "F6").unwrap().unwrap().text,
                "covered-value"
            );
            assert_eq!(
                reopened.cell_info(0, "G7").unwrap().unwrap().text,
                "covered-tail"
            );
        }

        #[test]
        fn consecutive_edits_and_session_restore_keep_anchor_and_covered_data_separate() {
            let bytes = fixture_bytes(false);
            let mut model = model(&bytes);
            let mut values = initial_merge_values(&model);
            let mut editor = XlsxEditor::open(&bytes).unwrap();
            commit_cell_edit(&mut editor, &mut model, 0, 0, 0, "first").unwrap();
            refresh_committed_merge_value(&model, &mut values, 0, 0, 0, "first");
            assert_eq!(display_cell_value(&model, &values, 0, 0, 0), "first");
            assert_eq!(model.sheets[0].cells[0][0], "covered-value");
            let restored = merge_values_from_session(&model, &editor).unwrap();
            assert_eq!(display_cell_value(&model, &restored, 0, 0, 0), "first");
            commit_cell_edit(&mut editor, &mut model, 0, 0, 0, "second").unwrap();
            refresh_committed_merge_value(&model, &mut values, 0, 0, 0, "second");
            assert_eq!(display_cell_value(&model, &values, 0, 0, 0), "second");
            let saved = editor.save_to_vec().unwrap();
            let mut reopened = XlsxEditor::open(&saved).unwrap();
            assert_eq!(reopened.cell_info(0, "E5").unwrap().unwrap().text, "second");
            assert_eq!(
                reopened.cell_info(0, "F6").unwrap().unwrap().text,
                "covered-value"
            );
            assert_eq!(
                reopened.cell_info(0, "G7").unwrap().unwrap().text,
                "covered-tail"
            );
            let book =
                read_book_from_bytes_with_options(&saved, &ReadOptions { compat: true }).unwrap();
            assert_eq!(
                (book.sheets[0].merges[0].start, book.sheets[0].merges[0].end),
                ((4, 4), (6, 6))
            );
        }
    }

    // 实际窗口事件路径；不把本case冒称本地真机/截图验证。
    #[gpui::test]
    fn original_anchor_double_click_reedit_session_restore_and_save(cx: &mut gpui::TestAppContext) {
        let bytes = value_fix_io_tests::fixture_bytes(true);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("merge-values-{}-{nonce}.xlsx", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        let model = XlsxGridModel::load(&path).unwrap();
        assert_eq!(model.sheets[0].start_col, 5); // E5原锚在F列起的数据盒外。
        cx.update(|app| {
            theme::init(theme::LoadThemes::JustBase, app);
            theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
        });
        let mut slot = None;
        let window = cx.add_window(|_, cx| {
            let view = cx.new(|cx| XlsxGridView::new(model, path.clone(), cx));
            slot = Some(view.clone());
            GridHost(view)
        });
        let mut view = slot.unwrap();
        let (x, y) = window
            .update(cx, |_, _, cx| {
                view.update(cx, |v, _| {
                    (GUTTER_W + v.col_offsets[0] + 5.0, 4.0 * ROW_H + ROW_H / 2.0)
                })
            })
            .unwrap();
        grid_redraw(&window, cx);
        grid_click_at(&window, cx, x, y, 2);
        window
            .update(cx, |_, _, cx| {
                view.update(cx, |v, _| {
                    assert_eq!(v.selected_a1().as_deref(), Some("E5"));
                    assert_eq!(
                        v.editing.as_ref().unwrap().buffer,
                        "",
                        "空E5不应预填F5/F6覆盖值"
                    );
                    assert_eq!(display_cell_value(&v.model, &v.merge_values, 0, 4, 0), "");
                })
            })
            .unwrap();
        grid_key_down(&window, cx, grid_key_event("enter", None, false));
        window
            .update(cx, |_, _, cx| {
                view.update(cx, |v, _| {
                    assert!(!v.is_modified(), "确认未改不置脏");
                })
            })
            .unwrap();
        for (input, expected_before, expected_after) in
            [("first", "", "first"), ("-second", "first", "first-second")]
        {
            grid_redraw(&window, cx);
            grid_click_at(&window, cx, x, y, 2);
            window
                .update(cx, |_, _, cx| {
                    view.update(cx, |v, _| {
                        assert_eq!(v.editing.as_ref().unwrap().buffer, expected_before);
                    })
                })
                .unwrap();
            for ch in input.chars() {
                let character = ch.to_string();
                grid_key_down(
                    &window,
                    cx,
                    grid_key_event(&character, Some(&character), false),
                );
            }
            grid_key_down(&window, cx, grid_key_event("enter", None, false));
            window
                .update(cx, |_, _, cx| {
                    view.update(cx, |v, _| {
                        assert_eq!(
                            display_cell_value(&v.model, &v.merge_values, 0, 4, 0),
                            expected_after
                        );
                        assert_eq!(
                            v.model.sheets[0].cells[4][0], "covered-value",
                            "F5模型槽保持原值"
                        );
                    })
                })
                .unwrap();
        }
        // 模拟Shell会话银行：磁盘model仍旧，pending会话必须提供新E5值。
        let (session, dirty) = window
            .update(cx, |_, _, cx| {
                view.update(cx, |v, _| {
                    let session = v.take_session().unwrap();
                    (session, v.is_modified())
                })
            })
            .unwrap();
        let old_model = XlsxGridModel::load(&path).unwrap();
        let next = window
            .update(cx, |_, _, cx| {
                cx.new(|cx| XlsxGridView::new_with_session(old_model, path.clone(), session, cx))
            })
            .unwrap();
        window
            .update(cx, |host, _, cx| {
                next.update(cx, |v, _| v.set_modified(dirty));
                host.0 = next.clone();
                cx.notify();
            })
            .unwrap();
        view = next;
        grid_redraw(&window, cx);
        grid_click_at(&window, cx, x, y, 2);
        window
            .update(cx, |_, _, cx| {
                view.update(cx, |v, _| {
                    assert_eq!(
                        v.editing.as_ref().unwrap().buffer,
                        "first-second",
                        "旧model不得覆盖会话E5新值"
                    );
                })
            })
            .unwrap();
        grid_key_down(&window, cx, grid_key_event("enter", None, false));
        grid_redraw(&window, cx);
        grid_key_down(&window, cx, grid_key_event("s", None, true));
        let saved = std::fs::read(&path).unwrap();
        let mut reopened = XlsxEditor::open(&saved).unwrap();
        assert_eq!(
            reopened.cell_info(0, "E5").unwrap().unwrap().text,
            "first-second"
        );
        for reference in ["F5", "F6"] {
            assert_eq!(
                reopened.cell_info(0, reference).unwrap().unwrap().text,
                "covered-value"
            );
        }
        assert_eq!(
            reopened.cell_info(0, "G7").unwrap().unwrap().text,
            "covered-tail"
        );
        let book =
            read_book_from_bytes_with_options(&saved, &ReadOptions { compat: true }).unwrap();
        assert_eq!(
            (book.sheets[0].merges[0].start, book.sheets[0].merges[0].end),
            ((4, 4), (6, 6))
        );
        std::fs::remove_file(path).unwrap();
    }
}
