# libreoffice-trim-analysis.md — LibreOffice 裁剪与运行时分析

> **基线声明**：本文调查基线 = main `83d63d8`（2026-11-09，含 #633 LOK worker 合并）。
> 入库时远端 main 已前进至 `3d8d203`（2026-11-12）：LOK 已在 mang-shell 接线（#50 C1/C2）、
> 旧壳 crates/shell-native 已剪裁移除（本 PR）。正文表述保留基线时点原话，
> 逐项更新见 `docs/architecture-audit/2026-11-12-update.md`。

> 调查日期：2026-11-12 · 只读调查。上游资料经 web 实际打开核实（URL 附文中）。
> 注：任务书引用的「981.3MB 完整 / 377.9MB 删 DLL / 86.1MiB 7z」等实验数据，本轮**未在仓库/沙箱中找到对应产物或脚本**（grep 无 DSHKit 裁剪实验目录），按调查纪律标记为「未复核、待重做」。以下为可实证的部分。
>
> **2026-11-12 重做已完成（后续剪裁轮）**：以沙箱实测替代该缺口（Linux so 面口径；Windows DLL 面仍未复核）——294MB 全量树按 mdcore 真实 LOK 消费面逐项门裁至 **174MB 未压 / 57.6MB tar.gz / 41.2MB tar.xz**，两门（lok_smoke 全链 26 步 + export_pdf 全文/页范围/单页）始终全绿；非内核必经库唯一 = libloglo。明细与复跑指引：`test-artifacts/lo-trim/REPORT.md`（脚本 `scripts/lok/lo_trim_*.sh`）。

## 1. mdcore 当前 LibreOffice 消费面（代码实测）

**整个项目对 LibreOffice 的真实依赖只有两个点：**

| 消费点 | 方式 | 位置 | 可裁剪性 |
|---|---|---|---|
| DOCX→PDF 导出 | `soffice --headless --norestore -env:UserInstallation=<tmp> --convert-to pdf` 外部子进程 | crates/office-io/src/convert.rs:225-263 | 每次调用独立临时 profile；**用户自装 LibreOffice，不随 mdcore 发行** |
| LOK worker 原型 | dlopen libmergedlo.so + 独立 worker 进程 | crates/lok（#633，零消费者） | 探测序：MDC_LOK_HOME → exe 旁 LOK/ → 仓 bin/ → 平台常见位 |

**关键结论：mdcore 当前不捆绑、不分发任何 LibreOffice 运行时**——裁剪问题实际是「未来若捆绑（A 路线接线时），参照哪条路线、裁到什么程度」的预研。本机实测参照：Debian libreoffice-core 25.2.3.2 = /usr/lib/libreoffice 共 294MB（program 211MB，122 个 .so）。

## 2. DSHKit（deepseek-ai/dsh-libreoffice-kit）方案逐项核实

仓库 https://github.com/deepseek-ai/dsh-libreoffice-kit（MPL-2.0，npm @deepseek-ai/libreoffice-kit）。核实引擎 native configure.mjs / docs/packaging.md / slim-native.mjs 原文：

| 方案 | 构建条件（实测核实） | 对 LOK 影响 | 对 Writer 影响 | 对 PDF 导出影响 | 体积收益（其自报） |
|---|---|---|---|---|---|
| `--disable-dynamic-loading` | configure.mjs:44 属实；LO 官方 configure.ac 约 1706 行仍在（「work in progress」），**硬约束：必须 --disable-gui + 单一 VCL 插件**；非苹果/Win 平台连带关 database-connectivity/nss/odk/python/skia/scripting | LOK 可工作（cppuhelper shlib.cxx DISABLE_DYNLOADING 分支查 lo_get_factory_map；官方 iOS/Android/Emscripten 即此形态） | Writer 全功能保留于合并库 | 无（导出走 vcl PDFWriter，不经 pdfium） | 合并静态库的先决条件 |
| `--enable-customtarget-components`（静态 UNO 映射） | 属实；仅 DISABLE_DYNLOADING=TRUE 时生效，native-code.py 生成 component_maps.cxx | 必需（否则组件无法注册） | — | — | 结构性 |
| LTO / ThinLTO | `--enable-lto` + 发布面 -Oz+ThinLTO 属实 | 启动略慢（链接期优化无运行时收益逆说）；无功能性影响 | — | — | macOS 98.85→60.49MB、WASM 56.47→35.85MB（其 README 实测） |
| `--optimization=Oz` / clang-cl | 属实（clang-cl 建 Skia；WebP 例外留 MSVC） | 无 | — | — | 计入上列数字 |
| PDFium 集成 | `--enable-pdfium` + 补丁 0023/0030 属实 | 无 | OOXML 内嵌 PDF/EMF 图形栅格化保留 | **无冲突**：LO 的 PDF 导出走自研 PDFWriter，pdfium 只管导入渲染 |
| 删 PDF Import（--disable-pdfimport + slim 删 xpdfimport） | 属实 | 无 | 失去 PDF→Draw 编辑（mdcore 有自家 pdfium 链，不需要） | 无 | 计入 |
| 删字体/模板/字典（--without-fonts/--without-myspell-dicts/--with-templates=no 等） | 属实 | 无 | **中文字体完全外置**——mdcore 中文排版场景必须系统字体兜底 | 排版结果依赖系统字体集 | 计入 |
| 扩展/数据库/脚本裁剪（--disable-extensions/--disable-database-connectivity/--disable-scripting 等） | 属实（slim 再删 basic/Scripts/access2base/scriptforge） | 无 | 失去 Basic 宏 | 无 | 计入 |
| services.rdb 静态检查 | 属实：slim-native.mjs 删库前断言 RDB 不引用，违规即 staging 失败 | 保证组件面完整 | — | — | 质量门 |
| SHA-256 验证 | 属实：外层 envelope+内层 tar 双校验、manifest 逐文件 hash | — | — | — | 分发完整性 |
| 「自动回滚删除失败项」 | **不属实**：rmSync 失败即抛错中止，恢复=重新 staging（restore-artifacts.mjs 是 CI 工具非运行时回滚） | — | — | — | — |

