# workspace-dependency-audit.md — Workspace 依赖审计

> **基线声明**：本文调查基线 = main `83d63d8`（2026-11-09，含 #633 LOK worker 合并）。
> 入库时远端 main 已前进至 `3d8d203`（2026-11-12）：LOK 已在 mang-shell 接线（#50 C1/C2）、
> 旧壳 crates/shell-native 已剪裁移除（本 PR）。正文表述保留基线时点原话，
> 逐项更新见 `docs/architecture-audit/2026-11-12-update.md`。

> 调查日期：2026-11-12 · 方法：cargo metadata（1.99.0 实跑）+ cargo tree + Cargo.toml 静态解析（scripts/dep_parse_static.py）+ FFI/系统库 grep
> 基线 main `83d63d8`：主 workspace 19 crate（Cargo.lock 三方 427 项）+ mang-shell 独立 workspace + third_party/zed 闭包（32 crate）

## 1. 真实 crate 依赖图（实测）

```
mdcore-wasm ─→ mdcore, workbench, office-io, canvas, evidence, collab
workbench ─→ evidence, office-io
office-io ─→ office, formula
overlay-pdf ─→ overlay, pdf, evidence
overlay ─→ evidence
canvas ─→ evidence（仅 Fingerprinter trait）, yrs
mdbridge ─→ mdcore, collab        relay ─→ collab
pdf-wasm ─→ pdf
shell ─→ mdcore, settings        shell-native ─→ mdcore, shell
lok ─→（serde_json only，零 path 依赖）
mang-shell[独立ws] ─→ office-io, pdf, canvas + zed 闭包
```

## 2. crate 清单与处置建议

| Crate | 实测职责 | 被谁依赖 | 主要问题 | 建议 |
|---|---|---|---|---|
| mdcore | MD 块核 | mdbridge/wasm/shell/native | 桌面链不消费它（双模型根源之一，非本 crate 问题） | 保留 |
| mdcore-wasm | web 桥 | 无（终端） | 汇聚 6 个域 crate，但它是合法的应用组合层 | 保留 |
| office-io | Office IO 引擎（DOCX/XLSX/PDF 导出） | workbench/wasm/mang-shell | **职责超载**：DOCX 读+原位编辑+写+XLSX+soffice 导出+dark_scan+匿名化，54 测试文件 | 拆分（远期：docx-edit 原位面独立） |
| office | sanitize 消毒 | office-io | 小而清晰 | 保留 |
| formula | ironcalc 包装 | office-io | 单一职责 | 保留 |
| lok | LOK 隔离 worker | **无人依赖** | 原型未接线（#633 设计如此） | 保留（A 路线接线即消费者出现） |
| pdf | PDF 域模型+双引擎 | overlay-pdf/pdf-wasm/mang-shell | pdfium/lopdf 均 optional 门控，纪律好 | 保留 |
| pdf-wasm | pdf 桥 | 无 | — | 保留 |
| evidence | 指纹/sidecar 基础层 | canvas/overlay/overlay-pdf/workbench/wasm | 被依赖最多——但仅作为通用指纹 trait，非文档引擎耦合 | 保留 |
| workbench | 证据管道 | wasm | — | 保留 |
| overlay / overlay-pdf | 格式无关标注 / PDF 组合 | overlay-pdf→overlay | overlay-pdf 是 overlay 与 pdf 的合法组合层（应用组合点模式） | 保留 |
| canvas | 画布域 | wasm/mang-shell | 对 evidence 仅用 Fingerprinter（session.rs:13 use trait）——**边界可接受但偏细**：指纹器属通用工具，非域渗透 | 保留 |
| collab / mdbridge / relay | 协作三件 | wasm/mdbridge/relay | mdbridge=组合点模式，relay=服务端形态 | 保留 |
| settings / shell / shell-native | 设置/宿主契约/旧壳 | shell/native | **shell-native 是死重**：打包链已判 GTK 插件 ~20MB 无用（#331 P1）；mang-shell（GPUI）已是主线 | 删除或归档（需产品确认，非本轮动作） |

## 3. 依赖边界目标核对（任务书 §七.4 逐条）

| 目标 | 现状 | 判定 |
|---|---|---|
| 核心模型不依赖格式引擎 | mdcore 零引擎依赖（ropey/serde/sha2 only）；pdf 与 mdcore 互不依赖 | ✅ 达标（三域纪律真实成立） |
| UI 不承载业务规则 | ⚠️ mang-shell editor/ 20k 行含 docx 桥、undo 栈、RPC 服务端——桌面「壳」实为厚客户端 | 部分 |
| 格式引擎不依赖 GPUI | office-io/pdf 零 GPUI；GPUI 在独立 workspace/third_party | ✅ |
| Overlay 不依赖具体引擎 | overlay 仅依赖 evidence；overlay-pdf 组合 overlay+pdf 属组合层 | ✅ |
| Evidence 不依赖文档实现 | evidence 零 path 依赖 | ✅ |
| AI 经稳定命令接口 | MCP→RPC 27 命令→Buffer；不碰引擎内部对象 | ✅ |
| 外部引擎经 Adapter/Worker | LOK=worker 进程；soffice=子进程；pdfium=optional FFI | ✅ |
| 应用层组合、底层不反依赖应用层 | mdbridge/overlay-pdf/wasm 均为组合点；无反向边 | ✅ |

**依赖渗透实测结论：反向依赖零、跨域渗透零。** 任务书担心的 9 类违规（Overlay→PDF 实现等）经逐 crate 依赖矩阵核查**无一成立**。唯二结构噪声：① shell-native 死重（历史遗留非渗透）；② mang-shell 厚壳（业务在壳内而非引擎，属「B-桌面变体」的实现形态，见 options 报告）。

## 4. FFI / 系统库 / 外部进程面

| 面 | 机制 | 隔离方式 |
|---|---|---|
| LOK | dlopen libmergedlo（abi.rs #[repr(C)] 手写 + nSize 守卫） | 宿主永不加载内核；worker 进程 JSON-lines stdio |
| pdfium | pdfium-render（libloading 动态） | optional feature；定位序 exe 同目录→bin/lib→系统（gates/210） |
| soffice | Command 子进程 | 临时 UserInstallation profile，每次独立 |
| wasm | wasm-bindgen | 浏览器/Node 宿主 |
| tao/wry/GTK3（shell-native） | 系统链接 | 未启用（编译期蓝图） |
| tesseract | 外部进程（OCR） | #306 接线 |

## 5. 平台差异

Windows：LOK 面编译零警告但实机门未执行（gates/633:92-97）；NSIS 捆 pdfium.dll 而 deb 不捆 libpdfium.so（#331 不对称，P1 已登记）。Linux：全链实测（LOK smoke 19s 过）。macOS：LOK 布局按官方头实现，未实测。


---

## 后记（2026-11-12 入库轮）

本节以下正文为 `83d63d8` 基线调查原话。入库时两点已变，详见
`2026-11-12-update.md`：① LOK 零消费者 → mang-shell 已接线（`#50 C1/C2`：
`editor/docx_lok_session.rs` 833 行会话层 + `docx_pages.rs` 条带渲染 + 壳层
`ensure_docx_pages` 打开入口；本机 grep 实测）；② 本 PR 将旧壳 `crates/shell-native`
剪裁移除（workspace 19→18，Cargo.lock 连带移除 GTK/tao/wry 树 45 包）。
