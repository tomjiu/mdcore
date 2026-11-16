//! 编辑缓冲区 —— 纯逻辑文本模型（#68 阶段 1.2 文本编辑能力的地基；
//! #163 换芯：mdcore::Core 为唯一文档真源，Zed text::Buffer 退役）。
//!
//! ## #163 换芯说明（主从裁决：mdcore = 真源，GPUI = 视图）
//!
//! - **语义层（唯一真源）**：树/Op/History/StableID/选区/字数全部在
//!   `mdcore::Core`（经 [`super::md_bridge::MdBridge`] 持有）。undo/redo
//!   = 核历史（1s 合并窗，验收 2）；块身份/revision/事件流同源。
//! - **存储层（显示投影）**：ropey Rope 持原文（含空行/`\r`）——CM6
//!   之于 web 的同构角色：视图缓存，非文档真源。快路径（单块原位
//!   op）+ 兜底（sync_from_markdown 差分）双路维护「核树≡解析」不变量。
//! - **交互层外壳**：光标/选区/粘性列/查找保持既有语义——公开 API
//!   与 41 项测试契约不动（undo 合并窗语义按 #163 裁决变更为核同源）。
//! - **坐标系**：对外 `Point { line, col }` 的 `col` 仍按 **char 索引**
//!   （CJK 列语义不变）；投影 char offset 由桥层 ropey 直算。
//! - **行尾语义**：`\r\n` 原样存取（投影面逐位保真；行内孤立 `\r`
//!   使行映射失配 → 桥自动降级兜底路径，正确性不受影响）。
//! - **许可证面**：Zed `text`/`rope`（GPL，10,912 LOC）随本次换芯下线。
//!
//! ## 坐标语义（r39 原文保持）
//!
//! - `Point { line, col }`：`col` 一律按 **char 索引** 计（非字节）——
//!   CJK 语义安全：一个汉字占一列。
//! - 文档至少一行（空文档 = 单空行）；行不含换行符。
//! - `desired_col`：上下移动的「期望列」粘性——经过短行 clamp 后回到
//!   长行时恢复原列（经典编辑器语义）。
//!
//! ## 编辑原语
//!
//! 所有文本变更收敛到 [`Buffer::replace_range`] 单点（内部走
//! [`super::md_bridge::MdBridge::edit`]：快路径 op / 兜底 sync）：删除
//! `[start, end)` 再插入 `text`（可含 `\n` 拆行）。
//!
//! undo/redo = mdcore History（合并窗：同为文本类 View 事务 1s 内并入
//! 末条；docx 面（挂钩子时）合并窗置 0 保持逐编辑粒度——
//! EditHook/UndoHook 的 history_id 1:1 契约）。单编辑条目的投影回放
//! = 四元组占位换回（与 r39 语义逐位等价）。
//!
//! ## 选区
//!
//! `anchor`（Shift 移动落下的锚点）与 `cursor`（头）构成选区；
//! 排序后 `[start, end)`。无 Shift 的移动清锚。编辑命令先消费选区。
//! 光标/选区每变更经桥同步为 mdcore Selection（块锚点——
//! anchor.rs 消费面，#163 验收 5）。
use std::ops::Range;
use std::time::Instant;

use super::md_bridge::MdBridge;

/// 文本坐标：`line` 从 0 起，`col` 为行内 **char** 索引。
/// #111：`Serialize` derive（doc_events 事件载荷直出 `{"line":n,"col":n}`
/// ——与 RPC 响应的 point_json 同构，字段名直出无 rename）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct Point {
    pub line: usize,
    pub col: usize,
}

impl Point {
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

/// G3：缩进单位（可配置——settings `indent_unit` 键："2" / "4" / "tab"）。
/// 缺省 `Spaces4`（与既有 Tab 键插 4 空格语义逐位一致——行为零漂移）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum IndentUnit {
    #[default]
    Spaces4,
    Spaces2,
    Tab,
}

impl IndentUnit {
    /// 单位文本（插入物）。
    pub fn as_text(self) -> &'static str {
        match self {
            IndentUnit::Spaces4 => "    ",
            IndentUnit::Spaces2 => "  ",
            IndentUnit::Tab => "\t",
        }
    }

    /// settings 键值解析（未知值归缺省 Spaces4——与主题模式归一同口径）。
    pub fn from_setting(key: &str) -> Self {
        match key {
            "2" => IndentUnit::Spaces2,
            "tab" => IndentUnit::Tab,
            _ => IndentUnit::Spaces4,
        }
    }

    /// settings 序列化键值（缺省也写回——往返一致）。
    pub fn setting_key(self) -> &'static str {
        match self {
            IndentUnit::Spaces4 => "4",
            IndentUnit::Spaces2 => "2",
            IndentUnit::Tab => "tab",
        }
    }

    /// 反缩进的 Tab 渲染宽度近似（Tab 单位删前导空格时的折算宽度）。
    pub fn tab_width_approx(self) -> usize {
        match self {
            IndentUnit::Spaces4 | IndentUnit::Tab => 4,
            IndentUnit::Spaces2 => 2,
        }
    }
}

/// #163：undo 栈条目的手势几何伴记（Buffer 侧镜像核 History 条目）。
#[derive(Clone, Debug)]
struct EntryMeta {
    /// 编辑期 id（Buffer revision 口径——docx 桥映射恢复键）。
    id: u64,
    /// Text-only history has no corresponding semantic core undo entry.
    projection_only: bool,
    /// Source-state identity (not stack depth or a whole-document hash).
    before_state: u64,
    after_state: u64,
    /// Exact source envelope is available, even for a multi-edit gesture.
    exact_replay: bool,
    /// 手势起点（undo 光标落点）。
    start: Point,
    /// 手势被替换域终点（pre 编辑坐标）。
    end: Point,
    /// 手势后光标（redo 光标落点）。
    cursor_after: Point,
    /// 插入文本换行数（redo 的 fence inserted_lines）。
    t_nl: usize,
    /// 被删文本换行数（undo 的 fence inserted_lines）。
    o_nl: usize,
    /// 条目内编辑数（#205 fence 判据：单编辑 = Single 几何，多编辑
    /// （批量/合并窗吸收的多点编辑）= Full——保守正确）。
    n_edits: usize,
    /// 精确 source envelope（单编辑或显式批事务）；普通击键合并条目
    /// 无合成 envelope 时 exact_replay=false，走既有 op 翻译。
    old_text: String,
    /// 单编辑条目的插入文本（redo 投影回放面）。
    text: String,
}

/// G1 批量子编辑（#163 换芯：投影 char 域 + Point——undo/redo 由核
/// History 驱动，不再需要撤销/重放专用坐标域；old_text 保留供钩子
/// 意图与摘要）。
#[derive(Clone, Debug)]
struct BatchEdit {
    history_id: u64,
    /// 原始文档坐标起点（批首光标/汇报用）。
    start: Point,
    /// 原始文档删除域（投影 char offset——pre 状态一次算定）。
    c_del: (usize, usize),
    old_text: String,
    new_text: String,
}

/// 词字符判定（`word_jump` 与双击选词 [`super::mouse::word_range_at`] 共用
/// 口径：字母数字（含 CJK 汉字——连续汉字算一个词）+ 下划线）。
pub(crate) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// char 索引 → 字节偏移（越界 clamp 到末尾）。
fn char_to_byte(s: &str, col: usize) -> usize {
    s.char_indices().nth(col).map(|(b, _)| b).unwrap_or(s.len())
}

/// 字节偏移 → char 索引（G1 批量编辑光标逆换算；越界 clamp 末尾）。
///
/// `pub(crate)`：#132 正则查找面（`buffer_find` 分离 impl）复用同一坐标口径。
pub(crate) fn byte_to_char(s: &str, byte_col: usize) -> usize {
    s.char_indices().take_while(|(b, _)| *b < byte_col).count()
}

/// [`char_to_byte`] 的模块内公开转发（display_map 的段文本切片消费）。
pub(crate) fn char_to_byte_pub(s: &str, col: usize) -> usize {
    char_to_byte(s, col)
}

/// 纯逻辑编辑缓冲区。
pub struct Buffer {
    /// #163 换芯桥：mdcore::Core（唯一真源）+ ropey 投影 + 行↔块映射。
    bridge: MdBridge,
    cursor: Point,
    /// Shift 选区锚点（无选区为 `None`）。
    anchor: Option<Point>,
    /// 上下移动的期望列粘性（被短行 clamp 时记忆原列）。
    desired_col: Option<usize>,
    /// O(1) source-state tokens; revision allocates new identities on every edit.
    current_state: u64,
    saved_state: u64,
    /// UndoHook 的 history_id 镜像 + 手势几何伴记：核 undo 栈每条目
    /// 对应一条 EntryMeta（id/起点/终点/后光标/双侧换行数/编辑数/
    /// 四元组文本）。undo/redo 的光标与 #205 fence 记账全部由此推导
    /// ——与旧四元组语义逐位对齐（合并条目记首编辑几何）。
    entry_ids: Vec<EntryMeta>,
    /// redo 镜像（同 entry_ids；mark_saved 清空与核同源）。
    redo_ids: Vec<EntryMeta>,
    /// #163：合并窗缺省（settings undo_coalesce_ms；挂 docx 钩子时
    /// 置 0 保持逐编辑粒度——钩子摘除后恢复）。
    default_coalesce_ms: u64,
    /// 文档版本号（#111 订阅模式）：每次变更事务 +1（replace_range /
    /// replace_batch / undo / redo——`mark_saved` 不算变更）。事件消费方
    /// 靠它检测乱序与丢事件。初始 0。
    revision: u64,
    /// 待发变更记录（#111 订阅模式）：每次变更事务 push，Editor 收尾
    ///（`after_edit`/undo/redo 路径）经 [`Buffer::take_pending`] 一次性
    /// 收割组装事件——纯光标移动类收尾时 pending 为空即不发事件。
    /// Clone 快照不带 pending（克隆体不是事件源）。
    pending: Vec<crate::doc_events::ChangeRecord>,
    /// pending 首条入队时的版本基线（一次收尾可能合并多步事务——
    /// 如一次 undo 手势 N 步；事件携带 base..current 跨度让消费方
    /// 判定中间是否有未见过的版本跳变）。
    pending_base: Option<u64>,
    /// #178：最近一次拒改记录（`EditHook::before_edit` 回 `Err` 时落；
    /// 编辑器收尾经 [`Buffer::take_rejection`] 一次性收割——GUI 可见
    /// 反馈的数据源。stderr 留痕不变，本字段只是补「用户看得见」面）。
    /// Clone 快照不带（克隆体不是事件源——与 pending 同口径）。
    last_reject: Option<RejectNotice>,
    /// #115：编辑意图钩子（docx 可编辑态挂桥；None = 纯文本面）。
    ///
    /// 语义：**变更之前**询问（`EditIntent` 回 `Err` = 拒绝本次编辑，
    /// Buffer 零变更、undo 栈零记录——锁定行/行间合并的防线）；
    /// **变更之后**通报（`TextChanged`——宿主据此重映射块表 + 节流回写）。
    /// 钩子只在 Buffer 单点入口（[`Buffer::replace_range`] /
    /// [`Buffer::replace_batch`]）处触发，undo/redo 不触发（撤销回放走的
    /// 是桥层投影编辑——撤销「物理上」必须生效，语义由核历史层
    /// 重新落账承担）。
    edit_hook: Option<Box<dyn EditHook>>,
    /// #115：撤销/重做通知（宿主据此同步行归属映射；None = 纯文本面）。
    undo_hook: Option<Box<dyn UndoHook>>,
    /// #205：fence 增量重建的脏行记账（自上次收割以来的变更几何——
    /// 见 [`FenceDirty`]）。所有文本变更入口（`replace_range` /
    /// `replace_batch` / undo / redo）在此落账；Editor::refresh_caches
    /// 经 [`Buffer::take_fence_dirty`] 一次性收割。Clone 快照不带记账
    /// （克隆体不是变更源——与 pending/last_reject 同口径；克隆体的首次
    /// 收割是 Clean → 全量，恒安全）。
    fence_dirty: FenceDirty,
}

impl Clone for Buffer {
    /// #163：深拷贝经 `Core::clone`（完整快照：树 + 历史栈 + revision
    /// 全量复制——对照旧 `inner.branch()` 共享历史，本实现是更强语义）。
    fn clone(&self) -> Self {
        Self {
            bridge: self.bridge.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
            desired_col: self.desired_col,
            current_state: self.current_state,
            saved_state: self.saved_state,
            entry_ids: self.entry_ids.clone(),
            redo_ids: self.redo_ids.clone(),
            default_coalesce_ms: self.default_coalesce_ms,
            revision: self.revision,
            pending: Vec::new(),
            pending_base: None,
            last_reject: None,
            // 钩子不随克隆走（克隆体是快照；新宿主自行接线）。
            edit_hook: None,
            undo_hook: None,
            // #205：记账不随克隆走（克隆体不是变更源；见字段注释）。
            fence_dirty: FenceDirty::Clean,
        }
    }
}

impl Default for Buffer {
    fn default() -> Self {
        Self::from_plain_text("")
    }
}

// ---------------------------------------------------------------------------
// #115：编辑意图钩子（docx 可编辑态与 Buffer 的接口）
// ---------------------------------------------------------------------------

/// 一次文本变更的几何摘要（样式无关——行/列 + 是否删过换行）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditIntent {
    /// 变更起点（含）——编辑器坐标。
    pub start: Point,
    /// 变更终点（不含）。
    pub end: Point,
    /// 变更后插入文本的行数（0 = 纯删除）。
    pub inserted_lines: usize,
    /// 变更是否删到了换行（跨行合并 = 本行吃掉下一行）。
    pub joins_next: bool,
    /// 变更是否从上一行行尾开始（= 删除行首换行，与前一行合并）。
    pub joins_prev: bool,
}

/// 变更结果摘要（钩子侧据此重映射块表）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditSummary {
    /// 同一 undo 条目的稳定身份；undo/redo 回放保持该 ID。
    pub history_id: u64,
    /// 变更前被替换区间的终点，不能用变更后 cursor 推算。
    pub before_end: Point,
    /// 变更起点。
    pub start: Point,
    /// 变更后插入文本末尾（纯删除时等同起点）。
    pub end: Point,
    /// 变更后总行数。
    pub lines: usize,
    /// 变更前被删除文本里是否含换行（区分「删除一段文字」与「删行合并」）。
    pub newline_removed: bool,
    /// 变更源（`false` = 派生复合编辑——查找替换重复项剥离等内部二次变更）。
    pub user_intent: bool,
    /// 插入的行数（含跨行时后行新增）。
    pub inserted_lines: usize,
}

