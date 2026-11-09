#!/usr/bin/env python3
"""检查所有 open PR 的 CI 状态与 mergeable"""
import json, subprocess, os

TOKEN = os.environ['GH_TOKEN']
H = ['Authorization: token ' + TOKEN, 'Accept: application/vnd.github+json']

def api(path):
    r = subprocess.run(['curl', '-s', '-H', H[0], '-H', H[1],
        f'https://api.github.com/repos/mmqz/mdcore{path}'], capture_output=True, text=True)
    return json.loads(r.stdout)

prs = api('/pulls?state=open&per_page=100')
print(f"{'PR':>5} {'CI':>10} {'merge':>8} {'base_ok':>8}  title")
for p in prs:
    n = p['number']
    sha = p['head']['sha']
    # combined status (legacy) + check runs
    cs = api(f'/commits/{sha}/status')
    state = cs.get('state', '?')
    cr = api(f'/commits/{sha}/check-runs')
    checks = cr.get('total_check_runs', 0) if isinstance(cr, dict) else 0
    concl = set()
    if isinstance(cr, dict):
        for c in cr.get('check_runs', []):
            concl.add(c.get('conclusion') or c.get('status'))
    # mergeable
    detail = api(f'/pulls/{n}')
    mg = detail.get('mergeable')
    ms = detail.get('mergeable_state')
    ci = state if state != 'pending' else 'PENDING'
    if concl and concl != {'success'}:
        ci = f"{ci}/{'|'.join(sorted(str(x) for x in concl))[:30]}"
    print(f"#{n:<4} {ci:>10.10} {str(mg):>8} {str(ms):>8}  {p['title'][:55]}")
