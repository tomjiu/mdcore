//! rpc —— 本机控制通道（#104 任务 2「RPC 桥接」+ #111 任务 2「RPC 扩展」）。
//!
//! ## 定位：Lead 长期设计「AI 操作运行中的 UI，实时反馈」的地基
//!
//! #109 打通了「请求-响应」链路（6 命令最小版）；#111 在其上扩展：
//! **订阅模式**——客户端 `subscribe` 后，服务端在每次文档变更（用户
//! 编辑 / AI 编辑 / 撤销重做）时**主动推送** `doc_changed` 事件，双向
//! 回显即成：AI 发命令改文档 → 用户界面实时看到；用户手动编辑 →
//! AI 通过订阅收到通知。两边走同一个事件流（[`crate::doc_events`]）。
//!
//! ## 架构（#111：响应与事件同一条出口）
//!
//! ```text
//! 外部客户端（python/nc …）                mang-shell 进程
//! ─────────────────────                  ────────────────────────────
//!   AF_UNIX socket  ──JSON 行──▶ accept 循环 ──▶ Job 队列（带 conn id）
//!   （每连接一个任务）                        │
//!                                            ▼
//!                                 dispatcher（单任务串行，主线程执行）
//!                                 cx.update → window.update
//!                                     │                │（编辑 → doc_events
//!   ◀──JSON 行（响应）─── out 通道 ◀────┘                │  总线 → 事件泵）
//!   ◀──JSON 行（事件）─── out 通道 ◀── 事件泵（订阅表广播）
//! ```
//!
//! - 每连接一条 `out` 通道（多生产者：dispatcher 写响应 / 事件泵写
//!   事件；单消费者：connection 任务内 `select` 写 socket）——**响应
//!   与事件同出口，天然保序**；多客户端各自独立通道互不干扰。
//! - `select` 读写交替（`futures_lite::future::select`）：读 future 半途
//!   被事件分支抢先而丢弃是安全的——数据留内核缓冲，下轮照读。
//! - **传输**：unix domain socket，JSON Lines（一行一 JSON，UTF-8）。
//!   本机通信——不做加密/认证/网络（#104/#111 明确边界）。
//! - **端点**：`$XDG_RUNTIME_DIR/mang-shell.sock`，缺省
//!   `/tmp/mang-shell-$UID.sock`；`--rpc-sock PATH` 显式覆盖（烟雾与
//!   多实例场景）；`--no-rpc` 关闭。
//! - **退出清理**：`ControlServer` Drop 时删除 socket 文件；异常退出
//!   的残留由下次启动 [`prepare_socket`] 探活清理（连得上 = 别的实例
//!   在监听 → 本实例禁用 RPC；连不上 = 陈旧文件 → 删除后重绑）。
//!
//! ## 协议（#134 后 27 命令 + 事件流）
//!
//! 面向调用方的完整参考（参数/返回/事件/沙箱/踩坑）见
//! `docs/rpc-protocol.md`——该文与本节同源，改动时两边一起改。
//!
//! 请求（`id` 可省——省略时响应 `id` 为 null；未知字段忽略——前向
//! 兼容）。#104 六命令（open_file / insert_text / replace_selection /
//! get_state / close_file / switch_tab）之上，#111 任务 2 新增导航/历史/
//! 保存/订阅面，#134 任务 1 新增文件系统面：
//!
//! ```json
//! {"id": 7,  "cmd": "set_cursor",     "line": 10, "col": 5}
//! {"id": 8,  "cmd": "set_selection",  "start": {"line": 0, "col": 1},
//!                                         "end":  {"line": 0, "col": 5}}
//! {"id": 9,  "cmd": "get_selection"}
//! {"id": 10, "cmd": "undo"}
//! {"id": 11, "cmd": "redo"}
//! {"id": 12, "cmd": "save"}
//! {"id": 13, "cmd": "save_as",        "path": "/tmp/b.md"}
//! {"id": 14, "cmd": "scroll_to",      "line": 42}
//! {"id": 15, "cmd": "subscribe"}
//! {"id": 16, "cmd": "list_dir",        "path": "src", "recursive": true}
//! {"id": 17, "cmd": "read_file",       "path": "src/main.rs"}
//! {"id": 18, "cmd": "write_file",      "path": "src/lib.rs", "content": "…"}
//! {"id": 19, "cmd": "create_file",     "path": "src/new.rs"}
//! {"id": 20, "cmd": "create_dir",      "path": "src/util"}
//! {"id": 21, "cmd": "delete",          "path": "src/old.rs"}
//! {"id": 22, "cmd": "rename",          "from": "src/a.rs", "to": "src/b.rs"}
//! {"id": 25, "cmd": "run_command",     "name": "goto_line"}
//! {"id": 26, "cmd": "list_commands"}
//! {"id": 27, "cmd": "get_outline"}
//! {"id": 28, "cmd": "apply_format",    "range": [{"line":0,"col":0},{"line":0,"col":4}],
//!                                        "format": "bold"}
//! ```
//!
//! ### #134 文件系统安全边界（沙箱）
//!
//! 全部 FS 命令只允许在**工作区根目录**内操作（无工作区 = 一律拒绝）
//! ——[resolve_sandboxed](#)：canonicalize 解析后检查前缀（组件级），
//! 相对路径相对根，`..`/符号链接逃逸全部拒在门外。写已打开文件的
//! 语义：只写盘不动 UI，已打开的干净文本标签自动 reload 同步（走
//! 完整编辑链——订阅端照收 doc_changed）；只读渲染态/脏标签拒绝
//!（引导 close/save）。删除：根自身拒绝、路径下脏标签拒绝；rename：
//! 不覆盖、已开标签身份自动迁移。
//!
//! 响应（恒一行 JSON；`ok` 分支二选一 `result`/`error`）：
//!
//! ```json
//! {"id": 2, "ok": true,  "result": {"cursor": {"line": 0, "col": 6}, ...}}
//! {"id": 5, "ok": false, "error": "文档有未保存修改——先保存再关闭"}
//! ```
//!
//! ## 事件流（订阅后服务端主动推）
//!
//! ```json
//! {"type": "doc_changed", "path": "/tmp/a.md", "revision_before": 41,
//!  "revision": 42, "source": "user", "changes": [
//!    {"kind": "replace", "start": {"line": 10, "col": 5},
//!     "end": {"line": 10, "col": 5}, "text": "hi"}]}
//! ```
//!
//! 语义详见 [`crate::doc_events`]（事件源头与字段口径）；端到端断言
//! 见 `tests/rpc_client.py`（订阅 / 双向回显 / 多客户端互不干扰）。
//!
//! ## 命令语义要点（#111 新面）
//!
//! - `set_cursor`/`set_selection`/`scroll_to`/`get_selection`：导航面
//!   ——只读文档同样可用（浏览场景）；越界 clamp 到文内最近点。
//! - `undo`/`redo`：与 `Ctrl+Z`/`Ctrl+Y` 同链（多光标批合并）；栈空
//!   不是错误（`undid`/`redid` = false）。
//! - `save`：未命名文档拒绝（无路径——引导 `save_as`）；只读渲染态
//!   拒绝（无保存语义）。`save_as`：沙箱解析后写盘 + 标签身份迁移
//!   （#275 起与 `open_file` 同受工作区门禁——见下节）。
//! - `subscribe`：幂等（重复订阅替换旧通道）；断连自动注销（事件泵
//!   send 失败即移除）。
//!
//! ## 命令语义要点（#275 沙箱收口）
//!
//! - `open_file`/`save_as` 与 FS 命令族同门（`resolve_sandboxed` +
//!   `workspace_root_of`）：无工作区拒、根外拒——补上 #134 沙箱在
//!   ControlCommand 族的两处旁路（S-1 读链 / S-2 写链的源头，审计
//!   P1-9）。`save` 的 file_path 源头由此全部受控（已门禁的
//!   open_file/save_as + GUI 对话框用户意图）。GUI 菜单/对话框不经
//!   RPC 命令面，不受影响。
//!
//! ## 命令语义要点（#134 文件系统面）
//!
//! - `list_dir`/`read_file`：浏览面——只读盘不碰 UI；`list_dir` 递归
//!   上限 5000 条（截断置 truncated），`read_file` 上限 1 MiB（截断
//!   到字符边界）+ 非 UTF-8 拒绝。
//! - `write_file`：磁盘是权威——写盘不动 UI；已打开的干净文本标签
//!   reload 同步（可 Ctrl+Z 撤回）。`create_file`/`create_dir`：已存
//!   在拒绝（不隐式覆盖/不隐式建多层——操作可预期）。
//! - `delete`：AI 有责任直接删（第一版无回收站，#134 给的取舍）；
//!   根自身 + 脏标签路径拒（防丢用户未保存的编辑），干净标签自动关。
//! - `rename`：不覆盖已存在目标；from 是目录时整棵子树的已开标签
//!   身份迁移（修改态跟随新名——下次 save 落新路径）。
//!
//! ## 命令语义要点（#134 编辑器命令面）
//!
//! - `run_command`：按机器名调现有 18 命令（`Cmd::from_id`——与命令
//!   面板/键位同一分派路径，只读门禁全同语义）；未知名拒并引导
//!   `list_commands`。`args` 保留前向兼容（现有命令均零参）。
//! - `list_commands`：机器名 + 面板标题 + 默认键位（`Cmd::ALL` 全集）。
//! - `get_outline`：md 标题树（[`highlight::heading_level`] 同源判定；
//!   平铺 entries 带 level/line/text——AI 自行组树）；渲染态拒。
//! - `apply_format`：范围文本 md 标记对包裹（bold/italic/code/
//!   strikethrough）；一条 undo；空范围/只读态/行模型态拒。
//!
//! ## 验证
//!
//! - 单测：协议解析/序列化、事件行形态、订阅表逻辑、沙箱边界
//!   （逃逸/符号链接/组件前缀）——文件尾 mod tests。
//! - 端到端：`rpc_smoke.sh`（Xvfb + `tests/rpc_client.py`——27 命令
//!   全链 + 订阅/双向回显（`--smoke-user-type` 合成用户键入路径）+
//!   双客户端并发 + `--smoke-workspace` 沙箱实测（FS 命令 + 越界拒绝
//!   + 写已开文件同步 + rename 标签迁移））。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_net::unix::{UnixListener, UnixStream};
use futures_lite::io::{AsyncReadExt, AsyncWriteExt};
use serde::{Deserialize, Serialize};

use gpui::{App, AppContext, AsyncApp, Context, Task, Window, WindowHandle};

use crate::editor::SaveOutcome;
use crate::editor::buffer::Point;
use crate::shell::ShellApp;

