#!/usr/bin/env python3
"""内容级判定：分支触碰的文件 vs main 当前版本是否实质一致"""
import json, subprocess, os

os.chdir('/home/z/my-project/work/mdcore')

def run(cmd):
    return subprocess.run(cmd, shell=True, capture_output=True, text=True).stdout.strip()

pr_map = json.load(open('/home/z/my-project/work/branch_pr_map.json'))
candidates = ['audit/recheck-t0', 'docs/agents-claim-timeout', 'docs/agents-decompose',
              'feat/240-print-pipeline', 'feat/304-page-transform', 'feat/308-digest-verification',
              'feat/351-canvas-stroke', 'fix/309-field-cache-write', 'fix/330-depends-gpui',
              'fix/main-pagegeometry-sync']

print(f"{'分支':<33} {'触碰文件数':>6} {'vs main 残差':>10}  判定")
print('-' * 95)
out = {}
for b in candidates:
    mb = run(f'git merge-base origin/main origin/{b}')
    files = [f for f in run(f'git diff --name-only {mb} origin/{b}').split('\n') if f]
    if not files:
        verdict = '空分支 → 可删'
        residual = 0
    else:
        filelist = ' '.join(f'"{f}"' for f in files)
        residual = int(run(f'git diff origin/main origin/{b} -- {filelist} | wc -l'))
        if residual == 0:
            verdict = '★ 内容已全在main → 可删'
        elif residual < 30:
            verdict = f'△ 残差{residual}行(≈版本演进) → 报Lead备删'
        else:
            verdict = f'◆ 残差{residual}行(真独有) → 报Lead'
    pr = pr_map.get(b, ('无PR',''))[0]
    out[b] = {'files': len(files), 'residual': residual, 'pr': pr, 'verdict': verdict,
              'touched': files[:6]}
    print(f"{b:<33} {len(files):>6} {residual:>10}  {verdict}  (PR史:#{pr})")

json.dump(out, open('/home/z/my-project/work/content_check.json', 'w'), ensure_ascii=False, indent=1)
print()
for b, d in out.items():
    if 0 < d['residual'] <= 30:
        print(f"--- {b} 残差明细 ---")
        mb = run(f'git merge-base origin/main origin/{b}')
        files = [f for f in run(f'git diff --name-only {mb} origin/{b}').split('\n') if f]
        for f in files:
            r = int(run(f'git diff origin/main origin/{b} -- "{f}" | wc -l'))
            if r:
                print(f'  {f}: {r} 行差异')