/// 撤销/重做通知（独立于 [`EditHook`]）：
///
/// 撤销/重做是**物理回放**（不经 Buffer 唯一变更入口），但宿主侧的行
/// 归属映射必须跟着走——否则「删行 → 撤销」之后映射表少了某段的行，
/// 保存时就认不出这段动没动（#115 实测坑：撤销到底后标题丢失）。
pub trait UndoHook {
    /// 撤销/重做前（`redo = false` = 撤销）：行数变化量，宿主据此先
    /// 调整映射行数（返回 `Err` 不拦——撤销不可拦，仅作诊断）。
    fn before_history(&mut self, redo: bool, delta_lines: i64) -> Result<(), String>;
    /// 精确历史身份（旧实现默认转发，通用 DOCX 桥据 ID 恢复对应映射）。
    fn before_history_at(
        &mut self,
        redo: bool,
        delta_lines: i64,
        _history_id: u64,
    ) -> Result<(), String> {
        self.before_history(redo, delta_lines)
    }
    /// 撤销/重做后实际行数（宿主对齐映射行数）。
    fn after_history(&mut self, redo: bool, lines: usize);
}

/// Buffer 编辑钩子（宿主实现；docx 可编辑态的桥挂在这里）。
pub trait EditHook {
    /// 变更**之前**：`Err(原因)` = 拒绝本次编辑（Buffer 零变更）。
    fn before_edit(&mut self, intent: &EditIntent) -> Result<(), String>;
    /// 变更**之后**：宿主据摘要重映射（块表/映射状态）。
    fn after_edit(&mut self, summary: &EditSummary);
    /// 批内子编辑按原始坐标逆序排列；同 ID 为一个撤销单元。
    /// 旧钩子仍接收原有汇总，精确映射钩子覆盖此方法消费子编辑。
    fn after_batch(&mut self, _edits: &[EditSummary], aggregate: &EditSummary) {
        self.after_edit(aggregate);
    }
}

/// #205：fence 增量重建的脏行记账（Buffer 侧记账，Editor 侧
/// `refresh_caches` 经 [`Buffer::take_fence_dirty`] 一次性收割消费）。
///
/// 语义：自上次收割以来的变更几何——
/// - [`FenceDirty::Single`]：恰一次**单区间**替换（`replace_range` /
///   undo / redo 的单点回放）→ Editor 可从 `line` 起增量推进；
/// - [`FenceDirty::Full`]：多区间（`replace_batch` / 批量撤销重放）或
///   连续多次单区间编辑（多光标逐光标 insert 等）——合并几何不可靠，
///   回退全量重建；
/// - [`FenceDirty::Clean`]：无变更（初始态 / 收割后复位）→ 全量
///   （初次构建缓存也走全量）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FenceDirty {
    /// 自上次收割以来无文档变更。
    #[default]
    Clean,
    /// 恰一次单区间替换的几何：
    /// - `line`：变更起点行（**编辑时**文档坐标——窗口内仅有这一次
    ///   变更，编辑时文档即消费方缓存所描述的文档）；
    /// - `inserted_lines`：插入文本的换行数（编辑影响区新坐标
    ///   `[line, line + inserted_lines + 1)`）；
    /// - `pre_lines`：编辑前总行数（消费方校验 `fence_cache` 长度
    ///   一致——失配即缓存不描述编辑前文档，回退全量）。
    Single {
        line: usize,
        inserted_lines: usize,
        pre_lines: usize,
    },
    /// 多区间 / 连续多编辑 / 批量撤销重放等不确定几何——回退全量。
    Full,
}

/// #178：一次拒改的记录（GUI 反馈数据源）。
///
/// 拒改点（`replace_range` / `replace_batch` 的钩子 `Err` 分支）落账，
/// 编辑器收尾收割 → 状态栏提示；与 stderr 留痕并行（诊断面不动）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectNotice {
    /// 面向用户的拒改原因（docx 桥给的是用户可读文案）。
    pub reason: String,
    /// 被拒变更的起点行（编辑器行号——提示定位/行内呈现用）。
    pub line: usize,
}

impl std::fmt::Debug for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Buffer")
            .field("lines", &self.line_count())
            .field("chars", &self.char_count())
            .field("cursor", &self.cursor)
            .field("modified", &self.is_modified())
            .finish_non_exhaustive()
    }
}

impl Buffer {
    // ---------------------------------------------------------------------------
    // 构造与序列化
    // ---------------------------------------------------------------------------

    /// 从纯文本构造（行语义按 `\n` 划分；空文本 = 单空行）。
    /// 经 `new_normalized` 直通——不走 `Buffer::new` 的 `\r\n`→`\n`
    /// normalize（mang-shell 行语义中 `\r` 属行内容，roundtrip 逐位保真）。
    pub fn from_plain_text(text: &str) -> Self {
        Self {
            bridge: MdBridge::new(text),
            cursor: Point::default(),
            anchor: None,
            desired_col: None,
            current_state: 0,
            saved_state: 0,
            entry_ids: Vec::new(),
            redo_ids: Vec::new(),
            default_coalesce_ms: mdcore::history::COALESCE_MS,
            revision: 0,
            pending: Vec::new(),
            pending_base: None,
            last_reject: None,
            edit_hook: None,
            undo_hook: None,
            fence_dirty: FenceDirty::Clean,
        }
    }

    /// 序列化为纯文本（`\r\n` 等行尾原样——与构造逐位互逆）。
    pub fn to_plain_text(&self) -> String {
        self.bridge.to_plain_text()
    }

    // ---------------------------------------------------------------------------
    // 只读访问
    // ---------------------------------------------------------------------------

    /// 行数。
    pub fn line_count(&self) -> usize {
        self.bridge.line_count()
    }

    /// 指定行内容（越界返回空串——渲染层免判）。
    /// 返回 `String`（rope 无行借用面）；行长场景的分配开销见头注释。
    pub fn line(&self, line: usize) -> String {
        self.bridge.line(line)
    }

    /// 指定行 char 数。
    pub fn line_len(&self, line: usize) -> usize {
        self.line(line).chars().count()
    }

    /// 光标点。
    pub fn cursor(&self) -> Point {
        self.cursor
    }

    /// 选区（anchor 与 cursor 排序后的 `[start, end)`；无选区 `None`）。
    pub fn selection(&self) -> Option<std::ops::Range<Point>> {
        self.anchor.map(|a| {
            if a <= self.cursor {
                a..self.cursor
            } else {
                self.cursor..a
            }
        })
    }

    /// 选区文本（含跨行 `\n`；无选区空串）。
    pub fn selection_text(&self) -> String {
        match self.selection() {
            None => String::new(),
            Some(range) => self.range_text(&range),
        }
    }

    /// 全文 char 总数（可见字符 + 换行；CJK 逐字语义天然成立）。
    /// #163：标书口径字数另见 [`Buffer::word_count`]（mdcore 单一真源）。
    pub fn char_count(&self) -> usize {
        self.bridge.char_count()
    }

    /// #163（验收 4）：标书口径字数（mdcore word_count——汉字数 +
    /// 西文词数的单一真源；此前 B 轨 char_count 自算口径绕开了它）。
    pub fn word_count(&self) -> mdcore::word_count::WordCount {
        self.bridge.word_count()
    }

    /// #163：核 revision（字数缓存的守卫键——视图面 revision 另见
    /// [`Buffer::revision`]）。
    pub fn core_revision(&self) -> u64 {
        self.bridge.core().revision()
    }

    /// `[start, end)` 范围文本（跨行以 `\n` 连接；两点自动排序）。
    pub fn range_text(&self, range: &std::ops::Range<Point>) -> String {
        let Range { start, end } = self.ordered_range(range.clone());
        if start == end {
            return String::new();
        }
        if start.line == end.line {
            let s = self.line(start.line);
            let b0 = char_to_byte(&s, start.col);
            let b1 = char_to_byte(&s, end.col);
            return s[b0..b1].to_string();
        }
        let mut out = String::new();
        // 首行尾段
        let first = self.line(start.line);
        out.push_str(&first[char_to_byte(&first, start.col)..]);
        // 中间整行
        for l in start.line + 1..end.line {
            out.push('\n');
            out.push_str(&self.line(l));
        }
        // 末行头段
        out.push('\n');
        let last = self.line(end.line);
        out.push_str(&last[..char_to_byte(&last, end.col)]);
        out
    }

    /// 是否偏离保存时的 source-state 身份（同深度的新分支仍为 dirty）。
    pub fn is_modified(&self) -> bool {
        self.current_state != self.saved_state
    }

    /// 显式落盘：记录 source-state 身份并 fence 后续击键；保留 undo，
    /// 清空 redo（既有保存契约）。
    pub fn mark_saved(&mut self) {
        self.saved_state = self.current_state;
        self.bridge.break_coalesce();
        self.bridge.mark_saved();
        self.redo_ids.clear();
    }

    // ---------------------------------------------------------------------------
    // 编辑原语
    // ---------------------------------------------------------------------------

    /// 设置编辑钩子（#115：docx 可编辑态挂桥；`None` = 纯文本面无门禁）。
    /// 设置撤销/重做通知（#115：docx 映射随撤销同步）。
    pub fn set_undo_hook(&mut self, hook: Option<Box<dyn UndoHook>>) {
        self.undo_hook = hook;
        self.refresh_coalesce_window();
    }

    pub fn take_edit_hook(&mut self) -> Option<Box<dyn EditHook>> {
        self.edit_hook.take()
    }

    /// #178：收割最近一次拒改记录（取出即清空——编辑器收尾单点消费；
    /// 无拒改/已收割 = `None`）。与 [`Buffer::take_pending`] 同「一次性
    /// 收割」口径：拒改是事件性事实，收割后不留残影。
    pub fn take_rejection(&mut self) -> Option<RejectNotice> {
        self.last_reject.take()
    }

    /// #178：拒改记录窥视（不取走——克隆口径测试/诊断用）。
    pub(crate) fn last_reject_note(&self) -> Option<&RejectNotice> {
        self.last_reject.as_ref()
    }

    pub fn set_edit_hook(&mut self, hook: Option<Box<dyn EditHook>>) {
        self.edit_hook = hook;
        self.refresh_coalesce_window();
    }

    /// #163：合并窗裁决——docx 面（任一钩子在）置 0（逐编辑粒度，
    /// EditHook/UndoHook 的 history_id 1:1 契约不破）；纯 md 面恢复
    /// 设置值（缺省 1s——验收 2 的核同源合并窗）。
    fn refresh_coalesce_window(&mut self) {
        let ms = if self.edit_hook.is_some() || self.undo_hook.is_some() {
            0
        } else {
            self.default_coalesce_ms
        };
        self.bridge.set_coalesce_ms(ms);
    }

    /// #163（验收 8）：合并窗设置（settings `undo_coalesce_ms` → 引擎；
    /// docx 面挂钩期间暂存，摘钩后生效）。
    pub fn set_coalesce_ms(&mut self, ms: u64) {
        self.default_coalesce_ms = ms;
        self.refresh_coalesce_window();
    }

    pub(crate) fn point_to_coff(&self, point: Point) -> usize {
        self.bridge.point_to_coff(point)
    }

    pub(crate) fn coff_to_point(&self, offset: usize) -> Point {
        self.bridge.coff_to_point_p(offset)
    }

    /// #163：命令边界破组——下一次编辑必开新 undo 条目（命令级手势
    /// 分界：查找替换逐条替换等「显式命令」各自成步，不并入此前击键
    /// 的合并窗；与批量路径（桥内置破组）同语义）。
    pub fn break_undo_group(&mut self) {
        self.bridge.break_coalesce();
    }

    // -----------------------------------------------------------------------
    // #205：fence 脏行记账（Editor::refresh_caches 消费）
    // -----------------------------------------------------------------------

    /// #205：记账一次单区间变更（replace_range / undo / redo 的单点
    /// 回放路径）。**必须在桥层编辑之前调**（pre_lines 取编辑前
    /// 行数）。已有未收割窗口时升级 Full——连续多次单区间编辑的合并
    /// 几何不可靠（多光标逐光标编辑等），回退全量（现状行为，语义
    /// 恒等）。
    fn note_fence_single(&mut self, line: usize, inserted_lines: usize) {
        let pre_lines = self.line_count();
        self.fence_dirty = match self.fence_dirty {
            FenceDirty::Clean => FenceDirty::Single {
                line,
                inserted_lines,
                pre_lines,
            },
            _ => FenceDirty::Full,
        };
    }

    /// #205：undo/redo 路径的记账（pre_lines 由伴记反推——此时文档
    /// 已是变更后状态，不能重取行数）。
    fn note_fence_single_pre(&mut self, line: usize, inserted_lines: usize, pre_lines: usize) {
        self.fence_dirty = match self.fence_dirty {
            FenceDirty::Clean => FenceDirty::Single {
                line,
                inserted_lines,
                pre_lines,
            },
            _ => FenceDirty::Full,
        };
    }

    /// #205：记账不确定几何（replace_batch / 批量撤销重放）——回退全量。
    fn note_fence_full(&mut self) {
        self.fence_dirty = FenceDirty::Full;
    }

    /// #205：收割 fence 脏行记账（取出即复位 Clean——refresh_caches
    /// 单点消费，与 `take_pending`/`take_rejection` 同「一次性收割」
    /// 口径）。
    pub fn take_fence_dirty(&mut self) -> FenceDirty {
        std::mem::replace(&mut self.fence_dirty, FenceDirty::Clean)
    }

    /// **唯一文本变更入口**：删除 `[start, end)` 再插入 `text`（可含 `\n`）。
    /// 光标落在新插入文本末尾；清选区与列粘性；清 redo 栈。
    ///
    /// #115：变更前问钩子（拒 → 零变更返回 false），变更后通报摘要。
    /// 返回是否实际变更。
    pub fn replace_range(&mut self, start: Point, end: Point, text: &str) -> bool {
        self.replace_range_with_intent(start, end, text, true)
    }

