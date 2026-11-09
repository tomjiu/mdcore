#!/usr/bin/env python3
"""创建 PR：AGENTS.md 0.0.12"""
import json, subprocess, os

TOKEN = os.environ['GH_TOKEN']

body = """**提交者**：Agent-3ivu

## 交付概要

AGENTS.md 新增 **0.0.12 开新 issue 前必查重——认领前全局搜同题**；头部「最后更新」同步 2026-11-07。分支 `docs/agents-issue-dedup`，1 commit，仅改 AGENTS.md（+58/-1）。

## 做了什么

**问题**：0.0 开工四查只回答「这个 issue 有没有人认领」，不回答「这个问题有没有人登记过」。今天单日 4 组同题事故全部落在这个盲区：

| 组 | 事实 | 后果 |
|---|---|---|
| #350 / #351 | 同一任务（#44 步1）两个 issue，创建相差 1 分钟 | 双认领，靠代理事后自查才发现 |
| #359 / #362 | compliance scan 红（packaging/ GPUI 词面）两代理独立发现，相差 2 分钟各开一个 | 同一问题两份登记，Lead 要分诊两遍 |
| #361 / #365 | main 红 w1_toc 断言，#365 全文包含 #361 主题再叠一项 | 两个 PR 修重叠面，互相牵制 |
| #240 任务③ / #396 | 母 issue 08:07 认领任务③、08:26 交付 PR #395；另一代理 08:35 拆成 #396 再认领 | 同题双轨，注定一个 PR 白做 |

**规则内容**（0.0.12，插在 0.0.2 之后、零之二之前）：
1. 开新 issue 前按核心词全局搜 open issue（附命令）；搜到同题 → 不新开，去先有 issue 下认领/评论
2. 认领前除 0.0 四查外再全局搜同题
3. 发现重复后的处置：后开的关自己的留指针；两边都在做 → 报 Lead 归并，不自作主张关别人的
4. 拆分场景补一查：母 issue 认领面（任务 N 已被认领在交付 → 不拆成新 issue）——补 0.0.7 三查的盲区
5. 八、快速检查清单「开工前」加一行对应项

## 实测证据

```
git diff --stat origin/main
 AGENTS.md | 59 ++++++++++++++++++++++++++++++++++++++++++++++++++++++++++-
 1 file changed, 58 insertions(+), 1 deletion(-)

grep -n '^## 0\\.0\\.' AGENTS.md   # 0.0.5/6/7/8/1/3/2 + 新增 0.0.12，无编号撞车
冲突标记扫描：无
```

## 插入点说明（为什么 0.0.12 放在 0.0.2 之后）

在途 PR #383（0.0.9）/ #391（0.0.10）/ #399（0.0.11）全部插在 0.0.8→0.0.1 边界（同一插入点，三路互相冲突，待您顺序合并）。本 PR 的插入点在 0.0.2→零之二 边界，与三者错开 ≥100 行，**任意合并顺序均零冲突**；编号 0.0.12 = 当前下一个可用号。

## 未做的 / 有偏离的

- 未顺带修任何其他文件（铁律 0：纯文档改动也走 PR，本 PR 即是）
- 头部日期从 2026-11-06 → 2026-11-07（正文实际含 11-07 规则，原日期已过期；三个在途 PR 均未动此行，无冲突）

## 需要 Lead 裁定的

1. **#359 / #362 归并**：两 issue 同题（compliance scan packaging/ GPUI 词面），均待您分诊（删除/改名/豁免三选一）。建议归并后只裁一遍。
2. **#361 / #365 归并**：#361 修复已随 PR #372 合入 main，#365 剩余面（D5 往返）有 PR #388 在途；两 issue 主题重叠，建议归并。
"""

r = subprocess.run(['curl', '-s', '-X', 'POST',
    '-H', f'Authorization: token {TOKEN}',
    '-H', 'Accept: application/vnd.github+json',
    'https://api.github.com/repos/mmqz/mdcore/pulls',
    '-d', json.dumps({
        'title': 'docs(AGENTS): 0.0.12 开新 issue 前必查重——认领前全局搜同题',
        'head': 'docs/agents-issue-dedup',
        'base': 'main',
        'body': body,
    })], capture_output=True, text=True)
d = json.loads(r.stdout)
print('PR #' + str(d.get('number')), '|', d.get('html_url'))
print('state:', d.get('state'), '| base:', d.get('base', {}).get('ref'))
if 'message' in d and d.get('number') is None:
    print('ERROR:', d['message'])
