#!/usr/bin/env python3
"""创建 PR #428（stacked on #429）"""
import json, subprocess, os

TOKEN = os.environ['GH_TOKEN']

body = """**提交者**：Agent-3ivu

> **⚠️ stacked on PR #429**（同文件测试区——#429 先合，本 PR 后合；base 皆 main，#429 合并后本 PR 需 rebase 一次，冲突面 = 本 PR 自有改动，零外来冲突）。

## 交付概要

PR for #428（本 issue 为本人 #416 抢救过程发现）。分支 `fix/428-fold-current-walkup`（基于 chore/416-draft-sweep @ 35105d9 = PR #429 头），1 commit，单文件 mang-shell/src/editor/mod.rs。

## 做了什么（对照 #428）

| #428 任务 | 交付 | 证据 |
|---|---|---|
| fold_current 补回溯 | ✅ 光标行非标题/围栏时，向上扫描最近的**包含光标行**的标题块折它——取**最内层**（从光标行向上第一处覆盖性标题即所属小节）。只回溯标题（围栏已有 fence_span_at 块内判定——不对称即消） | mod.rs fold_current ② 分支 |
| doc 注释对齐 | ✅ 重写为四分支语义表（已折叠内部幂等 / 光标行本身 / 正文行回溯 / 无结构 false），声称=实现 | 同上 doc |
| 不移动光标 | ✅ 与既有 fold_containing 路径口径一致（display 层把内部行映射到折叠头） | — |

### 测试（3 件）

```
fold_current_collapses_heading_block    → 断言翻转：#429 钉死的「正文行不折」
                                          false 断言按原稿语义翻转为 true + 区间断言
fold_current_walkup_takes_innermost_section（新）→ 嵌套 # A/## B/### C 光标在
                                          c-body → 折最内层 ### C 区间 (3,5)
fold_current_walkup_refuses_text_above_first_heading（新）→ 首标题前正文 → false
                                          （尾部正文会被末节覆盖到文末，负例取头部）
```

既有负例 `fold_current_refuses_plain_paragraph`（纯正文无结构 → false）继续通过——回溯只认标题块，不误伤。

## 实测证据（本机 Linux，rustc 1.99.0 = 仓钉版本）

```
mang-shell（独立 workspace）：
  cargo test --lib fold  → 25 passed / 0 failed
  cargo test --lib       → 478 passed / 0 failed / 5 ignored（476 基线 + 2 新增）
  cargo fmt --all -- --check → exit 0
```

## 未做的 / 有偏离的

- #427（restore_folds 接线）为姊妹缺口，仍待 Lead 裁定接线时机（装载即恢复 vs 首帧后）——不在本 PR。

## 需要 Lead 裁定的

1. 合并顺序：**#429 先、本 PR 后**（stacked 依赖，见顶部声明）。
2. （沿袭 #429）#427 接线时机语义。"""

r = subprocess.run(['curl', '-s', '-X', 'POST',
    '-H', f'Authorization: token {TOKEN}',
    '-H', 'Accept: application/vnd.github+json',
    'https://api.github.com/repos/mmqz/mdcore/pulls',
    '-d', json.dumps({
        'title': 'fix(#428): fold_current 正文行回溯所属标题块——与围栏块内判定对称（取最内层）',
        'head': 'fix/428-fold-current-walkup',
        'base': 'main',
        'body': body,
    })], capture_output=True, text=True)
d = json.loads(r.stdout)
print('PR #' + str(d.get('number')), '|', d.get('html_url'))
print('state:', d.get('state'), '| base:', d.get('base', {}).get('ref'))
if d.get('number') is None:
    print('ERROR:', d.get('message'))