/// 单行请求字节上限（1 MiB）——防无换行洪水把内存吃爆；超限回错误
/// 并断连（客户端侧修包重连）。
const MAX_LINE_BYTES: usize = 1024 * 1024;

/// accept 连续失败上限——不可恢复错误（如 fd 耗尽）下停止服务打日志，
/// 不无限刷屏。
const ACCEPT_ERROR_LIMIT: u32 = 32;

// ---------------------------------------------------------------------------
// 协议面（纯数据 + 纯函数——单测主战场）
// ---------------------------------------------------------------------------

/// 控制命令（#104 六命令 + #111 任务 2 八命令 + 订阅）。
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "cmd")]
pub enum ControlCommand {
    /// 打开文件（已开 → 激活既有标签，不重复开）。
    #[serde(rename = "open_file")]
    OpenFile { path: String },

    /// 在指定行列插入文本（行列 0 基；越界由 Buffer 钳制到文内最近点）。
    #[serde(rename = "insert_text")]
    InsertText {
        line: usize,
        col: usize,
        text: String,
    },

    /// 替换当前选区（无选区 = 光标处插入；空文本 + 有选区 = 删除选区）。
    #[serde(rename = "replace_selection")]
    ReplaceSelection { text: String },

    /// 获取当前状态（标签清单 + 活动文档光标/选区/字数）。
    #[serde(rename = "get_state")]
    GetState,

    /// 关闭指定文件的标签（脏标签拒绝——先保存）。
    #[serde(rename = "close_file")]
    CloseFile { path: String },

    /// 切换到指定标签（索引 0 基）。
    #[serde(rename = "switch_tab")]
    SwitchTab { index: usize },

    // ---------------- #111 任务 2：8 新命令 + 订阅 ----------------
    /// 设置光标位置（0 基；越界钳制。只读文档可用——导航面）。
    #[serde(rename = "set_cursor")]
    SetCursor { line: usize, col: usize },

    /// 设置选区（两点任意次序，排序后 anchor/cursor；越界钳制）。
    #[serde(rename = "set_selection")]
    SetSelection { start: PointWire, end: PointWire },

    /// 获取当前选区（含文本——AI 上下文消费主通道；无选区时 result
    /// 里 `selection` 为 null）。
    #[serde(rename = "get_selection")]
    GetSelection,

    /// 撤销一步手势（同 `Ctrl+Z`——多光标批合并；栈空不是错误）。
    #[serde(rename = "undo")]
    Undo,

    /// 重做一步（同 `Ctrl+Y`）。
    #[serde(rename = "redo")]
    Redo,

    /// 保存当前文件（未命名/只读拒绝——引导 save_as / 明确无语义）。
    #[serde(rename = "save")]
    Save,

    /// 另存为（绝对化后写盘 + 标签身份迁移——路径/display_name 更新）。
    #[serde(rename = "save_as")]
    SaveAs { path: String },

    /// 滚动到指定行（纯视口移动——不动光标；只读文档可用）。
    #[serde(rename = "scroll_to")]
    ScrollTo { line: usize },

    /// 订阅文档变更事件（后续每次变更服务端主动推 `doc_changed`；
    /// 幂等；断连自动注销）。
    #[serde(rename = "subscribe")]
    Subscribe,

    // ---------------- #134 任务 1：文件系统命令（工作区沙箱内） ----------------
    /// 列目录（相对路径相对工作区根；`recursive` 缺省 false——只列
    /// 一层。条目上限 [`LIST_DIR_MAX_ENTRIES`]，超限截断置 truncated）。
    #[serde(rename = "list_dir")]
    ListDir {
        path: String,
        #[serde(default)]
        recursive: bool,
    },

    /// 读文件（不打开 UI——直接读盘给 AI 消费；上限
    /// [`READ_FILE_CAP_BYTES`]，超限截断置 truncated；非 UTF-8 拒绝）。
    #[serde(rename = "read_file")]
    ReadFile { path: String },

    /// 写文件（只写盘，不动 UI——已打开的干净文本标签自动 reload
    /// 同步；只读渲染态/脏标签拒绝）。父目录必须存在。
    #[serde(rename = "write_file")]
    WriteFile { path: String, content: String },

    /// 新建文件（已存在拒绝——覆盖走 write_file；`content` 可省 = 空文件）。
    #[serde(rename = "create_file")]
    CreateFile {
        path: String,
        #[serde(default)]
        content: Option<String>,
    },

    /// 新建目录（单层——父目录必须存在；已存在拒绝）。
    #[serde(rename = "create_dir")]
    CreateDir { path: String },

    /// 删除（文件 remove_file / 目录递归 remove_dir_all；工作区根
    /// 自身拒绝；路径下有脏标签拒绝、干净标签自动关闭）。
    #[serde(rename = "delete")]
    Delete { path: String },

    /// 重命名/移动（`from`→`to` 都必须在沙箱内；`to` 已存在拒绝；
    /// 已打开标签身份自动迁移——修改态跟随新名）。
    #[serde(rename = "rename")]
    Rename { from: String, to: String },

    // ---------------- #134 任务 2：编辑器命令补完 ----------------
    /// 执行命令（按机器名——与命令面板/键位同一分派路径；可用清单
    /// `list_commands`；`args` 保留前向兼容，现有 18 命令均零参）。
    #[serde(rename = "run_command")]
    RunCommand {
        name: String,
        #[serde(default)]
        args: Option<serde_json::Value>,
    },

    /// 列出可用命令（机器名 + 面板标题 + 默认键位）。
    #[serde(rename = "list_commands")]
    ListCommands,

    /// 查询文档大纲（md 标题树——与语法高亮同一判定源；平铺条目带
    /// level，AI 可自行组树）。渲染态文档无 md 大纲语义拒。
    #[serde(rename = "get_outline")]
    GetOutline,

    /// 应用 md 行内格式（范围文本标记对包裹——bold/italic/code/
    /// strikethrough；一条 undo；两点任意次序）。
    #[serde(rename = "apply_format")]
    ApplyFormat {
        range: (PointWire, PointWire),
        format: crate::editor::FormatKind,
    },

    // ---------------- #134 任务 3：批量操作 ----------------
    /// 跨文件批量替换（沙箱内；已打开文本标签走 Buffer 一条 undo 全批，
    /// 未打开文件直接改盘；返回每文件改动数）。
    #[serde(rename = "batch_replace")]
    BatchReplace {
        files: Vec<String>,
        pattern: String,
        replacement: String,
        #[serde(default)]
        is_regex: bool,
    },
}

/// 点位线上形态（`{"line": n, "col": n}`——与响应 `point_json` 同构）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct PointWire {
    pub line: usize,
    pub col: usize,
}

impl PointWire {
    fn into_point(self) -> Point {
        Point::new(self.line, self.col)
    }
}

impl ControlCommand {
    /// 命令名（证据行/日志面——与线上 `cmd` 字段一致）。
    pub fn name(&self) -> &'static str {
        match self {
            ControlCommand::OpenFile { .. } => "open_file",
            ControlCommand::InsertText { .. } => "insert_text",
            ControlCommand::ReplaceSelection { .. } => "replace_selection",
            ControlCommand::GetState => "get_state",
            ControlCommand::CloseFile { .. } => "close_file",
            ControlCommand::SwitchTab { .. } => "switch_tab",
            ControlCommand::SetCursor { .. } => "set_cursor",
            ControlCommand::SetSelection { .. } => "set_selection",
            ControlCommand::GetSelection => "get_selection",
            ControlCommand::Undo => "undo",
            ControlCommand::Redo => "redo",
            ControlCommand::Save => "save",
            ControlCommand::SaveAs { .. } => "save_as",
            ControlCommand::ScrollTo { .. } => "scroll_to",
            ControlCommand::Subscribe => "subscribe",
            ControlCommand::ListDir { .. } => "list_dir",
            ControlCommand::ReadFile { .. } => "read_file",
            ControlCommand::WriteFile { .. } => "write_file",
            ControlCommand::CreateFile { .. } => "create_file",
            ControlCommand::CreateDir { .. } => "create_dir",
            ControlCommand::Delete { .. } => "delete",
            ControlCommand::Rename { .. } => "rename",
            ControlCommand::RunCommand { .. } => "run_command",
            ControlCommand::ListCommands => "list_commands",
            ControlCommand::GetOutline => "get_outline",
            ControlCommand::ApplyFormat { .. } => "apply_format",
            ControlCommand::BatchReplace { .. } => "batch_replace",
        }
    }
}

/// 响应（`ok` 分支二选一：`result` 或 `error`，未用的键不序列化）。
#[derive(Debug, Serialize)]
pub struct ControlResponse {
    pub id: Option<u64>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ControlResponse {
    fn ok(id: Option<u64>, result: serde_json::Value) -> Self {
        Self {
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    fn err(id: Option<u64>, error: String) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(error),
        }
    }

    /// 序列化（Value 面恒成功；兜底仅防未知值形态）。
    fn to_line(&self) -> String {
        let mut s = serde_json::to_string(self)
            .unwrap_or_else(|_| r#"{"id":null,"ok":false,"error":"serialize failed"}"#.into());
        s.push('\n');
        s
    }
}

/// 解析一行请求。返回 `(id 回显, 命令或错误消息)`——JSON 整体损坏时
/// id 取不到（None），命令字段错但 JSON 合法时 id 可回显（客户端能对
/// 上是哪条失败）。
pub fn parse_command(line: &str) -> (Option<u64>, Result<ControlCommand, String>) {
    // 两步走：先宽容取 id（整体 Value），再严格反序列化命令（内部
    // 标签枚举直接吃整对象——未知键自动忽略）。
    let id = serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|v| v.get("id").and_then(|i| i.as_u64()));
    let parsed = serde_json::from_str::<ControlCommand>(line).map_err(|e| e.to_string());
    (id, parsed)
}

// ---------------------------------------------------------------------------
// 端点解析与 socket 预备（纯函数面——单测）
// ---------------------------------------------------------------------------

/// 端点解析（纯函数核心——env 读取在 [`resolve_socket_path`] 薄壳）。
///
/// 优先级：显式覆盖 > `$XDG_RUNTIME_DIR`（XDG 规范要求绝对路径，
/// 相对值忽略）> `/tmp/mang-shell-$UID.sock`（无 uid 信息时退
/// `/tmp/mang-shell.sock`——单用户机兜底）。
pub fn default_socket_path(
    explicit: Option<PathBuf>,
    xdg_runtime: Option<PathBuf>,
    uid: Option<u32>,
) -> PathBuf {
    if let Some(path) = explicit {
        return path;
    }
    if let Some(dir) = xdg_runtime {
        return dir.join("mang-shell.sock");
    }
    match uid {
        Some(uid) => PathBuf::from(format!("/tmp/mang-shell-{uid}.sock")),
        None => PathBuf::from("/tmp/mang-shell.sock"),
    }
}

/// 端点解析（进程面薄壳——读 env 与 /proc 属主 uid）。
pub fn resolve_socket_path(explicit: Option<&str>) -> PathBuf {
    default_socket_path(
        explicit.map(PathBuf::from),
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute()),
        process_uid(),
    )
}

