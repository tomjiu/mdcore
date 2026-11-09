//! #414 —— MD 文档 diff 引擎：块级对齐（L1）+ 行内 diff（L2）+ mark 感知 diff（L3，#600）。
//!
//! ## 定位：redline 与「改了哪里」的结构基座
//!
//! mang 此前没有任何文档比较能力。本模块提供三层：
//!
//! - **L1 块级对齐** [`diff_documents`] / [`diff_markdown`]：两版文档
//!   的块序列对齐——新增/删除/移动/修改/未变 五分类（[`BlockDiff`]）；
//! - **L2 行内 diff** [`diff_inline`]：修改块内的文字级定位
//!   （[`InlineDiff`] run 序）。
//! - **L3 mark 感知 diff** [`diff_documents_l3`] / [`diff_markdown_l3`]
//!   （#600）：可选 marks 比较——同文本块的样式变化可观察（去粗/
//!   加斜体不再「文本 Unchanged」吞掉；「重要 重要」位置互换报
//!   Moved）。签名基 = **归一净文本**（行内投影后），与 L1 原入口的
//!   归一源文本口径不同——旧入口语义零漂移（r29/r30 不回归）。
//!
//! 与 #413（DOCX 修订写面）组成 redline 链：L1 产 Added/Removed/
//! Modified 块 → #413 `insert_run_revised`/`delete_run_revised` 落成
//! Word 修订标记；Modified 块经 L2 定位到片段。
//!
//! ## 对齐策略（issue 三步——按 #415/#456 现状裁剪）
//!
//! issue 原文「① 按 block_id 匹配（如果稳定）② 按内容相似度 ③ 纯
//! 增删」。**#415/#456 已闭：P0-1 重编号撞号修复 + 身份验收合入**
//! （#244→PR #249 meta v2 ID 持久化 + r30 三条不变性锁）——同谱系
//! 文档（capture/restore、save→open meta v2——ID 同源）的块 ID
//! 是最强锚。但两份**独立** `from_markdown` 的 ID 空间各自位置序
//! 发号（1..N，重叠），跨谱系按 ID 对齐 = 错锚。因此入口分两形：
//!
//! - [`diff_documents`] / [`diff_markdown`]（默认）：**不作 ID 锚**
//!   （ID 仅在 [`BlockDiff::old_id`]/[`BlockDiff::new_id`] 报告），
//!   对齐全走内容面——语义零漂移；
//! - [`diff_documents_lineage`]（#599）：显式同谱系入口——调用方
//!   声明两侧 ID 同源，第 0 步 ID pass 优先占用唯一配对（同 ID
//!   异文 → Modified，不受文本阈值拆分；同 ID 改 kind → 显式
//!   拆分 Removed+Added——见下）。
//!
//! 内容面步序（两入口共用 [`align`] 实现）：
//!
//! 1. **精确匹配**：`(BlockKind, 归一化文本)` 签名——行尾空白与
//!    末行换行归一（重排序列化不产生伪差），代码块/公式内部逐字
//!    （空白敏感内容不做更深的归一）；
//! 2. **相似度匹配**：同 kind 未配对块，位置窗口内按归一 Levenshtein
//!    相似度贪心配对（阈值 [`SIMILARITY_THRESHOLD`]，窗口
//!    [`SIMILARITY_WINDOW`] 界住平方面）；
//! 3. **剩余**：旧侧未配 = Removed，新侧未配 = Added；
//! 4. **Moved 判定**：同文 ID 对（谱系入口）与精确文本对**共用**
//!    全局 LIS（最长递增子序列，O(n log n)）——LIS 内 = Unchanged
//!    （原位）、LIS 外 = Moved（纯移动 ≠ 删除 + 新增）。相似配对
//!    （Modified）不参与 Moved 二分：
//!    - 内容变化为主标签（Modified）；
//!    - 位置变化可由 old_index/new_index 观察（如实不折叠）。
//!
//! **同 ID 改 BlockKind（#599/#626 契约）**：谱系入口的 ID pass 仅配
//! 同 kind 的同 ID 对——kind 变更（如段落→标题）**显式拆分**为
//! Removed（旧型携旧侧 ID）+ Added（新型携新侧 ID，同一 ID 值——
//! 身份仍可观察），不输出双侧异型的 Modified（[`BlockDiff::block_kind`]
//! 双侧同型契约不得默默破坏）。拆分两端**退出后续全部配对面**
//! （#626：精确/相似 pass 不得偷配——旧端从签名与相似候选池排除、
//! 新端置占位；否则同文/近文块会把拆分端偷配成 Unchanged/Modified，
//! 丢失应保留的 Removed/Added 事件）。文本 pass 同样要求同 kind，
//! 拆分结果与内容面自然一致。
//!
//! ## L2：行内 diff（零新依赖的 clean-room Myers）
//!
//! 研究仓建议 vendor diff-match-patch（Apache-2.0）——本仓 mdcore
//! 依赖纪律为 ropey/serde/wasm-bindgen 三项闭包（R8），vendoring
//! 第三方算法体量（含模糊匹配/语义清整约千行）突破闭包；此处按
//! Myers 1986 中点蛇形分治（O((N+M)·D) 时间 / O(N+M) 空间——相似
//! 文本近线性）clean-room 实现词元级 diff：
//!
//! - 词元化 [`tokenize`]：拉丁词连续 / CJK 单字 / 空白段 / 单标点
//!   ——CJK 无空格分词面按字粒度（中文改动可定位），拉丁面按词
//!   粒度（输出可读）；
//! - 相等/插入/删除 run 序（连续同类合并），`Modified`/`Moved` 两
//!   变体不用于行内层。
//!
//! ## 性能（验收 3：10k 行 < 1s）
//!
//! L1：精确匹配 O(N)（哈希签名）+ LIS O(N log N) + 相似窗口界
//! （窗口外零 DP、带内 banded Levenshtein + 长度差早拒）——10k 块
//! 实测见 `tests/r29_diff_engine.rs::perf_10k_lines_under_1s`。
//! L2：Myers D 界——相似文本近线性；块内文本量天然有界。

use serde::{Deserialize, Serialize};

use crate::id::BlockId;
use crate::tree::BlockKind;
use crate::Core;

// ---------------------------------------------------------------------------
// 类型
// ---------------------------------------------------------------------------

/// diff 分类（L1 五类；L2 行内层只用前三类）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffKind {
    /// 块未变（精确匹配且原位）。
    Unchanged,
    /// 块新增（仅新侧存在）。
    Added,
    /// 块删除（仅旧侧存在）。
    Removed,
    /// 块纯移动（精确匹配但位序变——非 LIS 内）。
    Moved,
    /// 块修改（相似度配对，内容有变）。
    Modified,
}

/// L1 块级 diff 条目（自包含：携带双侧 ID/序/文本——wasm/wire 面
/// 可直接序列化，消费方无需回读源文档）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockDiff {
    /// 分类。
    pub kind: DiffKind,
    /// 旧侧块 ID（Added 为 None）。
    pub old_id: Option<BlockId>,
    /// 新侧块 ID（Removed 为 None）。
    pub new_id: Option<BlockId>,
    /// 旧侧先序块号（Added 为 None）。
    pub old_index: Option<usize>,
    /// 新侧先序块号（Removed 为 None）。
    pub new_index: Option<usize>,
    /// 块类型（双侧同型——精确/相似匹配均要求同 kind；Added 取新侧、
    /// Removed 取旧侧）。
    pub block_kind: BlockKind,
    /// 旧侧文本（归一化形态——见模块文档；Added 为 None）。
    pub text_old: Option<String>,
    /// 新侧文本（归一化形态；Removed 为 None）。
    pub text_new: Option<String>,
}

/// L2 行内 diff run（连续同类词元合并；`Unchanged`/`Added`/`Removed`
/// 三类——`Moved`/`Modified` 不用于行内层）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlineDiff {
    pub kind: DiffKind,
    pub text: String,
}

// ---------------------------------------------------------------------------
// L3：mark 感知 diff（#600）
// ---------------------------------------------------------------------------

/// #600 L3：mark 变更种类（[`MarkChange`] 的分类面）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkChangeKind {
    /// 新侧独有（旧侧无同签名实例）——样式新增。
    Added,
    /// 旧侧独有（新侧无同签名实例）——样式移除。
    Removed,
    /// 同签名实例双侧均在但位置不同——「重要 重要」互换面。
    Moved,
}