**DSHKit 结论**：全静态 LOK worker 路线（Linux/macOS/Windows/WASM 四平台落地、70 个 native 补丁支撑）被生产验证可行；其体积上限 ~60MB 级（macOS ARM64）。对 mdcore 的适用性前提：接受 --disable-gui（LOK 本就 headless，无冲突）+ 自备 lo_get_*_map 映射 + 承担 70+ 平台静态链接补丁的维护面。**「自动回滚」声称不实，需自建**。

## 3. ElyOffice（ELY-Open-Source/ElyOffice）方案核实

仓库 https://github.com/ely-open-source/elyoffice（MIT）。mdcore 的 #633 已参照其 commit dc32744d（gates/633.md:107 注明「研究参照，无代码拷贝」）。

**技术路线与 DSHKit 相反**：动态加载 + `--enable-mergelibs` + **三应用共享单份 kernel**（release.yml：deb/rpm/Arch 包内一份 LibreOffice kernel 供 ElyDoc/ElyTable/ElyShow）。

- LO 版本：patched 26.2.6.3；固定提交 `8221e31b3ac356a1623c672912a3d2b492f7e3d1`（fetch-references.sh，sparse checkout 只取 LibreOfficeKit 相关目录）。
- configure：--enable-release-build --enable-mergelibs --without-java --disable-python + Linux 无头 LOKit 加 --disable-gui --disable-dbus --disable-gio --disable-dconf --disable-randr；禁 odk/online-update/extension-integration/beanshell/javascript/firebird/postgresql-sdbc/report-builder/sdremote/poppler/cups/gpgmepp。
- 裁剪：只留 dict-en（「每个字典都拖慢 LOK 启动 PERF-01」）、删 Lightproof、植入 ely.xcd 内核默认值；patchelf RUNPATH=$ORIGIN。
- **补丁面：patches/libreoffice/ 0001–0106（UPSTREAM.md 引用至 0126）几乎全是 LOK 缺陷修复**（macOS tile painting、dispatch-to-document-frame、win32 paints-then-copies、broken-package-refused 等）——这正是 mdcore 未来接线 LOK 会踩的坑位地图。
- 构建环境：Linux 用 AlmaLinux 8 Docker（glibc 2.28 锚）；Windows VS2022+clang-cl+Cygwin；macOS Xcode+Homebrew。

**对任务书 5 个问题的回答**：
1. mdcore 是否已完整吸收 ElyOffice 有效方案？——**没有**。已吸收的只有 ABI 绑定纪律（slot 注释防 vtable 中段插入、#633 的 nSize 守卫）与运行时发现序；构建/裁剪/补丁体系一样都没建。
2. 还缺什么：LO 构建线（当前依赖用户系统装）、mergelibs 或静态链接决策、kernel 部署位（exe 旁 LOK/ 已预留）、字典/字体策略。
3. 哪些补丁只是其自身需要：gpui/mupdf 子目录补丁、ely.xcd 内核默认值（Excel A1 formulas）、FSEvents/公证类。
4. 对 mdcore DOCX 编辑+PDF 导出有价值的：LOK callback/tile/焦点/包损坏拒绝类补丁（0001/0002/0043/0044/0057/0064 序号段）——若 A 路线接线直接踩这些坑。
5. 维护成本：125+ 补丁对上游 26.2 逐 rebase；ElyOffice 自身 UPSTREAM.md 承认「None has been sent yet (2026-11-10)」——全部私有补丁、每版本人工重放。

