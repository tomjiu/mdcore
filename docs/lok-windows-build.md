# Windows LOK 内核：构建产物、体积构成与三个平台坑

> 关联：#69（Windows LOK 真机链）| #158 / #159 / #160（三坑）
> 代码修复：PR #161（`crates/lok` 10 文件 +344/−74）
> 本文档解决一件事：**云沙箱不要编 LibreOffice，直接下产物。**

## 1. 先读这条：云沙箱编不了 LO

实测云沙箱（`main` box，Ubuntu 24.04）：

| 资源 | 实测 | LO 构建需要 |
|---|---|---|
| 磁盘 | 20 G（当时仅用 43 M） | ≥ 40 G（源码 8 G + 对象文件 + instdir） |
| 内存 | 3.8 G（XFCE 已占 1.4 G） | ≥ 8 G（`MERGELIBS=TRUE` 下并行链接仍吃满） |
| 工具链 | 无 cargo / rustc / cmake | 全套 + autoconf + 大量 -dev 包 |
| 耗时 | — | 本机 Windows 全量一次约 10 小时（`[build CXX]` 5,010 次） |

**结论：想要 Windows 内核，下载release 产物，不要尝试重建。**
云沙箱的定位是 GUI 验收环境，不是构建机。

## 2. 产物位置

| 项 | 值 |
|---|---|
| Release tag | `lok-win-x64-26.2.6.3-patched` |
| 附件 | `instdir-lok-win-x64-patched.zip` + `README.md` |
| 压缩包 | 319,747,024 B（305 MiB） |
| SHA256 | `EABE5FBA39BC36E4B8A3D7C2DC8F3C33D0A9DB8E7CA92346BE0A0BE129C06A44` |
| 解压后 | 1,028,985,304 B（982 MiB）/ 5,705 文件 / 585 目录 |
| 内核 | `program/mergedlo.dll` 90,696,192 B，SHA256 `202E0FD3B91DA1EB31E31429FFE373781FF984A2D62E64AFC290DD6EA33C9E35` |

仓库是私有的，**匿名下载会拿到登录页 HTML**（HTTP 200 但 `Content-Type: text/html`），
带 token 才是 `206 / application/octet-stream`。

一行下载（`FJT` = 派单下发的 token）：

```bash
curl -fL --retry 3 -H "Authorization: token $FJT" \
  "https://git.mingjiu.store/sandbox/mdcore/releases/download/lok-win-x64-26.2.6.3-patched/instdir-lok-win-x64-patched.zip" \
  -o instdir-lok-win-x64-patched.zip
```

或直接用 `scripts/fetch-lok-win-build.sh`（含 SHA256 校验）。

## 3. `instdir` 是什么，为什么 982 MiB

`instdir` = **install dir**，LibreOffice `make` 的安装输出目录。`make` 不把产物散在
源码树里，而是装配成一棵**可直接运行**的树——它本身就是交付物，不需要再 `make install`。

**982 MiB 里没有"我们的软件"。** `mdc-lok-worker.exe` 只有几 MB；剩下的是
LibreOffice 整个办公套件的运行时。逐项拆开：

### share/ —— 600 MB（3,753 文件）

| 子目录 | 大小 | 文件数 | 是什么 |
|---|---|---|---|
| `extensions` | **486.9 MB** | 750 | **63 种语言的拼写词典 + 同义词库**（`dict-*`） |
| `fonts` | 53.0 MB | 135 | 打包字体（Liberation / Noto / DejaVu 等） |
| `config` | 35.2 MB | 1,923 | 各类 XML 配置与 schema |
| `gallery` | 8.3 MB | 87 | 剪贴画图库 |
| `template` | 7.7 MB | 138 | 文档模板 |
| `xslt` | 4.3 MB | 55 | 导入导出 XSLT |
| `basic` | 4.2 MB | 186 | Basic 宏库 |
| `registry` | 2.8 MB | 22 | **核心配置（必需）** |
| `filter` | 1.8 MB | 5 | **过滤器注册（必需）** |
| `liblangtag` | 1.7 MB | 14 | 语言标签数据 |

词典单项最大的几个：

```
83.1 MB  dict-de        36.9 MB  dict-tr        26.1 MB  dict-pt-BR
25.5 MB  dict-en        24.0 MB  dict-es        18.4 MB  dict-mn
```

**半壁江山是 63 国词典。** 一份中文产品用不上 `dict-mn`（蒙古语）、`dict-be`（白俄）…

### program/ —— 360 MB（1,916 文件）

| 文件/目录 | 大小 | 是什么 |
|---|---|---|
| `mergedlo.dll` | 86.5 MB | **内核本体**（MERGELIBS 合并了 vcl/sfx/base 等） |
| `icudt78.dll` | 31.6 MB | ICU 数据（Unicode 表，**不可裁**） |
| `swlo.dll` | 17.7 MB | Writer 模块 |
| `sclo.dll` | 17.1 MB | Calc 模块 |
| `python-core-3.12.14/` | 35.0 MB | Python 运行时（1,473 文件，宏/脚本过滤器用） |
| `skialo.dll` | 8.9 MB | Skia 渲染后端 |
| `sdlo.dll` | 8.1 MB | Impress/Draw 模块 |
| `mwaw.dll` | 7.3 MB | Mac 旧格式过滤器 |
| `python312.dll` | 7.1 MB | Python 解释器 |
| `libcrypto-3.dll` | 6.8 MB | OpenSSL（签名/加密 PDF 用） |
| `pdfiumlo.dll` | 6.3 MB | PDF 渲染 |

