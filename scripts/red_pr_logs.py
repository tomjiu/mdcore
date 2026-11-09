#!/usr/bin/env python3
"""拉取红 PR 的失败 job 日志关键行"""
import json, subprocess, os

TOKEN = os.environ['GH_TOKEN']
H = ['Authorization: token ' + TOKEN, 'Accept: application/vnd.github+json']

def api(path):
    r = subprocess.run(['curl', '-s', '-H', H[0], '-H', H[1],
        f'https://api.github.com/repos/mmqz/mdcore{path}'], capture_output=True, text=True)
    return json.loads(r.stdout)

RED = [445, 470, 481, 487, 444]

for n in RED:
    p = api(f'/pulls/{n}')
    sha = p['head']['sha']
    runs = api(f'/actions/runs?head_sha={sha}&per_page=20').get('workflow_runs', [])
    print(f"{'='*72}")
    print(f"PR #{n}: {p['title'][:60]}")
    print(f"head {sha[:10]} branch {p['head']['ref']}")
    for r in runs:
        if r['conclusion'] == 'failure':
            run_id = r['id']
            wf = r['name']
            jobs = api(f'/actions/runs/{run_id}/jobs?per_page=20').get('jobs', [])
            for j in jobs:
                if j['conclusion'] == 'failure':
                    print(f"  [红] {wf} / job={j['name']} id={j['id']}")
                    steps = [s for s in j.get('steps', []) if s.get('conclusion') == 'failure']
                    for s in steps:
                        print(f"    失败步骤: {s['name']}")
                    # 拉日志关键行
                    lr = subprocess.run(['curl', '-s', '-L', '-H', f'Authorization: token {TOKEN}',
                        f"https://api.github.com/repos/mmqz/mdcore/actions/jobs/{j['id']}/logs"],
                        capture_output=True, text=True)
                    lines = lr.stdout.splitlines()
                    hits = []
                    for i, line in enumerate(lines):
                        if any(k in line for k in ['error[', 'error:', 'FAILED', 'panicked', 'test result:', 'warning: unused', 'Diff in']):
                            hits.append(line.strip()[:180])
                    for h in hits[:18]:
                        print(f"      | {h}")
                    if not hits:
                        print(f"      | (无关键词命中，日志 {len(lines)} 行，末尾 5 行：)")
                        for line in lines[-5:]:
                            print(f"      | {line.strip()[:180]}")
