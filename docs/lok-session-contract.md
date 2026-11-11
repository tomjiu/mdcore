# LOK 文档会话契约（B2 · #49 工作线 B → #50 C 线）

> 状态：**已实测基线**（LibreOffice 25.2.3.2 / Debian，2026-11-10，
> `crates/lok/tests/session_contract.rs` 真实运行时门）。本文是 C 线（GPUI
> 页面接入 #50）消费 `mdc-lok` 的**唯一契约参考**；API 以真实 LOK 能力
> 为准——每个契约点都有对应测试断言（标注 §N）。

## 1. 会话形态与所有权

| 类型 | 用途 | 所有权 |
|---|---|---|
| `DocSession` | **产品接入形态**（C 线用这个） | 拥有 worker 子进程；无生命周期参数；`Send`（可跨线程 move / 放进 GPUI Entity） |
| `DocHandle<'a>` | 借用式（smoke / 内部测试） | 借 `&WorkerClient`；自引用——**C 线不要用** |

- **一文档一会话一进程**（单文档 worker 契约）；多文档 = 多会话并行
  （独立 user profile，不互锁——双会话实测 §9）。
- `DocSession::open(runtime, session_root, url, options)` 一步到位：
  spawn（发现序自动定位运行时）→ load → init_rendering。首启含 LO
  hook_2 可达数秒（`DEFAULT_TIMEOUT=120s`）。

## 2. 线程与阻塞契约（C 线必读）

- 方法**同步阻塞**（毫秒级；`open` 数秒）。**不在 UI 线程调用**——
  GPUI 宿主在后台任务线程调用，UI 线程只消费事件流。
- 事件通道一次性：`take_events()` 首次返回 `Receiver<Event>`，之后
  `try_recv_event` / `recv_event_timeout` 继续可用（内部暂存转发）。
- worker 死亡（kill/崩溃）后：`is_dead()=true`，所有请求明确
  `LokError::WorkerDied`（含 stderr 尾部）；宿主可 `open` 新会话恢复
  （源文件不受影响 §B1）。

## 3. 文档打开 / 关闭

| 操作 | 契约 | 测试 |
|---|---|---|
| `open(url, None)` | url=file:// 绝对路径；返回即已可 size/paint | §1 |
| `close()` | 逻辑关闭（后续 doc 请求 `E_NO_DOCUMENT`）；**不调 LO destroy**（25.2 teardown abort 规避） | smoke |
| `shutdown()` | close + kill；**落盘只认 saveAs**（内存态编辑随进程消亡——C 线关闭交互必须先引导保存） | §11 |

## 4. 几何与 tile

- `size() -> (width_twips, height_twips)`（§2）。
- `page_geometry() -> Vec<PageRect>`（B 线 2026-11-11，#50 C 线跳页/
  页码/页间分隔的协议缺口应答）：LOK `getPartPageRectangles` → 每页一条
  `{x, y, w, h}`、顺序即页序、**画布坐标系 twips（与 paintTile 同源）**。
  25.2.3.2 实测语义：画布 = 页 + 四周 **284 twips 边距**、页间隙同为
  **284**——单页 `(284, 284, w, h)`，第 i 页 `y = 284 + (页高+284)×i`，
  画布总高 = 末页底 + 284（与 `size()` 自洽；A4 = 11906×16838）。
  **页数 = `pages.len()`**（派生；内核 `getPagesCount` 在 #19+ 未实证
  vtable 槽，不扩绑——ABI 防漂移纪律）。跳页目标 y 直接取 `pages[n].y`；
  页码视口映射 = 包含视口顶的首个页。**跨版本哨兵**：LO 26.x 若改边距/
  页隙，`tests/page_geometry.rs` 三页契约会亮黄牌，须重实证再放行。
- `paint(canvas_w_px, canvas_h_px, tile_x, tile_y, tile_w_twips, tile_h_twips)`：
  **tile 尺寸是文档 twips（1px = 15twips）**——canvas 是像素目标、tile
  是文档坐标窗口；返回 JSON：`tile`（side-file BMP 路径）、`tile_mode`
  （实测 1=BGRA）、`stats.{unique_colors, non_white_ratio, colored_ratio}`
  （§3：非白 >0.5% 证明真实渲染）。
- **tile 失效**：`INVALIDATE_TILES` 回调（Event::Callback name=
  `INVALIDATE_TILES`，§10）；C 线收到即重画对应区域。文档尺寸变化后
  （`DOCUMENT_SIZE_CHANGED`）页几何需随 `size()` 同拍重查（与 C2 的
  失效链同拍——页矩形源自布局，布局变即变）。

## 5. dirty / revision

- `dirty`：编辑类成功置位（`type_text` / `uno` 变更类）；**仅 `save_as`
  成功清位**；超时/死亡/保存失败一律保留。
- `revision`（宿主侧单调计数，初始 0）：每次成功编辑 +1、`save_as`
  成功 +1。C 线用它丢弃过期 tile。**缺口**（登记 #50）：worker 侧回调
  流尚不携带 revision——B4 计划下沉到 worker 并随事件回传。

## 6. 键鼠 / UNO / 选区

- `type_text(&mut self, &str)`：逐字符 KEYINPUT+KEYUP，**选区被替换**
  （标准编辑语义；先 `uno(".uno:SelectAll")` 再输入 = 全文替换）。
  **这是产品 IME 定稿通道**（2026-11-11 实证，tests/ime_text.rs）：
  GPUI 宿主渲染 preedit（A 线批次 12 架构），定稿串经 type_text 下发，
  中文全链（落盘/重开）实测绿。
- `post_text(&mut self, commit, text)`：LOK 槽 #40 内核 preedit 通道
  （机械面可用：帧到达/守卫生效/编辑类语义正确）——**stock 25.2.3.2
  语义不可靠，产品勿用**（见 §10.6 实证记录）。
