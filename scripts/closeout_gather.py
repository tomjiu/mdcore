#!/usr/bin/env python3
"""收口批：收集 已合并PR→仍openissue 的映射与证据"""
import json, subprocess, os

TOKEN = os.environ['GH_TOKEN']
H = ['Authorization: token ' + TOKEN, 'Accept: application/vnd.github+json']

def api(path):
    r = subprocess.run(['curl', '-s', '-H', H[0], '-H', H[1],
        f'https://api.github.com/repos/mmqz/mdcore{path}'], capture_output=True, text=True)
    return json.loads(r.stdout)

# issue → merged PR 映射（收口候选）
PAIRS = [
    (344, 376), (402, 436), (404, 442), (407, 437), (408, 424),
    (411, 446), (413, 449), (414, 455), (415, 456), (452, 458),
    (454, 457), (460, 461), (475, 476), (477, 478),
    (493, 494), (499, 501), (389, 500),
]
# 需查关闭原因的 closed-unmerged PR
CLOSED_PRS = [440, 453, 473]

print("=" * 70)
print("A. 已合并 PR → issue 状态核查")
print("=" * 70)
for issue, prn in PAIRS:
    p = api(f'/pulls/{prn}')
    i = api(f'/issues/{issue}')
    if not p.get('merged_at'):
        print(f"issue#{issue} PR#{prn}: !! PR 未合并，跳过")
        continue
    if i.get('state') != 'open':
        print(f"issue#{issue} PR#{prn}: issue 已 {i.get('state')}，无需处理")
        continue
    sha = (p.get('merge_commit_sha') or '')[:10]
    # PR 正文的验收/证据段落摘要
    body = (p.get('body') or '')
    has_accept = ('验收' in body) or ('对照' in body)
    has_evid = ('实测' in body) or ('输出' in body) or ('证据' in body)
    # issue 是否标记为分阶段
    ibody = (i.get('body') or '')
    staged = ('阶段' in ibody) or ('分批' in ibody)
    print(f"issue#{issue} [open] PR#{prn} merged@{p['merged_at'][:16]} sha={sha} 验收段={has_accept} 证据段={has_evid} issue分阶段={staged}")
    print(f"    issue标题: {i['title'][:60]}")

print()
print("=" * 70)
print("B. closed-unmerged PR 的关闭原因")
print("=" * 70)
for prn in CLOSED_PRS:
    p = api(f'/pulls/{prn}')
    print(f"PR#{prn}: merged={bool(p.get('merged_at'))} closed@{(p.get('closed_at') or '?')[:16]} [{p['title'][:55]}]")
    comments = api(f'/issues/{prn}/comments?per_page=10')
    for c in comments[-3:]:
        print(f"    [{c['user']['login']}] {(c.get('body') or '')[:150]}")
