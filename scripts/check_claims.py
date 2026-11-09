#!/usr/bin/env python3
"""检查 mdcore 所有 open issues 的认领状态（AGENTS.md 0.0 清第 1 步）"""
import json, subprocess, os, re, sys

TOKEN = os.environ['GH_TOKEN']
REPO = "mmqz/mdcore"

def gh(url):
    r = subprocess.run(['curl', '-s', '-H', f'Authorization: token {TOKEN}', url],
                       capture_output=True, text=True)
    return json.loads(r.stdout)

issues = json.load(open('/home/z/my-project/work/issues1.json')) + \
         json.load(open('/home/z/my-project/work/issues2.json'))
issues = [i for i in issues if 'pull_request' not in i]

result = {}
for i in sorted(issues, key=lambda x: x['number']):
    n = i['number']
    comments = gh(f'https://api.github.com/repos/{REPO}/issues/{n}/comments?per_page=100')
    claim_info = []
    for c in comments:
        body = c['body']
        if re.search(r'认领|承接|开做|在做|已认领', body):
            first_line = body.split('\n')[0][:120]
            claim_info.append({'user': c['user']['login'], 'at': c['created_at'], 'head': first_line})
    result[n] = {
        'title': i['title'][:80],
        'labels': [l['name'] for l in i['labels']],
        'n_comments': len(comments),
        'claims': claim_info,
    }

json.dump(result, open('/home/z/my-project/work/claim_status.json', 'w'), ensure_ascii=False, indent=1)

# 打印摘要：有认领的 + 无认领的
print('=== 有认领痕迹的 issue ===')
for n, info in result.items():
    if info['claims']:
        for cl in info['claims']:
            print(f"#{n}: {cl['at']} {cl['user']}: {cl['head']}")
print()
print('=== 无认领痕迹的 issue（0 评论或纯讨论） ===')
for n, info in result.items():
    if not info['claims']:
        print(f"#{n} (comments={info['n_comments']}): {info['title']}")
