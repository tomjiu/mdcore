# AGENTS.md — 代理协作规则（两仓通用）

> 本文件是**唯一权威**的代理协作规则。与 issue 正文冲突时以本文件为准。
> 最后更新：2026-11-10（迁移到自部署 Forgejo；0.0.11 / 0.0 / 0.0.2 / 0.0.3 / 0.0.7 / 0.0.12 / 0.0.13 / 0.0.14 / 二之二 / 二之三 命令全部改为 API）

---

# 负一、当前协作平台（2026-11-10 起，最高优先）

> **GitHub 已停用**（`mmqz` 账号被 suspended，不可恢复）。所有代码提交走**自部署 Git 服务**。

## -1.1 唯一提交入口

```
地址：https://git.mingjiu.store
账号：sandbox-agent
令牌：见任务派发时下发的 TOKEN（不入库、不写文件）
```

**六个仓库都在 `sandbox` 组织下**：

| 仓库 | 说明 |
|---|---|
| `sandbox/mdcore` | 文档引擎（PDF/DOCX/XLSX/Canvas/mang-shell GPUI） |
| `sandbox/ai-bid-research` | 标书应用（bid-mcp/bid-domain/mang-app） |
| `sandbox/kokoa` | Kokoa 浏览器 |
| `sandbox/mang-research-archive` | 研究仓 |
| `sandbox/agents-hub` | Agent 中心 |
| `sandbox/cpa-multi-plugins` | CPA 插件集 |

## -1.2 提交方式：HTTPS + Token，不用 SSH

```bash
# 克隆
git clone https://sandbox-agent:<TOKEN>@git.mingjiu.store/sandbox/mdcore.git
cd mdcore

# 开分支（必须带身份前缀，防撞车）
git checkout -b <你的身份>/<issue号>-<简短描述>
# 例：git checkout -b agent-x7m2/601-extract

# 改代码 → 提交
git add -A
git commit -m "feat(#601): 一句话说清"

# 推分支
git push origin <你的身份>/<issue号>-<简短描述>
```

**为什么不用 SSH**：

```
· 服务器 22 端口有 fail2ban，容易被封
· HTTPS + token 全程走 443，不受影响
· 证书是 Let's Encrypt，不需要 http.sslVerify=false
```

## -1.3 开 PR

**方式 A：网页**（推荐）

```
https://git.mingjiu.store/sandbox/<仓库>/pulls
点 New Pull Request，选你的分支 → base 选 main
```

**方式 B：API**

```bash
curl -X POST "https://git.mingjiu.store/api/v1/repos/sandbox/mdcore/pulls" \
  -H "Authorization: token <TOKEN>" \
  -H "Content-Type: application/json" \
  -d '{"title":"feat(#601): xxx","head":"<你的分支>","base":"main"}'
```

## -1.4 不跑 CI

**本平台不配置、不触发任何 CI。**

```
· Forgejo 未启用 Actions
· 提交不会触发任何流水线
· 代码正确性由你自己本地验证
```

**必须在 PR 描述里写明你跑了什么**：

```markdown
## 本地验证
- cargo check --workspace → 0 error
- cargo test -p mdcore → 123 passed
- 或 python -m pytest → 305 passed
```

## -1.5 main 自动同步到云托管

```
你的 push → git.mingjiu.store
              ↓ 服务器每 2 分钟自动同步
         Codeberg（公开）+ CNB（私有）
```

**你只需要往 `git.mingjiu.store` 推。不要直接推 Codeberg / CNB——会被覆盖。**

| 云托管 | 同步范围 | 说明 |
|---|---|---|
| Codeberg | main only | 3 个文档引擎仓库，公开 |
| CNB | main only | 5 个仓库，私有 |

## -1.6 红线

```
❌ 不要 push 到 Codeberg / CNB（会被服务器自动同步覆盖）
❌ 不要 push 到 GitHub（账号已封，不该再依赖）
❌ 不要 push main 分支（走 PR，同 §0 规矩）
❌ 不要在代码/提交里写 token
❌ 不要期待 CI 反馈——没有 CI
```

## -1.7 与下文旧规则的关系

```
下文 §0 ~ §八 的流程规则（认领、PR 纪律、rebase、
冲突处理、测试环境门等）全部继续有效。

只有「用哪个平台」和「用什么命令」变了：
  PR 创建              → 网页 / curl POST .../pulls（见 -1.3）
  issue 查询           → curl "$FJ/repos/$ORG/<repo>/issues?labels=..."（见 0.0.11）
  其他 API             → https://git.mingjiu.store/api/v1/...
  origin = github.com → origin = git.mingjiu.store
```

---


---


---

# 零、最高优先级铁律（违反即违规）

## 0. 代码完成 → 提交 PR，**永不直接合并 main**

```
❌ 绝对禁止：
  git checkout main && git merge <branch> && git push origin main
  git push origin HEAD:main
  任何形式的「直接推 main」

✅ 唯一正确流程：
  1. 在功能分支上提交
  2. push 功能分支
  3. 开 PR（网页 / REST API，见 -1.3）
  4. 等 Lead 审核合并
```

**为什么**：

```
· main 是唯一可信基线，任何未经审核的改动都可能破坏它
· 直接推 main 绕过审核 = 其他代理的 stale base 事故源头
· PR 是审核痕迹，没有 PR 就无法追溯「谁在什么时候改了什么」
· 回滚单个 PR 容易，回滚被污染的 main 极难
```

**如果分支与 main 冲突**：

```
✅ 正确：git fetch origin main && git rebase origin/main && push 功能分支（--force）
❌ 错误：把 main 合进分支后直接推 main
```

**例外：无。** 即使是「一行文档改动」「紧急修复」也必须走 PR。


---


## 0.0 开工前必查清单（缺一步就是违规）

**在写任何代码之前，必须依次做完这四步：**

```bash
# 环境（见 0.0.11）
export FJ=https://git.mingjiu.store/api/v1
export FJT=<TOKEN>
export ORG=sandbox
export REPO=mdcore          # 或 ai-bid-research

# 第 1 步：这个 issue 有没有人已经认领？
#   查评论含『认领』二字的 + 是否已打 status/claimed
curl -s -H "Authorization: token $FJT" "$FJ/repos/$ORG/$REPO/issues/<N>/comments" \
  | python3 -c "import sys,json; [print(c['user']['login'],':',c['body'][:60]) for c in json.load(sys.stdin) if '认领' in c['body']]"

curl -s -H "Authorization: token $FJT" "$FJ/repos/$ORG/$REPO/issues/<N>" \
  | python3 -c "import sys,json; d=json.load(sys.stdin); print('state=',d['state']); print('labels=',[l['name'] for l in d['labels']])"

# 第 2 步：这个 issue 有没有已存在的 PR？
curl -s -H "Authorization: token $FJT" "$FJ/repos/$ORG/$REPO/pulls?state=all&limit=100" \
  | python3 -c "
import sys, json
for p in json.load(sys.stdin):
    if '#<N>' in p['title']:
        print('#%d %s → %s%s' % (p['number'], p['title'][:50], p['state'], ' MERGED' if p['merged_at'] else ''))
"

# 第 3 步：这个 issue 有没有对应的分支？
curl -s -H "Authorization: token $FJT" "$FJ/repos/$ORG/$REPO/branches?limit=100" \
  | python3 -c "import sys,json; [print(b['name']) for b in json.load(sys.stdin) if '<N>' in b['name']]"

# 第 4 步：全局同题搜——有没有别处已登记同一问题（防同题撞车，详见 0.0.12）
#   Forgejo 无全文搜索 API，改为拉全量 open issue 本地过滤
curl -s -H "Authorization: token $FJT" "$FJ/repos/$ORG/$REPO/issues?state=open&limit=100" \
  | python3 -c "import sys,json; [print('#%d %s' % (i['number'],i['title'])) for i in json.load(sys.stdin) if '<核心词>' in i['title']]"
```

**判定规则：**

```
有 open PR        → 停！去帮那个 PR 收尾，不要新开
有已合并 PR       → 停！issue 该关了，不要重做
有分支在推送      → 停！可能别人在做，先问 Lead
有认领但 >12h 不动 → 报告 Lead 决定，不要自己抢
全都没有          → ✅ 可以开工
```

**违反后果：**

```
重复 PR 会被 Lead 直接关闭，你的工作白做；
如果造成 main 冲突，额外增加 Lead 的清理成本。
2026-11-07 单日出现 5 组重复 PR（#334/#335、#343/#344、#342/#349、#346/#353、#395/#396）。
第 5 组（母项命名空间撞车）见 0.0.9——四查抓不住，必须扫母 issue。
```

## 0.0.5 给结构体加字段后必须检查所有字面量

**`#[serde(default)]` 只让反序列化兼容——Rust 的结构体字面量初始化仍是全字段必需。**

```rust
// 你以为加了 serde(default) 就兼容了：
#[serde(default)]
pub crop_box: Option<PageBox>,

// 但所有旧字面量全部编译失败：
PageGeometry {
    width_pt: 612.0,
    height_pt: 792.0,
    rotation: Rotation::Deg0,
}  // → error[E0063]: missing fields crop_box and media_box
```

**2026-11-07 单日三次同类事故：**

```
① Anchor 加 block_idx/page/line（#285②）→ 6 个分支编译失败
② ParagraphData 加 resolved_number（#266）→ PR #299 红
③ PageGeometry 加 media_box/crop_box（#304）→ main 红，所有 PR 继承
```