/// #600 L3：单条 mark 变更事件（同文本配对块内）。
///
/// 坐标基 = 该块 [`BlockDiffL3::base`] 携带的**归一净文本** char 偏移
/// （Unicode 码点——不做字节索引切片）。`text` 为区间文本快照（钳制
/// 后的可观察凭据——位置互换判定与消费方展示的依据）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkChange {
    /// mark 类型（[`crate::tree::MarkSpan::kind`] 原样——markdown 面
    /// em/strong/code/del/math/u，op 面自定义串）。
    pub kind: String,
    /// 变更种类。
    pub change: MarkChangeKind,
    /// 旧侧 char 区间（Added 为 None）。
    pub old_range: Option<(usize, usize)>,
    /// 新侧 char 区间（Removed 为 None）。
    pub new_range: Option<(usize, usize)>,
    /// 区间文本快照（Removed/Moved 取旧侧、Added 取新侧）。
    pub text: String,
}

/// #600 L3：块级 diff + mark 变更（可选 marks 比较面——wire 形）。
///
/// - **基座兼容**：`#[serde(flatten)]` 内联 [`BlockDiff`] 全字段——
///   旧 L1 JSON（无 `mark_changes` 键）可直读本结构（`serde(default)`
///   补空）；Rust 构造点仍须全字面量（AGENTS 0.0.5——serde 宽容不
///   替代构造检查）。
/// - **口径声明**：L3 入口的 `text_old`/`text_new` 为**归一净文本**
///   （行内投影后）——与 L1 原入口的归一源文本不同基，字段同名、
///   语义面在此标注。
/// - **marks 比较面**：仅同文本配对块（Unchanged/Moved）参与——
///   Added/Removed/Modified 的样式随块级事件表达（Modified 的
///   L2×mark 联合坐标映射另排，见 #600 任务边界）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockDiffL3 {
    /// L1 基座原样携带（注意上述口径声明）。
    #[serde(flatten)]
    pub base: BlockDiff,
    /// 本块 mark 变更事件（空 = 无样式变化——双侧 marks 等价或非比较面）。
    #[serde(default)]
    pub mark_changes: Vec<MarkChange>,
}

/// 相似度配对阈值（归一 Levenshtein：1 - dist/max_len；≥ 阈值视为
/// 同块修改，< 阈值视为无关块）。0.5 = 中改；重写块按 Added/Removed
/// 报告（保守——不把不相关的块硬配成「修改」）。
pub const SIMILARITY_THRESHOLD: f64 = 0.5;

/// 相似度配对的位置窗口（块数）——窗口外的同 kind 未配对块不做
/// DP（O(N²) 界），窗口内做 banded Levenshtein。
const SIMILARITY_WINDOW: usize = 96;

// ---------------------------------------------------------------------------
// L1：块级对齐
// ---------------------------------------------------------------------------

/// 两版文档块级 diff（自包含条目序：按新侧块序排列——Added/Unchanged/
/// Moved/Modified 按新序，Removed 按旧序插回其邻位之后）。
///
/// 等价糖衣 [`diff_markdown`]（两段 MD 直 diff）。
///
/// **默认入口不作 ID 锚**（两侧可能各自独立 parse——位置序 ID 空间
/// 重叠，错锚；见模块文档）。同谱系（ID 同源）请走
/// [`diff_documents_lineage`]。
pub fn diff_documents(old: &Core, new: &Core) -> Vec<BlockDiff> {
    let old_blocks = flatten(old);
    let new_blocks = flatten(new);
    align(&old_blocks, &new_blocks)
}

/// #599：同谱系块级 diff——第 0 步 ID pass 优先占用唯一配对。
///
/// **谱系前提（调用方声明）**：两侧 Core 的块 ID 同源——capture/
/// restore、`save_document`→`open_document`（meta v2 ID 持久化）
/// 等路径。两份独立 `from_markdown` 的 ID 空间各自位置序发号
/// （重叠），走本入口 = 错锚（伪 Modified）。
///
/// 语义（相对 [`diff_documents`] 的唯一增量 = 第 0 步）：
/// - 同 ID 同 kind：优先配对（唯一占用）——异文 → Modified
///   （不受 [`SIMILARITY_THRESHOLD`] 拆分），同文 → 参与全局 LIS
///   （Unchanged/Moved 判定与精确文本对共用）；
/// - 同 ID 改 kind：显式拆分 Removed（旧型）+ Added（新型）——
///   [`BlockDiff::block_kind`] 双侧同型契约保持（见模块文档）；
/// - 后续精确/相似 pass 跳过已占 old/new（不覆盖配对、不复用
///   new、不丢事件）。
pub fn diff_documents_lineage(old: &Core, new: &Core) -> Vec<BlockDiff> {
    let old_blocks = flatten(old);
    let new_blocks = flatten(new);
    align_with_id_pass(&old_blocks, &new_blocks)
}

/// MD 文本直 diff（`from_markdown` 双侧 + [`diff_documents`]）。
pub fn diff_markdown(old_md: &str, new_md: &str) -> Vec<BlockDiff> {
    let old = Core::from_markdown(old_md);
    let new = Core::from_markdown(new_md);
    diff_documents(&old, &new)
}

/// #600 L3：mark 感知块级 diff（内容锚——默认入口的净文本增强面）。
///
/// 与 [`diff_documents`] 的差异（其余语义含 LIS/Moved 判定全部同构）：
///
/// 1. **签名基 = 归一净文本**：每块先做行内投影（markdown 面
///    `parse_inline_marks` 剥定界符 / op 面 `Node.marks` 权威）再对齐
///    ——`**重要** 事项` → `重要 事项` 配对为同文本，样式差异落
///    [`BlockDiffL3::mark_changes`] 而非 Modified 文本差；
/// 2. **marks 比较**（同文本配对块——Unchanged/Moved）：同签名实例
///    互配（精确位 → 文本签名+序位两 pass），位置差异显式报
///    [`MarkChangeKind::Moved`]（「重要 重要」互换可观察），独有端报
///    Added/Removed；
/// 3. **wire**：[`BlockDiffL3`]（flatten 基座——旧 L1 JSON 可直读）。
///
/// 边界（#600 任务面登记）：Modified 块不做 mark 坐标映射（L2×mark
/// 联合另排）；Table/CodeBlock/Math 无行内投影（Table 逐格坐标基不
/// 可复合——样式随文本事件表达）；谱系 ID pass 面未接（同谱系调用方
/// 可先 L1 lineage 后续轮接线）。
pub fn diff_documents_l3(old: &Core, new: &Core) -> Vec<BlockDiffL3> {
    let old_blocks = flatten_l3(old);
    let new_blocks = flatten_l3(new);
    // 对齐视图：FlatBlock 文本槽装净文本（私有结构——语义面按本入口
    // 口径），复用 align_impl 全部步序（精确/相似/LIS/组装）。
    let old_view: Vec<FlatBlock> = old_blocks
        .iter()
        .map(|b| FlatBlock {
            id: b.id,
            kind: b.kind,
            text: b.net.clone(),
        })
        .collect();
    let new_view: Vec<FlatBlock> = new_blocks
        .iter()
        .map(|b| FlatBlock {
            id: b.id,
            kind: b.kind,
            text: b.net.clone(),
        })
        .collect();
    let base = align_impl(&old_view, &new_view, false);
    // 后处理：同文本配对块做 marks 比较（坐标基 = 净文本——即基座
    // 携带的 text_old/text_new）。
    base.into_iter()
        .map(|d| {
            let mark_changes = match (d.kind, d.old_index, d.new_index) {
                (DiffKind::Unchanged | DiffKind::Moved, Some(i), Some(j)) => {
                    let text = d.text_new.as_deref().unwrap_or("");
                    mark_changes_between(&old_blocks[i].marks, &new_blocks[j].marks, text)
                }
                _ => Vec::new(),
            };
            BlockDiffL3 {
                base: d,
                mark_changes,
            }
        })
        .collect()
}

/// #600 L3：MD 文本直 diff（`from_markdown` 双侧 + [`diff_documents_l3`]）。
pub fn diff_markdown_l3(old_md: &str, new_md: &str) -> Vec<BlockDiffL3> {
    let old = Core::from_markdown(old_md);
    let new = Core::from_markdown(new_md);
    diff_documents_l3(&old, &new)
}

/// 先序扁平块（跳过虚根 Document；父先子后同层按序——与
/// `preorder_block_ids` 同序）。
struct FlatBlock {
    id: BlockId,
    kind: BlockKind,
    /// 归一化文本（行尾空白 + 末行换行归一）。
    text: String,
}

