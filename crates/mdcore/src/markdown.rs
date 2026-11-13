//! Markdown <-> Block Tree（解析边界；spike markdown.ts 的树化升级）。
//!
//! v0 规则（刻意保持最小，偏差已登记 README）：
//! - 行级解析：fence 聚合多行；`|` 开头连续行合并为 table；其余一行一块。
//! - 数学块（#261）：`$$` 定界——单独 `$$` 行 fence 同构聚合（开行起、收行止）；
//!   `$$x$$` 同行自含形态（行首尾皆 `$$` 且内容非空）；其余含 `$` 行不猜
//!   （段落，行内 `$...$` 由 [`crate::inline`] 消费时派生，不进块级）。
//! - 嵌套：list_item / blockquote 参与深度栈建树；quote 同深度连续行
//!   复用同一容器（children = 段落）；paragraph/heading/fence/math/thematic 落根层。
//! - 序列化：list 祖先贡献 2 空格缩进，quote 祖先贡献 "> " 前缀；
//!   marks 不写入 MD（spike 同行为）；ID 不写入 MD（§8.3）。
//! - round-trip 保证方向：parse → serialize → parse 结构等价（canonical 输入；
//!   数学块恒规范化为 fence 形态，单行形态序列化后不再保留——结构等价口径）。
//!
//! #163 围栏/标题单源化：本模块的行识别原语（[`fence_info`] / [`heading_level`]）
//! 是**全仓唯一**实现——mang-shell 编辑器（highlight/fence/mod 的渲染面）与
//! 本解析器共用同一函数，消灭「三处围栏实现漂移」（audit/notes/22 §⑦）。
//! 围栏语义对齐编辑器现行渲染（``` 或 ~~~、≥3 字符、前导空白 ≤3、
//! 任意标记行闭栏）——CommonMark 严格同字符闭栏留给 P1 pulldown-cmark 评估。

use crate::tree::{Attrs, BlockKind, BlockTree, NodeId};

// ---------- #163 行识别原语（单源；编辑器渲染面与解析器共用） ----------

/// 围栏标记行的识别结果（#163 单源原语）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FenceInfo {
    /// 标记结束的 char 位置（前导空白 + 围栏字符数）。
    pub marker_end: usize,
    /// 围栏字符（`` ` `` 或 `~`）。
    pub fence_char: char,
    /// info string 首词（语言标注；未标注 = 空串）。
    pub lang: String,
}

/// 围栏标记行形态识别：`` ``` `` 或 `~~~`（前导空白 ≤ 3、围栏字符 ≥ 3）。
///
/// #163 单源：此前解析器只认行首 `` ``` ``（无前导空白、无 ~~~、lang 取
/// 整段 info），而 mang-shell 渲染面认 `` ``` ``/`~~~` + ≤3 前导空白——
/// 「   ```rust」在解析器是段落、在渲染器是围栏（漂移实锤）。本函数
/// 以渲染面语义为准唯一化（解析器 lang 收窄为首词——`` ```rust, foo``
/// 的 lang 从 "rust, foo" 收窄为 "rust"，与编辑器 fence_lang_of 同口径）。
/// 闭栏判定 = 任意标记行（与编辑器 FenceCursor 的翻转语义一致；
/// CommonMark 同字符闭栏为 P1 项）。info string 含围栏字符的行
/// （` ``` ``` `）非围栏（CommonMark 同规——external_corpus spec#137
/// 幂等根因）。
pub fn fence_info(line: &str) -> Option<FenceInfo> {
    let mut chars = line.char_indices().peekable();
    let mut lead = 0usize;
    while let Some(&(_, ' ')) = chars.peek() {
        if lead >= 3 {
            break;
        }
        chars.next();
        lead += 1;
    }
    let &(_, c) = chars.peek()?;
    if c != '`' && c != '~' {
        return None;
    }
    let mut count = 0usize;
    let mut marker_end = lead;
    for (pos, ch) in chars {
        if ch == c {
            count += 1;
            marker_end = pos + ch.len_utf8();
        } else {
            break;
        }
    }
    if count < 3 {
        return None;
    }
    let info = line[marker_end..].trim();
    if info.contains(c) {
        return None;
    }
    let lang = info
        .split(|d: char| d == ',' || d.is_whitespace())
        .next()
        .unwrap_or("")
        .to_string();
    Some(FenceInfo {
        marker_end,
        fence_char: c,
        lang,
    })
}

