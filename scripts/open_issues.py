#!/usr/bin/env python3
"""开新 issue：仓库卫生（根部草稿文件）+ relay 接线登记"""
import json, subprocess, os

TOKEN = os.environ['GH_TOKEN']

body1 = """# P2: 仓库根部 10 个开发草稿/日志文件被 git 跟踪——含本地路径泄露

> 发现人：Agent-3ivu（#392 增量审计 固定项 4 仓库卫生，2026-11-07，main @ 65f0834）

## 现状

`git ls-files` 证实以下 10 个文件被跟踪在仓库根部，全部为开发过程草稿/日志，非交付物：

| 文件 | 大小 | 性质 |
|---|---|---|
| `.t.log` | 13.9KB | cargo check 警告日志，**内容含本地路径 `E:\\Code\\ai\\md\\audit\\_wt\\find\\third_party\\zed\\...`（Windows 开发机目录结构泄露）** |
| `_dm_add.txt` | 833B | 折叠 API 代码草稿 |
| `_foldtests.txt` | 4.9KB | 折叠测试代码草稿 |
| `_foldtests2.txt` | - | 同上（第二批） |
| `_helpers.txt` | - | 测试辅助代码草稿 |
| `_mod_fold_api.txt` | - | 模块 API 草稿 |
| `_persist.txt` | 1.8KB | 折叠持久化代码草稿 |
| `_purefns.txt` | - | 纯函数清单草稿 |
| `_settings_add.txt` | - | 设置面代码草稿 |
| `_settingstests.txt` | - | 设置面测试草稿 |

引入来源：commit `0b493bb`（fix(#132) rebase 后补…）与 `8f0d6f4`（fix(#132)）——#132/#134 折叠轮开发时把工作草稿一并提交。

## 为什么是问题

```
· 草稿内容与正式代码重复存在两份 → 读者无法判断哪份是真相
· .t.log 泄露开发机本地目录结构（E:\\Code\\...），与合规姿态不符
· 下划线开头根文件持续污染 `ls` 视野，新代理接手成本+1
```

## 范围

```
改：git rm --cached 上述 10 文件 + .gitignore 补 `_*.txt` / `*.log` / `.t.log` 规则
不改：任何正式源码；草稿若有未落地的独有内容，先由 Lead 判定是否需要抢救归档
```

## 验收

```
1. git ls-files 根部无下划线草稿文件与 .t.log
2. .gitignore 覆盖 _*.txt 与 *.log
3. compliance/scan.sh 结果不劣化（本轮 main 基线 FAIL 为 #359/#362 既有项，不计入）
```

## 关联

来源：#392 固定项 4（本轮审计报告见 #392 评论）。查重：已全局搜「草稿 / 仓库卫生 / .t.log / _foldtests」无同题 issue。"""

body2 = """# P3: mdc-relay 引擎在但零消费——协作中继状态机无宿主接线（集成项⑥登记）

> 发现人：Agent-3ivu（#392 增量审计 固定项 5 孤儿 crate，2026-11-07，main @ 65f0834）

## 现状

`crates/relay`（包名 `mdc-relay`）——协作服务端中继状态机（权威端/中继形态，集成项⑥）。

全仓消费面核查：

```
Cargo.toml 依赖：零消费方（18 个 crate 无一依赖 mdc-relay）
web/ 面：仅 web/src/collab_channel.ts:3 注释级提及（「对等网格——mdc-relay
        RelayMessage 语义的客户端侧镜像」），无代码依赖
mang-shell / 桌面：零引用
```

crate 自述定位「传输无关状态机，宿主把 RelayMessage 桥接到真实传输」——即设计上就是等宿主接线的引擎面，目前无任何宿主。

## 定性

与 overlay 孤岛（#387）同类：**引擎能力存在，产品零触达**（type/wiring）。差异：relay 属集成项⑥的规划内未来面，非缺陷回归；本 issue 仅作登记（P3 后置），供 #161 能力矩阵与后续接线轮引用。

## 建议（不阻塞 1.0）

```
· 短期：保持零依赖不动（无消费者 = 无编译成本）
· 接线轮：CRDT 协作桌面化/服务端化时，宿主侧实现 RelayMessage ↔ WebSocket/TCP 桥
· 若 2.0 前仍无消费决策，可按 #387 overlay 评估路线同批裁定
```

## 关联

来源：#392 固定项 5（本轮审计报告见 #392 评论）；已知孤儿面 overlay 岛（overlay + overlay-pdf）见 #387。查重：已全局搜「relay / 中继 / 未接线」无同题 issue。"""

for title, body, labels in [
    ("P2: 仓库根部 10 个开发草稿/日志文件被 git 跟踪——含本地路径泄露（#392 审计固定项4）", body1, ["P2", "type/debt"]),
    ("P3: mdc-relay 引擎在但零消费——协作中继状态机无宿主接线（集成项⑥登记）", body2, ["P3", "type/wiring"]),
]:
    r = subprocess.run(['curl', '-s', '-X', 'POST',
        '-H', f'Authorization: token {TOKEN}',
        '-H', 'Accept: application/vnd.github+json',
        'https://api.github.com/repos/mmqz/mdcore/issues',
        '-d', json.dumps({'title': title, 'body': body, 'labels': labels})],
        capture_output=True, text=True)
    d = json.loads(r.stdout)
    print(f"#{d.get('number')} {title[:50]} | labels={[l['name'] for l in d.get('labels', [])]}")
    if d.get('number') is None:
        print('ERROR:', d.get('message'), d.get('errors'))
