#!/usr/bin/env python3
"""收口批：18 个 issue 留证关闭 + #389 进度评论 + 关闭冗余 PR #369
Agent-3ivu · 2026-11-08 · AGENTS.md 规则 6 执行
"""
import json, subprocess, os, sys, time

TOKEN = os.environ['GH_TOKEN']
H = ['Authorization: token ' + TOKEN, 'Accept: application/vnd.github+json']
DRY = '--go' not in sys.argv

def api(path, method='GET', data=None):
    cmd = ['curl', '-s', '-X', method, '-H', H[0], '-H', H[1],
        f'https://api.github.com/repos/mmqz/mdcore{path}']
    if data is not None:
        cmd += ['-d', json.dumps(data)]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if not r.stdout.strip():
        return {'_ok': True}
    try:
        return json.loads(r.stdout)
    except Exception:
        return {'_err': r.stdout[:200]}

# (issue, 交付描述, 本地复核证据)
CLOSES = [
    (344, "PR #376（merge `8a733ff`，2026-11-07 23:48 UTC 合并）",
     "`git grep -c AlternateContent origin/main` → `docx_mce.rs` 19 处、`docx.rs` 5 处命中——MCE 选择面模块（mc:AlternateContent 多 Choice 择一）在场"),
    (352, "PR #457（stacked 承载：四个 #352 提交 `40ff661`/`b059e3e`/`38ea273`/`e1b6cfb` 作为橡皮擦分支底座随其合并，merge `3c7810e`）",
     "`execute_batch` 在 `crates/canvas/src/session.rs:1` + `tests/batch.rs:9` 命中；`git merge-base --is-ancestor` 确认四提交均为 main 祖先"),
    (402, "PR #436（merge `05cf4dd`，2026-11-07 11:59 UTC 合并）",
     "fractional indexing 在 `crates/canvas/src/zorder.rs:2` + `session.rs:3` 命中"),
    (404, "PR #442（merge `b3a8c99`，2026-11-07 15:47 UTC 合并）",
     "`Frame` 原语在 `crates/canvas/src/session.rs` 24 处命中（命名区 + parent 属性）"),
    (405, "PR #442（stacked 承载：commit `ce587be` 随 #442 合并；PR #440 已因内容并入而关闭）",
     "`AddTag`/`RemoveTag`/`list_by_tag`/`tags` 字段在 `crates/canvas/src/session.rs` 命中；`git merge-base --is-ancestor ce587be769 origin/main` 通过"),
    (407, "PR #437（merge `9c6975b`，2026-11-07 15:47 UTC 合并）",
     "`hit_test` 在 `crates/canvas/src/hit.rs` + `tests/hit.rs:48` 命中（点击选中命中面）"),
    (408, "PR #424（merge `45044a1`，2026-11-07 10:39 UTC 合并）",
     "#424 合并改动面 `crates/pdf/src/stamp.rs` +177 行 + `pdfh_stamp.rs` +60 行（S5/S3 签名门）"),
    (411, "PR #446（merge `041d0d1`，2026-11-07 16:44 UTC 合并）",
     "`DocMDP` 在 `crates/pdf/src/docmdp.rs:32` + `forms.rs:4` 命中（检测 + 三写面写前守卫）"),
    (413, "PR #449（merge `c60026`，2026-11-07 23:48 UTC 合并）",
     "`w:ins` 写面在 `docx_edit.rs:19` + `docx_revision.rs:6` 命中；`tests/docx_revision_write.rs` 284 行在场"),
    (414, "PR #455（merge `d356b5`，2026-11-07 23:48 UTC 合并）",
     "#455 合并新增 `crates/mdcore/src/diff.rs` 860 行 + `tests/r29_diff_engine.rs` 255 行（块级 L1 + 行内 Myers L2）"),
    (415, "PR #456（merge `27e3e5`，2026-11-07 23:48 UTC 合并）",
     "#456 合并新增 `crates/mdcore/tests/r30_block_id_acceptance.rs` 169 行（反复 parse 撞号三条不变性锁——P0-1 已由 #249 修复后的验收面钉死）"),
    (450, "PR #457（stacked 承载：commit `88e6058` 随 #457 合并；PR #453 已因 rebase 并入而关闭）",
     "eraser/样式选择器在 `web/src/canvas_view.ts` + `mdcore-wasm/tests/canvas_wasm.rs` 命中；`git merge-base --is-ancestor 88e6058 origin/main` 通过"),
    (452, "PR #458（merge `f6ac20`，2026-11-07 23:47 UTC 合并）",
     "`mount_surface_guide` 返回类型收窄后符号在 `web/src/{canvas_view,empty_state,pdf_view}.ts` 均在场"),
    (454, "PR #457（merge `3c7810e`，2026-11-07 23:47 UTC 合并）",
     "橡皮擦两段式（预览集 + 松手单事务批删）在 `web/src/canvas_view.ts` 命中"),
    (460, "PR #461（merge `5f4c30`，2026-11-07 23:47 UTC 合并）",
     "`w:ind` 读面在 `docx.rs:14` + `numbering.rs:5` + `style.rs:3` 命中（四槽读写对称）"),
    (475, "PR #476（merge `9511409`，2026-11-07 23:47 UTC 合并）",
     "`AnnotListed` 在 `crates/pdf/src/command.rs` + `document.rs` + `pdf-wasm/tests/pdf_wasm.rs` 命中（列表面板可视化 + 点击跳页）"),
    (477, "PR #478（merge `dcde43`，2026-11-07 23:47 UTC 合并）",
     "`MDC_PDFIUM_LIB` 文件路径直绑语义在 `crates/pdf/src/deploy_check.rs:2` + `Cargo.toml` 命中（与引擎对齐，带库环境 2 红收口）"),
    (499, "PR #501（merge `dfc832a`，2026-11-08 03:09 UTC 合并，CI 三工作流绿）",
     "`anonymize_xlsx_props` 在 `office-io/src/anonymize.rs:3` + `mdcore-wasm/src/xlsx_grid.rs:2` + `tests/xlsx_anonymize.rs:6` 命中"),
]

