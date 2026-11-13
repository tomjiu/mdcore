# architecture-current-state.md — mdcore 当前真实架构还原

> **基线声明**：本文调查基线 = main `83d63d8`（2026-11-09，含 #633 LOK worker 合并）。
> 入库时远端 main 已前进至 `3d8d203`（2026-11-12）：LOK 已在 mang-shell 接线（#50 C1/C2）、
> 旧壳 crates/shell-native 已剪裁移除（本 PR）。正文表述保留基线时点原话，
> 逐项更新见 `docs/architecture-audit/2026-11-12-update.md`。

> 调查日期：2026-11-12 · 证据基线：main `83d63d8`（含 #633 LOK worker 合并）
> 方法：cargo metadata 实测 + 全链代码追踪（file:line 引用）+ 本机运行时验证（LibreOffice 25.2.3.2 / Debian 13）
> 性质：只读调查，未修改任何代码与构建产物

## 0. 与任务基线的重大出入（先纠错，再展开）

任务书「A 路线」基线表与实际代码存在 **4 处重大出入**，本报告以代码实测为准：

| 基线假设 | 实测现状 | 证据 |
|---|---|---|
| LibreOfficeKit 负责 DOCX 交互编辑与排版 | **LOK 是零消费者原型**（#633，2026-11-09 刚合并），不在任何编辑链上 | 全仓 grep：`mdc-lok` 仅 workspace members 登记；mang-shell/web/office-io 均不依赖它 |
| MuPDF 负责 PDF 解析、渲染、提取 | **MuPDF 不存在**。PDF 引擎 = pdfium（读/渲染，pdfium-render 0.9.4 pin）+ lopdf 0.46（增量写） | `crates/pdf/Cargo.toml`；MuPDF 仅出现在注释（stamp.rs:651 交叉验证踩坑记录） |
| GPUI 桌面界面 | **属实但路径特殊**：GPUI 经 third_party/zed 搬运闭包（21→32 crate，源码零修改），桌面外壳 = mang-shell（独立 workspace） | `third_party/zed/PROVENANCE.md`；`mang-shell/Cargo.toml` |
| 单一 Document Model | **两套平行文档模型并存**：web/wasm 链（mdcore BlockTree）与桌面链（office-io 行模型 + XML 原位编辑），互不相通 | 见 §3 |

**真实架构一句话**：mdcore 当前是「双前端 + 纯 Rust Office 引擎 + 外挂式 LibreOffice 子进程（仅 PDF 导出）+ 未接线的 LOK 隔离 worker 原型」。

## 1. 真实模块版图（19 workspace crate + mang-shell 独立 workspace + web 前端）

### 1.1 主 workspace（/Cargo.toml，19 成员，cargo metadata 实测）