- `uno(&mut self, command, args)`：变更类（**dirty+rev 推进**）——
  实测可用：`.uno:Undo` / `.uno:Redo` / `.uno:SelectAll`（§4-§7）。
- `uno_query(&self, ...)`：查询类（不置脏）：`.uno:ExecuteSearch` 等。
- `text_selection("text/plain;charset=utf-8") -> (mime, text)`：
  **mime 必须带 charset**（裸 `text/plain` 返回空）。Writer 表格内
  SelectAll 语义：第 1 次选单元格 → 第 2 次选表 → 第 3 次全文
  （循环重试到含期望标记，§7）。
- 鼠标（`post_mouse`）经 `WorkerClient::request` 通路可用（smoke
  `mouse.*` 步在案）；`DocSession` 快捷方法 B4 按需补。

## 7. 保存

- `save_as(&mut self, url, format)`：目标**不得等于源文件**（worker
  硬守卫 `E_INVALID_REQUEST`）；成功 dirty=false、revision+1；失败
  （拒写/非法目录）dirty 保留（smoke §save.denied_keeps_dirty）。
- **安全链**（C 线职责）：saveAs 到临时产物 → `DocSession::open` 重开
  校验 → 决定替换原文件。worker 永不覆盖源。

## 8. 事件（回调）流

```rust
Event::Callback { type_id, name, payload, truncated }  // LOK 回调原文
Event::Life { kind, detail }                            // hello/ready/boot-failed/bye
```

回调名见 `abi::callbacks`（INVALIDATE_TILES=0 / STATE_CHANGED=8 /
UNO_COMMAND_RESULT=16 / SEARCH_RESULT_SELECTION / …）。回调在 LO 内核
线程触发、写线程单点串行化（帧不交错）。

## 9. 已实测契约点清单（session_contract.rs，2026-11-10）

| § | 契约 | 断言 |
|---|---|---|
| 1 | open 一步到位 | 返回会话可用，dirty=false rev=0 |
| 2 | size | twips 非零 |
| 3 | paint | stats.non_white_ratio > 0.5% |
| 4 | type_text | dirty=true，revision=1 |
| 5 | .uno:Undo | 真实撤销生效（rev=2） |
| 6 | .uno:Redo | 重做恢复（undo→redo 语义闭环） |
| 7 | SelectAll+text_selection | 全文含编辑文本 |
| 8 | save_as | dirty=false、rev+1、产物落盘 |
| 9 | 双会话并行 | 独立 profile 不互锁 |
| 10 | 事件通道 | 编辑后 INVALIDATE_TILES 到达（20s 窗口） |
| 11 | shutdown | 不 panic、产物保留 |

## 10. 不稳定 / 未验证面（如实声明，C 线接线前必读）

1. **`initializeForRendering(args)` 传参**：25.2 实测
   `"DeviceFormFactor=desktop"` 触发内核 Application Error（worker
   abort——stderr `Unspecified Application Error` + poll 回调不可
   unwind）。**契约默认不传参**（空 args 实测稳定）。参数格式按 LO
   版本有分叉，需要时先真实运行时验证再启用。
2. **Windows 实机门未执行**（B5）：运行时发现序 Windows 段 / DLL 依赖
   解析 / `_dup` fd 抢救 / 中文渲染 / 故障三件套——代码在、验证缺。
3. **revision 不随回调流回传**（B4 计划）；**命令状态查询**
   （getCommandValues 等 vtable 槽 >#18 未绑定）不可用——UNO 命令
   enabled/disabled 需 B4 评估。
4. **postMouseEvent 的 DocSession 快捷方法**未提供（经
   `WorkerClient::request("post_mouse", …)` 通路可用）。
5. IME / 组合输入：postKeyEvent 通路对 IME 组合不适用（GPUI 侧组词
   完成后整体 type_text，或 B4 评估 setTextSelection+paste 通路）。
   **2026-11-11 实证更新**：type_text 通道的中文定稿全链绿
   （tests/ime_text.rs）；内核 preedit 通道（post_text）不可靠见 6。
6. **post_text（槽 #40 postWindowExtTextInputEvent）语义不可靠**
   （2026-11-11 六流探针 + 会话序数实验，LibreOffice 25.2.3.2）：
   - 「进程首个 LOK 会话」：TEXTINPUT 组合串被当**定稿文本**插入
     （preedit 语义破缺），END 载荷被忽略；
   - 「后续会话」：事件被**静默丢弃**（零效果）；
   - 单帧 END 带文本（ely 形态）/ 空 END / ASCII vs CJK 均不改变上述
     行为（行为由会话序数决定，非帧序列）；
   - ElyOffice ely-lok 的可靠 postText 依赖其补丁版 LO（#115 比对
     报告 §3 patches/ 发现）——stock 内核不可复现。
   结论：`post_text` 保留为机械面（op/守卫/编辑类语义已验证）；
   产品 IME = type_text 通道（§6）。LO 升级或补丁运行时到位后
   重实证，恢复 tests/ime_text.rs 的文本落地断言。

## 11. 版本与来源

- ABI 钉定：LibreOffice/core `0d51fdc57aef…`（头文件四件，vtable
  append-only 槽位 + 运行时 nSize 守卫——缺槽明确报错不猜）。
- 实测运行时：Debian LibreOffice 25.2.3.2 BuildId 520(Build:2)，
  kernel `libmergedlo.so` SHA-256 `c6bcb62f2fe8bbf0eeabce1b284a2bee
  3056e0890dfffeef9e190d569817b86d`。
- 事件循环：unipoll（`SAL_LOK_OPTIONS=unipoll` + worker 主线程
  runLoop——B1，gates/633.md）。
