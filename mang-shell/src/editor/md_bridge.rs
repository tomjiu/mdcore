//! md_bridge —— Buffer ↔ mdcore::Core 换芯适配层（#163 P0）。
//!
//! ## 架构（对齐 web 先例 R7/R9：CM6 持文本投影、核持语义权威）
//!
//! ```text
//! Buffer（公开 API 面，41 项测试契约不动）
//!  ├─ core: mdcore::Core    ← 唯一文档真源（树/Op/History/StableID/选区/字数）
//!  ├─ proj: ropey::Rope     ← 显示投影（含空行/\r——显示缓存，非文档真源）
//!  └─ lines/blocks          ← 行↔块映射（快路径 op 翻译 + undo 投影翻译）
//! ```
//!
//! **不变量（本层一切正确性的根基）**：
//!
//! > `core.tree ≡ markdown::parse_with_lines(normalize(proj)).0`
//!
//! 即核树恒等于投影文本的解析。四条维护路径：
//! 1. 快路径 A（单块原位 `ReplaceText`）——守卫证明「编辑不改变行分类」
//!    后原位翻译，两侧同步演化；
//! 2. 快路径 B（空行首字符 → 顶层 `InsertBlock`）——v0 解析器恒把纯
//!    文本行挂根层，顶层插入与重解析同构；
//! 3. 兜底（`sync_from_markdown`）——web fullResync 同款：整篇差分进核；
//! 4. undo/redo——Buffer 侧伴记四元组：单编辑条目**直接投影回放**（与
//!    旧 r39 语义逐位等价，空行/\r 按原文复原）；多编辑条目走逆 op
//!    翻译（类 a：ReplaceText；类 a2：+ 顶层段落叶 RemoveBlock 译为
//!    删行/清行）或全量重建（类 b：核序列化 + 空行保留通道）。
//!
//! ## 行映射几何
//!
//! [`mdcore::markdown::parse_with_lines`] 逐行产出 slot（block 先序号 +
//! prefix/tail/text_off/text_len）。桥侧叠加 `\r` 尾差（解析器先行规范化，
//! **行数相等 ⇔ 无行内 `\r`**——失配即 `aligned = false`，快路径整体
//! 降级、恒走兜底，正确性不受影响）。
//!
//! ## 快路径守卫（soundness）
//!
//! 编辑限定在 `[prefix, line_len - tail)` 的块文本映射段内 **且** 结果行
//! 重分类与原 slot 分类一致（[`mdcore::markdown::classify_line`] 单源）
//! ——两条件合取保证「该行的块语义不变」，原位 op 与整篇重解析产生
//! 同一棵树。

use std::collections::HashMap;

use mdcore::markdown::{SlotClass, classify_line, parse_with_lines};
use mdcore::op::Op;
use mdcore::tree::{BlockKind, NodeId};
use mdcore::{ApplyMeta, Core};

use super::buffer::Point;

// ---------------------------------------------------------------------------
// 数据结构
// ---------------------------------------------------------------------------

/// 投影行 slot（解析 slot + `\r` 尾差叠加）。
#[derive(Clone, Copy, Debug)]
struct ProjSlot {
    /// 块先序号（`None` = 空行/丢弃行）。
    block: Option<u32>,
    /// 行内容前缀宽（char）。
    prefix: u32,
    /// 行内容尾部宽（char = 解析 tail + 行尾 `\r` 计数）。
    tail: u32,
    /// 行内容在块文本内的 char 起点。
    text_off: u32,
    /// 行内容的块文本 char 长。
    text_len: u32,
    /// 解析角色（快路径守卫面）。
    class: SlotClass,
}

/// 块表条目（下标 = 核树先序号）。
#[derive(Clone, Debug)]
struct BlockSlot {
    id: mdcore::id::BlockId,
    node: NodeId,
    kind: BlockKind,
    /// 树路径（op 寻址面；顶层插入后随块顶层段 +1 平移）。
    path: Vec<usize>,
    /// 投影行区间 [first, last]（含；容器 = 子块行并集）。
    first_line: u32,
    last_line: u32,
}

/// 一次 undo/redo 的结果摘要（Buffer 侧消费：光标落点/事件记录/hook 通报）。
pub(crate) struct HistoryOutcome {
    /// 建议光标（回放/类 a = 编辑起点；类 b = 传入恢复位）。
    pub cursor: Option<Point>,
    /// #205 fence 记账面：Some((line, inserted_lines, pre_lines)) =
    /// 单区间几何（pre_lines = 变更前行数）；None = 多区间/不确定 → Full
    ///（Buffer 侧伴记推导为主，此面为多编辑条目的保守判据）。
    pub fence_single: Option<(usize, usize, usize)>,
    /// 该步的 op 数（last_undo_batch_count 口径）。
    pub ops: usize,
    /// 投影编辑几何（恰一个单区间时给精确记录；None = 粗粒度）。
    pub precise: Option<(Point, Point, String)>,
    /// 行数变化量（UndoHook 通报）。
    pub delta_lines: i64,
}

/// 批量快路径的守卫产物（pre 状态几何 + 文本）。
#[derive(Clone)]
struct GuardedEdit {
    block: usize,
    /// pre 状态行号（映射回放的行定位——post 坐标会因批内前序编辑
    /// 漂移，不可反查）。
    line: usize,
    /// pre 状态的块内偏移域。
    bstart: usize,
    bend: usize,
    /// pre 状态的投影 char 域。
    cstart: usize,
    cend: usize,
    text: String,
}

/// 顶层块间空行快照（类 b 重建的保留通道）。
struct GapSnapshot {
    /// (块 ID, 其后空行数)——按文档序。
    after: Vec<(mdcore::id::BlockId, u32)>,
    leading: u32,
    trailing: u32,
}

// ---------------------------------------------------------------------------
// MdBridge
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct MdBridge {
    core: Core,
    proj: ropey::Rope,
    lines: Vec<ProjSlot>,
    blocks: Vec<BlockSlot>,
    /// 块 ID → 先序下标。
    by_id: HashMap<mdcore::id::BlockId, u32>,
    /// 行内 `\r`（行数失配）→ 快路径整体关闭，恒走兜底。
    aligned: bool,
}

impl MdBridge {
    // ------------------------------------------------------------------
    // 构造与检视
    // ------------------------------------------------------------------

    pub(crate) fn new(text: &str) -> Self {
        let core = Core::from_markdown(text);
        let proj = ropey::Rope::from_str(text);
        let mut bridge = Self {
            core,
            proj,
            lines: Vec::new(),
            blocks: Vec::new(),
            by_id: HashMap::new(),
            aligned: false,
        };
        bridge.rebuild_map();
        bridge
    }