fn flatten(core: &Core) -> Vec<FlatBlock> {
    let tree = &core.tree;
    let mut out = Vec::new();
    fn walk(tree: &crate::tree::BlockTree, node: crate::tree::NodeId, out: &mut Vec<FlatBlock>) {
        let n = tree.node(node);
        out.push(FlatBlock {
            id: n.id,
            kind: n.kind,
            text: normalize_block_text(&n.text.to_string()),
        });
        for &c in &n.children {
            walk(tree, c, out);
        }
    }
    let top: Vec<crate::tree::NodeId> = tree.node(tree.root()).children.clone();
    for t in top {
        walk(tree, t, &mut out);
    }
    out
}

/// 块文本归一：行尾空白剥离 + 末行换行剥离（重排序列化的伪差面）。
/// 代码块/公式内部（缩进、空行）逐字保留——空白敏感内容不做更深归一。
fn normalize_block_text(text: &str) -> String {
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(|l| l.trim_end_matches([' ', '\r']))
        .collect();
    while matches!(lines.last(), Some(&"")) {
        lines.pop();
    }
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// #600 L3 内部：行内投影（净文本 + marks 坐标基）
// ---------------------------------------------------------------------------

/// L3 扁平块（净文本坐标基——对齐签名与 marks 比较共用同一基）。
struct FlatBlockL3 {
    id: BlockId,
    kind: BlockKind,
    /// 归一净文本（行内投影 + 行尾归一后）。
    net: String,
    /// 净文本坐标基上的有效 marks。
    marks: Vec<crate::tree::MarkSpan>,
}

/// #600 L3 扁平（先序——与 [`flatten`] 同序；每块带行内投影）。
fn flatten_l3(core: &Core) -> Vec<FlatBlockL3> {
    let tree = &core.tree;
    let mut out = Vec::new();
    fn walk(tree: &crate::tree::BlockTree, node: crate::tree::NodeId, out: &mut Vec<FlatBlockL3>) {
        let n = tree.node(node);
        let (net, marks) = effective_inline(n.kind, &n.text.to_string(), &n.marks);
        out.push(FlatBlockL3 {
            id: n.id,
            kind: n.kind,
            net,
            marks,
        });
        for &c in &n.children {
            walk(tree, c, out);
        }
    }
    let top: Vec<crate::tree::NodeId> = tree.node(tree.root()).children.clone();
    for t in top {
        walk(tree, t, &mut out);
    }
    out
}

/// 块的行内有效投影（#600 L3 坐标基单点）。
///
/// 两来源面（沿 `word_count::net_text` 同款约定）：
/// - **markdown 面**（`marks` 空——`from_markdown` 树）：先
///   [`normalize_block_text`] 归一（坐标基稳定），再
///   [`crate::inline::parse_inline_marks`] 剥定界符——marks 天然落在
///   归一净文本坐标上；
/// - **op 面**（`marks` 非空——SetMark 路径，text 已净）：归一剥尾随
///   空白引起的坐标漂移按 [`normalize_with_map`] 幸存者映射重定位。
///
/// 非投影面：CodeBlock/Math（定界即内容，逐字）与 Table（逐格坐标基
/// 不可复合——word_count 逐格同规，L3 整块不投影）恒返回
/// 归一原文 + 空 marks。ThematicBreak 无文本面。
fn effective_inline(
    kind: BlockKind,
    text: &str,
    marks: &[crate::tree::MarkSpan],
) -> (String, Vec<crate::tree::MarkSpan>) {
    if matches!(
        kind,
        BlockKind::CodeBlock | BlockKind::Math | BlockKind::Table
    ) {
        return (normalize_block_text(text), Vec::new());
    }
    if marks.is_empty() {
        let norm = normalize_block_text(text);
        let (net, marks) = crate::inline::parse_inline_marks(&norm);
        (net, marks)
    } else {
        let (norm, map) = normalize_with_map(text);
        let n = norm.chars().count();
        let remapped = marks
            .iter()
            .filter_map(|m| {
                // 越界端点钳制到文本长（敌意/漂移面——不 panic）；
                // start > end 的敌意形态（钳后仍倒挂）剔除。
                let s = map[m.start.min(n)];
                let e = map[m.end.min(n)];
                (s <= e).then(|| crate::tree::MarkSpan {
                    start: s,
                    end: e,
                    kind: m.kind.clone(),
                })
            })
            .collect();
        (norm, remapped)
    }
}

/// 归一 + char 索引映射（#600 L3 op 面坐标重定位）。
///
/// 语义与 [`normalize_block_text`] 逐字符等价（单测锁）；额外产出
/// `map[i]` = 原始前 i 个 char 中**幸存者个数**（= 原始索引 i 的归一
/// 位点——被剥字符向内钳制到其前幸存尾部）。map 长度 = char 数 + 1
/// （含末位哨兵——end 边界可直索引）。
fn normalize_with_map(text: &str) -> (String, Vec<usize>) {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    // 行切分（'\n' 为行界符——不属行内容；空文本 = 单空行）。
    let mut line_content: Vec<(usize, usize)> = Vec::new();
    let mut line_nl: Vec<Option<usize>> = Vec::new();
    let mut i = 0;
    loop {
        let start = i;
        while i < n && chars[i] != '\n' {
            i += 1;
        }
        line_content.push((start, i));
        if i < n {
            line_nl.push(Some(i));
            i += 1;
        } else {
            line_nl.push(None);
            break;
        }
    }
    // 每行剥尾随 [' ', '\r']（normalize 的 trim_end 面）。
    let kept: Vec<(usize, usize)> = line_content
        .iter()
        .map(|&(s, e)| {
            let mut ce = e;
            while ce > s && matches!(chars[ce - 1], ' ' | '\r') {
                ce -= 1;
            }
            (s, ce)
        })
        .collect();
    // 尾部空行剥除（内容为空——normalize 的 pop 语义；中段空行保留）。
    let mut last = kept.len();
    while last > 0 && kept[last - 1].0 == kept[last - 1].1 {
        last -= 1;
    }
    // 幸存掩码：保留行内容 + 行间 '\n'（最后一保留行的 '\n' 不存活——
    // join 不产尾换行）。
    let mut survivor = vec![false; n];
    for li in 0..last {
        let (s, ce) = kept[li];
        for cell in survivor.iter_mut().take(ce).skip(s) {
            *cell = true;
        }
        if let Some(nl) = line_nl[li] {
            if li + 1 < last {
                survivor[nl] = true;
            }
        }
    }
    // 组装：净文本 + 前缀幸存计数。
    let mut norm = String::new();
    let mut map = Vec::with_capacity(n + 1);
    let mut count = 0usize;
    map.push(0);
    for k in 0..n {
        if survivor[k] {
            norm.push(chars[k]);
            count += 1;
        }
        map.push(count);
    }
    (norm, map)
}

/// #600 L3：mark 区间文本快照（char 流钳制切片——**不做字节索引**；
/// 越界端点按 skip/take 自然截断，start ≥ end 为空）。
fn span_text(text: &str, start: usize, end: usize) -> String {
    if start >= end {
        return String::new();
    }
    text.chars().skip(start).take(end - start).collect()
}

/// #600 L3：同文本块的两版 marks 比较（事件推导——纯函数）。
///
/// 两 pass 配对（`text` = 该块净文本，双侧同基）：
/// 1. **精确三元组**（kind, start, end）互配——零事件（「两处相同样式
///    删一处只报对应变化」：位相合者互消，仅剩端报 Removed/Added）；
/// 2. **同 kind + 文本签名**按双侧序位互配（左 ↔ 左）——位置差异
///    显式报 [`MarkChangeKind::Moved`]（「重要 重要」互换面：不签名
///    消除位置差异）。
///
/// 剩余：旧侧独有 → Removed；新侧独有 → Added。
///
/// 事件序 = 起始位置字典序（Removed/Moved 取旧侧起点、Added 取新侧
/// 起点；同位按 Removed → Moved → Added 稳定序）——确定可复现。
/// 重叠 marks 逐条独立比较（不合并、不互斥）；空区间（start == end）
/// 参与比较（事件可观察——不静默剔除）。
fn mark_changes_between(
    old: &[crate::tree::MarkSpan],
    new: &[crate::tree::MarkSpan],
    text: &str,
) -> Vec<MarkChange> {
    if old.is_empty() && new.is_empty() {
        return Vec::new();
    }
    let mut old_used = vec![false; old.len()];
    let mut new_used = vec![false; new.len()];
    // Pass 1：精确三元组互配。
    for i in 0..old.len() {
        if old_used[i] {
            continue;
        }
        for j in 0..new.len() {
            if !new_used[j]
                && old[i].kind == new[j].kind
                && old[i].start == new[j].start
                && old[i].end == new[j].end
            {
                old_used[i] = true;
                new_used[j] = true;
                break;
            }
        }
    }
    // Pass 2：同 kind + 同文本签名按双侧序位互配 → Moved。
    let mut events: Vec<MarkChange> = Vec::new();
    for i in 0..old.len() {
        if old_used[i] {
            continue;
        }
        let old_txt = span_text(text, old[i].start, old[i].end);
        for j in 0..new.len() {
            if new_used[j] || old[i].kind != new[j].kind {
                continue;
            }
            let new_txt = span_text(text, new[j].start, new[j].end);
            if old_txt == new_txt {
                old_used[i] = true;
                new_used[j] = true;
                events.push(MarkChange {
                    kind: old[i].kind.clone(),
                    change: MarkChangeKind::Moved,
                    old_range: Some((old[i].start, old[i].end)),
                    new_range: Some((new[j].start, new[j].end)),
                    text: old_txt,
                });
                break;
            }
        }
    }
    // 剩余：Removed（旧侧独有）/ Added（新侧独有）。
    for i in 0..old.len() {
        if !old_used[i] {
            events.push(MarkChange {
                kind: old[i].kind.clone(),
                change: MarkChangeKind::Removed,
                old_range: Some((old[i].start, old[i].end)),
                new_range: None,
                text: span_text(text, old[i].start, old[i].end),
            });
        }
    }
    for j in 0..new.len() {
        if !new_used[j] {
            events.push(MarkChange {
                kind: new[j].kind.clone(),
                change: MarkChangeKind::Added,
                old_range: None,
                new_range: Some((new[j].start, new[j].end)),
                text: span_text(text, new[j].start, new[j].end),
            });
        }
    }
    // 起始位置字典序 + 同位种类稳定序（removed < moved < added）。
    events.sort_by(|a, b| {
        let pa = a
            .old_range
            .map(|(s, _)| s)
            .unwrap_or_else(|| a.new_range.map(|(s, _)| s).unwrap_or(0));
        let pb = b
            .old_range
            .map(|(s, _)| s)
            .unwrap_or_else(|| b.new_range.map(|(s, _)| s).unwrap_or(0));
        let ra = rank_of(a.change);
        let rb = rank_of(b.change);
        pa.cmp(&pb)
            .then(ra.cmp(&rb))
            .then(a.kind.cmp(&b.kind))
            .then(a.old_range.cmp(&b.old_range))
            .then(a.new_range.cmp(&b.new_range))
    });
    events
}

/// 事件种类排序秩（Removed → Moved → Added——同位稳定序）。
fn rank_of(c: MarkChangeKind) -> u8 {
    match c {
        MarkChangeKind::Removed => 0,
        MarkChangeKind::Moved => 1,
        MarkChangeKind::Added => 2,
    }
}

/// 对齐主体（纯函数——旧/新扁平块 → diff 序）。
///
/// 默认面（内容锚）——语义与 #414 交付零漂移。谱系面见
/// [`align_with_id_pass`]（共用 [`align_impl`]，第 0 步 ID pass 开）。
fn align(old: &[FlatBlock], new: &[FlatBlock]) -> Vec<BlockDiff> {
    align_impl(old, new, false)
}

/// #599：谱系对齐（第 0 步 ID pass 开——见 [`diff_documents_lineage`]）。
fn align_with_id_pass(old: &[FlatBlock], new: &[FlatBlock]) -> Vec<BlockDiff> {
    align_impl(old, new, true)
}

/// 对齐实现（`id_pass` = 谱系入口的第 0 步开关；步序见模块文档）。
fn align_impl(old: &[FlatBlock], new: &[FlatBlock], id_pass: bool) -> Vec<BlockDiff> {
    // 配对表：old_idx → new_idx（ID/精确/相似三 pass 填充）。
    let mut pair_old_new: Vec<Option<usize>> = vec![None; old.len()];
    let mut paired_new = vec![false; new.len()];
    // #626：同 ID 异 kind 的强制拆分端——旧端强制 Removed（从精确/
    // 相似候选池排除，不得被后续 pass 偷配）；新端经 `paired_new` 标
    // 占即天然排除（组装段自然输出 Added）。全 false 时（默认入口）
    // 对各 pass 判定零影响——`id_pass=false` 语义零漂移。
    let mut forced_removed_old = vec![false; old.len()];
    // 同文配对表（ID 同文对 + 精确文本对——共用全局 LIS）。
    let mut exact: Vec<(usize, usize)> = Vec::new();

    // --- pass 0（仅谱系入口）：同 ID 优先占用唯一配对。同 ID 同 kind
    //     才配（改 kind = 显式拆分 Removed+Added——模块文档契约）；
    //     同文 ID 对入 exact（与精确文本对共用 LIS）；异文 ID 对
    //     仅占位（Modified——组装段判定）。新侧 ID 首现胜（同谱系
    //     ID 唯一是树不变量，防御位不取尽）。
    //
    //     同 ID 异 kind（#626）：两端**双占排除**——旧端记
    //     `forced_removed_old`（pass 1 签名跳过 + pass 2 候选排除），
    //     新端置 `paired_new`（pass 1 队列弹出即弃 + pass 2 未配对
    //     表排除）。不排除则同文/近文的后续块会把这两端偷配成
    //     Unchanged/Modified，丢失拆分契约要求的 Removed/Added 事件
    //     （PR #614 审核三反例）。组装段两端各按未配对自然输出
    //     Removed（旧型携旧 ID）/ Added（新型携同 ID）。
    if id_pass {
        use std::collections::HashMap;
        let mut id_map: HashMap<u64, usize> = HashMap::with_capacity(new.len());
        for (j, b) in new.iter().enumerate() {
            id_map.entry(b.id.0).or_insert(j);
        }
        for (i, b) in old.iter().enumerate() {
            let Some(&j) = id_map.get(&b.id.0) else {
                continue;
            };
            if old[i].kind != new[j].kind {
                // 改 kind：显式拆分——两端退出后续全部配对面。
                forced_removed_old[i] = true;
                paired_new[j] = true;
                continue;
            }
            if paired_new[j] {
                continue; // 已占（防御）
            }
            pair_old_new[i] = Some(j);
            paired_new[j] = true;
            if same_text(&old[i].text, &new[j].text) {
                exact.push((i, j)); // 同文 ID 对共用全局 LIS
            }
        }
    }

    // --- pass 1：精确签名匹配（同内容重复块按先到先配——多副本
    // 场景（同文重复段）按序配对，语义稳定可预测）。签名键 =
    // (kind 判别值, 归一文本)——BlockKind 无 Hash derive（树模块
    // 原有 derive 面不为本模块扩），无字段枚举 `as u8` 即稳定判别。
    // 已占面（ID pass）跳过：旧侧已配不重配；新侧队首已占则丢弃
    // 继续弹（已占位不可复用——不覆盖 ID 配对/不偷 new 块）。
    use std::collections::{HashMap, VecDeque};
    // 键借用两侧切片的文本（old/new 同函数存活——零 clone，万块级
    // 文档的匹配主面免 2N 次字符串拷贝）。
    let mut sig_map: HashMap<(u8, &str), VecDeque<usize>> = HashMap::new();
    for (j, b) in new.iter().enumerate() {
        sig_map
            .entry((b.kind as u8, b.text.as_str()))
            .or_default()
            .push_back(j);
    }
    for (i, b) in old.iter().enumerate() {
        if pair_old_new[i].is_some() || forced_removed_old[i] {
            continue; // ID pass 已占 / #626 强制拆分端（不可偷配）
        }
        if let Some(queue) = sig_map.get_mut(&(b.kind as u8, b.text.as_str())) {
            while let Some(j) = queue.pop_front() {
                if paired_new[j] {
                    continue; // ID pass 已占（不可复用）——丢弃继续弹
                }
                pair_old_new[i] = Some(j);
                paired_new[j] = true;
                exact.push((i, j));
                break;
            }
        }
    }

    // --- pass 2：相似度匹配（同 kind 未配对，位置窗口 + 阈值，按
    // 相似度降序贪心取不冲突对）。#626：强制拆分旧端不入候选池
    // （新端经 paired_new 已排除）——相似文本不得偷配拆分端。
    let unmatched_old: Vec<usize> = (0..old.len())
        .filter(|&i| pair_old_new[i].is_none() && !forced_removed_old[i])
        .collect();
    let unmatched_new: Vec<usize> = (0..new.len()).filter(|&j| !paired_new[j]).collect();
    let mut candidates: Vec<(f64, usize, usize)> = Vec::new();
    for &i in &unmatched_old {
        for &j in &unmatched_new {
            if old[i].kind != new[j].kind {
                continue;
            }
            // 位置窗口（按双侧块数比例对齐的期望位差放宽）。
            let expect = (i as f64) * (new.len() as f64) / (old.len().max(1) as f64);
            if j.abs_diff(expect as usize) > SIMILARITY_WINDOW {
                continue;
            }
            let sim = similarity_bounded(&old[i].text, &new[j].text);
            if sim >= SIMILARITY_THRESHOLD {
                candidates.push((sim, i, j));
            }
        }
    }
    candidates.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap().then(a.1.cmp(&b.1)));
    for (_, i, j) in candidates {
        if pair_old_new[i].is_none() && !paired_new[j] {
            pair_old_new[i] = Some(j);
            paired_new[j] = true;
        }
    }

    // --- Moved 判定：精确配对按新序取 old_idx 的 LIS——LIS 内 =
    // Unchanged（原位），LIS 外 = Moved（纯移动）。
    exact.sort_by_key(|&(_, j)| j);
    let seq: Vec<usize> = exact.iter().map(|&(i, _)| i).collect();
    let in_lis = lis_mask(&seq);
    let mut exact_moved = vec![false; exact.len()];
    for (k, &keep) in in_lis.iter().enumerate() {
        exact_moved[k] = !keep;
    }
    // exact 序 → (old_idx, new_idx, moved) 映射回 old_idx 索引。
    let mut moved_by_old: Vec<bool> = vec![false; old.len()];
    for (k, &(i, _)) in exact.iter().enumerate() {
        moved_by_old[i] = exact_moved[k];
    }

    // --- 组装（按新侧块序；Removed 插回其旧序邻位之后）。
    let mut out: Vec<BlockDiff> = Vec::with_capacity(old.len().max(new.len()));
    let mut pair_new_old: Vec<Option<usize>> = vec![None; new.len()];
    for (i, p) in pair_old_new.iter().enumerate() {
        if let Some(j) = p {
            pair_new_old[*j] = Some(i);
        }
    }
    let mut last_old_emitted: usize = 0;
    for j in 0..new.len() {
        // 先补 Removed：本新块的配对旧块左侧未发射区（未配对旧块 =
        // 纯删除——按旧序插回）；未配对新块（Added）的插入位 = 上一
        // 配对锚之后的旧序位（upper 不回退——保持已发射区单调）。
        let upper = pair_new_old[j].unwrap_or(old.len()).min(old.len());
        for i in last_old_emitted..upper {
            if pair_old_new[i].is_none() {
                let b = &old[i];
                out.push(BlockDiff {
                    kind: DiffKind::Removed,
                    old_id: Some(b.id),
                    new_id: None,
                    old_index: Some(i),
                    new_index: None,
                    block_kind: b.kind,
                    text_old: Some(b.text.clone()),
                    text_new: None,
                });
            }
        }
        last_old_emitted = last_old_emitted.max(upper);
        let b = &new[j];
        match pair_new_old[j] {
            Some(i) => {
                let ob = &old[i];
                let kind = if pair_old_new[i] == Some(j) && same_text(&ob.text, &b.text) {
                    // 精确配对：原位 vs 移动。
                    if moved_by_old[i] {
                        DiffKind::Moved
                    } else {
                        DiffKind::Unchanged
                    }
                } else {
                    DiffKind::Modified
                };
                out.push(BlockDiff {
                    kind,
                    old_id: Some(ob.id),
                    new_id: Some(b.id),
                    old_index: Some(i),
                    new_index: Some(j),
                    block_kind: b.kind,
                    text_old: Some(ob.text.clone()),
                    text_new: Some(b.text.clone()),
                });
            }
            None => out.push(BlockDiff {
                kind: DiffKind::Added,
                old_id: None,
                new_id: Some(b.id),
                old_index: None,
                new_index: Some(j),
                block_kind: b.kind,
                text_old: None,
                text_new: Some(b.text.clone()),
            }),
        }
    }
    // 尾部剩余 Removed（新侧耗尽后的旧侧未配块）。
    for i in last_old_emitted..old.len() {
        if pair_old_new[i].is_none() {
            let b = &old[i];
            out.push(BlockDiff {
                kind: DiffKind::Removed,
                old_id: Some(b.id),
                new_id: None,
                old_index: Some(i),
                new_index: None,
                block_kind: b.kind,
                text_old: Some(b.text.clone()),
                text_new: None,
            });
        }
    }
    out
}

