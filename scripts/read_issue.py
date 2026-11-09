#!/usr/bin/env python3
"""读取指定 issue 的正文与评论"""
import json, subprocess, os, sys

TOKEN = os.environ['GH_TOKEN']
REPO = "mmqz/mdcore"

def gh(url):
    r = subprocess.run(['curl', '-s', '-H', f'Authorization: token {TOKEN}', url],
                       capture_output=True, text=True)
    return json.loads(r.stdout)

for n in sys.argv[1:]:
    i = gh(f'https://api.github.com/repos/{REPO}/issues/{n}')
    print('=' * 90)
    print(f"#{n} {i['title']}")
    print(f"created: {i['created_at']} by {i['user']['login']} | state={i['state']}")
    print('--- body (first 2500 chars) ---')
    print(i['body'][:2500] if i['body'] else '(empty)')
    comments = gh(f'https://api.github.com/repos/{REPO}/issues/{n}/comments?per_page=100')
    print(f'--- {len(comments)} comments ---')
    for c in comments:
        print(f"  [{c['created_at']}] {c['user']['login']}:")
        for line in c['body'].split('\n')[:25]:
            print(f"    {line}")
        if len(c['body'].split('\n')) > 25:
            print("    ...(truncated)")
        print()
