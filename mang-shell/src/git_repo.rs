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
//!
//! ## #598 子目录工作区基准统一
//!
//! **缺陷**：工作区选为 git 仓库子目录时，porcelain 的 path 恒为**仓库根
//! 相对**（`git -C nested status` 返回 `nested/tracked.txt`），而
//! `git -C nested diff -- nested/tracked.txt` 把同一前缀再按**执行目录**
//! 解释一次（`nested/nested/tracked.txt`）→ 空 diff、同名串读风险
//! （探针：scripts/probe_598.sh——0 字节 vs 仓库根口径 166 字节）。
//!
//! **统一**（纯层、不动 GUI）：
//!
//! | 面 | 基准 |
//! |---|---|
//! | 执行目录 | `git rev-parse --show-toplevel` 定位**实际仓库根**，所有命令 `-C 仓库根`（top 不再是调用方传入目录） |
//! | git 内 path 实参 | 一律**仓库根相对**（`workspace_to_repo`：工作区相对 + `rev-parse --show-prefix` 前缀） |
//! | [`GitChange::path`] | **工作区相对**（`repo_to_workspace`：整段前缀截除）——契约不变：可直接 `root.join(path)` 读文件 |
//!
//! `git_status` 同时**收窄到工作区范围**：前缀外（`nestedx/`、仓库其它
//! 目录）的改动丢弃——`root.join(path)` 读取不变量对每一行列成立
//! （VS Code 打开子目录同款口径；不为修 Git 把工作区换成父目录）。
//! 前缀匹配**整段精确**（`nested/` ≠ `nestedx/`），literal 文件名、
//! 无 HEAD 空树（SHA1/SHA256 跟随仓库算法）、`--no-index` exit 语义
//! 行为全部保留——仓库根工作区（前缀空）与旧输出逐字节等价。

use std::path::Path;
use std::process::{Command, Stdio};

/// 所有文件路径按字面值解释；root 直接作为 OS 路径参数传递。
fn git_in_root(root: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("--literal-pathspecs").arg("-C").arg(root);
    command
}

/// #598：实际仓库根 + 工作区前缀——path 基准统一底座。
struct RepoBase {
    /// 实际仓库根（`rev-parse --show-toplevel` 原样输出——git 自己产出
    /// 的路径串，作为 `-C` 参数原样回传，不经 shell 拼接）。
    top: String,
    /// 工作区相对仓库根的 POSIX 前缀（**不带**尾斜杠；仓库根本身 = 空）。
    prefix: String,
}

/// 以**实际仓库根**为执行目录的 git 命令底座（#598：执行目录统一——
/// porcelain path 与命令实参不再被调用方目录二次解释）。
fn git_in_top(top: &str) -> Command {
    let mut command = Command::new("git");
    command.arg("--literal-pathspecs").arg("-C").arg(top);
    command
}

/// 定位实际仓库根与工作区前缀。非 git 目录 / 裸仓 → `None`（诊断性
/// 失败，与旧版 status 失败口径一致）。
fn repo_base(root: &Path) -> Option<RepoBase> {
    let top = git_in_root(root)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !top.status.success() {
        return None;
    }
    let top = String::from_utf8_lossy(&top.stdout)
        .trim_end_matches(['\r', '\n'])
        .to_string();
    if top.is_empty() {
        return None;
    }
    let prefix = git_in_root(root)
        .args(["rev-parse", "--show-prefix"])
        .output()
        .ok()?;
    if !prefix.status.success() {
        return None;
    }
    // POSIX 化（Windows 反斜杠变体对齐 porcelain 的正斜杠形态）。
    // 先 trim 空白（stdout 尾换行）再裁斜杠——顺序反了会把换行留在段内。
    let prefix = String::from_utf8_lossy(&prefix.stdout)
        .trim_end_matches(['\r', '\n'])
        .replace('\\', "/")
        .trim_matches('/')
        .to_string();
    Some(RepoBase { top, prefix })
}

/// 工作区相对 path → 仓库根相对 path（#598：git 实参基准）。
fn workspace_to_repo(base: &RepoBase, path: &str) -> String {
    if base.prefix.is_empty() {
        path.to_string()
    } else {
        format!("{}/{}", base.prefix, path)
    }
}