| Crate | 实测职责 | 直接依赖 |
|---|---|---|
| mdcore | Markdown 块模型核（BlockTree/op/history/snapshot/pagination） | ropey, serde, serde_json, sha2 |
| mdcore-wasm | wasm 桥（import/export/apply_ops/undo/snapshot） | mdcore, workbench, office-io, canvas, evidence, collab, wasm-bindgen |
| office-io | **Office 域 IO 引擎**：DOCX 读（docx-rs）+ 窄面原位编辑（quick-xml）+ 写（docx-rs DocxWriter）+ XLSX（calamine/rust_xlsxwriter）+ PDF 导出（soffice 子进程） | docx-rs, quick-xml, calamine, zip, ironcalc_base(经 formula), rust_xlsxwriter |
| office | Office 域基础设施（sanitize 敌意输出消毒） | 无（零三方） |
| formula | 公式引擎（ironcalc 包装） | ironcalc_base |
| lok | **LOK 隔离 worker 原型**（dlopen libmergedlo + 单文档 worker + JSON-lines 协议） | serde_json（仅此一项） |
| pdf | PDF 第二 Document Model（页/大纲/标注 + pdfium 读渲染 + lopdf 增量写） | pdfium-render*, lopdf*, serde, sha2 |
| pdf-wasm | pdf 的 wasm 桥 | mdc-pdf, wasm-bindgen |
| evidence | 外挂式证据系统基础层（指纹/sidecar） | serde, serde_json |
| workbench | 证据工作台管道（ingest→指纹→sidecar，只读验证链） | evidence, office-io |
| overlay | 跨文档索引/标注投影层（格式无关语义） | evidence, serde |
| overlay-pdf | overlay 的 PDF 组合层 | overlay, pdf, evidence |
| canvas | Canvas 引擎（节点/连线/分组独立领域模型） | evidence（仅 Fingerprinter trait）, yrs, serde_json |
| collab | Yjs 协作（yrs） | yrs, serde_json |
| mdbridge | mdcore op 流 ↔ collab 命令流翻译（应用层组合点） | mdcore, collab |
| relay | 服务端中继（权威副本 + 扇出） | collab |
| settings | 设置（toml_edit） | serde, toml_edit |
| shell | 宿主契约层（headless 可验证，零 GUI） | mdcore, settings |
| shell-native | **旧壳蓝图**（tao+wry+GTK3，ADR-H1） | mdcore, shell, gtk, tao, wry |

*均为 optional feature 门控，无 pdfium 二进制环境测试仍全绿。

### 1.2 mang-shell（独立 workspace，桌面 GPUI 外壳，~55k 行）

43 个顶层模块 + editor/ 子树（16 文件 20k 行）。依赖本地 crate：**office-io、pdf、canvas**（不依赖 mdcore、不依赖 lok）。third_party/zed 闭包：gpui/gpui_platform/theme/ui/menu/zlog/text 等 32 crate。

### 1.3 web/（浏览器前端）

TS + CM6 + wasm（mdcore-wasm / pdf-wasm）。18 个 src 模块。

### 1.4 sdk/mcp（Python MCP 服务器）

mang_mcp：11 个 MCP 工具，经 unix socket JSON-lines 直连 mang-shell RPC。

## 2. 五条真实调用链（DOCX 全生命周期）

### 2.1 桌面打开 DOCX（全部进程内纯 Rust，无 LOK 无子进程）

```
Ctrl+O (gpui prompt_for_paths) / 文件树点击 / 最近列表
  → ShellApp::open_document_tab          mang-shell/src/shell.rs:1636
  → document::open_editor → open          mang-shell/src/document.rs:180,93（"docx" 分派 :111）
  → docx_doc::load_editable              mang-shell/src/editor/docx_doc.rs:221
      std::fs::read 读字节
      ├─ 宽面：office_io::docx::read_docx_from_bytes_with_options   docx_doc.rs:239
      │        （docx-rs 模型级读，crates/office-io/src/docx.rs:136）
      └─ 窄面：office_io::docx_edit::DocxEditor::open(&bytes)       docx_doc.rs:245
               （quick-xml 扫 word/document.xml，docx_edit.rs:61）
      两扫描器按段落序对账；对不上 → 降级只读（bridge=None）
  → 行模型 DocxPayload{text, spans, LineBlock}   docx_doc.rs:126-140
  → Editor::new_docx_editable + install_docx_hook  editor/mod.rs:674,741
```

### 2.2 桌面人工编辑（Buffer rope + XML 外科手术）

```
键盘输入 → GPUI Editor → Buffer（Zed text::Buffer，rope+锚点，buffer.rs:196）
  → EditHook::before_edit（锁定行拒改）     docx_edit_bridge.rs:926
  → after_edit（行归属重排）                docx_edit_bridge.rs:934
  → 显示层即时更新；DOCX 侧延迟落：
    DocxEditing::flush                     docx_edit_bridge.rs:901
    → DocxBridge::compute（纯函数出 Writeback 计划）:642
    → engine.set_paragraph_text            office-io/src/docx_edit.rs:399
      （只改目标 w:p 的文本节点，其余字节不动）
```

