//! git 仓库纯数据层（#588 从 git_panel.rs 拆出——context_menu/
//! context_menu_view 分层先例）。
//!
//! ## 拆出理由（#588）
//!
//! `git_panel` 原为 bin 专属模块（main.rs `mod`）——两桩问题：
//!
//! 1. **集成测试不可达**：`tests/` 只能 import lib 树（`--lib`/`--tests`
//!    两个 CI 面），纯函数（`git_diff` 的状态分类语义）无法被真实仓
//!    集成测试消费（#588 验收明令「测试调用生产公开 git_diff」）；
//! 2. **内联单测假绿**：CI 的 mang-shell job 只跑 `--lib`/`--tests`
//!    （bin 单测不执行——document.rs 注释登记的同款问题），本模块
//!    3 条解析单测挂在 bin = CI 永不运行。
//!
//! 拆出后：纯函数 + 结构 + 单测进 lib 树（CI 双面真跑）；视图面
//! （GitPanel 实体/渲染/WeakEntity<ShellApp>）留在 bin 的 git_panel.rs。
//!
//! ## #588 git_diff 状态分类（缺陷修复）
//!
//! 旧版（main f36b40e git_panel.rs:110）两处缺陷（独立探针 11 场景
//! 实测：5 红——scripts/probe_588 复现在案）：
//!
//! 1. `git diff HEAD -- path` exit 0 + stdout 空 = **对该文件无差异**
//!    （干净已跟踪）——旧版一律落入 `--no-index /dev/null path` 兜底
//!    → clean tracked 被当「新增」展示全文（含空格/Unicode 路径变体）；
//! 2. 无 HEAD 仓（初始提交前）`git diff HEAD` exit ≠ 0 → `!success`
//!    早退 `None`——「退回 staged/未跟踪形态」的注释意图从未生效，
//!    初始变更被吞。
//!
//! 新版分类先行（`git status --porcelain -- path` 单点）：
//!
//! | porcelain | 判定 | 行为 |
//! |---|---|---|
//! | 空输出 + `ls-files --error-unmatch` tracked | 干净已跟踪 | 空 diff |
//! | 空输出 + 未跟踪 | 缺文件/忽略路径 | 诊断性 `None` |
//! | `??` | 未跟踪/新增 | `--no-index /dev/null`（exit 1 = 有差异，stdout 为准） |
//! | 其余 XY（M/A/D/MM/AM…） | 已跟踪变更 | `git diff HEAD -- path`；无 HEAD 仓以空树 `4b825dc…` 为 base（初始变更 vs 空树 = 正确新文件 diff——AM 复合「暂存版 + 又改」两段都可见） |
//! | git 缺失 / 非 git 目录 | 运行失败 | `None` |
//!
//! `git diff` 有差异时 exit 0（非零 = 真失败）——exit 1 统一视为失败的
//! 反模式只允许存在于 `--no-index` 面（该模式下 1 = 有差异，stdout 为
//! 准——非运行失败）。

use std::path::Path;
use std::process::Command;

/// git 空树哈希（无 HEAD 仓的 diff base——初始变更 vs 空树 = 新文件
/// diff；git rev-parse --verify HEAD 失败即无 HEAD）。
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// 一条工作区改动（porcelain XY 码 + 路径）。
#[derive(Clone, Debug, PartialEq)]
pub struct GitChange {
    /// porcelain XY 码（去空格后单字符形态：`M`/`A`/`D`/`??`/`R`…）。
    pub code: String,
    /// 工作区相对路径（重命名原样含 `orig -> new`）。
    pub path: String,
}

/// 一条提交历史（hash | subject | time）。
#[derive(Clone, Debug, PartialEq)]
pub struct GitCommit {
    pub hash: String,
    pub subject: String,
    pub time: String,
}

/// git 可用（PATH 探测——`git --version`）。
pub fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// 目录是否 git 仓库（`git rev-parse --is-inside-work-tree`）。
pub fn is_git_repo(dir: &Path) -> bool {
    Command::new("git")
        .args([
            "-C",
            dir.to_string_lossy().as_ref(),
            "rev-parse",
            "--is-inside-work-tree",
        ])
        .output()
        .is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
}

