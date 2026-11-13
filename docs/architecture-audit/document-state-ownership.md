# document-state-ownership.md — 文档状态归属调查

> **基线声明**：本文调查基线 = main `83d63d8`（2026-11-09，含 #633 LOK worker 合并）。
> 入库时远端 main 已前进至 `3d8d203`（2026-11-12）：LOK 已在 mang-shell 接线（#50 C1/C2）、
> 旧壳 crates/shell-native 已剪裁移除（本 PR）。正文表述保留基线时点原话，
> 逐项更新见 `docs/architecture-audit/2026-11-12-update.md`。

> 调查日期：2026-11-12 · 基线 main `83d63d8` · 只读调查

## 1. 核心结论

**不存在单一权威状态。当前是「三条链、两套编辑模型、两个 undo 栈」的并行状态格局**，且每条链内部又存在「内存模型 vs 磁盘 OOXML」双层。LOK 状态当前不存在于任何生产链（零消费者原型）。

## 2. 状态归属总表

| 状态 | 当前所有者 | 是否唯一 | 是否同步 | 潜在冲突 |
|---|---|---|---|---|
| 桌面 DOCX 内存状态（行模型） | mang-shell Buffer（Zed rope）+ office-io DocxEditor（段落索引） | 链内唯一 | 延迟同步（flush 时写 XML） | 撤销回放绕过钩子→靠 flush 兜底（mod.rs:1909） |
| 桌面 DOCX 磁盘状态 | .docx 文件（quick-xml 原位改写 + 临时文件 rename） | 链内唯一 | 保存时同步 | AI/人工共用，盘直改被拒改门拦截 |
| 桌面 undo 状态 | mang-shell buffer.rs:206 四元组栈（非 mdcore History） | 链内唯一 | — | 与 web 链 undo 完全独立 |
| web DOCX 内存状态 | mdcore::Core BlockTree（DOCX 已降级 Markdown） | 链内唯一 | apply_ops 即时 | 降级有损：图片/合并单元格/域丢失（stats 计数） |
| web undo 状态 | mdcore history.rs（Source=View/Ai/Undo/Redo/System 同栈） | 链内唯一 | — | AI 与人手同栈，Source 区分 |
| AI 操作入口状态 | 桌面链：经 RPC 操作同一 Buffer；web 链：Source::Ai apply_ops | 两条链各自共享 | — | MCP 只连桌面链，不连 web 链 |
| LOK 文档状态 | mdc-lok worker 进程内 DocHandle（脏位仅 saveAs 清零） | **不存在于生产链** | — | 原型：#633 |
| Evidence/Overlay 文档标识 | evidence 指纹（FNV-64/SHA-256）+ sidecar 文件 | 独立体系 | 只读验证 | 指纹锚定磁盘字节，编辑后需重算（workbench 管道） |
| XLSX 状态 | office-io xlsx_edit + mang-shell xlsx_grid_view | 链内唯一 | 保存同步 | 同 DOCX 桌面模式 |

## 3. 按检查点逐项核实

### 3.1 LOK 内部文档状态
存在且语义严谨（client.rs DocHandle 脏位：编辑置位，仅 saveAs 成功清零；超时/死亡/保存失败保留），但**零消费者**——状态冲突问题对它是「未来的问题」。#633 落地时的关键设计：宿主永不 dlopen 内核、单文档 worker、_exit 跳过 LO atexit。

### 3.2 Rust Document Model 状态
两个互不相通的实现：
- **mdcore::Core**（web 链）：BlockTree + op 流 + history + snapshot/restore（20k 块三重恒等实测，gates.md §15.3 Gate3 PASS）。AI（`replace_text_at_anchor`，lib.rs:82 注释「锚点寻址（AI 路径）」）与人手共用。
- **office-io DocxEditor + mang-shell Buffer**（桌面链）：行模型 + 段落号寻址，非 BlockId。

**BlockId 仅存在于 web 链**（mdcore id.rs u64 BlockId，NodePath 寻址）；桌面链的寻址是「段落索引 + 行号」，两套寻址空间无映射。mdbridge（mdcore op ↔ collab 命令，block_id 字符串寻址）也只在 web 链。

### 3.3 OOXML 文件状态
- 桌面链：原位保真（未改部件逐字节透传），临时文件 + rename 原子写。
- web 链：**不存在「同一文件的往返」**——导入即降级 Markdown，导出是 docx-rs 全新生成（原文件格式不保真）。
- **不存在两个模块同时修改同一个 DOCX 的情况**：桌面链单一 flush 序列；web 链导出是下载新文件。MCP 的 write_file/batch_replace 对 docx 行模型态显式拒改（rpc.rs:1518/1966）——这是防止「盘直改 vs 内存行模型」分裂的现存防线。