/// ATX 标题级别识别（#163 单源原语）：`#` × 1..=6 + 行尾（空标题）或后随空格。
///
/// 与 mang-shell 旧 `heading_level_of` 语义逐位一致（"#" 独占行 = 空标题，
/// CommonMark 同规）；"#x"（无空格）非标题。解析器 [`heading_of`] 与编辑器
/// 渲染面均经由本函数——标题判定单源。
pub fn heading_level(line: &str) -> Option<usize> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = line.chars().nth(hashes);
    match rest {
        None => Some(hashes),
        Some(' ') => Some(hashes),
        _ => None,
    }
}

// ---------- 渐进打开的 chunk 完整性（R10-a） ----------

/// chunk 是否收在块边界上（无悬挂状态）。与 `markdown_to_tree` 的 fence/math
/// 判定同规（状态机互斥：fence 内的 `$$` 行与 math 内的 ``` 行不翻对面状态——
/// 两类围栏内文都是逐字面，不猜）；`$$x$$` 同行自含形态无悬挂态。
/// 悬挂 = 扫描结束时任一围栏仍开启（闭合收在下一个 chunk）→ 调用方必须
/// 重切边界；核内 `extend_from_markdown` 据此返回 Reject::IncompleteChunk。
/// 其余块型（段落/列表/quote/table）单行自含，无悬挂态可检测。
pub fn is_complete_chunk(md: &str) -> bool {
    let mut fence_open = false;
    let mut math_open = false;
    for line in md.split('\n') {
        if fence_open {
            // #163：闭栏判定与解析器/渲染面同源（fence_info——任意标记行
            // 闭栏；此前行首 `` ``` `` 硬前缀，与渲染面漂移）。
            if fence_info(line).is_some() {
                fence_open = false;
            }
        } else if math_open {
            if line.trim() == "$$" {
                math_open = false;
            }
        } else if fence_info(line).is_some() {
            fence_open = true;
        } else if line.trim() == "$$" {
            math_open = true;
        }
    }
    !fence_open && !math_open
}

// ---------- 序列化 ----------

pub fn tree_to_markdown(tree: &BlockTree) -> String {
    let mut lines: Vec<String> = Vec::new();
    for &child in tree.top_level() {
        render(tree, child, "", "", &mut lines);
    }
    lines.join("\n")
}

/// 单子树序列化（#163：mang-shell undo 翻译消费——InsertBlock 的逆 op
/// 重建投影行时，需要把插入的子树渲染回 markdown 行）。
/// 与 [`tree_to_markdown`] 共用 [`render`]——单源，零漂移。
pub fn render_subtree_markdown(tree: &BlockTree, id: NodeId) -> String {
    let mut lines: Vec<String> = Vec::new();
    render(tree, id, "", "", &mut lines);
    lines.join("\n")
}

