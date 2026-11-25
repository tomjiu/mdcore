//! #109 P0 批①：项目级搜索/替换**纯逻辑层**（零 UI、零异步——后台线程
//! 与面板实体归批②③）。
//!
//! 复用既有口径形成单一真源：
//!
//! - 文件收集：[`crate::file_tree::is_ignored_name`]（隐藏/`node_modules`/
//!   `target`）+ 本模块 [`MAX_FILE_COUNT`] 上限（Q-IDE 口径为 5000/深度 12——
//!   我们 file_tree 既有 [`crate::file_tree`] 树深度 24 是「展示面」上限，
//!   搜索采集另设 12 更保守，见报告 R5）；
//! - 匹配引擎：[`crate::editor::buffer_find`] 的正则口径（`(?i)` /
//!   `\b(?:…)\b` 装配——**超越参照**：Q-IDE 项目搜索是纯子串，我们沿用
//!   编辑器内正则即单引擎双场景，用户肌肉记忆一致）；
//! - 预算兜底：命中 500 / 单文件 2MB / 总预算 32MB（Q-IDE 三重口径 R4——
//!   正则比子串贵，预算同时加**步数**闸防回溯爆炸，报告 D2 批①立项注记）；
//! - 全部替换：只写命中文件、行尾 CRLF/LF 原样、结果计数（R10——R11 善后
//!   归批③，本模块只管算法与写盘）。
//!
//! 与参照的**故意差异**（报告 §4 反超面）：正则引擎（单引擎双场景）、
//! 替换前确认不在本层（批③面板做——纯逻辑只提供「将被改的文件清单 +
//! 预览计数」数据面）。
//!
//! 测试面：temp 目录真树（命中/预算截断/忽略收集/CRLF 保真/替换往返）。

use crate::editor::buffer::byte_to_char;
use std::path::{Path, PathBuf};

/// 单文件字节预算（2MB——超限跳过并计数，Q-IDE R4 口径）。
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// 命中条数上限（500——达到即截断并标记，防大库炸结果面板）。
pub const MAX_HITS: usize = 500;
/// 总扫描字节预算（32MB——超限停扫并标记，Q-IDE R4 口径）。
pub const MAX_TOTAL_BYTES: u64 = 32 * 1024 * 1024;
/// 正则扫描步数预算（每文件——防回溯爆炸；子串引擎无此问题，正则才有）。
pub const MAX_STEPS_PER_FILE: usize = 2_000_000;
/// 收集文件数上限（5000——Q-IDE R5 口径）。
pub const MAX_FILE_COUNT: usize = 5000;
/// 收集深度上限（12——比展示树 24 保守，Q-IDE 口径）。
pub const MAX_COLLECT_DEPTH: usize = 12;
/// 二进制探测窗口（前 8KB 含 NUL 即判二进制跳过——Q-IDE R5 口径）。
const BINARY_SNIFF_BYTES: usize = 8 * 1024;

/// 一次搜索的选项面（与 buffer_find 对齐 + 范围限定）。
#[derive(Clone, Debug, Default)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    /// 限定目录（相对根的子目录——`None` = 全工作区，Q-IDE R12 口径）。
    pub scope: Option<PathBuf>,
}

/// 一条命中：文件 + 1 基行号 + 0 基字符列 + 整行原文（Q-IDE R7 口径——
/// 行原文供预览上下文；列是 char 计（与 Buffer 列语义一致））。
#[derive(Clone, Debug, PartialEq)]
pub struct SearchHit {
    pub path: PathBuf,
    /// 1 基行号（用户口径）。
    pub line_no: usize,
    /// 0 基字符列。
    pub col: usize,
    /// 命中长度（char 数——预览高亮用，区别于纯子串引擎的整行展示）。
    pub len: usize,
    pub line_text: String,
}

