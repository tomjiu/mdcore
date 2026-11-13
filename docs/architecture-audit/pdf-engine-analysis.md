# pdf-engine-analysis.md — PDF 引擎与 PDF 2.0 评估

> **基线声明**：本文调查基线 = main `83d63d8`（2026-11-09，含 #633 LOK worker 合并）。
> 入库时远端 main 已前进至 `3d8d203`（2026-11-12）：LOK 已在 mang-shell 接线（#50 C1/C2）、
> 旧壳 crates/shell-native 已剪裁移除（本 PR）。正文表述保留基线时点原话，
> 逐项更新见 `docs/architecture-audit/2026-11-12-update.md`。

> 调查日期：2026-11-12 · 基线 main `83d63d8` · 只读调查
> **基线纠错：mdcore 无 MuPDF。** PDF 引擎 = pdfium（pdfium-render 0.9.4 pin，chromium/8086 二进制）读/渲染 + lopdf 0.46 增量写。MuPDF 仅出现在代码注释的交叉验证踩坑记录（stamp.rs:651「pymupdf 交叉验证抓到 /SMask 内联字典不认」）。

## 1. mdc-pdf 实测能力面（crates/pdf，34 个 src 文件）

| 能力 | 状态 | 证据 |
|---|---|---|
| PDF 打开/解析 | ✅ 已实现 | engine_pdfium.rs；容器三门 + 安全面（security.rs xref 8388607 上限等 ISO 32000 附录 C） |
| 页面渲染 | ✅ | render.rs + LRU 页缓存 + 渲染工人（worker.rs）；mang-shell pdf_pages.rs/pdf_thumbs.rs 消费 |
| 文本提取/搜索 | ✅ | pdfium 文本层；table.rs 表格结构化提取（gates/232 ✅） |
| 选择复制/页面操作 | ✅ | organize.rs（#304 页面变换）、桌面选择 |
| 注释/标记 | ✅ | annotate.rs/annotation.rs/stamp.rs（lopdf 增量写，DocMDP ISO 32000-2 §12.8.2.2 已建模 docmdp.rs） |
| AcroForm 表单 | ✅ 真增量修订 | forms.rs（gates/227 ✅） |
| PDF 保存/重写 | ✅ | writer_lopdf.rs 增量写；rebuild 面在 office-io |
| 匿名化/安全扫描 | ✅ | anonymize.rs / security_scan.rs / dark_scan |
| OCR | ✅ 接线 | ocr.rs + ocr_word.rs + ocr_writeback.rs（tesseract 外置，#306） |
| 打印 | ✅ | print.rs（#240 管道） |
| 签名验证 | ✅ | signature.rs |
| PDF 2.0 读 | ⚠️ 部分 | pdfium FPDF_GetFileVersion 返回 2.0；但 pdfium 固定版本（chromium/8086=LO 25.2 同款 pdfium-6764 前身线）**早于 2026-10-27 的 PDF 2.0 保存头修复**（chromium issue 576363144：保存后 %PDF-2.0 被写成 %PDF-1.0）；/BrotliDecode 实验特性需 PDF_ENABLE_BROTLI |

## 2. PDF 2.0 三层区分（任务书要求）

| 层 | 事实 |
|---|---|
| PDF 2.0 标准（ISO 32000-2） | AES-256 加密强制、新标注类型、透明度组改进、关联数组等 |
| LibreOffice 实测 | **25.2 不支持 PDF 2.0/PDF/A-4 导出**（25.2 源码 pdfexport.cxx 有未接线 TODO 分支；加密只写 RC4-128）；**25.8 正式支持**（ReleaseNotes/25.8：PDF 2.0 导出 + AES-256 + PDF/A-4，Tomaž Vajngerl）。mdcore 当前 soffice 导出链用系统 LO（本机 25.2.3.2→只能出 PDF 1.7/PDF/A-1b/2b/3b） |
| pdfium 实测 | 读 2.0 OK（GetFileVersion=20）；写 2.0 有已修复但**固定版本未含**的头版本 bug；维护者曾表态「只支持到 1.7 (32000-1)」（issue 42270738，2021） |
| mdcore 已实现 | lopdf 增量写面（标注/表单）不主动写版本头——跟随原文档；无任何 2.0 专属特性面 |