fn same_text(a: &str, b: &str) -> bool {
    a == b
}

/// LIS 掩码（严格递增；O(n log n) 贪心 + 二分，回填掩码）。
fn lis_mask(seq: &[usize]) -> Vec<bool> {
    // tails[k] = 长度 k+1 的递增子序列最小尾值；tail_idx[k] = 该尾的序号。
    let mut tails: Vec<usize> = Vec::new();
    let mut tail_idx: Vec<usize> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; seq.len()];
    for (k, &v) in seq.iter().enumerate() {
        let pos = tails.partition_point(|&t| t < v);
        if pos == tails.len() {
            tails.push(v);
            tail_idx.push(k);
        } else {
            tails[pos] = v;
            tail_idx[pos] = k;
        }
        prev[k] = if pos > 0 {
            Some(tail_idx[pos - 1])
        } else {
            None
        };
    }
    let mut mask = vec![false; seq.len()];
    let mut cur = tail_idx.last().copied();
    while let Some(k) = cur {
        mask[k] = true;
        cur = prev[k];
    }
    mask
}

/// 归一 Levenshtein 相似度（1 - dist/max_len），带长度差早拒。
/// 距离下界 = |len_a - len_b|（编辑距离三角不等式）——下界已低于
/// 阈值要求的直接拒（省 DP）。
fn similarity_bounded(a: &str, b: &str) -> f64 {
    let (la, lb) = (a.chars().count(), b.chars().count());
    let max_len = la.max(lb);
    if max_len == 0 {
        return 1.0; // 双空 = 全同（pass 1 已兜；防御位）
    }
    let lower_bound = la.abs_diff(lb);
    let sim_upper = 1.0 - lower_bound as f64 / max_len as f64;
    if sim_upper < SIMILARITY_THRESHOLD {
        return 0.0;
    }
    let dist = levenshtein_chars(a, b);
    1.0 - dist as f64 / max_len as f64
}

