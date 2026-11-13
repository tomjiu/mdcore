# architecture-decisions.md — 架构原则与下一阶段

> **基线声明**：本文调查基线 = main `83d63d8`（2026-11-09，含 #633 LOK worker 合并）。
> 入库时远端 main 已前进至 `3d8d203`（2026-11-12）：LOK 已在 mang-shell 接线（#50 C1/C2）、
> 旧壳 crates/shell-native 已剪裁移除（本 PR）。正文表述保留基线时点原话，
> 逐项更新见 `docs/architecture-audit/2026-11-12-update.md`。

> 调查日期：2026-11-12 · 基线 main `83d63d8` · 只读调查产出

## 1. 本轮建议固定的架构原则（依据实测）

| # | 原则 | 依据 |
|---|---|---|
| D1 | 三域纪律（MD/PDF/Office 独立域模型+引擎，零 path 交叉）维持不变 | 依赖审计实测零渗透（workspace-dependency-audit.md §3） |
| D2 | LOK 只经 worker 进程接入，宿主永不加载内核 | #633 铁律已实测（段错误/C++ 异常穿透风险在档） |
| D3 | office-io 原位外科手术面保留为「轻量编辑/降级路径」，不因 A 接线而删除 | docx-fidelity 优势（未改字节不动）+ 降级门既有结构 |
| D4 | soffice 子进程导出与 LOK saveAs 二选一收敛为「LO 基线 ≥25.8」单一版本策略 | PDF 2.0/AES-256/PDF-A4 只在 25.8+（pdf-engine-analysis.md） |
| D5 | AI 只经 RPC 命令面操作文档，禁止直改盘/直碰引擎对象 | 现有拒改门（rpc.rs:1518/1966）是正确防线，应保持并扩展到 LOK 形态 |
| D6 | pdfium（读渲染）+ lopdf（增量写）双引擎分工维持；不引入 MuPDF/新 PDF SDK | 229 测试 + Acrobat 对齐验证在档 |
| D7 | 双前端（web/桌面）在统一编辑内核前，不得互相同步状态——先收敛单链内纪律 | state-ownership 报告：跨链无冲突检测是结构性事实 |

## 2. 暂不能决定的事项（明确留给下阶段）

1. **MCP→LOK 命令形态**（G1）：insert_text 走 postKeyEvent 还是 office-io 直改+LOK 重载——需原型对比两种往返的延迟与保真。
2. **LO 捆绑与裁剪路线终选**（G3）：动态 mergelibs（~90-120MB 压缩）vs 全静态（~60MB，70+ 补丁维护）——需先跑通一条真实构建线才能裁决；任务书引用的 377.9MB/86.1MiB 实验数据本轮未能复核（仓库无产物），须重做。
3. **web 链命运**：Markdown 降级链与 A 路线的关系（继续作轻量 web 查看？还是导入导出走 LOK？）——产品决策非代码决策。
4. **shell-native 删除**（#331 P1 关联）：需产品确认 GPUI 主线后归档。
5. **LOK undo 归一**（G4）：worker undo 命令面未实现。

## 3. 下一阶段优先级（建议顺序）

1. **P0：G2 实机门**——Windows LOK 实测 + LO 25.8 升级评估（ABI 复核 + PDF 2.0 验证），一次投入双收益。
2. **P0：G1 原型**——MCP insert_text 的 LOK 形态 200 行级 spike（复用 smoke.rs 骨架），量测往返延迟。
3. **P1：G3 构建线**——按 trim-analysis §4 路线表跑动态裁剪 MVP（删 Java/Python/扩展/PDF Import + dict-en only），用 office-io 既有 convert 测试做回归门。
4. **P1：状态收敛**——get_state 盘读偏差修补（AI 决策前 flush）+ batch_replace 双语义拆分（独立工具名）。
5. **P2**：web 链产品定位决策；shell-native 归档。

## 4. 对任务书 12 个核心问题的最终回答（浓缩）

1. A 路线能力：LOK 原型 Linux 全链 26/26 实测过、Windows/macOS 门未执行；未接线任何前端——**半成品**。
2. LOK 是当前最合理主编辑引擎吗？**是（作为排版/导出内核）**，但需过 G1-G4；轻量编辑保留 office-io 面。
3. 可安全裁剪：Java/Python/Basic/扩展/数据库/报表/PDF Import/字典（留 en）/模板/画廊；字体不捆。依据 DSHKit+ElyOffice 双源核实。
4. DSHKit 静态方案可用于 mdcore 吗？技术可行（四平台生产验证），但 70+ 补丁维护成本高——**远期选项，非首选**；其「自动回滚」声称不实。
5. ElyOffice 值得长期维护的补丁：LOK callback/tile/焦点/包损坏拒绝系列（0001/0002/0043/0044/0057/0064 等）；其 mupdf/gpui/ely.xcd 补丁不需要。
6. MuPDF 继续作 PDF 核心引擎？**问题不成立——从未使用**；pdfium+lopdf 维持。
7. PDF 2.0 输出由谁负责？**LibreOffice ≥25.8**（soffice 或 LOK saveAs 同引擎）；自有面只做增量修订。
8. 真实依赖渗透数：**0**（9 类担忧逐条核查无一成立）；结构噪声 2 处（shell-native 死重、mang-shell 厚壳）。
9. 拆/合/隔离建议：office-io 远期拆 docx-edit 面；shell-native 删除（待确认）；其余 17 crate 保留。
10. A 路线待解决：G1 命令形态、G2 实机+25.8、G3 捆绑构建线、G4 undo。
11. B 值得迁移吗？B-桌面变体已是现状（保留）；B-纯自有排版**不可行**（无排版引擎，用户已裁定不补）。
12. 体积优化空间：现状 0（不捆 LO）；捆绑后动态路线 ~90-120MB 压缩、静态极限 ~60MB；应用本体另有 #331 三处打包减重（web/dist 硬依赖已移除、GTK 死重、pdfium 不对称）。

## 5. 调查纪律执行说明

- 未修改任何代码/构建产物/Issue/PR（任务书「做完提交 PR 报告」——本轮为只读调查，PR 报告以本目录 7 份文件交付；如需入库可后续以 docs PR 形式提交）。
- 任务书引用的裁剪实验数据（981.3MB/377.9MB/86.1MiB）未能复现出处，已标记待重做——未当作结论使用。
- 所有体积数字标注口径：LO Debian 包未压缩 294MB（本机实测）；DSHKit/ElyOffice 数字为其自报发布口径。
- 区分标记：已实现=有测试+代码；原型=#633；仅有接口=shell-native；未实现=LOK undo/Windows 实机门。


---

## 后记（2026-11-12 入库轮）

本节以下正文为 `83d63d8` 基线调查原话。入库时两点已变，详见
`2026-11-12-update.md`：① LOK 零消费者 → mang-shell 已接线（`#50 C1/C2`：
`editor/docx_lok_session.rs` 833 行会话层 + `docx_pages.rs` 条带渲染 + 壳层
`ensure_docx_pages` 打开入口；本机 grep 实测）；② 本 PR 将旧壳 `crates/shell-native`
剪裁移除（workspace 19→18，Cargo.lock 连带移除 GTK/tao/wry 树 45 包）。
