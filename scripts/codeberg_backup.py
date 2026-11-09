#!/usr/bin/env python3
"""Codeberg 成果备份推送器.

背景：GitHub 账号被封禁（suspended，403），本地三克隆 + Codeberg 已迁移内容是仅存副本。
cb/main (83d63d8) 已含 GitHub main 全史（958b9bb 并入），本脚本补齐 CB 缺失的成果：
  1) 本地分支 / GitHub 快照分支：CB 无该名→按原名新建；同名同 SHA→跳过；
     同名不同 SHA：内容不在 CB 历史时先试 FF 更新，失败转 backup/gh-<名>-<sha8> 备名；
     内容已在 CB 历史→跳过指针（记录到 docs）
  2) 悬空 / 仅 reflog 可达的 WIP 提交（日期下限 2026-10-25）→ backup/wip-<sha8>
  3) 标签按原 refname 推送
不触发 CI：全部 -o ci.skip（Gitea push option），仓库 has_actions=False，不建任何 PR。

用法：CB_TOKEN=... python3 codeberg_backup.py          # 只打印计划
      CB_TOKEN=... python3 codeberg_backup.py --apply # 执行推送并校验
"""
import subprocess
import sys
import os
import json
from collections import OrderedDict, defaultdict

APPLY = '--apply' in sys.argv
TOKEN = os.environ.get('CB_TOKEN')
if not TOKEN:
    sys.exit('need CB_TOKEN env')
CB_URL = 'https://codeberg.org/mingjiu/mdcore.git'
CB_URL_AUTH = 'https://mingjiu:' + TOKEN + '@codeberg.org/mingjiu/mdcore.git'
CLONES = OrderedDict([
    ('work', '/home/z/my-project/work/mdcore'),
    ('sec',  '/home/z/my-project/mdcore'),
    ('repo', '/home/z/my-project/repo-mdcore'),
])
ANALYSIS = CLONES['repo']
FLOOR = '2026-10-25'
CRED = 'credential.helper=!f() { echo username=mingjiu; echo password=' + TOKEN + '; }; f'


def git(repo, *args, check=True):
    r = subprocess.run(['git', '-C', repo, *args], capture_output=True, text=True)
    if check and r.returncode != 0:
        raise RuntimeError('git -C {} {}: {}'.format(repo, ' '.join(args[:4]), r.stderr.strip()[:300]))
    return r.stdout


def redact(s):
    return s.replace(TOKEN, '***')


def obj_in_analysis(sha):
    return subprocess.run(['git', '-C', ANALYSIS, 'cat-file', '-e', sha + '^{commit}'],
                          capture_output=True).returncode == 0


def cb_push(clone, refspecs, force=False):
    """推送一组 refspec，返回 (ok, output)；push option 不被支持时自动降级。"""
    base = ['git', '-C', CLONES[clone], '-c', CRED, 'push']
    last = ''
    for use_opt in (True, False):
        args = base + (['-o', 'ci.skip'] if use_opt else []) + \
            (['--force'] if force else []) + [CB_URL_AUTH] + list(refspecs)
        r = subprocess.run(args, capture_output=True, text=True)
        last = redact((r.stdout + r.stderr)[-3000:])
        if r.returncode == 0:
            return True, last
        if use_opt and 'push option' in last:
            continue
        return False, last
    return False, last


# ---------- CB 现状 ----------
cb_branches = {}
_CB_PREFIX = 'refs/remotes/cb/'
for line in git(ANALYSIS, 'for-each-ref', 'refs/remotes/cb', '--format=%(refname) %(objectname)').splitlines():
    n, s = line.split()
    cb_branches[n[len(_CB_PREFIX):]] = s
cb_tags = {}
r = subprocess.run(['git', '-C', ANALYSIS, 'ls-remote', CB_URL_AUTH, 'refs/tags/*'],
                   capture_output=True, text=True)
for line in r.stdout.splitlines():
    s, ref = line.split('\t')
    cb_tags[ref[len('refs/tags/'):]] = s

refs_cb = ['refs/remotes/cb/{}'.format(n) for n in cb_branches]
cb_reachable = set(git(ANALYSIS, 'rev-list', *refs_cb).split())
print('CB: {} 分支, {} 标签, 可达提交 {} 个'.format(len(cb_branches), len(cb_tags), len(cb_reachable)))

