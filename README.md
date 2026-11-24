# mdcore

模块化 Markdown 编辑核心 + web 桥（CodeMirror 6）+ 原生桌面壳 **mang-shell（GPUI 自绘，#228 换代）** + 三域文档引擎（md / mdc-pdf / mdc-office / mdc-lok / mdc-canvas）。
**私有产品仓。**

> 2026-11 架构剪裁：原 tao+wry+GTK3 宿主壳（crates/shell-native）已移除——mang-shell 为唯一产品壳后其零消费者（#331 在档判 GTK 插件 ~20MB 死重）；历史形态见 git 历史与 gates/。

## 平台支持矩阵（D-1 · 第一屏在档）

| 平台 | 引擎核心（Rust crates） | wasm（mdcore-wasm + web） | 桌面壳（mang-shell，GPUI 自绘） |
|---|---|---|---|
| **Linux** | ✅ 全量测试基线（开发主环境，`cargo test --workspace --tests` **2186 用例 / 216 套件**，2026-11-24） | ✅ 构建 + 19 web e2e 套件全绿 | ✅ 构建可运行 + 烟雾验证链（`--smoke-sec N` Xvfb 无头通道；xkbcommon/x11 链接系库经 `scripts/rust-env.sh` 免 root 自愈） |
| **Windows** | ✅ CI `cargo check` 全绿（`.github/workflows/windows-check.yml`，`#[cfg(not(unix))]` 分支编译覆盖） | ✅ wasm32 target 同 CI 在检 | ⚠️ 交叉检查链在档（`--with-mingw`），LOK 面加载/消息循环代码齐备但**实机实测门未执行**（mdc-lok 头注如实注明）；安装包/冒烟 = #29 打包项（A5） |
| **macOS** | ❓ 无 CI 无实机构建（纯 Rust 预期可编，未验证） | ❓ 同左（wasm 产物平台无关，构建链未验） | ❓ GPUI 名义支持，**从未构建/运行** |

> 状态口径：✅ = 本仓流程内实证；⚠️ = 部分实证（编译级）；❓ = 未验证（非「不可用」——是「没做过」）。矩阵随验证轮更新（打包轮 A5 后 Windows/macOS 面预期翻转）。

## 架构（R10/R22 蓝图，ADR-H1 定谳）

```text
┌─ 桌面壳 mang-shell（GPUI 自绘独立 workspace，#228 换代）────────────┐
│   编辑器（buffer/display_map/multi_cursor/completion）/ 大纲面板 /    │
│   文件树 / 命令面板 / 主题系统（31 槽 + 自定义）/ 键位轨 / 状态栏；    │
│   md 可编辑 + docx/pdf 只读渲染（LOK 桥接 docx_lok_session）；        │
│   rpc.rs 本机控制通道（JSON-lines stdio）；嵌套 workspace 含 mdc-dock │
├─ LOK 隔离引擎 mdc-lok（#633，2026-11-09）─────────────────────────┤
│   LibreOfficeKit dlopen 动态加载 + 单文档 worker 进程（JSON-lines 协议│
│   / 帧长上限/超时看门狗）+ 会话独立 user profile + 真实运行时 smoke；  │
│   Windows 面消息驱动主循环代码齐备（实机门未执行，如实注明）          │
├─ web 桥（web/，CodeMirror 6 + wasm-bindgen）─────────────────────┤
│   window.__bridge 契约面；UTF-16↔码点单一换算对（ADR-V2）             │
│   （CM6 只服务 MD 面；PDF 面为独立轻量视图，不经 CM6）              │
├─ PDF 核 mdc-pdf（Rust，第二 Document Model，PDF-b/c/d/e 完成）    │
│   页/大纲/标注三层模型 + Command 边界 + 安全策略（S1-S6） +         │
│   pdfium 引擎/LRU 页缓存/渲染工人（engine-pdfium）+ lopdf 标注     │
│   增量写 + open-scan 安全评估（engine-lopdf）                      │
├─ Office 域 mdc-office + mdc-office-io────────────────────────────┤
│   消毒六面（CSV 公式/XML 字符/XML 实体/HTML/域白名单/URL scheme）  │
│   + XLSX 读路径（容器双门 → calamine 四面）+ .doc 旧格式识别(#234)  │
│   三域纪律：对 mdcore 零编译依赖；DOCX 面经 mdc-lok 桥接            │
├─ 编辑核 mdcore（Rust）+ mdc-canvas/overlay/collab/evidence 等──────┤
│   ropey 树 + op log + undo（ADR-B2/B4/B5）+ 块身份（ADR-B1）        │
│   revision/anchor（R13/R17 语义锁）+ 渐进打开 + Human+AI 共用状态    │
└──────────────────────────────────────────────────────────────────┘
```

宿主壳 ↔ web 桥 ↔ 核的接口以 **`CONTRACT.md` v1.6** 为准（R23-d 冻结 v1；
破坏性变更走升版纪律：版本常量 + 文档 + 双侧锁测试三处同步）。

## 分支说明

- **main**：产品代码主体（crates/ 引擎群 + web/ + mang-shell/ + CONTRACT.md + compliance/）。
  2026-11 起主线为 mang-shell（GPUI 自绘壳）+ 编辑器增强批（#47/#108/#110/#111 工作线）；
  原 tao+wry 壳已剪裁（见上）。