**加字段后必做：**

```bash
# 1. 找出所有字面量构造点
grep -rn "StructName {" --include="*.rs" crates/

# 2. 两选一
#    方案 A（推荐）：给结构体加 impl Default，字面量改成 ..Default::default()
#    方案 B：逐个补字段
```

**为什么推荐 A：**

```rust
impl Default for Foo {
    fn default() -> Self { Self { /* 所有字段的合理默认 */ } }
}

// 以后所有调用方：
Foo {
    existing_field: value,
    ..Default::default()   // 新增字段不用逐个跟
}
```

**mdcore 仓有现成工具查风险面：**

```bash
./scripts/struct-field-guard.ps1 -Repo .
# 列出所有加过 serde(default) 字段的结构体 + 它们的字面量构造点
```

**报告样例：**

```
Struct                         新增字段       字面量点         定义文件
[!] PageGeometry                 2          17           page.rs
[!] EvidenceRecord               1          7            evidence.rs
合计: 8 个加过字段的结构体 / 42 处字面量构造点
```

---
## 0.0.6 认领超时自动释放 + 报告型任务的产出定义

**认领不是占位——超过时限无产出，Lead 会直接释放认领，让其他代理接。**

## 判定标准（认领后 12 小时）

```
看下面三条，任一条成立即为「有效产出」：

  ✅ 有 PR                                    → 正常（可能在等 CI，CI 本身跑 1-2 小时）
  ✅ 有分支在推送                              → 正常
  ✅ issue 里有实质性报告评论（>500 字的交付物）  → 正常（报告型任务）

  三条全无                                    → 僵尸认领，Lead 直接释放
```

## 两类任务，产出的定义不同

```
A. 代码型任务（大多数）
   产出 = 分支 + PR

B. 报告型任务（黑盒考验 / 审计 / 调研 / 验收）
   产出 = issue 里的报告评论
   没 PR 是正常的——这类任务本来就不改代码
```

**报告型任务的例子**：

```
ai-bid #269  引擎可用性黑盒考验
  认领 39 小时，无 PR——但 issue 里有 5 份报告：
    · 批次一修复前基线（六场跑完）
    · 复跑·合并前预验证
    · 黑盒试跑报告（考题设计验证）
  -> 不是僵尸，是长期研究任务
```

## 报告型任务的认领格式

```
认领：<身份>
类型：报告型（不改代码 / 只产出报告）
阶段：<分几个阶段>
首份产出预计：<多久出第一份报告>
```

**每个阶段出报告时：**

```
· 直接在 issue 里贴报告（不要只存在本地）
· 标题写清阶段（如「阶段二：XX 实测结果」）
```

## Lead 的处理

```
① 在 issue 留言「认领已释放」+ 现状说明
② 删除原认领评论（避免后人误判已有人在座）
③ 其他代理可自由接取
④ 原认领人回来可重新认领（不追责，但别静默挂着）
```

## 为什么这么严（2026-11-07 实测）

```
mdcore #240  认领 23 小时，分支写一半，无 PR
ai-bid #320  认领 13.6 小时，零产出（无分支/无 PR/无评论）

后果：
  · 任务池显示「已认领」，其他代理不敢碰
  · 实际上没人在做
  · 任务就这么挂着
```

## 卡住 / 不做了 怎么办

```
卡住  -> 立刻留言：卡在哪、需要什么、还要多久
不做了 -> 留言「释放认领」+ 说明分支状态

不要静默。静默 = 僵尸认领。
```

---

## 0.0.7 没有可接任务时，从长期文档拆功能独立项

**任务池空了的正确反应不是「没活干」，是去拆长期文档。**

## 什么样的文档可以拆

```
✅ 可以拆（它们本身不可交付，但内部有清楚的分层）
   例：mdcore #240（打印管线）——三层：引擎面 / 壳面 / web 面
       mdcore #44 （画布）    ——七篇调研 + 落地顺序步1-N
       mdcore #318（DOCX 缺陷）——D1-D7 逐条
       ai-bid #307（假绿总纲）——FG-01~18 逐条

❌ 不能拆（没有可拆的结构）
   例：#161 工作地图 / #230 排期总纲 / #272 任务池——这些是索引，不是任务
```

## 拆之前必查三件事

```bash
# ① 这个子项是不是已经有人拆过/在做？
curl -s -H "Authorization: token $FJT" "$FJ/repos/$ORG/$REPO/issues?state=all&limit=100" \
  | python3 -c "import sys,json; [print('#%d %s' % (i['number'],i['title'])) for i in json.load(sys.stdin) if '<关键词>' in i['title']]"

# ② 这个方向是不是被明确后置了？
#    看长期文档正文有没有「现在不做」「后置」的表态
#    有 → 不要拆，先问 Lead 是否解冻

# ③ 拆出来的子项是否功能独立？
#    判据：能单独验收、单独交付、不依赖另一个未完成子项
```

## 拆的标准（缺一条就不要拆）

```
1. 功能独立     —— 能单独验收（有明确的完成判据）
2. 边界清楚     —— 说得出改哪个 crate / 哪个模块，不碰哪里
3. 不依赖未完成 —— 前置件已合入 main，或在 issue 里明写依赖
4. 不是重复     —— 查过没有同名/同关键词的 issue 和 PR
5. 有出处       —— issue 正文引用长期文档的原句 + 章节位置
```

## issue 模板（照抄）

```markdown
# <优先级>: <子项名>——<一句话说清做什么>（#<母 issue> 分层第 N 层，功能独立项）

> 来源：#<母 issue> 第 N 层 / 落地顺序步 M
> 母文档原文：「<引用那一句>」

## 为什么独立

```
· 前置件已就位：<列出来>
· 本项不依赖：<列出还没做的兄弟项，说明为什么不阻塞>
· 可单独验收：<验收判据>
```

## 范围

```
改：<具体文件/模块>
不改：<明确排除的范围>
```

## 验收

```
1. <可测的判据>
2. <可测的判据>
```

## 关联

```
母 issue：#<N>
兄弟项：#<N1> #<N2>（并行不冲突/有依赖顺序）
```
```

## 实现纪律（高内聚低耦合）

```
① 一个 issue 只做一件事
   不要把「顺手也修了 XX」塞进同一个 PR

② 新模块优先
   能新建文件就不要改老文件；必须改老文件时，改动的行数要能说清为什么

③ 不跨层
   引擎面不碰 UI，UI 不碰引擎内部；边界靠 trait / 公开 API

④ 加字段前先看 AGENTS 0.0.5
   结构体加字段会打穿所有字面量构造点

⑤ 加 MCP 工具后跑派生面生成
   cargo run -p bid-mcp --bin tool-surface -- --write
```

## 反面教材（2026-11-07 实测）

```
❌ 一次 PR 混做两件事
   ai-bid #344 声明只做 Skill 启动器，实际混了 #321 的七工具
   → 与 #338 重复 → 被 Lead 关闭，工作白做

❌ 没查已有人做
   同一天 5 组重复 PR（#334/#335、#343/#344、#342/#349、#346/#353、#395/#396）
   → 全部靠 Lead 手工裁决
```

## 拆出来之后

```
1. 认领自己拆出来的那个 issue（不是认领母 issue）
2. 母 issue 里登记：「已拆出 #<N>（第 N 层）」
3. 完成时关子 issue，母 issue 保持开着直到全部层完成
```

## 推送前必看 main（再说一遍，因为最容易忘）

```bash
git fetch origin main
git log --oneline origin/main -5
git rev-list --count HEAD..origin/main

# 落后 > 0 就必须先 rebase
git rebase origin/main

# 开 PR 前确认 base 是 main
curl -s -H "Authorization: token $FJT" "$FJ/repos/$ORG/$REPO/pulls/<N>" \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['base']['ref'])"   # 必须是 main
```

**2026-11-07 实测代价**：6 个 PR 因 base 落后变 dirty → GitHub 不跑 CI → 永远「没有 CI」→ Lead 手工 rebase 10+ 次。

---


## 0.0.8 提交 PR 后不要傻等——去做下一个任务，定期回来看

> 平台无 CI。原文的「等 CI」已不适用；本节的「不傻等、并行推进」方法论仍然有效。

**PR 审核和合并由 Lead 做，你可能要等。傻等 = 一半时间在干等。**

## 为什么不能等

```
实测（2026-11-07）：
  ai-bid windows / cargo check 单次 50-110 分钟
  cargo test 全量 workspace 含 mang-app，慢是真的慢
  历史成功 run：51 / 77 / 78 / 91 / 107 / 109 / 112 分钟

代理如果交完 PR 就盯着 CI：
  · 1-2 小时啥也不干
  · 沙盒会杀后台任务，盯着也盯不住
  · 交付速度减半
```

## 正确做法（三步）

### 第 1 步：交完 PR 立刻去做下一个任务

```bash
# 1. 推送分支
git push ... HEAD:<branch>

# 2. 开 PR（记下 PR 号）
curl -X POST "$FJ/repos/$ORG/$REPO/pulls" -H "Authorization: token $FJT" \
  -H 'Content-Type: application/json' \
  -d '{"title":"feat(#N): xxx","head":"<branch>","base":"main"}'

# 3. 记下「回来检查时间」
#    预计 CI 时长 + 15 分钟缓冲
#    例：CI 通常 60-90 分钟 → 记 105 分钟后回来看
```

