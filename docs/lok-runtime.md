# LOK 运行时：部署、诊断与损失边界（B5 · #49 工作线 B）

> 运行时发现/ABI 钉定/Windows 复现清单见 `crates/lok/src/runtime.rs`
> 模块文档（权威）；会话 API 契约见 `docs/lok-session-contract.md`。
> 本文是 **B5 交付**：可运行冒烟/依赖部署清单、诊断收集、故障损失
> 边界声明。

## 1. 依赖部署（Linux，已实测链）

```bash
# Debian/Ubuntu：LibreOffice 全量（含 Writer 内核）
sudo apt install libreoffice          # 本轮实测 = Debian 13, LO 25.2.3.2

# 验证 hook 导出（发现序在 worker 内自动走，此处人工预检）
objdump -T /usr/lib/libreoffice/program/libmergedlo.so | grep libreofficekit_hook_2

# 内核 SHA-256（与 runtime 登记对照）
sha256sum /usr/lib/libreoffice/program/libmergedlo.so
```

环境变量（发现优先级：env → 部署位 → 系统常见位）：

| 变量 | 语义 |
|---|---|
| `MDC_LOK_HOME` | LibreOffice 安装根（含 `program/`、`share/`） |
| `MDC_LOK_LIB` | 直接指定内核库文件（program 目录取父目录） |
| `MDC_LOK_WORKER_BIN` | worker 可执行文件（部署 resources/ 布局或测试注入） |

## 2. 可运行冒烟（Linux 命令原文）

```bash
# 全链 26 步：发现→load→init→size→paint→搜索→键入编辑→回调脏态→
# 拒写保脏→saveAs→源 hash 不变→双 worker 并行→重开校验→看门狗→外杀
cargo test -p mdc-lok --test lok_smoke -- --nocapture --test-threads=1

# B2-B5 附加门
cargo test -p mdc-lok --test session_contract -- --test-threads=1   # 契约 11 面
cargo test -p mdc-lok --test session_lifecycle -- --test-threads=1  # 生命周期 5 面
cargo test -p mdc-lok --test edit_save_reliability -- --test-threads=1  # 保存可靠性 3 面
cargo test -p mdc-lok --test fault_injection -- --test-threads=1    # 坏输入 2 面

# 一键全量（约 4-5 分钟，含真实 LO 各面）
cargo test -p mdc-lok -- --test-threads=1
```

缺运行时时全部真实门打 SKIP 横幅（SKIP ≠ PASS）；协议/帧面
（`tests/protocol.rs`）恒跑不需要运行时。

## 3. 诊断收集

| 证据 | 位置/方法 |
|---|---|
| worker stderr 尾部 | `LokError::WorkerDied { stderr_tail }`（宿主自动保留 200 行） |
| 死亡详情 | `DocSession::death_detail()` / `WorkerClient::death_detail()` |
| 会话目录 | `DocSession::session_dir()`（profile + tile side-file 诊断产物） |
| runtime 来源 | `runtime::discover()` → `{program_dir, kernel_lib, kernel_sha256}` |
| 版本信息 | `DocSession` 通路 `WorkerClient::request("version")`（LO `getVersionInfo` JSON） |
| smoke 诊断产物 | `/tmp/mdc-lok-smoke-*`：steps.jsonl / report.json / tile-*.bmp |

## 4. 故障与损失边界（如实声明）

### 故障面证据（全部实测）

| 故障 | 行为 | 证据 |
|---|---|---|
| worker 挂起 | 看门狗超时（默认 120s）→ 杀 worker → `E_TIMEOUT` | B1 smoke |
| worker 外杀（kill -9） | 死亡扩散 → `WorkerDied`（含 stderr 尾部）→ 新会话恢复 | B3 lifecycle |
| 保存拒写 | `E_SAVE` + **dirty 保留** + 会话仍可用（恢复保存成功） | B4 reliability |
| 坏运行时（垃圾 .so） | 发现/dlopen 明确失败（路径化信息），不挂死 | B3 lifecycle |
| 坏输入：缺文件 | `E_LOAD`：`Unsupported URL ... type detection failed` | B5 fault_injection |
| 坏输入：垃圾字节（伪 PK 头） | LO type detection 内部挂起 → **120s 看门狗终止**（真实边界，宿主不挂死） | B5 fault_injection |
| 坏帧（非 JSON/超限） | 协议层拒绝（分配前），worker 存活 | tests/protocol.rs 恒跑 |
| profile 锁残留 | spawn 前清空会话目录 + `SAL_NOLOCK_PROFILE=1`；×5 反复开关无互锁 | B3 lifecycle |

### 损失边界（C 线 #50 关闭交互的依据）

- **内存态编辑**（未 saveAs 的 dirty 内容）随 worker 进程消亡——
  kill/崩溃/超时/保存失败均**不可恢复**。C 线关闭交互必须在
  `is_dirty()` 时引导保存（保存安全链 = 暂存产物 → verify 独立断言
  → 决定替换，见 B4）。
- **已 saveAs 的暂存产物**始终完好（源文件永不被动——worker 硬守卫
  禁止覆盖加载源；替换决策与「最近有效产物」由宿主/C 线持有）。
- **源文件**在任何故障下字节不变（hash 断言在 B1/B3/B4 三面实证）。
- 保存失败**不会静默报成功**（`save_as` Err 即未落盘、dirty 保留）。

### Windows（未执行——如实声明）

- 代码就绪面：发现序 Windows 段（`Program Files` 候选）/
  `LoadLibraryExW` 搜索标志 / `steal_stdin_fd` win 分支（`_dup`/
  `_dup2`/`NUL`）/ **#104 message-only window 消息驱动主循环**
  （`messages_loop`：RegisterClassW + CreateWindowExW(HWND_MESSAGE)
  + reader 线程 PostMessageW(ASK/CLOSED) + WndProc 复用 handle_frame
  同步分发 + runLoop 空钩子——unipoll 机件已 cfg 收窄到非 Windows；
  ElyOffice `worker/messages.rs` 参照；Windows 编译面
  `--target x86_64-pc-windows-msvc --all-targets` 绿）。
- **实机门未执行**（沙箱无 Windows）：全链 + 故障三件套按
  `runtime.rs` 模块文档「Windows 复现清单」在实机执行（#48/#52
  协调）；不得以 Linux 结果或测试绿宣称 Windows 完成。#104 实机
  验收脚本：`scripts/lok/win104_messages_accept.ps1`（三帧全应答 +
  存活门 + 干净关机，_ape2.ps1 匿名管道逐帧模式）。
- 已知未实测风险面（ElyOffice 实证，防御已内置）：fd 0 抢读 /
  per-monitor DPI tile 偏移 / liblangtag LT_LOCK 句柄泄漏（长会话
  反复开关可能泄漏，冒烟一轮不受影响）。

## 5. 发行参数证据

最终发行（#52：DOCX 外部内核方向与运行时分发验收）需要的输入已
备齐：runtime 来源三件组（program_dir / kernel_lib / kernel_sha256）、
版本信息（getVersionInfo JSON）、ABI 钉定 commit（`0d51fdc…`，见
gates/633.md）、许可证面（LO = MPL-2.0；本 crate 自研面无拷贝）。