/// Levenshtein（字符面，两行 DP——O(len_a·len_b)，相似窗口界内可接受；
/// 窗口外的调用已被 align 的位置窗滤除）。
fn levenshtein_chars(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        curr[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

// ---------------------------------------------------------------------------
// L2：行内 diff（词元化 + Myers 中点蛇形分治）
// ---------------------------------------------------------------------------

/// 行内 diff：词元级（拉丁词 / CJK 单字 / 空白段 / 标点），相等/
/// 插入/删除 run 序（连续同类合并；首尾 Unchanged run 可被削去——
/// 语义不变，输出最小化）。
pub fn diff_inline(old: &str, new: &str) -> Vec<InlineDiff> {
    let a = tokenize(old);
    let b = tokenize(new);
    let script = myers_diff(&a, &b);
    // 合并同类 run。
    let mut runs: Vec<InlineDiff> = Vec::new();
    for op in script {
        let (kind, text) = match op {
            DiffOp::Equal(t) => (DiffKind::Unchanged, t),
            DiffOp::Delete(t) => (DiffKind::Removed, t),
            DiffOp::Insert(t) => (DiffKind::Added, t),
        };
        match runs.last_mut() {
            Some(last) if last.kind == kind => last.text.push_str(&text),
            _ => runs.push(InlineDiff { kind, text }),
        }
    }
    // 首尾全等 run 削除（定位面最小化——中间上下文保留可读性）。
    while matches!(runs.first(), Some(r) if r.kind == DiffKind::Unchanged && runs.len() > 1) {
        runs.remove(0);
    }
    while matches!(runs.last(), Some(r) if r.kind == DiffKind::Unchanged && runs.len() > 1) {
        runs.pop();
    }
    runs
}

/// 词元化：拉丁字母数字下划线连续段为词；CJK 逐字；空白段（含
/// 换行）一段；其余标点逐字。（词元直接产 String——块内文本量有界，
/// 分配开销可接受；比对面以 String 相等判。）
fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let c = text[i..].chars().next().unwrap();
        let clen = c.len_utf8();
        if c.is_whitespace() {
            let start = i;
            while i < text.len() {
                let ch = text[i..].chars().next().unwrap();
                if !ch.is_whitespace() {
                    break;
                }
                i += ch.len_utf8();
            }
            out.push(text[start..i].to_owned());
        } else if c.is_ascii_alphanumeric() || c == '_' {
            let start = i;
            while i < text.len() {
                let ch = text[i..].chars().next().unwrap();
                if !(ch.is_ascii_alphanumeric() || ch == '_') {
                    break;
                }
                i += ch.len_utf8();
            }
            out.push(text[start..i].to_owned());
        } else {
            // 单字词元（CJK / 标点——CJK 非拉丁字母数字面，逐字自然
            // 落到本支：中文无空格分词，字粒度使改动可定位）。
            out.push(c.to_string());
            i += clen;
        }
    }
    out
}