**然后立刻去看下一个可接 issue，不要站在原地。**

### 第 2 步：定期回来看（三种方式，任选）

**方式 A：自己定时回来（推荐，最可靠）**

```
交完 PR 后，在做下一个任务的过程中：
  · 每完成一个自然段（如写完一个函数、跑完一次本地测试）
  · 花 10 秒查一下上一个 PR 的状态

一条命令：
```

```bash
curl -s -H "Authorization: token $FJT" "$FJ/repos/$ORG/$REPO/pulls/<N>" \
  | python3 -c "import sys,json; d=json.load(sys.stdin); print('state=',d['state'],'merged=',bool(d.get('merged_at')))"
```

**方式 B：派子代理监督（懒人版）**

```
如果你的客户端支持子代理：
  派一个子代理，任务 = 「每 20 分钟查 PR #<N> 的 CI，红了立刻报告」
  你自己继续做下一个任务

注意：子代理的沙盒也会被杀 → 让它把每次检查结果写到文件，
      你回来时读文件即可。
```

**方式 C：等 GitHub 通知（最不可靠）**

```
不推荐——很多客户端不推送 PR CI 结果。
```

### 第 3 步：回来看结果，红了立刻修

```
CI 绿  → 在 issue 留言报 PR 号，继续下一个任务
CI 红  → 立刻停下来修（这是最高优先级，比继续做新任务重要）
         修完推送，重新计时间回来检查
```

## 什么算「红了」

```
❌ 不能这样解释：
  「是 CI 环境问题」                → 除非你能证明（贴证据）
  「本机能过啊」                    → CI 是唯一判据
  「可能是 flaky」                  → 重跑一次证明；重跑还红就是真红
  「不影响我的改动」                → main 红就是所有人的问题

✅ 正确处理：
  红了 → 看日志 → 定位 → 修 → 推送 → 再等
```

## 修红的时间预算

```
小问题（改断言/补字段/跑 --write）：  ≤ 30 分钟
中问题（改逻辑/加测试）：              ≤ 2 小时
大问题（要重构）：                     停下来报告 Lead，不要自己扛
```

## 已知的高频红因（先查这些，能省一半时间）

```
① 加工具后没跑 tool-surface
   cargo run -p bid-mcp --bin tool-surface -- --write

② 结构体加字段后旧字面量没补（AGENTS 0.0.5）
   grep -rn "StructName {" --include="*.rs" crates/

③ base 落后 main → dirty → CI 不跑（AGENTS 0.0.3）
   git rev-list --count HEAD..origin/main

④ 测试互踩（并行读写同一个全局文件）
   找 static Mutex / 全局路径
```

## 反例（2026-11-07 实测）

```
❌ 代理交完 PR 就消失，CI 红了一整天没人管
   结果：Lead 手工修了 10+ 个 PR 的断言/字段/锁
   单日 6 个 PR 因同一类问题红（draft_tool 断言 35 → 37/39/42）

✅ 正确：交完继续做，每 20-30 分钟扫一眼，红了立刻修
```

## 一句话总结

```
交 PR 不等待，做下一个；
定时回来看，红了立刻修；
修红优先于做新任务。
```

---

## 0.0.11 项目状态用平台原生机制查，建 issue 必须打标签

**文档会落后。label / issue 搜索是活数据，永远准。**

> 平台已于 2026-11-10 从 GitHub 迁到 `git.mingjiu.store`（Forgejo）。
> 本节命令已全部换成 Forgejo REST API，可直接复制执行。

## 为什么废弃「任务池 / 排期总纲 / 工作地图」

```
旧做法的问题：
  · 任务池、排期总纲、代理任务单 说的是同一件事
  · 每个都要手工维护，永远落后于真实状态
  · 代理要先读三个文档才知道该做什么

新做法：
  · 优先级用 label（P0/P1/P2/P3）
  · 领域用 label（area/pdf、area/office、area/gpui...）
  · 类型用 label（type/wiring、type/fake-green...）
  · 状态用 label（status/ready、status/claimed、status/blocked、status/done）
  · 「谁在做」看 issue 认领评论 + 关联 PR
```

## 环境准备（每台机器做一次）

```bash
export FJ=https://git.mingjiu.store/api/v1
export FJT=<TOKEN>     # 派单时下发的 token，不要写进任何文件
export ORG=sandbox

# 验证连通（应返回仓库 JSON）
curl -s -H "Authorization: token $FJT" "$FJ/repos/$ORG/mdcore" | head -c 100
```

## 标签体系（三个仓库已建好，23 个）

### 优先级

```
P0   最高优先——阻塞或用户直接受损
P1   重要——1.0 必须有
P2   一般——有空再做
P3   后置——仅登记
```

### 领域

```
area/pdf       PDF 读写/渲染/批注/表单/签名
area/office    DOCX/XLSX/PPTX 读写
area/gpui      mang-shell UI 与交互
area/canvas    画布/笔迹/绘图
area/bid       标书业务逻辑
area/mcp       MCP 工具面
area/build     打包/发行/CI
area/overlay   统一标注/双向索引层
```

### 类型

```
type/wiring      ★接线缺口——引擎有但产品没接
type/fake-green  ★假绿——测试绿但产品不work
type/debt        技术债
type/perf        性能
type/security    安全
type/docs        文档
```

### 状态

```
status/ready     可接取
status/claimed   已认领（开工时打）
status/blocked   被阻塞
status/stale     僵尸认领/长期无动静
status/done      已完成（关闭前打）
```

## ★ 找活干（不要读任务池）

```bash
# 列出全部 open issue（含标签）
curl -s -H "Authorization: token $FJT" \
  "$FJ/repos/$ORG/mdcore/issues?state=open&limit=50" \
  | python3 -c "import sys,json; [print('#%-3d [%s] %s' % (i['number'], ','.join(l['name'] for l in i['labels']), i['title'])) for i in json.load(sys.stdin)]"

# 按标签筛选（例：接线缺口——最高价值）
curl -s -H "Authorization: token $FJT" \
  "$FJ/repos/$ORG/mdcore/issues?state=open&labels=type/wiring"

# 按多个标签筛（P0 + area/pdf）
curl -s -H "Authorization: token $FJT" \
  "$FJ/repos/$ORG/mdcore/issues?state=open&labels=P0,area/pdf"

# 可接取的
curl -s -H "Authorization: token $FJT" \
  "$FJ/repos/$ORG/mdcore/issues?state=open&labels=status/ready"
```

## ★★ 建 issue 的硬要求（任何时候发现缺口都要遵守）

**三条缺一不可：**

```
① 标题以优先级开头（P0: / P1: / P2: / P3:）
② 打标签：优先级 + 领域 + 类型（至少三个）
③ 正文四段（见下）
```

**命令模板：**

```bash
# 1. 把正文写到临时文件（避免 shell 转义地狱）
cat > /tmp/body.md << 'EOF'
## 现象
用户看到什么 / 哪一步断了

## 证据
文件:行 / 命令输出 / 测试名——不要泛泛而谈

## 任务
改什么、不改什么

## 验收
可测的判据（能写 assert 的那种）
EOF

# 2. 建 issue（用 python 组 JSON，避免引号问题）
python3 -c "
import json, urllib.request
body = open('/tmp/body.md').read()
data = json.dumps({'title': 'P1: <一句话说清做什么>', 'body': body}).encode()
req = urllib.request.Request('$FJ/repos/$ORG/mdcore/issues', data=data, method='POST',
    headers={'Authorization': 'token $FJT', 'Content-Type': 'application/json'})
print(json.load(urllib.request.urlopen(req))['number'])
"

# 3. 打标签（拿到 number 后）
curl -s -X POST "$FJ/repos/$ORG/mdcore/issues/<N>/labels" \
  -H "Authorization: token $FJT" -H 'Content-Type: application/json' \
  -d '{"labels":["P1","area/pdf","type/wiring"]}'
```

## 认领 / 完成 / 阻塞（状态流转）

```bash
# 认领：加 status/claimed + 留言
curl -s -X POST "$FJ/repos/$ORG/mdcore/issues/<N>/labels" \
  -H "Authorization: token $FJT" -H 'Content-Type: application/json' \
  -d '{"labels":["status/claimed"]}'

curl -s -X POST "$FJ/repos/$ORG/mdcore/issues/<N>/comments" \
  -H "Authorization: token $FJT" -H 'Content-Type: application/json' \
  -d '{"body":"认领：Agent-xxxx\n预计：2 小时\n分支：agent-xxxx/123-xxx\n计划：1.… 2.…"}'

# 完成：加 status/done + 留言 + 关闭
curl -s -X POST "$FJ/repos/$ORG/mdcore/issues/<N>/labels" \
  -H "Authorization: token $FJT" -H 'Content-Type: application/json' \
  -d '{"labels":["status/done"]}'

curl -s -X PATCH "$FJ/repos/$ORG/mdcore/issues/<N>" \
  -H "Authorization: token $FJT" -H 'Content-Type: application/json' \
  -d '{"state":"closed"}'

# 阻塞：加 status/blocked + 说明原因
curl -s -X POST "$FJ/repos/$ORG/mdcore/issues/<N>/labels" \
  -H "Authorization: token $FJT" -H 'Content-Type: application/json' \
  -d '{"labels":["status/blocked"]}'
```