# ---------- 本地候选 ----------
cands = []
for ck, path in CLONES.items():
    for line in git(path, 'for-each-ref', '--format=%(refname) %(objectname)').splitlines():
        parts = line.split()
        if len(parts) < 2:
            continue
        ref, sha = parts[0], parts[1]
        if ref.startswith('refs/remotes/cb/'):
            continue
        if ref.startswith('refs/heads/'):
            n = ref[11:]
            if n == 'main':
                continue  # 本地 main 均为陈旧状态，cb/main 已包含
            cands.append(dict(clone=ck, name=n, sha=sha, kind='branch', ref=ref))
        elif ref.startswith('refs/remotes/origin/'):
            n = ref[len('refs/remotes/origin/'):]
            if not n or n == 'HEAD':
                continue
            if n == 'main':
                cands.append(dict(clone=ck, name=None, sha=sha, kind='ghmain', ref=ref))
            else:
                cands.append(dict(clone=ck, name=n, sha=sha, kind='snap', ref=ref))
        elif ref.startswith('refs/tags/'):
            cands.append(dict(clone=ck, name=ref[10:], sha=sha, kind='tag', ref=ref))

seen = set()
uniq = []
for c in cands:
    k = (c['name'], c['sha'])
    if k in seen:
        continue
    seen.add(k)
    uniq.append(c)
print('本地候选 {} 条 → 去重后 {} 条'.format(len(cands), len(uniq)))

# ---------- 决策 ----------
plan = []
docs = []
for c in uniq:
    sha, n, kind = c['sha'], c['name'], c['kind']
    if kind == 'tag':
        if cb_tags.get(n) == sha:
            docs.append((c['ref'], 'CB 已有同 SHA 标签'))
        else:
            plan.append(dict(clone=c['clone'], sha=sha, target='refs/tags/' + n,
                             refspec='{}:{}'.format(c['ref'], c['ref']), action='tag',
                             final_target='refs/tags/' + n, src=c['ref']))
        continue
    if kind == 'ghmain':
        if sha in cb_reachable:
            docs.append((c['ref'], 'GitHub main 历史已并入 cb/main'))
        else:
            t = 'refs/heads/backup/github-main-{}'.format(sha[:8])
            plan.append(dict(clone=c['clone'], sha=sha, target=t,
                             refspec='{}:{}'.format(sha, t), action='push_new',
                             final_target=t, src=c['ref']))
        continue
    if cb_branches.get(n) == sha:
        docs.append((c['ref'], 'CB 已有同名同 SHA 分支'))
        continue
    in_cb_hist = (sha in cb_reachable) if obj_in_analysis(sha) else None
    if n in cb_branches:
        if in_cb_hist:
            docs.append((c['ref'], '内容已并入 CB 历史（CB 分支 {} 占名），跳过指针'.format(cb_branches[n][:8])))
            continue
        plan.append(dict(clone=c['clone'], sha=sha, target='refs/heads/' + n,
                         refspec='{}:refs/heads/{}'.format(sha, n), action='push_update',
                         alt='backup/gh-{}-{}'.format(n, sha[:8]),
                         final_target='refs/heads/' + n, src=c['ref']))
    else:
        if in_cb_hist:
            docs.append((c['ref'], '内容已并入 cb/main 历史，跳过指针'))
            continue
        plan.append(dict(clone=c['clone'], sha=sha, target='refs/heads/' + n,
                         refspec='{}:refs/heads/{}'.format(sha, n), action='push_new',
                         final_target='refs/heads/' + n, src=c['ref']))

# 同目标冲突守卫（理论上不应出现）
by_target = defaultdict(list)
for p in plan:
    by_target[p['target']].append(p)
for t, ps in by_target.items():
    if len(ps) > 1:
        print('WARN 同目标多源 {}: {}'.format(t, [(x['clone'], x['sha'][:8]) for x in ps]))
        plan = [p for p in plan if p is not ps[0]]

# ---------- 悬空 / reflog WIP 抢救 ----------
planned_commits = [p['sha'] for p in plan if p['action'] != 'tag']
pr_set = set()
testable = [s for s in planned_commits if obj_in_analysis(s)]
if testable:
    pr_set = set(git(ANALYSIS, 'rev-list', *testable).split())

