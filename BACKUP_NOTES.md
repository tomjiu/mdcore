# 备份清单（Backup Manifest）—— GitHub 账户封禁应急备份

> 生成：2026-11-09 · Agent-ix4l
> 原仓库：github.com/mmqz/mdcore（账户已被封禁，2026-11-09 起无法访问）
> 备份目标：codeberg.org/mingjiu/mdcore（本仓库，仅作成果备份，不触发 CI）

## 一、基线（main）

- 本仓 `main` = 快照提交 `931c2715`（原 GitHub mmqz/mdcore main @ `83d63d8e`，2026-11-09 抓取）。
- 快照为**单条孤儿提交，不含提交历史**。
- 本分支（`backup/history-graft`）在快照之上嫁接回**完整真实提交史**（905 个提交，
  含 GitHub main @ `a3dc14b` 即最后一次成功 fetch 的状态、PR #471…#619 全部合并拓扑），
  树内容与快照完全一致（`-s ours`，零内容改动）。

## 二、分支备份（全部为真实提交历史，已推送本仓）

| 分支 | 尖端 | 说明 |
| --- | --- | --- |
| `backup/github-main-a3dc14b` | `a3dc14b` | 最后一次成功 fetch 的 GitHub main（含 PR #471…#619 合并史） |
| `feat/306-ocr-wiring` | `4c492ab` | OCR 桌面读面接线 + 可搜索 PDF 写回（词级识别 + 3 Tr 隐形文本层） |
| `feat/385-b4-menu-bar` | `58a40c8` | 菜单栏迁移 ui::ContextMenu 批 4（手写下拉/遮罩/键盘导航退役） |
| `feat/420-pdf-thumbs-sidebar` | `cb295a1` | PDF 页面缩略图侧栏（低 dpi 懒加载 + 点击跳页 + 高亮跟随） |
| `feat/517-scan-note-2d-key` | `6adccb0` | 已并入 main（GitHub PR 已合并） |
| `fix/365-d5-roundtrip-regression` | `1db054b` | 已并入 main；本地独有分支，从未推送 GitHub |
| `fix/523-clippy-approx-constant` | `32381a5` | 已并入 main（GitHub PR 已合并） |
| `backup/stash-385-wip` | `4d2995d` | 未提交 WIP stash（385 批次中间态，9 文件 +337/−903） |

## 三、内容核验结论（2026-11-09）

对三个未合并分支（#306 / #385 批 4 / #420）与 stash 的关键内容逐一与基线快照比对：

- `#306`：`ocr_writeback.rs` 等核心文件与快照**逐字节一致**，其余 2 文件快照内容更多；
- `#420`：`pdf_thumbs.rs` 已在快照中（合并后又有小步精化）；
- `#385 批 4`：`menu_bar.rs` 已在快照中（快照多 20 行后续改动）；
- stash：40/40 关键标记全部命中快照。

**结论：GitHub 封禁前全部成果内容均已包含于基线快照（@ 83d63d8e），无内容丢失；
本备份补齐的是完整提交历史（作者/时间/提交信息/合并拓扑）与未合并分支的原始提交。**

## 四、CI 说明

- Codeberg 仓库 Actions 已禁用（`has_actions=false`）；
- 仓库无 `.woodpecker/` 配置，`.github/workflows/` 在 Codeberg 不执行；
- 因此全部推送与 PR **不触发任何 CI**；新增提交均带 `[skip ci]` 标记。

## 五、备份 PR（仅存档）

- `backup(#306)` / `backup(#385 批4)` / `backup(#420)`：三个成果分支的存档 PR。
  内容已在基线快照中，**无需合并**（因快照为孤儿提交，PR diff 含全树噪音，以分支尖端为准）。
- `backup: 历史嫁接 + 备份清单`（本分支）：如合并可将完整真实提交史接入 main，
  树内容不变（仅新增本清单文件）。
