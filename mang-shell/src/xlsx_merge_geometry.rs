//! #555：XLSX 合并单元格几何——范围携带、guard 裁剪、命中映射纯函数层。
//!
//! # 定位（issue #555 范围声明）
//!
//! `xlsx_grid_view.rs` 此前把 `office-io` 的 `sheet.merges`（绝对坐标
//! `Dimensions` 列表）缩成数量丢弃实际范围——合并区在桌面网格视图里
//! 渲染成「左上一格 + 一排普通空格」，点击覆盖格选中/编辑的是覆盖格
//! 自身（无值语义）。本模块把合并几何独立成**纯函数层**（零 GPUI
//! 依赖——CI 无 GUI 同样真跑），`xlsx_grid_view.rs` 只做模型携带
//! （`GridSheet::merge_spans`）与绘制/选择/编辑接线。
//!
//! # 坐标语义（两层换算，方向固定）
//!
//! ```text
//! 绝对坐标（office-io Dimensions：工作表 0 基，含端点）
//!   │  MergeSpan::clip_to_box（减包围盒起点 + guard 盒裁剪）
//!   ▼
//! 显示坐标（本模块唯一坐标系：模型包围盒相对 0 基，含端点）
//!   │  span_px（+ col_offsets / col_widths）
//!   ▼
//! 像素坐标（渲染/命中——与既有网格几何同源口径）
//! ```
//!
//! # Excel 语义锚（显示 = 左上单点）
//!
//! - **值**只存在左上格（写面 `merge_range` 物化 blank，读面覆盖格
//!   空）——编辑合并格只改左上，**禁止复制值填满合并区域**（issue
//!   #555 明文禁令，e2e 断言覆盖格不写入重复值）；
//! - **选择/编辑命中**覆盖格 → 映射左上（[`canonical_cell`]）；
//! - **绘制**合并区 = 一个宽格（跨行跨列单视觉格），内部网格线
//!   抑制，滚动/视窗只覆盖范围中部时边框仍正确（`intersects_window`
//!   + 行分段边框拼接）。

// ---------------------------------------------------------------------------
// 数据（纯显示坐标——无 calamine/gpui 依赖）
// ---------------------------------------------------------------------------

/// 合并范围（可见段 = 显示坐标含端点 + 原始绝对锚）。
///
/// 由绝对 `Dimensions` 经 [`MergeSpan::clip_to_box`] 换算 + guard 盒
/// 裁剪而来；越出渲染护栏（行 ≤1000 / 列 ≤64）的可见部分被裁掉，
/// 整段在护栏外的直接丢弃（`None`）。
///
/// # #584 返工：锚与可见范围分离（两套坐标不同源）
///
/// - **可见范围**（`row_start`/`col_start` 起）——渲染/命中/高亮用
///   （guard 盒内显示坐标）；
/// - **`abs_anchor`**（原始绝对左上，**不裁剪**）——值存储位/
///   编辑提交/A1 引用用（Excel 语义：合并区左上可在数据包围盒
///   之外[空合并格场景：值锚在 E5 而包围盒从 F6 起]，此时可见
///   起点与值锚不同格——锚一律指原始绝对左上，不随裁剪漂移）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MergeSpan {
    /// 可见段首行（含——显示坐标，guard 盒内）。
    pub row_start: usize,
    /// 可见段末行（含）。
    pub row_end: usize,
    /// 可见段首列（含——显示坐标，guard 盒内）。
    pub col_start: usize,
    /// 可见段末列（含）。
    pub col_end: usize,
    /// 原始绝对左上（0 基工作表坐标，不裁剪——见类型文档；
    /// 可见范围被裁剪时与可见起点不同格）。
    pub abs_anchor: (u32, u32),
}