## 3. PDF 2.0 输出方案评估（本轮不引入新 SDK，只评估）

| 方案 | 可行性 | 证据与代价 |
|---|---|---|
| 升级捆绑 LO ≥25.8（导出仍走 soffice/LOK） | **推荐首选** | 官方原生 PDF 2.0 + PDF/A-4 + AES-256 全支持；代价 = LO 基线从 25.2 提到 25.8+（#633 ABI pin 0d51fdc 需复核 25.8 的 Doc#18/Kit#12 布局——LO 26.x 曾在 Doc#19 前插成员，ElyOffice slot 注释实证） |
| pdfium 写 2.0 | 不推荐 | pdfium 定位是读/渲染引擎；写面 bug 2026-10-27 才修且 pin 版本未含；密码学面（AESV3/R6）非其主业 |
| lopdf 直写 2.0 头 | 形式可行、实质不足 | 头改 %PDF-2.0 容易，但 2.0 合规（AES-256、新结构）需自实现——mdcore 增量写面已建模 DocMDP（32000-2 引用），说明团队按 2.0 语义演进的意识在 |
| 引入独立 PDF 生成引擎（如 typst/lopdf 全量自建） | 无必要 | mdcore 的 PDF **生成**需求全部由 DOCX 排版导出承接（那是 LO 的主业）；自有面只做**增量修改**（标注/表单/OCR 回写），lopdf 已够 |

**结论**：PDF 2.0 输出应由 LibreOffice ≥25.8 承担（soffice 或 LOK saveAs 均可触发 PDFWriter）；mdcore 自有 lopdf 面继续只做增量修订、跟随原版本号。**不必引入新 PDF SDK**。若 A 路线接线 LOK，升级 LO 基线与 PDF 2.0 支持是同一件事，一次解决。

## 4. MuPDF 问题回答

MuPDF 不在 mdcore（非「继续是否合理」而是「从未使用」）。当前 pdfium+lopdf 双引擎分工（读渲染 vs 增量写）经 229 测试（gates/210 Linux 实证）+ Acrobat 1:1 对齐验证（gates/229 ✅）验证，无更换理由。ElyOffice 用 MuPDF 做 ElyPDF 是其产品选择（MIT 项目自带 mupdf 1.28.5 pin + 补丁），与 mdcore 无关。可选关注点：pdfium 二进制 pin（chromium/8086）升级时应带上 2026-10-27 修复。

## 5. 引用（实际打开）

- https://wiki.documentfoundation.org/ReleaseNotes/25.2（无 PDF 2.0 字样，api.php 全文核对）
- https://wiki.documentfoundation.org/ReleaseNotes/25.8（PDF 2.0/AES-256/PDF/A-4 章节）
- LibreOffice 25.2 源码 filter/source/pdf/{pdfexport.cxx,impdialog.cxx}、vcl/source/gdi/pdfwriter_impl.cxx（raw.githubusercontent libreoffice-25.2.0.3 tag）
- https://issues.chromium.org/issues/576363144（pdfium PDF 2.0 保存头 bug + 修复状态）
- https://pdfium-review.googlesource.com/c/pdfium/+/161811（修复 CL，MERGED 2026-10-27）
- https://issues.chromium.org/issues/42270738（pdfium 32000-1 支持边界表态）


---

## 后记（2026-11-12 入库轮）

本节以下正文为 `83d63d8` 基线调查原话。入库时两点已变，详见
`2026-11-12-update.md`：① LOK 零消费者 → mang-shell 已接线（`#50 C1/C2`：
`editor/docx_lok_session.rs` 833 行会话层 + `docx_pages.rs` 条带渲染 + 壳层
`ensure_docx_pages` 打开入口；本机 grep 实测）；② 本 PR 将旧壳 `crates/shell-native`
剪裁移除（workspace 19→18，Cargo.lock 连带移除 GTK/tao/wry 树 45 包）。
