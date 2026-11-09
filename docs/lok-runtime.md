# LOK 运行时发现与实机验收清单（#633）

`crates/lok`（mdc-lok）的运行时发现序、版本/哈希记录方法、各平台
实机门状态。**本文件本身不构成验收**——验收证据在 gates/633.md 与
PR 描述（实测输出原文）。

## 运行时发现序（代码：`crates/lok/src/ffi.rs`）

1. 环境变量 `MDC_LOK_DIR` —— 显式指定 LibreOffice **program 目录**
   （可逆原型首选：外部显式安装路径，不猜系统）；
2. 平台候选：
   - Linux：`/usr/lib/libreoffice/program` → `/usr/local/lib/libreoffice/program` → `/opt/libreoffice/program`
   - Windows：`%ProgramFiles%\LibreOffice\program` → `%ProgramFiles(x86)%\LibreOffice\program`
3. 目录内按钉定 `LibreOfficeKitInit.h` 的 `lok_dlopen` 规则挑库：
   `libsofficeapp.so`（>1000 字节才用，merged 构建里它是单行文本
   stub）否则 `libmergedlo.so`；Windows `sofficeapp.dll` / `mergedlo.dll`。

### 关键实证（Linux，本仓沙箱，2026-11-09）

- 发行版 **Debian 系 LibreOffice 25.2.3.2**（`soffice --version`）的
  LOK hook 导出于 **`program/libmergedlo.so`**（`nm -D` 实证
  `libreofficekit_hook` / `libreofficekit_hook_2`；`libuno_sal.so.3`
  上**没有**）；
- `libreofficekit_hook_2(install_path, user_profile_url)` 的
  `install_path` 必须传 **program 目录**（不是安装根目录）：钉定
  `desktop/source/lib/init.cxx` `lo_initialize` 以
  `BRAND_BASE_DIR = aAppURL + "/.."` 推导安装根——传根目录会找错
  `share/registry`，引导抛
  `component context fails to supply singleton ...theDefaultProvider`
  （实测两次翻车记录，勿重蹈）；
- `user_profile_url` 接受 `vnd.sun.star.pathname:<系统路径>`（init.cxx
  会转 URL；路径含空格/中文无需手工百分号编码）；
- **saveAs 返回值是布尔语义：非 0 = 成功**（`return true`→1；探针
  实证 rc=1 时产物有效落盘）。按 POSIX 0=成功 解读会把成功当失败；
- tile 像素序：本运行时 `getTileMode()=1`（BGRA）；按 tile_mode 动态
  转 RGBA，不硬编码通道序；
- **unipoll 必须开**（`SAL_LOK_OPTIONS=unipoll`，client.rs 对真实
  worker 自动设置）：非 unipoll 下键鼠/UNO 事件经 `PostUserEvent`
  入队、依赖 `lo_startmain` 线程消费——外部宿主场景实测该线程可
  整段沉睡（futex），排队事件永不被处理，**编辑静默丢失**（完整
  诊断记录见 gates/633.md「异步事件黑洞」）。unipoll 下 worker 分发
  线程进入 `runLoop` 成为主循环线程，请求在 poll 回调里同步分发。

### 版本 / 哈希记录命令（每个实机环境执行并归档输出）

```bash
soffice --version                                    # ProductVersion/BuildId
ls -la <program_dir>/libmergedlo.so                  # 库文件与大小
sha256sum <program_dir>/libmergedlo.so               # 库哈希（Linux）
nm -D <program_dir>/libmergedlo.so | grep libreofficekit   # hook 符号实证
```

Windows 等价：

```powershell
& "C:\Program Files\LibreOffice\program\soffice.exe" --version
Get-FileHash "C:\Program Files\LibreOffice\program\mergedlo.dll" -Algorithm SHA256
# 符号检查（VS Developer PowerShell）：
dumpbin /exports "C:\Program Files\LibreOffice\program\mergedlo.dll" | Select-String libreofficekit
```