impl MergeSpan {
    /// 绝对范围 → 显示坐标 + guard 盒裁剪。
    ///
    /// - `abs_start` / `abs_end`：office-io `Dimensions` 的 `(row, col)`
    ///   绝对端点（0 基含端点，start ≤ end 由读面保证；防御乱序 = None）；
    /// - `box_row` / `box_col`：模型包围盒起点（`GridSheet::start_row/
    ///   start_col`）；
    /// - `box_rows` / `box_cols`：渲染维度（护栏后）。
    ///
    /// 裁剪规则：显示区间与 `[0, box_rows) × [0, box_cols)` 求交；交集
    /// 空（整段在盒外 / 盒自身为空）= `None`。**部分在盒内的范围保留
    /// 盒内段**——护栏截断表上合并区仍按可见部分正确渲染（issue #555
    /// 「视窗只覆盖合并范围中部时仍保持正确边框」的模型层同构面）。
    pub fn clip_to_box(
        abs_start: (u32, u32),
        abs_end: (u32, u32),
        box_row: u32,
        box_col: u32,
        box_rows: usize,
        box_cols: usize,
    ) -> Option<Self> {
        if abs_start.0 > abs_end.0 || abs_start.1 > abs_end.1 {
            return None; // 乱序防御（读面不该出现——出现即丢弃）
        }
        let rows = box_rows as i64;
        let cols = box_cols as i64;
        if rows <= 0 || cols <= 0 {
            return None; // 空盒（空表 / 护栏零维）——无渲染面
        }
        // 显示坐标（i64 中转——合并范围可在数据包围盒外，显示坐标为负）。
        let r0 = abs_start.0 as i64 - box_row as i64;
        let c0 = abs_start.1 as i64 - box_col as i64;
        let r1 = abs_end.0 as i64 - box_row as i64;
        let c1 = abs_end.1 as i64 - box_col as i64;
        // guard 盒裁剪（钳到 [0, 维度-1]——交集空返回 None）。
        let r0c = r0.clamp(0, rows - 1);
        let r1c = r1.clamp(0, rows - 1);
        let c0c = c0.clamp(0, cols - 1);
        let c1c = c1.clamp(0, cols - 1);
        // #584 返工：可见段裁剪 + 原始绝对锚分离——锚不随裁剪漂移
        //（空合并格场景：值锚在 E5 而包围盒从 F6 起，编辑/A1 引用
        // 一律锚 E5——不能把裁剪后可见起点当锚，否则错指 F6）。
        if r0c > r1 || r1c < r0 || c0c > c1 || c1c < c0 {
            return None; // 原始区间与盒不相交（钳制值越原始端点 = 无交集）
        }
        Some(Self {
            row_start: r0c as usize,
            row_end: r1c as usize,
            col_start: c0c as usize,
            col_end: c1c as usize,
            abs_anchor: (abs_start.0, abs_start.1),
        })
    }

    /// 覆盖判定（含左上——列宽计算排除 / 渲染跳格都用它：左上的文本
    /// 由 span 宽格承载，不该再撑独立列宽）。
    pub fn contains(&self, row: usize, col: usize) -> bool {
        row >= self.row_start && row <= self.row_end && col >= self.col_start && col <= self.col_end
    }

    /// 左上判定（值/选择/编辑锚点）。
    pub fn is_top_left(&self, row: usize, col: usize) -> bool {
        row == self.row_start && col == self.col_start
    }
}

// ---------------------------------------------------------------------------
// 命中映射（Excel 语义：覆盖格 → 左上）
// ---------------------------------------------------------------------------

/// 覆盖格 → 所属合并范围（首个命中——合法 xlsx 无重叠，防御面取首）。
pub fn span_at(merges: &[MergeSpan], row: usize, col: usize) -> Option<&MergeSpan> {
    merges.iter().find(|m| m.contains(row, col))
}

/// 覆盖判定（含左上——[`MergeSpan::contains`] 的列表面）。
pub fn is_covered(merges: &[MergeSpan], row: usize, col: usize) -> bool {
    merges.iter().any(|m| m.contains(row, col))
}

/// 命中映射：坐标 → 合并区可见锚（显示坐标——渲染/高亮链；**编辑
/// 提交/A1 引用走 [`span_anchor_abs` 的原始绝对锚**，两链分源）。
pub fn canonical_cell(merges: &[MergeSpan], row: usize, col: usize) -> Option<(usize, usize)> {
    span_at(merges, row, col).map(|m| (m.row_start, m.col_start))
}