/// 本进程 uid（std-only 口径：/proc/self 属主即进程 uid——免引 libc）。
fn process_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").ok().map(|m| m.uid())
}

/// socket 预备结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prepared {
    /// 可绑（路径不存在，或陈旧残留已清）。
    Ready,
    /// 路径上有活实例在监听——本实例应禁用 RPC。
    Busy,
}

/// socket 预备：探活 + 清残留。
///
/// - 路径不存在 → `Ready`。
/// - 存在且能连通 → `Busy`（另一实例在监听）。
/// - 存在但连不通 → 陈旧残留（上次异常退出）→ 删除后 `Ready`。
/// - 存在但不是 socket 文件（普通文件/目录）→ 报错不动它（防
///   `--rpc-sock` 误指重要文件被删）。
pub fn prepare_socket(path: &Path) -> std::io::Result<Prepared> {
    use std::os::unix::fs::FileTypeExt;

    if !path.exists() {
        return Ok(Prepared::Ready);
    }
    if std::os::unix::net::UnixStream::connect(path).is_ok() {
        return Ok(Prepared::Busy);
    }
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.file_type().is_socket() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("路径已存在且不是 socket 文件：{}", path.display()),
        ));
    }
    std::fs::remove_file(path)?;
    Ok(Prepared::Ready)
}

// ---------------------------------------------------------------------------
// 行分帧（连接读缓冲——纯逻辑，单测钉定）
// ---------------------------------------------------------------------------

/// 字节流 → 行分帧器（feed 追加；next_line 吐完整行；overflowed 防洪水）。
struct LineAssembler {
    buf: Vec<u8>,
    cap: usize,
}

impl LineAssembler {
    fn new(cap: usize) -> Self {
        Self {
            buf: Vec::new(),
            cap,
        }
    }

    fn feed(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
    }

    /// 弹出下一条完整行（到 `\n`，含 `\r\n` 归一）；无完整行 `None`。
    /// 行内容 UTF-8 有损解码（坏字节不炸连接）。
    fn next_line(&mut self) -> Option<String> {
        let pos = self.buf.iter().position(|&b| b == b'\n')?;
        let line: Vec<u8> = self.buf.drain(..=pos).collect();
        let mut line = &line[..line.len() - 1]; // 去 \n
        if line.last() == Some(&b'\r') {
            line = &line[..line.len() - 1];
        }
        Some(String::from_utf8_lossy(line).into_owned())
    }

    /// 待处理字节超上限（首个未完行已超 cap）。
    fn overflowed(&self) -> bool {
        self.buf.len() > self.cap
    }

    /// EOF 尾包：无换行的末行（宽容客户端不补换行）。
    fn take_remainder(&mut self) -> Option<String> {
        if self.buf.is_empty() {
            return None;
        }
        let line = String::from_utf8_lossy(&self.buf).into_owned();
        self.buf.clear();
        Some(line)
    }
}

// ---------------------------------------------------------------------------
// 服务面
// ---------------------------------------------------------------------------

/// 一条待执行命令（dispatcher 投递单元）。
///
/// `reply` = 该连接的 out 通道发送端（#111：响应不再 per-command
/// oneshot——与事件流同一条出口，单 writer 消费保序）；`conn` = 连接
/// id（`subscribe` 注册订阅的键；断连注销的键）。
struct Job {
    id: Option<u64>,
    conn: u64,
    command: ControlCommand,
    reply: async_channel::Sender<String>,
}

/// 订阅表（conn id → 该连接 out 通道发送端）。
///
/// dispatcher（Subscribe 命令时写入）与事件泵（每次文档变更时遍历
/// 广播）共享；Mutex 短临界区（clone sender + send 都是非阻塞面）。
/// send 失败（对端已断）即移除——断连自动注销。
type Subscriptions =
    std::sync::Mutex<std::collections::HashMap<u64, async_channel::Sender<String>>>;

/// 注册订阅（幂等：同 conn id 重复订阅替换旧通道）。
fn subscribe_insert(subs: &Subscriptions, conn: u64, out: async_channel::Sender<String>) {
    subs.lock().expect("订阅表锁中毒").insert(conn, out);
}

/// 广播一条已序列化的事件行（send 失败即移除订阅——断连自动注销）。
/// 返回实际送达的订阅者数。
fn subscribe_broadcast(subs: &Subscriptions, line: &str) -> usize {
    let mut guard = subs.lock().expect("订阅表锁中毒");
    let mut delivered = 0;
    let mut dead: Vec<u64> = Vec::new();
    for (conn, out) in guard.iter() {
        // try_send：unbounded 通道发送不阻塞——失败即对端已断。
        if out.try_send(line.to_string()).is_ok() {
            delivered += 1;
        } else {
            dead.push(*conn);
        }
    }
    for conn in dead {
        guard.remove(&conn);
        eprintln!("rpc: 订阅注销（conn {conn} 对端已断）");
    }
    delivered
}

/// 控制服务句柄。存活周期 = 应用全局（GPUI `Global`）——注意 `run`
/// 闭包只是 setup，闭包返回后事件循环才开跑，局部变量会在 setup 尾
/// 被 Drop 导致 socket 立即解除链接，**必须**挂全局存到 App 退出；
/// Drop 时清 socket 文件（正常退出不留残；异常退出的残留由下次启动
/// [`prepare_socket`] 探活清理）。
pub struct ControlServer {
    path: PathBuf,
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl gpui::Global for ControlServer {}

impl ControlServer {
    /// 启动控制服务：预备 socket → 绑定 → 派生 accept 循环、
    /// dispatcher 与事件泵（均为 detached 任务，随应用退出消亡）→
    /// **挂应用全局**（存活到 App 退出——见类型文档的 Drop 陷阱）。
    ///
    /// #111：成功时初始化 [`crate::doc_events`] 总线并启动事件泵
    ///（总线接收端 → 序列化 → 订阅表广播）——「订阅模式」的服务端面。
    ///
    /// 失败不致命（返回 `false`，应用照常跑）：端点被占（另一实例）/
    /// 绑定失败/预备失败只损失 RPC 能力，桌面外壳本体不受影响。
    /// 成功返回 `true`。
    pub fn start(path: PathBuf, window: WindowHandle<ShellApp>, cx: &mut App) -> bool {
        match prepare_socket(&path) {
            Ok(Prepared::Ready) => {}
            Ok(Prepared::Busy) => {
                eprintln!(
                    "rpc: 端点被占用（{}）——另一实例在监听，本实例 RPC 禁用",
                    path.display()
                );
                return false;
            }
            Err(err) => {
                eprintln!("rpc: socket 预备失败（{}）：{err}", path.display());
                return false;
            }
        }
        let listener = match UnixListener::bind(&path) {
            Ok(listener) => listener,
            Err(err) => {
                eprintln!("rpc: 绑定失败（{}）：{err}", path.display());
                return false;
            }
        };
        let (tx, rx) = async_channel::unbounded::<Job>();
        let subs: Arc<Subscriptions> =
            Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        // #111：事件总线初始化（单接收端契约）+ 事件泵（detached）。
        // 失败（已被初始化）只损失事件面——命令面照常。
        match crate::doc_events::init_bus() {
            Some(bus_rx) => {
                let pump_subs = subs.clone();
                cx.spawn(async move |cx| event_pump(bus_rx, pump_subs, cx).await)
                    .detach();
                eprintln!("rpc: 事件泵已启动（订阅模式就绪）");
            }
            None => eprintln!("rpc: 事件总线已被占用——订阅模式不可用（命令面照常）"),
        }
        // WindowHandle 实现 Copy——直传免 clone（clippy::clone_on_copy）。
        cx.spawn(async move |cx| dispatcher_loop(rx, window, subs, cx).await)
            .detach();
        cx.spawn(async move |cx| accept_loop(listener, tx, cx).await)
            .detach();
        eprintln!("rpc: listening at {}", path.display());
        // 挂应用全局（存活到 App 退出——run 闭包返回后事件循环开跑，
        // 局部绑定会在 setup 尾被 Drop 把 socket 立即解除链接）。
        cx.set_global(Self { path });
        true
    }
}

/// accept 循环：每连接一个后台任务（收发不互阻——多客户端并发基础）。
/// #111：每连接分配递增 conn id（订阅键）+ 独立 out 通道（响应与
/// 事件同出口）。
async fn accept_loop(listener: UnixListener, tx: async_channel::Sender<Job>, cx: &mut AsyncApp) {
    let live = Arc::new(AtomicUsize::new(0));
    let next_conn = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let mut errors: u32 = 0;
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                errors = 0;
                let live_now = live.fetch_add(1, Ordering::SeqCst) + 1;
                let conn = next_conn.fetch_add(1, Ordering::SeqCst);
                eprintln!("rpc: client connected（conn {conn}，{live_now} 路）");
                let tx = tx.clone();
                let live = live.clone();
                let task: Task<()> = cx.background_spawn(async move {
                    connection_task(stream, tx, conn).await;
                    let live_now = live.fetch_sub(1, Ordering::SeqCst) - 1;
                    eprintln!("rpc: client disconnected（conn {conn}，{live_now} 路）");
                });
                task.detach();
            }
            Err(err) => {
                errors += 1;
                eprintln!("rpc: accept 失败（{errors}/{ACCEPT_ERROR_LIMIT}）：{err}");
                if errors >= ACCEPT_ERROR_LIMIT {
                    eprintln!("rpc: accept 连续失败达上限——控制服务停止");
                    return;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(200))
                    .await;
            }
        }
    }
}