    fn replace_range_with_intent(
        &mut self,
        start: Point,
        end: Point,
        text: &str,
        user_intent: bool,
    ) -> bool {
        let Range { start, end } = self.ordered_range(start..end);
        // no-op 短路（#111）：空域 + 空文本 = 无内容变化——不落 undo
        // 记录、不计版本、不发事件（防无意义事件流 + undo 栈噪音）。
        if start == end && text.is_empty() {
            return false;
        }
        let old_text = self.range_text(&(start..end));
        let text_lines = text.matches('\n').count();
        let newline_removed = old_text.contains('\n');
        let intent = EditIntent {
            start,
            end,
            inserted_lines: text_lines,
            // 行尾换行被删：本行吃掉下一行（终点在本行之后）。
            joins_next: end.line > start.line && start.col > 0,
            // 行首换行被删：起点在行尾且删到了下一行首。
            joins_prev: old_text.starts_with('\n'),
        };
        if user_intent
            && let Some(hook) = self.edit_hook.as_mut()
            && let Err(reason) = hook.before_edit(&intent)
        {
            eprintln!("edit-guard: 拒绝编辑（{reason}）");
            // #178：拒改落账（GUI 反馈面——stderr 之外的可见通道）。
            self.last_reject = Some(RejectNotice {
                reason,
                line: start.line,
            });
            return false;
        }
        // #205：脏行记账（钩子已放行、变更必然发生；编辑前取行数）。
        self.note_fence_single(start.line, text_lines);
        // #163 换芯：桥单点（快路径 op / 兜底 sync——核为真源先行，
        // 拒绝则投影不动、返回 false 与钩子拒改同面）。
        let core_revision = self.bridge.core().revision();
        if !self.bridge.edit(start, end, text) {
            return false;
        }
        // #111 订阅模式：precise 记录（end = 变更前被替换域终点；
        // start == end 即纯插入）。note 在版本递增**前**——基线 =
        // 变更前版本（revision_before 语义）。
        self.note_change(crate::doc_events::ChangeRecord::precise(
            "replace",
            start,
            end,
            text.to_string(),
        ));
        self.revision += 1;
        // #163：核事件收割（防队列堆积）+ 条目镜像维护（合并入既有
        // 条目时仅累计编辑数——fence 判据）。
        let mut projection_only = self.bridge.core().revision() == core_revision;
        let (core_entry_created, core_event_seen) = self.bridge.take_view_events();
        // A Markdown no-op produces **no** core View event at all; a coalesced
        // core edit produces an event without a new entry. Only the former is
        // projection-only, so check for the event rather than the revision.
        if projection_only && core_event_seen {
            projection_only = false;
        }
        if core_entry_created || projection_only {
            let cursor = self.end_after_insert(&start, text);
            self.entry_ids.push(EntryMeta {
                id: self.revision,
                projection_only,
                before_state: self.current_state,
                after_state: self.revision,
                exact_replay: true,
                start,
                end,
                cursor_after: cursor,
                t_nl: text.matches('\n').count(),
                o_nl: old_text.matches('\n').count(),
                n_edits: 1,
                old_text: old_text.clone(),
                text: text.to_string(),
            });
            self.redo_ids.clear();
        } else if let Some(last) = self.entry_ids.last_mut() {
            last.n_edits += 1;
            last.after_state = self.revision;
            last.exact_replay = false;
        }
        self.current_state = self.revision;
        if projection_only {
            // A source-only ledger entry must not split a coalesced Core entry.
            self.bridge.break_coalesce();
        }
        let cursor = self.end_after_insert(&start, text);
        self.anchor = None;
        self.desired_col = None;
        self.cursor = cursor;
        self.clamp_cursor();
        let line_count = self.line_count();
        if let Some(hook) = self.edit_hook.as_mut() {
            hook.after_edit(&EditSummary {
                history_id: self.revision,
                before_end: end,
                start,
                end: cursor,
                lines: line_count,
                inserted_lines: text_lines,
                newline_removed,
                user_intent,
            });
        }
        true
    }

    /// **批量编辑原语**（G1 查找替换“全部替换”等一撤销多点位消费）：
    /// 一次应用多点替换 + **落一条** undo 记录（`Ctrl+Z` 一次整批回滚）。
    ///
    /// 实现口径（#91 G1 验收：全部替换 = **一次**批量编辑，不是循环
    /// 单点替换）：全部坐标先行换算（文档未动、Point 恒有效），
    /// 交桥层单点（全快路径 = 核一次 `apply_ops` 单条目；否则一次
    /// `sync_from_markdown` 兜底——一撤销步语义保持）。
    ///
    /// 契约：各替换互不重叠（重叠/同起点整批拒绝）；原始 snapshot 的
    /// char 坐标排序后一次捕获。空域空文本忽略，返回实际应用点数。
    /// 光标落在文档序最后一个替换的尾部；清选区/列粘性；清 redo 栈。
    pub fn replace_batch(&mut self, edits: Vec<(Point, Point, String)>) -> usize {
        if edits.is_empty() {
            return 0;
        }
        let mut ordered: Vec<_> = edits
            .into_iter()
            .filter_map(|(s, e, text)| {
                let Range { start, end } = self.ordered_range(s..e);
                (start != end || !text.is_empty()).then_some((start, end, text))
            })
            .collect();
        ordered.sort_by_key(|(s, _, _)| *s);
        if ordered.is_empty() {
            return 0;
        }
        // 先行换算：old_text 捕获 + 双坐标域（原始/应用后）一次算定。
        let mut records: Vec<BatchEdit> = Vec::with_capacity(ordered.len());
        // #111：pending 预留容量（千处批量 = 千条记录——避免均摊扩容
        // 的重复分配；perf 线 <100ms 贴线环境下的微优化）。
        self.pending.reserve(ordered.len());
        // #111 订阅模式：每点位一条 precise 记录（start/end = 变更前
        // 域，坐标在文档未动时先行换算——恒有效），随 records 同循环
        // 构建避免二次迭代（一次事务版本只 +1 在事务尾）。
        // #163：投影 char 域（pre 状态一次算定——桥 batch 消费）。
        for (start, end, new_text) in ordered {
            let old_text = self.range_text(&(start..end));
            let c_start = self.bridge.point_to_coff(start);
            let c_end_del = self.bridge.point_to_coff(end);
            if records
                .last()
                .is_some_and(|prev: &BatchEdit| prev.c_del.1 > c_start || prev.c_del.0 == c_start)
            {
                self.last_reject = Some(RejectNotice {
                    reason: "overlapping batch ranges".into(),
                    line: start.line,
                });
                return 0;
            }
            records.push(BatchEdit {
                history_id: self.revision + 1,
                start,
                c_del: (c_start, c_end_del),
                old_text,
                new_text,
            });
        }
        let mut newline_removed_batch = false;
        // #115：变更前逐条问钩子（锁定行/跨行合并的防线；整批原子——任何
        // 一条被拒 = 整批拒，Buffer 零变更）。
        // 预检时文本与映射尚未变更，所有意图都使用原始删除区间。
        // 成功后精确结果按原始坐标逆序通知，前项不会被后项行差影响。
        // 快速路径：无钩子（md 文本面）且整批都不含换行——省去逐条意图
        // 装箱（千处替换的计时门禁在毫秒级，装箱开销会压线）。
        let need_hook = self.edit_hook.is_some()
            || records
                .iter()
                .any(|r| r.old_text.contains('\n') || r.new_text.contains('\n'));
        for r in records.iter().filter(|_| need_hook) {
            newline_removed_batch |= r.old_text.contains('\n');
            let before_end = self.end_after_insert(&r.start, &r.old_text);
            let intent = EditIntent {
                start: r.start,
                end: before_end,
                inserted_lines: r.new_text.matches('\n').count(),
                joins_next: before_end.line > r.start.line && r.start.col > 0,
                joins_prev: r.old_text.starts_with('\n'),
            };
            if let Some(hook) = self.edit_hook.as_mut()
                && let Err(reason) = hook.before_edit(&intent)
            {
                eprintln!("edit-guard: 拒绝批量替换（{reason}）");
                self.last_reject = Some(RejectNotice {
                    reason,
                    line: r.start.line,
                });
                return 0;
            }
        }
        let before_lines = self.line_count();
        let before_end = records
            .last()
            .map(|r| self.end_after_insert(&r.start, &r.old_text))
            .expect("nonempty batch");
        let mut summaries = Vec::new();
        if self.edit_hook.is_some() {
            let mut mapped_lines = before_lines;
            for r in records.iter().rev() {
                let inserted = r.new_text.matches('\n').count();
                mapped_lines = mapped_lines - r.old_text.matches('\n').count() + inserted;
                summaries.push(EditSummary {
                    history_id: r.history_id,
                    start: r.start,
                    before_end: self.end_after_insert(&r.start, &r.old_text),
                    end: self.end_after_insert(&r.start, &r.new_text),
                    lines: mapped_lines,
                    newline_removed: r.old_text.contains('\n'),
                    user_intent: true,
                    inserted_lines: inserted,
                });
            }
        }
        // Exact source envelope, including untouched blanks/CRLF between edits.
        // Core remains the semantic history authority; this is its source companion.
        let first = records.first().expect("nonempty batch");
        let last = records.last().expect("nonempty batch");
        let envelope_start = first.start;
        let envelope_end = self.end_after_insert(&last.start, &last.old_text);
        let old_envelope = self.range_text(&(envelope_start..envelope_end));
        let mut applied_end = last.c_del.1 as i64;
        for r in &records {
            applied_end += r.new_text.chars().count() as i64 - (r.c_del.1 - r.c_del.0) as i64;
        }
        let n = records.len();
        // #163 换芯：桥批量单点（全快路径 = 核一次 `apply_ops` 单条目；
        // 否则一次 `sync_from_markdown` 兜底；拒绝 = 整批不生效。
        let batch_pts: Vec<(Point, Point, String)> = records
            .iter()
            .map(|r| {
                let end = self.end_after_insert_pub(&r.start, &r.old_text);
                (r.start, end, r.new_text.clone())
            })
            .collect();
        if !self.bridge.edit_batch(&batch_pts) {
            return 0;
        }
        // Publish only after acceptance: rejection changes neither events nor fences.
        for r in &records {
            self.note_change(crate::doc_events::ChangeRecord::precise(
                "replace",
                r.start,
                self.end_after_insert(&r.start, &r.old_text),
                r.new_text.clone(),
            ));
        }
        self.note_fence_full();
        self.revision += 1;
        let (created, semantic) = self.bridge.take_view_events();
        debug_assert!(
            !semantic || created,
            "explicit batch must be a new Core entry"
        );
        let cursor_after = self.bridge.coff_to_point_p(applied_end as usize);
        let new_envelope = self.range_text(&(envelope_start..cursor_after));
        self.entry_ids.push(EntryMeta {
            id: self.revision,
            projection_only: !semantic,
            before_state: self.current_state,
            after_state: self.revision,
            exact_replay: true,
            start: envelope_start,
            end: envelope_end,
            cursor_after,
            t_nl: records
                .iter()
                .map(|r| r.new_text.matches('\n').count())
                .sum(),
            o_nl: records
                .iter()
                .map(|r| r.old_text.matches('\n').count())
                .sum(),
            n_edits: records.len(),
            old_text: old_envelope,
            text: new_envelope,
        });
        self.current_state = self.revision;
        self.redo_ids.clear();
        self.cursor = cursor_after;
        let first_start = records.first().map(|r| r.start).unwrap_or(self.cursor);
        let inserted_lines: usize = records
            .iter()
            .map(|r| r.new_text.matches('\n').count())
            .sum();
        self.anchor = None;
        self.desired_col = None;
        self.clamp_cursor();
        let cursor_now = self.cursor;
        let line_count = self.line_count();
        if let Some(hook) = self.edit_hook.as_mut() {
            hook.after_batch(
                &summaries,
                &EditSummary {
                    history_id: self.revision,
                    before_end,
                    start: first_start,
                    end: cursor_now,
                    lines: line_count,
                    inserted_lines,
                    newline_removed: newline_removed_batch,
                    user_intent: true,
                },
            );
        }
        n
    }

    /// #115：**派生编辑**（非用户意图的内部二次变更：查找替换的重复项剥离）。
    ///
    /// 与 [`Buffer::replace_range`] 同一变更语义，只跳过意图裁决；仍通报
    /// 准确结果与历史身份，防止行归属与撤销栈悄然失同步。
    pub fn replace_range_derived(&mut self, start: Point, end: Point, text: &str) -> bool {
        self.replace_range_with_intent(start, end, text, false)
    }

    /// 在光标处插入文本（有选区则先被替换）。
    pub fn insert(&mut self, text: &str) {
        let (s, e) = self.deletion_range();
        self.replace_range(s, e, text);
    }

    /// 换行（Enter）。
    pub fn newline(&mut self) {
        self.insert("\n");
    }

    /// Tab（md 最小语义：插 4 空格，不引入缩进块模型）。
    pub fn tab(&mut self) {
        self.insert("    ");
    }

    /// 退格（Backspace）：选区删除；行首与前行合并；否则删光标前一字符。
    pub fn backspace(&mut self) {
        if self.selection().is_some() {
            self.delete_selection();
            return;
        }
        let c = self.cursor;
        if c.col > 0 {
            let s = Point::new(c.line, c.col - 1);
            self.replace_range(s, c, "");
        } else if c.line > 0 {
            // 行首：与前一行合并（删两行间的隐式换行）。
            let s = Point::new(c.line - 1, self.line_len(c.line - 1));
            self.replace_range(s, c, "");
        }
    }

    /// 前删（Delete）：选区删除；行尾与下行合并；否则删光标后一字符。
    pub fn delete_forward(&mut self) {
        if self.selection().is_some() {
            self.delete_selection();
            return;
        }
        let c = self.cursor;
        if c.col < self.line_len(c.line) {
            let e = Point::new(c.line, c.col + 1);
            self.replace_range(c, e, "");
        } else if c.line + 1 < self.line_count() {
            let e = Point::new(c.line + 1, 0);
            self.replace_range(c, e, "");
        }
    }

    /// 删除当前选区（无选区无操作）。
    pub fn delete_selection(&mut self) {
        if let Some(range) = self.selection() {
            self.replace_range(range.start, range.end, "");
        }
    }

    /// 当前编辑命令的删除范围：有选区取选区，无选区取光标零宽。
    fn deletion_range(&self) -> (Point, Point) {
        match self.selection() {
            Some(r) => (r.start, r.end),
            None => (self.cursor, self.cursor),
        }
    }

    // ---------------------------------------------------------------------------
    // undo / redo
    // ---------------------------------------------------------------------------

