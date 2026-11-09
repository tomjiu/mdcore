//! #617 —— DOCX TOC 自动重算（读面派生 + 稳定名 + 页码缓存语义）。
//!
//! ## 定位：静态 TOC 的重算对偶面
//!
//! #196 任务 2 交付了静态目录项发射（[`crate::docx::DocxWriter::toc_entries`]
//! ——三段式跨段域 + `toc_N` 书签 + 跳转闭环），但目录内容一经生成就
//! 固定：标题增删/层级移动/文本变更后目录不随内容更新。本模块补
//! **重算面**：从文档当前标题树重新派生目录项集，差分旧目录项，并
//! 产出**内容寻址稳定书签名**——重算后同标题同名，跳转配对不漂移。
//!
//! ```ignore
//! // 读 → 重算 →（产品面接线时）重写发射：
//! let mut ed = DocxEditor::open(&bytes)?;
//! let plan = ed.recompute_toc(None)?;
//! assert_eq!(plan.entries.last().unwrap().text, "新增的第四章");
//!
//! // 写面：稳定名发射（PAGEREF 缓存保留 + 无缓存 → "—" 显式未知）
//! let names = w.toc_entries_recomputed(&plan, None);
//! // 调用方在对应标题处 heading_bookmarked(level, text, &names[i])
//! ```
//!
//! ## 页码语义（显式化——不产假页码）
//!
//! 不排版不知真页码（真页码面属 #309 排版路线 M1-M4，本模块明确排除）：
//!
//! - 旧目录项携带的 PAGEREF 缓存值**保留**（`TocRecompute::cached_pages`
//!   逐项 `Some(缓存文本)`）；
//! - 无旧缓存（新标题 / 无旧 TOC / sdt 盲区）→ `None` = **待重算标记**，
//!   写面发射落 `"—"` 显式未知——不沿用静态面的 `"1"` 占位（那是
//!   假页码；本面从数据上拒绝）。
//!
//! ## 书签保真（#627——存续标题复用旧锚，新项内容寻址避让）
//!
//! [`stable_bookmark_name`]：`fnv1a64(level, text)` → `toc_h{7 hex}`。
//! 同内容（层级+文本）→ 同名；`_` 后首字符 `h` 与 `toc_N` 计数器名域
//! 零碰撞；V1-V6 域全过；同文重复标题 `_2`/`_3` 后缀去重。
//!
//! **#627 语义**：重算时**先配对后分配**——存续标题按**当前文本**
//! 多重集配对已有 `toc_N` 锚（段落文本 pending 感知——改文/改层级
//! 后的标题仍配自己的旧锚，书签节点在段落树手术中原样保留），
//! 配对成功者复用旧名（旧引用不悬空）；只有新标题（无同文旧锚）
//! 才分配内容寻址名，且 `used` 预置文档全部已存在书签名（含用户
//! 书签）——新名零碰撞。Word `_Toc…` 锚因 V5 保留名不可再发射——
//! 复用域不含，悬空为已登记局限。
//!
//! ## 已登记局限（诚实降级，不静默）
//!
//! - **表内标题不可见**：读模型 `TableData::rows` 已扁平化为字符串
//!   （cell 段不承载 `ParagraphData`）——表内标题段不入重算标题树。
//! - **sdt 包裹 TOC 的旧缓存**：编辑面原文坐标可见（`in_sdt` 段在
//!   `paragraphs()` 扁平序内），本面经编辑面提取故**不受** docx-rs 读
//!   模型对 sdt 容器的结构性丢弃影响；但 sdt 包裹的**标题段**在读模型
//!   不可见（同源盲区）——该形态下标题树可能少项。
//! - **差分按文本多重集**：文本编辑（B→B'）表现为 removed(B) +
//!   added(B')（文本身份语义——无独立 changed 计数，不发明配对）；
//!   `level_changed` 按同文本首现层级比对。
//!
//! ## 与 issue 验收的对应
//!
//! - 重算后目录项与标题树一一对应 → `TocRecompute::entries`（读面
//!   单一真源 `ParagraphData::heading_level` 派生）；
//! - PAGEREF 缓存保留 + 待重算标记 → `cached_pages` / 写面 `"—"`;
//! - `toc_N` 稳定断言 → 内容寻址名（同标题跨两次重算同名）；
//! - t8b / #196 零回归 → [`crate::docx::DocxWriter::toc_entries`]
//!   发射路径不动（共享发射肋提取为私有单点，字节面等价）。
//!
//! 产品接线（mang-shell / web 触发入口）明确不在本面——另拆。