/// 单连接任务：`race` 交替「读 socket（命令）」与「写 out 通道消费
///（响应/事件）」。
///
/// #111 关键改造（原：读行 → oneshot 等响应 → 回写）：
/// - 响应不再被读循环等待——dispatcher 执行完直接 send 到本连接的
///   out 通道（fire-and-forget）；读循环可以连续投递命令。
/// - 事件泵也向同一 out 通道 send——**响应与事件同一出口、单消费
///   者写出，天然保序**（async_channel FIFO）。
/// - `race`（futures-lite 无 `select`——同输出型双 future 竞速替代）
///   先完成者赢，另一个被 drop：读 future 半途被事件分支抢先而丢弃
///   是安全的——数据留内核缓冲，下轮照读（out 通道的 recv 半途丢弃
///   同样安全——async_channel 消息未取走不丢失）。
async fn connection_task(mut stream: UnixStream, tx: async_channel::Sender<Job>, conn: u64) {
    /// race 双分支统一输出（futures-lite 无 select/Completed——同型
    /// 竞速面）。
    enum Step {
        Read(std::io::Result<usize>),
        Out(Result<String, async_channel::RecvError>),
    }
    let (out_tx, out_rx) = async_channel::unbounded::<String>();
    let mut asm = LineAssembler::new(MAX_LINE_BYTES);
    let mut chunk = [0u8; 8192];
    loop {
        let read_step = async { Step::Read(stream.read(&mut chunk).await) };
        let out_step = async { Step::Out(out_rx.recv().await) };
        match futures_lite::future::race(read_step, out_step).await {
            Step::Read(read) => match read {
                Ok(0) => break, // EOF
                Ok(n) => {
                    asm.feed(&chunk[..n]);
                    while let Some(line) = asm.next_line() {
                        // 投递不等待响应（响应走 out 通道）；写失败
                        //（对端断）由后续轮次的事件写出面发现。
                        handle_line(&line, &tx, &out_tx, conn);
                    }
                    if asm.overflowed() {
                        let resp = ControlResponse::err(
                            None,
                            format!("单行超过 {MAX_LINE_BYTES} 字节上限——连接关闭"),
                        );
                        let _ = out_tx.send(resp.to_line()).await;
                        return;
                    }
                }
                Err(_) => break, // 读错（对端异常断）——静默收工
            },
            Step::Out(out) => match out {
                Ok(line) => {
                    if stream.write_all(line.as_bytes()).await.is_err() {
                        return; // 对端已断——写失败即收工
                    }
                }
                Err(_) => {
                    // out 通道关闭（所有 sender 已 drop）——但本任务还持
                    // 有 out_tx，不可达；防御早退。
                    return;
                }
            },
        }
    }
    // EOF 尾包：最后一条没带换行的行也处理（宽容 netcat/echo 习惯）。
    if let Some(line) = asm.take_remainder() {
        let (reply_tx, _reply_rx) = async_channel::bounded::<String>(1);
        handle_line(&line, &tx, &reply_tx, conn);
    }
    // EOF 后排水剩余响应（尽力而为——对端可能不再读；写错即止）。
    while let Ok(line) = out_rx.try_recv() {
        if stream.write_all(line.as_bytes()).await.is_err() {
            break;
        }
    }
}

/// 单行处理：空行忽略；解析失败回错（走 out 通道）；成功投递
/// dispatcher（不等响应——#111 fire-and-forget）。
fn handle_line(
    line: &str,
    tx: &async_channel::Sender<Job>,
    out: &async_channel::Sender<String>,
    conn: u64,
) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    let (id, parsed) = parse_command(line);
    match parsed {
        Ok(command) => {
            let _ = tx.try_send(Job {
                id,
                conn,
                command,
                reply: out.clone(),
            });
        }
        Err(err) => {
            let resp = ControlResponse::err(id, err);
            let _ = out.try_send(resp.to_line());
        }
    }
}

/// #111 事件泵：文档事件总线接收端 → 序列化 JSON 行 → 订阅表广播。
///（detached 任务，随应用退出消亡；总线接收端 drop 即 publish 面静默
/// 落空——不影响编辑链路。）
async fn event_pump(
    bus: async_channel::Receiver<std::sync::Arc<crate::doc_events::DocEvent>>,
    subs: Arc<Subscriptions>,
    _cx: &mut AsyncApp,
) {
    while let Ok(event) = bus.recv().await {
        let mut line = match serde_json::to_string(event.as_ref()) {
            Ok(s) => s,
            Err(_) => continue, // 序列化失败只丢这一条（不可达——Value 面恒成功）
        };
        line.push('\n');
        let delivered = subscribe_broadcast(&subs, &line);
        eprintln!(
            "rpc: doc-event push（rev {}，{} 订阅者）",
            event.revision, delivered
        );
    }
}

/// dispatcher：单任务串行——命令全部落主线程执行（GPUI 实体修改的
/// 正确性保证；多客户端命令在此天然排队，互不穿插半执行）。
///
/// #111：执行期间置 [`crate::doc_events::mark_rpc_source`]（RAII）——
/// 编辑命令触发的文档变更事件 `source = "rpc"`；用户键入路径无此标记
///（`source = "user"`）。响应回写与订阅注册共用 Job.reply。
async fn dispatcher_loop(
    rx: async_channel::Receiver<Job>,
    window: WindowHandle<ShellApp>,
    subs: Arc<Subscriptions>,
    cx: &mut AsyncApp,
) {
    while let Ok(job) = rx.recv().await {
        // #111 Subscribe 特判：纯通道注册（不碰 GPUI 实体，无需落
        // 主线程）——把本连接 out 通道挂进订阅表，响应与后续事件同一
        // 出口。注册即达——幂等（同 conn id 替换旧通道）。
        if matches!(job.command, ControlCommand::Subscribe) {
            subscribe_insert(&subs, job.conn, job.reply.clone());
            eprintln!(
                "rpc: exec subscribe ok=true（conn {}，现 {} 订阅者）",
                job.conn,
                subs.lock().expect("订阅表锁中毒").len()
            );
            let resp = ControlResponse::ok(job.id, serde_json::json!({ "subscribed": true }));
            let _ = job.reply.send(resp.to_line()).await;
            continue;
        }
        let executed = {
            let _rpc_source = crate::doc_events::mark_rpc_source();
            cx.update(|cx| {
                window
                    .update(cx, |shell, window, cx| {
                        execute_command(shell, window, cx, &job.command)
                    })
                    .unwrap_or_else(|err| Err(format!("窗口不可用：{err}")))
            })
        };
        let resp = match executed {
            Ok(result) => {
                eprintln!("rpc: exec {} ok=true", job.command.name());
                ControlResponse::ok(job.id, result)
            }
            Err(err) => {
                eprintln!("rpc: exec {} ok=false（{err}）", job.command.name());
                ControlResponse::err(job.id, err)
            }
        };
        let _ = job.reply.send(resp.to_line()).await;
    }
}

// ---------------------------------------------------------------------------
// 命令执行（主线程——ShellApp/Editor 公开面上的薄编排）
// ---------------------------------------------------------------------------

/// 执行一条命令（主线程内调用——`window.update` 闭包体）。
///
/// 编辑类命令与用户键入同链（Buffer::replace_range 单点 + undo 一条 +
/// after_edit 收尾）——「AI 操作，UI 实时反馈」走的正是同一渲染管线。
fn execute_command(
    shell: &mut ShellApp,
    window: &mut Window,
    cx: &mut Context<ShellApp>,
    command: &ControlCommand,
) -> Result<serde_json::Value, String> {
    match command {
        ControlCommand::OpenFile { path } => cmd_open_file(shell, window, cx, path),
        ControlCommand::InsertText { line, col, text } => {
            cmd_insert_text(shell, cx, *line, *col, text)
        }
        ControlCommand::ReplaceSelection { text } => cmd_replace_selection(shell, cx, text),
        ControlCommand::GetState => Ok(state_json(shell, cx)),
        ControlCommand::CloseFile { path } => cmd_close_file(shell, window, cx, path),
        ControlCommand::SwitchTab { index } => cmd_switch_tab(shell, window, cx, *index),
        // ---------------- #111：导航面（只读文档可用） ----------------
        ControlCommand::SetCursor { line, col } => cmd_set_cursor(shell, cx, *line, *col),
        ControlCommand::SetSelection { start, end } => cmd_set_selection(shell, cx, *start, *end),
        ControlCommand::GetSelection => cmd_get_selection(shell, cx),
        ControlCommand::ScrollTo { line } => cmd_scroll_to(shell, cx, *line),
        // ---------------- #111：历史面 ----------------
        ControlCommand::Undo => cmd_undo(shell, cx),
        ControlCommand::Redo => cmd_redo(shell, cx),
        // ---------------- #111：保存面 ----------------
        ControlCommand::Save => cmd_save(shell, cx),
        ControlCommand::SaveAs { path } => cmd_save_as(shell, cx, path),
        // #111 Subscribe 在 dispatcher_loop 内特判（纯通道注册，不经
        // 主线程）——此分支不可达，但 match 穷尽性要求列出。
        ControlCommand::Subscribe => unreachable!("subscribe 在 dispatcher 侧特判"),
        // ---------------- #134 任务 1：文件系统命令（沙箱内） ----------------
        ControlCommand::ListDir { path, recursive } => cmd_list_dir(shell, cx, path, *recursive),
        ControlCommand::ReadFile { path } => cmd_read_file(shell, cx, path),
        ControlCommand::WriteFile { path, content } => cmd_write_file(shell, cx, path, content),
        ControlCommand::CreateFile { path, content } => {
            cmd_create_file(shell, cx, path, content.as_deref())
        }
        ControlCommand::CreateDir { path } => cmd_create_dir(shell, cx, path),
        ControlCommand::Delete { path } => cmd_delete(shell, window, cx, path),
        ControlCommand::Rename { from, to } => cmd_rename(shell, cx, from, to),
        // ---------------- #134 任务 2：编辑器命令补完 ----------------
        ControlCommand::RunCommand { name, .. } => cmd_run_command(shell, window, cx, name),
        ControlCommand::ListCommands => cmd_list_commands(),
        ControlCommand::GetOutline => cmd_get_outline(shell, cx),
        ControlCommand::ApplyFormat { range, format } => {
            cmd_apply_format(shell, cx, range.0, range.1, *format)
        }
        // ---------------- #134 任务 3：批量操作 ----------------
        ControlCommand::BatchReplace {
            files,
            pattern,
            replacement,
            is_regex,
        } => cmd_batch_replace(shell, cx, files, pattern, replacement, *is_regex),
    }
}

