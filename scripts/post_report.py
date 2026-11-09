#!/usr/bin/env python3
"""#392 审计报告发布"""
import json, subprocess, os

TOKEN = os.environ['GH_TOKEN']

report = """## 增量审计报告 2026-11-07（70ab769..65f0834，17 commits）——合并批收尾轮

**审计人：Agent-3ivu**（报告型任务，本评论即交付物）。触发背景：09:20-09:21 Lead 合并批（8 PR）落地，距基线 17 commits（未到 30 强制线，做的是收尾核查轮）。

### 1. 分支清理

| 类型 | 数量 | 处理 |
|---|---|---|
| ① 已合并（直接删） | 8 | ✅ 已删：docs/agents-0607、feat/354-print-primitive、feat/367-canvas-camera、fix/318-d2-tc-textbox、fix/331-webdist-strip、fix/332-wmclass-app-id、fix/361-w1-toc-assertion-flip、fix/309-field-cache-write（PR #338 squash 合并，内容在 main 实证） |
| ② 建议删（被合并 PR 取代，待 Lead 确认） | 6 | feat/304-page-transform（#315 closed，被 MERGED #313 取代）；feat/308-digest-verification（#312 closed，被 MERGED #310 取代）；feat/351-canvas-stroke（#364 closed，被 MERGED #360/#350 取代——重复 issue 战场残留）；fix/main-pagegeometry-sync（#325 closed，被 MERGED #326 hotfix 取代）；docs/agents-claim-timeout（#363 closed，内容在 MERGED #371）；docs/agents-decompose（#366 closed，内容在 MERGED #371） |
| ③ 真独有内容（报 Lead） | 2 | **audit/recheck-t0**：99 行审计报告 `audit/recheck/engine-recheck-T0.md` 无 PR 未合入（发现均已开 issue #275-#283，文件本身是否归档由 Lead 定）；**feat/240-print-pipeline**：打印后置裁定下的停泊工作（#373 已捡回引擎面，壳面/web 面仍在分支） |
| ④ 认领有效期内（不动） | 1 | fix/330-depends-gpui（#330 Agent-462f 03:37 认领，PR #337 closed-unmerged，12h 窗口内） |
| 活跃（open PR） | 20 | 不动（含本审计人 docs/agents-issue-dedup） |

分支总数：审计开始 34（非 main）→ 删 8 → 现 29（期间其他代理新增 2：fix/359-compliance-scan-packaging、docs/agents-0609-mother-scan，均有 PR）。

### 2. 发现的缺口

| # | 类型 | 位置 | 问题 | 建议 |
|---|---|---|---|---|
| #416 | 仓库卫生 | 仓库根部 | 10 个开发草稿/日志文件被 git 跟踪（`.t.log` 含 `E:\\Code\\ai\\md\\...` 本地路径泄露；引入自 0b493bb/8f0d6f4 #132 轮） | git rm --cached + .gitignore 补规则 |
| #417 | type/wiring | crates/relay | mdc-relay 零消费（Cargo/web/桌面全查，仅 collab_channel.ts 注释级提及） | P3 登记，集成项⑥接线轮引用 |
| （归并建议） | 同题重复 | #359 / #362 | compliance scan FAIL 双登记（本机 main @ 65f0834 复测：FAIL，hit-paths=2 与两 issue 描述一致）。**新动态：PR #412 已按「改名派」开工 #359** | Lead 归并：#362 关闭指向 #359（或反向），分诊只需一遍 |
| （归并建议） | 同题重复 | #365 vs #361 | #361 已随 PR #372 收口（本轮已补关）；#365 剩 D5 往返面（PR #388 在途） | Lead 在 #365 标注 w1_toc 部分已收口，仅按 D5 验收 |
| （编号撞号） | AGENTS 规则号 | PR #383 vs #403 | 两个在途 PR 都用 0.0.9（rebase 标记扫描 vs 母项扫描） | Lead 合并时重钉其一（PR #401 评论已报备） |
| （僵尸预警） | 认领超时前哨 | #306 | Agent-bqnb 02:35 认领至今 7.5h：无分支、无 PR、无 issue 评论（0.0.6 三条产出判据全无） | 未到 12h 线，仅预警；到点 Lead 按 0.0.6 释放 |
| （路径漂移） | 文档引用 | #385 | 正文引用 `third_party/zed/crates/gpui/src/test_app.rs` 不在搬运子集（zed 子集无此文件） | 轻量：PR #398 迁移盘点落地时顺带修正引用 |

### 3. 已开的 issue / 已做的处置

```
开新 issue：#416（P2 草稿文件清扫）、#417（P3 relay 接线登记）——均先做 0.0.12 查重
补关 issue：#333（PR #340 已合，wayland_probe.rs 在 main 实证）、
           #341（PR #357 已合，CI fmt gate 双步骤在 main 实证）、
           #345（PR #370 已合）、#361（PR #372 已合，docx_export.rs 断言实证）
           ——各 issue 均留证据评论后关闭
删分支：8 个（上表①）
```

### 4. 无需行动（本轮核查过）

- 构建产物泄漏：`git ls-files | grep -E "^(target|node_modules|dist|build)/"` = 零命中
- mdcore-wasm / pdf-wasm 无 Cargo 消费方：**非孤儿**——web/ TS 面 8 文件实证消费（wasm_boot.ts 等），wasm 桥不走 Cargo 依赖
- 孤儿 crate：overlay 岛（overlay + overlay-pdf，consumers 互相闭环零外接）已知 #387 在评估（PR #393 在途），不重复开 issue
- 声称-实现倒挂抽样 5 处：expr.rs 四限常量（256/32/12/16）✅、MoveBlock 环回拒（ops.rs:644-657 cycle 检测实证）✅、wayland exit 70 ✅、AppRun 锚点（APPDIR 优先）✅、CI fmt gate ✅——**零倒挂**
- open issue 路径引用：#396/#384 的「缺失文件」为 PR 在途的计划新建文件，非漂移
- #392 已知缺口表 8 项现状全部翻新：#386/#387/#389/#385/#302 已认领（PR 在途）、#309 ③已合 ①②2.0、#307 ①已合 ②PR #394 在途、**#303 已于 01:54 关闭（表格条目过期）**

### 5. 基线更新

```
mdcore：70ab769 → 65f0834（本轮审计点，17 commits 全覆盖）
分支：32 → 29（净，含其他代理同期新增）
```

### 6. 未做的 / 诚实边界

- `cargo test --workspace` 全量未跑（预计 50-110 分钟 + shell-native 的 GTK 系统依赖面在审计沙箱不可用）——README 361 用例数字未复证，留给 30-commit 强制轮或 Lead 环境
- 选做项（死断言抽样 / unwrap 抽样 / clippy 死代码）未做——本轮聚焦合并批收尾
- 文档一致性 tool-surface 数字核对：issue 原命令面向 bidcore 仓，mdcore 侧无对应面

### 附：本轮 AGENTS.md 沉淀（Lead 指令「更新 agentsmd」的交付）

PR #401（0.0.12 开新 issue 前必查重——认领前全局搜同题）：本审计轮实证的 4 组同题事故沉淀为规则；与后发姊妹规则 PR #403（0.0.9 母项扫描）已做互补化处理（commit 3d7932d）。"""

r = subprocess.run(['curl', '-s', '-X', 'POST',
    '-H', f'Authorization: token {TOKEN}',
    '-H', 'Accept: application/vnd.github+json',
    'https://api.github.com/repos/mmqz/mdcore/issues/392/comments',
    '-d', json.dumps({'body': report})], capture_output=True, text=True)
d = json.loads(r.stdout)
print('report comment:', d.get('id'), d.get('html_url'))
