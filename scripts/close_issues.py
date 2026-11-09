#!/usr/bin/env python3
"""关闭已验证完成的 issue（带证据评论）"""
import json, subprocess, os, sys

TOKEN = os.environ['GH_TOKEN']

comments = {
'333': """Agent-3ivu 审计关闭（#392 固定项 2① 核查，非原交付人补关）

PR #340 已于 2026-11-07T05:58:24Z 合并：`fix(#333): Wayland 无 seat 优雅降级——panic 101 → exit 70 双语提示`。

main 直接证据：`mang-shell/src/wayland_probe.rs` 在档（PR #340 新增文件），退出码 70 路径在位。

验收对照：panic 101 → 优雅降级 ✅（PR 标题即交付面）。本 issue 悬空 3.5 小时，按 #392 审计纪律补关。""",
'341': """Agent-3ivu 审计关闭（#392 固定项 2① 核查，非原交付人补关）

PR #357 已于 2026-11-07T06:50:26Z 合并：`chore(#341): fmt 门禁补位 + 全仓漂移清扫——CI 双遍 --check 常驻`。

main 直接证据（本机 main @ 65f0834 实测）：
- `.github/workflows/windows-check.yml` 含 `fmt gate (main workspace)` 与 `fmt gate (mang-shell independent workspace)` 两个步骤（`cargo fmt --all -- --check`，注释标 #341）✅
- 漂移清扫面：PR #357 改动含 mdcore-wasm/docx_import.rs、office-io 四文件、mdcore/tests 等 23 文件 ✅

issue 悬空 3 小时+，按 #392 审计纪律补关。""",
'345': """Agent-3ivu 审计关闭（#392 固定项 2① 核查，非原交付人补关）

PR #370 已于 2026-11-07T09:20:54Z 合并：`fix(#345): D2 表内文本框补提取——tc 内 DrawingML 文本框第二趟（宿主表跨度后发射）+ 账本 txbx_in_tc`。

交付人 Agent-8ukb 已在任务池登记交付（#272 05:59:29Z 评论）。合并批落地后 issue 悬空，按 #392 审计纪律补关。母登记 #318 保持开放至 D1-D7 全收口。""",
'361': """Agent-3ivu 审计关闭（#392 固定项 2① 核查；与 #365 归并建议一并报 Lead）

PR #372 已于 2026-11-07T09:21:00Z 合并：`fix(#361): w1_toc_field 断言随 #338 dirty 语义翻转同步`。

main 直接证据：`crates/mdcore-wasm/src/docx_export.rs` 在档含 w1_toc_field 断言（本机 grep 实证）。

归并备注：#365 与本 issue 主题重叠（其正文第一项 = 本 issue 全部内容；剩余 D5 往返面有 PR #388 在途）。建议 Lead 将 #365 的 w1_toc 部分标注为已随本 issue 收口，仅按 D5 面验收 #388。""",
}

for n, body in comments.items():
    # 先发证据评论
    r = subprocess.run(['curl', '-s', '-X', 'POST',
        '-H', f'Authorization: token {TOKEN}',
        '-H', 'Accept: application/vnd.github+json',
        f'https://api.github.com/repos/mmqz/mdcore/issues/{n}/comments',
        '-d', json.dumps({'body': body})], capture_output=True, text=True)
    c = json.loads(r.stdout)
    # 再关闭
    r2 = subprocess.run(['curl', '-s', '-X', 'PATCH',
        '-H', f'Authorization: token {TOKEN}',
        '-H', 'Accept: application/vnd.github+json',
        f'https://api.github.com/repos/mmqz/mdcore/issues/{n}',
        '-d', json.dumps({'state': 'closed'})], capture_output=True, text=True)
    d = json.loads(r2.stdout)
    print(f"#{n}: comment={c.get('id')} state={d.get('state')} closed_at={d.get('closed_at')}")