fn cmd_open_file(
    shell: &mut ShellApp,
    window: &mut Window,
    cx: &mut Context<ShellApp>,
    path: &str,
) -> Result<serde_json::Value, String> {
    // #275 沙箱门禁：open_file 此前是 #134 沙箱旁路（ControlCommand 族
    // 里唯一带路径却不走 resolve_sandboxed 的命令）——S-1 读链（open
    // 任意路径 → run_command("select_all") → get_state/get_selection
    // 每次泄 ≤2000 字符）与 S-2 写链（open 工作区外 → insert_text →
    // run_command("save") 原地越权回写）的共同源头。与 FS 命令族同门：
    // 无工作区拒、根外拒。resolve_sandboxed 产出 canonical 绝对路径，
    // 后续标签匹配/打开逻辑零改动；GUI 菜单/对话框打开不经此命令
    // （用户意图），不受影响。
    let root = workspace_root_of(shell, cx)?;
    let path = resolve_sandboxed(&root, Path::new(path))?;
    // 已开 → 激活既有标签（与 open_document_tab 语义一致；先探便于
    // 响应区分 already_open）。
    if let Some(idx) = shell.tab_index_by_path(&path) {
        shell.activate_tab(idx, window, cx);
        return Ok(serde_json::json!({
            "tab_index": idx,
            "already_open": true,
        }));
    }
    match shell.open_document_tab(path.clone(), cx) {
        Some(focus) => {
            window.focus(&focus, cx);
            Ok(serde_json::json!({
                "tab_index": shell.active_index(),
                "already_open": false,
            }))
        }
        None => Err(format!(
            "打开失败（{}）——不支持或读取错误（详见应用提示横幅）",
            path.display()
        )),
    }
}

fn cmd_insert_text(
    shell: &mut ShellApp,
    cx: &mut Context<ShellApp>,
    line: usize,
    col: usize,
    text: &str,
) -> Result<serde_json::Value, String> {
    let Some(editor) = shell.editor().cloned() else {
        return Err("无活动文档（空态——先 open_file）".into());
    };
    let inserted = text.chars().count();
    match editor.update(cx, |ed, cx| ed.insert_text_at(line, col, text, cx)) {
        Some(cursor) => Ok(serde_json::json!({
            "cursor": point_json(cursor),
            "inserted_chars": inserted,
            "line_count": editor.read(cx).line_count(),
            "char_count": editor.read(cx).char_count(),
        })),
        None => Err("只读文档（渲染态）拒绝编辑".into()),
    }
}

fn cmd_replace_selection(
    shell: &mut ShellApp,
    cx: &mut Context<ShellApp>,
    text: &str,
) -> Result<serde_json::Value, String> {
    let Some(editor) = shell.editor().cloned() else {
        return Err("无活动文档（空态——先 open_file）".into());
    };
    match editor.update(cx, |ed, cx| ed.replace_selection_text(text, cx)) {
        Some((had_selection, cursor)) => Ok(serde_json::json!({
            "had_selection": had_selection,
            "cursor": point_json(cursor),
            "char_count": editor.read(cx).char_count(),
        })),
        None => Err("只读文档（渲染态）拒绝编辑".into()),
    }
}

fn cmd_close_file(
    shell: &mut ShellApp,
    window: &mut Window,
    cx: &mut Context<ShellApp>,
    path: &str,
) -> Result<serde_json::Value, String> {
    let path = std::path::absolute(Path::new(path)).unwrap_or_else(|_| PathBuf::from(path));
    let Some(idx) = shell.tab_index_by_path(&path) else {
        return Err(format!("标签未找到：{}", path.display()));
    };
    if shell.tab_is_modified(idx, cx).unwrap_or(false) {
        return Err(format!(
            "文档有未保存修改（{}）——先保存再关闭",
            path.display()
        ));
    }
    if !shell.close_tab_no_confirm(idx, window, cx) {
        return Err(format!("关闭失败（索引越界？）：{}", path.display()));
    }
    Ok(serde_json::json!({
        "closed_index": idx,
        "tabs_remaining": shell.tab_count(),
    }))
}

fn cmd_switch_tab(
    shell: &mut ShellApp,
    window: &mut Window,
    cx: &mut Context<ShellApp>,
    index: usize,
) -> Result<serde_json::Value, String> {
    if index >= shell.tab_count() {
        return Err(format!(
            "标签索引越界：{index}（共 {} 个）",
            shell.tab_count()
        ));
    }
    shell.activate_tab(index, window, cx);
    Ok(serde_json::json!({ "active_index": index }))
}

// ---------------------------------------------------------------------------
// #111 任务 2 新命令（主线程薄编排——Editor 公开面）
// ---------------------------------------------------------------------------

fn cmd_set_cursor(
    shell: &mut ShellApp,
    cx: &mut Context<ShellApp>,
    line: usize,
    col: usize,
) -> Result<serde_json::Value, String> {
    let Some(editor) = shell.editor().cloned() else {
        return Err("无活动文档（空态——先 open_file）".into());
    };
    let cursor = editor.update(cx, |ed, cx| ed.move_cursor_to(line, col, cx));
    Ok(serde_json::json!({ "cursor": point_json(cursor) }))
}

fn cmd_set_selection(
    shell: &mut ShellApp,
    cx: &mut Context<ShellApp>,
    start: PointWire,
    end: PointWire,
) -> Result<serde_json::Value, String> {
    let Some(editor) = shell.editor().cloned() else {
        return Err("无活动文档（空态——先 open_file）".into());
    };
    let (a, b) = editor.update(cx, |ed, cx| {
        ed.set_selection_range(start.into_point(), end.into_point(), cx)
    });
    Ok(serde_json::json!({
        "selection": { "start": point_json(a), "end": point_json(b) },
    }))
}

fn cmd_get_selection(
    shell: &mut ShellApp,
    cx: &mut Context<ShellApp>,
) -> Result<serde_json::Value, String> {
    let Some(editor) = shell.editor().cloned() else {
        return Err("无活动文档（空态——先 open_file）".into());
    };
    let editor = editor.read(cx);
    let selection = editor.selection_points().map(|(start, end)| {
        let text = editor.selection_text();
        let chars = text.chars().count();
        // 与 get_state 同口径：选区文本带回（截断防爆载荷）。
        let (text, truncated) = if chars <= SELECTION_TEXT_CAP {
            (text, false)
        } else {
            (text.chars().take(SELECTION_TEXT_CAP).collect(), true)
        };
        serde_json::json!({
            "start": point_json(start),
            "end": point_json(end),
            "chars": chars,
            "text": text,
            "text_truncated": truncated,
        })
    });
    Ok(serde_json::json!({
        "path": editor.file_path().map(|p| p.display().to_string()),
        "revision": editor.revision(),
        "selection": selection,  // 无选区 = null
    }))
}

fn cmd_scroll_to(
    shell: &mut ShellApp,
    cx: &mut Context<ShellApp>,
    line: usize,
) -> Result<serde_json::Value, String> {
    let Some(editor) = shell.editor().cloned() else {
        return Err("无活动文档（空态——先 open_file）".into());
    };
    let scrolled = editor.update(cx, |ed, cx| ed.scroll_to_line(line, cx));
    Ok(serde_json::json!({
        "scrolled_to": scrolled,
        "line_count": editor.read(cx).line_count(),
    }))
}

fn cmd_undo(shell: &mut ShellApp, cx: &mut Context<ShellApp>) -> Result<serde_json::Value, String> {
    let Some(editor) = shell.editor().cloned() else {
        return Err("无活动文档（空态——先 open_file）".into());
    };
    let undid = editor.update(cx, |ed, cx| {
        let did = ed.undo_edit();
        cx.notify();
        did
    });
    let editor = editor.read(cx);
    Ok(serde_json::json!({
        "undid": undid,
        "revision": editor.revision(),
    }))
}

fn cmd_redo(shell: &mut ShellApp, cx: &mut Context<ShellApp>) -> Result<serde_json::Value, String> {
    let Some(editor) = shell.editor().cloned() else {
        return Err("无活动文档（空态——先 open_file）".into());
    };
    let redid = editor.update(cx, |ed, cx| {
        let did = ed.redo_edit();
        cx.notify();
        did
    });
    let editor = editor.read(cx);
    Ok(serde_json::json!({
        "redid": redid,
        "revision": editor.revision(),
    }))
}

fn cmd_save(shell: &mut ShellApp, cx: &mut Context<ShellApp>) -> Result<serde_json::Value, String> {
    let Some(editor) = shell.editor().cloned() else {
        return Err("无活动文档（空态——先 open_file）".into());
    };
    let Some(path) = editor.read(cx).file_path().map(|p| p.to_path_buf()) else {
        return Err("未命名文档无路径——先 save_as 指定目标".into());
    };
    // #386：RPC 保存与 UI（Ctrl+S/关闭确认/退出）同源走 Shell 分流
    //（`save_tab`——pdf 批注增量落盘在此发生）。旧「只读一律拒绝」
    // 是批注落盘的调用方缺口之一（rpc 保存 pdf = 批注丢失同形态）。
    // 三态消费：Saved/Unmodified 成功；ReadOnlyRejected/Failed 显式
    // Err 回给调用方（不静默）。
    let idx = shell.active_index();
    match shell.save_tab(idx, cx) {
        SaveOutcome::Saved | SaveOutcome::Unmodified => {}
        SaveOutcome::ReadOnlyRejected => {
            return Err("只读文档（渲染态）拒绝保存".into());
        }
        SaveOutcome::Failed => {
            return Err(format!(
                "保存失败（{}）——写盘错误（详见应用日志）",
                path.display()
            ));
        }
    }
    Ok(serde_json::json!({
        "saved": true,
        "path": path.display().to_string(),
        // #386：与 UI 同源（tab_is_modified——pdf 批注脏计入；保存
        // 成功后恒 false）。
        "modified": shell.tab_is_modified(idx, cx).unwrap_or(false),
    }))
}

fn cmd_save_as(
    shell: &mut ShellApp,
    cx: &mut Context<ShellApp>,
    path: &str,
) -> Result<serde_json::Value, String> {
    let Some(editor) = shell.editor().cloned() else {
        return Err("无活动文档（空态——先 open_file）".into());
    };
    if editor.read(cx).is_read_only() {
        return Err("只读文档（渲染态）不可另存".into());
    }
    // #275 沙箱门禁：save_as 此前可写任意绝对路径（审计笔记 23「全表
    // 最危险单命令」——任意路径写盘 + 无覆盖确认 + socket 无认证）。
    // 与 open_file 同批收口：无工作区拒、根外拒（新目标不存在时
    // resolve_sandboxed 经最近存在祖先解析，新建文件同样逃不出根）。
    // save 的 file_path 源头由此只剩「已门禁的 open_file/save_as」与
    // 「GUI 对话框（用户意图）」——S-2 写链整体闭合。GUI 另存对话框
    // 不经此命令，不受影响。
    let root = workspace_root_of(shell, cx)?;
    let path = resolve_sandboxed(&root, Path::new(path))?;
    // 走 ShellApp::save_active_as 完整链（editor.save_as + TabItem.path
    // 迁移 + 文件树 active 同步 + notify）——直调 editor.save_as 会漏
    // tab 身份迁移（tab_index_by_path 按旧路径查找失败——#111 烟雾
    // 实测踩到）。
    shell.save_active_as(path.clone(), cx);
    // 成功判定：标签身份已迁移到新路径（失败路径 save_active_as 早退
    // 不改 tab path）。
    match shell.tab_index_by_path(&path) {
        Some(idx) => Ok(serde_json::json!({
            "saved": true,
            "path": path.display().to_string(),
            "tab_index": idx,
        })),
        None => Err(format!(
            "另存失败（{}）——写盘错误（目录不存在/无权限？）",
            path.display()
        )),
    }
}

