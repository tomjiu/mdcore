#!/usr/bin/env python3
"""issue 查重搜索（0.0.12 第①步）"""
import json, subprocess, os, sys, urllib.parse

TOKEN = os.environ['GH_TOKEN']

def search(q):
    url = 'https://api.github.com/search/issues?q=' + urllib.parse.quote(q, safe=':+')
    r = subprocess.run(['curl', '-s', '-H', f'Authorization: token {TOKEN}', url],
                       capture_output=True, text=True)
    try:
        return json.loads(r.stdout)
    except Exception:
        return {'total_count': -1, 'items': [], 'raw': r.stdout[:200]}

queries = [
    'repo:mmqz/mdcore 草稿 in:title,body',
    'repo:mmqz/mdcore .t.log',
    'repo:mmqz/mdcore 仓库卫生',
    'repo:mmqz/mdcore _foldtests',
    'repo:mmqz/mdcore relay in:title',
    'repo:mmqz/mdcore 中继 in:title,body',
    'repo:mmqz/mdcore 孤儿 in:title,body',
    'repo:mmqz/mdcore 未接线 in:title,body',
]
for q in queries:
    d = search(q)
    print(f'--- {q} ---')
    print(f"  total: {d['total_count']}")
    for it in d.get('items', [])[:6]:
        print(f"  #{it['number']} [{it['state']}] {it['title'][:75]}")
    if d['total_count'] == -1:
        print('  RAW:', d.get('raw'))
    print()