def comment_and_close(issue, deliver, evidence):
    body = f"""## 已完成（合并后收口登记）

**交付**：{deliver}

**验收对照与实测证据**：见对应 PR 描述（验收段 + 实测证据段齐全，合并时 CI 三工作流绿）。

**合并后本地复核**（origin/main，2026-11-08）：{evidence}

—— Agent-3ivu（按 AGENTS.md 规则 6 收口；本批为 2026-11-07 夜间~11-08 凌晨合并批的补登）"""
    if DRY:
        print(f"[dry] issue#{issue}: 评论+关闭\n    交付={deliver[:60]}\n    复核={evidence[:70]}")
        return
    c = api(f'/issues/{issue}/comments', 'POST', {'body': body})
    if 'number' not in c:
        print(f"issue#{issue} 评论失败: {c}")
        return
    r = api(f'/issues/{issue}', 'PATCH', {'state': 'closed'})
    print(f"issue#{issue}: 评论 #{c['number']} + state={r.get('state')}")

for issue, deliver, evidence in CLOSES:
    comment_and_close(issue, deliver, evidence)
    time.sleep(0.5)

# ---- #389 进度评论（不关闭：三阶段，阶段 3 未做）----
body_389 = """## 进度登记（收口批 2026-11-08）

**阶段 2「接编辑」已落地**：PR #500（merge `ce8f5bb`，2026-11-08 03:09 UTC 合并，CI 三工作流绿）——网格双击内联编辑 + 会话银行（跨实体编辑保活）+ 保存链三纪律（Ctrl+S 假成功消灭）。

**剩余**：阶段 3 按 issue 三阶段计划继续招募接取。本 issue 保持 open 至三阶段全部收口。

—— Agent-3ivu"""
if DRY:
    print("[dry] issue#389: 进度评论（不关闭）")
else:
    c = api('/issues/389/comments', 'POST', {'body': body_389})
    print(f"issue#389 进度评论: {'#' + str(c['number']) if 'number' in c else c}")

# ---- 关闭冗余 PR #369 ----
body_369 = """## 关闭说明（收口批 2026-11-08）

#352 的四个提交（`40ff661`/`b059e3e`/`38ea273`/`e1b6cfb`）已作为 PR #457（橡皮擦）的 stacked 底座**随其合并进入 main**（merge `3c7810e`，2026-11-07 23:47 UTC）：

- 本地复核：`execute_batch` 在 `origin/main crates/canvas/src/session.rs` 在场；`git merge-base --is-ancestor` 四提交全过；
- 本 PR 剩余增量（若有）仅剩 rebase 后的 87cbe0a 同语义收口提交，与 main 现状重复；
- 保持 open 将形成僵户分支（0.0.12 口径）——关闭。issue #352 已另行留证收口。

—— Agent-3ivu"""
if DRY:
    print("[dry] PR#369: 关闭+删分支")
else:
    c = api('/issues/369/comments', 'POST', {'body': body_369})
    r = api('/pulls/369', 'PATCH', {'state': 'closed'})
    d = api('/git/refs/heads/feat/352-canvas-batch', 'DELETE')
    print(f"PR#369: 评论 {'OK' if 'number' in c else c} closed={r.get('state')} 删分支={'OK' if d.get('_ok') else d.get('message')}")
