//! #414 —— MD 文档 diff 引擎：块级对齐（L1）+ 行内 diff（L2）。
//!
//! ## 定位：redline 与「改了哪里」的结构基座
//!
//! mang 此前没有任何文档比较能力。本模块提供两层：
//!
//! - **L1 块级对齐** [`diff_documents`] / [`diff_markdown`]：两版文档
//!   的块序列对齐——新增/删除/移动/修改/未变 五分类（[`BlockDiff`]）；
//! - **L2 行内 diff** [`diff_inline`]：修改块内的文字级定位
//!   （[`InlineDiff`] run 序）。
//!
//! 与 #413（DOCX 修订写面）组成 redline 链：L1 产 Added/Removed/
//! Modified 块 → #413 `insert_run_revised`/`delete_run_revised` 落成
//! Word 修订标记；Modified 块经 L2 定位到片段。
//!
//! ## 对齐策略（issue 三步——按 #415 现状裁剪）
//!
//! issue 原文「① 按 block_id 匹配（如果稳定）② 按内容相似度 ③ 纯
//! 增删」。**#415 在档：块 ID 重编号撞号缺陷（P0-1）未修，ID 当前
//! 不是稳定锚**——独立 parse 的两版文档 ID 空间各自从 1 发号（位置
//! 序），按 ID 对齐会把无关块配成对（错锚）。因此 v0 **不作 ID 锚**
//! （ID 仅在 [`BlockDiff::old_id`]/[`BlockDiff::new_id`] 报告，供
//! 消费方展示/追踪），对齐全走内容面：
//!
//! 1. **精确匹配**：`(BlockKind, 归一化文本)` 签名——行尾空白与
//!    末行换行归一（重排序列化不产生伪差），代码块/公式内部逐字
//!    （空白敏感内容不做更深的归一）；
//! 2. **相似度匹配**：同 kind 未配对块，位置窗口内按归一 Levenshtein
//!    相似度贪心配对（阈值 [`SIMILARITY_THRESHOLD`]，窗口
//!    [`SIMILARITY_WINDOW`] 界住平方面）；
//! 3. **剩余**：旧侧未配 = Removed，新侧未配 = Added；
//! 4. **Moved 判定**：精确配对做 LIS（最长递增子序列，O(n log n)）
//!    ——LIS 内 = Unchanged（原位）、LIS 外 = Moved（纯移动 ≠ 删除
//!    + 新增）。相似配对（Modified）不参与 Moved 二分：
//!    - 内容变化为主标签（Modified）；
//!    - 位置变化可由 old_index/new_index 观察（如实不折叠）。
//!
//! #415 修复后可加「ID 先行匹配」作为第 0 步（同谱系文档——保存档
//! vs 编辑档——ID 是最强锚），本模块的对齐管线已按可插拔步序组织。
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
pub fn diff_documents(old: &Core, new: &Core) -> Vec<BlockDiff> {
    let old_blocks = flatten(old);
    let new_blocks = flatten(new);
    align(&old_blocks, &new_blocks)
}

/// MD 文本直 diff（`from_markdown` 双侧 + [`diff_documents`]）。
pub fn diff_markdown(old_md: &str, new_md: &str) -> Vec<BlockDiff> {
    let old = Core::from_markdown(old_md);
    let new = Core::from_markdown(new_md);
    diff_documents(&old, &new)
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

/// 对齐主体（纯函数——旧/新扁平块 → diff 序）。
fn align(old: &[FlatBlock], new: &[FlatBlock]) -> Vec<BlockDiff> {
    // 配对表：old_idx → new_idx（精确 + 相似两 pass 填充）。
    let mut pair_old_new: Vec<Option<usize>> = vec![None; old.len()];
    let mut paired_new = vec![false; new.len()];

    // --- pass 1：精确签名匹配（同内容重复块按先到先配——多副本
    // 场景（同文重复段）按序配对，语义稳定可预测）。签名键 =
    // (kind 判别值, 归一文本)——BlockKind 无 Hash derive（树模块
    // 原有 derive 面不为本模块扩），无字段枚举 `as u8` 即稳定判别。
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
    let mut exact: Vec<(usize, usize)> = Vec::new();
    for (i, b) in old.iter().enumerate() {
        if let Some(queue) = sig_map.get_mut(&(b.kind as u8, b.text.as_str())) {
            if let Some(j) = queue.pop_front() {
                pair_old_new[i] = Some(j);
                paired_new[j] = true;
                exact.push((i, j));
            }
        }
    }

    // --- pass 2：相似度匹配（同 kind 未配对，位置窗口 + 阈值，按
    // 相似度降序贪心取不冲突对）。
    let unmatched_old: Vec<usize> = (0..old.len())
        .filter(|&i| pair_old_new[i].is_none())
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

/// 中点蛇形分治递归体。
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
    let a = &a[lo..hi_a];
    let b = &b[lo..hi_b];
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
    let size = n + m + 2;
    let offset = (size / 2 + 1) as isize;
    let mut vf = vec![isize::MIN; size];
    let mut vb = vec![isize::MIN; size];
    vf[(offset + 1) as usize] = 0;
    vb[(offset + 1) as usize] = 0;
    let mut found: Option<(usize, usize)> = None; // (x, y) 中点
    let dmax = (n + m).div_ceil(2) + 1;
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
}
