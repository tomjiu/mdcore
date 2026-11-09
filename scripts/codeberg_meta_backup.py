#!/usr/bin/env python3
"""把 agent 工作区元数据打包为孤儿分支 backup/agent-meta 推到 Codeberg.

内容：协作脚本（scripts/*.py）、issue/PR 快照与状态（work/*.json|txt|AGENTS.md）、
自动生成的 SUMMARY.md 备份说明。env.sh 与含密钥的文件一律排除。
推送带 -o ci.skip，不触发 CI。
"""
import subprocess
import os
import sys
import json
import glob
import re
import shutil

TOKEN = os.environ.get('CB_TOKEN')
if not TOKEN:
    sys.exit('need CB_TOKEN env')
CB_URL_AUTH = 'https://mingjiu:' + TOKEN + '@codeberg.org/mingjiu/mdcore.git'
SRC = '/home/z/my-project'
TMP = '/tmp/metabackup'
PLAN = json.load(open(SRC + '/work/cb_backup_plan.json'))

# ---------- 1) 选文件 ----------
files = []
files += glob.glob(SRC + '/scripts/*.py')
files += glob.glob(SRC + '/scripts/*.sh')
for pat in ('*.json', '*.txt', 'AGENTS.md'):
    files += glob.glob(SRC + '/work/' + pat)
files = sorted(set(files))
exclude = re.compile(r'env\.sh$|local-libs|tool-results')
files = [f for f in files if not exclude.search(f)]

# ---------- 2) 秘密扫描 ----------
secret = re.compile(r'ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|'
                    + TOKEN + r'|GH_TOKEN=["\x27]?[A-Za-z0-9_]{16,}')
dirty = []
for f in files:
    try:
        if secret.search(open(f, errors='ignore').read()):
            dirty.append(f)
    except OSError:
        dirty.append(f)
if dirty:
    print('排除含密钥文件:', [os.path.basename(f) for f in dirty])
    files = [f for f in files if f not in dirty]

# ---------- 3) SUMMARY.md ----------
def esc(s):
    return (s or '').replace('|', '\\|').replace('\n', ' ')[:80]

rows_new = [p for p in PLAN['plan'] + PLAN['wip'] if p['action'] in ('push_new', 'wip')]
rows_upd = [p for p in PLAN['plan'] if p['action'] == 'push_update']
rows_tag = [p for p in PLAN['plan'] if p['action'] == 'tag']
docs = PLAN['docs']

summary = []
summary.append('# mdcore 成果备份说明\n')
summary.append('- 生成时间：' + PLAN.get('generated', '') + '\n')
summary.append('- 备份目标：codeberg.org/mingjiu/mdcore（私有库）\n')
summary.append('## 背景\n')
summary.append('GitHub 账号被封禁（suspended / 403），GitHub 侧已不可读写。'
               '本地三处克隆（work/mdcore、mdcore、repo-mdcore）与 Codeberg 已迁移内容为仅存副本。\n')
summary.append('Codeberg 上已由迁移会话先行备份：main（含 GitHub main 全部历史，'
               'GitHub 最后已知 main = 958b9bb「Merge pull request #508」已并入）、'
               '约 47 个未合并分支、12 个 PR、backup/* 备份分支。\n')
summary.append('## 本次补传（共 {} 项）\n'.format(len(PLAN['plan']) + len(PLAN['wip'])))
summary.append('### 新建分支（未合并的 GitHub 快照分支 / 本地分支）\n')
summary.append('| 分支 | 提交 | 来源 | 说明 |')
summary.append('|---|---|---|---|')
for p in rows_new:
    if p['action'] != 'push_new':
        continue
    summary.append('| {} | {} | {} | {} |'.format(
        p['final_target'][len('refs/heads/'):], p['sha'][:10], p['clone'], esc(p['src'])))
summary.append('\n### 旧分支头备名（CB 同名分支已推进，旧头孤悬，存 backup/gh-*）\n')
summary.append('| 备名分支 | 提交 | 原 GitHub 分支 |')
summary.append('|---|---|---|')
for p in rows_upd:
    summary.append('| backup/gh-{} | {} | {} |'.format(
        p['target'][len('refs/heads/'):], p['sha'][:10], p['target'][len('refs/heads/'):]))
summary.append('\n### 悬空 / 仅 reflog 可达的 WIP 提交（存 backup/wip-*）\n')
summary.append('| 备份分支 | 提交 | 原始信息 |')
summary.append('|---|---|---|')
for p in PLAN['wip']:
    summary.append('| backup/wip-{} | {} | {} |'.format(
        p['sha'][:8], p['sha'][:10], esc(p['src'].replace('dangling: ', ''))))
if rows_tag:
    summary.append('\n### 标签\n')
    for p in rows_tag:
        summary.append('- `{}` -> {}'.format(p['final_target'][len('refs/tags/'):], p['sha'][:10]))
summary.append('\n## 跳过项（{} 条）\n'.format(len(docs)))
summary.append('以下引用内容已并入 Codeberg main 历史（或 CB 已有同名同 SHA 分支），无需重复推送：\n')
summary.append('| 本地引用 | 原因 |')
summary.append('|---|---|')
for ref, why in docs:
    summary.append('| {} | {} |'.format(ref[len('refs/heads/'):] if ref.startswith('refs/heads/') else ref, esc(why)))
summary.append('\n## 本分支（backup/agent-meta）内容\n')
summary.append('- `scripts/`：协作脚本工具箱（认领检查、PR 创建、CI 轮询、收口留证等）\n')
summary.append('- `work/`：issue/PR 快照 JSON、认领状态、分支-PR 映射、AGENTS.md\n')
summary.append('- `SUMMARY.md`：本说明\n')
summary.append('共 {} 个文件。env.sh（含令牌）与疑似含密钥文件已排除。\n'.format(len(files)))
SUMMARY = '\n'.join(summary)

# ---------- 4) 临时 git 仓库 ----------
shutil.rmtree(TMP, ignore_errors=True)
os.makedirs(TMP)
subprocess.run(['git', 'init', '-q', '-b', 'main', TMP], check=True)
for f in files:
    rel = os.path.relpath(f, SRC)
    dst = os.path.join(TMP, rel)
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    shutil.copy2(f, dst)
with open(os.path.join(TMP, 'SUMMARY.md'), 'w') as f:
    f.write(SUMMARY)
subprocess.run(['git', '-C', TMP, 'add', '-A'], check=True)
r = subprocess.run(['git', '-C', TMP, 'commit', '-q', '-m',
                    'agent workspace backup: scripts + issue/PR snapshots + SUMMARY (GitHub suspended, Codeberg migration)'],
                   capture_output=True, text=True)
if r.returncode != 0:
    print('commit 失败:', r.stderr[:300])

# ---------- 5) 推送 ----------
for opt in (['-o', 'ci.skip'], []):
    args = ['git', '-C', TMP, 'push'] + opt + [CB_URL_AUTH, 'main:refs/heads/backup/agent-meta']
    r = subprocess.run(args, capture_output=True, text=True)
    out = (r.stdout + r.stderr).replace(TOKEN, '***')
    if r.returncode == 0:
        print('backup/agent-meta 推送 OK')
        print(out.strip()[-500:])
        break
    if 'push option' in out:
        continue
    print('backup/agent-meta 推送 FAIL')
    print(out.strip()[-800:])
    break
print('打包文件数:', len(files))