// ---------------------------------------------------------------------------
// #134 任务 1：工作区沙箱（纯函数面——单测主战场）
// ---------------------------------------------------------------------------

/// 把 AI 给的路径解析进工作区沙箱（canonicalize 解析后检查前缀——
/// issue #134 安全要求）。
///
/// - 相对路径相对工作区根（AI 的 `"src/main.rs"` = 根下 src/main.rs）
/// - 绝对路径必须落在根内（`Path::starts_with` 按组件比较——`/ws/a`
///   不是 `/ws/ab` 的前缀，不会误放行）
/// - 目标存在 → 整条 canonicalize（符号链接/`..` 全摊平）；不存在
///   （create_file / write_file 新目标）→ canonicalize 最近存在的祖
///   先再拼回不存在的尾段——逃逸链接在祖先解析时即被摊平拒掉
/// - 根外一律 `Err`（消息含目标与根——AI 可诊断自救）
///
/// TOCTOU 窄窗（检查与实际 IO 之间路径被换）本机单用户场景 + AI 有
/// 责任的前提下接受（#134 明确边界：本机、不做认证；fd-based 检查
/// 留待网络化时升级）。
pub fn resolve_sandboxed(root: &Path, target: &Path) -> Result<PathBuf, String> {
    let joined = if target.is_absolute() {
        target.to_path_buf()
    } else {
        root.join(target)
    };
    let canon_root = root
        .canonicalize()
        .map_err(|e| format!("工作区根不可解析（{}）：{e}", root.display()))?;
    let resolved = match joined.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            // 目标不存在：canonicalize 最近存在的祖先，拼回尾段。
            let mut existing = joined.clone();
            let mut tail: Vec<std::ffi::OsString> = Vec::new();
            while !existing.exists() {
                let Some(name) = existing.file_name().map(|n| n.to_os_string()) else {
                    return Err(format!(
                        "路径不可解析（越过文件系统根）：{}",
                        joined.display()
                    ));
                };
                tail.push(name);
                let Some(parent) = existing.parent().map(|p| p.to_path_buf()) else {
                    return Err(format!("路径不可解析（无父目录）：{}", joined.display()));
                };
                existing = parent;
            }
            let mut p = existing
                .canonicalize()
                .map_err(|e| format!("路径解析失败（{}）：{e}", existing.display()))?;
            for name in tail.iter().rev() {
                p.push(name);
            }
            p
        }
    };
    if resolved.starts_with(&canon_root) {
        Ok(resolved)
    } else {
        Err(format!(
            "路径越出工作区边界（目标 {}，根 {}）——FS 命令只允许工作区内操作",
            resolved.display(),
            canon_root.display()
        ))
    }
}

/// 沙箱根获取（无工作区 = 全部 FS 命令拒绝——「无边界」比「错误的
/// 边界」更危险）。
fn workspace_root_of(shell: &ShellApp, cx: &Context<ShellApp>) -> Result<PathBuf, String> {
    shell.workspace_root(cx).ok_or_else(|| {
        "无打开的工作区——FS 命令限定工作区根目录内（Ctrl+K Ctrl+O 打开文件夹）".into()
    })
}

/// 按 canonical 路径找已打开标签（`tab.path` 是打开时的绝对路径，未必
/// 摊平符号链接——磁盘权威面对齐用 canonical 比较）。
fn open_tab_by_canonical(shell: &ShellApp, target: &Path) -> Option<usize> {
    (0..shell.tab_count()).find(|&i| {
        shell
            .tab_path(i)
            .is_some_and(|p| p.canonicalize().is_ok_and(|c| c.as_path() == target))
    })
}

/// 收集「路径下」（自身或子孙）的已打开标签索引。
fn tabs_under_path(shell: &ShellApp, root: &Path) -> Vec<usize> {
    (0..shell.tab_count())
        .filter(|&i| {
            shell
                .tab_path(i)
                .and_then(|p| p.canonicalize().ok())
                .is_some_and(|cp| cp.starts_with(root))
        })
        .collect()
}

/// `list_dir` 条目上限（防一次递归把响应撑爆；超限 `truncated=true`）。
const LIST_DIR_MAX_ENTRIES: usize = 5000;

/// `read_file` 载荷上限（字节；超限截断到字符边界 + `truncated=true`）。
const READ_FILE_CAP_BYTES: usize = 1024 * 1024;

// ---------------------------------------------------------------------------
// #134 任务 1：文件系统命令（主线程薄编排——磁盘权威面）
// ---------------------------------------------------------------------------

fn cmd_list_dir(
    shell: &mut ShellApp,
    cx: &mut Context<ShellApp>,
    path: &str,
    recursive: bool,
) -> Result<serde_json::Value, String> {
    let root = workspace_root_of(shell, cx)?;
    let dir = resolve_sandboxed(&root, Path::new(path))?;
    if !dir.is_dir() {
        return Err(format!("不是目录（{}）", dir.display()));
    }
    let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut entries = Vec::new();
    let mut truncated = false;
    if recursive {
        walk_dir_recursive(&dir, &canon_root, &mut entries, &mut truncated);
    } else {
        let rd =
            std::fs::read_dir(&dir).map_err(|e| format!("列目录失败（{}）：{e}", dir.display()))?;
        let mut items: Vec<_> = rd.flatten().collect();
        items.sort_by_key(|e| e.file_name());
        for entry in items {
            if entries.len() >= LIST_DIR_MAX_ENTRIES {
                truncated = true;
                break;
            }
            entries.push(dir_entry_json(&entry, &canon_root));
        }
    }
    let count = entries.len();
    Ok(serde_json::json!({
        "path": dir.display().to_string(),
        "entries": entries,
        "count": count,
        "truncated": truncated,
    }))
}

/// 递归列目录（先序——目录条目紧跟其子项；读不了的子目录静默跳过：
/// 单个坏子目录不炸整次列举）。
fn walk_dir_recursive(
    dir: &Path,
    canon_root: &Path,
    out: &mut Vec<serde_json::Value>,
    truncated: &mut bool,
) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut items: Vec<_> = rd.flatten().collect();
    items.sort_by_key(|e| e.file_name());
    for entry in items {
        if out.len() >= LIST_DIR_MAX_ENTRIES {
            *truncated = true;
            return;
        }
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        out.push(dir_entry_json(&entry, canon_root));
        if is_dir {
            walk_dir_recursive(&entry.path(), canon_root, out, truncated);
        }
    }
}

/// 单条目录项的 JSON 形态（`rel` 相对工作区根——AI 上下文省字；
/// canonical 后不在根内时 `rel` 为 null）。
fn dir_entry_json(entry: &std::fs::DirEntry, canon_root: &Path) -> serde_json::Value {
    let name = entry.file_name().to_string_lossy().into_owned();
    let path = entry.path();
    let (is_dir, size) = match entry.metadata() {
        Ok(meta) => (meta.is_dir(), if meta.is_dir() { 0 } else { meta.len() }),
        Err(err) => {
            return serde_json::json!({
                "name": name, "path": path.display().to_string(),
                "error": format!("metadata 失败：{err}"),
            });
        }
    };
    serde_json::json!({
        "name": name,
        "path": path.display().to_string(),
        "rel": path.strip_prefix(canon_root).ok().map(|p| p.display().to_string()),
        "is_dir": is_dir,
        "size": size,
    })
}

/// #111 G1 写回半：`read_file` 载荷解码核心（纯函数——与 document::open
/// 同一探测口径；browse 面不再是「UTF-8 专属」：GBK/UTF-16/BOM 文档
/// 可读。p4tr 报告 §6 双读路径边界备忘的同步面）。探测不出 = Err——
/// 二进制/未知编码边界不猜（与 detect_encoding 诚实边界一致）。
///
/// 截断语义：探测解码先于截断（字节面截断会把 GBK 双字节/UTF-16
/// 单元劈半，残字节探测不出反而误伤）；截断落在解码后文本的
/// UTF-8 字节面上（JSON 载荷按字节计），多字节字符边界退让。
/// 返回 `(text, truncated)`；Err 为面向用户的原因（不含路径——调用方拼）。
fn decode_read_payload(bytes: &[u8], cap: usize) -> Result<(String, bool), String> {
    let (text, _) = crate::status_info::decode_text(bytes).ok_or_else(|| {
        "无法判定文本编码（非 UTF-8/GBK/UTF-16）——二进制文件不支持读取".to_string()
    })?;
    let truncated = text.len() > cap;
    let mut end = text.len().min(cap);
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    Ok((text[..end].to_string(), truncated))
}

fn cmd_read_file(
    shell: &mut ShellApp,
    cx: &mut Context<ShellApp>,
    path: &str,
) -> Result<serde_json::Value, String> {
    let root = workspace_root_of(shell, cx)?;
    let path = resolve_sandboxed(&root, Path::new(path))?;
    let bytes = std::fs::read(&path).map_err(|e| format!("读取失败（{}）：{e}", path.display()))?;
    let total = bytes.len();
    let (text, truncated) = decode_read_payload(&bytes, READ_FILE_CAP_BYTES)
        .map_err(|e| format!("{e}（{}）", path.display()))?;
    Ok(serde_json::json!({
        "path": path.display().to_string(),
        "size": total,
        "chars": text.chars().count(),
        "text": text,
        "truncated": truncated,
    }))
}

fn cmd_write_file(
    shell: &mut ShellApp,
    cx: &mut Context<ShellApp>,
    path: &str,
    content: &str,
) -> Result<serde_json::Value, String> {
    let root = workspace_root_of(shell, cx)?;
    let path = resolve_sandboxed(&root, Path::new(path))?;
    // 已打开标签：渲染态/行模型态拒（UI 与盘脱节），脏标签拒（静默
    // 覆盖丢用户修改——AI 有责任先 save）。
    let open_idx = open_tab_by_canonical(shell, &path);
    if let Some(idx) = open_idx {
        if shell.tab_is_readonly(idx, cx).unwrap_or(false) {
            return Err(format!(
                "文件以只读渲染态打开（{}）——先 close_file 再写",
                path.display()
            ));
        }
        if !shell.tab_can_reload(idx, cx) {
            return Err(format!(
                "文件以 docx 行模型态打开（{}）——行模型与磁盘字节不同构，先 close_file 再写",
                path.display()
            ));
        }
        if shell.tab_is_modified(idx, cx).unwrap_or(false) {
            return Err(format!(
                "文件有未保存修改（{}）——先 save 再覆盖写",
                path.display()
            ));
        }
    }
    // 父目录必须存在（新建目录走 create_dir——不隐式 mkdir，AI 的
    // 操作结果可预期）。
    if let Some(parent) = path.parent()
        && !parent.is_dir()
    {
        return Err(format!(
            "父目录不存在（{}）——先 create_dir",
            parent.display()
        ));
    }
    std::fs::write(&path, content).map_err(|e| format!("写入失败（{}）：{e}", path.display()))?;
    // 已打开的干净文本标签：reload 同步（走完整编辑链——订阅端也
    // 会收到这次变更的 doc_changed 事件，双向回显不破）。
    let synced = open_idx
        .map(|idx| shell.reload_tab(idx, cx))
        .unwrap_or(false);
    Ok(serde_json::json!({
        "written": true,
        "path": path.display().to_string(),
        "bytes": content.len(),
        "synced_open_tab": synced,
    }))
}