### 2.3 桌面保存

```
Ctrl+S → save_tab (shell.rs:556) → Editor::save → save_docx   editor/mod.rs:1909
  ① DocxEditing::flush 兜底同步（撤销回放不走钩子）
  ② DocxEditor::save_to_vec              office-io/src/docx_edit.rs:1224
     （quick-xml 定点替换 document.xml + zip 重打包透传原条目）
  ③ 同目录临时文件 .docx.saving + rename 原子写盘   editor/mod.rs:1933-1950
```

### 2.4 AI 编辑 DOCX（MCP → RPC → 同一 Buffer，与人工同栈）

```
MCP 宿主 ⇄ stdio JSON-RPC ⇄ mang_mcp server（11 工具）
  ⇄ AF_UNIX socket JSON-lines（$XDG_RUNTIME_DIR/mang-shell.sock）
  ⇄ mang-shell rpc.rs ControlServer（27 命令，rpc.rs:354-380）
AI insert_text → cmd_insert_text (rpc.rs:922) → insert_text_at (mod.rs:1280)
  → buffer.replace_range —— 落在同一 Buffer、同一 undo 栈
防冲突门：write_file/batch_replace 对 docx 行模型态显式拒改
  （rpc.rs:1518「行模型与磁盘字节不同构」、rpc.rs:1966）
```

**AI 不直接操作 OOXML，不经 LOK**——它驱动与人手相同的 Buffer 编辑面。

### 2.5 DOCX 导出 PDF（唯一 LibreOffice 消费点：外部 soffice 子进程）

```
菜单「导出为 PDF」(menu_bar.rs:125) → Act::ExportPdf (shell.rs:1572)
  → export_active_pdf                     shell.rs:2215
  ① docx 标签 + 干净态检查 :2219
  ② office_io::convert::detect_libreoffice  convert.rs:131（PATH→平台候选，--version 探活）
  ③ 后台线程：office_io::convert::docx_to_pdf  convert.rs:225
     → soffice --headless --norestore -env:UserInstallation=<临时profile>
       --convert-to pdf --outdir <tmp> <input>   convert.rs:263 附近
  ④ 产物读回 → 写盘
```

**此链与 LibreOfficeKit 无关**（LOK = dlopen libmergedlo 进程内方案，此链 = 独立 soffice 进程）。另有 pdf_export_docx.rs 是**反向** PDF→DOCX（pdfium 提取 + office-io pdf_import_to_docx）。

## 3. web/wasm 链（与桌面链平行的第二套模型）

```
导入：import_async.ts:97 spawn worker → import-worker.ts:68 import_docx(u8)
  → mdcore-wasm/src/docx_import.rs:987 → office-io read_docx_from_bytes_with_options
  → docx_to_markdown (:801) —— 产出 Markdown 字符串（非块树！）
  → main.ts:2033 __inject → WasmCore::new(md) → mdcore::Core::from_markdown
编辑：CM6 → apply_ops(Source=View|Ai) → mdcore::Core::apply_ops → BlockTree
导出：export_docx_styled → docx_export.rs:1595 emit_block → office-io DocxWriter(docx.rs:6532, docx-rs)
```

**关键事实**：web 链 DOCX 已降级为 Markdown 块模型——图片跳过、合并单元格/编号细节/域缓存丢弃（stats 显式计数 `images_skipped`/`merged_ranges`/`fields_dropped`）；导出是**重建式**（docx-rs 全新生成）而非桌面链的**原位保真式**。

## 4. LOK（crates/lok）现状：原型已验证、未接线

- 本机实测（本次调查复跑）：`cargo test -p mdc-lok` 单元 15 过 + `lok_smoke` 全链 **1 passed 19.09s**（26 步：发现→版本→hash→load→渲染→编辑→saveAs→重开校验→双 worker 并行→超时看门狗），LibreOffice 25.2.3.2 + libmergedlo.so。
- gates/633.md 诚实声明：Windows/macOS 实机门未执行；destroy 跳过（LO 25.2 teardown SalAbort bug）依赖进程退出回收。
- **消费者为零**：这是「A 路线」的唯一直接证据，但它是刚落地的原型，不是既成架构。

