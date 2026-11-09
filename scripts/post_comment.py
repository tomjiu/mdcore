#!/usr/bin/env python3
"""在 issue 上发布评论"""
import json, subprocess, os, sys

TOKEN = os.environ['GH_TOKEN']
number = sys.argv[1]
body = open(sys.argv[2]).read()

r = subprocess.run(['curl', '-s', '-X', 'POST',
    '-H', f'Authorization: token {TOKEN}',
    '-H', 'Accept: application/vnd.github+json',
    f'https://api.github.com/repos/mmqz/mdcore/issues/{number}/comments',
    '-d', json.dumps({'body': body})], capture_output=True, text=True)
d = json.loads(r.stdout)
print('comment id:', d.get('id'), '| at:', d.get('html_url'))
if d.get('id') is None:
    print('ERROR:', d.get('message'))