fn render(
    tree: &BlockTree,
    id: NodeId,
    list_indent: &str,
    quote_prefix: &str,
    out: &mut Vec<String>,
) {
    let node = tree.node(id);
    let emit = |content: &str, out: &mut Vec<String>| {
        for line in content.split('\n') {
            out.push(format!("{quote_prefix}{list_indent}{line}"));
        }
    };
    match node.kind {
        BlockKind::Document => {}
        BlockKind::Paragraph | BlockKind::Table => emit(&node.text.to_string(), out),
        BlockKind::Heading => {
            let level: usize = node
                .attrs
                .get("level")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            let hashes = "#".repeat(level.clamp(1, 6));
            emit(&format!("{hashes} {}", node.text), out);
        }
        BlockKind::CodeBlock => {
            let lang = node.attrs.get("lang").cloned().unwrap_or_default();
            emit(&format!("```{lang}"), out);
            emit(&node.text.to_string(), out);
            emit("```", out);
        }
        // 数学块恒规范化为 fence 形态（单行自含形态序列化后不再保留——
        // round-trip 结构等价口径，见模块文档）。
        BlockKind::Math => {
            emit("$$", out);
            emit(&node.text.to_string(), out);
            emit("$$", out);
        }
        BlockKind::ThematicBreak => emit("---", out),
        BlockKind::ListItem => {
            let marker = node
                .attrs
                .get("marker")
                .cloned()
                .unwrap_or_else(|| "-".into());
            let head = format!("{quote_prefix}{list_indent}{marker} {}", node.text);
            for (i, line) in head.split('\n').enumerate() {
                if i == 0 {
                    out.push(line.to_string());
                } else {
                    out.push(format!("{quote_prefix}{list_indent}  {line}"));
                }
            }
            for &c in &node.children {
                render(tree, c, &format!("{list_indent}  "), quote_prefix, out);
            }
        }
        BlockKind::Blockquote => {
            if node.children.is_empty() {
                emit(&node.text.to_string(), out);
            }
            for &c in &node.children {
                render(tree, c, list_indent, &format!("{quote_prefix}> "), out);
            }
        }
    }
}

// ---------- 解析 ----------

/// 嵌套深度门（#278 审计 B-4）：与 [`crate::html`] 侧同常数同语义
/// （html.rs MAX_DEPTH=256，超深跳过子树）。此前 markdown 建树无
/// 深度上限，同仓双标；敌意逐行递进（`>` / `>>` / `>>>` …）或缩进
/// 列表递进可造万层深树，blocks_json/to_markdown/word_count 等递归
/// 消费者栈溢出（wasm = trap / 原生 = 崩溃）。行深度（quote 的 >
/// 计数 / list 的缩进层号）即嵌套容器层编号（1 基）：拦
/// depth ≥ MAX_DEPTH = 容器层封顶 255（quote 内容段落叶子 +1）
/// ——树深硬顶 256，与 html 侧树深上限同构。
const MAX_DEPTH: usize = 256;

// ---------- #163 行映射（解析器单源产出；mang-shell 换芯桥消费） ----------

/// 解析行的快路径分类（编辑器守卫消费——「编辑后的行还能不能按原块
/// 语义原位改」的判定依据；由 [`parse_with_lines`] 随树一起产出）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotClass {
    /// 空行（无块——首字符编辑走 InsertBlock 快路径）。
    Blank,
    /// 标记独占行（围栏开/闭、分隔线、数学开/闭）——无块文本可映射，
    /// 永不走原位快路径。
    MarkerOnly,
    /// 深度门丢弃行（#278——解析器不建块；编辑恒兜底）。
    Dropped,
    Paragraph,
    Heading(u32),
    /// 代码围栏体行（守卫：结果行不得为围栏标记行）。
    BodyCode,
    /// 数学块体行（守卫：结果行 trim 后不得为 `$$`）。
    BodyMath,
    TableRow,
    /// 引用行（参数 = 深度；行内容映射到 quote 容器内的段落子块文本）。
    Quote(u32),
    /// 列表项行（参数 = 深度；行内容映射到 ListItem 文本）。
    ListItem(u32),
    /// `$$x$$` 同行自含数学（内容两侧各 $$ 定界）。
    MathInline,
}

/// 单行的解析映射（投影行 ↔ 块文本几何）。
///
/// 行内容 = `[prefix, line_len - tail)` 内的 char 段，对应块文本
/// `[text_off, text_off + text_len)`。`\r` 尾巴由消费方另行叠加
///（解析器先规范化，行数相等 ⇔ 无行内 `\r`——消费方计数互验）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParsedLineSlot {
    /// 块的先序序号（创建序 == 先序 == 文档序；`None` = 空行/丢弃行）。
    pub block: Option<u32>,
    /// 行内容前的标记宽（char）：标题 `#`+空格 / 列表缩进+标记 / 引用符 /
    /// 围栏开闭行 = 整行。
    pub prefix: u32,
    /// 行内容后的尾部宽（char）：`$$x$$` 收界等（`\r` 由消费方叠加）。
    pub tail: u32,
    /// 行内容在块文本内的 char 起点（多行块的累加值）。
    pub text_off: u32,
    /// 行内容的块文本 char 长（标记/空行 = 0）。
    pub text_len: u32,
    /// 快路径分类。
    pub class: SlotClass,
}