    /// 显示投影全文（`\r`/空行原样——显示面，非核序列化）。
    pub(crate) fn to_plain_text(&self) -> String {
        self.proj.to_string()
    }

    /// 行数（mang 语义 = `split('\n')` 计数；LF-only ropey len_lines 同构）。
    pub(crate) fn line_count(&self) -> usize {
        self.proj.len_lines()
    }

    /// 指定行内容（不含换行；`\r` 属行内容原样；越界空串）。
    pub(crate) fn line(&self, line: usize) -> String {
        if line >= self.line_count() {
            return String::new();
        }
        self.proj
            .line(line)
            .to_string()
            .trim_end_matches('\n')
            .to_string()
    }

    pub(crate) fn line_len(&self, line: usize) -> usize {
        self.line(line).chars().count()
    }

    pub(crate) fn char_count(&self) -> usize {
        self.proj.len_chars()
    }

    #[cfg(test)]
    pub(crate) fn set_test_clock(&mut self, clock: std::sync::Arc<dyn Fn() -> u64 + Send + Sync>) {
        self.core.set_clock(clock);
    }

    pub(crate) fn core(&self) -> &Core {
        &self.core
    }

    pub(crate) fn word_count(&self) -> mdcore::word_count::WordCount {
        self.core.word_count()
    }

    // ------------------------------------------------------------------
    // 坐标换算（Point ↔ 投影 char offset）
    // ------------------------------------------------------------------

    /// 行文本的 char 列切片 `[col_s, col_e)`。
    fn line_slice(s: &str, col_s: usize, col_e: usize) -> String {
        if col_s >= col_e {
            return String::new();
        }
        let b0 = s
            .char_indices()
            .nth(col_s)
            .map(|(b, _)| b)
            .unwrap_or(s.len());
        let b1 = s
            .char_indices()
            .nth(col_e)
            .map(|(b, _)| b)
            .unwrap_or(s.len());
        s[b0..b1].to_string()
    }

    /// Point → 投影 char offset（越界钳制）。
    pub(crate) fn point_to_coff(&self, p: Point) -> usize {
        let rows = self.line_count();
        let line = p.line.min(rows.saturating_sub(1));
        let col = p.col.min(self.line_len(line));
        self.proj.line_to_char(line) + col
    }

    /// 投影 char offset → Point。
    pub(crate) fn coff_to_point_p(&self, c: usize) -> Point {
        let c = c.min(self.proj.len_chars());
        let line = self.proj.char_to_line(c);
        Point::new(line, c - self.proj.line_to_char(line))
    }

    // ------------------------------------------------------------------
    // 行映射重建（不变量维护的兜底路径）
    // ------------------------------------------------------------------

    fn rebuild_map(&mut self) {
        let text = self.proj.to_string();
        let proj_lines: Vec<&str> = text.split('\n').collect();
        let (_, slots) = parse_with_lines(&text);
        self.aligned = slots.len() == proj_lines.len();
        self.lines = Vec::with_capacity(proj_lines.len());
        for (i, s) in slots.iter().enumerate() {
            let cr_tail = if i < proj_lines.len() {
                proj_lines[i]
                    .chars()
                    .rev()
                    .take_while(|&c| c == '\r')
                    .count() as u32
            } else {
                0
            };
            self.lines.push(ProjSlot {
                block: s.block,
                prefix: s.prefix,
                tail: s.tail + cr_tail,
                text_off: s.text_off,
                text_len: s.text_len,
                class: s.class,
            });
        }
        // 块表：核树先序遍历（不变量保证 == 解析先序）。
        self.blocks.clear();
        self.by_id.clear();
        self.walk_blocks();
        // 行区间回填：叶子 slot 占位，容器 = 子树并集（先序逆序自底向上）。
        let mut spans: Vec<(u32, u32)> = vec![(u32::MAX, 0); self.blocks.len()];
        for (li, slot) in self.lines.iter().enumerate() {
            if let Some(b) = slot.block {
                let b = b as usize;
                if b < spans.len() {
                    spans[b].0 = spans[b].0.min(li as u32);
                    spans[b].1 = spans[b].1.max(li as u32);
                }
            }
        }
        for i in (0..self.blocks.len()).rev() {
            let parent = self.core.tree.node(self.blocks[i].node).parent;
            if let Some(pn) = parent {
                let pid = self.core.tree.node(pn).id;
                if let Some(&pi) = self.by_id.get(&pid) {
                    let (f, l) = spans[i];
                    if f != u32::MAX {
                        spans[pi as usize].0 = spans[pi as usize].0.min(f);
                        spans[pi as usize].1 = spans[pi as usize].1.max(l);
                    }
                }
            }
        }
        for (i, span) in spans.into_iter().enumerate() {
            if span.0 != u32::MAX {
                self.blocks[i].first_line = span.0;
                self.blocks[i].last_line = span.1;
            }
        }
    }

    fn walk_blocks(&mut self) {
        fn walk(bridge: &mut MdBridge, node: NodeId, path: Vec<usize>) {
            let (id, kind, children) = {
                let n = bridge.core.tree.node(node);
                (n.id, n.kind, n.children.clone())
            };
            let idx = bridge.blocks.len() as u32;
            bridge.by_id.insert(id, idx);
            bridge.blocks.push(BlockSlot {
                id,
                node,
                kind,
                path: path.clone(),
                first_line: u32::MAX,
                last_line: 0,
            });
            for (ci, &c) in children.iter().enumerate() {
                let mut p = path.clone();
                p.push(ci);
                walk(bridge, c, p);
            }
        }
        let top = self.core.tree.top_level().to_vec();
        for (i, &t) in top.iter().enumerate() {
            walk(self, t, vec![i]);
        }
    }

    /// 树路径 → 块表下标（结构不变期恒有效）。
    fn resolve_path(&self, path: &[usize]) -> Option<u32> {
        if path.is_empty() {
            return None;
        }
        let top: Vec<NodeId> = self.core.tree.top_level().to_vec();
        let mut node = *top.get(path[0])?;
        for &seg in &path[1..] {
            let children = self.core.tree.node(node).children.clone();
            node = *children.get(seg)?;
        }
        let id = self.core.tree.node(node).id;
        self.by_id.get(&id).copied()
    }

    // ------------------------------------------------------------------
    // 编辑入口（Buffer 单点调用）
    // ------------------------------------------------------------------