    /// 撤销一步（#163：mdcore History 驱动——合并窗语义与核同源）；
    /// 返回是否发生撤销。
    pub fn undo(&mut self) -> bool {
        let Some(meta) = self.entry_ids.pop() else {
            return false;
        };
        // #163：单编辑条目走**四元组回放**（投影 = 占位换回原文——与
        // 旧 r39 语义逐位等价：空行/\r/行几何全部按原文复原，不经
        // 序列化重建）；多编辑条目走 op 翻译（类 a/a2/b）。
        let replay = meta
            .exact_replay
            .then(|| (meta.start, meta.cursor_after, meta.old_text.clone()));
        let outcome = if meta.projection_only {
            self.bridge.replay_projection(meta.start, replay)
        } else {
            self.bridge.undo_replay(meta.start, meta.end, replay)
        };
        let Some(outcome) = outcome else {
            // 栈镜像失配（理论不可达）——压回保形。
            self.entry_ids.push(meta);
            return false;
        };
        let delta = outcome.delta_lines;
        if let Some(h) = self.undo_hook.as_mut() {
            let _ = h.before_history_at(false, delta, meta.id);
        }
        self.current_state = meta.before_state;
        self.redo_ids.push(meta.clone());
        // #205：fence 几何从伴记推导（单编辑条目 = Single——与旧
        // 四元组物理回放的记账语义逐位一致；多编辑条目（批量/合并
        // 吸收的多点编辑）= Full——保守正确）。pre_lines 由当前行数
        // 反推（桥已应用）。
        if meta.n_edits == 1 {
            let pre_lines =
                (self.line_count() as i64 - (meta.o_nl as i64 - meta.t_nl as i64)) as usize;
            self.note_fence_single_pre(meta.start.line, meta.o_nl, pre_lines);
        } else {
            self.note_fence_full();
        }
        // A multi-edit gesture remains a coarse history event; the exact source
        // envelope is an internal replay companion, not a single user edit.
        match &outcome.precise.as_ref().filter(|_| meta.n_edits == 1) {
            Some((s, e, t)) => self.note_change(crate::doc_events::ChangeRecord::precise(
                "undo",
                *s,
                *e,
                t.clone(),
            )),
            None => self.note_change(crate::doc_events::ChangeRecord::coarse("undo")),
        }
        self.revision += 1;
        self.anchor = None;
        self.desired_col = None;
        self.cursor = meta.start;
        self.clamp_cursor();
        let lines = self.line_count();
        if let Some(h) = self.undo_hook.as_mut() {
            h.after_history(false, lines);
        }
        true
    }

    /// 重做一步（#163：mdcore History 驱动）；返回是否发生重做。
    pub fn redo(&mut self) -> bool {
        let Some(meta) = self.redo_ids.pop() else {
            return false;
        };
        let replay = meta
            .exact_replay
            .then(|| (meta.start, meta.end, meta.text.clone()));
        let outcome = if meta.projection_only {
            self.bridge.replay_projection(meta.cursor_after, replay)
        } else {
            self.bridge.redo_replay(meta.cursor_after, meta.end, replay)
        };
        let Some(outcome) = outcome else {
            self.redo_ids.push(meta);
            return false;
        };
        let delta = outcome.delta_lines;
        if let Some(h) = self.undo_hook.as_mut() {
            let _ = h.before_history_at(true, delta, meta.id);
        }
        self.current_state = meta.after_state;
        self.entry_ids.push(meta.clone());
        // #205：redo 的 fence 几何（同 undo 面——插入侧 = t_nl）。
        if meta.n_edits == 1 {
            let pre_lines =
                (self.line_count() as i64 - (meta.t_nl as i64 - meta.o_nl as i64)) as usize;
            self.note_fence_single_pre(meta.start.line, meta.t_nl, pre_lines);
        } else {
            self.note_fence_full();
        }
        // A multi-edit gesture remains a coarse history event; the exact source
        // envelope is an internal replay companion, not a single user edit.
        match &outcome.precise.as_ref().filter(|_| meta.n_edits == 1) {
            Some((s, e, t)) => self.note_change(crate::doc_events::ChangeRecord::precise(
                "redo",
                *s,
                *e,
                t.clone(),
            )),
            None => self.note_change(crate::doc_events::ChangeRecord::coarse("redo")),
        }
        self.revision += 1;
        self.anchor = None;
        self.desired_col = None;
        self.cursor = meta.cursor_after;
        self.clamp_cursor();
        let lines = self.line_count();
        if let Some(h) = self.undo_hook.as_mut() {
            h.after_history(true, lines);
        }
        true
    }

    /// 文档版本号（#111 订阅模式——事件流的顺序/丢失判定位）。
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// 待发记录入队（首次入队时定格版本基线 `pending_base`——多次事务
    /// 合并一次收尾时事件携带 base..current 跨度，消费方据此判跳变）。
    fn note_change(&mut self, rec: crate::doc_events::ChangeRecord) {
        if self.pending_base.is_none() {
            self.pending_base = Some(self.revision);
        }
        self.pending.push(rec);
    }

    /// 收割待发变更（#111 订阅模式——Editor 编辑收尾单点调用）。
    /// 返回 `(记录列表, 版本基线)`；空记录 = 本次收尾无内容变更
    ///（纯光标移动），不组装事件；基线 = 首条记录入队时的版本号
    ///（无变更时 None）。
    pub fn take_pending(&mut self) -> (Vec<crate::doc_events::ChangeRecord>, Option<u64>) {
        (std::mem::take(&mut self.pending), self.pending_base.take())
    }

    /// 最近一条 undo 记录的子编辑数（G1 查找替换烟雾/日志面：一次
    /// 撤销手势到底还原了几处替换）。栈空 = 0。
    pub fn last_undo_batch_count(&self) -> usize {
        self.bridge.core().last_undo_ops_count()
    }

    /// 在 `start` 插入 `text` 后的终点（undo / redo 的占位末点重算）。
    fn end_after_insert(&self, start: &Point, text: &str) -> Point {
        let newlines = text.matches('\n').count();
        if newlines == 0 {
            Point::new(start.line, start.col + text.chars().count())
        } else {
            let last = text.rsplit('\n').next().unwrap_or("");
            Point::new(start.line + newlines, last.chars().count())
        }
    }

    /// [`Buffer::end_after_insert`] 的模块内公开转发（multi_cursor 的
    /// 多点插入尾部定位消费——同一公式保证与 Buffer 光标语义一致）。
    pub(crate) fn end_after_insert_pub(&self, start: &Point, text: &str) -> Point {
        self.end_after_insert(start, text)
    }

    // ---------------------------------------------------------------------------
    // 光标移动
    // ---------------------------------------------------------------------------

    /// 左移一字符（行首回退到上一行行尾）。`extend`：Shift 选区。
    /// 无 Shift 且有选区：先收点到选区头（一次取消语义）。
    pub fn move_left(&mut self, extend: bool) {
        if !extend && let Some(r) = self.selection() {
            self.set_cursor(r.start);
            return;
        }
        self.begin_move(extend);
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
        } else if self.cursor.line > 0 {
            self.cursor.line -= 1;
            self.cursor.col = self.line_len(self.cursor.line);
        }
        self.desired_col = None;
    }

    /// 右移一字符（行尾进下一行行首）。
    pub fn move_right(&mut self, extend: bool) {
        if !extend && let Some(r) = self.selection() {
            self.set_cursor(r.end);
            return;
        }
        self.begin_move(extend);
        if self.cursor.col < self.line_len(self.cursor.line) {
            self.cursor.col += 1;
        } else if self.cursor.line + 1 < self.line_count() {
            self.cursor.line += 1;
            self.cursor.col = 0;
        }
        self.desired_col = None;
    }

    /// 上移一行（列 clamp + 粘性）。
    pub fn move_up(&mut self, extend: bool) {
        self.begin_move(extend);
        if self.cursor.line > 0 {
            self.cursor.line -= 1;
            self.apply_desired_col();
        }
    }

    /// 下移一行（列 clamp + 粘性）。
    pub fn move_down(&mut self, extend: bool) {
        self.begin_move(extend);
        if self.cursor.line + 1 < self.line_count() {
            self.cursor.line += 1;
            self.apply_desired_col();
        }
    }

    /// 行首（Home）。
    pub fn move_home(&mut self, extend: bool) {
        self.begin_move(extend);
        self.cursor.col = 0;
        self.desired_col = None;
    }

    /// 行尾（End）。
    pub fn move_end(&mut self, extend: bool) {
        self.begin_move(extend);
        self.cursor.col = self.line_len(self.cursor.line);
        self.desired_col = None;
    }

    /// 文首（Ctrl+Home）。
    pub fn move_doc_start(&mut self, extend: bool) {
        self.begin_move(extend);
        self.cursor = Point::new(0, 0);
        self.desired_col = None;
    }

    /// 文尾（Ctrl+End）。
    pub fn move_doc_end(&mut self, extend: bool) {
        self.begin_move(extend);
        let last = self.line_count() - 1;
        self.cursor = Point::new(last, self.line_len(last));
        self.desired_col = None;
    }

    /// 左跳一词（Ctrl+Left）：先跨非词字符，再吃掉词体；行首再退上一行行尾。
    pub fn move_word_left(&mut self, extend: bool) {
        self.begin_move(extend);
        let mut p = self.cursor;
        let text = self.line(p.line);
        let chars: Vec<char> = text.chars().collect();
        while p.col > 0 && !is_word_char(chars[p.col - 1]) {
            p.col -= 1;
        }
        while p.col > 0 && is_word_char(chars[p.col - 1]) {
            p.col -= 1;
        }
        if p.col == 0 && p.line > 0 {
            p.line -= 1;
            p.col = self.line_len(p.line);
        }
        self.cursor = p;
        self.desired_col = None;
    }

    /// 右跳一词（Ctrl+Right）：先跨非词字符，再吃掉词体；行尾再进下一行行首。
    pub fn move_word_right(&mut self, extend: bool) {
        self.begin_move(extend);
        let mut p = self.cursor;
        let text = self.line(p.line);
        let chars: Vec<char> = text.chars().collect();
        let len = chars.len();
        while p.col < len && !is_word_char(chars[p.col]) {
            p.col += 1;
        }
        while p.col < len && is_word_char(chars[p.col]) {
            p.col += 1;
        }
        if p.col == len && p.line + 1 < self.line_count() {
            p.line += 1;
            p.col = 0;
        }
        self.cursor = p;
        self.desired_col = None;
    }

    /// 移动起点纪律：Shift 落锚（无锚时），无 Shift 清锚。
    fn begin_move(&mut self, extend: bool) {
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
        } else {
            self.anchor = None;
        }
    }

    /// 上下行移动后的列 clamp 与粘性：
    /// 期望列 = 粘性记忆 ?? 当前列；超短行则 clamp 并记忆，否则应用并清记忆。
    fn apply_desired_col(&mut self) {
        let len = self.line_len(self.cursor.line);
        let want = self.desired_col.unwrap_or(self.cursor.col);
        if want > len {
            self.desired_col = Some(want);
            self.cursor.col = len;
        } else {
            self.cursor.col = want;
            self.desired_col = None;
        }
    }

    /// 直接设置光标（清锚与粘性；clamp 防御）。
    pub fn set_cursor(&mut self, p: Point) {
        self.cursor = p;
        self.anchor = None;
        self.desired_col = None;
        self.clamp_cursor();
    }

    /// 全选（Ctrl+A）：锚在文首、光标到文尾。
    pub fn select_all(&mut self) {
        let last = self.line_count() - 1;
        self.anchor = Some(Point::new(0, 0));
        self.cursor = Point::new(last, self.line_len(last));
        // #163：select_all 不经 clamp——显式同步锚点。
        self.bridge.sync_selection(self.cursor, self.anchor);
    }

    /// 直接设置选区（锚点 + 光标头；查找跳转等程序化选中用）。
    pub fn set_selection(&mut self, anchor: Point, cursor: Point) {
        self.anchor = Some(anchor);
        self.cursor = cursor;
        self.desired_col = None;
        self.clamp_cursor();
    }

    /// 光标 clamp 到合法域（外部 set 后保证不变量）。
    fn clamp_cursor(&mut self) {
        if self.line_count() == 0 {
            return; // 投影恒非空；防御早退（不可达分支）
        }
        if self.cursor.line >= self.line_count() {
            self.cursor.line = self.line_count() - 1;
        }
        let len = self.line_len(self.cursor.line);
        if self.cursor.col > len {
            self.cursor.col = len;
        }
        // #163（验收 5）：光标/选区 → mdcore Selection（块锚点落核，
        // clamp_cursor 是全部交互变更的单点出口）。
        self.bridge.sync_selection(self.cursor, self.anchor);
    }

    /// G3：行首既有缩进宽度（char 计——前导空格与 Tab 均按 1 char）。
    pub fn leading_indent_len(&self, line: usize) -> usize {
        self.line(line)
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .count()
    }

    // ---------------------------------------------------------------------------
    // 查找（Ctrl+F）
    // ---------------------------------------------------------------------------

    /// 全文大小写不敏感查找：返回所有匹配起点（按文档序；空 query 空）。
    pub fn find_matches(&self, query: &str) -> Vec<Point> {
        self.find_matches_opts(query, false, false)
    }

    /// 带选项全文查找（G1 查找替换）：
    /// - `case_sensitive`：大小写敏感（关闭 = 每字符 lowercase 比对）
    /// - `whole_word`：全词匹配（前后边界不得为词字符——口径与
    ///   [`is_word_char`] 一致：字母数字（含 CJK）+ 下划线）
    /// 返回所有匹配起点（按文档序；含重叠命中——消费方按需过滤，
    /// 替换全部在 find_bar 侧做非重叠筛选）。
    ///
    /// #164 1c：本函数为全文查找计时包装（MANG_PERF 门控——未开零开销）；
    /// 实现在 [`Self::find_matches_opts_impl`]（查找 <1s 红线的归因面）。
    pub fn find_matches_opts(
        &self,
        query: &str,
        case_sensitive: bool,
        whole_word: bool,
    ) -> Vec<Point> {
        let since = crate::perf::on().then(Instant::now);
        let hits = self.find_matches_opts_impl(query, case_sensitive, whole_word);
        if let Some(since) = since {
            crate::perf::find(query.chars().count(), self.line_count(), hits.len(), since);
        }
        hits
    }

    /// [`Self::find_matches_opts`] 的实现体（原实现原样下沉——语义零变更）。
    fn find_matches_opts_impl(
        &self,
        query: &str,
        case_sensitive: bool,
        whole_word: bool,
    ) -> Vec<Point> {
        if query.is_empty() {
            return Vec::new();
        }
        let needle: Vec<char> = if case_sensitive {
            query.chars().collect()
        } else {
            query.to_lowercase().chars().collect()
        };
        let mut hits = Vec::new();
        for li in 0..self.line_count() {
            let line = self.line(li);
            let hay: Vec<char> = if case_sensitive {
                line.chars().collect()
            } else {
                line.chars()
                    .map(|c| c.to_lowercase().next().unwrap_or(c))
                    .collect()
            };
            if hay.len() < needle.len() {
                continue;
            }
            for start in 0..=hay.len() - needle.len() {
                if hay[start..start + needle.len()] != needle[..] {
                    continue;
                }
                if whole_word && !self.word_bounded(&hay, start, needle.len()) {
                    continue;
                }
                hits.push(Point::new(li, start));
            }
        }
        hits
    }

    /// 全词边界判定：`[start, start+len)` 前后均不得为词字符。
    fn word_bounded(&self, hay: &[char], start: usize, len: usize) -> bool {
        let before_ok = start == 0 || !is_word_char(hay[start - 1]);
        let after = start + len;
        let after_ok = after >= hay.len() || !is_word_char(hay[after]);
        before_ok && after_ok
    }

    // ---------------------------------------------------------------------------
    // 内部工具（rope 坐标换算层——外壳 char 语义 <-> rope byte 列）
    // ---------------------------------------------------------------------------

    /// 两点排序（start <= end）。
    fn ordered_range(&self, r: Range<Point>) -> Range<Point> {
        if r.start <= r.end { r } else { r.end..r.start }
    }
}