/// 任意行的独立分类（#163 守卫侧原语：编辑结果行的重分类——与解析器
/// 分支判定共用同一套私有函数，零漂移）。与 [`SlotClass`] 的区别：
/// 这是对**任意行**的纯函数；SlotClass 是解析时行在其块上下文里的角色。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineClass {
    Blank,
    Fence,
    MathFence,
    MathInline,
    Heading(usize),
    Thematic,
    TableRow,
    Quote(usize),
    ListItem(usize),
    Paragraph,
}

/// 行分类（#163 单源：解析器分支同函数族；mang-shell 快路径守卫消费）。
pub fn classify_line(line: &str) -> LineClass {
    if fence_info(line).is_some() {
        return LineClass::Fence;
    }
    let trimmed = line.trim();
    if trimmed == "$$" {
        return LineClass::MathFence;
    }
    if trimmed.starts_with("$$") && trimmed.ends_with("$$") && trimmed.len() > 4 {
        return LineClass::MathInline;
    }
    if trimmed.is_empty() {
        return LineClass::Blank;
    }
    if let Some(h) = heading_of(line) {
        return LineClass::Heading(h.0);
    }
    if is_thematic(line) {
        return LineClass::Thematic;
    }
    if line.starts_with('|') {
        return LineClass::TableRow;
    }
    if let Some(bq) = blockquote_of(line) {
        return LineClass::Quote(bq.0);
    }
    if let Some(li) = list_item_of(line) {
        return LineClass::ListItem(li.0);
    }
    LineClass::Paragraph
}

