# mdcore 成果备份说明

- 生成时间：2026-11-09T14:11:12+00:00

- 备份目标：codeberg.org/mingjiu/mdcore（私有库）

## 背景

GitHub 账号被封禁（suspended / 403），GitHub 侧已不可读写。本地三处克隆（work/mdcore、mdcore、repo-mdcore）与 Codeberg 已迁移内容为仅存副本。

Codeberg 上已由迁移会话先行备份：main（含 GitHub main 全部历史，GitHub 最后已知 main = 958b9bb「Merge pull request #508」已并入）、约 47 个未合并分支、12 个 PR、backup/* 备份分支。

## 本次补传（共 30 项）

### 新建分支（未合并的 GitHub 快照分支 / 本地分支）

| 分支 | 提交 | 来源 | 说明 |
|---|---|---|---|
| docs/agents-0609-mother-scan | 671ba01ec5 | work | refs/remotes/origin/docs/agents-0609-mother-scan |
| feat/384-pdf-docx-text | 51668c9753 | work | refs/remotes/origin/feat/384-pdf-docx-text |
| fix/318-d1-mce-selection | dda60badc1 | work | refs/remotes/origin/fix/318-d1-mce-selection |
| fix/318-d6-empty-attr-sectpr | aadc5bbc77 | work | refs/remotes/origin/fix/318-d6-empty-attr-sectpr |
| fix/318-d7-strict-refs | 09e58d7834 | work | refs/remotes/origin/fix/318-d7-strict-refs |
| feat/385-b5a-panels-list | ce5279cf85 | sec | refs/heads/feat/385-b5a-panels-list |
| feat/385-ui-migration-batch1 | d1a149d4d5 | repo | refs/remotes/origin/feat/385-ui-migration-batch1 |
| fix/309-field-cache-write | c11e78168b | repo | refs/remotes/origin/fix/309-field-cache-write |

### 旧分支头备名（CB 同名分支已推进，旧头孤悬，存 backup/gh-*）

| 备名分支 | 提交 | 原 GitHub 分支 |
|---|---|---|
| backup/gh-feat/352-canvas-batch | fbbf1056be | feat/352-canvas-batch |
| backup/gh-fix/318-d5-odd-header-ref | e452eaa3b1 | fix/318-d5-odd-header-ref |
| backup/gh-feat/420-pdf-thumbs-sidebar | 9b7afe8afc | feat/420-pdf-thumbs-sidebar |

### 悬空 / 仅 reflog 可达的 WIP 提交（存 backup/wip-*）

