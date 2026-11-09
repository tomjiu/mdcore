#!/usr/bin/env python3
"""#392 阶段一：分支分类（已合并/完全落后/僵尸/活跃）"""
import json, subprocess, os

os.chdir('/home/z/my-project/work/mdcore')

def run(cmd):
    return subprocess.run(cmd, shell=True, capture_output=True, text=True).stdout.strip()

# open PR 的 head 分支集合
open_prs = json.load(open('/home/z/my-project/work/open_prs.json'))
pr_branches = {p['head']['ref']: p['number'] for p in open_prs}

# 我自己的分支也算活跃
mine = 'docs/agents-issue-dedup'

branches = [b for b in run('git branch -r --format="%(refname:short)"').split('\n')
            if b and b != 'origin' and b != 'origin/main' and 'HEAD' not in b]
branches = [b.replace('origin/', '', 1) for b in branches]

merged = set(run('git branch -r --merged origin/main --format="%(refname:short)"').split('\n'))
merged = {b.replace('origin/', '', 1) for b in merged if b}

rows = []
for b in sorted(branches):
    ahead = int(run(f'git rev-list --count origin/main..origin/{b}'))
    behind = int(run(f'git rev-list --count origin/{b}..origin/main'))
    is_merged = b in merged
    has_pr = b in pr_branches
    if has_pr or b == mine:
        cls = '活跃(open PR)' + (f' #{pr_branches[b]}' if has_pr else ' (本代理 PR #401)')
    elif is_merged:
        cls = '① 已合并→可删'
    elif ahead == 0:
        cls = '② 完全落后→可删'
    else:
        cls = '③ 僵尸(独有commit无PR)→报Lead'
    last = run(f'git log -1 --format="%ci" origin/{b}')[:10]
    rows.append((b, ahead, behind, cls, last))

print(f"{'分支':<38} {'独有':>4} {'落后':>4}  分类                        最后推送")
print('-' * 110)
for b, a, bh, cls, last in rows:
    print(f"{b:<38} {a:>4} {bh:>4}  {cls:<28} {last}")

print()
print('汇总：')
from collections import Counter
c = Counter(r[3].split('(')[0].split('→')[0] for r in rows)
for k, v in c.items():
    print(f'  {k}: {v}')