## 5. 模块实际职责总表（替换任务书基线表）

| 模块 | 实测职责 | 状态 |
|---|---|---|
| GPUI（mang-shell） | 桌面界面、窗口、交互、**自绘编辑器（含 DOCX 行编辑）** | 已实现（#616 输入/IME/标签闭环进行中） |
| LibreOfficeKit | **无职责**（零消费者原型） | 原型（#633 已合并，Linux 实测过） |
| soffice 子进程 | DOCX→PDF 一次性导出 | 已实现（convert.rs） |
| Rust OOXML（office-io） | DOCX 全生命周期：读/原位编辑/写 + XLSX + .doc 识别 | 已实现（54 测试文件/320 测试函数） |
| Document Model（mdcore） | Markdown 块模型（web 链专属） | 已实现 |
| AI/MCP | RPC 命令驱动桌面 Buffer | 已实现（11 工具） |
| pdfium+lopdf（非 MuPDF） | PDF 读/渲染/标注/增量写/OCR 接线 | 已实现（229 测试过） |
| Overlay | 格式无关标注投影 | 已实现 |
| Evidence | 外挂指纹/sidecar | 已实现 |
| Canvas | 独立画布域模型 | 已实现 |
| Collaboration | yrs 协作 + 中继 | 已实现（web 链侧） |

## 6. 调用时序图（桌面 DOCX 全生命周期真实链）

```
用户            mang-shell(GPUI)         office-io(Rust)           LibreOffice
 │ open ────────→ open_document_tab ─────→ read(docx-rs)+scan(quick-xml)
 │                                        ← 行模型+DocxEditor句柄
 │ type ────────→ Buffer(rope) ──────────→ (延迟) set_paragraph_text
 │               ← 行归属重排(hook)          [XML 外科手术，未改段字节不动]
 │ save ────────→ flush → save_to_vec ───→ zip 重打包
 │               ← .docx.saving+rename 原子写盘
 │ export-pdf ──→ detect + spawn ────────────────────────────────→ soffice --headless
 │               ←  ←──────────────────────────────────────────── ← PDF 文件
 │ AI(MCP) ⇄ RPC socket ⇄ 同一 Buffer（同栈同 undo，防盘直改门）
```

LOK worker（未接线）画外：`mang-shell ─(未来)→ mdc-lok client → worker 进程 → libmergedlo`。

## 7. 结论

1. 当前架构既非任务书 A（LOK 主编辑）也非纯 B（自有模型+LO 一次性排版），而是 **「B-桌面变体（行模型+XML 原位）+ B-web 变体（Markdown 降级）+ soffice 一次性导出 PDF」**，LOK 是为替代人工排版而刚孵化的原型。
2. #633 的立项动机（gates/633.md:3-4：用户裁定「停止给 DOCX 行编辑器补页面排版，GPUI 统一交互、各格式选现成内核」）说明 A 路线是**既定方向**，但代码尚未走到。
3. 双前端双模型并存是当前最大结构事实，任何「统一状态」设计必须先面对它（详见 document-state-ownership.md）。


---

## 后记（2026-11-12 入库轮）

本节以下正文为 `83d63d8` 基线调查原话。入库时两点已变，详见
`2026-11-12-update.md`：① LOK 零消费者 → mang-shell 已接线（`#50 C1/C2`：
`editor/docx_lok_session.rs` 833 行会话层 + `docx_pages.rs` 条带渲染 + 壳层
`ensure_docx_pages` 打开入口；本机 grep 实测）；② 本 PR 将旧壳 `crates/shell-native`
剪裁移除（workspace 19→18，Cargo.lock 连带移除 GTK/tao/wry 树 45 包）。