wip_plan = []
wip_seen = set()
for ck, path in CLONES.items():
    all_reach = set(git(path, 'rev-list', '--all').split())
    reflog_only = set(git(path, 'rev-list', '--reflog', '--not', '--all').split())
    fsck_out = git(path, 'fsck', '--unreachable', '--no-reflogs', check=False)
    fsck_only = {l.split()[2] for l in fsck_out.splitlines() if l.startswith('unreachable commit')}
    pool = (reflog_only | fsck_only) - all_reach
    kept = 0
    for s in sorted(pool):
        if s in pr_set or s in wip_seen:
            continue
        if obj_in_analysis(s) and s in cb_reachable:
            continue
        date = git(path, 'show', '-s', '--format=%cI', s)[:10]
        if date < FLOOR:
            continue
        wip_seen.add(s)
        t = 'refs/heads/backup/wip-{}'.format(s[:8])
        subj = git(path, 'show', '-s', '--format=%s', s)[:60]
        wip_plan.append(dict(clone=ck, sha=s, target=t, refspec='{}:{}'.format(s, t),
                             action='wip', final_target=t, src='dangling: ' + subj))
        kept += 1
    print('{}: 悬空/reflog 独有 {}，抢救 {}'.format(ck, len(pool), kept))

# ---------- 计划输出 ----------
cnt = defaultdict(int)
for p in plan + wip_plan:
    cnt[p['action']] += 1
print('计划: ' + ', '.join('{}={}'.format(k, v) for k, v in sorted(cnt.items())))
for p in plan + wip_plan:
    print('  {:8s} {:40s} {} [{}] <- {}'.format(p['action'], p['final_target'], p['sha'][:8], p['clone'], p['src']))
print('跳过 {} 条（已并入 CB / 同 SHA）'.format(len(docs)))

with open('/home/z/my-project/work/cb_backup_plan.json', 'w') as f:
    json.dump(dict(generated=subprocess.run(['date', '-Is'], capture_output=True, text=True).stdout.strip(),
                   cb_branches_before=cb_branches, plan=plan, wip=wip_plan, docs=docs),
              f, ensure_ascii=False, indent=1, default=str)

# ---------- 执行 ----------
if APPLY:
    # Phase A: 新建分支 + WIP 备份（批量）
    for ck in CLONES:
        batch = [p for p in plan + wip_plan if p['clone'] == ck and p['action'] in ('push_new', 'wip')]
        if not batch:
            continue
        for i in range(0, len(batch), 40):
            chunk = batch[i:i + 40]
            ok, out = cb_push(ck, [p['refspec'] for p in chunk])
            print('[{}] 批量新建 {} 条: {}'.format(ck, len(chunk), 'OK' if ok else 'FAIL'))
            if not ok:
                print(out)
    # Phase B: 更新已有分支（逐个，失败转备名）
    for p in [q for q in plan if q['action'] == 'push_update']:
        ok, out = cb_push(p['clone'], [p['refspec']])
        if ok:
            print('[{}] FF 更新 {}: OK'.format(p['clone'], p['target']))
        else:
            alt = p['alt']
            ok2, out2 = cb_push(p['clone'], ['{}:refs/heads/{}'.format(p['sha'], alt)])
            p['final_target'] = 'refs/heads/' + alt if ok2 else p['final_target']
            print('[{}] 更新 {} 失败 → 备名 {}: {}'.format(p['clone'], p['target'], alt, 'OK' if ok2 else 'FAIL'))
            if not ok2:
                print(out2)
    # Phase C: 标签
    for ck in CLONES:
        tags = [p for p in plan if p['clone'] == ck and p['action'] == 'tag']
        if tags:
            ok, out = cb_push(ck, [p['refspec'] for p in tags])
            print('[{}] 标签 {} 条: {}'.format(ck, len(tags), 'OK' if ok else 'FAIL'))
            if not ok:
                print(out)
    # 校验
    r = subprocess.run(['git', '-C', ANALYSIS, 'ls-remote', CB_URL_AUTH],
                       capture_output=True, text=True)
    remote = {}
    for line in r.stdout.splitlines():
        s, ref = line.split('\t')
        remote[ref] = s
    miss = [(p['final_target'], p['sha'][:8], (remote.get(p['final_target']) or '-')[:8])
            for p in plan + wip_plan if remote.get(p['final_target']) != p['sha']]
    print('校验: 计划 {} 条, 未达成 {} 条'.format(len(plan) + len(wip_plan), len(miss)))
    for m in miss:
        print('  MISS {} want={} got={}'.format(*m))
    with open('/home/z/my-project/work/cb_backup_plan.json', 'w') as f:
        json.dump(dict(generated=subprocess.run(['date', '-Is'], capture_output=True, text=True).stdout.strip(),
                       cb_branches_before=cb_branches, plan=plan, wip=wip_plan, docs=docs,
                       missed=miss),
                  f, ensure_ascii=False, indent=1, default=str)