> `sofficeapp.dll` 只有 21 字节，内容是 `invalid - merged lib` —— `MERGELIBS=TRUE` 的
> 正常产物；真正的可执行是 `soffice.bin`，代码已并入 `mergedlo.dll`。

## 4. 能瘦到多少（未实测，按需裁剪）

| 裁剪项 | 省 | 风险 |
|---|---|---|
| `share/extensions` 全删 | 487 MB | 拼写检查、同义词失效（`SpellCheck` 类命令静默无效） |
| 仅留 `dict-en` + `dict-zh` + `hyph-*` | ~460 MB | 中文产品够用 |
| `program/python-core-*/` | 35 MB | 依赖 Python 的过滤器/宏失效 |
| `share/fonts` | 53 MB | Windows 用系统字体；**但排版度量会变**，PDF 导出可能位移 |
| `gallery` + `template` + `basic` + `xslt` + `palette` | ~23 MB | 图库/模板/宏不可用 |

保守可到 **~300 MB**（≈100 MB 压缩包），激进可到 **~250 MB**。

> **每一刀都必须重跑 smoke。** LO 缺资源时是**静默降级**（功能消失但不报错），
> 不会启动失败——这正是本仓最忌讳的"假绿"。
> 裁剪前请先跑 `mdc-lok-smoke` 拿基线，裁完再跑一次比对 `steps.jsonl`。

## 5. 三个 Windows 平台坑（必读，别重踩）

### 坑 1：`SAL_LOK_OPTIONS=unipoll` 在 Windows 上不可用（#158）

`lok_init_2` 看到 unipoll 就走 `else InitVCL();` 分支——**不建 kernel 主线程、
不建用户安装**。`hook_2` 仍返回非空 kit（宿主误判 ready），`documentLoad` 直接
`0xC0000005`。stock LO 26.8 同样复现 → 与 118 个补丁无关。

**#104/PR #122 的 message-loop 路线因此翻案**（其前提正是 unipoll）。Windows 上：

```
env::remove_var("SAL_LOK_OPTIONS")   // 必须清，子进程会继承
主线程走 serial_loop（串行派发）
非 unipoll 模式不调 runLoop（槽 13）
```

旧路线保留在 `MDC_LOK_WIN_MSGLOOP` 诊断开关后，可双向复现。

### 坑 2：文件 URL 必须三斜杠（#159）

`file://C:/...` 被 lo 解析成 `authority="C:"` → `hook_2` 返回 NULL + `_exit(77)`。
**stderr 是管道时**该致命路径退化成**阻塞对话框**：worker 过了 `hello` 静默挂死，
stderr 一个字都没有。

```
C:\tmp\a.docx        → file:///C:/tmp/a.docx   （三斜杠）
\\srv\share\a.docx  → file://srv/share/a.docx （UNC，authority=主机名）
\\?\C:\tmp\a.docx  → 先剥 \\?\ 再三斜杠
```

统一走 `mdc_lok::profile::file_url_of`，**不要手搓**。

### 坑 3：`postUnoCommand` 是异步的（#160）

`.uno:SelectAll` 后立即读 `getTextSelection` 拿到空/陈旧值（连续 4 次空读）。
"重试到非空"的循环会过派发、抬高 `revision`（实测 `left: 6, right: 5`）。
→ **单次派发 + 轮询**。

配套事实：`INVALIDATE_TILES` 单编辑恒 0，只有「全网搜索 + 编辑 + 撤销/重做」全链
产出 1 次 `EMPTY, 0, 0`。契约用例不要断言它。

## 6. 拿到产物后怎么验证（Windows）

```bat
call "<VS>\VC\Auxiliary\Build\vcvars64.bat" -vcvars_ver=14.44
set MDC_LOK_HOME=<解压路径>\instdir
set MDC_LOK_LIB=%MDC_LOK_HOME%\program\mergedlo.dll
cargo test -p mdc-lok
```

期望：**10 个二进制 53 个用例全绿**（lib 31 / edit_save_reliability 3 / export_pdf 1 /
fault_injection 2 / ime_text 2 / lok_smoke 1 / page_geometry 1 / protocol 6 /
session_contract 1 / session_lifecycle 5）。

真机 smoke 期望：**27 步，失败 0**，产出 `edited.docx` 6,627 B + `tile-001.bmp`。

> 探测顺序（`runtime.rs::discover_paths`）：`MDC_LOK_HOME` → `MDC_LOK_LIB` →
> 部署根 → 平台通用根。坏运行时用例要毒**两处**，只毒 `MDC_LOK_LIB` 会被
> `MDC_LOK_HOME` 抢先命中。