/// 命中映射 → 原始绝对锚（0 基工作表坐标，不裁剪——值存储位）。
/// #584 返工新增：合并区左上可在数据包围盒外（空合并格场景），
/// 编辑提交/`selected_a1` 一律锚此（Excel 语义），不随可见裁剪漂移。
pub fn span_anchor_abs(merges: &[MergeSpan], row: usize, col: usize) -> Option<(u32, u32)> {
    span_at(merges, row, col).map(|m| m.abs_anchor)
}

// ---------------------------------------------------------------------------
// 渲染几何（行级 span 查询 + 像素换算 + 视窗相交 + 边框分段）
// ---------------------------------------------------------------------------

/// 某行需要绘制的 span 列表（行区间相交即入列——绘制面按行切片，
/// uniform_list 行渲染回调的消费口径）。
pub fn spans_in_row<'a>(merges: &'a [MergeSpan], row: usize) -> Vec<&'a MergeSpan> {
    merges
        .iter()
        .filter(|m| row >= m.row_start && row <= m.row_end)
        .collect()
}

/// span 像素几何：(x 起点, 总宽)——`x = col_offsets[col_start]`，
/// `宽 = col_offsets[col_end] + col_widths[col_end] - col_offsets[col_start]`
/// （内部 COL_GAP 自然计入——跨列间隙属于合并区内部，不画线）。
/// 下标越界（护栏外的脏 span）= `None`（调用方跳过，不 panic）。
pub fn span_px(
    offsets: &[f32],
    widths: &[f32],
    col_start: usize,
    col_end: usize,
) -> Option<(f32, f32)> {
    if col_start > col_end || col_end >= offsets.len() || col_end >= widths.len() {
        return None;
    }
    let x = *offsets.get(col_start)?;
    let right = *offsets.get(col_end)? + *widths.get(col_end)?;
    if right < x {
        return None; // 防御：offsets 非单调（不该发生）
    }
    Some((x, right - x))
}

/// 视窗相交：span 列区间与可见列窗口 `[first, end)` 是否有公共部分
/// （**横跨视窗**的 span——左端滚出窗外/右端未入窗，任一部分可见即
/// 需绘制；绝对定位天然承载部分可见，无需拆分）。
/// #584 返工：空窗口（`first >= end`，含 `[3,3)` 形态）恒 false
/// ——空窗口零相交（裁剪后 span 起点也可能落在窗口内例：span
/// (2,5) vs [3,3)——旧实现 `2 < 3 && 5 >= 3` 误判 true）。
pub fn intersects_window(col_start: usize, col_end: usize, first: usize, end: usize) -> bool {
    if first >= end {
        return false; // 空窗口零相交
    }
    col_start < end && col_end >= first
}

/// span 在指定行的边框分段（跨行 span 的选择/编辑态四边框按行拼接：
/// 首行画 top、末行画 bottom、每行都画 left+right——拼起来 = 整区
/// 完整外框，行间不画横线 = 合并区视觉整体）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpanEdges {
    /// 该行画上边（首行）。
    pub top: bool,
    /// 该行画下边（末行）。
    pub bottom: bool,
}

/// 指定行属于 span 的边框分段（行不在 span 内 = `None`）。
pub fn span_edges(span: &MergeSpan, row: usize) -> Option<SpanEdges> {
    if !span.contains(row, span.col_start) {
        return None;
    }
    Some(SpanEdges {
        top: row == span.row_start,
        bottom: row == span.row_end,
    })
}