/// 解析 + 行映射（#163：单一循环双产出——树与逐行映射，**零漂移由构造
/// 保证**：`markdown_to_tree` 即本函数的树投影）。
pub fn parse_with_lines(md: &str) -> (BlockTree, Vec<ParsedLineSlot>) {
    // TR-1（issue #9）：行尾规范化前置——CRLF / 孤立 CR 统一为 LF。
    // CommonMark 规范三种行尾等价；模型 = 权威层持有 LF 规范形，使
    // CM6 视图（输入即规范化 LF）、__md() 投影（to_markdown）、
    // docx/xlsx 导出面三面一致（保存内容 = 所见）。三条解析入口
    // （from_markdown / sync_from_markdown / extend_from_markdown）
    // 全部漏斗经本函数，单点覆盖；快路径零分配（无 \r 时 contains
    // 单扫通过，不进入 replace 分支）。
    let normalized;
    let md: &str = if md.contains('\r') {
        normalized = md.replace("\r\n", "\n").replace('\r', "\n");
        &normalized
    } else {
        md
    };
    let mut tree = BlockTree::new();
    let lines: Vec<&str> = md.split('\n').collect();
    let mut slots: Vec<ParsedLineSlot> = Vec::with_capacity(lines.len());
    // 块先序序号计数器（创建序 == 先序 == 文档序，见 ParsedLineSlot.block）。
    let mut seq: u32 = 0;
    // 深度栈：(kind, depth, node)
    let mut stack: Vec<(BlockKind, usize, NodeId)> = Vec::new();
    let root = tree.root();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        // #163 单源围栏识别（fence_info）：``` / ~~~、≤3 前导空白、
        // 任意标记行闭栏——与渲染面逐位一致（此前行首 ``` 硬前缀）。
        if let Some(info) = fence_info(line) {
            let lang = info.lang;
            let mut j = i + 1;
            while j < lines.len() && fence_info(lines[j]).is_none() {
                j += 1;
            }
            let body = &lines[i + 1..j.min(lines.len())];
            let inner = body.join("\n");
            let idx = seq;
            seq += 1;
            let id = tree.create_node(
                BlockKind::CodeBlock,
                &inner,
                attr("lang", &lang),
                Vec::new(),
            );
            attach(&mut tree, root, id);
            // 开行：标记独占（prefix = 整行）。
            slots.push(slot(
                idx,
                line.chars().count() as u32,
                0,
                0,
                0,
                SlotClass::MarkerOnly,
            ));
            // 体行：块文本连加（\n join 的 char 几何）。
            let mut off = 0u32;
            for bl in body {
                let len = bl.chars().count() as u32;
                slots.push(slot(idx, 0, 0, off, len, SlotClass::BodyCode));
                off += len + 1;
            }
            // 闭行（EOF 未闭合 = 无闭行 slot）。
            if j < lines.len() {
                slots.push(slot(
                    idx,
                    lines[j].chars().count() as u32,
                    0,
                    0,
                    0,
                    SlotClass::MarkerOnly,
                ));
            }
            i = j + 1;
            continue;
        }
        // 数学块（#261）：单独 `$$` 行 = fence 同构聚合；`$$x$$` 同行自含。
        // 其余形态（`$$ x` 无收界 / 行中 `$`）不猜——落 paragraph（行内
        // `$...$` 是消费时派生面，见 inline.rs，不进块级判定）。
        let trimmed = line.trim();
        if trimmed == "$$" {
            let mut j = i + 1;
            while j < lines.len() && lines[j].trim() != "$$" {
                j += 1;
            }
            let body = &lines[i + 1..j.min(lines.len())];
            let inner = body.join("\n");
            let idx = seq;
            seq += 1;
            let id = tree.create_node(BlockKind::Math, &inner, Attrs::new(), Vec::new());
            attach(&mut tree, root, id);
            slots.push(slot(
                idx,
                line.chars().count() as u32,
                0,
                0,
                0,
                SlotClass::MarkerOnly,
            ));
            let mut off = 0u32;
            for bl in body {
                let len = bl.chars().count() as u32;
                slots.push(slot(idx, 0, 0, off, len, SlotClass::BodyMath));
                off += len + 1;
            }
            if j < lines.len() {
                slots.push(slot(
                    idx,
                    lines[j].chars().count() as u32,
                    0,
                    0,
                    0,
                    SlotClass::MarkerOnly,
                ));
            }
            i = j + 1;
            continue;
        }
        if trimmed.starts_with("$$") && trimmed.ends_with("$$") && trimmed.len() > 4 {
            let inner = &trimmed[2..trimmed.len() - 2];
            let idx = seq;
            seq += 1;
            let id = tree.create_node(BlockKind::Math, inner, Attrs::new(), Vec::new());
            attach(&mut tree, root, id);
            // 前缀 = 前导空白 + 开界 `$$`；尾 = 收界 `$$` + 后随空白。
            let line_chars = line.chars().count() as u32;
            let lead = line.chars().take_while(|c| c.is_whitespace()).count() as u32;
            let inner_chars = inner.chars().count() as u32;
            slots.push(slot(
                idx,
                lead + 2,
                line_chars - lead - 2 - inner_chars,
                0,
                inner_chars,
                SlotClass::MathInline,
            ));
            i += 1;
            continue;
        }
        if line.trim().is_empty() {
            slots.push(slot(u32::MAX, 0, 0, 0, 0, SlotClass::Blank));
            i += 1;
            continue;
        }
        if let Some(h) = heading_of(line) {
            let idx = seq;
            seq += 1;
            let id = tree.create_node(
                BlockKind::Heading,
                &h.1,
                attr("level", &h.0.to_string()),
                Vec::new(),
            );
            attach(&mut tree, root, id);
            // 前缀 = `#`×level + 空格（空标题无空格——"#" 独占行）。
            let has_space = !h.1.is_empty() || line.chars().nth(h.0) == Some(' ');
            let prefix = h.0 as u32 + if has_space { 1 } else { 0 };
            slots.push(slot(
                idx,
                prefix,
                0,
                0,
                h.1.chars().count() as u32,
                SlotClass::Heading(h.0 as u32),
            ));
            i += 1;
            continue;
        }
        if is_thematic(line) {
            let idx = seq;
            seq += 1;
            let id = tree.create_node(BlockKind::ThematicBreak, "", Attrs::new(), Vec::new());
            attach(&mut tree, root, id);
            slots.push(slot(
                idx,
                line.chars().count() as u32,
                0,
                0,
                0,
                SlotClass::MarkerOnly,
            ));
            i += 1;
            continue;
        }
        if line.starts_with('|') {
            // 连续表格行合并为一个 table 块
            let mut j = i;
            let mut rows = Vec::new();
            while j < lines.len() && lines[j].starts_with('|') {
                rows.push(lines[j]);
                j += 1;
            }
            let idx = seq;
            seq += 1;
            let id = tree.create_node(BlockKind::Table, &rows.join("\n"), Attrs::new(), Vec::new());
            attach(&mut tree, root, id);
            let mut off = 0u32;
            for r in &rows {
                let len = r.chars().count() as u32;
                slots.push(slot(idx, 0, 0, off, len, SlotClass::TableRow));
                off += len + 1;
            }
            i = j;
            continue;
        }
        if let Some(bq) = blockquote_of(line) {
            let depth = bq.0;
            // #278 深度门：超深行跳过（不建节点、不出内容——与 html
            // 侧超深子树跳过同语义；无 Loss 通道故不登记，行为由
            // spike_semantics.rs 深度门测试钉定）。
            if depth >= MAX_DEPTH {
                slots.push(slot(u32::MAX, 0, 0, 0, 0, SlotClass::Dropped));
                i += 1;
                continue;
            }
            let content_chars = bq.1.chars().count() as u32;
            let prefix = line.chars().count() as u32 - content_chars;
            // 同深度同一 quote 容器复用
            if let Some((k, d, id)) = stack.last().copied() {
                if k == BlockKind::Blockquote && d == depth {
                    let p_idx = seq;
                    seq += 1;
                    let p = tree.create_node(BlockKind::Paragraph, &bq.1, Attrs::new(), Vec::new());
                    attach(&mut tree, id, p);
                    slots.push(slot(
                        p_idx,
                        prefix,
                        0,
                        0,
                        content_chars,
                        SlotClass::Quote(depth as u32),
                    ));
                    i += 1;
                    continue;
                }
            }
            pop_ge(&mut stack, depth);
            let _q_idx = seq;
            seq += 1;
            let q = tree.create_node(BlockKind::Blockquote, "", Attrs::new(), Vec::new());
            attach_under_stack(&mut tree, root, &stack, q);
            let p_idx = seq;
            seq += 1;
            let p = tree.create_node(BlockKind::Paragraph, &bq.1, Attrs::new(), Vec::new());
            attach(&mut tree, q, p);
            slots.push(slot(
                p_idx,
                prefix,
                0,
                0,
                content_chars,
                SlotClass::Quote(depth as u32),
            ));
            stack.push((BlockKind::Blockquote, depth, q));
            i += 1;
            continue;
        }
        if let Some(li) = list_item_of(line) {
            let depth = li.0;
            // #278 深度门：同 blockquote 面（缩进层号即嵌套层编号）。
            if depth >= MAX_DEPTH {
                slots.push(slot(u32::MAX, 0, 0, 0, 0, SlotClass::Dropped));
                i += 1;
                continue;
            }
            pop_ge(&mut stack, depth);
            let idx = seq;
            seq += 1;
            let id = tree.create_node(
                BlockKind::ListItem,
                &li.2,
                {
                    let mut a = Attrs::new();
                    a.insert("marker".into(), li.1);
                    a.insert("depth".into(), depth.to_string());
                    a
                },
                Vec::new(),
            );
            attach_under_stack(&mut tree, root, &stack, id);
            stack.push((BlockKind::ListItem, depth, id));
            slots.push(slot(
                idx,
                line.chars().count() as u32 - li.2.chars().count() as u32,
                0,
                0,
                li.2.chars().count() as u32,
                SlotClass::ListItem(depth as u32),
            ));
            i += 1;
            continue;
        }
        // paragraph（一行一块，v0 规则）
        let idx = seq;
        seq += 1;
        let id = tree.create_node(BlockKind::Paragraph, line, Attrs::new(), Vec::new());
        attach(&mut tree, root, id);
        slots.push(slot(
            idx,
            0,
            0,
            0,
            line.chars().count() as u32,
            SlotClass::Paragraph,
        ));
        i += 1;
    }
    debug_assert_eq!(slots.len(), lines.len(), "slot 逐行产出（含空行/丢弃行）");
    (tree, slots)
}