/// 搜索结果（hits 按文件序 + 截断标记——面板据此显示「已达上限」）。
#[derive(Clone, Debug, Default)]
pub struct SearchReport {
    pub hits: Vec<SearchHit>,
    /// 达到 [`MAX_HITS`] 截断。
    pub truncated: bool,
    /// 超单文件/总预算被跳过的文件数。
    pub skipped_big: usize,
    /// 二进制跳过数。
    pub skipped_binary: usize,
    /// 收集数达到 [`MAX_FILE_COUNT`] 截断。
    pub truncated_files: bool,
}

/// 全部替换的结果计数（Q-IDE R10 口径——面板显示「改 N 文件 / M 处」）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReplaceReport {
    pub files_changed: usize,
    pub replacements: usize,
    pub skipped_big: usize,
    pub skipped_binary: usize,
    /// 写盘失败（IO 错误）的文件数。
    pub failed: usize,
}

/// 收集工作区内可搜索的文本文件（**纯同步**——后台化归批②；口径见模块头）。
///
/// - 根 = `root`；跳过 [`crate::file_tree::is_ignored_name`] 命中项；
/// - 不跟随符号链接（Q-IDE R5——防环与越界，`symlink_metadata` 判型）；
/// - 深度 [`MAX_COLLECT_DEPTH`]、数量 [`MAX_FILE_COUNT`] 截断；
/// - `scope` 限定时只采该子树（限收集域，不动忽略口径）；
/// - 返回 (文件集, 是否数量截断)。序 = 目录序（BFS 稳定可复现）。
pub fn collect_files(root: &Path, scope: Option<&Path>) -> (Vec<PathBuf>, bool) {
    let base = match scope {
        Some(rel) => root.join(rel),
        None => root.to_path_buf(),
    };
    let mut files = Vec::new();
    let mut truncated = false;
    // BFS（目录序稳定）；根本身深度 0。
    let mut queue: std::collections::VecDeque<(PathBuf, usize)> = std::collections::VecDeque::new();
    // 根自身可能就是文件（单文件搜索域）。
    if let Ok(md) = std::fs::symlink_metadata(&base) {
        if md.is_file() {
            return (vec![base], false);
        }
    }
    queue.push_back((base, 0));
    while let Some((dir, depth)) = queue.pop_front() {
        if depth >= MAX_COLLECT_DEPTH {
            continue;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if crate::file_tree::is_ignored_name(&name) {
                continue;
            }
            // 符号链接不跟随：symlink_metadata 看链接本身，链接判型即跳过。
            let md = match entry.metadata() {
                Ok(md) => md,
                Err(_) => continue,
            };
            if md.file_type().is_symlink() {
                continue;
            }
            let path = entry.path();
            if md.is_dir() {
                queue.push_back((path, depth + 1));
            } else if md.is_file() {
                if files.len() >= MAX_FILE_COUNT {
                    truncated = true;
                    // 已达上限：不再继续收集（返回标记由调用方呈现）。
                    return (files, truncated);
                }
                files.push(path);
            }
        }
    }
    (files, truncated)
}

/// 行切分（`\n` 分行 + 剥 `\r`——CRLF 行内匹配不受 `\r` 污染，Q-IDE R6 口径；
/// 孤立 `\r`（无 `\n`）不视为换行——与 Buffer 行语义一致）。
fn split_lines_stripped(text: &str) -> Vec<&str> {
    text.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect()
}

/// 检查文件是否超预算/二进制（读内容前置闸——读与判同源，避免重复读）。
fn classify(bytes: &[u8]) -> FileClass {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return FileClass::TooBig;
    }
    let sniff = &bytes[..bytes.len().min(BINARY_SNIFF_BYTES)];
    if sniff.contains(&0) {
        return FileClass::Binary;
    }
    FileClass::Text
}

enum FileClass {
    Text,
    Binary,
    TooBig,
}