// =============================================================================
// 单元测试 —— 纯逻辑全绿是本模块的交付门
// （r39 的 41 个测试逐字迁移（公开 API 语义即契约）；r42 追加坐标层/换芯专项）
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 快捷构造点。
    fn p(line: usize, col: usize) -> Point {
        Point::new(line, col)
    }

    // ---------------- 构造与序列化 ----------------

    #[test]
    fn plain_text_roundtrip() {
        let cases = ["", "hello", "a\nb\nc", "中文\nEnglish\n混合 line", "\n\n"];
        for t in cases {
            let b = Buffer::from_plain_text(t);
            assert_eq!(b.to_plain_text(), t, "roundtrip: {t:?}");
        }
    }

    #[test]
    fn crlf_roundtrip_bit_fidelity() {
        // r42 专项：\r\n 属行内容原样存取（不走 Buffer::new 的 normalize）
        let b = Buffer::from_plain_text("a\r\nb\rc");
        assert_eq!(b.to_plain_text(), "a\r\nb\rc");
        assert_eq!(b.line_count(), 2, "\\n 划行（1 个 \\n）：\\r 留行内");
        assert_eq!(b.line(0), "a\r");
        assert_eq!(b.line(1), "b\rc");
    }

    #[test]
    fn empty_is_single_empty_line() {
        let b = Buffer::from_plain_text("");
        assert_eq!(b.line_count(), 1);
        assert_eq!(b.line(0), "");
    }

    #[test]
    fn trailing_newline_keeps_empty_last_line() {
        // r42 专项：末行 \n → 尾空行保位（旧 split 语义 = rope max_point 语义）
        let b = Buffer::from_plain_text("a\n");
        assert_eq!(b.line_count(), 2);
        assert_eq!(b.line(1), "");
        assert_eq!(b.to_plain_text(), "a\n");
    }

    // ---------------- 光标移动矩阵 ----------------

    #[test]
    fn move_left_wraps_to_prev_line_tail() {
        let mut b = Buffer::from_plain_text("ab\ncd");
        b.set_cursor(p(1, 0));
        b.move_left(false);
        assert_eq!(b.cursor(), p(0, 2), "第二行行首左移 → 第一行行尾");
        b.move_left(false);
        b.move_left(false);
        assert_eq!(b.cursor(), p(0, 0));
        b.move_left(false);
        assert_eq!(b.cursor(), p(0, 0), "文首左移不动");
    }

    #[test]
    fn move_right_wraps_to_next_line_head() {
        let mut b = Buffer::from_plain_text("ab\ncd");
        b.set_cursor(p(0, 2));
        b.move_right(false);
        assert_eq!(b.cursor(), p(1, 0), "第一行行尾右移 → 第二行行首");
        b.move_right(false);
        b.move_right(false);
        assert_eq!(b.cursor(), p(1, 2));
        b.move_right(false);
        assert_eq!(b.cursor(), p(1, 2), "文尾右移不动");
    }

    #[test]
    fn move_up_down_clamps_and_sticks() {
        let mut b = Buffer::from_plain_text("abcdef\nxy\nabcdefgh");
        // 从第 2 行 col 6 上移到短行 "xy"（len 2）→ clamp 2，记忆 6
        b.set_cursor(p(2, 6));
        b.move_up(false);
        assert_eq!(b.cursor(), p(1, 2), "短行 clamp");
        // 再上移到长行 → 恢复记忆列 6
        b.move_up(false);
        assert_eq!(b.cursor(), p(0, 6), "粘性列恢复");
    }

    #[test]
    fn home_end_doc_bounds() {
        let mut b = Buffer::from_plain_text("ab\ncd\n ef");
        b.set_cursor(p(1, 1));
        b.move_home(false);
        assert_eq!(b.cursor(), p(1, 0));
        b.move_end(false);
        assert_eq!(b.cursor(), p(1, 2));
        b.move_doc_start(false);
        assert_eq!(b.cursor(), p(0, 0));
        b.move_doc_end(false);
        assert_eq!(b.cursor(), p(2, 3));
    }

    #[test]
    fn cjk_columns_are_char_indexed() {
        let mut b = Buffer::from_plain_text("中文编辑");
        assert_eq!(b.line_len(0), 4, "四个汉字 = 4 列");
        b.set_cursor(p(0, 2));
        b.move_right(false);
        assert_eq!(b.cursor(), p(0, 3));
        b.insert("x");
        assert_eq!(b.line(0), "中文编x辑");
    }

    // ---------------- 编辑原语 ----------------

    #[test]
    fn insert_mid_line() {
        let mut b = Buffer::from_plain_text("ac");
        b.set_cursor(p(0, 1));
        b.insert("b");
        assert_eq!(b.to_plain_text(), "abc");
        assert_eq!(b.cursor(), p(0, 2));
    }

    #[test]
    fn insert_multiline_splits() {
        let mut b = Buffer::from_plain_text("abc");
        b.set_cursor(p(0, 1));
        b.insert("X\nY\nZ");
        assert_eq!(b.to_plain_text(), "aX\nY\nZbc");
        assert_eq!(b.cursor(), p(2, 1));
        assert_eq!(b.line_count(), 3);
    }

    #[test]
    fn newline_splits_line() {
        let mut b = Buffer::from_plain_text("ab");
        b.set_cursor(p(0, 1));
        b.newline();
        assert_eq!(b.to_plain_text(), "a\nb");
        assert_eq!(b.cursor(), p(1, 0));
    }

    #[test]
    fn backspace_merges_lines() {
        let mut b = Buffer::from_plain_text("ab\ncd");
        b.set_cursor(p(1, 0));
        b.backspace();
        assert_eq!(b.to_plain_text(), "abcd");
        assert_eq!(b.cursor(), p(0, 2));
    }

    #[test]
    fn backspace_deletes_char() {
        let mut b = Buffer::from_plain_text("abc");
        b.set_cursor(p(0, 2));
        b.backspace();
        assert_eq!(b.to_plain_text(), "ac");
    }

    #[test]
    fn backspace_cjk_single_column() {
        let mut b = Buffer::from_plain_text("中文字");
        b.set_cursor(p(0, 1));
        b.backspace();
        assert_eq!(b.to_plain_text(), "文字", "一个退格删一个汉字");
    }

    #[test]
    fn delete_forward_merges_lines() {
        let mut b = Buffer::from_plain_text("ab\ncd");
        b.set_cursor(p(0, 2));
        b.delete_forward();
        assert_eq!(b.to_plain_text(), "abcd");
        assert_eq!(b.cursor(), p(0, 2));
    }

    #[test]
    fn delete_forward_deletes_char() {
        let mut b = Buffer::from_plain_text("abc");
        b.set_cursor(p(0, 0));
        b.delete_forward();
        assert_eq!(b.to_plain_text(), "bc");
    }

    #[test]
    fn tab_inserts_four_spaces() {
        let mut b = Buffer::from_plain_text("");
        b.tab();
        assert_eq!(b.to_plain_text(), "    ");
    }

    #[test]
    fn replace_range_cross_line_middle() {
        let mut b = Buffer::from_plain_text("aa\nbb\ncc\ndd");
        // 删 (0,1)..(3,1)（"a\nbb\ncc\nd"）换 "X\nY"
        b.replace_range(p(0, 1), p(3, 1), "X\nY");
        assert_eq!(b.to_plain_text(), "aX\nYd");
        assert_eq!(b.cursor(), p(1, 1));
    }

    #[test]
    fn replace_range_out_of_bounds_is_clamped() {
        // r42 专项：越界点防御性 clamp（旧实现 panic 面，语义超集）。
        // "ab\ncd"：p(0,99)→(0,2)，p(99,99)→(1,2)；即 (0,2)..(1,2)
        // 的 "\ncd" 被换成 "X" → "abX"。
        let mut b = Buffer::from_plain_text("ab\ncd");
        b.replace_range(p(0, 99), p(99, 99), "X");
        assert_eq!(b.to_plain_text(), "abX");
        assert_eq!(b.cursor(), p(0, 3), "插入末尾点（start=(0,2) + 1 char）");
    }

    // ---------------- 选区 ----------------

    #[test]
    fn shift_move_creates_selection() {
        let mut b = Buffer::from_plain_text("hello world");
        b.set_cursor(p(0, 6));
        b.move_right(true);
        b.move_right(true);
        let sel = b.selection().expect("有选区");
        assert_eq!((sel.start, sel.end), (p(0, 6), p(0, 8)));
        assert_eq!(b.selection_text(), "wo");
        // 无 Shift 移动 → 收点到选区头
        b.move_left(false);
        assert_eq!(b.cursor(), p(0, 6));
        assert!(b.selection().is_none());
    }

    #[test]
    fn selection_replaced_by_insert() {
        let mut b = Buffer::from_plain_text("hello world");
        b.set_cursor(p(0, 6));
        for _ in 0..5 {
            b.move_right(true);
        }
        b.insert("GPUI");
        assert_eq!(b.to_plain_text(), "hello GPUI");
        assert!(b.selection().is_none(), "编辑后选区清空");
    }

    #[test]
    fn selection_spanning_lines() {
        let mut b = Buffer::from_plain_text("ab\ncd\nef");
        b.set_cursor(p(0, 1));
        b.move_down(true);
        b.move_down(true);
        let sel = b.selection().unwrap();
        assert_eq!(b.range_text(&sel), "b\ncd\ne");
    }

    #[test]
    fn select_all_and_delete() {
        let mut b = Buffer::from_plain_text("a\nb\nc");
        b.select_all();
        assert_eq!(b.selection_text(), "a\nb\nc");
        b.delete_selection();
        assert_eq!(b.to_plain_text(), "");
        assert_eq!(b.line_count(), 1, "删空后仍保单空行");
    }

    // ---------------- undo / redo ----------------

    #[test]
    fn undo_redo_roundtrip_simple() {
        let mut b = Buffer::from_plain_text("abc");
        b.set_cursor(p(0, 3));
        b.insert("def");
        assert_eq!(b.to_plain_text(), "abcdef");
        assert!(b.is_modified());
        assert!(b.undo());
        assert_eq!(b.to_plain_text(), "abc");
        assert!(!b.is_modified(), "undo 到底 = 回初始，修改标记清");
        assert!(b.redo());
        assert_eq!(b.to_plain_text(), "abcdef");
        assert!(b.is_modified(), "redo 后修改标记复立");
    }

    #[test]
    fn undo_restores_cursor_and_multiline() {
        let mut b = Buffer::from_plain_text("one two");
        b.set_cursor(p(0, 3)); // col3 = 空格：拆行后第二行 " two"
        b.newline();
        b.insert("three");
        assert_eq!(b.to_plain_text(), "one\nthree two");
        assert!(b.undo());
        assert_eq!(b.to_plain_text(), "one\n two");
        assert_eq!(b.cursor(), p(1, 0), "undo 后光标回该次编辑起点");
        assert!(b.undo());
        assert_eq!(b.to_plain_text(), "one two");
        assert_eq!(b.cursor(), p(0, 3), "第二次 undo 光标回首个编辑起点");
        assert!(!b.undo(), "栈空再 undo 返回 false");
        assert!(b.redo());
        assert_eq!(b.to_plain_text(), "one\n two");
        assert_eq!(b.cursor(), p(1, 0), "redo 光标在插入末尾");
    }

    #[test]
    fn undo_backspace_merge() {
        let mut b = Buffer::from_plain_text("ab\ncd");
        b.set_cursor(p(1, 0));
        b.backspace();
        assert_eq!(b.to_plain_text(), "abcd");
        assert!(b.undo());
        assert_eq!(b.to_plain_text(), "ab\ncd");
        assert_eq!(b.cursor(), p(0, 2), "undo 合并 → 光标回上行行尾");
    }

    #[test]
    fn undo_replacement_not_just_insert() {
        let mut b = Buffer::from_plain_text("hello world");
        b.set_cursor(p(0, 6));
        for _ in 0..5 {
            b.move_right(true);
        }
        b.insert("GPUI");
        assert_eq!(b.to_plain_text(), "hello GPUI");
        assert!(b.undo());
        assert_eq!(b.to_plain_text(), "hello world", "undo 恢复被替换的 world");
        assert!(b.redo());
        assert_eq!(b.to_plain_text(), "hello GPUI");
    }

    #[test]
    fn undo_cross_line_replace() {
        let mut b = Buffer::from_plain_text("aa\nbb\ncc\ndd");
        b.replace_range(p(0, 1), p(3, 1), "X\nY");
        assert_eq!(b.to_plain_text(), "aX\nYd");
        assert!(b.undo());
        assert_eq!(b.to_plain_text(), "aa\nbb\ncc\ndd", "跨行替换完整可逆");
        assert!(b.redo());
        assert_eq!(b.to_plain_text(), "aX\nYd");
    }

    #[test]
    fn edit_after_undo_clears_redo() {
        let mut b = Buffer::from_plain_text("a");
        b.insert("b");
        assert!(b.undo());
        b.insert("c");
        assert!(!b.redo(), "新编辑后 redo 栈清空");
        assert_eq!(
            b.to_plain_text(),
            "ca",
            "undo 光标回编辑起点 (0,0)，新字符插在行首"
        );
    }

    fn assert_gesture_semantic_matches_source(b: &Buffer) {
        let source = mdcore::markdown::markdown_to_tree(&b.to_plain_text());
        assert_eq!(
            mdcore::markdown::tree_to_markdown(&b.bridge.core().tree),
            mdcore::markdown::tree_to_markdown(&source)
        );
    }

    #[test]
    fn gesture_batch_preserves_canonical_ids_and_semantic_history() {
        let mut b = Buffer::from_plain_text("# heading\r\n\r\nbody\r\n");
        let ids: Vec<_> = b
            .bridge
            .core()
            .tree
            .top_level()
            .iter()
            .map(|n| b.bridge.core().tree.node(*n).id)
            .collect();
        b.replace_batch(vec![
            (Point::new(0, 3), Point::new(0, 3), "中😀".into()),
            (Point::new(2, 1), Point::new(2, 1), "X".into()),
        ]);
        let after = b.to_plain_text();
        for stage in 0..3 {
            assert_gesture_semantic_matches_source(&b);
            let current_ids: Vec<_> = b
                .bridge
                .core()
                .tree
                .top_level()
                .iter()
                .map(|n| b.bridge.core().tree.node(*n).id)
                .collect();
            assert_eq!(current_ids, ids);
            if stage == 0 {
                assert!(b.undo());
                assert_eq!(b.to_plain_text(), "# heading\r\n\r\nbody\r\n");
            }
            if stage == 1 {
                assert!(b.redo());
                assert_eq!(b.to_plain_text(), after);
            }
        }
    }

    #[test]
    fn gesture_source_only_then_typing_keeps_ledger_and_saved_identity() {
        let mut b = Buffer::from_plain_text("ab\n\n");
        b.replace_range(Point::new(0, 1), Point::new(0, 1), "X");
        b.replace_batch(vec![(Point::new(2, 0), Point::new(2, 0), "\n\n".into())]);
        let saved = b.to_plain_text();
        b.mark_saved();
        b.replace_range(Point::new(0, 0), Point::new(0, 0), "Q");
        b.replace_range(Point::new(0, 1), Point::new(0, 1), "R");
        assert!(b.is_modified());
        assert!(b.undo());
        assert_eq!(b.to_plain_text(), saved, "ordinary typing still coalesces");
        assert!(!b.is_modified());
        assert!(b.undo());
        assert_eq!(b.to_plain_text(), "aXb\n\n");
        assert!(b.is_modified());
        assert!(b.redo());
        assert_eq!(b.to_plain_text(), saved);
        assert!(!b.is_modified());
        assert_gesture_semantic_matches_source(&b);
    }

    #[test]
    fn gesture_empty_and_overlap_batches_do_not_change_state() {
        let mut b = Buffer::from_plain_text("ab\n\n");
        assert_eq!(
            b.replace_batch(vec![(Point::new(2, 0), Point::new(2, 0), String::new())]),
            0
        );
        assert_eq!(
            b.replace_batch(vec![
                (Point::new(0, 0), Point::new(0, 2), "X".into()),
                (Point::new(0, 1), Point::new(0, 2), "Y".into()),
            ]),
            0
        );
        assert_eq!(b.to_plain_text(), "ab\n\n");
        assert_eq!(b.revision(), 0);
        assert!(!b.is_modified());
        assert!(b.take_pending().0.is_empty());
        assert!(!b.undo());
    }

    #[test]
    fn gesture_clock_crosses_one_second_without_splitting() {
        use super::super::multi_cursor::Cursors;
        use std::sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        };
        let mut b = Buffer::from_plain_text("abcd");
        let clock = Arc::new(AtomicU64::new(0));
        let ticks = clock.clone();
        b.bridge
            .set_test_clock(Arc::new(move || ticks.fetch_add(1500, Ordering::SeqCst)));
        let mut c = Cursors::single(Point::new(0, 1));
        c.push(Point::new(0, 3), Point::new(0, 3));
        c.insert_all(&mut b, "X");
        assert!(b.undo());
        assert_eq!(b.to_plain_text(), "abcd");
        assert!(!b.undo());
        assert!(b.redo());
        assert_eq!(b.to_plain_text(), "aXbcXd");
    }

    #[test]
    fn gesture_late_hook_rejection_is_atomic_for_document_and_cursors() {
        use super::super::multi_cursor::Cursors;
        struct RejectFirstLine;
        impl EditHook for RejectFirstLine {
            fn before_edit(&mut self, intent: &EditIntent) -> Result<(), String> {
                if intent.start.line == 0 {
                    Err("locked".into())
                } else {
                    Ok(())
                }
            }
            fn after_edit(&mut self, _: &EditSummary) {
                panic!("rejected batch cannot notify");
            }
        }
        let mut b = Buffer::from_plain_text("ab\ncd");
        b.set_edit_hook(Some(Box::new(RejectFirstLine)));
        let mut c = Cursors::single(Point::new(0, 1));
        c.push(Point::new(1, 1), Point::new(1, 1));
        let before = c.clone();
        c.insert_all(&mut b, "X");
        assert_eq!(b.to_plain_text(), "ab\ncd");
        assert_eq!(c, before);
        assert_eq!(b.revision(), 0);
        assert!(!b.is_modified());
        assert!(b.take_pending().0.is_empty());
        assert_eq!(b.take_fence_dirty(), FenceDirty::Clean);
        assert!(!b.undo());
    }

    #[test]
    fn gesture_source_only_batch_does_not_consume_semantic_history() {
        let mut b = Buffer::from_plain_text("ab\r\n\r\n");
        b.set_coalesce_ms(0);
        b.replace_range(Point::new(0, 1), Point::new(0, 1), "X");
        let semantic = b.to_plain_text();
        assert_eq!(
            b.replace_batch(vec![
                (Point::new(1, 0), Point::new(1, 0), "\n".into()),
                (Point::new(2, 0), Point::new(2, 0), "\n".into()),
            ]),
            2
        );
        let after = b.to_plain_text();
        assert_eq!(after, "aXb\r\n\n\r\n\n");
        assert!(b.undo());
        assert_eq!(b.to_plain_text(), semantic);
        assert!(b.undo());
        assert_eq!(b.to_plain_text(), "ab\r\n\r\n");
        assert!(b.redo());
        assert_eq!(b.to_plain_text(), semantic);
        assert!(b.redo());
        assert_eq!(b.to_plain_text(), after);
    }

    #[test]
    fn gesture_mixed_batch_exact_crlf_and_eof_roundtrip() {
        for original in ["ab\r\n\r\ncd", "ab\r\n\r\ncd\r\n", "\n\n"] {
            let mut b = Buffer::from_plain_text(original);
            b.set_coalesce_ms(0);
            let last = b.line_count() - 1;
            let eof = Point::new(last, b.line_len(last));
            b.replace_batch(vec![
                (Point::new(0, 0), Point::new(0, 0), "X".into()),
                (eof, eof, "\n\n".into()),
            ]);
            let after = b.to_plain_text();
            assert!(b.undo(), "{original:?}");
            assert_eq!(b.to_plain_text(), original);
            assert_gesture_semantic_matches_source(&b);
            assert!(b.redo());
            assert_eq!(b.to_plain_text(), after);
            assert_gesture_semantic_matches_source(&b);
        }
    }

    #[test]
    fn gesture_saved_identity_branch_same_depth_and_length() {
        let mut b = Buffer::from_plain_text("ab");
        b.set_coalesce_ms(0);
        b.replace_batch(vec![(Point::new(0, 1), Point::new(0, 1), "X".into())]);
        b.mark_saved();
        assert!(!b.is_modified());
        assert!(b.undo());
        assert!(b.is_modified());
        b.replace_batch(vec![(Point::new(0, 1), Point::new(0, 1), "Y".into())]);
        assert_eq!(b.to_plain_text(), "aYb");
        assert!(b.is_modified(), "equal depth/length is not the saved state");
        assert!(!b.redo(), "new branch invalidates redo");
    }

    #[test]
    fn mark_saved_resets_modified_flag() {
        let mut b = Buffer::from_plain_text("a");
        b.insert("b");
        assert!(b.is_modified());
        b.mark_saved();
        assert!(!b.is_modified(), "落盘后修改标记清零");
    }

    // ---------------- 词跳 ----------------

    #[test]
    fn word_jump_ascii() {
        let mut b = Buffer::from_plain_text("foo bar_baz  qux");
        b.set_cursor(p(0, 15)); // "qux" 词内（x 上）
        b.move_word_left(false);
        assert_eq!(b.cursor(), p(0, 13), "跳到 qux 词首");
        b.move_word_left(false);
        assert_eq!(b.cursor(), p(0, 4), "跨过空格跳到 bar_baz 词首");
        b.move_word_left(false);
        assert_eq!(b.cursor(), p(0, 0), "跳到 foo 词首");
        b.move_word_right(false);
        assert_eq!(b.cursor(), p(0, 3), "跳到 foo 词尾（空格前）");
        b.move_word_right(false);
        assert_eq!(b.cursor(), p(0, 11), "跳过空格吃掉 bar_baz 到词尾");
        b.move_word_right(false);
        assert_eq!(b.cursor(), p(0, 16), "跨双空格吃掉 qux 词体到行尾");
    }

    #[test]
    fn word_jump_cjk_treated_as_word() {
        let mut b = Buffer::from_plain_text("中文 混合text");
        b.set_cursor(p(0, 9)); // 行尾
        b.move_word_left(false);
        assert_eq!(b.cursor(), p(0, 3), "混合text 连续词字符整段一词");
        b.move_word_left(false);
        assert_eq!(b.cursor(), p(0, 0), "跨空格跳到 中文 词首");
    }

    #[test]
    fn word_jump_crosses_lines() {
        let mut b = Buffer::from_plain_text("ab\ncd");
        b.set_cursor(p(1, 0));
        b.move_word_left(false);
        assert_eq!(b.cursor(), p(0, 2), "下行行首词跳 → 上行行尾");
    }

    // ---------------- 查找 ----------------

    #[test]
    fn find_all_matches_case_insensitive() {
        let b = Buffer::from_plain_text("Foo bar\nfoo baz\nFOO");
        let hits = b.find_matches("foo");
        assert_eq!(hits, vec![p(0, 0), p(1, 0), p(2, 0)]);
    }

    #[test]
    fn find_multiple_hits_in_line_and_cjk() {
        let b = Buffer::from_plain_text("xaxa 中文中");
        let hits = b.find_matches("a");
        assert_eq!(hits, vec![p(0, 1), p(0, 3)]);
        let hits = b.find_matches("文中");
        assert_eq!(hits, vec![p(0, 6)], "CJK 子串按 char 定位");
    }

    #[test]
    fn find_empty_query_no_hits() {
        let b = Buffer::from_plain_text("abc");
        assert!(b.find_matches("").is_empty());
    }

    // ---------------- 边界防御 ----------------

    #[test]
    fn out_of_bounds_access_is_safe() {
        let b = Buffer::from_plain_text("a");
        assert_eq!(b.line(99), "");
        assert_eq!(b.line_len(99), 0);
    }

    #[test]
    fn char_count_counts_chars_and_newlines() {
        let b = Buffer::from_plain_text("中文ab\nx");
        // 5 可见字符 + 1 换行 = 6。
        assert_eq!(b.char_count(), 6);
    }

    // ---------------- r42 换芯专项（坐标层 / Clone / 大文档） ----------------

    #[test]
    fn clone_is_independent_deep_copy() {
        let mut b = Buffer::from_plain_text("ab\ncd");
        b.set_cursor(p(1, 1));
        b.insert("X");
        let c = b.clone();
        assert_eq!(c.to_plain_text(), "ab\ncXd");
        // 克隆体独立编辑不回写源
        let mut c2 = c;
        c2.insert("Y");
        assert_eq!(c2.to_plain_text(), "ab\ncXYd");
        assert_eq!(b.to_plain_text(), "ab\ncXd", "克隆体编辑不影响源");
        // undo 历史随行（四元组外壳栈深拷贝）
        assert!(b.undo());
        assert_eq!(b.to_plain_text(), "ab\ncd");
    }

    #[test]
    fn debug_impl_is_compact_summary() {
        let b = Buffer::from_plain_text("中文\nx");
        let s = format!("{b:?}");
        assert!(s.contains("lines: 2"), "Debug 含行数：{s}");
        assert!(s.contains("chars: 4"), "Debug 含字符数：{s}");
    }

    #[test]
    fn large_document_edit_and_navigate() {
        // r42 换芯收益冒烟：10 万行 / 50 万字级文档的随机点编辑 + 深点导航
        // （旧 Vec<String> 实现的逐行 drain/insert 在此规模显著退化）
        let lines: Vec<String> = (0..100_000)
            .map(|i| format!("第{i}行内容padding"))
            .collect();
        let text = lines.join("\n");
        let mut b = Buffer::from_plain_text(&text);
        assert_eq!(b.line_count(), 100_000);
        // 深点导航（末行行尾）
        b.set_cursor(p(99_999, b.line_len(99_999)));
        // 中段随机替换（跨 3 行）
        b.replace_range(p(50_000, 0), p(50_002, 2), "换芯");
        assert!(b.line(50_000).starts_with("换芯"));
        assert!(b.line(50_000).ends_with("padding"), "前段保留");
        // 全文序列化完整
        let round = b.to_plain_text();
        assert!(round.contains("第0行内容padding"));
        assert!(round.contains("第99999行内容padding"));
        // undo 完整可逆
        assert!(b.undo());
        assert_eq!(b.to_plain_text(), text, "undo 恢复 50 万字原文");
    }
}