fn cmd_create_file(
    shell: &mut ShellApp,
    cx: &mut Context<ShellApp>,
    path: &str,
    content: Option<&str>,
) -> Result<serde_json::Value, String> {
    let root = workspace_root_of(shell, cx)?;
    let path = resolve_sandboxed(&root, Path::new(path))?;
    if path.exists() {
        return Err(format!(
            "文件已存在（{}）——覆盖请用 write_file",
            path.display()
        ));
    }
    if let Some(parent) = path.parent()
        && !parent.is_dir()
    {
        return Err(format!(
            "父目录不存在（{}）——先 create_dir",
            parent.display()
        ));
    }
    let content = content.unwrap_or("");
    std::fs::write(&path, content).map_err(|e| format!("创建失败（{}）：{e}", path.display()))?;
    Ok(serde_json::json!({
        "created": true,
        "path": path.display().to_string(),
        "bytes": content.len(),
    }))
}

fn cmd_create_dir(
    shell: &mut ShellApp,
    cx: &mut Context<ShellApp>,
    path: &str,
) -> Result<serde_json::Value, String> {
    let root = workspace_root_of(shell, cx)?;
    let path = resolve_sandboxed(&root, Path::new(path))?;
    if path.exists() {
        return Err(format!("路径已存在（{}）", path.display()));
    }
    // 单层 create_dir（父必须存在）——隐式建多层会让 delete/rename
    // 的路径假设失效；AI 需要 -p 语义就逐层 create_dir。
    std::fs::create_dir(&path).map_err(|e| format!("创建失败（{}）：{e}", path.display()))?;
    Ok(serde_json::json!({
        "created": true,
        "path": path.display().to_string(),
    }))
}

fn cmd_delete(
    shell: &mut ShellApp,
    window: &mut Window,
    cx: &mut Context<ShellApp>,
    path: &str,
) -> Result<serde_json::Value, String> {
    let root = workspace_root_of(shell, cx)?;
    let target = resolve_sandboxed(&root, Path::new(path))?;
    // 根自身拒绝（AI 一行命令蒸发整个工作区——防呆）。
    if root
        .canonicalize()
        .is_ok_and(|canon_root| target == canon_root)
    {
        return Err("拒绝删除工作区根目录自身".into());
    }
    if !target.exists() {
        return Err(format!("目标不存在（{}）", target.display()));
    }
    let was_dir = target.is_dir();
    // 路径下有脏标签 → 拒（连用户未保存的编辑一起删掉——AI 有责任
    // 先处理）；干净标签删后自动关（文件已不存在，标签留着无意义）。
    let under = tabs_under_path(shell, &target);
    for &idx in &under {
        if shell.tab_is_modified(idx, cx).unwrap_or(false) {
            let tab_path = shell.tab_path(idx).unwrap_or_default();
            return Err(format!(
                "路径下有未保存的标签（{}）——先保存或关闭再删除",
                tab_path.display()
            ));
        }
    }
    if was_dir {
        std::fs::remove_dir_all(&target)
            .map_err(|e| format!("删除目录失败（{}）：{e}", target.display()))?;
    } else {
        std::fs::remove_file(&target)
            .map_err(|e| format!("删除文件失败（{}）：{e}", target.display()))?;
    }
    let closed = under.len();
    for &idx in under.iter().rev() {
        shell.close_tab_no_confirm(idx, window, cx);
    }
    Ok(serde_json::json!({
        "deleted": true,
        "path": target.display().to_string(),
        "was_dir": was_dir,
        "closed_tabs": closed,
    }))
}

fn cmd_rename(
    shell: &mut ShellApp,
    cx: &mut Context<ShellApp>,
    from: &str,
    to: &str,
) -> Result<serde_json::Value, String> {
    let root = workspace_root_of(shell, cx)?;
    let from = resolve_sandboxed(&root, Path::new(from))?;
    let to = resolve_sandboxed(&root, Path::new(to))?;
    if !from.exists() {
        return Err(format!("源不存在（{}）", from.display()));
    }
    if to.exists() {
        return Err(format!("目标已存在（{}）——rename 不覆盖", to.display()));
    }
    if let Some(parent) = to.parent()
        && !parent.is_dir()
    {
        return Err(format!(
            "目标父目录不存在（{}）——先 create_dir",
            parent.display()
        ));
    }
    // 迁移计划在 rename **前**采集（旧路径还能 canonicalize 时）；
    // 文件/目录（from 是目录时整棵子树）下的标签全部跟随。rest 为空
    // = 标签正是 from 自身（join("") 会带尾分隔符——直接用 to）。
    let plan: Vec<(usize, PathBuf)> = (0..shell.tab_count())
        .filter_map(|idx| {
            let canon = shell.tab_path(idx)?.canonicalize().ok()?;
            let rest = canon.strip_prefix(&from).ok()?.to_path_buf();
            let new_path = if rest.as_os_str().is_empty() {
                to.to_path_buf()
            } else {
                to.join(rest)
            };
            Some((idx, new_path))
        })
        .collect();
    std::fs::rename(&from, &to)
        .map_err(|e| format!("改名失败（{} → {}）：{e}", from.display(), to.display()))?;
    let migrated = shell.apply_tab_renames(&plan, cx);
    Ok(serde_json::json!({
        "renamed": true,
        "from": from.display().to_string(),
        "to": to.display().to_string(),
        "migrated_tabs": migrated,
    }))
}

// ---------------------------------------------------------------------------
// #134 任务 2：编辑器命令补完（主线程薄编排——Cmd 注册表/大纲/格式）
// ---------------------------------------------------------------------------

fn cmd_run_command(
    shell: &mut ShellApp,
    window: &mut Window,
    cx: &mut Context<ShellApp>,
    name: &str,
) -> Result<serde_json::Value, String> {
    let Some(cmd) = crate::editor::commands::Cmd::from_id(name) else {
        return Err(format!("未知命令（{name}）——list_commands 查看可用清单"));
    };
    if shell.editor().is_none() {
        return Err("无活动文档（空态——先 open_file）".into());
    }
    // 与命令面板/键位**真同一条**分派路径（ShellApp::dispatch_editor_cmd
    // → Editor::run_command）。#386 前 rpc 在此直调 `Editor::run_command`
    // ——`run_command("save")` 绕过 Shell 保存分流（批注落盘被跳过，
    // 与面板/键位不同语义）；现经 Shell 派发与 UI 处处同源（只读门
    // 禁/折叠/查找全同语义——exec_command 内部各命令自带门）。
    shell.dispatch_editor_cmd(cmd, window, cx);
    Ok(serde_json::json!({
        "ran": true,
        "command": name,
    }))
}

fn cmd_list_commands() -> Result<serde_json::Value, String> {
    let commands: Vec<serde_json::Value> = crate::editor::commands::Cmd::ALL
        .iter()
        .map(|&c| {
            serde_json::json!({
                "id": c.id(),
                "title": c.title(),
                "default_key": c.default_key(),
            })
        })
        .collect();
    let count = commands.len();
    Ok(serde_json::json!({
        "commands": commands,
        "count": count,
    }))
}

fn cmd_get_outline(
    shell: &mut ShellApp,
    cx: &mut Context<ShellApp>,
) -> Result<serde_json::Value, String> {
    let Some(editor) = shell.editor().cloned() else {
        return Err("无活动文档（空态——先 open_file）".into());
    };
    let editor = editor.read(cx);
    let Some(entries) = editor.md_outline() else {
        return Err(format!(
            "文档无 md 大纲（kind={}）——get_outline 只对文本态文档可用",
            editor.doc_kind()
        ));
    };
    let items: Vec<serde_json::Value> = entries
        .into_iter()
        .map(
            |(level, line, text)| serde_json::json!({ "level": level, "line": line, "text": text }),
        )
        .collect();
    let count = items.len();
    Ok(serde_json::json!({
        "entries": items,
        "count": count,
    }))
}

fn cmd_apply_format(
    shell: &mut ShellApp,
    cx: &mut Context<ShellApp>,
    start: PointWire,
    end: PointWire,
    format: crate::editor::FormatKind,
) -> Result<serde_json::Value, String> {
    let Some(editor) = shell.editor().cloned() else {
        return Err("无活动文档（空态——先 open_file）".into());
    };
    match editor.update(cx, |ed, cx| {
        ed.apply_format(start.into_point(), end.into_point(), format, cx)
    }) {
        Some(cursor) => Ok(serde_json::json!({
            "applied": true,
            "format": format.id(),
            "cursor": point_json(cursor),
            "chars": editor.read(cx).char_count(),
        })),
        None => Err("无法应用格式（空范围 / 只读渲染态 / docx 行模型态）".into()),
    }
}

// ---------------------------------------------------------------------------
// #134 任务 3：批量替换（纯函数 collect 层——单测主战场）
// ---------------------------------------------------------------------------

/// 单个命中（行列/长度均 char 口径——与 Buffer 的 Point 语义一致；
/// `repl` 已展开：正则的 $1 捕获引用在收集时即展开）。
#[derive(Clone, Debug, PartialEq, Eq)]
struct ReplaceHit {
    line: usize,
    col: usize,
    len: usize,
    repl: String,
}

/// 字面模式命中收集（非重叠——`str::find` 循环推进同口径；命中
/// 不跨行：按行喂入）。空模式拒绝。
fn collect_literal_hits(
    lines: &[String],
    pattern: &str,
    replacement: &str,
) -> Result<Vec<ReplaceHit>, String> {
    if pattern.is_empty() {
        return Err("空模式拒绝——pattern 必须非空".into());
    }
    let mut out = Vec::new();
    for (li, line) in lines.iter().enumerate() {
        let mut from = 0;
        while let Some(off) = line[from..].find(pattern) {
            let byte_start = from + off;
            out.push(ReplaceHit {
                line: li,
                col: line[..byte_start].chars().count(),
                len: pattern.chars().count(),
                repl: replacement.to_string(),
            });
            from = byte_start + pattern.len();
        }
    }
    Ok(out)
}