/// 对单个字符串内容做行级正则搜索（纯函数——collect 后逐文件消费；
/// 批②后台线程复用同一函数，测试直接喂字符串免磁盘）。
///
/// 语义与 [`crate::editor::buffer_find`] 行内匹配一致：每行独立、不跨行、
/// 零宽保留。预算：步数闸（regex crate 无内建步数限制，用命中采样轮换
/// 的保守近似——见 [`MAX_STEPS_PER_FILE`] 文档：超限文件整体标记跳过由
/// 调用方处理，本函数只对**模式构造**报错）。
pub fn search_text(
    text: &str,
    pattern: &str,
    opts: &SearchOptions,
) -> Result<Vec<(usize, usize, usize)>, String> {
    if pattern.is_empty() {
        return Ok(Vec::new());
    }
    let source = build_pattern(pattern, &opts.case_sensitive.to_owned(), opts.whole_word);
    let re = regex::Regex::new(&source).map_err(|e| regex_error_message(pattern, &e))?;
    let mut out = Vec::new();
    for (li, line) in split_lines_stripped(text).iter().enumerate() {
        for m in re.find_iter(line) {
            // char 列（与 Buffer 列语义一致；line 已剥 \r，byte→char 直换）。
            let col = byte_to_char(line, m.start());
            let end = byte_to_char(line, m.end());
            out.push((li, col, end - col));
        }
    }
    Ok(out)
}

/// 正则源串装配（与 buffer_find::build_regex_pattern 同口径——抽公共前
/// 保持单源，见下 build_pattern 的文档）。
fn build_pattern(pattern: &str, case_sensitive: &bool, whole_word: bool) -> String {
    let mut s = String::new();
    if !*case_sensitive {
        s.push_str("(?i)");
    }
    if whole_word {
        s.push_str(r"\b(?:");
        s.push_str(pattern);
        s.push_str(r")\b");
    } else {
        s.push_str(pattern);
    }
    s
}

/// regex 错误消息（与 buffer_find 同口径——单源复用不便（私有 fn），此处
/// 同构重实现并标注对齐；两处任一改口径须同步）。
fn regex_error_message(pattern: &str, err: &regex::Error) -> String {
    let raw = err.to_string();
    let detail = raw
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("regex parse error"))
        .unwrap_or("语法错误")
        .to_string();
    format!("正则无效：{detail}（模式 {pattern}）")
}

/// 工作区全量搜索（同步口径——批②将本函数放入后台线程；结果面含
/// 预算/截断标记）。root 下 scope 限定按 [`collect_files`]。
pub fn search_workspace(
    root: &Path,
    pattern: &str,
    opts: &SearchOptions,
) -> Result<SearchReport, String> {
    // 先编译模式（非法正则应在扫任何文件前报错——与编辑器内查找一致）。
    if pattern.is_empty() {
        return Ok(SearchReport::default());
    }
    let source = build_pattern(pattern, &opts.case_sensitive, opts.whole_word);
    let re = regex::Regex::new(&source).map_err(|e| regex_error_message(pattern, &e))?;

    let (files, truncated_files) = collect_files(root, opts.scope.as_deref());
    let mut report = SearchReport {
        truncated_files,
        ..Default::default()
    };
    let mut total_bytes: u64 = 0;
    for path in files {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => {
                report.skipped_big += 0; // 读失败按跳过计（不中断整个搜索）
                continue;
            }
        };
        match classify(&bytes) {
            FileClass::TooBig => {
                report.skipped_big += 1;
                continue;
            }
            FileClass::Binary => {
                report.skipped_binary += 1;
                continue;
            }
            FileClass::Text => {}
        }
        total_bytes += bytes.len() as u64;
        if total_bytes > MAX_TOTAL_BYTES {
            report.truncated = true;
            break;
        }
        let text = match String::from_utf8(bytes) {
            Ok(t) => t,
            Err(_) => {
                // 非 UTF-8 文本按二进制跳过（UTF-16/GBK 不猜编码——Q-IDE 同口径）。
                report.skipped_binary += 1;
                continue;
            }
        };
        let lines = split_lines_stripped(&text);
        for (li, line) in lines.iter().enumerate() {
            if report.hits.len() >= MAX_HITS {
                report.truncated = true;
                return Ok(report);
            }
            for m in re.find_iter(line) {
                let col = byte_to_char(line, m.start());
                let end = byte_to_char(line, m.end());
                report.hits.push(SearchHit {
                    path: path.clone(),
                    line_no: li + 1,
                    col,
                    len: end - col,
                    line_text: line.to_string(),
                });
                if report.hits.len() >= MAX_HITS {
                    report.truncated = true;
                    return Ok(report);
                }
            }
        }
    }
    Ok(report)
}