/// Myers 编辑脚本操作（词元串）。
#[derive(Debug, PartialEq, Eq)]
enum DiffOp {
    Equal(String),
    Delete(String),
    Insert(String),
}

/// Myers O((N+M)D) 中点蛇形分治（线性空间；词元以 String 简化——
/// 块内文本量有界，分配开销可接受）。
fn myers_diff(a: &[String], b: &[String]) -> Vec<DiffOp> {
    // 公共前缀/后缀先剥（D 界收缩——近同文本主面）。
    let mut lo = 0;
    while lo < a.len() && lo < b.len() && a[lo] == b[lo] {
        lo += 1;
    }
    let mut hi_a = a.len();
    let mut hi_b = b.len();
    while hi_a > lo && hi_b > lo && a[hi_a - 1] == b[hi_b - 1] {
        hi_a -= 1;
        hi_b -= 1;
    }
    let mut ops: Vec<DiffOp> = Vec::new();
    if lo > 0 {
        ops.push(DiffOp::Equal(a[..lo].concat()));
    }
    myers_middle(&a[lo..hi_a], &b[lo..hi_b], &mut ops);
    if hi_a < a.len() {
        ops.push(DiffOp::Equal(a[hi_a..].concat()));
    }
    normalize_hunk_order(ops)
}

/// 变更 hunk 内的删序归一：连续非 Equal 操作段内 **先删后插**（redline
/// 展示约定——删除线在前，插入在后）。中点蛇分治的子问题切分可使
/// Insert 先于 Delete 出现（两侧子 diff 的拼接序），内容不受影响——
/// 本归一化只定序，不改语义，保证输出确定性（同输入恒同输出）。
fn normalize_hunk_order(ops: Vec<DiffOp>) -> Vec<DiffOp> {
    let mut out: Vec<DiffOp> = Vec::with_capacity(ops.len());
    let mut dels: Vec<String> = Vec::new();
    let mut ins: Vec<String> = Vec::new();
    for op in ops {
        match op {
            DiffOp::Equal(t) => {
                flush_hunk(&mut out, &mut dels, &mut ins);
                out.push(DiffOp::Equal(t));
            }
            DiffOp::Delete(t) => dels.push(t),
            DiffOp::Insert(t) => ins.push(t),
        }
    }
    flush_hunk(&mut out, &mut dels, &mut ins);
    out
}

/// hunk 冲刷：段内 Delete 段合并 → Insert 段合并（各连续同类合
/// 并成一个 run——diff_inline 的 run 合并下游因此看到稳定序）。
fn flush_hunk(out: &mut Vec<DiffOp>, dels: &mut Vec<String>, ins: &mut Vec<String>) {
    if !dels.is_empty() {
        out.push(DiffOp::Delete(dels.concat()));
        dels.clear();
    }
    if !ins.is_empty() {
        out.push(DiffOp::Insert(ins.concat()));
        ins.clear();
    }
}

/// 中点蛇形分治递归体（每层剥公共前后缀；#624：剥出的后缀在本层
/// 尾部补报——见函数内注释）。
fn myers_middle(a: &[String], b: &[String], out: &mut Vec<DiffOp>) {
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        if m > 0 {
            out.push(DiffOp::Insert(b.concat()));
        }
        return;
    }
    if m == 0 {
        out.push(DiffOp::Delete(a.concat()));
        return;
    }
    // 每层递归先剥公共前缀/后缀（与 myers_diff 顶层同款）：①D 界收缩
    // ——子问题近似段主面近同；②消灭退化中点（剥后首尾必异，蛇搜索
    // 的 found 点不再与问题端点重合——(n,m)/(0,0) 形态不可达，等值
    // run 不会被防御分支吞掉）。
    let mut lo = 0;
    while lo < n && lo < m && a[lo] == b[lo] {
        lo += 1;
    }
    let mut hi_a = n;
    let mut hi_b = m;
    while hi_a > lo && hi_b > lo && a[hi_a - 1] == b[hi_b - 1] {
        hi_a -= 1;
        hi_b -= 1;
    }
    if lo > 0 {
        out.push(DiffOp::Equal(a[..lo].concat()));
    }
    // #624 修复：剥出的公共后缀必须补报——原实现剥而不报，等值
    // 词元在递归各层被静默吞掉（双向重建丢串：a 的空格词元 + b 的
    // 空格词元在 2×2 子问题的后缀剥离中同时消失）。剥壳/补报在
    // wrapper 统一收口，core 的各早退路径不背尾部。
    myers_core(&a[lo..hi_a], &b[lo..hi_b], out);
    if hi_a < n {
        out.push(DiffOp::Equal(a[hi_a..].concat()));
    }
}

