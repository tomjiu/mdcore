//! #588 —— git_diff 真实文件状态分类验收（真实临时 Git 仓）。
//!
//! 调用**生产公开** [`mang_shell::git_repo::git_diff`]（#588 拆出进
//! lib 树——集成测试可达），在每测试独立的真实临时仓上按验收四条
//! 逐场景断言。缺陷对照（main f36b40e 基线，独立探针 11 场景 5 红）
//! 在 issue #588 评论在案：clean tracked 误报新增 / 恢复 clean 复发 /
//! 无 HEAD 双场景吞初始变更 / 空格+Unicode clean 误报。
//!
//! 环境门（AGENTS 诚实边界）：git 缺失环境跳过全场景（横幅明示，不
//! 伪装通过）——CI windows-latest 与开发机恒有 git。

use std::path::{Path, PathBuf};
use std::process::Command;

use mang_shell::git_repo::git_diff;

// ------------------------------------------------------------------
// 夹具：真实临时仓（每测试独立；进程号隔离防并行互踩）
// ------------------------------------------------------------------

fn sh(root: &Path, args: &[&str]) {
    let out = Command::new("git")
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

/// 建仓（已 config 身份——commit 可跑）。
fn repo(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mdc-588-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    sh(&dir, &["init", "-q"]);
    sh(&dir, &["config", "user.email", "probe@local"]);
    sh(&dir, &["config", "user.name", "probe"]);
    dir
}

/// 建仓 + 首提（含一个种子文件——HEAD 在场）。
fn repo_with_head(tag: &str, seed: &str, seed_content: &str) -> PathBuf {
    let dir = repo(tag);
    std::fs::write(dir.join(seed), seed_content).unwrap();
    sh(&dir, &["add", seed]);
    sh(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

/// git 门（缺 git 全跳——不伪装）。
fn git_present() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

// ------------------------------------------------------------------
// 验收 1：真实 Git 仓——干净/已改/已暂存/恢复四态
// ------------------------------------------------------------------

#[test]
fn clean_tracked_file_yields_empty_diff() {
    if !git_present() {
        eprintln!("SKIP（#588 环境门）：无 git");
        return;
    }
    let dir = repo_with_head("clean", "probe.txt", "第一行\n第二行\n");
    let d = git_diff(&dir, "probe.txt").expect("clean tracked 应有结果（非运行失败）");
    assert!(
        d.is_empty(),
        "干净已跟踪 → 空 diff（#588 核心缺陷面；实测 {} 字节，含 new file mode={}",
        d.len(),
        d.contains("new file mode")
    );
}

#[test]
fn modified_tracked_yields_real_hunk() {
    if !git_present() {
        eprintln!("SKIP（#588 环境门）：无 git");
        return;
    }
    let dir = repo_with_head("mod", "probe.txt", "旧\n");
    std::fs::write(dir.join("probe.txt"), "旧\n新行\n").unwrap();
    let d = git_diff(&dir, "probe.txt").expect("modified 应有结果");
    assert!(
        d.contains("+新行"),
        "已修改 tracked → 正确新增行（实测 {:?}）",
        &d[..d.len().min(300)]
    );
    assert!(!d.contains("new file mode"), "已跟踪修改不是新文件");
}

#[test]
fn staged_tracked_yields_real_hunk() {
    if !git_present() {
        eprintln!("SKIP（#588 环境门）：无 git");
        return;
    }
    let dir = repo_with_head("staged", "probe.txt", "一\n");
    std::fs::write(dir.join("probe.txt"), "一\n二\n").unwrap();
    sh(&dir, &["add", "probe.txt"]);
    let d = git_diff(&dir, "probe.txt").expect("staged 应有结果");
    assert!(
        d.contains("+二"),
        "已暂存 tracked → 暂存差异可见（合并 diff 语义）"
    );
}

#[test]
fn restored_clean_yields_empty_diff_again() {
    if !git_present() {
        eprintln!("SKIP（#588 环境门）：无 git");
        return;
    }
    let dir = repo_with_head("restore", "probe.txt", "一\n");
    // 改 → 查 → 外部还原（checkout）→ 再查：状态分类必须取实时
    // porcelain（不能沿用旧输出）——#588 现象段「刷新后被外部提交或
    // 还原」的回归面。
    std::fs::write(dir.join("probe.txt"), "一\n改\n").unwrap();
    let _ = git_diff(&dir, "probe.txt");
    sh(&dir, &["checkout", "--", "probe.txt"]);
    let d = git_diff(&dir, "probe.txt").expect("恢复后应有结果");
    assert!(
        d.is_empty(),
        "恢复 clean → 再次空 diff（实测 {} 字节）",
        d.len()
    );
}

// ------------------------------------------------------------------
// 验收 2：untracked 与无 HEAD 新仓
// ------------------------------------------------------------------

#[test]
fn untracked_yields_full_new_content() {
    if !git_present() {
        eprintln!("SKIP（#588 环境门）：无 git");
        return;
    }
    let dir = repo_with_head("untracked", "seed.txt", "种\n");
    std::fs::write(dir.join("new.txt"), "全新\n内容\n").unwrap();
    let d = git_diff(&dir, "new.txt").expect("untracked 应有结果");
    assert!(d.contains("+全新"), "未跟踪 → 新增内容（+ 行）");
    assert!(d.contains("new file mode"), "未跟踪 → new file 形态");
}

#[test]
fn no_head_staged_keeps_initial_change() {
    if !git_present() {
        eprintln!("SKIP（#588 环境门）：无 git");
        return;
    }
    let dir = repo("nohead-staged");
    std::fs::write(dir.join("init.txt"), "初始\n").unwrap();
    sh(&dir, &["add", "init.txt"]);
    let d = git_diff(&dir, "init.txt").expect("无 HEAD staged 应有结果（初始变更不吞）");
    assert!(
        d.contains("+初始"),
        "无 HEAD 新仓 staged → 初始变更可见（vs 空树）"
    );
}

#[test]
fn no_head_untracked_yields_new_content() {
    if !git_present() {
        eprintln!("SKIP（#588 环境门）：无 git");
        return;
    }
    let dir = repo("nohead-untracked");
    std::fs::write(dir.join("u.txt"), "无头新\n").unwrap();
    let d = git_diff(&dir, "u.txt").expect("无 HEAD untracked 应有结果");
    assert!(d.contains("+无头新"), "无 HEAD untracked → 新增内容");
}

#[test]
fn no_head_staged_plus_edit_sees_both_segments() {
    if !git_present() {
        eprintln!("SKIP（#588 环境门）：无 git");
        return;
    }
    // AM 复合：暂存版 + 后续未暂存修改——vs 空树的合并 diff 两段都可见。
    let dir = repo("nohead-am");
    std::fs::write(dir.join("a.txt"), "暂存版\n").unwrap();
    sh(&dir, &["add", "a.txt"]);
    std::fs::write(dir.join("a.txt"), "暂存版\n又改\n").unwrap();
    let d = git_diff(&dir, "a.txt").expect("AM 复合应有结果");
    assert!(
        d.contains("+暂存版") && d.contains("+又改"),
        "无 HEAD staged+后改 → 完整变更（两段可见）"
    );
}

// ------------------------------------------------------------------
// 验收 3：缺文件 / 二进制 / 空格与 Unicode 路径 / git 失败
// ------------------------------------------------------------------

#[test]
fn missing_file_is_diagnosable_not_fake_diff() {
    if !git_present() {
        eprintln!("SKIP（#588 环境门）：无 git");
        return;
    }
    let dir = repo_with_head("missing", "a.txt", "x\n");
    if let Some(t) = git_diff(&dir, "不存在.txt") {
        // 明确结果两可：空（无状态）或 None（诊断性缺席）——都合法；
        // 非法 = 伪 diff（把缺席当新增展示）。
        assert!(
            t.is_empty(),
            "缺文件 → 空或 None，不产伪 diff（实测 {} 字节）",
            t.len()
        );
    }
}

#[test]
fn binary_modified_is_labeled_binary() {
    if !git_present() {
        eprintln!("SKIP（#588 环境门）：无 git");
        return;
    }
    let dir = repo_with_head("binary", "b.bin", "not-binary-seed");
    let bytes: &[u8] = &[0u8, 1, 2, 3];
    std::fs::write(dir.join("b.bin"), bytes).unwrap();
    sh(&dir, &["add", "b.bin"]);
    sh(&dir, &["commit", "-q", "-m", "seed-as-text"]);
    let new_bytes: &[u8] = &[9u8, 9, 9, 9, 9, 9, 9, 9];
    std::fs::write(dir.join("b.bin"), new_bytes).unwrap();
    let d = git_diff(&dir, "b.bin").expect("二进制修改应有结果");
    assert!(
        d.contains("Binary files") || d.contains("binary"),
        "二进制 modified → Binary 标记（实测 {:?}）",
        &d[..d.len().min(200)]
    );
}

#[test]
fn unicode_and_space_path_clean_and_modified() {
    if !git_present() {
        eprintln!("SKIP（#588 环境门）：无 git");
        return;
    }
    let name = "带 空格 名.txt";
    let dir = repo_with_head("uni", name, "中文\n");
    // clean 态：空 diff（不误报——基线在此 273 字节伪新增）。
    let d = git_diff(&dir, name).expect("Unicode clean 应有结果");
    assert!(
        d.is_empty(),
        "空格+Unicode clean → 空 diff（实测 {} 字节——基线误报 273）",
        d.len()
    );
    // modified 态：差异正确。
    std::fs::write(dir.join(name), "中文\n追加\n").unwrap();
    let d2 = git_diff(&dir, name).expect("Unicode modified 应有结果");
    assert!(d2.contains("+追加"), "空格+Unicode modified → 差异正确");
}

#[test]
fn non_git_directory_returns_diagnostic_none() {
    if !git_present() {
        eprintln!("SKIP（#588 环境门）：无 git");
        return;
    }
    let dir = std::env::temp_dir().join(format!("mdc-588-nogit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("x.txt"), "x\n").unwrap();
    // 非仓（无 .git）：git status --porcelain 失败 → None（诊断性失败，
    // 不产伪结果）。注意 tmp 根可能恰在某仓内——探测一下保确定性。
    let inside_repo = Command::new("git")
        .args([
            "-C",
            dir.to_string_lossy().as_ref(),
            "rev-parse",
            "--is-inside-work-tree",
        ])
        .output()
        .is_ok_and(|o| o.status.success());
    if inside_repo {
        eprintln!("SKIP：tmp 在某 git 工作树内（环境偶发）——非仓面不可测");
        return;
    }
    assert_eq!(git_diff(&dir, "x.txt"), None, "非 git 目录 → 诊断性 None");
}

// ------------------------------------------------------------------
// 出口语径：合法 diff 的 exit 1 不视为失败（--no-index 面）
// ------------------------------------------------------------------

#[test]
fn no_index_exit_one_still_returns_stdout() {
    if !git_present() {
        eprintln!("SKIP（#588 环境门）：无 git");
        return;
    }
    // untracked 有差异时 `git diff --no-index` exit 1——stdout 为准
    // （非运行失败）：本测试即验收「exit1 不当失败吞输出」。
    let dir = repo_with_head("exit1", "seed.txt", "种\n");
    std::fs::write(dir.join("big.txt"), "多行\n内容\n在这里\n").unwrap();
    let d = git_diff(&dir, "big.txt").expect("--no-index exit 1 ≠ 运行失败");
    assert!(
        d.contains("+多行") && d.contains("+在这里"),
        "stdout 完整返回"
    );
}

#[test]
fn status_path_roundtrip_unicode_space() {
    if !git_present() {
        eprintln!("SKIP: git unavailable");
        return;
    }
    let dir = repo_with_head("status-path", "seed.txt", "seed\n");
    let name = "带 空格 名.txt";
    std::fs::write(dir.join(name), "literal-path\n").unwrap();
    sh(&dir, &["config", "core.quotepath", "true"]);
    let changes = mang_shell::git_repo::git_status(&dir).unwrap();
    let change = changes.iter().find(|c| c.code == "??").unwrap();
    assert_eq!(
        change.path, name,
        "actual Git status path is operational, not C-quoted display text"
    );
    let diff = git_diff(&dir, &change.path).expect("actual panel path yields diff");
    assert!(diff.contains("+literal-path"));
}

#[test]
fn leading_dash_untracked_status_path_yields_diff() {
    if !git_present() {
        eprintln!("SKIP: git unavailable");
        return;
    }
    let dir = repo_with_head("leading", "seed.txt", "seed\n");
    let name = "-lead.txt";
    std::fs::write(dir.join(name), "leading-dash\n").unwrap();
    let changes = mang_shell::git_repo::git_status(&dir).unwrap();
    let change = changes.iter().find(|c| c.path == name).unwrap();
    let diff = git_diff(&dir, &change.path).expect("leading '-' path is not an option");
    assert!(diff.contains("+leading-dash"));
}

#[test]
fn renamed_status_path_is_destination_and_diff_keeps_rename() {
    if !git_present() {
        eprintln!("SKIP: git unavailable");
        return;
    }
    let dir = repo_with_head("rename-z", "old.txt", "one\ntwo\nthree\nfour\n");
    let name = "重命名 文件.txt";
    sh(&dir, &["mv", "--", "old.txt", name]);
    let changes = mang_shell::git_repo::git_status(&dir).unwrap();
    let change = changes.iter().find(|c| c.code.contains('R')).unwrap();
    assert_eq!(
        change.path, name,
        "rename destination, not 'old -> new' display text"
    );
    assert_eq!(changes.len(), 1, "source NUL record is not a second change");
    let diff = git_diff(&dir, &change.path).expect("renamed path remains readable");
    assert!(diff.contains("rename from") && diff.contains("rename to"));
}

#[test]
fn copy_status_source_record_is_not_a_change() {
    if !git_present() {
        eprintln!("SKIP: git unavailable");
        return;
    }
    let content = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n";
    let dir = repo_with_head("copy-z", "source.txt", content);
    let name = "复制 文件.txt";
    std::fs::write(dir.join(name), content).unwrap();
    std::fs::write(dir.join("source.txt"), format!("changed\n{content}")).unwrap();
    sh(&dir, &["config", "status.renames", "copies"]);
    sh(&dir, &["add", "--", "source.txt", name]);
    let changes = mang_shell::git_repo::git_status(&dir).unwrap();
    let copied = changes
        .iter()
        .find(|c| c.code.contains('C'))
        .expect("real Git reports copy");
    assert_eq!(copied.path, name);
    assert_eq!(
        changes.len(),
        2,
        "one source modification and one copy, no extra source record"
    );
    let diff = git_diff(&dir, &copied.path).unwrap();
    assert!(diff.contains("+one"));
    assert!(
        !diff.contains("+changed"),
        "copy diff must not include the source's independent modification"
    );
}

#[test]
fn no_head_sha256_uses_repository_empty_tree() {
    if !git_present() {
        eprintln!("SKIP: git unavailable");
        return;
    }
    let dir = std::env::temp_dir().join(format!("mdc-588-sha256-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    sh(&dir, &["init", "-q", "--object-format=sha256"]);
    std::fs::write(dir.join("initial.txt"), "initial-sha256\n").unwrap();
    sh(&dir, &["add", "--", "initial.txt"]);
    let diff =
        git_diff(&dir, "initial.txt").expect("unborn SHA256 repo must not use SHA1 empty tree");
    assert!(diff.contains("+initial-sha256"));
}

#[cfg(unix)]
#[test]
fn status_path_roundtrip_quotes_and_newline() {
    if !git_present() {
        eprintln!("SKIP: git unavailable");
        return;
    }
    let dir = repo_with_head("quoted-newline", "seed.txt", "seed\n");
    let name = "引号\"和\n换行.txt";
    std::fs::write(dir.join(name), "special-literal\n").unwrap();
    let changes = mang_shell::git_repo::git_status(&dir).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].path, name);
    assert!(
        git_diff(&dir, &changes[0].path)
            .unwrap()
            .contains("+special-literal")
    );
}

#[test]
fn literal_filename_does_not_match_sibling_glob() {
    if !git_present() {
        eprintln!("SKIP: git unavailable");
        return;
    }
    let dir = repo("literal-pathspec");
    std::fs::write(dir.join("r[1].txt"), "literal\n").unwrap();
    std::fs::write(dir.join("r1.txt"), "sibling\n").unwrap();
    sh(&dir, &["add", "--all"]);
    sh(&dir, &["commit", "-q", "-m", "literal fixture"]);
    std::fs::write(dir.join("r1.txt"), "sibling\nchanged\n").unwrap();
    let diff = git_diff(&dir, "r[1].txt").expect("exact tracked filename has a result");
    assert!(
        diff.is_empty(),
        "unchanged literal r[1].txt must not pick r1.txt changes: {diff}"
    );
    assert!(git_diff(&dir, "r1.txt").unwrap().contains("+changed"));
}

#[test]
fn no_index_execution_failure_returns_none() {
    if !git_present() {
        eprintln!("SKIP: git unavailable");
        return;
    }
    let dir = repo_with_head("no-index-failure", "seed.txt", "seed\n");
    std::fs::write(dir.join("new.txt"), "new-content\n").unwrap();
    sh(
        &dir,
        &["config", "diff.external", "mdcore-nonexistent-diff-tool"],
    );
    let direct = Command::new("git")
        .args([
            "-C",
            dir.to_str().unwrap(),
            "diff",
            "--no-index",
            "--",
            "/dev/null",
            "new.txt",
        ])
        .output()
        .unwrap();
    assert!(
        !matches!(direct.status.code(), Some(0 | 1)),
        "real external diff fails to execute"
    );
    assert_eq!(
        git_diff(&dir, "new.txt"),
        None,
        "execution failure must not appear as empty success"
    );
}
