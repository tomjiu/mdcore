#!/usr/bin/env python3
"""僵尸候选分支的 patch 等价性判定（git cherry：-=已在main，+=真独有）"""
import json, subprocess, os

os.chdir('/home/z/my-project/work/mdcore')

def run(cmd):
    return subprocess.run(cmd, shell=True, capture_output=True, text=True).stdout.strip()

pr_map = json.load(open('/home/z/my-project/work/branch_pr_map.json'))
open_pr_map = {p['head']['ref']: p['number'] for p in json.load(open('/home/z/my-project/work/open_prs.json'))}

candidates = ['audit/recheck-t0', 'docs/agents-claim-timeout', 'docs/agents-decompose',
              'feat/240-print-pipeline', 'feat/304-page-transform', 'feat/308-digest-verification',
              'feat/351-canvas-stroke', 'fix/309-field-cache-write', 'fix/330-depends-gpui',
              'fix/main-pagegeometry-sync']

print(f"{'分支':<34} {'PR史':<22} {'独有/等价':<10} 判定")
print('-' * 100)
results = {}
for b in candidates:
    cherry = run(f'git cherry origin/main origin/{b}').split('\n')
    n_eq = sum(1 for c in cherry if c.startswith('-'))   # patch 已在 main
    n_uniq = sum(1 for c in cherry if c.startswith('+')) # 真独有
    pr_info = '无PR'
    if b in pr_map:
        n, st = pr_map[b]
        pr_info = f'#{n} {st}'
    elif b in open_pr_map:
        pr_info = f'#{open_pr_map[b]} OPEN'
    if n_uniq == 0:
        verdict = '★ 内容已全在main → 可删'
    elif b in pr_map and pr_map[b][1] == 'MERGED':
        verdict = '★ PR已合并(部分吸收) → 可删/待Lead'
    else:
        verdict = '◆ 有真独有内容 → 报Lead'
    results[b] = (n_eq, n_uniq, pr_info, verdict)
    print(f"{b:<34} {pr_info:<22} {n_uniq}/{n_eq+n_uniq:<8} {verdict}")

json.dump(results, open('/home/z/my-project/work/zombie_results.json', 'w'), ensure_ascii=False, indent=1)