## 建完自检

```bash
# ① 能搜到吗（按标签筛）
curl -s -H "Authorization: token $FJT" \
  "$FJ/repos/$ORG/mdcore/issues?state=open&labels=P1,area/pdf" \
  | python3 -c "import sys,json; [print('#%d %s' % (i['number'],i['title'])) for i in json.load(sys.stdin)]"

# ② 标签打全了吗
curl -s -H "Authorization: token $FJT" "$FJ/repos/$ORG/mdcore/issues/<N>" \
  | python3 -c "import sys,json; d=json.load(sys.stdin); print([l['name'] for l in d['labels']])"
```

**不打标签的 issue = 搜不到 = 等于不存在 = 白建。**

## 每周整理（Lead 做）

```bash
# 找僵尸：认领超过 7 天无动静
curl -s -H "Authorization: token $FJT" \
  "$FJ/repos/$ORG/mdcore/issues?state=open&labels=status/claimed" \
  | python3 -c "
import sys, json, datetime
now = datetime.datetime.now(datetime.timezone.utc)
for i in json.load(sys.stdin):
    u = datetime.datetime.strptime(i['updated_at'][:19], '%Y-%m-%dT%H:%M:%S').replace(tzinfo=datetime.timezone.utc)
    days = (now - u).days
    if days > 7: print('#%d 停摆 %d 天: %s' % (i['number'], days, i['title'][:50]))
"
```

## 反面教材

```
❌ 「我去看任务池 issue 有哪些任务」→ 那个 issue 可能三天没更新
✅ 「curl .../issues?labels=P0」→ 永远当前真实状态

❌ 「我发现缺口，写个 issue 就完了」→ 没标签，没人搜得到
✅ 「建 issue + 打 P1,area/pdf,type/wiring」
```

---
## 0.0.9 rebase 解完冲突后必须扫残留标记

**`git rebase --continue` 成功 ≠ 冲突解干净了。**

## 为什么

```
rebase 时如果漏了某个冲突文件没 git add：
  · git rebase --continue 仍会成功（它只看已 add 的文件）
  · 但漏掉的文件里还留着 <<<<<<< ======= >>>>>>>
  · 本地不编译 → 推上去 CI 红 → 白等 3 分钟才发现
```

**2026-11-07 实测**：ai-bid #371 rebase 后 `lib.rs:72` 残留 `>>>>>>>`，
导致 `error: unknown start of token: \u{2014}`——本地编译失败，CI 必红。

## 一条命令（< 0.5 秒）

```bash
grep -rn --include="*.rs" --include="*.md" --include="*.py" --include="*.toml" --include="*.json" \
  -E "^(<<<<<<<|=======$|>>>>>>>)" . | head -20
```

**有输出 = 还有残留，必须继续解。**

## 完整流程（rebase 后必做）

```bash
# 1. rebase
git fetch origin main && git rebase origin/main

# 2. 解冲突（每个冲突文件都要 git add）
#    ...

# 3. continue
git rebase --continue

# 4. ★ 扫残留标记（最容易忘的一步）
grep -rn --include="*.rs" --include="*.md" --include="*.py" -E "^(<<<<<<<|=======$|>>>>>>>)" . | head

# 5. 确认无未合并文件
git diff --name-only --diff-filter=U     # 必须为空

# 6. 本地编译（比等 CI 快 10 倍）
cargo check --workspace --all-targets

# 7. 推
git push --force
```

## 为什么第 6 步必做

```
本地 cargo check：  1-3 分钟（有缓存时几十秒）
等 CI cargo check： 3-5 分钟 + 排队

差 2-4 倍。而且本地报错立刻能改，CI 报错要等一轮。
```

## 一次漏检的真实代价

```
漏检 → push → 等 3 分钟 CI → 看到 unknown start of token
     → 修 → 再 push → 再等 3 分钟
= 白等 6-10 分钟 + 一轮 CI 配额

grep 一次 = 0.5 秒。
```

## 高频残留位置

```
① 手写面（TOOL_NAMES / schema.rs / tools.rs）——加工具时最容易漏
② 派生面（state_machine_matrix.rs）——但 tool-surface 会覆盖，问题不大
③ 文档（contract.md / SKILL.md）——两节都编号 23 这种
④ 配置文件（Cargo.toml 的 [[bin]] 段）——每个新 bin 都往末尾加
```

## 补充：冲突解完先跑 tool-surface

```bash
# 如果冲突涉及工具面/位串矩阵：
cargo run -p bid-mcp --bin tool-surface -- --write
cargo run -p bid-mcp --bin tool-surface -- --check
```

**它能自动修掉 4 个派生面的不同步，比手工算位串快得多。**



## rebase --continue 之后必须验证再 push

**2026-11-07 事故：脚本里 rebase --continue 失败（文件没 add），但脚本继续执行下一句 push —— 把 main 推到了 D6 分支上，PR #379 被 GitHub 自动关闭。**

```bash
# ✅ 正确顺序
git add <冲突文件>
git diff --name-only --diff-filter=U    # 必须为空，否则 continue 会失败
git -c core.editor=true rebase --continue

# ★ continue 之后必须验证三件事
git status                    # 不能有 unmerged
git log --oneline -1          # 必须是期望的 commit，不能是 main 的 merge commit
test ! -d .git/rebase-merge   # rebase 必须真的结束

# 三件都过了才 push
git push --force
```

**自动化脚本里尤其注意：rebase --continue 失败不要继续往下走，要 exit 1。**

## 0.0.10 引擎能力 ≠ 产品可用——PR 必须证明「用户操作 → 产品行为」闭环

**本仓最贵的教训：代码写了、测试绿了、issue 关了，但用户点下去没反应。**

## 2026-11-07 一次核查抓出四个「测试绿但没闭环」

```
① PDF 批注不落盘
   PdfAnnotateController::save() 存在，零调用方
   Editor::save() 对只读态 return true（假装成功）
   → 用户画高亮 → Ctrl+S 无报错 → 关闭 → 重开 → 没了
   → PR #336 的测试测的是「控制器能存盘」，不是「用户操作调到存盘」

② overlay 层孤岛
   crates/overlay 六模块完整，git grep mdc-overlay -- Cargo.toml 零命中
   → 两套批注体系并存，统一层没人用

③ XLSX 桌面只读
   PR #300 只接了 mdcore-wasm，mang-shell 里 XlsxEditor 零命中
   → issue #283 关闭了，但桌面端消费倒挂没修

④ GPUI 六缺陷补洞而非重做
   PR #297 修了六个 bug，但根因「自己手写 UI」没解决
```

## 提交 PR 前自问三个问题

```
① 用户怎么触发这个功能？
   「打开 xlsx 双击单元格改值」——说得出来吗？
   说不出来 = 没有产品入口 = 别急着交 PR

② 触发之后，文件/状态真的变了吗？
   「Ctrl+S 后 .pdf 字节变了」——能证明吗？
   只证明「控制器能存盘」不算

③ 测试是从用户操作开始的吗？
   ✅ app.click(...) → app.save_active() → 读文件断言
   ❌ 直接调控制器 API 断言返回值
```

## 三种「假闭环」形态

```
形态 A：有引擎没接线
   crate 写了，测试绿了，但没有任何产品 crate 依赖它
   查法：git grep "<CrateName>" -- Cargo.toml

形态 B：有接线没落盘
   视图能改内存，但不写文件（或写入后不读回验证）
   查法：从视图的 save 方法往上追到 fs::write

形态 C：测试跳过视图层
   测控制器/服务层，不测「用户手势 → Shell → 文件」
   查法：看测试第一行是 app.xxx() 还是 Controller::xxx()
```

## PR 描述里必须写的三行

```markdown
## 产品闭环

- 用户入口：<菜单/快捷键/右键，具体到哪个按钮>
- 操作后变化：<文件字节变了 / 状态变了 / 界面刷新了>
- 端到端测试：<测试名>（从 <用户操作> 开始，断言 <可观测结果>）
```

**缺这三行的 PR，Lead 会要求补。**

## 例外

```
以下 PR 不需要「产品闭环」三行：
  · 纯引擎面（明确标注「本 PR 只做引擎，产品接线见 #<N>」）
  · 纯文档 / CI / 重构（行为不变）
  · 测试补充（不改变产品行为）

但纯引擎面 PR 必须在描述里写清楚「谁在什么时候接线」，
不能只是「我做完了」。
```

## 反例（真实发生）

```
❌ 声明：「forms.rs save 是增量的，签名仍有效」
   实际：全量重写，签名必毁
   → 声称与实现倒挂，比不做更糟

❌ PR #336 标题：「桌面 PDF 标注接线」
   实际：接了一半（内存态），没接文件
   → 「接线」这个词被用滥了，要说清接的是哪一段
```

## 一句话

```
能点、能改、能存住、能重开看到——四件事都做到才算闭环。
```

---

## 0.0.15 `rebase -X theirs` 会静默删掉 main 上的功能——禁用

**2026-11-08 实测：`git rebase origin/main -X theirs` 报「成功、零冲突」，但把 main 上已合入的整块功能删了。**

## 事故经过

