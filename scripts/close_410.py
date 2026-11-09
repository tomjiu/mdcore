#!/usr/bin/env python3
"""#410 核查报告 + 关闭"""
import json, subprocess, os

TOKEN = os.environ['GH_TOKEN']

claim = """认领：Agent-3ivu
类型：报告型（核查确认——issue 第一步「先确认是否真的累积」即本任务全部内容）
阶段：单阶段
首份产出预计：即刻（下方报告）

按 0.0 四查 + 0.0.12 同题搜索无冲突后接取。"""

report = """## 核查结论：P1-12 已由 #279 修复——本 issue 立项依据为研究仓陈旧表格行

**Agent-3ivu 核查报告（issue 第一步「先确认是否真的累积」的交付）**

### 证据链

**① 修复在档**：commit `065921a` `fix(#279): worker 换代上限 MAX_GENERATIONS=16——敌意反复超时的线程/引擎实例硬上界（审计 P1-12）`——正是本 issue 证据栏引用的 P1-12（研究仓 acrobat/mdcore-mapping.md「worker 线程累积（P1-12）」对应行）。worker.rs:80 `pub const MAX_GENERATIONS: u64 = 16` + 换代上限耗尽即退役态（exhausted：不再派生新代，submit 即刻合成 Timeout）。

**② 既有验证测试（本机刚复跑）**：

```
cargo test -p mdc-pdf --features engine-lopdf --test pdfc_cache_worker worker_generation_cap
  test worker_generation_cap_bounds_stuck_threads ... ok
  test result: ok. 1 passed; 0 failed（2026-11-07 本机 Linux）
```

该测试（pdfc_cache_worker.rs:1089）钉死的正是本 issue 要的断言：代数封顶 ≤ MAX_GENERATIONS（含首代）、退役线程计数有界（≤ 上限）、退役态 submit 合成 Timeout 不悬挂。

**③ 无「每页一个线程」**：全 pdf crate 线程派生点仅 3 处（`git grep thread::spawn|thread::Builder`）：worker.rs:200（构造，1 线程）/ :541（换代派生）/ :571（看门狗）——全部在 worker 生命周期内；每代线程**串行服务多个作业**，与页数无关。

**④ 产品路径零消费 RenderWorker**：`RenderWorker` 全仓消费方 = 测试三件（pdfc_cache_worker / pdfc_engine / pdfc_perf）+ document.rs:268 一处 doc 注释提及。桌面 PDF 视图（mang-shell/src/editor/pdf_doc.rs，998 行）零渲染/零线程——只读结构视图；web 渲染走 pdf.js（pdf-wasm 自述「渲染分离真相不分离：pdf.js 纯显示面」）。「长期打开大 PDF 内存持续上涨」在当前产品路径不可能由 worker 引发。

### 处置

按 AGENTS 0.0 判定「有已合并 PR → issue 该关」关闭。**边界如实声明**：若未来 #302（PDF 消费倒挂）接线轮把 RenderWorker 接进产品后出现真实内存增长，请带实测数据（retired_alive / 代数 / 线程数）另立 issue——本 issue 的 worker.rs 面已由 #279 + 既有测试闭环。"""

def post(n, body):
    r = subprocess.run(['curl', '-s', '-X', 'POST',
        '-H', f'Authorization: token {TOKEN}',
        '-H', 'Accept: application/vnd.github+json',
        f'https://api.github.com/repos/mmqz/mdcore/issues/{n}/comments',
        '-d', json.dumps({'body': body})], capture_output=True, text=True)
    return json.loads(r.stdout).get('id')

def close(n):
    r = subprocess.run(['curl', '-s', '-X', 'PATCH',
        '-H', f'Authorization: token {TOKEN}',
        '-H', 'Accept: application/vnd.github+json',
        f'https://api.github.com/repos/mmqz/mdcore/issues/{n}',
        '-d', json.dumps({'state': 'closed'})], capture_output=True, text=True)
    return json.loads(r.stdout).get('state')

print('claim:', post(410, claim))
print('report:', post(410, report))
print('closed:', close(410))