use std::collections::HashSet;

use crate::docx::{Block, DocxDocument, TocEntry};

// ---------------------------------------------------------------------------
// 公开类型
// ---------------------------------------------------------------------------

/// #617 重算产物：新目录项集 + 稳定书签名 + 页码缓存语义 + 差分诊断。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocRecompute {
    /// 新目录项集（文档序——标题树派生，`heading_range` 过滤后）。
    pub entries: Vec<TocEntry>,
    /// 与 `entries` 等长：内容寻址稳定书签名（`toc_h<hex>`；同文重复
    /// 标题 `_2`/`_3` 去重）。写面经
    /// [`crate::docx::DocxWriter::toc_entries_recomputed`] 消费；调用方
    /// 在对应标题处 `heading_bookmarked` 同名注册构成跳转闭环。
    pub names: Vec<String>,
    /// 与 `entries` 等长：旧目录项携带的 PAGEREF 缓存值（按文本配对
    /// ——`None` = 无旧缓存 = **待重算标记**；写面落 `"—"` 显式未知）。
    pub cached_pages: Vec<Option<String>>,
    /// 旧目录域在场且被解析（false = 无 TOC 域 / 退化占位形态——
    /// 全量待重算）。
    pub old_toc_present: bool,
    /// 重算差分诊断（四场景证据：增/删/层级变更；文本编辑表现为
    /// removed+added 对——文本身份语义）。
    pub diff: TocDiff,
}

/// 重算差分（按文本多重集比对——见模块文档「已登记局限」）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TocDiff {
    /// 新增标题的文本（旧目录无、新标题树有）。
    pub added: Vec<String>,
    /// 移除标题的文本（旧目录有、新标题树无）。文本编辑（B→B'）
    /// 表现为 removed(B) + added(B')——文本身份语义。
    pub removed: Vec<String>,
    /// 同文本层级变更数。**恒 0**：旧目录项不承载层级（层级只落在
    /// 发射缩进，编辑面文本提取不回提）——不可诚实计算（见
    /// [`diff_and_cache`] 登记）；层级移动场景经「重算后 entries
    /// 层级正确」断言覆盖（验收主判据）。
    pub level_changed: usize,
}

// ---------------------------------------------------------------------------
// 纯函数（pub(crate)——docx.rs / docx_edit.rs 消费）
// ---------------------------------------------------------------------------

/// FNV-1a 64 位（内容寻址名的哈希基——无依赖自实现，稳定跨版本）。
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 内容寻址稳定书签名：`toc_h{fnv1a64(level,text) 截 7 hex}`。
///
/// 域安全（V1-V6）：非空 ✓ 首字符 `t` 字母 ✓ `[a-z0-9_]` ✓ 非
/// `_Toc` 前缀 ✓ 长度 11（去重后 ≤14）✓。`used` 内撞名（同文重复
/// 标题）→ `_2`/`_3`… 后缀递增（先现者得基名）。与 `toc_N` 计数器
/// 名域零碰撞（`_` 后 `h` vs 纯数字）。
pub(crate) fn stable_bookmark_name(level: u8, text: &str, used: &mut HashSet<String>) -> String {
    let mut key = Vec::with_capacity(1 + text.len());
    key.push(level);
    key.push(0x1f); // 层级/文本分隔符——防 (1,"a2") 与 (12,"a") 混叠
    key.extend_from_slice(text.as_bytes());
    let hex = fnv1a64(&key) & 0x0fff_ffff;
    let base = format!("toc_h{hex:07x}");
    if used.insert(base.clone()) {
        return base;
    }
    for n in 2..=99u32 {
        let cand = format!("{base}_{n}");
        if used.insert(cand.clone()) {
            return cand;
        }
    }
    // 99 个同文重复标题（极端面）——顺序号兜底（域内、唯一）。
    let mut n = 100u32;
    loop {
        let cand = format!("{base}_{n}");
        if used.insert(cand.clone()) {
            return cand;
        }
        n += 1;
    }
}