| 备份分支 | 提交 | 原始信息 |
|---|---|---|
| backup/wip-671179ce | 671179cef5 | fix(#346): DOCX 组内文本框全保留——第二趟补发被丢成员（#318 D4）  |
| backup/wip-6eb3f377 | 6eb3f3778f | feat(#384): PDF→DOCX 文本版导出桥——office-io 写面 + 桌面菜单接线（#307 ②功能独 |
| backup/wip-de57a25a | de57a25a45 | docs(AGENTS): 0.0.11 项目状态用 GitHub 原生机制查，废弃任务池文档  |
| backup/wip-19207c69 | 19207c69a5 | On chore/341-fmt-gate-and-sweep: fmt-sweep-wip  |
| backup/wip-64857a16 | 64857a1663 | index on chore/341-fmt-gate-and-sweep: 575df6c feat(#266): 多 |
| backup/wip-6a623d28 | 6a623d2843 | index on chore/341-fmt-gate-and-sweep: 575df6c feat(#266): 多 |
| backup/wip-728a73c3 | 728a73c310 | index on fix/318-d2-tc-textbox: 575df6c feat(#266): 多级列表自动编号 |
| backup/wip-7ca28f1d | 7ca28f1d4a | index on chore/341-fmt-gate-and-sweep: 575df6c feat(#266): 多 |
| backup/wip-92f14c1f | 92f14c1fd0 | On chore/341-fmt-gate-and-sweep: verify-clean  |
| backup/wip-9480100d | 9480100d36 | index on chore/341-fmt-gate-and-sweep: 575df6c feat(#266): 多 |
| backup/wip-a8369904 | a836990466 | On chore/341-fmt-gate-and-sweep: scan-check  |
| backup/wip-af90a240 | af90a240e5 | On fix/318-d2-tc-textbox: clippy-base  |
| backup/wip-dc51aed9 | dc51aed92d | index on feat/485-docx-footnote-import: 5c56087 chore(#485): |
| backup/wip-e4abff6a | e4abff6a43 | On chore/341-fmt-gate-and-sweep: verify-clean3  |
| backup/wip-e9e9a717 | e9e9a71765 | index on chore/341-fmt-gate-and-sweep: 575df6c feat(#266): 多 |
| backup/wip-fab61f09 | fab61f09d7 | fix(#345): D2 表内文本框补提取——tc 内 DrawingML 文本框第二趟 + 账本 txbx_in_t |
| backup/wip-fb13be08 | fb13be0851 | WIP on feat/485-docx-footnote-import: 5c56087 chore(#485): C |
| backup/wip-fea9a2d6 | fea9a2d6b1 | On chore/341-fmt-gate-and-sweep: verify-clean2  |

### 标签

- `checkpoint/2026-10-19-pre-batch1` -> 0bb09dcb77

## 跳过项（97 条）

以下引用内容已并入 Codeberg main 历史（或 CB 已有同名同 SHA 分支），无需重复推送：

| 本地引用 | 原因 |
|---|---|
| chore/416-draft-sweep | 内容已并入 cb/main 历史，跳过指针 |
| docs/agents-issue-dedup | 内容已并入 cb/main 历史，跳过指针 |
| fix/408-stamp-signature-gate | 内容已并入 cb/main 历史，跳过指针 |
| fix/428-fold-current-walkup | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/audit/recheck-t0 | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/ci/358-wasm-test-coverage | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/docs/385-ui-migration-survey | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/docs/387-overlay-assessment | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/docs/agents-claim-timeout | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/docs/agents-conflict-markers | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/docs/agents-decompose | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/docs/agents-github-native | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/docs/agents-product-loop | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/feat/240-print-pipeline | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/feat/302-annotate-kinds | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/feat/304-page-transform | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/feat/308-digest-verification | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/feat/351-canvas-stroke | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/feat/389-xlsx-grid-view | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/240-web-print-pdf | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/318-d4-group-textbox | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/330-depends-gpui | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/fix/359-compliance-scan-packaging | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/365-d5-roundtrip-regression | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/fix/main-pagegeometry-sync | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/main | GitHub main 历史已并入 cb/main |
| feat/385-b1-context-menu | CB 已有同名同 SHA 分支 |
| feat/385-b5c-git-panel-list | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/main | GitHub main 历史已并入 cb/main |
| chore/341-fmt-gate-and-sweep | 内容已并入 cb/main 历史，跳过指针 |
| feat/419-json-canvas | 内容已并入 cb/main 历史，跳过指针 |
| feat/421-pdf-pages-zoom | 内容已并入 cb/main 历史，跳过指针 |
| feat/485-docx-footnote-import | 内容已并入 CB 历史（CB 分支 5c56087b 占名），跳过指针 |
| fix/318-d2-tc-textbox | 内容已并入 cb/main 历史，跳过指针 |
| fix/361-w1-toc-assertion-flip | 内容已并入 cb/main 历史，跳过指针 |
| fix/issue-469 | 内容已并入 cb/main 历史，跳过指针 |
| fix/issue-480 | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/ci/struct-field-guard | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/docs/270-desktop-acceptance-linux | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/docs/270-linux-acceptance | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/docs/460-enum-variant-match-check | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/docs/agents-branch-naming | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/docs/agents-ci-stuck | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/docs/agents-pr-base-check | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/docs/agents-prework-check | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/docs/agents-struct-field | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/feat/257-multisection-docx | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/feat/260-docx-structural-ledger | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/feat/261-latex-math | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/feat/264-pdf-corpus | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/feat/265-xlsx-corpus-gaps | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/feat/266-numbering-depth | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/feat/267-style-inheritance | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/feat/268-docx-columns | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/feat/283-xlsx-editor-wiring | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/feat/302-pdf-annotate-wiring | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/feat/307-docx-to-pdf | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/feat/352-canvas-batch | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/feat/385-b5b-file-tree-list | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/feat/385-ui-migration-batch3b | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/feat/389-xlsx-edit-wiring | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/feat/406-edge-route | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/feat/421-pdf-zoom | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/feat/493-encrypt-permissions | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/feat/496-xlsx-freeze-panes | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/fix/262-corpus-gate-empty-run | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/275-rpc-sandbox-escape | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/276-sanitize-wiring | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/277-stamp-encrypted-gate | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/278-md-depth-guard | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/279-worker-generation-cap | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/280-wasm-bridge-double-tax | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/281-statusbar-io-cache | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/282-web-sdk-boundary | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/289-ui-overhaul | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/304-page-transform | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/308-signature-digest | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/309b-para-align-spacing | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/318-d5-odd-header-ref | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/fix/319-packaging-mangshell | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/320-pdfium-bundle | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/321-desktop-action-cli | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/333-wayland-seat-preflight | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/358-ci-skip-removal | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/386-pdf-annotate-save | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/fix/issue-353 | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/issue-463 | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/fix/issue-466 | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/issue-472 | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/fix/issue-499 | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/issue-502 | CB 已有同名同 SHA 分支 |
| refs/remotes/origin/fix/issue-505 | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/issue-507 | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/fix/issue-509 | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/hotfix/pagegeometry-fields | 内容已并入 cb/main 历史，跳过指针 |
| refs/remotes/origin/main | GitHub main 历史已并入 cb/main |
| refs/remotes/origin/serial-verify/400-d-chain-stack | CB 已有同名同 SHA 分支 |

## 本分支（backup/agent-meta）内容

- `scripts/`：协作脚本工具箱（认领检查、PR 创建、CI 轮询、收口留证等）

- `work/`：issue/PR 快照 JSON、认领状态、分支-PR 映射、AGENTS.md

- `SUMMARY.md`：本说明

共 47 个文件。env.sh（含令牌）与疑似含密钥文件已排除。