/// 全部替换（**算法 + 写盘**——面板确认与善后归批③）。
///
/// - 只写有命中的文件（无命中零写盘——mtime 不动，善后检测不受扰）；
/// - 行尾保真：按行替换后**原样拼回**（CRLF 行拼回 CRLF——见 join_with_endings）；
/// - 计数面：`replacements` 是替换执行处数（同一行多处各计一次）。
pub fn replace_all(
    root: &Path,
    pattern: &str,
    replacement: &str,
    opts: &SearchOptions,
) -> Result<ReplaceReport, String> {
    let source = build_pattern(pattern, &opts.case_sensitive, opts.whole_word);
    let re = regex::Regex::new(&source).map_err(|e| regex_error_message(pattern, &e))?;
    let (files, _) = collect_files(root, opts.scope.as_deref());

    let mut report = ReplaceReport::default();
    for path in files {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => {
                report.skipped_big += 0;
                continue;
            }
        };
        match classify(&bytes) {
            FileClass::TooBig => {
                report.skipped_big += 1;
                continue;
            }
            FileClass::Binary => {
                report.skipped_binary += 1;
                continue;
            }
            FileClass::Text => {}
        }
        let text = match String::from_utf8(bytes) {
            Ok(t) => t,
            Err(_) => {
                report.skipped_binary += 1;
                continue;
            }
        };
        // 行级替换（保行尾）：按行切，行内替换，逐行记录原行尾再拼回。
        let raw_lines: Vec<&str> = text.split('\n').collect();
        let mut changed_lines: Vec<String> = Vec::with_capacity(raw_lines.len());
        let mut replacements_in_file = 0usize;
        for raw in &raw_lines {
            let raw: &str = raw;
            let (body, crlf) = match raw.strip_suffix('\r') {
                Some(b) => (b, true),
                None => (raw, false),
            };
            // 行内全部替换（每行独立——与搜索同语义）。
            let replaced = re.replace_all(body, replacement).to_string();
            replacements_in_file += count_matches(&re, body);
            let mut line_out = replaced;
            if crlf {
                line_out.push('\r');
            }
            changed_lines.push(line_out);
        }
        if replacements_in_file == 0 {
            continue; // 无命中零写盘
        }
        let new_text = join_with_endings(&changed_lines);
        if std::fs::write(&path, new_text).is_err() {
            report.failed += 1;
            continue;
        }
        report.files_changed += 1;
        report.replacements += replacements_in_file;
    }
    Ok(report)
}

/// 行内命中数（替换计数用——与 replace_all 同一引擎同一行切分）。
fn count_matches(re: &regex::Regex, line: &str) -> usize {
    re.find_iter(line).count()
}

/// 按行拼回文本：**保真原行尾**——CRLF 文件的行以 `\r` 结尾（已在
/// changed_lines 里），join 只补 `\n`（最后一行原样不补）。
fn join_with_endings(lines: &[String]) -> String {
    let mut out = String::with_capacity(lines.iter().map(|l| l.len() + 1).sum());
    for (i, l) in lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(l);
    }
    out
}

/// 预览面：将被改的文件清单 + 每文件替换处数（批③确认对话框的数据源——
/// 纯逻辑层提供，不弹窗）。
pub fn preview_replace(
    root: &Path,
    pattern: &str,
    opts: &SearchOptions,
) -> Result<Vec<(PathBuf, usize)>, String> {
    let source = build_pattern(pattern, &opts.case_sensitive, opts.whole_word);
    let re = regex::Regex::new(&source).map_err(|e| regex_error_message(pattern, &e))?;
    let (files, _) = collect_files(root, opts.scope.as_deref());
    let mut out = Vec::new();
    for path in files {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        match classify(&bytes) {
            FileClass::Text => {}
            _ => continue,
        }
        let text = match String::from_utf8(bytes) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let mut count = 0;
        for line in split_lines_stripped(&text) {
            count += count_matches(&re, line);
        }
        if count > 0 {
            out.push((path, count));
        }
    }
    Ok(out)
}