// ---------------------------------------------------------------------------
// 纯函数面单测（零 GPUI 依赖——CI 无 GUI 同样真跑）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── 换算与裁剪 ─────────────────────────────────────────────

    #[test]
    fn clip_to_box_maps_non_a1_origin() {
        // 非 A1 起点：数据包围盒 (3,2) 起（如 C4），合并 C4:E6 绝对
        // (3,2)-(5,4) → 显示 (0,0)-(2,2)。
        let s = MergeSpan::clip_to_box((3, 2), (5, 4), 3, 2, 10, 10).expect("盒内映射");
        assert_eq!(
            s,
            MergeSpan {
                row_start: 0,
                row_end: 2,
                col_start: 0,
                col_end: 2,
                abs_anchor: (3, 2),
            },
            "绝对 → 显示坐标 = 减包围盒起点（锚 = 原始绝对左上）"
        );
        // 左上判定锚（可见面——渲染/高亮链）。
        assert!(s.is_top_left(0, 0));
        assert!(!s.is_top_left(1, 0), "覆盖行不是左上");
        assert!(!s.is_top_left(0, 1), "覆盖列不是左上");
    }

    /// #584 返工：合并区左上在数据包围盒**外**（空合并格场景）——
    /// 可见段被裁剪，但原始绝对锚不漂移（编辑/A1 引用一律锚原点）。
    #[test]
    fn anchor_outside_box_keeps_absolute_origin() {
        // 审核复现场景：合并区 E5:G7（绝对 (4,4)-(6,6)），包围盒从
        // F6（(5,5)）起、护栏 10×10 → 可见段 (0,0)-(1,1)，
        // 锚 = E5 (4,4)——不能把可见起点 (0,0)（=F6）当锚。
        let s = MergeSpan::clip_to_box((4, 4), (6, 6), 5, 5, 10, 10).expect("部分在盒内");
        assert_eq!(
            (s.row_start, s.col_start),
            (0, 0),
            "可见段起点 = F6 显示坐标"
        );
        assert_eq!((s.row_end, s.col_end), (1, 1), "可见段末点 = G7 显示坐标");
        assert_eq!(s.abs_anchor, (4, 4), "原始绝对锚 = E5（不随裁剪漂移）");
        let merges = [s];
        // 可见锚（渲染/高亮链）：覆盖格 → 可见左上 (0,0)。
        assert_eq!(canonical_cell(&merges, 1, 1), Some((0, 0)));
        // 绝对锚（编辑/A1 链）：覆盖格 → 原始左上 E5——两链分源。
        assert_eq!(span_anchor_abs(&merges, 1, 1), Some((4, 4)));
        assert_eq!(
            span_anchor_abs(&merges, 0, 0),
            Some((4, 4)),
            "可见起点也锚 E5"
        );
        assert_eq!(span_anchor_abs(&merges, 2, 0), None, "span 外无锚");
    }

    #[test]
    fn clip_to_box_clips_partial_and_drops_full_out() {
        // 部分在盒内：合并 D5:H8（绝对 (4,3)-(7,7)），盒 (3,2) 起、
        // 5 行 5 列 → 显示区间 (1,1)-(4,4) 与盒 [0,5) 全交（不裁）。
        let s = MergeSpan::clip_to_box((4, 3), (7, 7), 3, 2, 5, 5).expect("全交");
        assert_eq!((s.row_start, s.row_end), (1, 4));
        assert_eq!((s.col_start, s.col_end), (1, 4));

        // 行越护栏：盒 3 行，合并绝对 (3,0)-(9,0) → 显示 0..9 与
        // [0,3) 交 = 0..2（尾部裁掉，保留可见段）。
        let s = MergeSpan::clip_to_box((3, 0), (9, 0), 3, 0, 3, 4).expect("行部分在盒");
        assert_eq!((s.row_start, s.row_end), (0, 2), "护栏外的行段被裁");

        // 列越护栏：盒 4 列，合并绝对 (3,2)-(3,7) → 显示 0..5 与
        // [0,4) 交 = 0..3。
        let s = MergeSpan::clip_to_box((3, 2), (3, 7), 3, 2, 4, 4).expect("列部分在盒");
        assert_eq!((s.col_start, s.col_end), (0, 3), "护栏外的列段被裁");

        // 整段在盒外（合并完全在数据包围盒上方/左方——空壳合并）。
        assert!(
            MergeSpan::clip_to_box((0, 0), (2, 1), 3, 2, 10, 10).is_none(),
            "整段在盒外 = 丢弃"
        );
        // 整段越护栏右/下。
        assert!(
            MergeSpan::clip_to_box((20, 0), (25, 3), 3, 0, 5, 4).is_none(),
            "整段越行护栏 = 丢弃"
        );
        // 空盒（空表）。
        assert!(MergeSpan::clip_to_box((0, 0), (2, 2), 0, 0, 0, 0).is_none());
        // 乱序防御。
        assert!(MergeSpan::clip_to_box((5, 2), (3, 2), 3, 2, 10, 10).is_none());
    }

    #[test]
    fn clip_to_box_single_cell_and_full_sheet() {
        // 单格「合并」（1×1——异常但防御面：正常渲染为普通格）。
        let s = MergeSpan::clip_to_box((0, 0), (0, 0), 0, 0, 5, 5).expect("1×1");
        assert_eq!(
            (s.row_start, s.row_end, s.col_start, s.col_end),
            (0, 0, 0, 0)
        );
        // A1 起整表：包围盒 (0,0)。
        let s = MergeSpan::clip_to_box((4, 1), (4, 2), 0, 0, 10, 10).expect("A1 表");
        assert_eq!((s.row_start, s.col_start, s.col_end), (4, 1, 2));
    }

    // ── 命中映射 ───────────────────────────────────────────────

    #[test]
    fn canonical_maps_covered_to_top_left() {
        let merges = vec![MergeSpan {
            row_start: 2,
            row_end: 4,
            col_start: 1,
            col_end: 3,
            abs_anchor: (2, 1),
        }];
        // 覆盖格（含右下角）→ 左上。
        assert_eq!(canonical_cell(&merges, 2, 1), Some((2, 1)), "左上映射自身");
        assert_eq!(canonical_cell(&merges, 3, 2), Some((2, 1)), "中部覆盖格");
        assert_eq!(canonical_cell(&merges, 4, 3), Some((2, 1)), "右下角覆盖格");
        // 非覆盖格 → None（调用方保持原坐标——单格语义零漂移）。
        assert_eq!(canonical_cell(&merges, 1, 1), None, "上邻行不覆盖");
        assert_eq!(canonical_cell(&merges, 5, 3), None, "下邻行不覆盖");
        assert_eq!(canonical_cell(&merges, 2, 0), None, "左邻列不覆盖");
        assert_eq!(canonical_cell(&merges, 2, 4), None, "右邻列不覆盖");
        // 空表合并列表。
        assert_eq!(canonical_cell(&[], 0, 0), None);
    }

    #[test]
    fn covered_includes_top_left_for_width_exclusion() {
        let merges = vec![MergeSpan {
            row_start: 1,
            row_end: 1,
            col_start: 1,
            col_end: 2,
            abs_anchor: (1, 1),
        }];
        // is_covered 含左上——列宽计算把整个 span 区排除（左上文本由
        // span 宽格承载，不撑单列）。
        assert!(is_covered(&merges, 1, 1), "左上也算覆盖（排除列宽）");
        assert!(is_covered(&merges, 1, 2), "覆盖格");
        assert!(!is_covered(&merges, 1, 0), "span 外不覆盖");
        assert!(!is_covered(&merges, 0, 1), "span 外行");
    }

    #[test]
    fn empty_top_left_still_maps() {
        // 空合并格（左上无值——几何映射不依赖文本，选择/编辑照常锚左上）。
        let merges = vec![MergeSpan {
            row_start: 0,
            row_end: 0,
            col_start: 0,
            col_end: 2,
            abs_anchor: (0, 0),
        }];
        assert_eq!(canonical_cell(&merges, 0, 2), Some((0, 0)));
    }

    // ── 渲染几何 ───────────────────────────────────────────────

    #[test]
    fn spans_in_row_inclusive_bounds() {
        let merges = vec![
            MergeSpan {
                row_start: 1,
                row_end: 3,
                col_start: 0,
                col_end: 1,
                abs_anchor: (1, 0),
            },
            MergeSpan {
                row_start: 5,
                row_end: 5,
                col_start: 2,
                col_end: 3,
                abs_anchor: (5, 2),
            },
        ];
        // 行 0：无。
        assert!(spans_in_row(&merges, 0).is_empty());
        // 行 1（首行）：span 0。
        assert_eq!(spans_in_row(&merges, 1).len(), 1);
        // 行 2（中部）：span 0。
        assert_eq!(spans_in_row(&merges, 2).len(), 1);
        // 行 3（末行）：span 0（含端点）。
        assert_eq!(spans_in_row(&merges, 3).len(), 1);
        // 行 4：无。
        assert!(spans_in_row(&merges, 4).is_empty());
        // 行 5：span 1（单行 span 含自身）。
        assert_eq!(spans_in_row(&merges, 5).len(), 1);
        // 行 6：无。
        assert!(spans_in_row(&merges, 6).is_empty());
    }

    #[test]
    fn span_px_math_with_internal_gaps() {
        // 列宽 [100, 80, 60] + COL_GAP=1 → offsets [0, 101, 182]。
        let widths = [100.0, 80.0, 60.0];
        let offs = [0.0, 101.0, 182.0];
        // 单列 span：x=101, w=80。
        assert_eq!(span_px(&offs, &widths, 1, 1), Some((101.0, 80.0)));
        // 跨列 span 1..2：x=101, w=182+60-101=141（内部 gap 计入）。
        assert_eq!(span_px(&offs, &widths, 1, 2), Some((101.0, 141.0)));
        // 全表 span 0..2：x=0, w=242。
        assert_eq!(span_px(&offs, &widths, 0, 2), Some((0.0, 242.0)));
        // 越界防御：col_end 越界 / 乱序。
        assert_eq!(span_px(&offs, &widths, 0, 3), None, "col_end 越界");
        assert_eq!(span_px(&offs, &widths, 2, 1), None, "乱序");
        // 空几何。
        assert_eq!(span_px(&[], &[], 0, 0), None);
    }

    #[test]
    fn intersects_window_crossing_cases() {
        // 横跨视窗：span 列 2..5，窗口 [3,5) → 相交（左端在窗外）。
        assert!(intersects_window(2, 5, 3, 5), "左端滚出窗外仍可见");
        // span 完全包住窗口。
        assert!(intersects_window(0, 9, 3, 5), "span 横跨整个视窗");
        // 右端在窗外（右端未入窗）。
        assert!(intersects_window(4, 8, 3, 5), "右端未入窗仍可见");
        // 恰好接壤：span 5..9 与窗口 [3,5) → col_start=5 不小于 end=5 → 不相交。
        assert!(!intersects_window(5, 9, 3, 5), "span 起点在窗口右缘外");
        // span 0..2 与窗口 [3,5)：col_end=2 < first=3 → 不相交。
        assert!(!intersects_window(0, 2, 3, 5), "span 整段在窗口左侧");
        // 空窗口（零宽 [3,3)）——#584 返工：旧实现 span(2,5) 例
        // `2 < 3 && 5 >= 3` 误判 true（覆盖格起点在窗口内例）。
        assert!(!intersects_window(0, 2, 3, 3), "空窗口零相交（左侧例）");
        assert!(
            !intersects_window(2, 5, 3, 3),
            "空窗口零相交（横跨例——返工红例）"
        );
        assert!(!intersects_window(3, 5, 3, 3), "空窗口零相交（起点重合例）");
        assert!(!intersects_window(9, 9, 3, 3), "退化 span + 空窗口");
    }

    #[test]
    fn span_edges_segmentation() {
        let span = MergeSpan {
            row_start: 2,
            row_end: 4,
            col_start: 0,
            col_end: 1,
            abs_anchor: (2, 0),
        };
        // 首行：top 有 bottom 无。
        assert_eq!(
            span_edges(&span, 2),
            Some(SpanEdges {
                top: true,
                bottom: false
            })
        );
        // 中部行：双无（只画左右边）。
        assert_eq!(
            span_edges(&span, 3),
            Some(SpanEdges {
                top: false,
                bottom: false
            })
        );
        // 末行：bottom 有 top 无。
        assert_eq!(
            span_edges(&span, 4),
            Some(SpanEdges {
                top: false,
                bottom: true
            })
        );
        // 单行 span：top+bottom 同真（完整小框）。
        let single = MergeSpan {
            row_start: 1,
            row_end: 1,
            col_start: 0,
            col_end: 0,
            abs_anchor: (1, 0),
        };
        assert_eq!(
            span_edges(&single, 1),
            Some(SpanEdges {
                top: true,
                bottom: true
            })
        );
        // span 外的行。
        assert_eq!(span_edges(&span, 1), None);
        assert_eq!(span_edges(&span, 5), None);
    }
}
