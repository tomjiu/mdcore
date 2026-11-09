#!/usr/bin/env python3
"""#416 抢救判定：草稿内容是否已落地到正式代码"""
import subprocess, os, re

os.chdir('/home/z/my-project/work/mdcore')

files = ['.t.log', '_dm_add.txt', '_foldtests.txt', '_foldtests2.txt', '_helpers.txt',
         '_mod_fold_api.txt', '_persist.txt', '_purefns.txt', '_settings_add.txt',
         '_settingstests.txt']

def run(cmd):
    return subprocess.run(cmd, shell=True, capture_output=True, text=True).stdout

for f in files:
    text = open(f, encoding='utf-8', errors='ignore').read()
    # 抽特征：函数名/标识符（snake_case，>=6 字符）
    ids = set(re.findall(r'\b([a-z][a-z0-9_]{5,})\b', text))
    ids -= {'return', 'unwrap', 'assert', 'string', 'option', 'result', 'self',
            'expect', 'fn_ctx', 'assert_eq', 'matches', 'vec_u8', 'println'}
    # 过滤常见非代码词
    ids = {i for i in ids if not i.startswith('test_') or len(i) > 10}
    probes = sorted(ids)[:8]
    if not probes:
        print(f'{f}: 无法抽取特征（跳过）')
        continue
    hit = {p: bool(run(f'git grep -q -w "{p}" -- "crates/*/src/*.rs" "crates/*/tests/*.rs"')) for p in probes}
    landed = sum(hit.values())
    print(f'{f}: 特征 {len(probes)} 个，命中 {landed} 个 ' +
          ('→ 已落地' if landed >= len(probes)*0.5 else '→ ⚠️ 半数以下命中，需人工看'))
    for p, h in hit.items():
        if not h:
            print(f'    未命中: {p}')