/// 剥壳后的中段求解（空早退 + 单边退化收口 + 正反向中点蛇搜索）。
fn myers_core(a: &[String], b: &[String], out: &mut Vec<DiffOp>) {
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        if m > 0 {
            out.push(DiffOp::Insert(b.concat()));
        }
        return;
    }
    if m == 0 {
        out.push(DiffOp::Delete(a.concat()));
        return;
    }
    // 单边走尽（退化收口——中点蛇找不到时避免死递归）：n==1 或
    // m==1 时线性扫描找相等锚，其余为删/插（避免 O(D²) 退化路径）。
    if n == 1 || m == 1 {
        if n == 1 {
            let j = b.iter().position(|t| t == &a[0]);
            match j {
                Some(j) => {
                    if j > 0 {
                        out.push(DiffOp::Insert(b[..j].concat()));
                    }
                    out.push(DiffOp::Equal(a[0].clone()));
                    if j + 1 < m {
                        out.push(DiffOp::Insert(b[j + 1..].concat()));
                    }
                }
                None => {
                    if !b.is_empty() {
                        out.push(DiffOp::Insert(b.concat()));
                    }
                    out.push(DiffOp::Delete(a[0].clone()));
                }
            }
        } else {
            let i = a.iter().position(|t| t == &b[0]);
            match i {
                Some(i) => {
                    if i > 0 {
                        out.push(DiffOp::Delete(a[..i].concat()));
                    }
                    out.push(DiffOp::Equal(b[0].clone()));
                    if i + 1 < n {
                        out.push(DiffOp::Delete(a[i + 1..].concat()));
                    }
                }
                None => {
                    out.push(DiffOp::Delete(a.concat()));
                    out.push(DiffOp::Insert(b[0].clone()));
                }
            }
        }
        return;
    }
    // 正反向中点搜索（Myers 1986 §4b 线性空间精化）。对角线映射推导：
    // 反向阵列的 k = 反向步数对角线（x' - y'，比较面 a[n-1-x']/
    // b[m-1-y']），其在前向坐标的对应对角线 kf = delta - k（
    // (n - x') - (m - y') = delta - (x' - y')）。重叠判定统一为
    // vf[kf] + vb[kr] >= n（同对角线正反 reach 相遇/穿越）。
    let delta = n as isize - m as isize;
    let odd = delta % 2 != 0;
    // #624：k 遍历 ±d 层（d ≤ dmax = ceil((n+m)/2)+1）——数组与偏移
    // 必须容纳 k ∈ ±dmax 全程读写。原 `size = n + m + 2` / `offset =
    // size/2 + 1` 在 n+m 奇偶两形态下分别短 2/1 格（d 高层的对角线
    // idx 越界——`diff_inline("alpha beta", "报价123元")` 的
    // len9/index9 panic 即此：n+m=7 → size=9、offset=5、d=4 层 k=4 →
    // idx=9 出界）。中点蛇定理保证 found 于 d = ceil(D/2) ≤
    // ceil((n+m)/2) = dmax-1 层出现——dmax 层仅余量，但数组必须兜住。
    let dmax = (n + m).div_ceil(2) + 1;
    let size = 2 * dmax + 1;
    let offset = dmax as isize;
    let mut vf = vec![isize::MIN; size];
    let mut vb = vec![isize::MIN; size];
    vf[(offset + 1) as usize] = 0;
    vb[(offset + 1) as usize] = 0;
    let mut found: Option<(usize, usize)> = None; // (x, y) 中点
    'outer: for d in 0..=dmax as isize {
        // 正向：本层对角线 -d..=d（步 2，同层互不读写）。
        for k in (-d..=d).rev().step_by(2) {
            let idx = (offset + k) as usize;
            let mut x = if k == -d || (k != d && vf[idx - 1] < vf[idx + 1]) {
                vf[idx + 1]
            } else {
                vf[idx - 1] + 1
            };
            let mut y = x - k;
            while (x as usize) < n && (y as usize) < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            vf[idx] = x;
            // delta 奇：与反向层 d-1 的对角线比对（反向对角线
            // kr = delta - k，|kr| ≤ d-1 与奇偶自洽同价）。
            if odd && (delta - k).abs() < d {
                let bidx = (offset + (delta - k)) as usize;
                if vb[bidx] != isize::MIN && vf[idx] + vb[bidx] >= n as isize {
                    // 重叠点：正反相遇——分割点取正向 reach 终点。
                    found = Some((x as usize, y as usize));
                    break 'outer;
                }
            }
        }
        // 反向：反向序列的正向 Myers 同构（同一移动规则）；delta 偶：
        // 与正向层 d 比对（前向对角线 kf = delta - k）。
        for k in (-d..=d).rev().step_by(2) {
            let idx = (offset + k) as usize;
            let mut x = if k == -d || (k != d && vb[idx - 1] < vb[idx + 1]) {
                vb[idx + 1]
            } else {
                vb[idx - 1] + 1
            };
            let mut y = x - k;
            while (x as usize) < n
                && (y as usize) < m
                && a[n - 1 - x as usize] == b[m - 1 - y as usize]
            {
                x += 1;
                y += 1;
            }
            vb[idx] = x;
            if !odd && (delta - k).abs() <= d {
                let fidx = (offset + (delta - k)) as usize;
                if vf[fidx] != isize::MIN && vf[fidx] + vb[idx] >= n as isize {
                    found = Some((n - x as usize, m - y as usize));
                    break 'outer;
                }
            }
        }
    }
    match found {
        // 分割点严格 interior——两侧子问题严格小于原问题（进度保证，
        // 递归必终止；理论恒成立——中点蛇定理）。
        Some((mx, my)) if (mx, my) != (0, 0) && (mx, my) != (n, m) => {
            myers_middle(&a[..mx], &b[..my], out);
            myers_middle(&a[mx..], &b[my..], out);
        }
        // 防御位（未找到 / 退化零进度形态——理论不可达，兜住不挂起）：
        // 整体按删+插报告，不 panic。
        _ => {
            if !a.is_empty() {
                out.push(DiffOp::Delete(a.concat()));
            }
            if !b.is_empty() {
                out.push(DiffOp::Insert(b.concat()));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 单测（算法面；产品验收面见 tests/r29_diff_engine.rs）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_trims_trailing_noise() {
        assert_eq!(normalize_block_text("hello \r\nworld  \n"), "hello\nworld");
        assert_eq!(
            normalize_block_text("code\n  keep  indent\n\n"),
            "code\n  keep  indent"
        );
        assert_eq!(normalize_block_text(""), "");
    }

    #[test]
    fn lis_mask_basic() {
        // [3,1,2,5,4] → LIS = [1,2,4]（或 [1,2,5]——掩码取其一）。
        let mask = lis_mask(&[3, 1, 2, 5, 4]);
        let kept: Vec<usize> = [3usize, 1, 2, 5, 4]
            .iter()
            .enumerate()
            .filter(|(k, _)| mask[*k])
            .map(|(_, v)| *v)
            .collect();
        assert_eq!(kept.len(), 3, "LIS 长度 3: {kept:?}");
        assert!(kept.windows(2).all(|w| w[0] < w[1]), "严格递增: {kept:?}");
    }

    #[test]
    fn levenshtein_known_vectors() {
        assert_eq!(levenshtein_chars("", ""), 0);
        assert_eq!(levenshtein_chars("abc", "abc"), 0);
        assert_eq!(levenshtein_chars("kitten", "sitting"), 3);
        assert_eq!(levenshtein_chars("报价有效期", "报价期限"), 3);
    }

    #[test]
    fn similarity_bounds_and_values() {
        assert!(similarity_bounded("same", "same") == 1.0);
        assert_eq!(similarity_bounded("", ""), 1.0);
        // 长度差早拒：4 vs 8 字符 → 上界 0.5 = 阈值边缘，按值判。
        let sim = similarity_bounded("abcd", "abcdefgh");
        assert!((sim - 0.5).abs() < 1e-9, "{sim}");
        // 长度差超界 → 0（早拒路径）。
        assert_eq!(similarity_bounded("ab", "abcdefgh"), 0.0);
    }

    #[test]
    fn tokenize_mixed_cjk_latin() {
        let toks = tokenize("质保期 5 years！");
        assert_eq!(toks, vec!["质", "保", "期", " ", "5", " ", "years", "！"]);
    }

    #[test]
    fn myers_known_script() {
        let a: Vec<String> = tokenize("the quick brown fox");
        let b: Vec<String> = tokenize("the slow brown cat");
        let ops = myers_diff(&a, &b);
        // 首尾 equal + 中段删/插。
        let kinds: Vec<&str> = ops
            .iter()
            .map(|o| match o {
                DiffOp::Equal(_) => "=",
                DiffOp::Delete(_) => "-",
                DiffOp::Insert(_) => "+",
            })
            .collect();
        assert_eq!(kinds, vec!["=", "-", "+", "=", "-", "+"]);
        let joined: String = ops
            .iter()
            .map(|o| match o {
                DiffOp::Equal(t) | DiffOp::Delete(t) | DiffOp::Insert(t) => t.clone(),
            })
            .collect();
        assert_eq!(joined, "the quickslow brown foxcat");
    }

    #[test]
    fn myers_full_replacement_and_empty() {
        let a: Vec<String> = tokenize("aaa");
        let b: Vec<String> = tokenize("bbb");
        let ops = myers_diff(&a, &b);
        assert_eq!(
            ops,
            vec![DiffOp::Delete("aaa".into()), DiffOp::Insert("bbb".into())]
        );
        assert_eq!(myers_diff(&[], &[]), vec![]);
        let ops = myers_diff(&[], &tokenize("x y"));
        assert_eq!(ops, vec![DiffOp::Insert("x y".into())]);
    }

    #[test]
    fn inline_diff_run_merge_and_trim() {
        let runs = diff_inline("报价有效期 30 天", "报价有效期 90 天");
        // 首尾等 run 双侧削除 → 最小 hunk：删 30 / 插 90。
        assert_eq!(runs.len(), 2, "删-插（首尾等 run 削除）: {runs:?}");
        assert_eq!(
            runs[0],
            InlineDiff {
                kind: DiffKind::Removed,
                text: "30".into()
            }
        );
        assert_eq!(
            runs[1],
            InlineDiff {
                kind: DiffKind::Added,
                text: "90".into()
            }
        );
    }

    // -------------------------------------------------------------------
    // #624：Myers 中点蛇形数组边界——常见输入索引越界（先红用例）。
    // -------------------------------------------------------------------

    /// 严格双向重建不变量（`myers_diff` ops 面——diff_inline 的首尾
    /// 削除不影响此层）：Equal+Delete 序 == 旧串；Equal+Insert == 新串
    /// （tokenize 为保序连续覆盖，拼接恒等原文）。
    fn assert_rebuilds(old: &str, new: &str) {
        let ops = myers_diff(&tokenize(old), &tokenize(new));
        let (mut o, mut n) = (String::new(), String::new());
        for op in &ops {
            match op {
                DiffOp::Equal(t) => {
                    o.push_str(t);
                    n.push_str(t);
                }
                DiffOp::Delete(t) => o.push_str(t),
                DiffOp::Insert(t) => n.push_str(t),
            }
        }
        assert_eq!(o, old, "Equal+Delete 须重建旧串（{old:?} vs {new:?}）");
        assert_eq!(n, new, "Equal+Insert 须重建新串（{old:?} vs {new:?}）");
        // diff_inline 面：不 panic（首尾削除只影响展示，重建在 ops 层钉定）。
        let _ = diff_inline(old, new);
    }

    /// #624 最小确定输入（issue 报告原对）：len9/index9 越界——常见
    /// 输入（拉丁词 vs CJK/数字混排、无公共 token、delta 奇）。
    #[test]
    fn inline_diff_reported_pair_no_panic() {
        assert_rebuilds("alpha beta", "报价123元");
    }

    /// #624：100 对矩阵——常见输入形态全扫（CJK/拉丁/混排/短长不等/
    /// 空/全重写/重复词元），逐对不 panic + 严格双向重建。
    #[test]
    fn inline_diff_matrix_boundaries_rebuild() {
        let olds = [
            "alpha beta",
            "报价123元",
            "the quick brown fox",
            "合同编号：2026-001 号",
            "",
            "a",
            "一二三四五",
            "hello world 2026",
            "第一章 概述",
            "spec_v2_final.docx",
        ];
        let news = [
            "报价123元",
            "alpha beta",
            "the slow brown cat",
            "报价 123 元（含税）",
            "x",
            "abcdefghijklmnopqrst",
            "五 四 三 二 一",
            "世界你好！2026",
            "第二章 详细设计说明书",
            "spec_v3_draft (final).docx",
        ];
        for old in olds {
            for new in news {
                assert_rebuilds(old, new);
            }
        }
    }

    /// #624 验收：长文性能保持——千词元级混合面（500 CJK 条目 +
    /// 公共尾段）不 panic、双向重建成立；O((N+M)D) 中点蛇形线性空间
    /// + 对数递归深度，debug 构建实测毫秒级（慢机 CI 余量充足）。
    #[test]
    fn inline_diff_long_text_rebuild() {
        let mut old = String::new();
        let mut new = String::new();
        for i in 0..500 {
            old.push_str(&format!("第{i}条 报价单 "));
            new.push_str(&format!("第{i}项 合同 "));
        }
        old.push_str("尾部公共段 alpha beta gamma");
        new.push_str("尾部公共段 alpha beta gamma");
        assert_rebuilds(&old, &new);
    }

    /// #624：单边/空串/全重写边界（Myers 正反向蛇形走尽路径）。
    #[test]
    fn inline_diff_degenerate_boundaries() {
        assert_rebuilds("", "");
        assert_rebuilds("", "报价 123 元");
        assert_rebuilds("报价 123 元", "");
        assert_rebuilds("a", "报价123元报价123元报价123元");
        assert_rebuilds("报价123元报价123元报价123元", "a");
        assert_rebuilds("aaaa", "bbbb");
        assert_rebuilds("一词", "一词一词一词");
        // 全等：单等 run 保留（削除守卫 runs.len() > 1——全等单 run
        // 是现有产品语义，不削成空）。
        assert_eq!(
            diff_inline("same", "same"),
            vec![InlineDiff {
                kind: DiffKind::Unchanged,
                text: "same".into()
            }]
        );
    }

    // ------------------------------------------------------------------
    // #600 L3 内部件
    // ------------------------------------------------------------------

    #[test]
    fn normalize_with_map_equivalence() {
        // 与 normalize_block_text 逐字符等价（代表性向量：尾空白/CRLF/
        // 尾空行/中段空行/空文本）。
        for t in [
            "hello \r\nworld  \n",
            "code\n  keep  indent\n\n",
            "",
            "a\n\nb",
            "a\n\n\n",
            "单行尾空  ",
            "行尾\r\n混合 \r\n\n",
        ] {
            let (norm, map) = normalize_with_map(t);
            assert_eq!(norm, normalize_block_text(t), "等价面: {t:?}");
            assert_eq!(map.len(), t.chars().count() + 1, "映射长度: {t:?}");
            assert!(map.windows(2).all(|w| w[0] <= w[1]), "单调不减: {t:?}");
        }
    }

    #[test]
    fn normalize_with_map_remaps_marks() {
        // "重要  "（尾随 2 空格）→ "重要"；mark(0,4) 覆盖 "重要  " →
        // 收拢到 (0,2)。
        let (norm, map) = normalize_with_map("重要  ");
        assert_eq!(norm, "重要");
        assert_eq!((map[0], map[4]), (0, 2), "尾随被剥端点向内收拢");
        // 多行：首行尾剥使跨行坐标前移；第二行 "cd" 的 mark(4,6) 重定位
        // 到 (3,5)（'c' 在 "ab\ncd" 的 norm 索引 3）。
        let (norm, map) = normalize_with_map("ab \ncd");
        assert_eq!(norm, "ab\ncd");
        assert_eq!((map[0], map[3]), (0, 2), "首行 mark 含被剥尾空格 → 收拢");
        assert_eq!((map[4], map[6]), (3, 5), "第二行 mark 平移一位");
    }

    #[test]
    fn span_text_char_slicing() {
        // char 坐标（非字节）：emoji（👍🏻 = 基字符+肤色修饰 = 2 char）+ CJK。
        let text = "👍🏻重要 事项";
        assert_eq!(text.chars().count(), 7, "char 计数基线");
        assert_eq!(span_text(text, 0, 2), "👍🏻".to_string());
        assert_eq!(span_text(text, 2, 4), "重要".to_string());
        // 越界端点自然截断（不 panic）。
        assert_eq!(span_text(text, 3, 99), "要 事项".to_string());
        assert_eq!(span_text(text, 99, 199), "");
        // 空区间。
        assert_eq!(span_text(text, 2, 2), "");
    }

    #[test]
    fn mark_changes_swap_and_dedup() {
        use crate::tree::MarkSpan;
        let mk = |kind: &str, s: usize, e: usize| MarkSpan {
            start: s,
            end: e,
            kind: kind.into(),
        };
        // 「重要 重要」互换：old(0,2) → new(3,5) 报 Moved（位置可观察）。
        let text = "重要 重要";
        let ev = mark_changes_between(&[mk("em", 0, 2)], &[mk("em", 3, 5)], text);
        assert_eq!(
            ev,
            vec![MarkChange {
                kind: "em".into(),
                change: MarkChangeKind::Moved,
                old_range: Some((0, 2)),
                new_range: Some((3, 5)),
                text: "重要".into(),
            }]
        );
        // 两处相同样式删一处：精确位互消 + 剩余 Removed（只报对应变化）。
        let ev = mark_changes_between(&[mk("em", 0, 2), mk("em", 3, 5)], &[mk("em", 0, 2)], text);
        assert_eq!(
            ev,
            vec![MarkChange {
                kind: "em".into(),
                change: MarkChangeKind::Removed,
                old_range: Some((3, 5)),
                new_range: None,
                text: "重要".into(),
            }]
        );
        // 双侧完全相同 → 零噪声。
        assert!(mark_changes_between(
            &[mk("em", 0, 2), mk("strong", 3, 5)],
            &[mk("strong", 3, 5), mk("em", 0, 2)],
            text
        )
        .is_empty());
        // 重叠 marks 逐条独立比较（不 panic 不合并）。
        let ev = mark_changes_between(
            &[mk("em", 0, 4), mk("strong", 2, 6)],
            &[mk("em", 0, 4), mk("strong", 2, 6)],
            "abcd ef",
        );
        assert!(ev.is_empty(), "重叠双侧等价 → 零事件");
        // 空区间参与比较（不静默剔除）。
        let ev = mark_changes_between(&[], &[mk("em", 2, 2)], "ab");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].change, MarkChangeKind::Added);
        assert_eq!(ev[0].text, "");
    }

    #[test]
    fn effective_inline_hostile_marks_clamped_not_panicking() {
        use crate::tree::MarkSpan;
        // 敌意 marks（越界 end / 倒挂 start>end）——op 面防御：钳制、
        // 剔除倒挂，不 panic。
        let (net, marks) = effective_inline(
            BlockKind::Paragraph,
            "重要",
            &[
                MarkSpan {
                    start: 0,
                    end: 99, // 越界
                    kind: "bold".into(),
                },
                MarkSpan {
                    start: 5,
                    end: 1, // 倒挂
                    kind: "em".into(),
                },
            ],
        );
        assert_eq!(net, "重要");
        assert_eq!(marks.len(), 1, "倒挂剔除、越界保留");
        assert_eq!((marks[0].start, marks[0].end), (0, 2), "越界端点钳制");
        // 字面块：marks 恒空（代码/数学/表格非投影面）。
        for kind in [BlockKind::CodeBlock, BlockKind::Math, BlockKind::Table] {
            let (net, marks) = effective_inline(
                kind,
                "a `x` **b** |",
                &[MarkSpan {
                    start: 0,
                    end: 1,
                    kind: "em".into(),
                }],
            );
            assert!(marks.is_empty(), "{kind:?} 非投影面");
            assert_eq!(net, "a `x` **b** |", "{kind:?} 逐字保留");
        }
    }

}