/// 行映射 slot 构造快捷（`block = u32::MAX` 表示无块）。
fn slot(
    block: u32,
    prefix: u32,
    tail: u32,
    text_off: u32,
    text_len: u32,
    class: SlotClass,
) -> ParsedLineSlot {
    ParsedLineSlot {
        block: (block != u32::MAX).then_some(block),
        prefix,
        tail,
        text_off,
        text_len,
        class,
    }
}

/// 解析入口（树投影——#163 起为 [`parse_with_lines`] 的薄封装，行为不变）。
pub fn markdown_to_tree(md: &str) -> BlockTree {
    parse_with_lines(md).0
}

fn attach(tree: &mut BlockTree, parent: NodeId, node: NodeId) {
    let len = tree.node(parent).children.len();
    tree.insert_child(parent, len, node).expect("parse attach");
}

fn attach_under_stack(
    tree: &mut BlockTree,
    root: NodeId,
    stack: &[(BlockKind, usize, NodeId)],
    node: NodeId,
) {
    match stack.last() {
        Some((_, _, id)) => attach(tree, *id, node),
        None => attach(tree, root, node),
    }
}

fn pop_ge(stack: &mut Vec<(BlockKind, usize, NodeId)>, depth: usize) {
    while stack.last().map(|(_, d, _)| *d >= depth).unwrap_or(false) {
        stack.pop();
    }
}