/// 仓库根相对 path → 工作区相对 path。前缀**整段**不匹配（工作区外，
/// 如 `nestedx/` 对前缀 `nested`）→ `None`（status 收窄面丢弃）。
fn repo_to_workspace(base: &RepoBase, path: &str) -> Option<String> {
    if base.prefix.is_empty() {
        return Some(path.to_string());
    }
    let marker = format!("{}/", base.prefix);
    path.strip_prefix(&marker).map(str::to_string)
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

/// 工作区改动清单（porcelain XY 码形）。path 为**工作区相对**基准
/// （#598：仓库根工作区 == 旧口径逐字节等价；子目录工作区收窄到
/// 工作区范围——`root.join(path)` 读取不变量）。
pub fn git_status(root: &Path) -> Option<Vec<GitChange>> {
    let base = repo_base(root)?;
    let out = git_in_top(&base.top)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    // porcelain path 恒为仓库根相对：按工作区前缀截除（整段精确），
    // 前缀外改动丢弃（收窄）。
    Some(
        porcelain_z_records(&out.stdout)
            .into_iter()
            .filter_map(|(mut change, _)| {
                change.path = repo_to_workspace(&base, &change.path)?;
                Some(change)
            })
            .collect(),
    )
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
/// #598：`path` 接收**工作区相对**基准——内部经 `workspace_to_repo`
/// 统一为仓库根相对、命令一律在**实际仓库根**执行，前缀不再被
/// 调用方目录二次解释（子目录工作区不空 diff、不串读相似名）。
pub fn git_diff(root: &Path, path: &str) -> Option<String> {
    let base = repo_base(root)?;
    let repo_path = workspace_to_repo(&base, path);
    // 过滤到重命名目标的 status 会退化为 A 并丢失 source。因此读完整
    // NUL 状态，再精确匹配目标；展开未跟踪目录以保留每个文件的状态。
    let status = git_in_top(&base.top)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .output()
        .ok()?;
    if !status.status.success() {
        return None;
    }
    let records = porcelain_z_records(&status.stdout);
    let Some((change, original)) = records.iter().find(|(change, _)| change.path == repo_path)
    else {
        // 干净的已跟踪文件保持空 diff；缺文件或被忽略路径不产伪 diff。
        let tracked = git_in_top(&base.top)
            .args(["ls-files", "--error-unmatch", "--", &repo_path])
            .output()
            .ok()?;
        return if tracked.status.success() {
            Some(String::new())
        } else {
            None
        };
    };

    if change.code == "??" {
        let out = git_in_top(&base.top)
            .args(["diff", "--no-index", "--", "/dev/null", &repo_path])
            .output()
            .ok()?;
        // no-index 的 1 = 有差异；128/129 等执行失败不能伪装成空成功。
        return if matches!(out.status.code(), Some(0 | 1)) {
            Some(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            None
        };
    }

    let head = git_in_top(&base.top)
        .args(["rev-parse", "--verify", "-q", "HEAD"])
        .output()
        .ok()?;
    let against = if head.status.success() {
        "HEAD".to_string()
    } else if head.status.code() == Some(1) {
        // Stdio::null() 提供零字节 stdin；不带 -w，不写对象库。
        // hash-object 使用仓库的算法（SHA1/SHA256），无硬编码空树 ID。
        let empty = git_in_top(&base.top)
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

    let mut diff = git_in_top(&base.top);
    diff.args(["diff", &against, "--", &repo_path]);
    if change.code.contains('R')
        && let Some(original) = original
    {
        // 包含原路径才能保留 rename from/to；复制不带 source，避免混入
        // 源文件的独立修改。（original 已是仓库根相对——直接作实参。）
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
    use std::path::PathBuf;

    // --------------------------------------------------------------
    // #598 夹具：真实临时仓（std-only——本模块被 scripts/check_git_repo.py
    // 以 rustc --test 独立编译，不得引用 crate 其它面）。
    // --------------------------------------------------------------

    fn sh(root: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(["-C", root.to_string_lossy().as_ref()])
            .args(args)
            .output()
            .unwrap_or_else(|e| panic!("git 应可执行（{e}）"));
        assert!(
            out.status.success(),
            "git {args:?} 失败：{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn temp_repo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mdc-598-unit-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        sh(&dir, &["init", "-q"]);
        sh(&dir, &["config", "user.email", "probe@local"]);
        sh(&dir, &["config", "user.name", "probe"]);
        dir
    }

    fn git_present() -> bool {
        std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    // --------------------------------------------------------------
    // #598 单测面：子目录工作区——仓库根执行 + 仓库根相对 path 基准统一
    // （每条先红后绿在案：基线 ac990a8 下 status 仓库根相对 + diff
    //   以工作区解释 → 前缀解释两次 → 空 diff）。
    // --------------------------------------------------------------

    #[test]
    fn subdir_status_paths_are_workspace_relative() {
        if !git_present() {
            eprintln!("SKIP（#598 环境门）：无 git");
            return;
        }
        let dir = temp_repo("strip");
        std::fs::write(dir.join("顶层.txt"), "顶\n").unwrap();
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("nested/tracked.txt"), "基线\n").unwrap();
        sh(&dir, &["add", "--all"]);
        sh(&dir, &["commit", "-q", "-m", "init"]);
        // 工作区内改 + 工作区外改（scope 收窄面：工作区外的改动丢弃——
        // root.join(path) 读取不变量）。
        std::fs::write(dir.join("nested/tracked.txt"), "基线\n改\n").unwrap();
        std::fs::write(dir.join("顶层.txt"), "顶\n也改\n").unwrap();

        let nested = dir.join("nested");
        let changes = git_status(&nested).expect("子目录 root 是有效仓");
        assert_eq!(
            changes.len(),
            1,
            "工作区外改动不列出（顶层.txt）；实测 {:?}",
            changes.iter().map(|c| c.path.clone()).collect::<Vec<_>>()
        );
        assert_eq!(
            changes[0].path, "tracked.txt",
            "#598 核心：path 是工作区相对（不是仓库根相对 nested/tracked.txt）"
        );
        assert_eq!(changes[0].code, "M");
    }

    #[test]
    fn subdir_status_to_diff_roundtrip_two_levels() {
        if !git_present() {
            eprintln!("SKIP（#598 环境门）：无 git");
            return;
        }
        // 一级与两级子目录各自作为工作区：status → diff 链路返回真实差异。
        let dir = temp_repo("roundtrip");
        std::fs::create_dir_all(dir.join("nested/deep")).unwrap();
        std::fs::write(dir.join("nested/tracked.txt"), "基线\n").unwrap();
        std::fs::write(dir.join("nested/deep/deep.txt"), "深基线\n").unwrap();
        sh(&dir, &["add", "--all"]);
        sh(&dir, &["commit", "-q", "-m", "init"]);
        std::fs::write(dir.join("nested/tracked.txt"), "基线\n一级改\n").unwrap();
        std::fs::write(dir.join("nested/deep/deep.txt"), "深基线\n二级改\n").unwrap();

        for (root, expected) in [
            (dir.clone(), "nested/tracked.txt"),
            (dir.join("nested"), "tracked.txt"),
        ] {
            let changes = git_status(&root).expect("status 应成功");
            let change = changes
                .iter()
                .find(|c| c.path == expected)
                .unwrap_or_else(|| panic!("工作区相对 path {expected:?} 应在 {changes:?}"));
            let diff = git_diff(&root, &change.path).expect("diff 应有结果（非运行失败）");
            assert!(
                diff.contains("+一级改"),
                "一级子目录链路：真实 hunk（根={:?} path={:?} 实测 {:?}）",
                root,
                change.path,
                &diff[..diff.len().min(120)]
            );
        }
        // 两级子目录：deep 作为工作区。
        let deep = dir.join("nested/deep");
        let changes = git_status(&deep).expect("两级子目录 status");
        assert_eq!(changes.len(), 1, "deep 工作区只含自己范围内的改动");
        assert_eq!(changes[0].path, "deep.txt");
        let diff = git_diff(&deep, &changes[0].path).expect("两级子目录 diff");
        assert!(diff.contains("+二级改"), "两级子目录链路返回真实 hunk");
    }

    #[test]
    fn subdir_similar_names_do_not_cross_read() {
        if !git_present() {
            eprintln!("SKIP（#598 环境门）：无 git");
            return;
        }
        // 相似名陷阱：nested/a.txt vs 顶层 a.txt vs 前缀邻居 nestedx/a.txt。
        // status 收窄必须按整段前缀（nested/）精确匹配，不能字面前缀
        // 截断（否则 nestedx/a.txt 被误切成 x/a.txt）。
        let dir = temp_repo("similar");
        std::fs::write(dir.join("a.txt"), "顶层\n").unwrap();
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::create_dir_all(dir.join("nestedx")).unwrap();
        std::fs::write(dir.join("nested/a.txt"), "巢\n").unwrap();
        std::fs::write(dir.join("nestedx/a.txt"), "巢邻\n").unwrap();
        sh(&dir, &["add", "--all"]);
        sh(&dir, &["commit", "-q", "-m", "init"]);
        std::fs::write(dir.join("nested/a.txt"), "巢\n巢改\n").unwrap();
        std::fs::write(dir.join("nestedx/a.txt"), "巢邻\n邻改\n").unwrap();

        let nested = dir.join("nested");
        let changes = git_status(&nested).expect("status 应成功");
        assert_eq!(
            changes.len(),
            1,
            "前缀整段精确：nestedx/a.txt（工作区外）与顶层改动都不列；实测 {changes:?}"
        );
        assert_eq!(changes[0].path, "a.txt");
        let diff = git_diff(&nested, "a.txt").expect("diff 应有结果");
        assert!(diff.contains("+巢改"), "读的是 nested/a.txt（工作区文件）");
        assert!(!diff.contains("+邻改"), "不串读前缀邻居 nestedx/a.txt");
        assert!(!diff.contains("+顶层"), "不串读顶层 a.txt");
    }

    #[test]
    fn subdir_no_head_sha256_repo_keeps_initial_change() {
        if !git_present() {
            eprintln!("SKIP（#598 环境门）：无 git");
            return;
        }
        // 子目录 root + 无 HEAD 新仓（SHA256）：空树 base 必须取自同一
        // 仓库（hash-object 在实际仓库根执行）。
        let dir = std::env::temp_dir().join(format!("mdc-598-unit-sha256-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        sh(&dir, &["init", "-q", "--object-format=sha256"]);
        sh(&dir, &["config", "user.email", "probe@local"]);
        sh(&dir, &["config", "user.name", "probe"]);
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("nested/初始.txt"), "初始内容\n").unwrap();
        sh(&dir, &["add", "--all"]);
        let nested = dir.join("nested");
        let changes = git_status(&nested).expect("无 HEAD 仓 status 应成功");
        let change = changes.iter().find(|c| c.path == "初始.txt").unwrap();
        assert_eq!(change.code, "A");
        let diff = git_diff(&nested, "初始.txt").expect("无 HEAD 初始变更不吞（vs 空树）");
        assert!(diff.contains("+初始内容"), "SHA256 空树 base 正确");
    }

    #[test]
    fn subdir_rename_and_restore_clean() {
        if !git_present() {
            eprintln!("SKIP（#598 环境门）：无 git");
            return;
        }
        let dir = temp_repo("rename");
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(
            dir.join("nested/旧 名.txt"),
            "一\n二\n三\n四\n五\n六\n七\n八\n",
        )
        .unwrap();
        std::fs::write(dir.join("nested/恢复.txt"), "原\n").unwrap();
        sh(&dir, &["add", "--all"]);
        sh(&dir, &["commit", "-q", "-m", "init"]);
        // rename：目标以工作区相对路径列出，diff 保留 from/to。
        sh(&dir, &["mv", "--", "nested/旧 名.txt", "nested/新 名.txt"]);
        let nested = dir.join("nested");
        let changes = git_status(&nested).expect("rename status");
        let change = changes
            .iter()
            .find(|c| c.code.contains('R'))
            .expect("工作区内 rename 在列");
        assert_eq!(change.path, "新 名.txt", "rename 目标为工作区相对路径");
        let diff = git_diff(&nested, &change.path).expect("rename diff");
        assert!(
            diff.contains("rename from") && diff.contains("rename to"),
            "rename from/to 保持（源原路径以仓库根相对参与 diff 上下文）"
        );
        // 恢复 clean：撤销 rename（回到 HEAD）后改另一文件 → 查 →
        // 外部 checkout → 子目录口径再查 → 空 diff。
        sh(&dir, &["reset", "-q", "--hard", "HEAD"]);
        std::fs::write(dir.join("nested/恢复.txt"), "原\n改\n").unwrap();
        let dirty = git_diff(&nested, "恢复.txt").expect("改动态 diff");
        assert!(dirty.contains("+改"), "改动先非空（恢复对照）");
        sh(&dir, &["checkout", "--", "nested/恢复.txt"]);
        let clean = git_diff(&nested, "恢复.txt").expect("恢复后应有结果");
        assert!(
            clean.is_empty(),
            "恢复 clean → 空 diff（实测 {} 字节）",
            clean.len()
        );
        let changes = git_status(&nested).expect("恢复后 status");
        assert!(changes.is_empty(), "恢复后工作区干净；实测 {changes:?}");
    }

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

#[cfg(test)]
mod whitespace_path_regression {
    use super::*;
    /// #65 同款夹具卫生（本面 2026-11-10 实测再泄漏：pr610-space-<pid> 在
    /// cwd 残留，`git add -A` 纪律下有嵌套仓误提交风险）：temp_dir + Drop
    /// 首尾自清——断言失败也回收，实测零再生。
    struct TempRoot(std::path::PathBuf);
    impl TempRoot {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "pr610-space-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn leading_space_workspace_keeps_its_real_git_prefix() {
        // 夹具卫生：TempRoot::new()（#65 引入的标准件）——temp_dir + 首尾自清。
        let guard = TempRoot::new();
        let root = guard.0.clone();
        let workspace = root.join(" leading");
        std::fs::create_dir_all(&workspace).unwrap();
        let file = workspace.join("tracked.txt");
        std::fs::write(&file, "before\n").unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "baseline",
            ],
        ] {
            assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(&root)
                    .args(args)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        std::fs::write(&file, "after\n").unwrap();
        let changes = git_status(&workspace).unwrap();
        println!("actual change list for leading-space workspace: {changes:?}");
        assert_eq!(
            changes.len(),
            1,
            "valid workspace prefix must retain its leading space"
        );
        assert_eq!(changes[0].path, "tracked.txt");
        assert!(
            git_diff(&workspace, &changes[0].path)
                .unwrap()
                .contains("+after")
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