    /// 单区间替换：快路径（单块原位 / 空行插段）→ 兜底 sync。
    /// 返回 false 仅当兜底也拒（编辑不生效——核为真源）。
    pub(crate) fn edit(&mut self, start: Point, end: Point, text: &str) -> bool {
        if let Some(()) = self.try_fast_text_edit(start, end, text) {
            return true;
        }
        if let Some(()) = self.try_fast_blank_insert(start, end, text) {
            return true;
        }
        self.sync_edit(start, end, text)
    }

    /// 兜底：投影先行 + 整篇重同步（web fullResync 同款）。
    fn sync_edit(&mut self, start: Point, end: Point, text: &str) -> bool {
        let cs = self.point_to_coff(start);
        let ce = self.point_to_coff(end);
        let backup = self.proj.slice(cs..ce).to_string();
        self.apply_proj(cs, ce, text);
        let full = self.proj.to_string();
        match self.core.sync_from_markdown(&full) {
            Ok(_) => {
                self.rebuild_map();
                true
            }
            Err(e) => {
                // 兜底也拒（理论不可达——合法文本恒可差分）：回滚投影。
                eprintln!("md_bridge: sync_from_markdown rejected: {e:?}");
                self.apply_proj(cs, cs + text.chars().count(), &backup);
                self.rebuild_map();
                false
            }
        }
    }

    /// 投影原位编辑（char 域）。
    fn apply_proj(&mut self, cs: usize, ce: usize, text: &str) {
        if cs < ce {
            self.proj.remove(cs..ce);
        }
        if !text.is_empty() {
            self.proj.insert(cs, text);
        }
    }

    // ------------------------------------------------------------------
    // 快路径 A：单块原位 ReplaceText
    // ------------------------------------------------------------------

    /// 守卫 + 翻译。`Some(())` = 已应用（核 + 投影 + 映射同步演化）。
    fn try_fast_text_edit(&mut self, start: Point, end: Point, text: &str) -> Option<()> {
        if !self.aligned || text.contains('\n') || start.line != end.line || start > end {
            return None;
        }
        let line = start.line;
        let slot = *self.lines.get(line)?;
        let block_idx = slot.block? as usize;
        let line_text = self.line(line);
        let line_chars = line_text.chars().count();
        // 编辑必须落在内容段内（\r 尾 / 收界符不吃）。
        if (start.col as u32) < slot.prefix || (end.col as u32) > line_chars as u32 - slot.tail {
            return None;
        }
        // 结果行重分类守卫（单源原语）。
        let mut result = Self::line_slice(&line_text, 0, start.col);
        result.push_str(text);
        result.push_str(&Self::line_slice(&line_text, end.col, line_chars));
        if !class_guard(slot.class, &result) {
            return None;
        }
        let path = self.blocks.get(block_idx)?.path.clone();
        let bstart = slot.text_off as usize + start.col - slot.prefix as usize;
        let bend = slot.text_off as usize + end.col - slot.prefix as usize;
        let op = Op::ReplaceText {
            path,
            start: bstart,
            end: bend,
            insert: text.into(),
        };
        // 核先行（真源先变更；拒绝 = 映射漂移 bug → 兜底自愈）。
        if self
            .core
            .apply_ops(vec![op], ApplyMeta::view("fast_edit"))
            .is_err()
        {
            return None;
        }
        let cs = self.point_to_coff(start);
        let ce = self.point_to_coff(end);
        self.apply_proj(cs, ce, text);
        let delta = text.chars().count() as i64 - (end.col - start.col) as i64;
        // 映射演化：本行 text_len 增减 + 同块后续行 text_off 平移。
        if let Some(s) = self.lines.get_mut(line) {
            s.text_len = (s.text_len as i64 + delta) as u32;
        }
        self.shift_block_offsets(block_idx, line, delta);
        Some(())
    }