/// 旧目录项段解析：`"[\t…]标题文本\t缓存页"` → `(标题文本, 缓存页)`。
///
/// 前导 `\t` 剩离：编辑面文本装配把段落属性里的 **tab stop 定义**
/// （`<w:tabs>/<w:tab w:val=… w:pos=…>`，pPr 层）也与 run 级真 tab
/// 同样拼为 `\t`——发射形态的目录项段因此带一个前导制表符
/// （`"\t标题\t1"`）。剩离所有前导 `\t` 后取末段为缓存值
/// （标题文本自带 `\t` 时缓存值取末段，中段归文本）。无 `\t` 或
/// 剩离后文本空 → None（域头/域尾/占位段——非目录项形态）。
pub(crate) fn parse_old_toc_entry(text: &str) -> Option<(&str, &str)> {
    let (entry, cached) = text.rsplit_once('\t')?;
    let entry = entry.trim_start_matches('\t');
    if entry.is_empty() {
        return None;
    }
    Some((entry, cached))
}

/// 标题树派生目录项集（读模型单一真源——`ParagraphData::heading_level`）。
///
/// 走 `blocks` 顶层段落 + 文本框内块递归（表内标题为已登记局限——
/// `TableData::rows` 扁平化，cell 段不承载标题层级）。`range` =
/// `Some((lo, hi))` 过滤 1-9 域层级（V8 同域）。
pub(crate) fn heading_entries(doc: &DocxDocument, range: Option<(u8, u8)>) -> Vec<TocEntry> {
    let mut out = Vec::new();
    fn walk(blocks: &[Block], range: Option<(u8, u8)>, out: &mut Vec<TocEntry>) {
        for b in blocks {
            match b {
                Block::Paragraph(p) => {
                    if let Some(level) = p.heading_level {
                        let lvl = level.min(9) as u8;
                        if let Some((lo, hi)) = range {
                            if lvl < lo || lvl > hi {
                                continue;
                            }
                        }
                        out.push(TocEntry::new(lvl, p.text()));
                    }
                }
                // 文本框内块与正文同构（#18）——递归承载标题。
                Block::TextBox(tb) => walk(&tb.blocks, range, out),
                // 表内标题：读面已登记的扁平化盲区（模块文档）——跳过。
                Block::Table(_) => {}
            }
        }
    }
    walk(&doc.blocks, range, &mut out);
    out
}

/// 差分 + 缓存配对（按文本多重集——见模块文档）。
///
/// 返回 `(cached_pages, diff)`：`cached_pages` 与 `new_entries` 等长
/// （旧目录同文本未消费项的缓存；无配对 → None）。`added`/`removed`
/// **排序后输出**（多重集差的确定性面——HashMap 迭代序不可依赖）。
/// 无基线（`old_toc_present = false`）语义由调用方处理（全空差分）。
pub(crate) fn diff_and_cache(
    new_entries: &[TocEntry],
    old: &[(String, String)],
) -> (Vec<Option<String>>, TocDiff) {
    use std::collections::HashMap;
    // 旧项按文本建立多重消费索引：text → 未消费缓存池（文档序）。
    let mut old_pool: HashMap<&str, Vec<&str>> = HashMap::new();
    for (text, cached) in old {
        old_pool
            .entry(text.as_str())
            .or_default()
            .push(cached.as_str());
    }
    let mut cached_pages = Vec::with_capacity(new_entries.len());
    let mut new_counts: HashMap<&str, u64> = HashMap::new();
    for e in new_entries {
        let t = e.text.as_str();
        let cached = old_pool
            .get_mut(t)
            .and_then(|pool| {
                if pool.is_empty() {
                    None
                } else {
                    Some(pool.remove(0))
                }
            })
            .map(str::to_string);
        cached_pages.push(cached);
        *new_counts.entry(t).or_insert(0) += 1;
    }
    // 旧侧计数（多重集差分基准）。
    let mut old_counts: HashMap<&str, u64> = HashMap::new();
    for (text, _) in old {
        *old_counts.entry(text.as_str()).or_insert(0) += 1;
    }
    // added / removed（多重集差——排序保证确定性）。
    let mut added = Vec::new();
    for (t, cnt) in &new_counts {
        let remain = cnt.saturating_sub(old_counts.get(t).copied().unwrap_or(0));
        for _ in 0..remain {
            added.push((*t).to_string());
        }
    }
    let mut removed = Vec::new();
    for (t, cnt) in &old_counts {
        let remain = cnt.saturating_sub(new_counts.get(t).copied().unwrap_or(0));
        for _ in 0..remain {
            removed.push((*t).to_string());
        }
    }
    added.sort();
    removed.sort();
    // level_changed：旧目录项不承载层级（层级只落在发射缩进，编辑面
    // 文本提取不回提）——本面不可诚实计算，恒 0。层级移动场景经
    // 「重算后 entries 层级正确」断言覆盖（entries 反映当前标题树
    // 层级——验收主判据）；产品接线若需差分级层级比对，扩参提供
    // 旧层级清单再算（登记，不发明数据）。
    let diff = TocDiff {
        added,
        removed,
        level_changed: 0,
    };
    (cached_pages, diff)
}