```
PR #433（fix/386-pdf-annotate-save）冲突在 pdf_pages.rs + shell.rs

用了 rebase -X theirs：
  → 零冲突，rebase「成功」
  → 编译报 14 个错误：
      cannot find type BTreeMap
      no method named jump_search / close_search / open_search
      no associated function dispatch_view_key / dispatch_search_key
  → 根因：#386 分支的 pdf_pages.rs 整体覆盖了 main 版本
      main 已合的 #422「页内查找」整套功能被删
      而 shell.rs 还在调这些函数 → 编译炸
```

**这是「rebase 成功」假象下的功能静默删除——比冲突更危险，因为它不报错。**

## 规则

```
❌ 禁用：git rebase origin/main -X theirs
❌ 禁用：git rebase origin/main -X ours
✅ 必须：git rebase origin/main，然后逐块人工解
```

## 唯一例外

```
分支确实「完全包含 main 的所有改动」时才能用 -X theirs。

判定方法：
  git diff origin/main...<your-branch> -- <冲突文件> | grep "^-"
  如果输出里有 main 独有的删除行 → 不能用

实际情况：几乎所有场景都不满足这个条件。
```

## 正确姿势：开 diff3 三方视图

```bash
git config merge.conflictStyle diff3
git rebase origin/main
# 冲突块会变成：
#   <<<<<<< HEAD              ← main 侧
#   ||||||| merged common ancestors
#   =======
#   >>>>>>> <commit>          ← 分支侧
# 三方一对比，谁加了什么一目了然
```

## 解完必做三查

```bash
# 1. 扫残留标记
git grep -n "^<<<<<<<" -- "*.rs" "*.md" "*.ts" "*.py"

# 2. 确认 main 的功能还在（拿冲突文件对比 main）
git diff origin/main -- <冲突文件> | grep "^-" | head -20
# 看到大量 main 独有的函数被删 → 合并错了

# 3. 全量编译
cargo check --workspace --all-targets
```

## 一句话

```
冲突是显性的，编译器会拦住你；
-X theirs 造成的功能删除是隐性的，rebase 报成功，测试可能也过，
直到某天用户发现功能没了。

宁可多花 20 分钟手工解冲突，不要用 -X 走捷径。
```

---

## 0.0.16 研究仓的结论必须有主仓 issue 承载

**研究仓（mang-research-archive）产出的任何结论，如果影响主仓：**

```
1. 必须在主仓建 issue（带标签 + milestone）
2. issue 正文必须写研究仓的锚点（文件 + 章节）
3. 研究仓 README 的反向索引必须同步更新
```

**反面教材（2026-11-08 实测）**：

```
cleanroom-analysis/DECISIONS.md 里 20 项决策，
其中 17 项「类别 3 阻塞」躺在研究仓 7 天，
主仓零 issue 承载 —— 研究做完了等于没做。

收口（同日 #526）：19 项 DEC 系 issue 建齐（#528-#546）
+ 研究仓 README 反向索引在档——此后新结论按本节纪律走。
```

**决策类 issue 的模板**（#528-#546 系同构）：

```
· 精确决策问题（照抄研究仓登记，不要改写）
· 选项与后果（a/b/c 及影响）
· 证据锚点（研究仓文件 + 章节）
· 不决策会阻塞什么
· 纪律：issue 只承载决策，不在 issue 里做技术调查；
  拍板后立刻拆实施 issue（带具体验收）；
  状态回写研究仓 DECISIONS.md（「已裁决」+ issue 链接）
```

---

## 0.0.14 拆子 issue 开工前，先扫母 issue——认领评论与 open PR 双查

**这条是 2026-11-07 #395/#396 撞车的教训（当日第 5 组重复 PR）。**

### 事故

```
08:07  Agent-9716 在母项 #240 认领任务③（web 面），范围声明写在认领评论里
08:26  Agent-9716 交付 PR #395（标题引母项号 #240——子 issue 还不存在，
       平行认领只能挂在母项名下）
08:35  Agent-v65o 未读 #240 评论，把任务③ 拆成子 issue #396 并认领开工
       → 0.0 四查全过：#396 无认领 / 无 PR / 无同名分支 / 池里只有母项
       → 撞车点全部登记在母项 #240 命名空间，四查查的是 #396 号
08:57  撞车浮出，归并交 Lead 裁定，一方工作作废
```

### 铁律

```
拆子 issue 之前、以及子 issue 开工之前，各做一遍「母项扫描」：

① 读母 issue 的全部认领评论（认领评论自带范围声明）
   curl -s -H "Authorization: token $FJT" "$FJ/repos/$ORG/$REPO/issues/<母N>/comments" \
     | python3 -c "import sys,json; [print(c['user']['login'],':',c['body'][:80]) for c in json.load(sys.stdin) if '认领' in c['body']]"
   → 对照你要拆的层：同层已认领 → 停，不拆不开工（0.0.1 后到让位）

② 扫 open PR 的标题与分支名（查「母项号 + 层关键词」）
   curl -s -H "Authorization: token $FJT" "$FJ/repos/$ORG/$REPO/pulls?state=open&limit=100" \
     | python3 -c "import sys,json; [print('#%d %s' % (p['number'],p['title'])) for p in json.load(sys.stdin) if '#<N>' in p['title'] or not '<N>' in '#<N>']"
   → 例：母项 #240 + 关键词「web / 打印」能直接命中 PR #395
   → 同层已有 PR → 停，去帮那个 PR（review / 补测试 / 修 CI）

③ 关键词兜底（0.0.7 ① 的扩展——issue 和 PR 一起搜）
   curl -s -H "Authorization: token $FJT" "$FJ/repos/$ORG/$REPO/issues?state=all&limit=100" \
     | python3 -c "import sys,json; [print('#%d %s' % (i['number'],i['title'])) for i in json.load(sys.stdin) if '<母文档原句关键词>' in i['title']]"
   → 用母文档原句关键词（如「@media print」「打印态」），
     不用自己发明的词
```

### 为什么 0.0 四查抓不住这种撞车

```
0.0 四查查的是「新子 issue 号」名下的认领 / PR / 分支——
而母任务下平行认领的痕迹全在母 issue 号名下：
  · 认领评论在母 issue 的 comments 里
  · PR 标题只能引母 issue 号（子 issue 尚不存在）
拆层 = 从母命名空间切出子命名空间；
切之前必须查母空间里有没有人正在做同一层。
```

### 判定

```
母项同层有认领且 <12h 有产出（PR/分支/实质报告） → 让位，另择他项
母项同层有 open PR                                  → 让位，去帮 PR 收尾
母项同层认领 >12h 零产出                            → 报告 Lead 裁定（0.0.6）
全都没有                                            → ✅ 拆 / 开工
```
---

## 0.0.1 一个 issue 只能有一个 PR

```
❌ 两个代理各开一个 PR 做同一件事
✅ 后到的看到已有 PR → 去帮它（review、补测试、修 CI），不新开

如果发现已有 PR 但质量不行：
  报告 Lead → Lead 决定是让它继续还是关闭重做
  不要自己新开一个「更好的」
```

## 0.0.3 提交 PR 前必查主分支进度（防冲突）

**开 PR 之前、以及每次推新 commit 之前，必须检查 main 有没有前进。**

```bash
git fetch origin main
git log --oneline origin/main -5
git rev-list --count HEAD..origin/main        # 落后几个 commit
git rev-list --count origin/main..HEAD        # 领先几个 commit
git rebase origin/main                        # 落后 > 0 就必须先 rebase
```

**判定规则：**

```
落后 0 个 commit          → ✅ 直接开 PR
落后 1-5 个 commit        → ⚠️ 先 rebase（多数能自动合）
落后 > 5 个 commit        → ⛔ 必须 rebase，且要重跑本地测试
main 编译不过（红）        → ⛔ 停下来，报告 Lead（你的分支也会红）
```

**为什么（2026-11-07 实测代价）：**

```
· 6 个 PR 因 stale base 变 dirty → GitHub 不跑 CI → 永远「没有 CI」
· #291/#290/#292/#347 全部卡在这个坑
· 每次都要 Lead 手工 rebase + 解冲突，单日解了 10+ 次
```

**开 PR 时还要确认：**

```bash
curl -s -H "Authorization: token $FJT" "$FJ/repos/$ORG/$REPO/pulls/<N>" \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['base']['ref'])"   # 必须是 main
```

**base 不是 main → 关掉重开**（改 base 不会重新触发 CI）。

**main 变红时的处理：**

```
现象：main 编译失败（如 export_tool.rs missing fields ...）
后果：所有下游分支继承此错误 → 多个代理全卡住
正确做法：立刻报告 Lead，不要自己「顺手修 main」
          （Lead 会开 hotfix 分支走 PR 合入）
```

---

## 0.0.2 认领要写在 issue 正文的可见位置

```bash
# 认领评论必须包含这三项（缺一不可）
curl -s -X POST "$FJ/repos/$ORG/$REPO/issues/<N>/comments" \
  -H "Authorization: token $FJT" -H 'Content-Type: application/json' \
  -d '{"body":"认领：<身份>\n\n预计完成：<时长>\n分支名：<branch-name>（马上开）\n计划：<3 步以内>"}'
```

**为什么写分支名**：后人能通过分支名查到你是否真的开工了。

---

## 0.0.13 认领→开 PR 之间有窗口——开 PR 前最后一查（#408 84 秒平行事故）

**四查是「时点判定」，不是「区间判定」——查完到动手之间，别人可能已经认领或交付。**

## 事故（2026-11-07 #408 实测）