    /// 块内后续行 text_off 平移（同块多行体的行内编辑）。
    fn shift_block_offsets(&mut self, block: usize, from_line: usize, delta: i64) {
        if delta == 0 || self.blocks.is_empty() {
            return;
        }
        let last = self.blocks[block].last_line as usize;
        for li in (from_line + 1)..=last {
            if let Some(s) = self.lines.get_mut(li) {
                if s.block == Some(block as u32) && s.text_len > 0 {
                    s.text_off = (s.text_off as i64 + delta) as u32;
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // 快路径 B：空行首字符 → 顶层 InsertBlock
    // ------------------------------------------------------------------

    fn try_fast_blank_insert(&mut self, start: Point, end: Point, text: &str) -> Option<()> {
        if !self.aligned || text.contains('\n') || start != end || text.is_empty() {
            return None;
        }
        let line = start.line;
        let slot = *self.lines.get(line)?;
        if slot.class != SlotClass::Blank {
            return None;
        }
        let line_text = self.line(line);
        let line_chars = line_text.chars().count();
        if start.col > line_chars {
            return None;
        }
        // 结果行 = 纯文本段落（块文本 = 整行内容，含原空白残留）。
        let mut result = Self::line_slice(&line_text, 0, start.col);
        result.push_str(text);
        result.push_str(&Self::line_slice(&line_text, start.col, line_chars));
        if classify_line(&result) != mdcore::markdown::LineClass::Paragraph {
            return None;
        }
        // 顶层插入位 = last_line < 本行的顶层块数（空行不属任何块）。
        let k = self
            .blocks
            .iter()
            .filter(|b| b.path.len() == 1 && (b.last_line as usize) < line)
            .count();
        let op = Op::InsertBlock {
            parent: Vec::new(),
            index: k,
            kind: BlockKind::Paragraph,
            attrs: Default::default(),
            text: result.as_str().into(),
            id: None,
        };
        if self
            .core
            .apply_ops(vec![op], ApplyMeta::view("fast_blank_insert"))
            .is_err()
        {
            return None;
        }
        // 投影编辑（行数不变——空行变内容行）。
        let cs = self.point_to_coff(start);
        self.apply_proj(cs, cs, text);
        // 映射演化：新块落先序位 p；其后块顶层段 ≥k 平移（含嵌套子块）。
        let p = self.preorder_insert_position(k);
        let (new_id, new_node) = {
            let top = self.core.tree.top_level();
            let node = *top.get(k).expect("insert_block 落位 k");
            (self.core.tree.node(node).id, node)
        };
        let slot_new = ProjSlot {
            block: Some(p as u32),
            prefix: 0,
            tail: 0,
            text_off: 0,
            text_len: result.chars().count() as u32,
            class: SlotClass::Paragraph,
        };
        self.blocks.insert(
            p,
            BlockSlot {
                id: new_id,
                node: new_node,
                kind: BlockKind::Paragraph,
                path: vec![k],
                first_line: line as u32,
                last_line: line as u32,
            },
        );
        for s in self.lines.iter_mut() {
            if let Some(b) = s.block.as_mut() {
                if *b as usize >= p {
                    *b += 1;
                }
            }
        }
        for (i, b) in self.blocks.iter_mut().enumerate() {
            if i > p && !b.path.is_empty() && b.path[0] >= k {
                b.path[0] += 1;
            }
        }
        self.by_id.clear();
        for (i, b) in self.blocks.iter().enumerate() {
            self.by_id.insert(b.id, i as u32);
        }
        if let Some(s) = self.lines.get_mut(line) {
            *s = slot_new;
        }
        Some(())
    }

    /// 顶层第 k 块前的先序插入位（= 前 k 个顶层子树的块总数）。
    fn preorder_insert_position(&self, k: usize) -> usize {
        let mut seen_top = 0usize;
        for (i, b) in self.blocks.iter().enumerate() {
            if b.path.len() == 1 {
                if seen_top == k {
                    return i;
                }
                seen_top += 1;
            }
        }
        self.blocks.len()
    }

    // ------------------------------------------------------------------
    // undo / redo（核驱动 → 投影翻译）
    // ------------------------------------------------------------------

    /// Replay source-only edits without consuming an unrelated core history entry.
    pub(crate) fn replay_projection(
        &mut self,
        restore: Point,
        replay: Option<(Point, Point, String)>,
    ) -> Option<HistoryOutcome> {
        let (start, end, text) = replay?;
        let pre_lines = self.line_count() as i64;
        self.apply_proj(self.point_to_coff(start), self.point_to_coff(end), &text);
        self.rebuild_map();
        self.core.break_coalesce();
        Some(HistoryOutcome {
            cursor: Some(restore),
            fence_single: None,
            ops: 1,
            precise: Some((start, end, text)),
            delta_lines: self.line_count() as i64 - pre_lines,
        })
    }

    pub(crate) fn undo_replay(
        &mut self,
        restore: Point,
        gesture_end: Point,
        replay: Option<(Point, Point, String)>,
    ) -> Option<HistoryOutcome> {
        self.history_step(false, restore, gesture_end, replay)
    }

    pub(crate) fn redo_replay(
        &mut self,
        restore: Point,
        gesture_end: Point,
        replay: Option<(Point, Point, String)>,
    ) -> Option<HistoryOutcome> {
        self.history_step(true, restore, gesture_end, replay)
    }

    fn history_step(
        &mut self,
        redo: bool,
        restore: Point,
        gesture_end: Point,
        replay: Option<(Point, Point, String)>,
    ) -> Option<HistoryOutcome> {
        let pre_lines = self.line_count() as i64;
        // 空行保留通道快照（类 b 重建时恢复块间空行）。
        let gaps = self.snapshot_gaps();
        if redo {
            self.core.redo()?;
        } else {
            self.core.undo()?;
        }
        // 取 undo/redo 事件的操作序列（逆 op = 待翻译面）。
        let ops: Vec<Op> = self
            .core
            .take_events()
            .into_iter()
            .filter(|e| {
                matches!(
                    e.meta.source,
                    mdcore::history::Source::Undo | mdcore::history::Source::Redo
                )
            })
            .flat_map(|e| e.ops)
            .collect();
        let op_count = ops.len();

        // 四元组回放（单编辑条目）：投影 = 占位换回原文——与旧 r39
        // 语义逐位等价（空行/\r/行几何按原文复原，不经序列化重建）。
        // 树侧由 core 逆 op 复原（语义真源）；回放后映射重建。
        if let Some((rs, re_, rt)) = replay {
            let cs = self.point_to_coff(rs);
            let ce = self.point_to_coff(re_);
            self.apply_proj(cs, ce, &rt);
            self.rebuild_map();
            let precise = Some((rs, re_, rt));
            return Some(HistoryOutcome {
                cursor: Some(restore),
                fence_single: None,
                ops: op_count.max(1),
                precise,
                delta_lines: self.line_count() as i64 - pre_lines,
            });
        }

        // 类 a/a2：ReplaceText（+ RemoveBlock·顶层段落叶——空行插段与
        // sync 拆行的逆）→ 逐序翻译；RemoveBlock 译为「清行/删行」——
        // 撤销插段 = 段落行退回空行（与旧四元组「占位换回空串」语义
        // 逐位一致，空行不丢）。
        let translated = if !ops.is_empty()
            && ops
                .iter()
                .all(|o| matches!(o, Op::ReplaceText { .. }) || matches!(o, Op::RemoveBlock { .. }))
            && self.a2_sound(&ops, restore.line, gesture_end.line)
        {
            let mut edits: Vec<(Point, Point, String)> = Vec::new();
            let mut ok = true;
            for op in &ops {
                let edit = match op {
                    Op::ReplaceText { .. } => self.translate_replace(op),
                    Op::RemoveBlock { path } => {
                        self.translate_remove_block(path, restore.line, gesture_end.line)
                    }
                    _ => None,
                };
                match edit {
                    Some(edit) => edits.push(edit),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                Some(edits)
            } else {
                // 部分翻译已应用——类 b 重建以核为准，自愈。
                None
            }
        } else {
            None
        };

        if let Some(edits) = translated {
            let precise = if edits.len() == 1 {
                let (s, e, t) = &edits[0];
                Some((*s, *e, t.clone()))
            } else {
                None
            };
            let cursor = edits.first().map(|(s, _, _)| *s);
            // delta 在翻译后取（翻译前的投影尚未反映逆 op——行数差
            // 恒 0 的假象；pre_lines 由最终行数反推同理）。
            let post_lines = self.line_count() as i64;
            let delta = post_lines - pre_lines;
            let fence_single = precise
                .as_ref()
                .map(|(s, _, t)| (s.line, t.matches('\n').count(), post_lines as usize));
            // 映射重校（undo 手势低频，O(doc) 自愈可接受）。
            self.rebuild_map();
            Some(HistoryOutcome {
                cursor,
                fence_single,
                ops: op_count,
                precise,
                delta_lines: delta,
            })
        } else {
            // 类 b：光标 = 手势起点（Buffer 侧伴记——与旧「光标回编辑
            // 起点」语义对齐；ops 推导不可靠：sync 差分 op 的起点是块内
            // 文本域，不是用户编辑位）。
            self.rebuild_from_serialization(&gaps);
            Some(HistoryOutcome {
                cursor: Some(restore),
                fence_single: None,
                ops: op_count,
                precise: None,
                delta_lines: self.line_count() as i64 - pre_lines,
            })
        }
    }

    /// a2 健全性：RemoveBlock 目标须为顶层段落叶（pre-undo 映射解析）；
    /// ReplaceText 的顶层位须在被删块之前（删除平移不牵动其路径解析）。
    /// 手势行域约束：RB 目标行须落在手势被替换域行区间内（sync 差分
    /// 是按位置的——域外 RB = 位置差分假象 → 类 b）。任一不满足 → 类 b。
    fn a2_sound(&self, ops: &[Op], gs_line: usize, ge_line: usize) -> bool {
        let mut removed_top: Vec<usize> = Vec::new();
        for op in ops {
            match op {
                Op::RemoveBlock { path } => {
                    if path.len() != 1 {
                        return false;
                    }
                    let Some(bi) = self.resolve_path_pre_undo(path) else {
                        return false;
                    };
                    let b = &self.blocks[bi as usize];
                    if b.kind != BlockKind::Paragraph || b.path.len() != 1 {
                        return false;
                    }
                    let l = b.first_line as usize;
                    if l < gs_line || l > ge_line {
                        return false;
                    }
                    removed_top.push(b.path[0]);
                }
                Op::ReplaceText { path, .. } => {
                    if path.is_empty() {
                        return false;
                    }
                    if let Some(&min_removed) = removed_top.iter().min() {
                        if path[0] >= min_removed {
                            return false;
                        }
                    }
                }
                _ => return false,
            }
        }
        true
    }

    /// pre-undo 映射的路径解析（core 树已应用逆 op——不可用 tree 解析；
    /// 映射 blocks 表仍是撤销前快照）。
    fn resolve_path_pre_undo(&self, path: &[usize]) -> Option<u32> {
        if path.is_empty() {
            return None;
        }
        let mut idx: Option<u32> = None;
        let mut seen_top = 0usize;
        for (i, b) in self.blocks.iter().enumerate() {
            if b.path.len() == 1 {
                if seen_top == path[0] {
                    idx = Some(i as u32);
                    break;
                }
                seen_top += 1;
            }
        }
        let mut cur = idx?;
        for &seg in &path[1..] {
            let parent = cur as usize;
            let plen = self.blocks[parent].path.len();
            let mut child_seen = 0usize;
            let mut next: Option<u32> = None;
            for (i, b) in self.blocks.iter().enumerate().skip(parent + 1) {
                if b.path.len() <= plen {
                    break;
                }
                if b.path.len() == plen + 1 && b.path[..plen] == self.blocks[parent].path[..plen] {
                    if child_seen == seg {
                        next = Some(i as u32);
                        break;
                    }
                    child_seen += 1;
                }
            }
            cur = next?;
        }
        Some(cur)
    }

    /// RemoveBlock（顶层段落叶）→ 删行 / 清行。
    ///
    /// 判据（手势被替换域的行域）：`L > 手势起点行` → 整行删（拆行/
    /// 跨行产生的行——undo 即并回；末行删除连同其前导换行）；否则清行
    /// 内容（撤销插段 = 段落行退回空行——与旧四元组「占位换回空串」
    /// 语义一致，空行不丢）。
    fn translate_remove_block(
        &mut self,
        path: &[usize],
        gs_line: usize,
        _ge_line: usize,
    ) -> Option<(Point, Point, String)> {
        let bi = self.resolve_path_pre_undo(path)? as usize;
        let line = self.blocks[bi].first_line as usize;
        let line_text = self.line(line);
        let slot = self.lines.get(line).copied()?;
        let cs_line_start = self.proj.line_to_char(line);
        if gs_line < line && line + 1 <= self.line_count() {
            // 末行删除需连同其前导换行（非末行的删除域已含尾换行）。
            let cs_del = if line + 1 < self.line_count() {
                cs_line_start
            } else {
                cs_line_start.saturating_sub(1)
            };
            let ce = if line + 1 < self.line_count() {
                self.proj.line_to_char(line + 1)
            } else {
                self.proj.len_chars()
            };
            let removed_text = self.proj.slice(cs_del..ce).to_string();
            self.apply_proj(cs_del, ce, "");
            let start = Point::new(line, 0);
            let end = Point::new(line, line_text.chars().count());
            return Some((start, end, removed_text));
        }
        // 清行内容（保行——空行回退语义）。
        let start = Point::new(line, slot.prefix as usize);
        let end = Point::new(line, line_text.chars().count() - slot.tail as usize);
        let cs = self.point_to_coff(start);
        let ce = self.point_to_coff(end);
        let removed_text = self.proj.slice(cs..ce).to_string();
        self.apply_proj(cs, ce, "");
        Some((start, end, removed_text))
    }

    /// 单个 ReplaceText op → 投影编辑（对**当前**映射翻译并即刻应用）。
    fn translate_replace(&mut self, op: &Op) -> Option<(Point, Point, String)> {
        let Op::ReplaceText {
            path,
            start,
            end,
            insert,
        } = op
        else {
            return None;
        };
        let block = self.resolve_path(path)? as usize;
        let sp = self.block_offset_to_point(block, *start)?;
        let ep = self.block_offset_to_point(block, *end)?;
        let (sp, ep) = if sp <= ep { (sp, ep) } else { (ep, sp) };
        let cs = self.point_to_coff(sp);
        let ce = self.point_to_coff(ep);
        let text = insert.to_string();
        self.apply_proj(cs, ce, &text);
        let delta = text.chars().count() as i64 - (ce - cs) as i64;
        if let Some(s) = self.lines.get_mut(sp.line) {
            if s.block == Some(block as u32) && s.text_len > 0 {
                s.text_len = (s.text_len as i64 + delta) as u32;
            }
        }
        self.shift_block_offsets(block, sp.line, delta);
        Some((sp, ep, text))
    }

    /// 块内 char 偏移 → (行, 列)。体行 span 顺延覆盖（换行位 = 次行内容首）。
    fn block_offset_to_point(&self, block: usize, off: usize) -> Option<Point> {
        let (first, last) = {
            let b = self.blocks.get(block)?;
            (b.first_line as usize, b.last_line as usize)
        };
        for li in first..=last {
            let Some(s) = self.lines.get(li) else {
                continue;
            };
            if s.block == Some(block as u32) && s.text_len > 0 {
                let span_end = s.text_off as usize + s.text_len as usize;
                if off <= span_end {
                    return Some(Point::new(
                        li,
                        s.prefix as usize + off - s.text_off as usize,
                    ));
                }
            }
        }
        // 越界（理论不可达）：钳到块末内容尾。
        for li in (first..=last).rev() {
            let Some(s) = self.lines.get(li) else {
                continue;
            };
            if s.block == Some(block as u32) && s.text_len > 0 {
                return Some(Point::new(li, s.prefix as usize + s.text_len as usize));
            }
        }
        None
    }

    /// 顶层块间空行快照（含首尾空行数）。
    fn snapshot_gaps(&self) -> GapSnapshot {
        let mut top: Vec<&BlockSlot> = self.blocks.iter().filter(|b| b.path.len() == 1).collect();
        top.sort_by_key(|b| b.first_line);
        let mut after: Vec<(mdcore::id::BlockId, u32)> = Vec::with_capacity(top.len());
        for (i, b) in top.iter().enumerate() {
            let next_first = top
                .get(i + 1)
                .map(|n| n.first_line as usize)
                .unwrap_or(self.line_count());
            let mut blanks = 0u32;
            let mut l = b.last_line as usize + 1;
            while l < next_first && self.line(l).trim().is_empty() {
                blanks += 1;
                l += 1;
            }
            after.push((b.id, blanks));
        }
        let first_block_line = top.first().map(|b| b.first_line as usize);
        let leading = first_block_line
            .map(|f| (0..f).filter(|l| self.line(*l).trim().is_empty()).count() as u32)
            .unwrap_or(0);
        let trailing = top
            .last()
            .map(|b| {
                ((b.last_line as usize + 1)..self.line_count())
                    .filter(|l| self.line(*l).trim().is_empty())
                    .count() as u32
            })
            .unwrap_or(0);
        GapSnapshot {
            after,
            leading,
            trailing,
        }
    }

    /// 类 b 重建：投影 = 核序列化 + 空行保留通道。
    ///
    /// 归并规则：幸存邻对 (a, b) 的间隔 = pre-undo 文档序里 [a, b) 区间
    /// 全部块的记录间隔之和——**被删块的间隔随其位置归并入幸存邻对**
    ///（拆行 undo：被删「段」块后的空行回到「第一段↔第二段」之间）。
    /// 新块（pre-undo 不存在）默认无间隔。
    fn rebuild_from_serialization(&mut self, gaps: &GapSnapshot) {
        let tree = &self.core.tree;
        let mut out = String::new();
        for _ in 0..gaps.leading {
            out.push('\n');
        }
        let top: Vec<NodeId> = tree.top_level().to_vec();
        // 幸存块在 pre-undo 间隔表中的下标（不存在 = None → 新块）。
        let idx_of = |id: mdcore::id::BlockId| -> Option<usize> {
            gaps.after.iter().position(|(gid, _)| *gid == id)
        };
        for (i, &node) in top.iter().enumerate() {
            if i > 0 {
                out.push('\n');
                let prev_id = tree.node(top[i - 1]).id;
                let cur_id = tree.node(node).id;
                if let (Some(a), Some(b)) = (idx_of(prev_id), idx_of(cur_id)) {
                    if a < b {
                        let blanks: u32 = gaps.after[a..b].iter().map(|(_, n)| *n).sum();
                        for _ in 0..blanks {
                            out.push('\n');
                        }
                    }
                }
            }
            out.push_str(&mdcore::markdown::render_subtree_markdown(tree, node));
        }
        // 尾部：末幸存块之后的全部记录间隔 + 尾空行。
        if let Some(&last) = top.last() {
            let last_id = tree.node(last).id;
            if let Some(a) = idx_of(last_id) {
                let blanks: u32 = gaps.after[a..].iter().map(|(_, n)| *n).sum();
                for _ in 0..blanks {
                    out.push('\n');
                }
            }
        }
        for _ in 0..gaps.trailing {
            out.push('\n');
        }
        self.proj = ropey::Rope::from_str(&out);
        self.rebuild_map();
    }

    // ------------------------------------------------------------------
    // 批量编辑（replace_batch：强制新条目 + 快路径组 / 兜底）
    // ------------------------------------------------------------------

    /// 批量替换（edits 已按文档序排列、互不重叠、坐标为 pre 状态）。
    /// 返回 false = 兜底也拒（整批不生效）。
    pub(crate) fn edit_batch(&mut self, edits: &[(Point, Point, String)]) -> bool {
        // 预检全部守卫（pre 状态——行分类不受其它行编辑影响）。
        let guarded: Vec<Option<GuardedEdit>> = edits
            .iter()
            .map(|(s, e, t)| self.guard_edit(s, e, t))
            .collect();
        if guarded.iter().all(|g| g.is_some()) {
            let mut batch: Vec<GuardedEdit> = guarded.into_iter().flatten().collect();
            // 降序应用：pre 状态域恒有效（后位编辑不牵动前位；core 单
            // 事务一次提交 = 一条 undo 条目，不依赖合并窗）。
            batch.sort_by(|a, b| b.cstart.cmp(&a.cstart));
            self.core.break_coalesce();
            let ops: Vec<Op> = batch
                .iter()
                .map(|g| Op::ReplaceText {
                    path: self.blocks[g.block].path.clone(),
                    start: g.bstart,
                    end: g.bend,
                    insert: g.text.as_str().into(),
                })
                .collect();
            if self
                .core
                .apply_ops(ops, ApplyMeta::view("fast_batch"))
                .is_ok()
            {
                for g in &batch {
                    self.apply_proj(g.cstart, g.cend, &g.text);
                }
                // 映射演化：行定位用伴记 pre 行号（post 坐标反查会因
                // 批内前序编辑漂移——text_len 本行增减 + 后续行平移）。
                for g in &batch {
                    let delta = g.text.chars().count() as i64 - (g.cend - g.cstart) as i64;
                    if let Some(s) = self.lines.get_mut(g.line) {
                        if s.block == Some(g.block as u32) && s.text_len > 0 {
                            s.text_len = (s.text_len as i64 + delta) as u32;
                        }
                    }
                    self.shift_block_offsets(g.block, g.line, delta);
                }
                self.core.break_coalesce(); // Explicit gesture also fences following typing.
                return true;
            }
            // apply_ops rejection is atomic; no projection edits have run.
            eprintln!("md_bridge: fast_batch rejected——降级 sync 自愈");
        }
        // 兜底：投影整批降序应用 + 一次 sync（强制新条目）。
        let mut coffs: Vec<(usize, usize, String)> = edits
            .iter()
            .map(|(s, e, t)| (self.point_to_coff(*s), self.point_to_coff(*e), t.clone()))
            .collect();
        coffs.sort_by_key(|(s, _, _)| *s);
        let source_before = self.proj.clone();
        self.core.break_coalesce();
        for (cs, ce, t) in coffs.into_iter().rev() {
            self.apply_proj(cs, ce, &t);
        }
        let full = self.proj.to_string();
        if self.core.sync_from_markdown(&full).is_ok() {
            self.rebuild_map();
            self.core.break_coalesce();
            return true;
        }
        self.proj = source_before;
        self.rebuild_map();
        false
    }

    /// 单编辑守卫（快路径 A 的纯判定面——pre 状态，不应用）。
    fn guard_edit(&self, start: &Point, end: &Point, text: &str) -> Option<GuardedEdit> {
        if !self.aligned || text.contains('\n') || start.line != end.line || start > end {
            return None;
        }
        let slot = *self.lines.get(start.line)?;
        let block = slot.block? as usize;
        let line_text = self.line(start.line);
        let line_chars = line_text.chars().count();
        if (start.col as u32) < slot.prefix || (end.col as u32) > line_chars as u32 - slot.tail {
            return None;
        }
        let mut result = Self::line_slice(&line_text, 0, start.col);
        result.push_str(text);
        result.push_str(&Self::line_slice(&line_text, end.col, line_chars));
        if !class_guard(slot.class, &result) {
            return None;
        }
        Some(GuardedEdit {
            block,
            line: start.line,
            bstart: slot.text_off as usize + start.col - slot.prefix as usize,
            bend: slot.text_off as usize + end.col - slot.prefix as usize,
            cstart: self.point_to_coff(*start),
            cend: self.point_to_coff(*end),
            text: text.to_string(),
        })
    }

    // ------------------------------------------------------------------
    // 选区锚点同步（anchor.rs 消费面）
    // ------------------------------------------------------------------

    /// 光标/选区 → mdcore Selection（块锚点落核——undo 选区快照与
    /// AI/外部面的数据源）。不可解析位（空行/标记行/越内容段）保持上次。
    pub(crate) fn sync_selection(&mut self, cursor: Point, anchor: Option<Point>) {
        let focus = self.point_to_anchor(cursor);
        let sel_anchor = anchor.and_then(|a| self.point_to_anchor(a));
        if let Some(f) = focus {
            let a = sel_anchor.unwrap_or(f);
            self.core.set_selection(mdcore::anchor::Selection {
                anchor: a,
                focus: f,
            });
        }
    }

    fn point_to_anchor(&self, p: Point) -> Option<mdcore::anchor::Anchor> {
        let rows = self.line_count();
        let line = p.line.min(rows.saturating_sub(1));
        let slot = self.lines.get(line)?;
        let block = slot.block? as usize;
        let b = self.blocks.get(block)?;
        let col = p.col.min(self.line_len(line));
        if (col as u32) < slot.prefix || (col as u32) > slot.prefix + slot.text_len {
            return None;
        }
        Some(mdcore::anchor::Anchor {
            block: b.id,
            offset: slot.text_off as usize + (col - slot.prefix as usize),
        })
    }

    // ------------------------------------------------------------------
    // 保存点 / 透传
    // ------------------------------------------------------------------

    pub(crate) fn mark_saved(&mut self) {
        self.core.mark_saved();
        self.core.clear_redo();
    }

    pub(crate) fn is_modified(&self, saved_depth: usize) -> bool {
        self.core.undo_depth() != saved_depth
    }

    pub(crate) fn undo_depth(&self) -> usize {
        self.core.undo_depth()
    }

    pub(crate) fn set_coalesce_ms(&mut self, ms: u64) {
        self.core.set_coalesce_ms(ms);
    }

    /// #163：命令边界破组（Buffer::break_undo_group 透传）。
    pub(crate) fn break_coalesce(&mut self) {
        self.core.break_coalesce();
    }

    /// #163：收割核事件队列，返回本窗口是否产生了**新的 View 条目**
    /// （合并入既有条目 = false——Buffer 侧 entry_ids 镜像只在新条目时
    /// 增补）。队列必须随每次编辑收割（防事件堆积）。
    pub(crate) fn take_view_entry_created(&mut self) -> bool {
        self.take_view_events().0
    }

    /// Harvest the core event queue once and report both signals Buffer needs:
    /// whether a *new* View entry appeared, and whether the core recorded **any**
    /// View event at all (a coalesced edit changes no revision and creates no new
    /// entry, yet it is still a real core edit — unlike a Markdown no-op).
    pub(crate) fn take_view_events(&mut self) -> (bool, bool) {
        let mut created = false;
        let mut any = false;
        for e in self.core.take_events() {
            if e.meta.source == mdcore::history::Source::View {
                any = true;
                if !e.meta.coalesced {
                    created = true;
                }
            }
        }
        (created, any)
    }

    /// #163：纯收割（批量路径——恒新条目由调用方自行记账）。
    pub(crate) fn drain_events(&mut self) {
        self.core.take_events();
    }
}

/// slot 分类 × 结果行分类的守卫表（单源原语 [`classify_line`]）。
fn class_guard(class: SlotClass, result_line: &str) -> bool {
    use mdcore::markdown::LineClass;
    match class {
        SlotClass::Paragraph => classify_line(result_line) == LineClass::Paragraph,
        SlotClass::Heading(l) => classify_line(result_line) == LineClass::Heading(l as usize),
        SlotClass::BodyCode => classify_line(result_line) != LineClass::Fence,
        SlotClass::BodyMath => result_line.trim() != "$$",
        SlotClass::TableRow => classify_line(result_line) == LineClass::TableRow,
        SlotClass::Quote(d) => classify_line(result_line) == LineClass::Quote(d as usize),
        SlotClass::ListItem(d) => classify_line(result_line) == LineClass::ListItem(d as usize),
        SlotClass::MathInline => classify_line(result_line) == LineClass::MathInline,
        SlotClass::Blank | SlotClass::MarkerOnly | SlotClass::Dropped => false,
    }
}

#[cfg(test)]
mod i163_bridge_tests {
    //! #163 换芯桥行为锁——不变量（核树 ≡ 投影解析）在快路径/兜底/
    //! undo 三路径上的可观察行为。

    use super::*;
    use crate::editor::buffer::Point;

    fn p(line: usize, col: usize) -> Point {
        Point::new(line, col)
    }

    #[test]
    fn fast_path_matches_full_resync_on_typing() {
        // 快路径逐字打字 vs 兜底整篇 sync——核树终态必须等价
        //（soundness 的可观察面：序列化一致）。
        let text =
            "# 标题\n\n段落一 abc\n\n```rust\nlet x = 1;\n```\n\n|a|b|\n|-|-|\n> 引用\n- 列表";
        let mut fast = MdBridge::new(text);
        let mut slow = MdBridge::new(text);
        for (i, ch) in ["中", "文", "输", "入"].iter().enumerate() {
            let at = p(2, 6 + i); // 段落一 abc| 追加
            assert!(fast.edit(at, at, ch), "fast edit {ch}");
            // 兜底对照：直接改投影 + sync（绕过快路径）。
            let cs = slow.point_to_coff(at);
            slow.apply_proj(cs, cs, ch);
            let full = slow.proj.to_string();
            slow.core.sync_from_markdown(&full).unwrap();
            slow.rebuild_map();
        }
        let f = mdcore::markdown::tree_to_markdown(&fast.core.tree);
        let sl = mdcore::markdown::tree_to_markdown(&slow.core.tree);
        assert_eq!(f, sl, "快路径与兜底终态等价");
        assert_eq!(fast.to_plain_text(), slow.to_plain_text());
    }

    #[test]
    fn crlf_projection_bit_fidelity_through_fast_path() {
        // CRLF 投影经快路径打字——\r 尾保真（tail 域守卫）。
        let mut b = MdBridge::new("a\r\nb\r\n");
        assert_eq!(b.line_count(), 3);
        // 行 0 末尾（\r 前）插入——块文本域内
        assert!(b.edit(p(0, 1), p(0, 1), "x"));
        assert_eq!(b.to_plain_text(), "ax\r\nb\r\n");
        // \r 后（tail 域）不可快路径——兜底 sync 仍整体保真
        //（CRLF 规范化只进核，投影原样）。
        let t = b.to_plain_text();
        assert!(t.contains("\r\n"));
    }

    #[test]
    fn blank_line_insert_creates_top_level_paragraph() {
        let mut b = MdBridge::new("aa\n\nbb");
        // 空行首字符 → InsertBlock 快路径
        assert!(b.edit(p(1, 0), p(1, 0), "mid"));
        assert_eq!(b.to_plain_text(), "aa\nmid\nbb");
        assert_eq!(b.core.tree.top_level().len(), 3, "三顶层块");
        // undo 还原
        assert!(b.undo_replay(p(1, 0), p(1, 0), None).is_some());
        assert_eq!(b.to_plain_text(), "aa\n\nbb");
    }

    #[test]
    fn structural_undo_preserves_blank_lines() {
        // 类 b 重建的空行保留通道：含空行文档经结构 undo 空行不丢
        //（web pushRender 的「view 独有态吸收」在本桥不发生）。
        let mut b = MdBridge::new("第一段\n\n第二段\n\n第三段");
        // 结构编辑：拆行（sync 兜底条目）
        assert!(b.edit(p(0, 2), p(0, 2), "\n"));
        assert_eq!(b.to_plain_text().lines().count(), 6);
        // 镜像 Buffer 真实流：单编辑条目走四元组回放（占位换回空串）。
        let out = b
            .undo_replay(p(0, 2), p(1, 0), Some((p(0, 2), p(1, 0), String::new())))
            .expect("undo");
        assert_eq!(out.delta_lines, -1);
        assert_eq!(
            b.to_plain_text(),
            "第一段\n\n第二段\n\n第三段",
            "空行全保留"
        );
    }

    #[test]
    fn coalesce_window_single_gesture() {
        // 合并窗：同窗多击键 = 一条目；undo 一步全撤（验收 2）。
        let mut b = MdBridge::new("abc");
        for i in 0..3 {
            assert!(b.edit(p(0, 3 + i), p(0, 3 + i), "x"));
        }
        assert_eq!(b.core.undo_depth(), 1, "三击键一条目");
        assert!(b.undo_replay(p(0, 3), p(0, 3), None).is_some());
        assert_eq!(b.to_plain_text(), "abc");
        assert_eq!(b.core.undo_depth(), 0);
    }

    #[test]
    fn fence_body_typing_stays_in_block() {
        // 围栏体行内编辑：块文本原位改（BodyCode 守卫——结果行非标记行）。
        let mut b = MdBridge::new("```rust\nlet x = 1;\n```");
        assert!(b.edit(p(1, 10), p(1, 10), " + 1"));
        assert_eq!(b.to_plain_text(), "```rust\nlet x = 1; + 1\n```");
        let node = b.core.tree.node(b.core.tree.top_level()[0]);
        assert_eq!(node.text.to_string(), "let x = 1; + 1");
        // 体行改成标记行 → 守卫拒绝 → 兜底 sync（闭栏结构变化）
        assert!(b.edit(p(1, 0), p(1, 0), "```"));
        let node = b.core.tree.node(b.core.tree.top_level()[0]);
        assert_eq!(node.text.to_string(), "", "``` 闭栏——体变空，后文成独立块");
    }

    #[test]
    fn undo_translation_restores_multiline_block() {
        let mut b = MdBridge::new("|a|b|\n|c|d|");
        // [(0,1),(1,1)) = "a|b|\n|" → "XY"（表两行并一行）
        assert!(b.edit(p(0, 1), p(1, 1), "XY"));
        assert_eq!(b.to_plain_text(), "|XYc|d|");
        assert!(b.undo_replay(p(0, 1), p(1, 1), None).is_some());
        assert_eq!(b.to_plain_text(), "|a|b|\n|c|d|");
    }

    #[test]
    fn selection_anchor_sync() {
        // 验收 5：光标/选区 → mdcore Selection（块锚点）。
        let mut b = MdBridge::new("para0\n# head\npara2");
        b.sync_selection(p(1, 4), Some(p(0, 2)));
        let sel = b.core.selection().expect("selection 落核");
        // 锚点（行 0 col 2）→ 块 0 offset 2
        assert_eq!(sel.anchor.block, mdcore::id::BlockId(1));
        assert_eq!(sel.anchor.offset, 2);
        // 焦点（标题行 col 4 = 前缀 2 后正文 col 2）→ 块 1 offset 2
        assert_eq!(sel.focus.block, mdcore::id::BlockId(2));
        assert_eq!(sel.focus.offset, 2);
    }
}