// =============================================================================
// r42 性能实测（--ignored 面板）：cargo test -- --ignored
#[test]
#[ignore = "r42 性能实测（#68 阶段 A 验收证据）"]
fn perf_rope_edit_find_2m_chars() {
    use std::time::Instant;
    // 10 万行 × ~20 字/行 ≈ 200 万字符（4 倍于 50 万字红线规格）
    let text: String = {
        let lines: Vec<String> = (0..100_000)
            .map(|i| format!("第{i}行内容padding"))
            .collect();
        lines.join("\n")
    };
    let expect_chars = text.chars().count();
    assert!(
        expect_chars > 1_500_000,
        "压力规格 ≥150 万字符，实际 {expect_chars}"
    );

    // ① 构造（红线 <2s——4 倍规格下仍须达标）
    let t0 = Instant::now();
    let mut b = Buffer::from_plain_text(&text);
    let t_build = t0.elapsed();
    // ② 输入延迟（红线 p99 <50ms）：随机点 1000 次 insert
    let mut rng_state: u64 = 0x9E3779B97F4A7C15;
    let mut rng = || {
        rng_state ^= rng_state << 13;
        rng_state ^= rng_state >> 7;
        rng_state ^= rng_state << 17;
        rng_state
    };
    let mut samples: Vec<std::time::Duration> = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let line = (rng() % b.line_count() as u64) as usize;
        let col = (rng() % (b.line_len(line) as u64 + 1)) as usize;
        b.set_cursor(Point::new(line, col));
        let t = Instant::now();
        b.insert("芯");
        samples.push(t.elapsed());
    }
    samples.sort();
    let p50 = samples[500];
    let p99 = samples[990];
    let t_edit_max = samples[999];
    // ③ undo 路径（核逆 op + 投影回放，红线 <50ms/次）× 200
    let mut undo_samples: Vec<std::time::Duration> = Vec::with_capacity(200);
    for _ in 0..200 {
        let t = Instant::now();
        assert!(b.undo());
        undo_samples.push(t.elapsed());
    }
    undo_samples.sort();
    let undo_p99 = undo_samples[198];
    // ④ 全文查找（红线 <1s）
    let t = Instant::now();
    let hits = b.find_matches("行内容");
    let t_find = t.elapsed();
    assert!(!hits.is_empty());
    // ⑤ 深点导航（末行行尾 line_len + set_cursor）
    let t = Instant::now();
    let last = b.line_count() - 1;
    let len = b.line_len(last);
    b.set_cursor(Point::new(last, len));
    let t_nav = t.elapsed();
    // ⑥ 序列化
    let t = Instant::now();
    let round = b.to_plain_text();
    let t_ser = t.elapsed();
    drop(round);
    println!("perf(r42 换芯, {expect_chars} chars):");
    println!("  构造      = {:>10.1?}  (红线 <2s)", t_build);
    println!("  insert    = p50 {p50:?} / p99 {p99:?} / max {t_edit_max:?}  (红线 p99 <50ms)");
    println!("  undo      = p99 {undo_p99:?}  (红线 <50ms)");
    println!(
        "  find      = {:>10.1?}  hits={} (红线 <1s)",
        t_find,
        hits.len()
    );
    println!("  深点导航  = {:>10.1?}", t_nav);
    println!("  序列化    = {:>10.1?}", t_ser);
    assert!(
        t_build < std::time::Duration::from_secs(2),
        "构造超红线：{t_build:?}"
    );
    assert!(
        p99 < std::time::Duration::from_millis(50),
        "insert p99 超红线：{p99:?}"
    );
    assert!(
        undo_p99 < std::time::Duration::from_millis(50),
        "undo p99 超红线：{undo_p99:?}"
    );
    assert!(
        t_find < std::time::Duration::from_secs(1),
        "查找超红线：{t_find:?}"
    );
}

