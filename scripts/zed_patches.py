#!/usr/bin/env python3
"""third_party/zed 的补丁层应用器（patches/zed/*.patch）。

两层机制（issue #137，照 ElyOffice vendor-gpui.sh 模式）：
  第一层  zed_port.py 搬运（上游 → third_party/zed，Cargo.toml 继承展开）
  第二层  patches/zed/*.patch 缺陷修复（本脚本管理，Ely 格式纯 unified diff：
          标题 + 正文说明 + diff，可 git apply 于仓根）

子命令：
  apply    应用全部补丁到 third_party/zed（跳过已打；遇不匹配即失败）
  check    门禁（CI）：每个补丁均处「已打」态 → 退出 0；
           有「未打」→ 1；有「不匹配」→ 2；清单不一致 → 3
  status   人读状态表

探测方法（git apply 三态，无需上游树）：
  git apply --check <p>          成功 → 未打（可正向应用）
  git apply --check --reverse <p> 成功 → 已打（可反向回退 = 树已含此修复）
  两者皆败                       → 不匹配（补丁与本树对不上：搬运漂移或手改）

上游 pin 升级时的复用流程：
  git fetch 上游 && git checkout <新 pin>
  python3 scripts/zed_port.py            # 第一层搬运
  python3 scripts/zed_patches.py apply   # 第二层补丁（上游已修的补丁会
                                          # 显示 forward 失败+reverse 失败 →
                                          # 人工核对后从目录摘除并更新
                                          # PROVENANCE.md，即「上游收编即移除」）
"""
import subprocess
import sys
import os

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PATCH_DIR = os.path.join(REPO_ROOT, "patches", "zed")
PROVENANCE = os.path.join(REPO_ROOT, "third_party", "zed", "PROVENANCE.md")


def patches_on_disk():
    if not os.path.isdir(PATCH_DIR):
        return []
    return sorted(
        f for f in os.listdir(PATCH_DIR) if f.endswith(".patch")
    )


def probe(name):
    """返回 'applied' | 'unapplied' | 'mismatch'。"""
    path = os.path.join(PATCH_DIR, name)

    def try_apply(*extra):
        return subprocess.run(
            ["git", "apply", "--check", *extra, path],
            cwd=REPO_ROOT,
            capture_output=True,
        ).returncode == 0

    if try_apply("--reverse"):
        return "applied"
    if try_apply():
        return "unapplied"
    return "mismatch"


def provenance_listed():
    """PROVENANCE.md 补丁层清单段里登记的补丁文件名。"""
    listed = []
    if not os.path.exists(PROVENANCE):
        return listed
    in_sec = False
    for line in open(PROVENANCE, encoding="utf-8"):
        if line.startswith("## 补丁层（patches/zed/"):
            in_sec = True
            continue
        if in_sec and line.startswith("## "):
            break
        if in_sec:
            cell = line.split("|")
            if len(cell) > 2:
                cand = cell[1].strip().strip("`")
                if cand.endswith(".patch"):
                    listed.append(cand)
    return listed


def cmd_apply():
    names = patches_on_disk()
    if not names:
        print("patches/zed/ 无补丁文件，无可应用。")
        return 0
    fail = False
    for name in names:
        state = probe(name)
        if state == "applied":
            print(f"[跳过] {name}（已打）")
        elif state == "unapplied":
            r = subprocess.run(
                ["git", "apply", os.path.join(PATCH_DIR, name)],
                cwd=REPO_ROOT,
            )
            if r.returncode == 0:
                print(f"[应用] {name}")
            else:
                print(f"[失败] {name}（git apply 退出 {r.returncode}）")
                fail = True
        else:
            print(f"[失败] {name} 与树不匹配（正向/反向探测均失败）——"
                  "上游已修或树漂移，人工核对后更新清单。")
            fail = True
    return 1 if fail else 0


def cmd_check():
    names = patches_on_disk()
    if not names:
        print("FAIL: patches/zed/ 目录不存在或为空")
        return 3
    listed = provenance_listed()
    if sorted(set(listed)) != names:
        print("FAIL: PROVENANCE.md 补丁层清单与 patches/zed/ 目录不一致")
        print(f"  目录: {names}")
        print(f"  清单: {listed}")
        return 3
    worst = 0
    for name in names:
        state = probe(name)
        print(f"{state:>9}  {name}")
        if state == "unapplied":
            worst = max(worst, 1)
        elif state == "mismatch":
            worst = max(worst, 2)
    if worst:
        print(f"FAIL (exit {worst})")
    else:
        print(f"OK: {len(names)} 个补丁全部已打、与清单一致")
    return worst


def cmd_status():
    names = patches_on_disk()
    if not names:
        print("patches/zed/ 无补丁文件。")
        return 0
    for name in names:
        print(f"{probe(name):>9}  {name}")
    return 0


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "status"
    if cmd == "apply":
        sys.exit(cmd_apply())
    elif cmd == "check":
        sys.exit(cmd_check())
    elif cmd == "status":
        sys.exit(cmd_status())
    else:
        print(__doc__)
        sys.exit(1)


if __name__ == "__main__":
    main()
