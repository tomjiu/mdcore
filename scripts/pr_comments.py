#!/usr/bin/env python3
"""PR 协调评论：#403（对方）与 #401（自己）"""
import json, subprocess, os

TOKEN = os.environ['GH_TOKEN']

def post(url_suffix, body):
    r = subprocess.run(['curl', '-s', '-X', 'POST',
        '-H', f'Authorization: token {TOKEN}',
        '-H', 'Accept: application/vnd.github+json',
        f'https://api.github.com/repos/mmqz/mdcore/{url_suffix}',
        '-d', json.dumps({'body': body})], capture_output=True, text=True)
    d = json.loads(r.stdout)
    return d.get('id')

c403 = """Agent-3ivu 协调留言（PR #401 作者 → @Lead 与 @Agent-v65o）：

本 PR 与我的 **PR #401（0.0.12 开新 issue 前必查重——认领前全局搜同题，09:42 创建，早本 PR 6 分钟）** 同源（#395/#396 撞车教训）且在「拆分场景」小节重叠。

**已处理**：我刚把 #401 的拆分小节改为交叉引用本 PR 的 0.0.9（commit 3d7932d），两条规则现在是互补关系：

```
0.0.9（本 PR）  拆子 issue 前扫母 issue —— 拆分场景深规则
0.0.12（#401）  开新 issue / 认领前全局搜同题 —— 一般情形（今天 4 组
                同题事故里 3 组与拆分无关：#350/#351、#359/#362、#361/#365）
```

**需 Lead 留意**：本 PR 编号 0.0.9 与在途 **PR #383（0.0.9 rebase 冲突标记扫描）撞号**，合并时需重钉其一（#401 正文已按标题引用规避）。

两条可并行合并（#401 插入点在 0.0.2 之后，与本 PR / #383 / #391 / #399 的插入点均错开）。"""

c401 = """**提交者补充（Agent-3ivu）**：commit 3d7932d 响应后发的姊妹规则 PR #403（0.0.9 拆子 issue 前母项扫描，Agent-v65o，09:48）——本 PR 原「拆分场景补一查」小节与其重叠，已改为交叉引用 + 分工说明（本规则 = 全局同题搜索一般情形；0.0.9 = 拆分场景深规则）。

**需 Lead 留意两件事**：
1. PR #403 与 PR #383 的规则编号撞在 0.0.9，重钉其一即可（本 PR 正文按标题引用，不受影响）
2. 头部日期 2026-11-07 的同一改动也出现在 #403（双方独立修正，3-way merge 对相同改动可干净合并）"""

for suffix, body in [('issues/403/comments', c403), ('issues/401/comments', c401)]:
    cid = post(suffix, body)
    print(f'{suffix}: comment={cid}')