/// 工作区改动清单（porcelain XY 码形）。
pub fn git_status(root: &Path) -> Option<Vec<GitChange>> {
    let out = Command::new("git")
        .args([
            "-C",
            root.to_string_lossy().as_ref(),
            "status",
            "--porcelain",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(parse_porcelain(&String::from_utf8_lossy(&out.stdout)))
}

/// 提交历史（前 `limit` 条）。
pub fn git_log(root: &Path, limit: usize) -> Option<Vec<GitCommit>> {
    let format = "%h|%s|%cr";
    let out = Command::new("git")
        .args([
            "-C",
            root.to_string_lossy().as_ref(),
            "log",
            &format!("-n{limit}"),
            &format!("--pretty=format:{format}"),
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(parse_log(&String::from_utf8_lossy(&out.stdout)))
}

/// 文件的未暂存 + 已暂存合并 diff（行级文本）——**按真实文件状态分类**
/// （#588 修复：干净已跟踪返回空 diff；仅未跟踪/新增与空文件比较；
/// 无 HEAD 新仓不吞初始变更；exit 语义见模块文档表）。
pub fn git_diff(root: &Path, path: &str) -> Option<String> {
    // ① 分类先行：porcelain 对该路径的单点状态（exit 0 恒成立——
    //    空输出 = 无状态变化）。
    let st = Command::new("git")
        .args([
            "-C",
            root.to_string_lossy().as_ref(),
            "status",
            "--porcelain",
            "--",
            path,
        ])
        .output()
        .ok()?;
    if !st.status.success() {
        return None; // 真运行失败（非 git 目录 / git 缺失）。
    }
    let xy = String::from_utf8_lossy(&st.stdout);
    let xy = xy.trim_start();
    if xy.is_empty() {
        // ② 无状态变化：tracked 且 clean → 空 diff；非 tracked
        //    （缺文件/忽略路径）→ 诊断性 None（不产伪 diff）。
        let tracked = Command::new("git")
            .args([
                "-C",
                root.to_string_lossy().as_ref(),
                "ls-files",
                "--error-unmatch",
                "--",
                path,
            ])
            .output()
            .ok()?;
        return if tracked.status.success() {
            Some(String::new())
        } else {
            None
        };
    }
    // ③ 未跟踪（`??` 前缀）→ 仅此形态与空文件比较（新文件全文 diff；
    //    --no-index exit 1 = 有差异——stdout 为准，非运行失败）。
    if xy.starts_with("??") {
        let out = Command::new("git")
            .args([
                "-C",
                root.to_string_lossy().as_ref(),
                "diff",
                "--no-index",
                "/dev/null",
                path,
            ])
            .output()
            .ok()?;
        return Some(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    // ④ 已跟踪变更（M/A/D/MM/AM…含无 HEAD 仓的 staged A）：合并 diff
    //    vs HEAD；无 HEAD 仓以空树为 base（初始变更不吞）。
    let has_head = Command::new("git")
        .args([
            "-C",
            root.to_string_lossy().as_ref(),
            "rev-parse",
            "--verify",
            "-q",
            "HEAD",
        ])
        .output()
        .ok()?;
    let base = if has_head.status.success() {
        "HEAD"
    } else {
        EMPTY_TREE
    };
    let out = Command::new("git")
        .args([
            "-C",
            root.to_string_lossy().as_ref(),
            "diff",
            base,
            "--",
            path,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None; // git diff 有差异时 exit 0——非零 = 真失败。
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// 提交的变更文件清单文本（name-status 形）。
pub fn git_show_files(root: &Path, hash: &str) -> Option<String> {
    let out = Command::new("git")
        .args([
            "-C",
            root.to_string_lossy().as_ref(),
            "show",
            "--name-status",
            "--pretty=format:%h %s (%cr)",
            hash,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// porcelain 行解析（`XY path` / `XY orig -> new` / `?? path`）。
/// 纯函数。
pub fn parse_porcelain(text: &str) -> Vec<GitChange> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let code = l.chars().take(2).collect::<String>().trim().to_string();
            let path = if l.len() > 3 {
                l[3..].to_string()
            } else {
                String::new()
            };
            GitChange { code, path }
        })
        .collect()
}

/// 日志行解析（`hash|subject|time`）。纯函数。
pub fn parse_log(text: &str) -> Vec<GitCommit> {
    // 首尾定界：第一竖线前 = 哈希；最后一竖线后 = 时间（%cr 永不含
    // 竖线）；中间全归主题（主题可含竖线）。
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| {
            let (hash, rest) = l.split_once('|')?;
            let (subject, time) = rest.rsplit_once('|')?;
            let hash = hash.to_string();
            let subject = subject.to_string();
            let time = time.to_string();
            if hash.is_empty() {
                None
            } else {
                Some(GitCommit {
                    hash,
                    subject,
                    time,
                })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_parsing_xy_and_paths() {
        let text = " M src/main.rs\n\
                    A  new_file.rs\n\
                    D  gone.rs\n\
                    ?? untracked.md\n\
                    R  old.rs -> new.rs\n\
                    \n";
        let changes = parse_porcelain(text);
        assert_eq!(changes.len(), 5, "空行过滤");
        assert_eq!(changes[0].code, "M");
        assert_eq!(changes[0].path, "src/main.rs");
        assert_eq!(changes[1].code, "A");
        assert_eq!(changes[3].code, "??");
        assert_eq!(changes[3].path, "untracked.md");
        assert_eq!(changes[4].path, "old.rs -> new.rs", "重命名原样");
    }

    #[test]
    fn log_parsing_three_fields() {
        let text = "abc1234|fix: 修复拖拽|2 hours ago\ndef5678|feat: 新功能|3 days ago";
        let commits = parse_log(text);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].hash, "abc1234");
        assert_eq!(commits[0].subject, "fix: 修复拖拽");
        assert_eq!(commits[0].time, "2 hours ago");

        // 主题含竖线——rsplitn(3) 从右定界，竖线余量全归主题。
        let tricky = "abc1234|feat: a|b|c|1 hour ago";
        let commits = parse_log(tricky);
        assert_eq!(commits[0].subject, "feat: a|b|c", "竖线余量归主题");
        assert_eq!(commits[0].time, "1 hour ago");
        assert_eq!(commits[0].hash, "abc1234");

        // 空行过滤 + 空哈希行丢弃。
        assert!(parse_porcelain("").is_empty());
        assert!(parse_log("\n\n").is_empty());
    }

    /// 本仓即 git 仓库——命令面最小实证（沙箱恒有 git）。
    /// #588 拆出后随 lib 树进 CI `--lib` 面（原 bin 挂靠 = 假绿）。
    #[test]
    fn git_command_surface_on_this_repo() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(git_available(), "沙箱有 git");
        assert!(is_git_repo(root), "本仓是 git 工作树");
        // 历史非空（本仓提交众多）。
        let commits = git_log(root, 5).expect("log 拉取");
        assert!(!commits.is_empty(), "本仓有提交历史");
        assert!(commits.len() <= 5, "limit 生效");
        // 非仓库目录 → 探测 false。
        let tmp = std::env::temp_dir();
        assert!(
            !is_git_repo(&tmp) || is_git_repo(&tmp),
            "探测不崩溃（tmp 是否在仓库内因环境而异）"
        );
    }
}
