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
//! 新版分类先行（`git status --porcelain=v1 -z --untracked-files=all`，精确匹配文件路径）：
//!
//! | porcelain | 判定 | 行为 |
//! |---|---|---|
//! | 空输出 + `ls-files --error-unmatch` tracked | 干净已跟踪 | 空 diff |
//! | 空输出 + 未跟踪 | 缺文件/忽略路径 | 诊断性 `None` |
//! | `??` | 未跟踪/新增 | `--no-index -- /dev/null path`（仅 exit 0/1 有效） |
//! | 其余 XY（M/A/D/MM/AM…） | 已跟踪变更 | `git diff HEAD -- path`；无 HEAD 仓以当前仓库哈希算法的空树为 base（初始变更 vs 空树 = 正确新文件 diff——AM 复合「暂存版 + 又改」两段都可见） |
//! | git 缺失 / 非 git 目录 | 运行失败 | `None` |
//!
//! `git diff` 有差异时 exit 0（非零 = 真失败）——exit 1 统一视为失败的
//! 反模式只允许存在于 `--no-index` 面（该模式下 1 = 有差异，stdout 为
//! 准——非运行失败）。

use std::path::Path;
use std::process::{Command, Stdio};

/// 所有文件路径按字面值解释；root 直接作为 OS 路径参数传递。
fn git_in_root(root: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("--literal-pathspecs").arg("-C").arg(root);
    command
}

/// 一条工作区改动（porcelain XY 码 + 路径）。
#[derive(Clone, Debug, PartialEq)]
pub struct GitChange {
    /// porcelain XY 码（去空格后单字符形态：`M`/`A`/`D`/`??`/`R`…）。
    pub code: String,
    /// 工作区相对路径；重命名/复制为目标路径，可直接用于读取文件和 diff。
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
    let out = git_in_root(root)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(parse_porcelain_z(&out.stdout))
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
    // 过滤到重命名目标的 status 会退化为 A 并丢失 source。因此读完整
    // NUL 状态，再精确匹配目标；展开未跟踪目录以保留每个文件的状态。
    let status = git_in_root(root)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .output()
        .ok()?;
    if !status.status.success() {
        return None;
    }
    let records = porcelain_z_records(&status.stdout);
    let Some((change, original)) = records.iter().find(|(change, _)| change.path == path) else {
        // 干净的已跟踪文件保持空 diff；缺文件或被忽略路径不产伪 diff。
        let tracked = git_in_root(root)
            .args(["ls-files", "--error-unmatch", "--", path])
            .output()
            .ok()?;
        return if tracked.status.success() {
            Some(String::new())
        } else {
            None
        };
    };

    if change.code == "??" {
        let out = git_in_root(root)
            .args(["diff", "--no-index", "--", "/dev/null", path])
            .output()
            .ok()?;
        // no-index 的 1 = 有差异；128/129 等执行失败不能伪装成空成功。
        return if matches!(out.status.code(), Some(0 | 1)) {
            Some(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            None
        };
    }

    let head = git_in_root(root)
        .args(["rev-parse", "--verify", "-q", "HEAD"])
        .output()
        .ok()?;
    let base = if head.status.success() {
        "HEAD".to_string()
    } else if head.status.code() == Some(1) {
        // Stdio::null() 提供零字节 stdin；不带 -w，不写对象库。
        // hash-object 使用仓库的算法（SHA1/SHA256），无硬编码空树 ID。
        let empty = git_in_root(root)
            .args(["hash-object", "-t", "tree", "--stdin"])
            .stdin(Stdio::null())
            .output()
            .ok()?;
        if !empty.status.success() {
            return None;
        }
        let hash = String::from_utf8_lossy(&empty.stdout).trim().to_string();
        if hash.is_empty() {
            return None;
        }
        hash
    } else {
        return None;
    };

    let mut diff = git_in_root(root);
    diff.args(["diff", &base, "--", path]);
    if change.code.contains('R')
        && let Some(original) = original
    {
        // 包含原路径才能保留 rename from/to；复制不带 source，避免混入
        // 源文件的独立修改。
        diff.arg(original);
    }
    let out = diff.output().ok()?;
    if !out.status.success() {
        return None;
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

/// porcelain v1 -z 解析：路径不做 C 引号转义；R/C 记录的目标在前，
/// 下一段是原路径。消费原路径记录，避免误当成第二条改动。
pub fn parse_porcelain_z(bytes: &[u8]) -> Vec<GitChange> {
    porcelain_z_records(bytes)
        .into_iter()
        .map(|(change, _)| change)
        .collect()
}

fn porcelain_z_records(bytes: &[u8]) -> Vec<(GitChange, Option<String>)> {
    let mut records = bytes.split(|byte| *byte == 0);
    let mut changes = Vec::new();
    while let Some(record) = records.next() {
        if record.len() < 4 || record[2] != b' ' {
            continue;
        }
        let code = String::from_utf8_lossy(&record[..2]).trim().to_string();
        let path = String::from_utf8_lossy(&record[3..]).into_owned();
        let original = if code.contains('R') || code.contains('C') {
            records
                .next()
                .map(|path| String::from_utf8_lossy(path).into_owned())
        } else {
            None
        };
        changes.push((GitChange { code, path }, original));
    }
    changes
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