// ---------------- #205：fence 脏行记账 ----------------
#[cfg(test)]
mod fence_dirty_tests {
    use super::*;

    fn p(line: usize, col: usize) -> Point {
        Point::new(line, col)
    }

    /// #205 测试钩子：全拒（拒改路径零变更零记账的判定面）。
    struct FenceRejectAllHook;
    impl EditHook for FenceRejectAllHook {
        fn before_edit(&mut self, _: &EditIntent) -> Result<(), String> {
            Err("测试全拒".into())
        }
        fn after_edit(&mut self, _: &EditSummary) {}
    }

    #[test]
    fn fence_dirty_single_recorded_by_replace_range() {
        // 单区间编辑 → Single 窗口（line/inserted_lines/pre_lines）。
        let mut b = Buffer::from_plain_text("a\nb\nc");
        assert!(b.replace_range(p(1, 0), p(1, 0), "x\ny\nz"));
        assert_eq!(
            b.take_fence_dirty(),
            FenceDirty::Single {
                line: 1,
                inserted_lines: 2,
                pre_lines: 3,
            }
        );
        // 收割后复位 Clean。
        assert_eq!(b.take_fence_dirty(), FenceDirty::Clean);
    }

    #[test]
    fn fence_dirty_rejected_edit_not_recorded() {
        // 钩子拒改 → 零变更零记账。
        let mut b = Buffer::from_plain_text("ab\ncd");
        b.set_edit_hook(Some(Box::new(FenceRejectAllHook)));
        assert!(!b.replace_range(p(0, 0), p(0, 1), "x"));
        assert_eq!(b.take_fence_dirty(), FenceDirty::Clean);
    }

    #[test]
    fn fence_dirty_consecutive_edits_upgrade_to_full() {
        // 连续两次单区间编辑（无收割间隔——多光标逐光标路径）→ Full。
        let mut b = Buffer::from_plain_text("a\nb");
        assert!(b.replace_range(p(0, 0), p(0, 0), "x"));
        assert!(b.replace_range(p(1, 0), p(1, 0), "y"));
        assert_eq!(b.take_fence_dirty(), FenceDirty::Full);
        // 收割后的新一轮重新从 Single 起步。
        assert!(b.replace_range(p(0, 0), p(0, 0), "z"));
        assert_eq!(
            b.take_fence_dirty(),
            FenceDirty::Single {
                line: 0,
                inserted_lines: 0,
                pre_lines: 2,
            }
        );
    }

    #[test]
    fn fence_dirty_noop_edit_not_recorded() {
        // 空域 + 空文本 = no-op 短路（不落 undo、不计版本、不记账）。
        let mut b = Buffer::from_plain_text("ab");
        assert!(!b.replace_range(p(0, 1), p(0, 1), ""));
        assert_eq!(b.take_fence_dirty(), FenceDirty::Clean);
    }

    #[test]
    fn fence_dirty_batch_recorded_as_full() {
        let mut b = Buffer::from_plain_text("ab\ncd\nef");
        let n = b.replace_batch(vec![
            (p(0, 0), p(0, 1), "x".into()),
            (p(2, 0), p(2, 1), "y".into()),
        ]);
        assert_eq!(n, 2);
        assert_eq!(b.take_fence_dirty(), FenceDirty::Full);
    }

    #[test]
    fn fence_dirty_undo_redo_single_recorded() {
        // 单点 undo/redo（物理回放）也是单区间 → Single（收割后）。
        let mut b = Buffer::from_plain_text("a\nb");
        assert!(b.replace_range(p(1, 0), p(1, 0), "x\ny"));
        assert_eq!(
            b.take_fence_dirty(),
            FenceDirty::Single {
                line: 1,
                inserted_lines: 1,
                pre_lines: 2,
            }
        );
        assert!(b.undo());
        assert_eq!(
            b.take_fence_dirty(),
            FenceDirty::Single {
                line: 1,
                inserted_lines: 0, // undo 的「插入文本」= old_text（""）
                pre_lines: 3,
            }
        );
        assert!(b.redo());
        assert_eq!(
            b.take_fence_dirty(),
            FenceDirty::Single {
                line: 1,
                inserted_lines: 1, // redo 的「插入文本」= new_text（"x\ny"）
                pre_lines: 2,
            }
        );
    }

    #[test]
    fn fence_dirty_undo_batch_recorded_as_full() {
        // 批量撤销回放（多区间）→ Full。
        let mut b = Buffer::from_plain_text("ab\ncd\nef");
        let n = b.replace_batch(vec![
            (p(0, 0), p(0, 1), "x".into()),
            (p(2, 0), p(2, 1), "y".into()),
        ]);
        assert_eq!(n, 2);
        assert_eq!(b.take_fence_dirty(), FenceDirty::Full);
        assert!(b.undo());
        assert_eq!(b.take_fence_dirty(), FenceDirty::Full);
        assert!(b.redo());
        assert_eq!(b.take_fence_dirty(), FenceDirty::Full);
    }

    #[test]
    fn fence_dirty_clone_starts_clean() {
        // 克隆体不是变更源——记账从 Clean 起步（消费方首收全量，恒安全）。
        let mut b = Buffer::from_plain_text("a\nb");
        assert!(b.replace_range(p(0, 0), p(0, 0), "x"));
        let mut snapshot = b.clone();
        assert_eq!(snapshot.take_fence_dirty(), FenceDirty::Clean);
        // 源窗口不受克隆影响。
        assert_eq!(
            b.take_fence_dirty(),
            FenceDirty::Single {
                line: 0,
                inserted_lines: 0,
                pre_lines: 2,
            }
        );
    }
}

// =============================================================================
// G1 查找替换：带选项查找 + 批量编辑原语（一次撤销整批）——专项单测
// =============================================================================

#[cfg(test)]
mod g1_find_replace_tests {

    #[test]
    fn batch_replace_undo_restores_exact_text() {
        use super::*;
        let mut b = Buffer::from_plain_text("foo one\nfoo two");
        let edits = vec![
            (Point::new(0, 0), Point::new(0, 3), "bar".to_string()),
            (Point::new(1, 0), Point::new(1, 3), "bar".to_string()),
        ];
        assert_eq!(b.replace_batch(edits), 2);
        assert_eq!(b.to_plain_text(), "bar one\nbar two");
        assert!(b.undo());
        println!("undo后: {:?}", b.to_plain_text());
        assert_eq!(b.to_plain_text(), "foo one\nfoo two", "undo 精确回原");
    }
    use super::*;

    fn p(line: usize, col: usize) -> Point {
        Point::new(line, col)
    }

    // ---------------- find_matches_opts ----------------

    #[test]
    fn find_opts_case_sensitive_splits_hits() {
        let b = Buffer::from_plain_text("foo bar\nfoo baz\nFOO");
        // 既有语义（不敏感）：3 处
        assert_eq!(b.find_matches_opts("foo", false, false).len(), 3);
        // 敏感：仅小写 2 处
        assert_eq!(b.find_matches_opts("foo", true, false).len(), 2);
        // 敏感大写 query：仅 FOO 1 处
        assert_eq!(b.find_matches_opts("FOO", true, false).len(), 1);
        assert_eq!(b.find_matches_opts("FOO", true, false)[0], p(2, 0));
    }

    #[test]
    fn find_opts_whole_word_skips_substrings() {
        let b = Buffer::from_plain_text("foo foobar foo_bar foo, 12foo");
        let hits = b.find_matches_opts("foo", false, true);
        // foobar/foo_bar/12foo 均非全词；仅第 0 列与 foo 后逗号两处
        assert_eq!(hits.len(), 2, "全词只命中独立 foo：{hits:?}");
        assert_eq!(hits[0], p(0, 0));
        assert_eq!(hits[1], p(0, 19), "foo, 的 foo（前是空格后是逗号）");
    }

    #[test]
    fn find_opts_whole_word_cjk_boundary() {
        // CJK 是词字符：查找「查找」在「查找中文」中命中（后是中文词字符
        // → 不命中）；「中文查找」中命中（前是词字符 → 不命中）。
        let b = Buffer::from_plain_text("查找中文\n中文 查找");
        let hits = b.find_matches_opts("查找", false, true);
        assert_eq!(hits.len(), 1, "仅第二行空格后的独立「查找」");
        assert_eq!(hits[0], p(1, 3));
    }

    #[test]
    fn find_opts_empty_query_and_overlapping_hits() {
        let b = Buffer::from_plain_text("aaa");
        assert!(b.find_matches_opts("", true, true).is_empty());
        // 重叠命中语义：aaa 含 aa 两处（col 0/1）——替换侧负责过滤
        assert_eq!(b.find_matches_opts("aa", false, false).len(), 2);
    }

    // ---------------- replace_batch ----------------

    #[test]
    fn replace_batch_applies_and_single_undo_restores_all() {
        let mut b = Buffer::from_plain_text("foo bar\nfoo baz");
        let n = b.replace_batch(vec![
            (p(0, 0), p(0, 3), "QUUX".to_string()),
            (p(1, 0), p(1, 3), "QUUX".to_string()),
        ]);
        assert_eq!(n, 2);
        assert_eq!(b.to_plain_text(), "QUUX bar\nQUUX baz");
        assert_eq!(b.last_undo_batch_count(), 2, "一条批量 undo 记录");
        // 一次 undo 整批回滚
        assert!(b.undo());
        assert_eq!(b.to_plain_text(), "foo bar\nfoo baz");
        assert_eq!(b.cursor(), p(0, 0), "批量撤销光标落批首");
        // 一次 redo 整批重放
        assert!(b.redo());
        assert_eq!(b.to_plain_text(), "QUUX bar\nQUUX baz");
        assert_eq!(b.cursor(), p(1, 4), "批量重做光标落批尾插入尾");
    }

    #[test]
    fn replace_batch_empty_is_noop() {
        let mut b = Buffer::from_plain_text("abc");
        assert_eq!(b.replace_batch(Vec::new()), 0);
        assert_eq!(b.to_plain_text(), "abc");
        assert!(!b.undo(), "无记录可撤");
        assert!(!b.is_modified());
    }

    #[test]
    fn replace_batch_multiline_replacement_cross_lines() {
        // 替换文本含换行：占位尾部按 end_after_insert 公式展开
        let mut b = Buffer::from_plain_text("a X c\nX");
        let n = b.replace_batch(vec![
            (p(0, 2), p(0, 3), "1\n2".to_string()),
            (p(1, 0), p(1, 1), "3\n4".to_string()),
        ]);
        assert_eq!(n, 2);
        assert_eq!(b.to_plain_text(), "a 1\n2 c\n3\n4");
        assert!(b.undo());
        assert_eq!(b.to_plain_text(), "a X c\nX");
        assert!(b.redo());
        assert_eq!(b.to_plain_text(), "a 1\n2 c\n3\n4");
    }

    #[test]
    fn replace_batch_mixed_with_single_undo_stack() {
        // 批量记录与单点记录在同一栈内交替：深度计数（is_modified）
        // 仍按条目数计——保存点语义不变
        let mut b = Buffer::from_plain_text("aa bb aa");
        b.replace_range(p(0, 3), p(0, 5), "cc"); // 单点 1 条
        assert!(b.is_modified());
        b.mark_saved();
        assert!(!b.is_modified());
        let n = b.replace_batch(vec![
            (p(0, 0), p(0, 2), "zz".to_string()),
            (p(0, 6), p(0, 8), "zz".to_string()),
        ]);
        assert_eq!(n, 2);
        assert!(b.is_modified(), "批量替换后偏离保存点（深度 1→2）");
        assert!(b.undo(), "撤批量");
        assert!(!b.is_modified(), "撤批量 = 回保存点（单点编辑在保存点内）");
    }

    /// #174/#547：千处批量替换性能采样——三个独立、等价样本取最优
    ///（排除单次调度抖动；样本等价性是 #547 的成果，分档重构原样
    /// 保留）。构建与匹配不计时，每轮真实替换及撤销均功能验证；
    /// 不复用已经修改的 Buffer 或旧 edits，避免后续样本变成空操作。
    /// 返回 `(replace_samples, undo_samples)`——门禁由分档包装断言。
    fn thousand_replace_perf_samples() -> (Vec<std::time::Duration>, Vec<std::time::Duration>) {
        let source = (0..1000)
            .map(|i| format!("第{i}行 foo 内容"))
            .collect::<Vec<_>>()
            .join("\n");
        let replaced = source.replace("foo", "FOO");
        let mut replace_samples = Vec::with_capacity(3);
        let mut undo_samples = Vec::with_capacity(3);
        for sample in 0..3 {
            let mut b = Buffer::from_plain_text(&source);
            let edits: Vec<(Point, Point, String)> = b
                .find_matches_opts("foo", false, false)
                .into_iter()
                .scan((0usize, 0usize), |(prev_line, prev_end_col), m| {
                    // 非重叠过滤与 find_bar::replace_all 同口径。
                    let ok = m.line > *prev_line || m.col >= *prev_end_col;
                    if ok {
                        *prev_line = m.line;
                        *prev_end_col = m.col + 3;
                    }
                    ok.then(|| (m, Point::new(m.line, m.col + 3), "FOO".to_string()))
                })
                .collect();
            assert_eq!(edits.len(), 1000, "样本{sample}：千行千处");
            let t = std::time::Instant::now();
            let n = b.replace_batch(edits);
            let elapsed = t.elapsed();
            assert_eq!(n, 1000, "样本{sample}：全部替换");
            assert_eq!(b.to_plain_text(), replaced, "样本{sample}：正文全部替换");
            replace_samples.push(elapsed);

            // 每轮独立计量撤销，功能断言不因最小耗时采样而减弱。
            let t = std::time::Instant::now();
            assert!(b.undo(), "样本{sample}：一次撤销全部替换");
            let undo_elapsed = t.elapsed();
            undo_samples.push(undo_elapsed);
            assert_eq!(b.to_plain_text(), source, "样本{sample}：撤销恢复原文");
            println!("perf(千处批量替换，样本{sample}) = {elapsed:?}");
            println!("perf(千处批量撤销，样本{sample}) = {undo_elapsed:?}");
        }
        (replace_samples, undo_samples)
    }