### 3.4 AI 工具操作时使用的状态
AI 感知的状态 = mang-shell 活动标签 Buffer + get_state 时从盘上读的 content（注意 get_state 的 content 经 read_file 取盘上版本，若标签脏则与 Buffer 有瞬时偏差——RPC 面未做合并读）。批量操作 batch_replace 对已打开文件走 buffer（可 undo）、未打开走盘直改（via=disk 不可 undo）——**同一工具双语义**，是真实的一致性风险点（有 via 标记可诊断，无强制收敛）。

### 3.5 Overlay/Evidence 使用的文档标识
evidence = 内容指纹（Fingerprinter trait：Fnv64/SHA-256），与文档内存状态无引用关系，只锚定磁盘字节快照。副作用：**任何编辑后旧指纹即失效**（这是设计语义：证据锚定的是「被引用那一刻的文件」），不是缺陷，但意味着 Overlay/Evidence 与编辑状态天然解耦、靠 sidecar 文件持久化。

### 3.6 保存与撤销使用的状态
- 桌面保存：flush（Buffer 行 → DocxEditor XML）→ save_to_vec → rename。**保存失败路径**：拒写目录测试显示失败保留脏位（lok 语义）；桌面链 docx 保存失败经 Editor::save 返回错误，临时文件机制保证原文件不被半写破坏。**「保存失败报告为成功」未发现证据**；但「flush 兜底在 undo 回放时才触发」意味着内存↔XML 差分窗口内崩溃 = 编辑丢失（有 .docx.saving 原子性兜底原文件）。
- web 保存：导出即下载，无服务器状态。

## 4. 专项风险清单（真实代码位置）

1. **双模型双栈**（结构性）：web/桌面两条 DOCX 链互不知道对方的存在。同一文件在两条链打开会产出两套编辑历史与两种保真度（原位 vs 重建），用户无感知选择机制。
2. **get_state 盘读偏差**（rpc.rs）：脏标签时 AI 看到的 content 是盘上旧版，可能基于旧快照决策——建议 AI 决策前强制 flush 或 get_state 走 Buffer 快照（本轮不实现，仅登记）。
3. **batch_replace 双语义**（rpc.rs:1880 起）：buffer 通道可 undo / disk 通道不可 undo，跨文件批量时同一命令混两种持久化语义。
4. **行模型降级门**（docx_doc.rs:223-309）：超门限/引擎拒/对账失败 → 只读。降级是静默的（用户拿到只读标签），AI 侧无对应感知工具。
5. **事件流**：桌面 doc_changed 订阅事件存在（rpc.rs），web 链 op 流即时——无跨链事件丢失问题，因为**根本不存在跨链同步**（问题更根本）。

## 5. 对「谁拥有权威状态」的回答

**现状**：每条链内「内存模型为权威、磁盘是序列化终点」，这个单一链内纪律执行得很好（原子写、脏位、拒改门、undo 同栈）。**跨链**：无权威、无同步、无冲突检测——因为两条链当前服务不同前端，从未被设计为同时操作同一文件。

**对 A 路线的含义**：若 #633 LOK 接线成为主编辑器，状态格局将从「2 模型」变「3 状态源」（Buffer 行模型、LOK 内核文档、磁盘 OOXML），#633 已预研的关键决策（宿主不加载内核、脏位仅 saveAs 清、worker 单文档）表明团队已按「LOK 内核为权威、宿主持投影」设计——但与现有 Buffer/MCP 工具链的合并点尚无设计（#616 接续时必须回答：MCP insert_text 最终落到 LOK postKeyEvent 还是继续走 office-io 外科手术）。


---

## 后记（2026-11-12 入库轮）

本节以下正文为 `83d63d8` 基线调查原话。入库时两点已变，详见
`2026-11-12-update.md`：① LOK 零消费者 → mang-shell 已接线（`#50 C1/C2`：
`editor/docx_lok_session.rs` 833 行会话层 + `docx_pages.rs` 条带渲染 + 壳层
`ensure_docx_pages` 打开入口；本机 grep 实测）；② 本 PR 将旧壳 `crates/shell-native`
剪裁移除（workspace 19→18，Cargo.lock 连带移除 GTK/tao/wry 树 45 包）。
