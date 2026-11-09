#!/usr/bin/env python3
"""用 Actions runs 数据精确判断每个 open PR 的 CI 状态"""
import json, subprocess, os
from collections import defaultdict

TOKEN = os.environ['GH_TOKEN']
H = ['Authorization: token ' + TOKEN, 'Accept: application/vnd.github+json']

def api(path):
    r = subprocess.run(['curl', '-s', '-H', H[0], '-H', H[1],
        f'https://api.github.com/repos/mmqz/mdcore{path}'], capture_output=True, text=True)
    return json.loads(r.stdout)

# 1. 收集 open PR 头 SHA
prs = api('/pulls?state=open&per_page=100')
head_info = {}
for p in prs:
    head_info[p['head']['sha']] = (p['number'], p['title'][:50], p['head']['ref'])

# 2. 拉 actions runs（多页），按 head_sha 归并最新一次
runs_by_sha = defaultdict(list)
for page in range(1, 4):
    runs = api(f'/actions/runs?per_page=100&page={page}')
    if not runs.get('workflow_runs'):
        break
    for r in runs['workflow_runs']:
        runs_by_sha[r['head_sha']].append(r)
    if len(runs['workflow_runs']) < 100:
        break

print(f"{'PR':>5} {'commit年龄':>10} {'run状态':>10} {'结论':>10} {'事件':>16}  workflow")
for sha, (n, title, ref) in sorted(head_info.items(), key=lambda x: x[1][0]):
    rs = runs_by_sha.get(sha, [])
    if not rs:
        # 没有 run——查 commit 日期
        c = api(f'/commits/{sha}')
        date = (c.get('commit', {}).get('committer', {}) or {}).get('date', '?')[:10]
        print(f"#{n:<4} {date:>10} {'NO-RUN':>10} {'-':>10} {'-':>16}  {title}")
        continue
    r = rs[0]  # 最新
    date = (r.get('head_commit', {}) or {}).get('timestamp', '?')[:16].replace('T', ' ')
    print(f"#{n:<4} {date:>11} {r['status']:>10} {str(r['conclusion']):>10} {r['event']:>16}  {r['name'][:40]}")
    if len(rs) > 1:
        for extra in rs[1:3]:
            print(f"      {'':>11} {extra['status']:>10} {str(extra['conclusion']):>10} {extra['event']:>16}  {extra['name'][:40]}")