// ---------------------------------------------------------------------------
// #627 —— 书签保真配对（存续标题复用旧锚名）
// ---------------------------------------------------------------------------

/// 引擎 TOC 书签模式判定——引擎两种发射形的精确名域：
///
/// - 静态面 [`crate::docx::DocxWriter::toc_entries`]：`toc_` + 纯数字
///   （计数器名 `toc_12`）；
/// - 重算面 [`stable_bookmark_name`]：`toc_h` + 恰 7 位小写 hex
///   （28 位掩码），可选 `_N` 去重后缀（`toc_h8e11da8` /
///   `toc_h8e11da8_2`）。
///
/// 只复用引擎域的原因：Word `_Toc…` 前缀是写面 V5 保留名（不可再
/// 发射——复用会在 save 漏斗拒收）；用户书签（含 `toc_hello` 等
/// 任意 `toc_` 前缀但非引擎形态的名）不属 TOC 机制——不消费，仅
/// 避让。旧 `_Toc` 锚在重算后悬空为已登记局限（V5 域约束）。
pub(crate) fn is_engine_toc_bookmark(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("toc_") else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    // 静态形：纯数字。
    if rest.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    // 重算形：h + 7 位小写 hex（可选 _N 数字后缀）。
    let Some(hex_part) = rest.strip_prefix('h') else {
        return false;
    };
    let hex = match hex_part.split_once('_') {
        Some((h, suffix)) => {
            if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_digit()) {
                return false;
            }
            h
        }
        None => hex_part,
    };
    hex.len() == 7
        && hex
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// 书签复用配对（#627 存续标题保真）：新目录项按**当前文本**多重集、
/// 文档序配对同文本的未消费 TOC 书签——配对成功者**复用旧锚名**。
///
/// 配对键说明：
/// - `pool` 项为 `(书签名, 所在段落当前文本)`——段落文本是 pending
///   感知的（编辑后的标题文本即时反映），因此**改文/改层级的存续
///   标题仍能配上自己的旧锚**（书签节点在段落树手术中原样保留——
///   见 `process_children`「其余面：原样」登记）：这是段落身份语义
///   的文本投影，非单纯文本匹配；
/// - 同文重复标题按文档序先现先配（确定性——重复面锚身份天然歧义，
///   顺序规则钉死）；
/// - 未配对项（新标题 / 重复溢出）→ `None`（调用方分配内容寻址新名，
///   `used` 预置全部已存在书签名后避让）。
pub(crate) fn reuse_bookmark_names(
    entries: &[TocEntry],
    pool: &[(String, String)],
) -> Vec<Option<String>> {
    // 文本 → 未消费书签池（文档序）。
    let mut by_text: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    for (name, para_text) in pool {
        by_text
            .entry(para_text.as_str())
            .or_default()
            .push(name.as_str());
    }
    entries
        .iter()
        .map(|e| {
            by_text
                .get_mut(e.text.as_str())
                .and_then(|names| {
                    if names.is_empty() {
                        None
                    } else {
                        Some(names.remove(0))
                    }
                })
                .map(str::to_string)
        })
        .collect()
}