- **compliance-baseline**：合规基线套件——`COMPLIANCE.md`（姿态/审计终态/常驻
  门禁/遗留登记）+ `compliance_release/`（THIRD-PARTY-NOTICES / ASSET_CHECK /
  TRADEMARK_RULES）。
- **mother/r23-snapshot**：R23 阶段核侧增量参照（单提交快照，保留不动）。

## 构建与门禁

```bash
bash scripts/rust-env.sh             # 环境自愈（推荐先跑：工具链 + xkb/x11 + pdfium，免 root）
cargo test --workspace --tests        # 语义锁全绿 2186 用例 / 216 套件（2026-11-24 实测；
                                      # 演进：361 → … → 2186[+mdc-lok 运行时 smoke / mang-shell
                                      # bin 904 GUI 测试族 / office-io 与 pdf 语料 harness]；
                                      # 默认零引擎依赖，裸环境全绿）
cd mang-shell && cargo test           # 桌面壳独立 workspace 门禁（纯函数提取真跑 + GUI 无头面）
cargo test -p mdc-pdf --features engine-pdfium    # 需原生 PDFium（Windows: pdfium.dll），安装见 docs/INSTALL.md
cargo test -p mdc-pdf --features engine-pdfium,engine-lopdf    # +17 PDF-d 标注写锁（lopdf 增量写；需 libpdfium.so + spike fixtures）
cargo test -p mdc-pdf --features engine-pdfium --test pdfc_perf -- --ignored --nocapture    # 性能门实测打印（阈值 = 实测×3）
cargo check --target wasm32-unknown-unknown
cd web && npm ci && npm run build   # wasm 构建 + CM6 桥
./compliance/scan.sh                # 产品面合规门禁（每次提交前，PASS 才可合入）
```

R23 新增测试面：`web/tests/ime_cdp.py`（IME CDP 28 checks，需
playwright + chromium）、`web/tests/contract_gate.py`（契约门 16 checks：
v1 面 13 + v1.1 设置面 3）、`crates/mdcore/tests/r23_fingerprint.rs`（15 条）/
`r23_external.rs`（14 条）。R24-a 新增：`crates/shell/tests/r24_settings.rs`
（设置面语义锁 14 条）。2026-11 新增大面：mang-shell bin 目标 904 GUI 测试族
（Xvfb 无头通道）+ mdc-lok 运行时 smoke（crates/lok/tests/）+ PDF 语料
display_corpus harness（mang-shell/tests/fixtures/display_corpus/）。

## 收口纪律（自首推起生效）

每轮收口双绿：`bash compliance/scan.sh`（strict 0 / ns 0）+ `cargo test --workspace --tests`
（216 套件全绿，2026-11-24 口径）；依赖变更必须再生成 THIRD-PARTY-NOTICES 并重跑 SBOM。

## 状态快照（2026-11-24）

- **桌面壳（mang-shell，GPUI 自绘，#228 换代）**：编辑器增强批全线推进——
  #47 工作线（A 系：大纲换绑/保存协调/工作区打开/高 DPI/git 分支文案等 20 批）；
  #108 大纲线（分区折叠 + 折叠持久化 + 过滤框 + 光标跟随 + 转到符号 Ctrl+Shift+O）；
  #110 主题线（G1 31 槽具名主题四态轴 + G4 取色器 + replace_saved 原子重存 + 像素断言链）；
  #111 编辑线（G1 编码写回 GBK/UTF-16/BOM round-trip、G2 括号/引号自动闭合、
  G3 词补全：文档词表 + 关键词池候选源 + 弹窗键拦截接受链）；#107 文件树线
  （P1 mtime 轮询重载 + 拖拽移动 + P2 复制路径）；A4 浮层点击穿透修复（#255）。
  bin 目标 904 GUI 测试族全绿（Xvfb 无头通道）。
- **mdc-lok（LibreOfficeKit 隔离 worker，#633）**：dlopen 动态加载 + 单文档 worker
  + 会话独立 profile + 真实运行时 smoke 全落地（2026-11-09）；#49 B3 多会话
  生命周期五面测试在档；DOCX 面经 docx_lok_session 桥接（只读渲染批 3/4）。
- **PDF 线**：PDF-a..e 全收口（双引擎 pdfium/lopdf + 安全评估 open-scan S2 +
  watchdog S3 + 表格结构化提取 #232 + stamp 写面安全门 #277）；D1 语料归因
  修复（Liberation Sans 嵌入语料，跨端静态门收口 #257）。
- **测试基线**：`cargo test --workspace --tests` **2186 用例 / 216 套件全绿**
  （2026-11-24，main `d31d59c` 实测）。
- **仓库卫生（#260）**：926MB core dump 误提交已从 main 树移除 + .gitignore 防再入。
- 遗留：#178 DOCX 真机缺陷总览（P0）、#125 LOK 根因锁定（P0，stock LO 回调
  刷新缺陷需打补丁重编）、#87 新站 CI 无 runner（门禁靠本地代跑 + PR 评论证据惯例）；
  clippy 存量 15 条（mdc-pdf 12 + lok tests 3）待卫生批；真机 IME 人工验收、
  定名后商标检索（R24-c 前置）仍在册。
- 研究与审计底稿存于私有分析仓（允许域）；本仓仅存产品代码与合规结论。
- 门禁交付记录按 issue 拆档于 `gates/NNN.md`（索引见 [gates.md](gates.md)）。