```
10:06:14  Agent-v65o 发认领评论（时点四查干净）
10:07:38  Agent-3ivu 的 PR #424 创建（其四查完成于上述认领评论
          可见之前——晚于此就不该是干净的；且未先发认领评论，
          先 PR 后补认领）
10:22:01  Agent-v65o 让位：本地实现不推送，转 review #424

窗口 = 84 秒。双方四查都做了、都没查错——错的是
「查完之后世界静止」的假设。
```

## 三条缓解（缺一不可）

```
① 认领评论必须真实发出后再开工（0.0.2 / 0.1 的重申）
   「先 PR 后认领」是窗口事故的直接成因：你的开工没有落点公示，
   后来者无从避让。

② 开 PR 前最后一查（一条命令，< 5 秒）
   认领到 PR 之间可能隔几小时，世界没有静止：

     curl -s -H "Authorization: token $FJT" "$FJ/repos/$ORG/$REPO/pulls?state=open&limit=100" \
       | python3 -c "import sys,json; [print('#%d %s' % (p['number'],p['title'])) for p in json.load(sys.stdin) if '#<N>' in p['title'] or not '<N>' in '#<N>']"

   出现新 PR → 让位（0.0.1），不重复开。

③ 平行已成事实 → 后到让位，转 review
   #408 的处置是范本：平行实现不推送（白做认赔），转为独立实测
   对方 PR + 把发现列成 review 意见（v65o 对 #424 的 review 抓出
   三条实质问题——让位不等于白干）。
```

## 为什么「后到」以 PR 时点判，不以认领时点判

```
认领是意图声明，PR 是事实交付。0.0.1 的裁决基准是「谁先有 open PR」；
认领了但 12h 无产出（0.0.6）同样要让位给已交付者。
```

---

## 0.0.12 开新 issue 前必查重——认领前全局搜同题

**0.0 清单查的是「这个 issue 有没有人认领」；本节补查的是「这个问题有没有人登记过」。两查都要做。**

## 怎么查（两条命令）

```bash
# ① 开新 issue 前：按问题核心词全局搜 open issue（标题+正文一起搜）
curl -s -H "Authorization: token $FJT" "$FJ/repos/$ORG/$REPO/issues?state=open&limit=100" \
  | python3 -c "import sys,json; [print('#%d %s' % (i['number'],i['title'])) for i in json.load(sys.stdin) if '<核心词1>' in i['title'] or '<核心词2>' in i['title']]"

# ② 认领前：除 0.0 四查外，再全局搜一遍同题
#    （两个代理各自发现同一个 main 红 / 门禁红，是 2026-11-07 的真实高频场景）
```

**判定：**

```
搜到同题 open issue    → 不新开。去先有 issue 下认领/评论，把你的发现补进去
搜到同题 closed issue  → 确认真做完了再动手；没做完 → 在旧 issue 重开或评论，不开新
什么都搜不到           → ✅ 可以开
```

## 为什么（2026-11-07 单日 4 组同题事故）

| 组 | 事实 | 后果 |
|---|---|---|
| #350 / #351 | 同一任务（#44 步1）两个 issue，创建相差 1 分钟 | 双认领，靠代理事后自查才发现重复 |
| #359 / #362 | compliance scan 红（packaging/ GPUI 词面）两代理独立发现，相差 2 分钟各开一个 | 同一问题两份登记，Lead 要分诊两遍 |
| #361 / #365 | main 红 w1_toc 断言，#365（06:50）全文包含 #361（06:01）主题再叠一项 | 两个 PR 修重叠面，互相牵制 |
| #240 任务③ / #396 | 母 issue 08:07 已认领任务③、08:26 交付 PR #395；另一代理 08:35 把任务③拆成新 issue #396 再认领 | 同题双轨，注定一个 PR 白做 |

**与 0.0 的分工**：0.0 第 1 步只看「目标 issue」的认领状态；本节要求认领前**全局**搜同题。拆分场景两查都要做（0.0.7 的拆前三查不含母 issue 认领面）。

## 发现重复后的处置

```
自己是后开的         → 关闭自己的 issue（未推送分支直接删），正文一行指向先有 issue
两边都在做           → 在两个 issue 各留归并评论 + 报 Lead 裁定保留哪个
                         （不要自作主张关别人的 issue）
自己是先开的         → 在自己 issue 下补「#N 与本 issue 同题」一条，请 Lead 归并
```

## 拆分场景（与「母项扫描」姊妹规则的分工）

```
拆子 issue 前扫母 issue（认领评论 + open PR 双查）由姊妹规则「母项扫描」专述
（PR #403，#395/#396 撞车教训）。该规则占用编号 0.0.9，但本文档 0.0.9 已被
「rebase 解完冲突后必须扫残留标记」（PR #383）占用；**编号冲突处理：**
**0.0.9 归 rebase 残留标记规则**，「母项扫描」暂以标题引用、不分配编号，
待 #403 合并后由 Lead 统一另编号。本节增量只有一句：不走拆分的
独立发现，同样要全局搜——2026-11-07 的 4 组同题事故里 3 组
（#350/#351、#359/#362、#361/#365）与拆分无关，母项扫描救不了，
全局同题搜索才救得了。
```

**一句话：开 issue 前先搜，认领前再搜——同题只剩一个 issue 是所有人的义务。**



# 零之二、认领与身份（必须先做）

## 0.1 开工前认领 issue

**开始任何工作之前，必须在 issue 上留言认领。**

```bash
curl -s -X POST "$FJ/repos/$ORG/$REPO/issues/<N>/comments" \
  -H "Authorization: token $FJT" -H 'Content-Type: application/json' \
  -d '{"body":"认领：<你的身份> 承接本任务，预计 <时间>"}'
```

**为什么**：

```
· 多个代理同时看到同一个 issue → 不认领就会重复劳动
· Lead 需要知道每个 issue 归属谁，才能派单和催进度
· 认领记录是「谁做的」的第一手证据
```

## 0.2 身份命名规则

### 情况 A：用户已命名

```
用户说「你叫 C」→ 你的身份就是 C
所有提交、PR、issue 评论都用这个名字
```

### 情况 B：用户没命名

**自己生成一个不重复的随机后缀，格式 Agent-<4位小写字母数字>**

```
示例：Agent-hd2k4 / Agent-x7m2 / Agent-p9qr

生成方法（任选）：
  Linux:   echo "Agent-$(head -c4 /dev/urandom | xxd -p | head -c4)"
  Python:  python3 -c "import secrets,string;print(chr(34)+chr(65)+chr(103)+chr(101)+chr(110)+chr(116)+chr(45)+chr(34) + ...)"
  手动:    随机敲 4 个字符
```

**避免重复**：

```
生成后先查一下有没有被占用（任务面板 #229 已废弃、不再维护名字列表，改用搜索）：
  curl -s -H "Authorization: token $FJT" "$FJ/repos/$ORG/$REPO/branches?limit=100" \
    | python3 -c "import sys,json; print(len([b for b in json.load(sys.stdin) if 'agent-<后缀>' in b['name'].lower()]))"
重复了 → 重新生成一个
```

### 身份一旦确定，全程不变

```
❌ 今天叫 Agent-hd2k4，明天叫 Agent-temp1
✅ 一个会话内所有产出都用同一个名字
   跨会话如果记得上次的名字，继续用；不记得就新生成
```

---

## 0.3 身份必须出现在四个地方

### ① issue 认领评论

```markdown
认领：Agent-hd2k4

承接本任务，预计 2 小时。
计划：
  1. xxx
  2. xxx
```

### ② commit message 末尾

```bash
git commit -m "fix(#285): 标题" -m "Co-Authored-By: Agent-hd2k4 <agent-hd2k4@mang.local>"
```

**格式固定**：`Co-Authored-By: <身份> <agent-<小写>@mang.local>`

### ③ PR 描述开头

```markdown
**提交者**：Agent-hd2k4

## 交付概要
...
```

### ④ 完成时在 issue 的关闭评论

```markdown
Agent-hd2k4 交付完成

PR #xxx 已合并，commit abc1234

验收对照：
| 要求 | 证据 |
|---|---|
...
```

---

# 0.4 为什么身份要可追溯

```
· 出问题时知道找谁（「这个 PR 是 Agent-x7m2 交付的，问他」）
· 质量统计（谁的 PR 一次过、谁的总要返工）
· 避免「无人认领」的悬空工作
· 避免 A 的代码记到 B 头上
```

## 0.5 认领冲突处理

```
如果发现 issue 已被人认领：
  · 不要重复做
  · 在评论里说明「看到 Agent-xxx 已认领，我不重复」
  · 去做别的 issue
```

### 如果认领者很久没动静（>1 天）

**不要去等另一个代理回复——代理之间没有互相等待的关系。**

```
❌ 错误：
  看到 Agent-xxx 认领了但 2 天没动
  → 在评论里问「你还在做吗？」
  → 等对方回复
  → 一直等下去（可能永远不来）

✅ 正确：
  看到 Agent-xxx 认领了但长时间没动
  → 直接向 Lead（用户）报告：
    「#285 被 Agent-xxx 认领，X 天无动静，我可以接手吗？」
  → 等 Lead 决定
  → Lead 说接手就接手
```

**理由**：

```
· 代理之间没有可靠的通信通道，互相等待 = 死锁
· 只有 Lead 知道全局状态（那个代理是不是被回收了、是不是在做更重要的）
· 由 Lead 裁决 = 单一权威，不会出现两个代理都以为对方在做
```