    /// #174 默认档（负载容忍线 300ms）——全量 `cargo test --lib`
    /// 常驻。#91 G1 原 100ms 线在并行满载套件（383+ 例同跑 + 编译
    /// 负载并存的真机环境）下贴线抖动（单跑全过、全量偶发——见
    /// #174 实测）；本档抓 **10 倍级退化**（rope/映射路径回归），
    /// 微优化级回归归严格档。替换/撤销语义由本模块功能用例锁定，
    /// 不靠时长门禁。
    #[test]
    fn replace_batch_thousand_matches_under_300ms() {
        let (replace_samples, undo_samples) = thousand_replace_perf_samples();
        let best = *replace_samples.iter().min().expect("固定三个独立样本");
        println!("perf(千处批量替换) = {replace_samples:?}; best = {best:?}（默认档<300ms）");
        println!("perf(千处批量撤销) = {undo_samples:?}");
        assert!(
            best < std::time::Duration::from_millis(300),
            "千处批量替换超默认档（负载容忍线 300ms）：best={best:?}, samples={replace_samples:?}"
        );
    }

    /// #174 严格档（#91 G1 原验收线 100ms）——`#[ignore]` 只在无负载
    /// 环境手动跑：
    ///
    /// ```text
    /// cargo test -p mang-shell --lib -- --ignored replace_batch_thousand_matches_strict
    /// ```
    ///
    /// 适用场景：性能专项/发版前空载基线（无并行套件、无编译负载）。
    /// 两档差异即 #174 的分档语义：默认档防退化量级，严格档保微优化
    /// 基线。
    #[test]
    #[ignore = "#174：100ms 线在并行负载下贴线抖动——无负载环境手动跑（--ignored）"]
    fn replace_batch_thousand_matches_strict_100ms() {
        let (replace_samples, undo_samples) = thousand_replace_perf_samples();
        let best = *replace_samples.iter().min().expect("固定三个独立样本");
        println!("perf(千处批量替换) = {replace_samples:?}; best = {best:?}（严格档<100ms）");
        println!("perf(千处批量撤销) = {undo_samples:?}");
        assert!(
            best < std::time::Duration::from_millis(100),
            "千处批量替换超原验收线：best={best:?}, samples={replace_samples:?}"
        );
    }
}

// =============================================================================
// =============================================================================

#[cfg(test)]
mod g3_indent_tests {
    use super::*;

    #[test]
    fn indent_unit_setting_roundtrip() {
        for unit in [IndentUnit::Spaces4, IndentUnit::Spaces2, IndentUnit::Tab] {
            let key = unit.setting_key();
            assert_eq!(IndentUnit::from_setting(key), unit, "键值往返一致");
        }
        // 未知值归缺省（4 空格——与主题模式归一同口径）
        assert_eq!(IndentUnit::from_setting("bogus"), IndentUnit::Spaces4);
        assert_eq!(IndentUnit::default(), IndentUnit::Spaces4);
    }

    #[test]
    fn indent_unit_texts() {
        assert_eq!(IndentUnit::Spaces4.as_text(), "    ");
        assert_eq!(IndentUnit::Spaces2.as_text(), "  ");
        assert_eq!(IndentUnit::Tab.as_text(), "\t");
    }

    #[test]
    fn leading_indent_counts_spaces_and_tabs() {
        let b = Buffer::from_plain_text("    indented\n  two\n\ttabbed\nnone");
        assert_eq!(b.leading_indent_len(0), 4);
        assert_eq!(b.leading_indent_len(1), 2);
        assert_eq!(b.leading_indent_len(2), 1, "Tab 按 1 char 计");
        assert_eq!(b.leading_indent_len(3), 0);
        // 混合前导（空格+Tab+空格 均计入，直到第一个非空白）
        let m = Buffer::from_plain_text("  \t mixed");
        assert_eq!(m.leading_indent_len(0), 4, "2 空格 + 1 Tab + 1 空格");
    }
}

/// #111 订阅模式：revision / pending_changes / 事件记录形态。
#[cfg(test)]
mod rpc_extend_tests {
    use super::*;

    fn p(line: usize, col: usize) -> Point {
        Point::new(line, col)
    }

    #[test]
    fn revision_increments_per_transaction_and_marks_saved_free() {
        // #163 合并窗语义：同窗内两个文本编辑 = 一条 undo 条目
        //（验收 2——B 轨逐编辑条目退役；per-edit 对照见下）。
        let mut b = Buffer::from_plain_text("hello");
        assert_eq!(b.revision(), 0, "初始版本 0");
        b.insert("x"); // 事务 1（视图版本仍逐事务 +1）
        assert_eq!(b.revision(), 1);
        b.replace_range(p(0, 0), p(0, 1), "H"); // 事务 2
        assert_eq!(b.revision(), 2);
        b.mark_saved();
        assert_eq!(b.revision(), 2, "mark_saved 不是内容变更——版本不动");
        assert!(b.undo());
        assert_eq!(b.revision(), 3, "undo 也是变更事务");
        assert_eq!(b.to_plain_text(), "hello", "一条目 = 两编辑一起撤");
        assert!(b.redo());
        assert_eq!(b.revision(), 4, "redo 同理");
        assert_eq!(b.to_plain_text(), "Hhello");
        assert!(b.undo());
        assert_eq!(b.revision(), 5);
        assert!(!b.undo(), "栈真空了（合并条目 = 一条）");
        assert_eq!(b.revision(), 5, "空撤销不动版本");
    }

    #[test]
    fn revision_per_edit_undo_when_coalesce_disabled() {
        // #163 验收 8：合并窗 = 0（旋钮接真）→ 逐编辑条目
        //（旧 r39 语义的显式选择面——docx 面同款）。
        let mut b = Buffer::from_plain_text("hello");
        b.set_coalesce_ms(0);
        b.insert("x");
        b.replace_range(p(0, 0), p(0, 1), "H");
        assert_eq!(b.to_plain_text(), "Hhello");
        assert!(b.undo(), "第二编辑独立成条");
        assert_eq!(b.to_plain_text(), "xhello");
        assert!(b.undo(), "insert 也能撤");
        assert_eq!(b.to_plain_text(), "hello");
        assert!(!b.undo());
    }

    #[test]
    fn noop_replace_is_short_circuited() {
        let mut b = Buffer::from_plain_text("abc");
        b.replace_range(p(0, 1), p(0, 1), "");
        assert_eq!(b.revision(), 0, "空域+空文本 = 无变更");
        assert!(b.take_pending().0.is_empty(), "无待发记录");
        assert!(!b.undo(), "no-op 不落 undo 记录");
        // 反例：空文本 + 有域 = 删除（真变更）
        b.replace_range(p(0, 0), p(0, 1), "");
        assert_eq!(b.revision(), 1);
        assert_eq!(b.to_plain_text(), "bc");
    }

    #[test]
    fn pending_records_replace_precisely() {
        let mut b = Buffer::from_plain_text("one\ntwo");
        b.replace_range(p(0, 1), p(0, 2), "X"); // "one"→"oXe"
        let (pending, _) = b.take_pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].kind, "replace");
        assert_eq!(pending[0].start, Some(p(0, 1)));
        assert_eq!(pending[0].end, Some(p(0, 2)), "end = 变更前被替换域终点");
        assert_eq!(pending[0].text, Some("X".into()));
        // drain 后清空（不重复发）
        assert!(b.take_pending().0.is_empty());
    }

    #[test]
    fn pending_records_undo_redo_single_precisely() {
        let mut b = Buffer::from_plain_text("abc");
        b.set_cursor(p(0, 3));
        b.insert("def"); // [0,3)-[0,3) 插 "def"
        let _ = b.take_pending();

        assert!(b.undo());
        let (pending, _) = b.take_pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].kind, "undo");
        assert_eq!(pending[0].start, Some(p(0, 3)));
        assert_eq!(
            pending[0].end,
            Some(p(0, 6)),
            "撤销前域 = new_text 占位域 [0,3)-[0,6)"
        );
        assert_eq!(pending[0].text, Some("".into()), "undo 恢复为删除该插入");

        assert!(b.redo());
        let (pending, _) = b.take_pending();
        assert_eq!(pending[0].kind, "redo");
        assert_eq!(pending[0].start, Some(p(0, 3)));
        assert_eq!(
            pending[0].end,
            Some(p(0, 3)),
            "重放前域 = end_del（纯插入）"
        );
        assert_eq!(pending[0].text, Some("def".into()));
    }

    #[test]
    fn pending_records_batch_replace_and_coarse_undo() {
        let mut b = Buffer::from_plain_text("aXbXc");
        // 两个点位一次批量事务（find-replace 全部替换路径）
        b.replace_batch(vec![
            (p(0, 1), p(0, 2), "Y".into()),
            (p(0, 3), p(0, 4), "Y".into()),
        ]);
        assert_eq!(b.revision(), 1, "一次批量事务只 +1");
        let (pending, _) = b.take_pending();
        assert_eq!(pending.len(), 2, "每点位一条记录");
        assert!(pending.iter().all(|r| r.kind == "replace"));
        assert_eq!(pending[0].start, Some(p(0, 1)));
        assert_eq!(pending[1].start, Some(p(0, 3)));

        // 批量撤销：粗粒度（kind-only）
        assert!(b.undo());
        let (pending, _) = b.take_pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].kind, "undo");
        assert!(pending[0].start.is_none(), "批量撤销 start 不可精确——省略");
        assert!(pending[0].text.is_none());
        assert_eq!(b.to_plain_text(), "aXbXc");
    }

    #[test]
    fn pending_records_base_revision_for_span() {
        let mut b = Buffer::from_plain_text("a b c ");
        // 一次手势多步（模拟 undo 批量前的连续编辑）
        b.insert("x");
        b.insert("y");
        let (records, base) = b.take_pending();
        assert_eq!(records.len(), 2, "两步合并一次收尾");
        assert_eq!(base, Some(0), "基线 = 首条入队时版本");
        assert_eq!(b.revision(), 2, "当前版本已 +2");

        // 无变更收尾：空记录 + None 基线
        let (records, base) = b.take_pending();
        assert!(records.is_empty());
        assert_eq!(base, None);
    }

    #[test]
    fn clone_drops_pending_but_keeps_revision() {
        let mut b = Buffer::from_plain_text("abc");
        b.insert("d");
        let snapshot = b.clone();
        assert_eq!(snapshot.revision(), 1, "版本随快照");
        let mut snapshot = snapshot;
        assert!(
            snapshot.take_pending().0.is_empty(),
            "克隆体不是事件源——不带待发记录"
        );
    }

    // ---------------- #178：拒改记录（GUI 反馈数据源） ----------------

    /// 恒拒钩子（锁定行门禁的测试替身——`Err` 即拒）。
    struct RejectAllHook;

    impl EditHook for RejectAllHook {
        fn before_edit(&mut self, _intent: &EditIntent) -> Result<(), String> {
            Err("此行受保护，不可编辑".into())
        }

        fn after_edit(&mut self, _summary: &EditSummary) {}
    }

    #[test]
    fn rejection_recorded_when_hook_denies() {
        let mut b = Buffer::from_plain_text("ab\ncd");
        b.set_edit_hook(Some(Box::new(RejectAllHook)));
        b.set_cursor(p(1, 1));
        let applied = b.replace_range(p(1, 1), p(1, 1), "x");
        assert!(!applied, "钩子拒绝 → 零变更");
        assert_eq!(b.line(1), "cd", "Buffer 未被改动");
        // 收割：原因 + 起点行都落账
        let notice = b.take_rejection().expect("拒改记录必须可收割");
        assert_eq!(notice.reason, "此行受保护，不可编辑");
        assert_eq!(notice.line, 1);
        // 一次性收割：再次取 = None（无残影）
        assert!(b.take_rejection().is_none());
    }

    #[test]
    fn no_rejection_without_hook_or_on_success() {
        // 无钩子（md 纯文本面）：编辑成功且无拒改记录。
        let mut b = Buffer::from_plain_text("ab");
        assert!(b.replace_range(p(0, 1), p(0, 1), "x"));
        assert!(b.take_rejection().is_none(), "成功编辑不留拒改记录");
        // 有钩子但放行：同样无记录（Ok 放行的钩子）。
        struct AllowHook;
        impl EditHook for AllowHook {
            fn before_edit(&mut self, _: &EditIntent) -> Result<(), String> {
                Ok(())
            }
            fn after_edit(&mut self, _: &EditSummary) {}
        }
        let mut b = Buffer::from_plain_text("ab");
        b.set_edit_hook(Some(Box::new(AllowHook)));
        assert!(b.replace_range(p(0, 1), p(0, 1), "x"));
        assert!(b.take_rejection().is_none());
    }

    #[test]
    fn rejection_recorded_for_denied_batch() {
        let mut b = Buffer::from_plain_text("ab\ncd");
        b.set_edit_hook(Some(Box::new(RejectAllHook)));
        let n = b.replace_batch(vec![(p(0, 0), p(0, 1), "z".into())]);
        assert_eq!(n, 0, "整批原子拒");
        let notice = b.take_rejection().expect("批量拒改也落账");
        assert_eq!(notice.line, 0, "行取被拒首条起点");
    }

    #[test]
    fn clone_drops_rejection_like_pending() {
        let mut b = Buffer::from_plain_text("ab");
        b.set_edit_hook(Some(Box::new(RejectAllHook)));
        b.replace_range(p(0, 0), p(0, 0), "x");
        assert!(b.last_reject_note().is_some(), "拒改已落账");
        let snapshot = b.clone();
        assert!(
            snapshot.last_reject_note().is_none(),
            "克隆体不是事件源——不带拒改记录（与 pending 同口径）"
        );
    }
}