/// 正则命中收集（每行 `captures_iter`；空匹配跳过——`a*` 型模式在
/// 每处空转，不产生有意义的编辑；`$1` 捕获引用用 `caps.expand` 展开）。
fn collect_regex_hits(lines: &[String], re: &regex::Regex, template: &str) -> Vec<ReplaceHit> {
    let mut out = Vec::new();
    for (li, line) in lines.iter().enumerate() {
        for caps in re.captures_iter(line) {
            let whole = caps.get(0).expect("组 0 恒在");
            if whole.as_str().is_empty() {
                continue;
            }
            let mut repl = String::new();
            caps.expand(template, &mut repl);
            out.push(ReplaceHit {
                line: li,
                col: line[..whole.start()].chars().count(),
                len: whole.as_str().chars().count(),
                repl,
            });
        }
    }
    out
}

/// 磁盘文本的行级替换（未打开文件的 disk 通道——**倒序应用**：命中
/// 在原文上收集，先替换行尾方向的命中使前序坐标不漂移；尾换行保
/// 真；命中不跨行使行级替换与全文替换等价）。
fn replace_text_lines(hits: &[ReplaceHit], lines: &mut [String]) {
    for h in hits.iter().rev() {
        let line = &mut lines[h.line];
        let start: usize = line
            .char_indices()
            .nth(h.col)
            .map(|(b, _)| b)
            .unwrap_or(line.len());
        let end: usize = line[start..]
            .char_indices()
            .nth(h.len)
            .map(|(b, _)| start + b)
            .unwrap_or(line.len());
        line.replace_range(start..end, &h.repl);
    }
}

/// `batch_replace` 文件数上限（防一次命令扫穿整个仓库把响应撑爆）。
const BATCH_MAX_FILES: usize = 200;

/// `batch_replace` 单文件命中上限（超出即该文件报错——批量操作要么
/// 全做要么明确失败，不静默截断）。
const BATCH_MAX_HITS_PER_FILE: usize = 10_000;

// ---------------------------------------------------------------------------
// #134 任务 3：批量替换（主线程薄编排——buffer/disk 双通道）
// ---------------------------------------------------------------------------

fn cmd_batch_replace(
    shell: &mut ShellApp,
    cx: &mut Context<ShellApp>,
    files: &[String],
    pattern: &str,
    replacement: &str,
    is_regex: bool,
) -> Result<serde_json::Value, String> {
    if files.is_empty() {
        return Err("文件清单为空".into());
    }
    if files.len() > BATCH_MAX_FILES {
        return Err(format!(
            "文件数超上限（{} > {BATCH_MAX_FILES}）——分批提交",
            files.len()
        ));
    }
    let root = workspace_root_of(shell, cx)?;
    let re = if is_regex {
        Some(regex::Regex::new(pattern).map_err(|e| format!("正则编译失败：{e}"))?)
    } else {
        None
    };
    if !is_regex && pattern.is_empty() {
        return Err("空模式拒绝——pattern 必须非空".into());
    }

    let mut file_results: Vec<serde_json::Value> = Vec::with_capacity(files.len());
    let mut total = 0usize;
    let mut files_changed = 0usize;
    for file in files {
        let (count, mut v) =
            batch_replace_one(shell, cx, &root, file, pattern, replacement, re.as_ref());
        if count > 0 {
            total += count;
            files_changed += 1;
        } else {
            // 统一补 skipped 标记（count=0 且无 error = 无命中）
            if v.get("error").is_none() {
                v["note"] = serde_json::json!("无命中");
            }
        }
        file_results.push(v);
    }
    Ok(serde_json::json!({
        "total_replacements": total,
        "files_changed": files_changed,
        "files": file_results,
        // undo 语义：via=buffer 的文件一次 undo 撤该文件全批；via=disk
        // 直接改盘不可 undo（AI 可先 open 全部文件再 batch 获得整批可撤销）。
        "undo_hint": "已打开文件（via=buffer）一条 undo 撤该文件全批；磁盘直改（via=disk）不可 undo",
    }))
}

/// 单文件替换（cmd_batch_replace 的子步骤）。返回 `(命中数, 结果 JSON)`。
fn batch_replace_one(
    shell: &mut ShellApp,
    cx: &mut Context<ShellApp>,
    root: &Path,
    file: &str,
    pattern: &str,
    replacement: &str,
    re: Option<&regex::Regex>,
) -> (usize, serde_json::Value) {
    let path = match resolve_sandboxed(root, Path::new(file)) {
        Ok(p) => p,
        Err(err) => return (0, batch_file_error(file, &err)),
    };
    // 已打开 → buffer 通道（一条 undo + 事件流；标签即使非活动也能改）
    if let Some(idx) = open_tab_by_canonical(shell, &path) {
        if !shell.tab_can_reload(idx, cx) {
            return (
                0,
                batch_file_error(file, "只读渲染态 / docx 行模型态——先 close_file 再批量替换"),
            );
        }
        let Some(editor) = shell.tab_editor(idx).cloned() else {
            return (0, batch_file_error(file, "标签实体不可达"));
        };
        // 收集命中（buffer 行 → 命中集 → Point 编辑集）
        let hits = editor.update(cx, |ed, _| {
            let lines: Vec<String> = (0..ed.line_count()).map(|i| ed.buffer_line(i)).collect();
            match re {
                Some(re) => collect_regex_hits(&lines, re, replacement),
                None => collect_literal_hits(&lines, pattern, replacement).unwrap_or_default(),
            }
        });
        if hits.len() > BATCH_MAX_HITS_PER_FILE {
            return (
                0,
                batch_file_error(
                    file,
                    &format!(
                        "命中数超上限（{} > {BATCH_MAX_HITS_PER_FILE}）——拒绝执行",
                        hits.len()
                    ),
                ),
            );
        }
        let edits: Vec<(Point, Point, String)> = hits
            .iter()
            .map(|h| {
                (
                    Point::new(h.line, h.col),
                    Point::new(h.line, h.col + h.len),
                    h.repl.clone(),
                )
            })
            .collect();
        let applied = editor.update(cx, |ed, cx| ed.apply_batch_replacements(&edits, cx));
        return (
            applied,
            serde_json::json!({
                "path": path.display().to_string(),
                "count": applied,
                "via": "buffer",
            }),
        );
    }
    // 未打开 → disk 通道（读盘 → 行级替换 → 有变更写回；不可 undo）
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(err) => return (0, batch_file_error(file, &format!("读取失败：{err}"))),
    };
    let text = match String::from_utf8(bytes) {
        Ok(t) => t,
        Err(_) => return (0, batch_file_error(file, "非 UTF-8 文本——不支持批量替换")),
    };
    let trailing_newline = text.ends_with('\n');
    let body = text.strip_suffix('\n').unwrap_or(&text);
    let mut lines: Vec<String> = body.split('\n').map(str::to_string).collect();
    let hits = match re {
        Some(re) => collect_regex_hits(&lines, re, replacement),
        None => match collect_literal_hits(&lines, pattern, replacement) {
            Ok(h) => h,
            Err(err) => return (0, batch_file_error(file, &err)),
        },
    };
    if hits.len() > BATCH_MAX_HITS_PER_FILE {
        return (
            0,
            batch_file_error(
                file,
                &format!(
                    "命中数超上限（{} > {BATCH_MAX_HITS_PER_FILE}）——拒绝执行",
                    hits.len()
                ),
            ),
        );
    }
    if hits.is_empty() {
        return (
            0,
            serde_json::json!({
                "path": path.display().to_string(),
                "count": 0,
                "via": "disk",
            }),
        );
    }
    replace_text_lines(&hits, &mut lines);
    let mut new_text = lines.join("\n");
    if trailing_newline {
        new_text.push('\n');
    }
    if let Err(err) = std::fs::write(&path, &new_text) {
        return (0, batch_file_error(file, &format!("写回失败：{err}")));
    }
    (
        hits.len(),
        serde_json::json!({
            "path": path.display().to_string(),
            "count": hits.len(),
            "via": "disk",
        }),
    )
}

/// 单文件错误结果（沙箱拒绝/IO/形态拒绝——count=0 + error）。
fn batch_file_error(file: &str, err: &str) -> serde_json::Value {
    serde_json::json!({
        "path": file,
        "count": 0,
        "via": "skipped",
        "error": err,
    })
}

/// 全量状态（get_state 的 result 载荷）。
fn state_json(shell: &ShellApp, cx: &Context<ShellApp>) -> serde_json::Value {
    let tabs: Vec<serde_json::Value> = (0..shell.tab_count())
        .map(|i| {
            serde_json::json!({
                "path": shell.tab_path(i).map(|p| p.display().to_string()),
                "modified": shell.tab_is_modified(i, cx).unwrap_or(false),
                "readonly": shell.tab_is_readonly(i, cx).unwrap_or(false),
                "active": i == shell.active_index(),
            })
        })
        .collect();
    let active = shell.editor().cloned().map(|ed| {
        let editor = ed.read(cx);
        let selection = editor.selection_points().map(|(start, end)| {
            let text = editor.selection_text();
            let chars = text.chars().count();
            // 选区文本带回（AI 消费主通道）；超长截断防爆载荷。
            let (text, truncated) = if chars <= SELECTION_TEXT_CAP {
                (text, false)
            } else {
                (text.chars().take(SELECTION_TEXT_CAP).collect(), true)
            };
            serde_json::json!({
                "start": point_json(start),
                "end": point_json(end),
                "chars": chars,
                "text": text,
                "text_truncated": truncated,
            })
        });
        Some(serde_json::json!({
            "path": editor.file_path().map(|p| p.display().to_string()),
            "name": editor.display_name(),
            "kind": editor.doc_kind(),
            "readonly": editor.is_read_only(),
            "modified": editor.is_modified(),
            "cursor": point_json(editor.cursor_point()),
            "selection": selection,
            "line_count": editor.line_count(),
            "char_count": editor.char_count(),
        }))
    });
    serde_json::json!({
        "tabs": tabs,
        "active_index": shell.active_index(),
        "active": active,
    })
}

/// get_state 选区文本带回上限（字符）。
const SELECTION_TEXT_CAP: usize = 2000;

fn point_json(p: Point) -> serde_json::Value {
    serde_json::json!({ "line": p.line, "col": p.col })
}

// ---------------------------------------------------------------------------
// 单测（纯函数面——协议/端点/分帧/探活；执行面由 rpc_smoke.sh 端到端）
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;
