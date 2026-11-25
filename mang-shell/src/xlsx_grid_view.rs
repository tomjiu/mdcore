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
#[path = "xlsx_grid_view_tests.rs"]
mod tests;
