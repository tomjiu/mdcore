# architecture-options.md — A/B 路线比较与推荐

> **基线声明**：本文调查基线 = main `83d63d8`（2026-11-09，含 #633 LOK worker 合并）。
> 入库时远端 main 已前进至 `3d8d203`（2026-11-12）：LOK 已在 mang-shell 接线（#50 C1/C2）、
> 旧壳 crates/shell-native 已剪裁移除（本 PR）。正文表述保留基线时点原话，
> 逐项更新见 `docs/architecture-audit/2026-11-12-update.md`。

> 调查日期：2026-11-12 · 基线 main `83d63d8` · 只读调查
> **前提纠错**：任务书将 A 定义为「LOK 主编辑」现状。实测现状是 **B 已在桌面链运行**（自有行模型+office-io 原位编辑，LO 只做导出），A 是 #633 刚落地的原型方向。因此本表 A/B 均按「目标形态」评估，现状列作为第三列对照。

## 1. 指标对照表（实测/评估混合，来源标注）

| 指标 | A：LOK 主编辑排版 | B：自有编辑模型（现状桌面变体） | 证据来源 |
|---|---|---|---|
| 基础文本编辑 | ✅ 已验证（postKeyEvent 逐字替换 + saveAs 重开校验，26 步 smoke） | ✅ 已验证（Buffer+XML 外科手术，54 测试文件 320 fn） | gates/633 + office-io tests |
| 表格编辑 | ✅ smoke 单元格替换过；完整表格编辑面未测 | ✅ table_cells/grid_width/shape API + merged_cells/nested_tables 测试 | 同上 |
| 图片操作 | 渲染过（9.9% 彩色=探针图片）；编辑面未测 | 原位保真（未改部件字节透传）；web 链导入丢图 | 同上 |
| 分页与排版 | ✅ **A 的核心优势**：getDocumentSize 12474×17406 twips 实测；LO Writer 全排版 | ❌ 无排版——行编辑器；gates/633:3 记录用户已裁定「停止给行编辑器补页面排版」 | gates/633 |
| 撤销重做 | worker 面未实现 undo 命令 | ✅ Buffer 四元组栈 | 代码 |
| AI 编辑 | 未接线（#616 接续点） | ✅ MCP 11 工具全链可用 | rpc.rs/server.py |
| 保存保真 | saveAs 走 LO 序列化（重排整个 XML，保真度=LO 读写往返） | ✅ 最高（未改段落字节不动，docx-fidelity-matrix.md） | 两者语义不同 |
| PDF 结果 | LO 内核 PDFWriter（所见即所得） | soffice 子进程同引擎（一致）；但壳内显示≠PDF 排版 | convert.rs |
| 字体一致性 | ✅ 内核排版=导出排版 | ⚠️ 壳内自绘无分页，与 PDF 排版天然不一致 | 结构性 |
| 运行时体积 | 捆绑 LO：动态裁剪 ~90-120MB 压缩 / 静态 ~60MB（DSHKit 上限参照） | 0（用户自装 LO 仅导出用） | trim-analysis |
| 内存占用 | worker 进程模型（LO 全内核常驻，典型数百 MB） | 壳进程内（行模型轻量） | 未做定量实测（登记） |
| 开发维护成本 | 高：LO 升级 ABI 漂移（26.x Doc#19 插入实证）+ 100+ 坑位补丁（ElyOffice 地图）+ 协议/worker 维护 | 中：office-io 已 320 测试，但排版/复杂 DOCX 能力天花板持续投入 | 评估 |
| 依赖复杂度 | +1 外部运行时（三域纪律已预留隔离） | 零引擎依赖 | 实测 |

## 2. B 路线关键风险实测（任务书点名的分页一致性）

**确认成立且已事件化**：B-桌面变体的壳内显示是**无分页行投影**（display_map.rs），与 soffice 导出的 PDF 分页布局**没有一致性机制**——不存在「投影排版=导出排版」的代码路径。web 变体（Markdown 化）同样无分页（mdcore pagination.rs 是 MD 分页语义非 DOCX 排版）。任务书列的 9 项复杂 DOCX 测试（跨页表格/页眉页脚/浮动图片/分节符…）在 B 路线**没有可运行的受测对象**（无排版引擎），office-io 的 docx_paragraph_layout/docx_columns_corpus 测试证明的是 XML 结构保真，非视觉排版。**结论：B 路线无法通过「分页一致性」测试，因为它没有分页**——这正是 #633 立项动机（用户裁定停止补排版）。

## 3. 推荐与切换条件

**推荐：A（LOK 接线）作为排版/导出内核，B 的 office-io 原位编辑面保留为「轻量快速编辑」降级路径。** 理由：
1. 排版能力是文档编辑器的硬需求，B 已被裁定不补（gates/633 用户裁决在档）。
2. A 的隔离成本已预付：worker 协议/脏位/看门狗/双 profile 并行全部就绪且实测（本机复跑 19.09s 通过）。
3. office-io 外科手术面不浪费：只读降级路径（对账失败→只读）可反向升级为「快速模式」。

**切换条件（进入 A 主线的 Gate）**：
- G1：MCP insert_text/replace_text 的 LOK 形态（postKeyEvent vs office-io 直改）设计定稿——状态归属报告 §5 指出的合并点。
- G2：Windows 实机门（#633 遗留）+ LO 基线升 25.8+（PDF 2.0 一次到位）+ ABI pin 复核。
- G3：捆绑 LO 裁剪构建线落地（trim-analysis §4 路线表，动态优先）。
- G4：LOK undo 语义（worker 命令面补 undo/redo + 与壳 undo 栈的归一策略）。

**不推荐**：全面迁移 B-纯自有排版（等于重写 Writer）；维持现状不动（排版缺口是已裁定的未了项）。


---

## 后记（2026-11-12 入库轮）

本节以下正文为 `83d63d8` 基线调查原话。入库时两点已变，详见
`2026-11-12-update.md`：① LOK 零消费者 → mang-shell 已接线（`#50 C1/C2`：
`editor/docx_lok_session.rs` 833 行会话层 + `docx_pages.rs` 条带渲染 + 壳层
`ensure_docx_pages` 打开入口；本机 grep 实测）；② 本 PR 将旧壳 `crates/shell-native`
剪裁移除（workspace 19→18，Cargo.lock 连带移除 GTK/tao/wry 树 45 包）。