---

# 一、工作流铁律

## 1. 开分支前必须同步 main

```bash
git fetch origin main && git checkout -f origin/main
git checkout -b <type>/<issue-number>-<slug>
```

**为什么**：2026-11-06 单日三次 stale base 事故，每次都携带数千行无关回滚：

```
#258  基于旧 main → 携带 1,231 行回滚
#259  基于旧 main → 携带 6,567 行回滚
#235  基于旧 main → 携带 165 行 web 删除
```

**开 PR 前自检**：

```bash
git diff --stat origin/main | tail -3
# 如果 diff 里出现你没改过的文件 → 分支基于旧 main，重新来
```

## 2. 每个 commit 立即推送并**确认成功**

```bash
git push origin HEAD:<branch>
# 必须看到 "xxxx..yyyy HEAD -> branch" 才算成功
# 只看到 "Everything up-to-date" 但本地有 commit → 推错了分支
```

**为什么**：沙箱会被回收，本地未推送的 commit 全部丢失。

**回收后的恢复检查清单**：

```
[ ] git log origin/<branch> --oneline | head -3    ← 确认云端有你的 commit
[ ] git status                                     ← 确认工作区干净
[ ] 工具链是否还在（cargo --version）
[ ] 如果工具链丢了，重装到持久化路径
```

## 3. 不要等 CI，开完 PR 立即做下一项

```
❌ 错误做法：
  开 PR → 盯 CI → 等 10 分钟 → 绿了 → 才开下一项
  结果：一个工作周期只推进一个任务

✅ 正确做法：
  开 PR → 立刻开下一个分支做下一项
  CI 红了由 Lead 反馈，你回来修
  结果：一个工作周期推进 2-3 个任务
```

**唯一例外**：改共享聚合文件（gates.md / manifest.json）时建议等 CI，
因为冲突在这类文件上。但 gates.md 正在拆分（#248），拆完这个例外消失。

## 4. 提交 PR，**不得自行合并**（见第 0 条）

```
Lead 负责合并。你只负责交付。
自行合并 = 绕过审核 = 违规。
这条是最高优先级，没有例外。
```

## 5. 一个 PR 对应一个 issue

```
❌ 一个 PR 塞 3 个 issue → 审核困难、回滚困难
✅ 一个 PR 一个 issue → 除明确的依赖链（stacked PR 要说明）
```

## 6. issue 做完就关，不留悬空

**PR 合并后立即：**

```
① 在 issue 留「已完成」评论，含：
   · 合并 commit SHA
   · 验收对照表（每条要求 → 证据）
   · 本地实测命令与输出
② 关闭 issue
```

**为什么**：issue 不关 → 积压数字虚高 → Lead 无法判断真实进度。

**2026-11-06 实例**：ai-bid 14 个 open issue 里，至少 7 个是「已完成但没关」。

---

# 二、代码质量铁律

## 7. 真实测试，不许伪报

```
✅ 「本机 Linux 实测：cargo test -p xxx 全绿，输出见下」
✅ 「Windows 未验证——CI 只做编译检查」
✅ 「环境缺 poppler，本测试 SKIP，未执行」

❌ 「测试应该没问题」（没跑）
❌ 「CI 全绿所以没问题」（CI 可能没覆盖到）
❌ 把 SKIP 当成 PASS 报告
```

**关键：CI 全绿 ≠ 稳定。** 实例：

```
songling 的 9 条验收门依赖 poppler，Windows CI 走 SKIP
→ 这些门在 CI 里从未运行
→ #266 造成的配额回归（0.824→0.820）在 main 上静默存在
→ 直到某代理在本机 Linux 跑才暴露
```

## 8. 不许伪造完成

```
❌ 代码写了一半就报「已完成」
❌ 测试只跑一部分就报「全绿」
❌ 断言跑了但没验证就报「通过」
```

**如果没做完，明确说「未完成 + 卡在哪 + 需要什么」。**

## 9. 不放宽断言迁就实现

```
❌ 测试红了 → 把断言改松 → 绿了 → 报告完成
✅ 测试红了 → 查清是「代码错了」还是「基线该更新」
   · 代码错了 → 修代码
   · 基线该更新 → 改基线 + 写清根因 + 说明为什么不是放宽标准
```

**判据**：`target` / 阈值如果动了，就是放宽。`known` 基线随上游修复同步，不是放宽。

## 10. 用子代理加速

```
可以并行的工作 → 开子代理并行做
例：3 个独立 crate 的改动 / 3 份语料的验证 / 多方案对比

但：
  · 子代理改的文件范围必须互不重叠（Lead 会检查）
  · 子代理的产出要自己复核（不能直接信）
  · 不确定子代理会怎么做的事，不要委派
```

---


---

# 二之二、push 前必读 Review

## 14.1 每次 push 前必须看 PR 评论

**这条是 2026-11-06 PR #292 的教训。**

### 事故

```
06:16  Lead 在 PR #292 发了「必须加环境门」的精确修法
06:24  代理推了新 commit（只差 8 分钟）
       但没改测试文件——因为没看到评论
06:25  CI 第二次红，同一个原因
```

### 铁律

```
每次 push 之前：
  [ ] 打开 PR 页面，看最新评论
  [ ] 如果有「必须修 X」「这个要改」的 Review，确认已修再 push
  [ ] 不要带着未处理的 Review 推新 commit
```

### 为什么

```
代理写代码时会专注手头的任务，不会主动回头看 PR 评论；
但 Lead 的 Review 往往指出的是「你没想到的环境/边界问题」；
忽略 Review 直接推 → CI 红第二次 → 浪费一整轮 CI 时间（10+ 分钟）。
```

## 14.2 如何高效检查

```bash
# 看 PR 上的评论
curl -s -H "Authorization: token $FJT" "$FJ/repos/$ORG/$REPO/issues/<N>/comments" \
  | python3 -c "import sys,json; [print(c['user']['login'],':',c['body'][:200]) for c in json.load(sys.stdin)]"

# 只看最近 5 条
# 或者直接看 PR 页面： https://git.mingjiu.store/sandbox/<repo>/pulls/<N>

# 或者直接看 PR 页面
```

## 14.3 CI 红了的正确处理顺序

```
❌ 错误顺序：
  CI 红 → 猜原因 → 改代码 → push → 又红 → 再猜

✅ 正确顺序：
  CI 红 → 看 CI 日志（找到具体失败的测试/行号）
         → 看 PR 评论（Lead 可能已经给了修法）
         → 本地复现（如果可能）
         → 改 → 本地验证 → push
```

**关键：先看日志和评论，不要猜。**

### 怎么查到失败原因（没 CI，只能看自己跑的输出）

```bash
# 本地跑测试，只看失败行
cargo test --workspace 2>&1 | grep -E "^error|FAILED|panicked" | head -30

# 或 python
python -m pytest -q --tb=short 2>&1 | tail -40
```

**输出里一定写着：哪个测试、哪一行、什么断言失败。**
把这段贴进 PR 描述或 issue 评论——这是你唯一的证据来源。

---


---

# 二之三、没有 CI 时的自我验证纪律

**本平台不跑 CI。没有「等它变绿」这回事——你就是唯一的验证者。**

## 15.1 每次提交前必须自己跑一遍

```bash
# Rust 仓
cargo check --workspace                    # 必须 0 error
cargo test -p <改动的 crate>                # 必须全绿
cargo fmt --check                          # 格式门

# Python 仓
python -m pytest -q                        # 必须全绿
```

## 15.2 PR 描述里必须贴实测输出

```markdown
## 本地验证

```
$ cargo check --workspace
    Finished `dev` profile [unoptimized] target(s) in 45.2s

$ cargo test -p mdcore
test result: ok. 123 passed; 0 failed; 0 ignored
```
```

**没贴输出的 PR，Lead 会打回。**

## 15.3 跑不了的，如实说

```
✅ 「cargo test -p mdcore 全绿，输出如下」
✅ 「本机无 poppler，9 条 SKIP，未验证」
✅ 「Windows 面未验证，只在 Linux 跑过」

❌ 「测试应该没问题」（没跑）
❌ 「代码看起来对」（没编译）
❌ 把 SKIP 当成 PASS 报告
```

## 15.4 合并质量由 Lead 把关

```
没有 CI 的后果：坏代码可能进 main。

补偿机制：
  · 你的 PR 必须带实测输出
  · Lead 合并前抽查关键改动
  · main 每 2 分钟同步到 Codeberg/CNB——坏了会同步出去
  · 发现 main 坏 → 立刻开 hotfix PR，不要自己直接改 main
```

---

# 三、环境与资源

## 11. 硬盘管理

```
沙箱磁盘有限（曾经被 cargo target 撑爆）。

必须做的：
  · 用独立 CARGO_TARGET_DIR，不要污染仓内
    export CARGO_TARGET_DIR=/tmp/cargo-target-<yourname>
  · 定期清理：rm -rf /tmp/cargo-target-*  （只清自己的）
  · 构建产物不要提交进仓（.gitignore 要覆盖）
  · 大文件（fixture PDF / 词料）用 git-lfs 或引用外部路径

检查磁盘：
  df -h .        # Linux
  Get-PSDrive C  # Windows
```

## 12. 工具链持久化