fn heading_of(line: &str) -> Option<(usize, String)> {
    let hashes = heading_level(line)?;
    // "#" 独占行 = 空标题（#163 采纳编辑器/CommonMark 超集）；"# x" = 空格后正文。
    let text = line
        .chars()
        .skip(hashes)
        .collect::<String>()
        .strip_prefix(' ')
        .unwrap_or("")
        .to_string();
    Some((hashes, text))
}

fn is_thematic(line: &str) -> bool {
    matches!(line.trim(), "---" | "***" | "___")
}

fn blockquote_of(line: &str) -> Option<(usize, String)> {
    let mut depth = 0usize;
    let mut rest = line;
    loop {
        if let Some(r) = rest.strip_prefix("> ") {
            depth += 1;
            rest = r;
        } else if let Some(r) = rest.strip_prefix('>') {
            depth += 1;
            rest = r.strip_prefix(' ').unwrap_or(r);
        } else {
            break;
        }
    }
    if depth > 0 {
        Some((depth, rest.to_string()))
    } else {
        None
    }
}

fn list_item_of(line: &str) -> Option<(usize, String, String)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[indent..];
    let (marker, after) = if let Some(r) = rest.strip_prefix("- ") {
        ("-".to_string(), r)
    } else if let Some(r) = rest.strip_prefix("* ") {
        ("*".to_string(), r)
    } else if let Some(r) = rest.strip_prefix("+ ") {
        ("+".to_string(), r)
    } else {
        // 有序列表 1. / 1)
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            let after_digits = &rest[digits.len()..];
            if let Some(r) = after_digits.strip_prefix(". ") {
                (format!("{digits}."), r)
            } else {
                let r = after_digits.strip_prefix(") ")?;
                (format!("{digits})"), r)
            }
        } else {
            return None;
        }
    };
    Some((indent / 2 + 1, marker, after.to_string()))
}

fn attr(k: &str, v: &str) -> Attrs {
    let mut a = Attrs::new();
    if !v.is_empty() {
        a.insert(k.into(), v.into());
    }
    a
}