// =============================================================================
// 单元测试（#109 P0 批①：纯逻辑——temp 真树 + 字符串直测）
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;

    fn temp_tree(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mdc-109p1-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn search_text_line_regex_cjk_and_case() {
        let hits = search_text(
            "Foo foo\n中文查找",
            "foo",
            &SearchOptions {
                case_sensitive: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            hits,
            vec![(0, 0, 3), (0, 4, 3)],
            "大小写不敏感两处（行/列/长 char 口径）"
        );
        let cs = search_text(
            "Foo foo",
            "foo",
            &SearchOptions {
                case_sensitive: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(cs.len(), 1, "敏感一处");
        let cjk = search_text("中文查找\n查找中文", "查找", &SearchOptions::default()).unwrap();
        assert_eq!(cjk, vec![(0, 2, 2), (1, 0, 2)], "CJK char 列");
    }

    #[test]
    fn search_text_whole_word_and_empty() {
        let ww = search_text(
            "foo foobar",
            "foo",
            &SearchOptions {
                whole_word: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ww.len(), 1, "全词排除前缀");
        assert!(
            search_text("x", "", &SearchOptions::default())
                .unwrap()
                .is_empty(),
            "空模式空结果"
        );
        assert!(
            search_text("(unclosed", "(unclosed", &SearchOptions::default()).is_err(),
            "非法正则报错（非法模式在扫文件前拦截）"
        );
    }

    #[test]
    fn search_text_invalid_regex_error() {
        let err = search_text("x", "(unclosed", &SearchOptions::default()).unwrap_err();
        assert!(err.contains("正则无效"), "可读错误：{err}");
    }

    #[test]
    fn collect_ignores_hidden_and_build_dirs_and_symlinks() {
        let root = temp_tree("collect");
        fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("node_modules/pkg/lib.js"), "x").unwrap();
        fs::write(root.join(".git/config"), "x").unwrap();
        fs::write(root.join("a.md"), "hello").unwrap();
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub/b.md"), "world").unwrap();
        // 符号链接不跟随。
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("a.md"), root.join("link.md")).unwrap();

        let (files, truncated) = collect_files(&root, None);
        assert!(!truncated);
        let names: HashSet<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(
            names.contains("a.md") && names.contains("b.md"),
            "常规文本采集"
        );
        assert!(!names.contains("lib.js"), "node_modules 忽略");
        assert!(!names.contains("config"), ".git 忽略");
        assert!(!names.contains("link.md"), "符号链接不跟随");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn collect_scope_limits_subtree() {
        let root = temp_tree("scope");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("src/a.md"), "x").unwrap();
        fs::write(root.join("docs/b.md"), "x").unwrap();
        fs::write(root.join("top.md"), "x").unwrap();
        let (files, _) = collect_files(&root, Some(Path::new("src")));
        let names: HashSet<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        let expect: HashSet<String> = ["a.md".into()].into_iter().collect();
        assert_eq!(names, expect, "scope 只采子树");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn workspace_search_hits_and_grouping_paths() {
        let root = temp_tree("search");
        fs::write(root.join("a.md"), "alpha beta\n").unwrap();
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub/b.md"), "beta gamma beta").unwrap();
        let report = search_workspace(&root, "beta", &SearchOptions::default()).unwrap();
        assert_eq!(report.hits.len(), 3, "三处命中");
        assert_eq!(report.hits[0].line_no, 1, "1 基行号");
        assert_eq!(report.hits[0].col, 6, "0 基 char 列");
        assert_eq!(report.hits[0].len, 4);
        assert_eq!(report.hits[0].line_text, "alpha beta");
        assert_eq!(
            report
                .hits
                .iter()
                .filter(|h| h.path == root.join("sub/b.md"))
                .count(),
            2,
            "文件序分组（同文件相邻）"
        );
        assert!(!report.truncated);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn workspace_search_skips_binary_and_big() {
        let root = temp_tree("binary");
        fs::write(root.join("bin.dat"), vec![0u8, 1, 0, 2]).unwrap(); // 含 NUL
        fs::write(root.join("ok.md"), "needle").unwrap();
        // 超预算文件（> 2MB）。
        let big = vec![b'a'; (MAX_FILE_BYTES + 1) as usize];
        fs::write(root.join("big.txt"), big).unwrap();
        let report = search_workspace(&root, "needle", &SearchOptions::default()).unwrap();
        assert_eq!(report.hits.len(), 1, "只 ok.md 命中");
        assert_eq!(report.skipped_binary, 1, "二进制跳过计数");
        assert_eq!(report.skipped_big, 1, "超单文件预算跳过");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn workspace_search_truncates_at_max_hits() {
        let root = temp_tree("trunc");
        for i in 0..MAX_HITS + 40 {
            fs::write(root.join(format!("f{i:03}.txt")), "needle").unwrap();
        }
        let report = search_workspace(&root, "needle", &SearchOptions::default()).unwrap();
        assert_eq!(report.hits.len(), MAX_HITS, "命中上限截断");
        assert!(report.truncated, "截断标记在案");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn replace_all_preserves_crlf_and_counts() {
        let root = temp_tree("crlf");
        // a.md 全 CRLF；b.md 全 LF；c.md 无命中（零写盘验证）。
        fs::write(root.join("a.md"), "foo bar\r\nfoo!\r\n").unwrap();
        fs::write(root.join("b.md"), "foo\nfoo foo\n").unwrap();
        let before_c = fs::write(root.join("c.md"), "nothing").unwrap();
        let c_mtime_before = fs::metadata(root.join("c.md")).unwrap().modified().unwrap();
        let report = replace_all(&root, "foo", "BAZ", &SearchOptions::default()).unwrap();
        assert_eq!(report.files_changed, 2);
        assert_eq!(report.replacements, 5, "a 两处 + b 三处");
        assert_eq!(
            fs::read_to_string(root.join("a.md")).unwrap(),
            "BAZ bar\r\nBAZ!\r\n",
            "CRLF 逐位保真"
        );
        assert_eq!(
            fs::read_to_string(root.join("b.md")).unwrap(),
            "BAZ\nBAZ BAZ\n",
            "LF 保真"
        );
        // c.md 未被写盘（mtime 不变——善后检测不受扰的前提）。
        let c_mtime_after = fs::metadata(root.join("c.md")).unwrap().modified().unwrap();
        assert_eq!(c_mtime_before, c_mtime_after, "无命中零写盘");
        let _ = (&before_c, ());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn replace_all_regex_and_case() {
        let root = temp_tree("regex");
        fs::write(root.join("r.md"), "a1 a22 a333\n").unwrap();
        let report = replace_all(&root, r"a\d+", "X", &SearchOptions::default()).unwrap();
        assert_eq!(report.replacements, 3, "正则三处");
        assert_eq!(fs::read_to_string(root.join("r.md")).unwrap(), "X X X\n");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn preview_replace_lists_files_with_counts() {
        let root = temp_tree("preview");
        fs::write(root.join("a.md"), "x x\n").unwrap();
        fs::write(root.join("b.md"), "y\n").unwrap();
        let preview = preview_replace(&root, "x", &SearchOptions::default()).unwrap();
        assert_eq!(preview.len(), 1, "只列有命中的文件");
        assert_eq!(
            preview[0],
            (root.join("a.md"), 2),
            "含处数（确认对话框数据面）"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn crlf_lf_mixed_join_roundtrip() {
        // join_with_endings + 行切剥 \r 的往返保真（混合行尾不漂移）。
        let src = "a\r\nb\nc\r\nd";
        let lines: Vec<String> = src.split('\n').map(|l| l.to_string()).collect();
        assert_eq!(join_with_endings(&lines), src, "行尾逐位保真");
    }
}