```
沙箱重置会丢工具链。装到持久化路径：
  · Rust: ~/.cargo （默认就在持久位置）
  · Python venv: ~/.venvs/xxx
  · 不要在 /tmp 里装工具链
```

## 13. 环境缺件如实报告

```
本仓 CI 无 poppler / tesseract / LibreOffice。
本机环境也常常缺。

遇到缺件：
  ① 测试框架会打 SKIP 横幅 → 如实报告「SKIP，未执行」
  ② 不要为了「让测试通过」去伪造环境检查
  ③ 如果任务必须依赖缺失工具，明确说「需要 Lead 提供 X 环境」
```

---


---

# 三之二、测试环境门（新增测试必读）

## 13.1 涉及真实语料 / 外部工具的测试，必须加环境门

**这条是 2026-11-06 PR #292 CI 红的教训。**

### 事故

```
新增测试 coverage_gate_corpus.rs 要求「11 份真件全部导入成功」；
但 CI（Windows runner）没有 poppler、没有 Word COM；
→ .pdf 件抽取链降级、.doc 件无法转换；
→ 两个测试 E_PARTIAL: 1 件导入失败；
→ CI 红。
```

**根因：新测试没走环境门机制。**

### 铁律

```
任何依赖以下资源的测试，必须加环境门：
  · poppler（pdftotext / pdftoppm / pdfinfo）  → POPPLER_TEXT_TOOLS
  · tesseract（OCR）                          → PDF_CHAIN_TOOLS
  · Word COM / LibreOffice（格式转换）        → 转换器可用性探测
  · 真实语料文件                               → skip_if_fixture_missing
  · 平台特定能力（Windows 打印 / macOS 构建）  → cfg 或运行时探测
```

### 正确写法

```rust
use bid_parse::env_preflight::{
    skip_if_env_missing, skip_if_fixture_missing, POPPLER_TEXT_TOOLS,
};

#[test]
fn my_real_corpus_test() {
    // ① 先查环境（会打印 SKIP 横幅 + 原因）
    if skip_if_env_missing("my_real_corpus_test", POPPLER_TEXT_TOOLS) {
        return;
    }

    // ② 再查语料是否在库
    if skip_if_fixture_missing("my_real_corpus_test", &[&fixture_path]) {
        return;
    }

    // ③ 真正跑
    ...
}
```

### 错误写法（会红 CI）

```rust
// ❌ 直接跑，不查环境
#[test]
fn my_test() {
    let svc = service();
    svc.import(&pdf).expect("预期成功");   // CI 无 poppler → 必红
}

// ❌ 静默 return，不打印原因
#[test]
fn my_test() {
    if !poppler_available() { return; }   // 看不出为什么跳过
    ...
}
```

## 13.2 SKIP 必须显式可见

```
SKIP 不是失败，但也不是成功——它是「本环境无法验证」。

必须做到：
  ① 打印横幅：SKIP: <test_id>
  ② 打印原因：reason: ENV_MISSING: <缺什么>
  ③ 打印补救指引：<怎么装>

仓里已有工具函数 skip_if_env_missing() 做了这三件事，直接用。
```

**为什么重要**：

```
2026-11-06 发现：songling 的 9 条核心验收门因为 poppler 缺失全部 SKIP，
这些门在 CI 里从未运行过；
#266 造成的配额回归（0.824→0.820）在 main 上静默存在了几天，
直到某代理在本机 Linux 跑才暴露。

如果 SKIP 当时是「可见的」，这个问题早就被发现了。
```

## 13.3 本地跑不了的，如实报告

```
✅ 「本机 Linux 实测：cargo test -p xxx 全绿」
✅ 「本机无 poppler，以下 9 条 SKIP，未验证」
✅ 「Windows 面未验证，CI 只做编译检查」

❌ 「测试应该没问题」（没跑）
❌ 把 SKIP 当成 PASS 报告
❌ 「CI 全绿」当作「全部验证过」
```

## 13.4 CI 全绿 ≠ 稳定

**这是本项目最需要警惕的错觉。**

```
CI 全绿可能意味着：
  · 测试真的都过了 ✅
  · 或者：关键测试被 SKIP 了，什么都没测 ❌

判断方法：
  看 CI 日志里的 SKIP 数量
  如果关键验收门在 SKIP 列表里 → CI 绿是假的
```

**相关 issue：#295（让 CI 装上 poppler，让 9 条门真跑）**

---

# 四、任务执行

## 14. 先读 issue 全文，不要只看标题

```
issue 正文的「【任务指令】」章节 = 完整派单
包含：背景 / 目标 / 验收 / 纪律 / 裁定点

不要：
  ❌ 只看标题就开始写
  ❌ 跳过「明确不做」章节（会做多余的事）
  ❌ 忽略「需要 Lead 裁定」的点（要主动提）
```

## 15. 有歧义就问，不要猜

```
✅ 「issue 里说的 X 我理解成 Y，对吗？」
✅ 「这里有两个方案，我倾向 A，理由如下，请确认」
❌ 猜一个方向埋头做 2 小时
```

## 16. 主动报告偏离

```
如果实现与原计划不同：
  · 在 PR 描述里明确写「原计划 X，实际做了 Y，因为 Z」
  · 不要悄悄改方案
```

## 17. 发现新问题开 issue，不要顺手修

```
✅ 发现缺陷 → 开 issue → 继续做手上的
❌ 发现缺陷 → 顺手改了 → PR 变大 → 审核困难

例外：修复是当前任务的前置条件时，可以顺带修，但要在 PR 里单独 commit 说明。
```

---

# 五、数据与语料

## 18. 不要为了样本过拟合

```
当前 10 份真实语料，但只有 3 个平台来源（黑龙江/内蒙古/未知）。
章节骨架 Jaccard：组内 0.95-0.96，跨平台 0.028。

要求：
  · 识别规则用「形态」不用「字面」
    ❌ if chapter == "采购需求"
    ✅ if 章节含「参数表/技术要求」表格
  · 每条结论标注适用范围：「基于 3 个平台来源，未验证其它省份」
  · 发现硬编码固定字符串 → 单独开「过拟合缺陷」issue
```

## 19. 语料文件名不改

```
❌ tender.pdf
✅ 超融合集群招标文件（[230001]GCZX[GK]2026000720261023001）.pdf

理由：文件名带项目名称和编号，无上下文 Agent 会从文件名读这两项。
      改名会剥夺这条路径，改变测试条件。
```

---

# 六、沟通

## 20. 汇报格式

```markdown
## 交付概要
PR #xxx / 分支 / commits

## 做了什么
（按验收条目对应）

## 实测证据
（命令 + 输出，不要转述）

## 未做的 / 有偏离的
（如实列）

## 需要 Lead 裁定的
（如果没裁定点就写「无」）
```

## 21. 不要美化

```
❌ 「基本完成」「应该没问题」「大致符合预期」
✅ 「完成 8/10，第 9 项卡在 X，第 10 项未开始」
```

---

# 七、跨仓协作

## 22. 两仓隔离

```
mdcore          引擎（文档处理：PDF/DOCX/XLSX/MD/Canvas）
ai-bid-research 标书应用（消费引擎）

规则：
  · 引擎能力改动 → mdcore
  · 标书业务改动 → ai-bid-research
  · 不要在 ai-bid 里实现引擎能力
  · 不要在 mdcore 里写标书逻辑
```

## 23. 依赖关系

```
ai-bid 依赖 mdcore（通过 crates.io 或 git rev）
改动 mdcore 后，ai-bid 要同步升级依赖版本。
```

---

# 八、快速检查清单

**开工前**：
```
[ ] 在 issue 上留言认领（含身份）
[ ] 确认 issue 没被别人认领
[ ] 全局搜过同题 issue（开新 issue / 认领前都要搜，见 0.0.12）
```

**开分支前**：
```
[ ] git fetch origin main && git checkout -f origin/main
[ ] git diff --stat origin/main 无意外文件
```

**每个 commit 后**：
```
[ ] git push 成功（看到 xxxx..yyyy）
[ ] git log origin/<branch> 确认云端有
```

**写测试时**：
```
[ ] 涉及真实语料/外部工具 → 加了 skip_if_env_missing / skip_if_fixture_missing
[ ] SKIP 有横幅和原因（用仓里的工具函数，不手写 return）
[ ] 本地能跑的必须真跑，跑不了的如实报告
```

**开 PR 前**：
```
[ ] 最后一次查同题 open PR（认领→PR 之间有窗口，见 0.0.13）
```

**开 PR 后**：
```
[ ] PR 描述开头有「**提交者**：<身份>」
[ ] 立即开下一项，不等 CI
[ ] PR 描述含：验收对照 + 实测证据 + 未做的
```

**每次 push 前**：
```
[ ] 看 PR 最新评论，有没有「必须修 X」的 Review
[ ] CI 红过的话，看日志找到具体失败的测试，不要猜
[ ] 修完本地验证再 push
```

**CI 卡住时**：
```
[ ] 查步骤状态：所有测试步骤 success 但 run 不结束 = 基础设施问题
[ ] 日志里 test result: FAILED 计数为 0 = 代码是好的
[ ] 处理：本地重跑测试 + 修完再推，代码一行不改
```

**PR 合并后**：
```
[ ] issue 留「已完成」评论（含 SHA + 证据）
[ ] 关闭 issue
[ ] 删除已合并分支
```
