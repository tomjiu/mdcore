//! #111 G3：词补全——纯函数核心（候选源 + 前缀提取 + 过滤）。
//!
//! 候选源一期口径（D3 决策：文档词表 + md 结构关键词）：
//!
//! - **文档词表**：全文提取 `[A-Za-z0-9_]{2,}` 标识符词（去重、保留
//!   首现序）。CJK 连续串**不构成词**——无分词引擎，整串进词表只会
//!   产出「整句候选」（噪声），诚实边界登记（中文词补全需分词，超
//!   一期面，等 #49 LOK 线再评估——D3 同口径）。
//! - **md 结构关键词**：复用 G5 既有 [`super::highlight::CodeLang`]
//!   关键词表（rust/python/js 三语词表）+ 围栏语言全名——零新造
//!   词表，单一真源（highlight.rs 与本模块同源维护）。
//!
//! 前缀匹配**大小写敏感**（`starts_with` 直通——大小写不敏感匹配是
//! 可选精化，登记边界不预设）。

use super::highlight::CodeLang;

/// 触发阈值：光标前词前缀达到该字符数才开弹窗（1 字符即弹 = 噪声）。
pub const MIN_PREFIX_CHARS: usize = 2;

/// 弹窗最大候选数（截断显示——文档词表可能很大）。
pub const MAX_ITEMS: usize = 8;

/// 词字符判定（补全面口径：ASCII 标识符字符——字母/数字/下划线）。
///
/// CJK 与全角字符均非词字符（见模块文档的诚实边界）。
pub fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// 全文提取词表：`[A-Za-z0-9_]{2,}` 连续段，去重、保留首现序。
///
/// 单字符段排除（"a"/"x" 无补全价值）；越界安全（空文本 → 空表）。
pub fn extract_words(text: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut cur = String::new();
    // 尾哨兵（非词字符）：冲刷末尾词段，循环体无需收尾分支。
    for ch in text.chars().chain(std::iter::once(' ')) {
        if is_word_char(ch) {
            cur.push(ch);
        } else {
            if cur.chars().count() >= 2 && seen.insert(cur.clone()) {
                out.push(cur.clone());
            }
            cur.clear();
        }
    }
    out
}

/// 行内光标前缀：`line` 的前 `col` 个 char 中，结尾连续词字符段。
///
/// `col` 超界按行尾计（不 panic——与 Buffer::line 越界空串同口径）。
/// 例：`("hello world", 7)` → `"wo"`；`("abc", 0)` → `""`。
pub fn word_prefix(line: &str, col: usize) -> String {
    // 正向扫描（Chars 非 ExactSizeIterator，.rev() 不可用）：遇非词
    // 字符清空累积——余量即结尾连续词段。
    let mut tail: Vec<char> = Vec::new();
    for c in line.chars().take(col) {
        if is_word_char(c) {
            tail.push(c);
        } else {
            tail.clear();
        }
    }
    tail.into_iter().collect()
}

/// 候选过滤：词表中以 `prefix` 开头、且**不等于前缀本身**的词
/// （等于前缀 = 已打完的词，无补全意义），保留输入序，截断 `max`。
///
/// 大小写敏感（见模块文档）。池内重复词只取首现（防御——合并池
/// 构造方已去重，此处兜底）。
pub fn candidates(pool: &[String], prefix: &str, max: usize) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for w in pool {
        if out.len() >= max {
            break;
        }
        if w.as_str() == prefix || !w.starts_with(prefix) {
            continue;
        }
        if seen.insert(w.clone()) {
            out.push(w.clone());
        }
    }
    out
}