## ABI 契约（钉定头）

- 头文件钉定于 LibreOffice/core **`0d51fdc57aef6707466320821b8c2931c00297c0`**
  （`include/LibreOfficeKit/{LibreOfficeKit,LibreOfficeKitInit,LibreOfficeKitEnums,LibreOfficeKitTypes}.h`）；
- vtable 前置 `size_t nSize`，调用每个成员前做
  `offset + 8 <= nSize` 整槽在界检查（严于 C 宏 `LIBREOFFICEKIT_HAS`
  的 `offsetof < nSize`）——运行时过旧时给 `runtime-too-old` 明确
  错误而不是越界调用；
- 实测本运行时：kit vtable nSize=208（< 钉定头 26 槽全量 216 →
  尾部 `registerFileSaveDialogCallback` 缺席，nSize 守卫正是为此）；
  doc vtable nSize=640；
- LO 返回的 `char*` 一律经 kit 的 `freeError` 释放（跨 CRT 边界
  禁止自己 free——钉定头明文，Windows 尤其致命）。

## Windows 实机门（本 PR 未执行——如实声明）

以下门**需要 Windows 真机 + LibreOffice 安装**才能执行，本 PR 只在
Linux 实机跑通对应链路（gates/633.md），**不得**以 CI 绿或 Linux 结果
宣称 Windows 完成：

1. 运行时发现与符号验证（上方 PowerShell 命令）；标准发行版若缺
   `libreofficekit_hook_2` 导出，把实际命令输出与错误记入本文件，
   最小替代路径：换用 ElyOffice 式补丁构建（其 `patches/libreoffice/`
   含 Windows 专项：0044 paint-then-copies、0057 focus、0060
   liblangtag-lock、0080 长路径 248——来源 MIT，模式参考）；
2. `mergedlo.dll` 依赖解析：加载用
   `LoadLibraryExW(LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS)`
   （libloading `os::windows`），依赖 DLL 从 program 同目录解析——
   实机验证加载不报 126/127；
3. Windows worker 专项（**已写代码未实机**）：LO 内嵌 Python 会 seek
   stdin 吞请求 → worker 已实现 fd 0 换 NUL（`steal_stdin_fd`，
   `_dup`/`_dup2`）；内核线程 DPI unaware（tiles assume 96 DPI）与
   请求投递走 message-only 窗口（ElyOffice `worker/messages.rs` 模式）
   **未实现**——本原型以 unipoll + 分发线程直跑，Windows 线程亲和性
   需实机复核（unipoll 模式下 `IsMainThread` 语义可能需改走消息泵）；
4. 真实 DOCX（中文/图片/表格）渲染、编辑、saveAs、重开链
   （`cargo test -p mdc-lok --test lok_real -- --nocapture` +
   `cargo run -p mdc-lok --example smoke`）；
5. 故障注入三件套（kill / 超时 / 拒写）在 Windows 的行为
   （`cargo test -p mdc-lok --test boundary`——TerminateProcess 语义）。

## Linux / macOS 状态

- **Linux（Debian 系 25.2.3.2）：已实测全链**（smoke 3/3 稳定 +
  lok_real + boundary 10/10 稳定，证据见 gates/633.md）；
- macOS：未测（待有实机后按同清单跑；加载 `libmergedlo.dylib`，
  hook 语义同 Linux）；
- 其他发行版（RHEL/Fedora 的 LO 打包布局可能不同）：未测；发现序
  覆盖不到时用 `MDC_LOK_DIR` 显式指定。

## 许可来源

- LibreOffice：MPL-2.0（本仓只 dlopen 动态链接，不分发 LO 二进制；
  运行时由用户安装或发行包提供）；
- ElyOffice（ely-lok/ely-ipc）：MIT——协议/进程模型/unipoll 模式
  参考（commit dc32744d），未整包搬运代码；
- mdc-lok 自身：随本仓许可（见仓库根 LICENSE）。
