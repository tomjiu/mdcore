#!/usr/bin/env python3
"""合并前检查：PR 评论 + review，然后合并 clean+green 的 PR"""
import json, subprocess, os, sys

TOKEN = os.environ['GH_TOKEN']
H = ['Authorization: token ' + TOKEN, 'Accept: application/vnd.github+json']

def api(path, method='GET', data=None):
    cmd = ['curl', '-s', '-X', method, '-H', H[0], '-H', H[1],
        f'https://api.github.com/repos/mmqz/mdcore{path}']
    if data is not None:
        cmd += ['-d', json.dumps(data)]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if not r.stdout.strip():
        return {'_empty': True, '_status_ok': r.returncode == 0}
    try:
        return json.loads(r.stdout)
    except json.JSONDecodeError:
        return {'_parse_error': r.stdout[:200]}

TARGETS = [494, 500, 501]  # clean + 三工作流全绿
dry = '--go' not in sys.argv

for n in TARGETS:
    # 1. issue comments（PR 本身也是 issue，评论走 issues API）
    comments = api(f'/issues/{n}/comments?per_page=20')
    # 2. review comments
    reviews = api(f'/pulls/{n}/reviews?per_page=20')
    blocker = []
    for c in comments:
        t = (c.get('body') or '')
        if any(k in t for k in ['必须修', '必须改', '这个要改', 'blocked', '不要合并', '先别合']):
            blocker.append(('comment', c['user']['login'], t[:80]))
    for rv in reviews:
        if rv.get('state') == 'CHANGES_REQUESTED':
            blocker.append(('review', rv['user']['login'], (rv.get('body') or '')[:80]))
    p = api(f'/pulls/{n}')
    print(f"#{n}: mergeable={p.get('mergeable')} state={p.get('mergeable_state')} comments={len(comments)} reviews={len(reviews)} blockers={blocker}")
    if blocker:
        print(f"  !! 有未处理阻塞评论，跳过合并")
        continue
    if dry:
        print(f"  [dry] 将合并")
        continue
    # 3. 合并（merge commit，保留分支删除随后做）
    m = api(f'/pulls/{n}/merge', 'PUT', {'merge_method': 'merge'})
    print(f"  merged: {m.get('merged')} sha={m.get('sha')} msg={m.get('message')}")
    if m.get('merged'):
        # 4. 删分支
        ref = p['head']['ref']
        d = api(f'/git/refs/heads/{ref}', 'DELETE')
        print(f"  删分支 {ref}: {'OK' if d.get('_status_ok') or d.get('_empty') else d.get('message')}")
