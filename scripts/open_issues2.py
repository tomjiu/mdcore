#!/usr/bin/env python3
"""开 #416 抢救过程中发现的两个新缺口 issue"""
import json, subprocess, os

TOKEN = os.environ['GH_TOKEN']

body_a = """# P2: 折叠持久化只写不读——restore_folds() 全仓零调用方，重启即丢

> 发现人：Agent-3ivu（#416 草稿抢救过程，2026-11-07）

## 现状

`mang-shell/src/editor/mod.rs` 的折叠持久化链**只有写半边**：

```
写侧（在位，5 处调用）：Cmd/key 分发 → persist_folds() → settings::save_folds
读侧（零调用）      ：pub fn restore_folds() ——全仓 grep 仅定义处一条命中
```

`restore_folds` 的 doc 注释自述「按当前文本恢复已落盘的折叠（装载/首次渲染后调用一次）」——但**没有任何装载点调它**。#146 关闭报告声称「持久化（272 测试全绿）」时，settings 读写面确有测试，但用户价值链（重启后折叠还在）从未接通。

## 用户可见后果

折叠一个长文档的若干小节 → 关闭 → 重开：**全部展开，静默丢失**。写侧还在每次折叠时持续写盘（eprintln 亦可见）——写了没人读的数据。

## 修复面（供接手人）

```
接线点：Editor::new 尾部 / 文档装载完成处（首次渲染后一次性）调
        restore_folds()；时机语义（装载即恢复 vs 首帧后恢复）需 Lead 裁定
测试  ：已有 fold_then_restore_by_head_lines（#416 抢救移植，单步切片）
        + settings.rs folds_roundtrip_per_file（落盘/读回）；
        接线后补端到端：折 → 新建 Editor 同路径 → 折叠恢复
注意  ：restore 的「宁缺毋错」语义已在实现内（头行漂移自动弃权）
```

## 关联

- #416（发现来源：草稿抢救）、#146（当初声称持久化完成的关闭报告）、#132（母任务）
- 查重：全局搜 restore_folds / 折叠 / 重启 无同题 open issue"""

body_b = """# P2: fold_current 光标在正文行不回溯所属小节——doc 注释声称了未实现的语义

> 发现人：Agent-3ivu（#416 草稿抢救过程，2026-11-07——散佚原稿暴露了语义差）

## 现状

`fold_current`（mang-shell/src/editor/mod.rs）的 doc 注释：

> 光标在折叠**内部**时先跳到折叠头（语义：折叠内部无可见行，用户按「折叠当前」应指向整个块）

实现只有**已折叠区间**的 fold_containing 回溯；**未折叠**时仅认光标行本身是标题/围栏才折。光标在正文行（小节内部）→ 返回 false。

## 证据（#416 抢救的原稿测试即钉此语义，现行为如实适配）

```
md_editor("# A\\npara1\\npara2\\n# B\\nafter")，光标 p(1,0)（A 小节正文）：
  原稿期望：fold_current() == true（折 [0,3)）
  实际    ：false（不可折）
对照    ：光标在围栏代码块内部行 → 能折（fence_span_at 覆盖块内行）
          ——围栏有「行在块内」判定，标题没有，两语义不对称
```

`fold_current_collapses_heading_block`（#416 移植）已把现行为钉死并留注释指向本 issue。

## 修复面（供接手人）

```
fold_current 的 else 分支：光标行非标题/围栏时，向上扫描最近的
  包含光标行的标题块（heading span）→ 折它；扫描到文档顶无标题 → false
与 gutter 点击折叠（fold_head_at）语义对齐即可
doc 注释与实现二选一对齐：改实现（推荐，原稿语义）或改注释
```

## 关联

- #416（发现来源 + 现行为锁定测试）、#132（母任务）
- 查重：全局搜 fold_current 无同题 issue"""

for title, body, labels in [
    ("P2: 折叠持久化只写不读——restore_folds() 零调用方，重启即丢（#416 抢救发现）", body_a, ["P2", "area/gpui", "type/wiring"]),
    ("P2: fold_current 光标在正文行不回溯所属小节——doc 注释声称未实现语义（#416 抢救发现）", body_b, ["P2", "area/gpui", "bug"]),
]:
    r = subprocess.run(['curl', '-s', '-X', 'POST',
        '-H', f'Authorization: token {TOKEN}',
        '-H', 'Accept: application/vnd.github+json',
        'https://api.github.com/repos/mmqz/mdcore/issues',
        '-d', json.dumps({'title': title, 'body': body, 'labels': labels})],
        capture_output=True, text=True)
    d = json.loads(r.stdout)
    print(f"#{d.get('number')} {title[:60]}")
    if d.get('number') is None:
        print('ERROR:', d.get('message'), d.get('errors'))