/// md 结构关键词池（G5 三语关键字表 + 围栏语言全名，去重保序）。
///
/// 语言全名清单（rust/python/javascript）与 [`CodeLang::parse`]
/// 别名集人工同步（新语言加入 G5 时同步此处——登记于注释锚点）。
pub fn keyword_pool() -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut push = |w: &str| {
        if seen.insert(w.to_string()) {
            out.push(w.to_string());
        }
    };
    for lang in [CodeLang::Rust, CodeLang::Python, CodeLang::Js] {
        for kw in lang.keywords() {
            push(kw);
        }
    }
    // 围栏语言全名（```rust 场景的语言标注补全）。
    for name in ["rust", "python", "javascript"] {
        push(name);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------- 纯函数面：词表提取 ----------------

    #[test]
    fn extract_words_dedup_order_and_min_len() {
        let text = "the structure and the structural spirit_axiom a";
        let words = extract_words(text);
        // 去重 + 首现序；单字符词（"a"）排除；下划线词保留。
        assert_eq!(
            words,
            vec![
                "the".to_string(),
                "structure".to_string(),
                "and".to_string(),
                "structural".to_string(),
                "spirit_axiom".to_string(),
            ],
            "词表 = 去重 + 首现序 + 最短 2 字符"
        );
    }

    #[test]
    fn extract_words_cjk_runs_are_not_words() {
        // 诚实边界钉死：CJK 连续串不进词表（无分词——整串候选是噪声）。
        let words = extract_words("中文串不构成词 TCP 网络协议 TCP");
        assert_eq!(
            words,
            vec!["TCP".to_string()],
            "CJK 串排除（边界口径），ASCII 词去重"
        );
    }

    #[test]
    fn extract_words_empty_and_boundaries() {
        assert!(extract_words("").is_empty(), "空文本 → 空表");
        // "__" 本身是 2 字符词段（下划线是词字符）；"_x" 同。单
        // 字符段（"_"/"x"）排除。
        assert_eq!(
            extract_words("__ _x"),
            vec!["__".to_string(), "_x".to_string()],
            "双下划线段（len>=2）与下划线词均保留"
        );
    }

    // ---------------- 纯函数面：前缀提取 ----------------

    #[test]
    fn word_prefix_various_cols() {
        // col 语义 = 光标前字符数（Point.col 同口径）：take(col)。
        assert_eq!(word_prefix("hello world", 8), "wo", "词中间截取");
        assert_eq!(word_prefix("hello world", 6), "", "空格后词首");
        assert_eq!(word_prefix("hello world", 0), "", "行首");
        assert_eq!(word_prefix("abc", 999), "abc", "col 越界按行尾计");
        assert_eq!(word_prefix("中文 abc", 6), "abc", "CJK 段后词前缀");
        assert_eq!(word_prefix("中文 abc", 2), "", "CJK 不构成前缀");
    }

    // ---------------- 纯函数面：候选过滤 ----------------

    #[test]
    fn candidates_filter_exclude_self_and_cap() {
        let pool: Vec<String> = ["str", "structure", "structural", "string", "other"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let out = candidates(&pool, "str", 3);
        assert_eq!(
            out,
            vec![
                "structure".to_string(),
                "structural".to_string(),
                "string".to_string()
            ],
            "前缀过滤 + 排除前缀自身 + 截断 max（保留输入序）"
        );
    }

    #[test]
    fn candidates_case_sensitive_and_empty_prefix() {
        let pool: Vec<String> = ["Structure", "structure"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // 大小写敏感：小写前缀不匹配大写词（钉死——精化另立批）。
        assert_eq!(candidates(&pool, "str", 8), vec!["structure".to_string()]);
        assert_eq!(candidates(&pool, "Str", 8), vec!["Structure".to_string()]);
        // 空前缀 = 未达阈值层的兜底（此处不过滤直接截断——阈值由
        // 调用方 refresh_completion 把关）。
        assert_eq!(candidates(&pool, "", 8).len(), 2);
    }

    // ---------------- 纯函数面：md 结构关键词池 ----------------

    #[test]
    fn keyword_pool_from_g5_tables() {
        let pool = keyword_pool();
        // 非空 + 三语关键字在池（G5 表同源）。
        assert!(pool.contains(&"fn".to_string()), "rust 关键字");
        assert!(pool.contains(&"def".to_string()), "python 关键字");
        assert!(pool.contains(&"function".to_string()), "js 关键字");
        // 围栏语言全名在池（```rust 场景补全）。
        assert!(pool.contains(&"rust".to_string()));
        assert!(pool.contains(&"python".to_string()));
        assert!(pool.contains(&"javascript".to_string()));
        // 去重（跨语重叠词如 let/const 只出现一次）。
        let mut sorted = pool.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), pool.len(), "关键词池无重复");
    }
}