## 4. mdcore 裁剪路线建议（预研结论，不实施）

| 决策点 | 推荐 | 理由 |
|---|---|---|
| 动态 vs 静态 | **先动态（ElyOffice 式 mergelibs+裁剪），静态为远期** | mdcore LOK 仅做编辑+排版+导出（无 Online 预加载依赖）；静态路线要背 70+ 补丁；DSHKit macOS 60MB 是上限参照 |
| PDF Import | 可删 | mdcore PDF 读走自家 pdfium 链（crates/pdf），不需要 LO 的 PDF Import |
| 字典 | 只留 dict-en 或全删（拼写非核心） | ElyOffice PERF-01 实证字典拖慢 LOK 启动；mdcore 无拼写需求 |
| 字体 | **不捆，系统字体** | 中文场景捆字体=数百 MB；当前 soffice 导出链已验证系统字体路径可行 |
| Java/Python/扩展/报表/数据库 | 全删 | 与 DSHKit/ElyOffice 共识 |
| 验证流程 | 采纳 DSHKit 门禁思想：删库前 services.rdb 断言 + 转换真实 DOCX→PDF + 页面几何/文字位置比对 + SHA-256 | mdcore 已有 convert 测试基线（office-io/tests/convert_docx_pdf.rs）可扩展 |

**体积空间总判断**（供下一阶段预算，非本轮实测）：捆绑动态裁剪路线 ~250-350MB 未压缩 / 90-120MB 压缩量级（ElyOffice deb 一份 kernel 为参照）；全静态极限 ~60MB（DSHKit macOS 实测）但补丁维护成本最高。**未捆绑现状下 mdcore 发行体积与 LibreOffice 无关**（deb Installed-Size 321MB 主要是 GPUI 应用本体+pdfium，见 packaging/ACCEPTANCE-270-LINUX.md）。

## 5. 本轮明确不做/未能做的

- ~~任务书引用的「已删 122 DLL→377.9MB」实验：仓库内无产物无脚本，无法复核「未加载」声明（延迟加载/未触发功能风险无法排除）——标记为**待重做**，重做时按上表验证流程执行。~~ → **2026-11-12 重做完成**（Linux so 面）：逐项门裁决替代「未加载」声明——每删一项即跑 lok_smoke 全链 + export_pdf 两门，红即回滚，不存在延迟加载盲区；结果 294→174MB / 压缩 57.6MB(tar.gz)，见 `test-artifacts/lo-trim/REPORT.md`。Windows DLL 面仍未复核（无 Windows 树）。
- ~~未做 7z/ZIP 压缩可安装性实测（无裁剪产物可测）。~~ → 重做轮已产压缩口径（tar.gz 57.6MB / tar.xz 41.2MB，sha256 在 REPORT 头）；「可安装性」（进 deb/AppImage 分发）仍属 A 路线决策件未做。
- 未改任何构建产物（调查纪律）。（重做轮为独立副本实验，产品构建链零改动——纪律保持。）

## 6. 引用（本轮实际打开）

- https://github.com/deepseek-ai/dsh-libreoffice-kit （configure.mjs / slim-native.mjs / docs/packaging.md / docs/building.md / engine/wasm-source/*）
- https://github.com/ely-open-source/elyoffice （scripts/fetch-references.sh / scripts/build-libreoffice.sh / patches/libreoffice/ / patches/UPSTREAM.md / .github/workflows/release.yml）
- https://github.com/LibreOffice/core/blob/master/configure.ac（--disable-dynamic-loading/--enable-customtarget-components/--disable-gui 语义）
- https://github.com/LibreOffice/core（include/osl/detail/component-mapping.h、cppuhelper/source/shlib.cxx、servicemanager.cxx）
- https://bugs.documentfoundation.org/show_bug.cgi?id=145127（macOS LOK 需 headless VCL）


---

## 后记（2026-11-12 入库轮）

本节以下正文为 `83d63d8` 基线调查原话。入库时两点已变，详见
`2026-11-12-update.md`：① LOK 零消费者 → mang-shell 已接线（`#50 C1/C2`：
`editor/docx_lok_session.rs` 833 行会话层 + `docx_pages.rs` 条带渲染 + 壳层
`ensure_docx_pages` 打开入口；本机 grep 实测）；② 本 PR 将旧壳 `crates/shell-native`
剪裁移除（workspace 19→18，Cargo.lock 连带移除 GTK/tao/wry 树 45 包）。
