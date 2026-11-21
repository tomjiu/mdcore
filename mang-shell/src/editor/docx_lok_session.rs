//! #50 C1：DOCX LOK 会话——宿主侧异步包装（线程演员模型）。
//!
//! **定位**：mang-shell 消费 `mdc-lok` 公开 client API 的唯一会话层。
//! GPUI 零依赖（lib 树可单测）；`crates/lok` 私有域一律不触碰（B 线
//! 私有，#49 契约）。
//!
//! **线程纪律**（#50 C1 门：GPUI 线程不等待同步 IPC）：
//! [`DocxLokSession::open_blocking`] 在**后台线程**完成整条打开链——
//! `discover → spawn worker → 等 ready → version → load →
//! init_rendering → size`（照 crates/lok/src/smoke.rs 同款序列，不猜造
//! 步骤）；此后会话线程持有 `WorkerClient`，宿主经 mpsc 提交命令、经
//! mpsc 收回执（16ms 轮询 drain——pdf_pages 搜索索引/OCR 同款纪律），
//! UI 线程永不做阻塞调用。
//!
//! **会话所有权**：`DocHandle<'a>` 借用 `WorkerClient`——会话结构体
//! 不能同时拥有两者（自引用禁）。打开链结束后句柄即弃（worker 进程
//! 侧文档仍在，宿主侧后续 op 走 `WorkerClient::request` 原始 wire op
//! ——`ping/size/paint/close`），dirty 由宿主镜像（C1 只读渲染恒
//! `false`；C3/C4 编辑/保存接入后同一位承接）。
//!
//! **互斥铁律**（#50）：LOK 会话在场时，同一文件的行投影编辑器必须
//! 干净（壳层 `toggle_docx_view_mode` 脏拒切保证）——单一编辑真源，
//! 会话与行投影永不同时编辑同一文件。
//!
//! **tile 载荷**：worker `paint` 把 twips 区域缩放渲染进画布并写 BMP
//! side file（BITMAPV4HEADER + BI_BITFIELDS 32 位 BGRA、行序自下而上
//! ——crates/lok/src/bmp.rs 同源格式）；会话线程读文件并解码为
//! **自上而下 BGRA**（gpui `RenderImage` 字节序直配），回执直接携带
//! 像素缓冲——UI 线程零文件 IO。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use mdc_lok::client::{DocSession, Event, LokError};
use mdc_lok::profile;
use mdc_lok::runtime;

// twips↔px 换算常量 `TWIPS_PER_PX`：C2 起单源于 `crate::docx_geom`
//（消费方 docx_pages 直连导入；本模块不再 re-export——零剩余消费面）。

/// worker boot 等待（DocSession 契约：load 请求内含就绪等待，
/// DEFAULT_TIMEOUT=120s 内置——本层不再独立等 boot 事件）。
/// 会话线程命令等待切片（期间顺带 drain 事件）。
const CMD_TICK: Duration = Duration::from_millis(100);

/// 会话目录父级（一次性会话目录 `<tmp>/mdc-docx-lok/<序号>`）。
fn session_root_for(seq: u64) -> PathBuf {
    std::env::temp_dir()
        .join("mdc-docx-lok")
        .join(format!("{}-{seq}", std::process::id()))
}

/// `LokError` → 会话阶段名（打开链口径：spawn / load 两段；discover
/// 由本层先行、died 由事件流上报——`boot` 段随 DocSession 契约
///（load 请求内含就绪等待，超时 120s 内置）不再独立出现）。
fn stage_of_open_error(e: &LokError) -> &'static str {
    match e {
        LokError::WorkerDied { .. } => "spawn",
        LokError::Protocol(m) if m.contains("spawn") || m.contains("可执行") => "spawn",
        _ => "load",
    }
}

/// 打开/运行期错误。`NoRuntime` 是「内核缺失」判据（宿主回落文本
/// 视图的口径——C1 门：内核缺失时明确静态阅读后备）。
#[derive(Debug, Clone, PartialEq)]
pub enum SessionError {
    /// LibreOffice 运行时未发现（discover 失败原文）。
    NoRuntime(String),
    /// worker 可执行/spawn 失败。
    Spawn(String),
    /// 180s 内未见 ready（或 boot-failed 事件）。
    Boot(String),
    /// documentLoadWithOptions / init_rendering / size 失败。
    Load(String),
    /// 就绪后的 op 失败（paint/ping/close）。
    Op { op: &'static str, message: String },
    /// worker 进程死亡（paint 中途/事件 bye）。
    Died { detail: String },
}

impl SessionError {
    /// 稳定阶段名（回执 `Failed.stage` 同源；宿主按段分流提示）。
    pub fn stage(&self) -> &'static str {
        match self {
            Self::NoRuntime(_) => "discover",
            Self::Spawn(_) => "spawn",
            Self::Boot(_) => "boot",
            Self::Load(_) => "load",
            Self::Op { op, .. } => *op,
            Self::Died { .. } => "died",
        }
    }

    /// 是否为「内核缺失」（宿主回落文本视图的判据）。
    pub fn is_no_runtime(&self) -> bool {
        matches!(self, Self::NoRuntime(_))
    }

    /// 用户可读信息。
    pub fn message(&self) -> &str {
        match self {
            Self::NoRuntime(m) | Self::Spawn(m) | Self::Boot(m) | Self::Load(m) => m,
            Self::Op { message, .. } => message,
            Self::Died { detail } => detail,
        }
    }
}

/// tile 渲染统计回执（worker `paint` 的 `stats` 面——真实像素证据，
/// 冒烟断言同源：`unique_colors >= 8 && non_white_ratio > 0.005`）。
#[derive(Debug, Clone, PartialEq)]
pub struct TileStats {
    pub unique_colors: u64,
    pub non_white_ratio: f64,
    pub colored_ratio: f64,
}

/// 会话线程 → 宿主回执（16ms drain；一次一条）。
#[derive(Debug)]
pub enum SessionReply {
    /// 打开链完成：文档几何（twips）+ 内核指纹 + 全链耗时。
    /// （C2 迁移 DocSession 契约后无 version 暴露面——版本证据以内核
    /// SHA256 为准，B 线运行时文档另有 version_info 记录。)
    Ready {
        width_twips: i64,
        height_twips: i64,
        kernel_sha256: String,
        elapsed_ms: u128,
    },
    /// 打开链失败（`stage` 见 [`SessionError::stage`]）。
    Failed {
        stage: &'static str,
        message: String,
    },
    /// tile 就绪：自上而下 BGRA 像素（`w*h*4` 字节）。
    Tile {
        seq: u64,
        width: usize,
        height: usize,
        bgra: Arc<Vec<u8>>,
        sha256: String,
        stats: TileStats,
        elapsed_ms: u128,
    },
    /// 内核回调转发（INVALIDATE_TILES / DOCUMENT_SIZE_CHANGED /
    /// STATE_CHANGED 等——C2 起视图层按名消费缓存失效/换尺寸，其余
    /// 登记；C3 接命令状态面）。
    Callback { name: String, payload: String },
    /// 文档几何重查回执（`SessionCmd::Size`——DOCUMENT_SIZE_CHANGED
    /// 时机重查；twips 单一真源 = `DocSession::size`）。
    Size { width_twips: i64, height_twips: i64 },
    /// #50 C5：页几何回执（按页序页矩形——画布坐标 twips；跳页
    /// tile 锚定与页码呈现的数据源）。
    PageGeometry {
        seq: u64,
        /// 每页 `(x, y, w, h)`（twips，画布坐标系）。
        pages: Vec<(i64, i64, i64, i64)>,
    },
    /// #50 C3：编辑命令完成（TypeText/Uno——回执带内核侧 dirty/revision，
    /// 宿主据此实时更新标签脏态 + INVALIDATE 后的 repaint 由内核回调
    /// 通道异步到——不在此复合格）。
    EditDone {
        seq: u64,
        ok: bool,
        dirty: bool,
        revision: u64,
        error: Option<String>,
    },
    /// #50 C3：剪贴板读回执（`text_selection` 的文本面——宿主写
    /// pasteboard；空串 = 无选区）。
    TextDone {
        seq: u64,
        text: String,
        error: Option<String>,
    },
    /// #50 C4：保存完成（`revision_saved` = 保存时内核修订号——宿主
    /// 对账当前 revision 相等才清 dirty）。
    SaveDone {
        seq: u64,
        ok: bool,
        revision_saved: u64,
        error: Option<String>,
    },
    /// worker 死亡（回执侧终态——宿主转错误态并停轮询）。
    Died { detail: String },
}

/// 宿主 → 会话线程命令。
enum SessionCmd {
    /// tile 渲染：`(canvas_w, canvas_h, tile_x, tile_y, tile_w, tile_h)`
    ///（画布 px + 区域 twips——`DocSession::paint` 契约）。
    Paint {
        seq: u64,
        canvas_w: usize,
        canvas_h: usize,
        tile_x: i32,
        tile_y: i32,
        tile_w: i32,
        tile_h: i32,
    },
    /// 文档几何重查（DOCUMENT_SIZE_CHANGED 时机——C2 缓存换尺寸链）。
    Size,
    /// #50 C5：页几何查询（`getPartPageRectangles`——跳页/翻页渲染的
    /// **页矩形坐标**单一真源。`getDocumentSize` 返回的是画布全局
    /// 尺寸（页 + 284 twips 边距 + 页隙），把它当单页尺寸用会把多页
    /// 文档压进一帧——C5 表格跨页样本实证的坐标病灶；页矩形
    /// （`PageRect.x/y`，画布坐标系、与 paintTile 同源）才是页级
    /// tile 的正确锚）。
    PageGeometry {
        seq: u64,
    },
    /// #50 C3：文本输入（逐字符 post_key + 内核置脏位——选区被替换
    /// 的标准编辑语义；IME 定稿同通道——契约 §10 宿主 preedit 口径）。
    TypeText {
        seq: u64,
        text: String,
    },
    /// #50 C3：UNO 命令（编辑类 `uno`——置脏位；查询类经调用方选
    /// `uno_query` 面时 notify=false 由线程侧区分）。
    Uno {
        seq: u64,
        command: String,
        args: Option<String>,
        query: bool,
    },
    /// #50 C3：剪贴板读——`text_selection(mime)` 取选区文本（copy/cut
    /// 的读面；cut 随后发 Uno `.uno:Cut`）。
    TextOf {
        seq: u64,
        mime: String,
    },
    /// #50 C4：保存（saveAs 回源路径 + 修订号绑定——回执 revision_saved
    /// 与宿主当前 revision 对账后才清 dirty，防延迟回执清掉新编辑）。
    SaveAs {
        seq: u64,
        url: String,
        format: String,
    },
    Close,
}

/// #179：会话回执通道三分面（[`DocxLokSession::try_recv_detailed`]）。
#[derive(Debug)]
pub enum SessionRecv {
    /// 一条回执。
    Reply(SessionReply),
    /// 暂时无消息（16ms 拍内正常态）。
    Empty,
    /// 通道断开（会话线程已退出）。若此前未见 `Failed`/`Died` 终态
    /// 回执 = 会话线程异常退出（panic/被杀）——消费方据此落可定位
    /// 失败，不再与「暂时无消息」同貌。
    Disconnected,
}

static SEQ: AtomicU64 = AtomicU64::new(1);

/// DOCX LOK 会话句柄（宿主侧；线程演员包装）。
///
/// Drop 即请求关闭（幂等）：会话线程 `shutdown` worker 后退出；宿主
/// 停止 drain 即回收。**不阻塞**——Drop 不 join 线程（后台 100ms 切片
/// 内自行收尾；worker 被 client Drop 兜底击杀）。
pub struct DocxLokSession {
    path: PathBuf,
    session_dir: PathBuf,
    cmd_tx: Sender<SessionCmd>,
    reply_rx: Receiver<SessionReply>,
    closed: bool,
}

impl std::fmt::Debug for DocxLokSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocxLokSession")
            .field("path", &self.path)
            .field("session_dir", &self.session_dir)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl DocxLokSession {
    /// 阻塞打开（**只许在非 UI 线程调用**——打开链含 worker boot 可达
    /// 数十秒）。worker 可执行定位交 `DocSession::open` 契约（同目录
    /// `mdc-lok-worker`——部署布局；测试经预部署仿真，见
    /// [`Self::open_blocking`] 测试注释）。
    pub fn open_blocking(path: &Path) -> Result<Self, SessionError> {
        let path = std::fs::canonicalize(path)
            .map_err(|e| SessionError::Load(format!("文档路径无效（{}）：{e}", path.display())))?;
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let session_dir = session_root_for(seq);

        let (cmd_tx, cmd_rx) = channel::<SessionCmd>();
        let (reply_tx, reply_rx) = channel::<SessionReply>();
        let thread_path = path.clone();
        let thread_dir = session_dir.clone();
        std::thread::Builder::new()
            .name(format!("docx-lok-session-{seq}"))
            .spawn(move || {
                session_main(thread_path, thread_dir, cmd_rx, reply_tx);
            })
            .map_err(|e| SessionError::Spawn(format!("会话线程启动失败：{e}")))?;

        Ok(Self {
            path,
            session_dir,
            cmd_tx,
            reply_rx,
            closed: false,
        })
    }

    /// 源文件绝对路径（标签路径绑定单一真源）。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 一次性会话目录（tile side file 落点）。
    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }

    /// 提交 tile 渲染（twips 区域缩放进 `canvas_w × canvas_h` 画布）。
    /// 返回自增 `seq`；结果经 [`Self::try_recv`] 以 [`SessionReply::Tile`]
    /// 回收（`seq` 对账——换代竞态的迟到 tile 机械丢弃）。
    pub fn request_tile(
        &self,
        canvas_w: usize,
        canvas_h: usize,
        tile_x: i32,
        tile_y: i32,
        tile_w: i32,
        tile_h: i32,
    ) -> u64 {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let _ = self.cmd_tx.send(SessionCmd::Paint {
            seq,
            canvas_w,
            canvas_h,
            tile_x,
            tile_y,
            tile_w,
            tile_h,
        });
        seq
    }

    /// #50 C3：文本输入（异步——回执 `EditDone`；逐字符 post_key 由
    /// 会话线程执行，内核置脏位）。`seq` 调用方分配（回执对账）。
    pub fn request_type_text(&self, seq: u64, text: &str) {
        let _ = self.cmd_tx.send(SessionCmd::TypeText {
            seq,
            text: text.to_owned(),
        });
    }

    /// #50 C3：UNO 命令（异步——回执 `EditDone`；编辑类置脏位；
    /// `query=true` 查询类 notify=false 不置脏）。
    pub fn request_uno(&self, seq: u64, command: &str, args: Option<&str>, query: bool) {
        let _ = self.cmd_tx.send(SessionCmd::Uno {
            seq,
            command: command.to_owned(),
            args: args.map(str::to_owned),
            query,
        });
    }

    /// #50 C3：剪贴板读（异步——回执 `TextDone`；copy/cut 的选区文本面）。
    pub fn request_text_of(&self, seq: u64, mime: &str) {
        let _ = self.cmd_tx.send(SessionCmd::TextOf {
            seq,
            mime: mime.to_owned(),
        });
    }

    /// #50 C4：保存（异步——回执 `SaveDone{revision_saved}`；宿主对账
    /// 当前 revision 相等才清 dirty——防延迟回执清掉新编辑）。
    pub fn request_save_as(&self, seq: u64, url: &str, format: &str) {
        let _ = self.cmd_tx.send(SessionCmd::SaveAs {
            seq,
            url: url.to_owned(),
            format: format.to_owned(),
        });
    }

    /// 文档几何重查（DOCUMENT_SIZE_CHANGED 时机；回执
    /// [`SessionReply::Size`]——twips 单一真源）。
    pub fn request_size(&self) {
        let _ = self.cmd_tx.send(SessionCmd::Size);
    }

    /// #50 C5：页几何查询（`getPartPageRectangles`→按页序矩形；返回
    /// 自增 `seq`，回执 [`SessionReply::PageGeometry`] 对账——多页跳页
    /// tile 的正确坐标源，见 [`SessionCmd::PageGeometry`] 病灶注释）。
    pub fn request_page_geometry(&self) -> u64 {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let _ = self.cmd_tx.send(SessionCmd::PageGeometry { seq });
        seq
    }

    /// #179：三分面收回执——通道断开与「暂时无消息」分流。旧
    /// `try_recv` 两者同貌返回 `None`：会话线程异常退出（无终态回执）
    /// 的通道断开无从诊断，视图永挂 Starting/Ready。
    pub fn try_recv_detailed(&self) -> SessionRecv {
        match self.reply_rx.try_recv() {
            Ok(reply) => SessionRecv::Reply(reply),
            Err(TryRecvError::Empty) => SessionRecv::Empty,
            Err(TryRecvError::Disconnected) => SessionRecv::Disconnected,
        }
    }

    /// #186 测试构造器：无 worker 线程的哑会话（cmd 通道无消费者——
    /// request_tile 只 send 不回执；reply_rx 恒空）。用于显示链回归
    /// （视口探针→派单→看门狗态机）——真实打开链由
    /// `docx_lok_session::real_runtime_*` 与 bad_path 面锁定。
    #[cfg(test)]
    pub fn dummy_for_test(path: &Path) -> Self {
        use std::sync::mpsc::channel;
        let (cmd_tx, _cmd_rx) = channel::<SessionCmd>();
        let (_reply_tx, reply_rx) = channel::<SessionReply>();
        Self {
            path: path.to_path_buf(),
            session_dir: std::env::temp_dir(),
            cmd_tx,
            reply_rx,
            closed: false,
        }
    }

    /// 主动关闭（幂等；Drop 同径）。
    pub fn request_close(&mut self) {
        if !self.closed {
            self.closed = true;
            let _ = self.cmd_tx.send(SessionCmd::Close);
        }
    }

    /// #179 受控时序测试面：由现成回执通道构造（会话线程缺位——回执
    /// 由测试驱动；命令通道接收端即弃，`request_*` 静默无操作）。仅
    /// 测试编译可见。
    #[cfg(test)]
    pub fn from_reply_channel_for_test(reply_rx: Receiver<SessionReply>) -> Self {
        Self {
            path: PathBuf::from("test://179-controlled-timing.docx"),
            session_dir: std::env::temp_dir().join("mdc-docx-lok-test"),
            cmd_tx: channel::<SessionCmd>().0,
            reply_rx,
            closed: false,
        }
    }
}

impl Drop for DocxLokSession {
    fn drop(&mut self) {
        self.request_close();
    }
}

/// 会话线程主体（一次生命周期；全链错误走 `Failed` 回执后退出）。
///
/// **C2 契约迁移**：打开链 = 本层 `runtime::discover`（NoRuntime 分类
/// + 内核指纹）→ `DocSession::open`（spawn + load + init_rendering——
/// profile worker 侧自建；boot 等待内含于 load 请求，超时 120s 内置）
/// → `size`。服务循环按 [`SessionCmd`] 消费 `DocSession` 同步方法
///（paint/size/close——B2 契约 §2/§4；dirty/revision C3/C4 接管）。
fn session_main(
    path: PathBuf,
    session_dir: PathBuf,
    cmd_rx: Receiver<SessionCmd>,
    reply_tx: Sender<SessionReply>,
) {
    let fail = |stage: &'static str, message: String| {
        let _ = reply_tx.send(SessionReply::Failed { stage, message });
    };
    let started = Instant::now();

    // ---- 0. 运行时发现（NoRuntime → 宿主回落文本视图判据；内核
    // SHA256 = 就绪回执的来源指纹——B3 版本钉定验收口径）。
    let rt = match runtime::discover() {
        Ok(rt) => rt,
        Err(e) => return fail("discover", e),
    };

    // ---- 1. DocSession::open（spawn → load → init_rendering——B2 契约
    // 产品会话形态；C 线不再触碰借用式 DocHandle）。
    let url = profile::file_url_of(&path);
    let opened = DocSession::open(
        Some(&rt),
        &session_dir,
        &url,
        Some("DeviceFormFactor=desktop"),
    );
    // #50 C3：编辑命令族（uno 编辑类 &mut 面）——服务循环臂需要可变。
    let mut session = match opened {
        Ok(s) => s,
        Err(e) => return fail(stage_of_open_error(&e), e.to_string()),
    };
    let events = session.take_events();

    // ---- 2. 文档几何（就绪门：尺寸非零）。
    let (w_tw, h_tw) = match session.size() {
        Ok(size) => size,
        Err(e) => return fail("load", format!("文档尺寸获取失败：{e}")),
    };

    if reply_tx
        .send(SessionReply::Ready {
            width_twips: w_tw,
            height_twips: h_tw,
            kernel_sha256: rt.kernel_sha256.clone(),
            elapsed_ms: started.elapsed().as_millis(),
        })
        .is_err()
    {
        return; // 宿主已走（视图被 drop）——直接收尾。
    }
    eprintln!(
        "docx-lok: ready url={url} size={w_tw}x{h_tw}tw pid={} total={}ms",
        session.pid(),
        started.elapsed().as_millis(),
    );

    // ---- 5. 服务循环（命令 + 事件双 drain）----
    loop {
        match cmd_rx.recv_timeout(CMD_TICK) {
            Ok(SessionCmd::Paint {
                seq,
                canvas_w,
                canvas_h,
                tile_x,
                tile_y,
                tile_w,
                tile_h,
            }) => {
                let paint_started = Instant::now();
                let result = session.paint(canvas_w, canvas_h, tile_x, tile_y, tile_w, tile_h);
                match result {
                    Ok(v) => match tile_reply(&v, &session_dir) {
                        Ok(Some((bgra, width, height, sha256, stats))) => {
                            let _ = reply_tx.send(SessionReply::Tile {
                                seq,
                                width,
                                height,
                                bgra: Arc::new(bgra),
                                sha256,
                                stats,
                                elapsed_ms: paint_started.elapsed().as_millis(),
                            });
                        }
                        Ok(None) => {
                            let _ = reply_tx.send(SessionReply::Failed {
                                stage: "paint",
                                message: format!("tile 载荷异常（canvas {canvas_w}x{canvas_h}）"),
                            });
                        }
                        Err(message) => {
                            let _ = reply_tx.send(SessionReply::Failed {
                                stage: "paint",
                                message,
                            });
                        }
                    },
                    Err(e) => {
                        let dead = session.is_dead();
                        let message = if dead {
                            format!("worker 死亡：{e}")
                        } else {
                            e.to_string()
                        };
                        let _ = reply_tx.send(SessionReply::Failed {
                            stage: "paint",
                            message,
                        });
                        if dead {
                            let _ = reply_tx.send(SessionReply::Died {
                                detail: session.death_detail(),
                            });
                            return;
                        }
                    }
                }
            }
            Ok(SessionCmd::Size) => {
                // `DocSession::size`（B2 契约 §4——geometry 单一真源）。
                match session.size() {
                    Ok((w, h)) => {
                        let _ = reply_tx.send(SessionReply::Size {
                            width_twips: w,
                            height_twips: h,
                        });
                    }
                    Err(e) => {
                        let _ = reply_tx.send(SessionReply::Failed {
                            stage: "size",
                            message: e.to_string(),
                        });
                    }
                }
            }
            Ok(SessionCmd::PageGeometry { seq }) => {
                // `DocSession::page_geometry`（B2 契约 §5——页矩形按页序，
                // 画布坐标系与 paintTile 同源）。
                match session.page_geometry() {
                    Ok(pages) => {
                        let rects: Vec<(i64, i64, i64, i64)> =
                            pages.iter().map(|p| (p.x, p.y, p.w, p.h)).collect();
                        let _ = reply_tx.send(SessionReply::PageGeometry { seq, pages: rects });
                    }
                    Err(e) => {
                        let _ = reply_tx.send(SessionReply::Failed {
                            stage: "page_geometry",
                            message: e.to_string(),
                        });
                    }
                }
            }
            Ok(SessionCmd::TypeText { seq, text }) => {
                // 契约 §10 口径：产品 IME 通道 = 宿主 preedit 渲染 +
                // 定稿/普通键入统一逐字符 post_key（type_text——成功即
                // 内核置脏位/revision+1）。
                let snapshot = (session.is_dirty(), session.revision());
                let result = session.type_text(&text);
                let _ = reply_tx.send(edit_done_of(seq, result, snapshot));
            }
            Ok(SessionCmd::Uno {
                seq,
                command,
                args,
                query,
            }) => {
                // 查询类（SelectAll/ExecuteSearch 等）notify=false 不置
                // 脏；编辑类（Undo/Redo/Delete/Cut/Paste/格式命令）走
                // `uno` 置脏位。
                // DocSession 契约：编辑类 `uno`（&mut——edit_ok 置脏
                // 位）/查询类 `uno_query`（&——不置脏）。
                let snapshot = (session.is_dirty(), session.revision());
                let result = if query {
                    session.uno_query(&command, args.as_deref())
                } else {
                    session.uno(&command, args.as_deref())
                };
                // Value 载荷不透传宿主（回执只需 ok/dirty/rev——Value
                // 是 result envelope，错误面由 Err 携带）。
                let result = result.map(|_| ());
                let _ = reply_tx.send(edit_done_of(seq, result, snapshot));
            }
            Ok(SessionCmd::TextOf { seq, mime }) => {
                // 剪贴板读面：`text_selection(mime)` 的 (text, _)——
                // 取文本面（html 面后续批次需要时再扩 mime 传参）。
                let result = session.text_selection(&mime).map(|(text, _)| text);
                let (text, error) = match result {
                    Ok(t) => (t, None),
                    Err(e) => (String::new(), Some(e.to_string())),
                };
                let _ = reply_tx.send(SessionReply::TextDone { seq, text, error });
            }
            Ok(SessionCmd::SaveAs { seq, url, format }) => {
                // C4：saveAs 回源路径——revision_saved 钉保存时刻内核
                // 修订号（宿主对账清 dirty 的唯一判据）。
                let result = session.save_as(&url, &format);
                let _ = reply_tx.send(SessionReply::SaveDone {
                    seq,
                    ok: result.is_ok(),
                    revision_saved: session.revision(),
                    error: result.err().map(|e| e.to_string()),
                });
            }
            Ok(SessionCmd::Close) => {
                session.shutdown();
                return;
            }
            Err(RecvTimeoutError::Timeout) => {} // 切片到点 → 顺带 drain 事件
            Err(RecvTimeoutError::Disconnected) => {
                // 宿主已走（视图 drop）——关 worker 收尾。
                session.shutdown();
                return;
            }
        }

        // 事件 drain（非阻塞）：内核回调转发 + 生命终态。
        if let Some(ev_rx) = events.as_ref() {
            while let Ok(ev) = ev_rx.try_recv() {
                match ev {
                    Event::Callback { name, payload, .. } => {
                        let _ = reply_tx.send(SessionReply::Callback { name, payload });
                    }
                    Event::Life { kind, detail } => {
                        if kind == "bye" {
                            let _ = reply_tx.send(SessionReply::Died {
                                detail: format!("worker 退出：{detail}"),
                            });
                            return;
                        }
                    }
                }
            }
        }
        if session.is_dead() {
            let _ = reply_tx.send(SessionReply::Died {
                detail: session.death_detail(),
            });
            return;
        }
    }
}

/// #50 C3：编辑命令回执装配——快照 = **命令执行前**的 (dirty,
/// revision)（保守序：失败回执不虚报成功编辑；成功时真实值由内核在
/// 执行中自增——EditDone 的 revision 因此反映命令后的最新值，两序
/// 差一由宿主端对账语义吸收：保存对账用 SaveDone 的 revision_saved
/// 独立通道，不受此差影响）。
fn edit_done_of<E: std::fmt::Display>(
    seq: u64,
    result: Result<(), E>,
    (dirty_before, rev_before): (bool, u64),
) -> SessionReply {
    match result {
        Ok(()) => SessionReply::EditDone {
            seq,
            ok: true,
            // 成功编辑命令后的内核真值：`type_text`/编辑类 `uno` 成功
            // 即 dirty=true/revision+1——直接以「执行后语义」装配
            //（快照仅失败回执兜底用，避免再次请求内核）。
            dirty: true,
            revision: rev_before.saturating_add(1),
            error: None,
        },
        Err(e) => SessionReply::EditDone {
            seq,
            ok: false,
            dirty: dirty_before,
            revision: rev_before,
            error: Some(e.to_string()),
        },
    }
}

/// `paint` 回执 → 像素载荷：读 BMP side file + 解码为自上而下 BGRA。
///
/// `Ok(None)` = 尺寸/载荷不一致（哨兵异常——如实上报不 panic）。
fn tile_reply(
    v: &serde_json::Value,
    session_dir: &Path,
) -> Result<Option<(Vec<u8>, usize, usize, String, TileStats)>, String> {
    let tile_path = v
        .get("tile_path")
        .and_then(serde_json::Value::as_str)
        .ok_or("paint 回执缺 tile_path")?;
    let tile_path = std::path::absolute(session_dir.join(tile_path))
        .map_err(|e| format!("tile 路径无效：{e}"))?;
    let bytes = std::fs::read(&tile_path)
        .map_err(|e| format!("tile 文件读取失败（{}）：{e}", tile_path.display()))?;
    let (width, height, bgra) = decode_bmp_bgra(&bytes)?;
    if bgra.len() != width * height * 4 {
        return Ok(None);
    }
    let sha256 = v
        .get("sha256_bgra")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let stats = v.get("stats");
    let stats = TileStats {
        unique_colors: stats
            .and_then(|s| s.get("unique_colors"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        non_white_ratio: stats
            .and_then(|s| s.get("non_white_ratio"))
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0),
        colored_ratio: stats
            .and_then(|s| s.get("colored_ratio"))
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0),
    };
    Ok(Some((bgra, width, height, sha256, stats)))
}

/// 解码 worker BMP side file → 自上而下 BGRA（32 位、BI_BITFIELDS）。
///
/// 格式与 `mdc_lok::bmp::write_bmp` 同源：14 字节文件头 + BITMAPV4HEADER
///（108 字节）+ 自下而上行序。**纯函数**（lib 树可单测——含与
/// `write_bmp` 的回环对拍）。
pub fn decode_bmp_bgra(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    const FILE_HEADER: usize = 14;
    if bytes.len() < FILE_HEADER + 108 {
        return Err("BMP 过短（< V4 头）".into());
    }
    if &bytes[0..2] != b"BM" {
        return Err("BPM 魔数不符".into());
    }
    let pixel_offset = u32::from_le_bytes(bytes[10..14].try_into().unwrap()) as usize;
    let header_size = u32::from_le_bytes(bytes[14..18].try_into().unwrap()) as usize;
    if header_size < 108 {
        return Err(format!(
            "非 BITMAPV4HEADER（size={header_size}）——仅支持本 worker 格式"
        ));
    }
    let width = i32::from_le_bytes(bytes[18..22].try_into().unwrap());
    let height = i32::from_le_bytes(bytes[22..26].try_into().unwrap());
    let planes = u16::from_le_bytes(bytes[26..28].try_into().unwrap());
    let bpp = u16::from_le_bytes(bytes[28..30].try_into().unwrap());
    let compression = u32::from_le_bytes(bytes[30..34].try_into().unwrap());
    if width <= 0 || height <= 0 || planes != 1 || bpp != 32 || compression != 3 {
        return Err(format!(
            "BMP 参数不符（w={width} h={height} planes={planes} bpp={bpp} comp={compression}——仅支持 32 位 BI_BITFIELDS）"
        ));
    }
    let (width, height) = (width as usize, height as usize);
    let stride = width * 4; // 32 位恒 4 字节对齐
    let data_len = stride * height;
    if pixel_offset + data_len > bytes.len() {
        return Err(format!(
            "BMP 像素区不足（需 {data_len}，实 {}）",
            bytes.len().saturating_sub(pixel_offset)
        ));
    }
    let mut bgra = Vec::with_capacity(data_len);
    // 自下而上 → 自上而下（行序翻转；行内 BGRA 保序直搬）。
    for row in (0..height).rev() {
        let start = pixel_offset + row * stride;
        bgra.extend_from_slice(&bytes[start..start + stride]);
    }
    Ok((width, height, bgra))
}

// ---------------------------------------------------------------------------
// 测试面（lib 树；无运行时恒绿 + 有运行时真链）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 与 `mdc_lok::bmp::write_bmp` 回环对拍：写 → 读 → 行序/像素无损。
    #[test]
    fn bmp_roundtrip_with_writer() {
        // 2x2：左上红、右上绿、左下蓝、右下白（BGRA 字节序）。
        let px = |b: u8, g: u8, r: u8| vec![b, g, r, 255];
        let src = [
            px(0, 0, 255),
            px(0, 255, 0),
            px(255, 0, 0),
            px(255, 255, 255),
        ]
        .concat();
        let dir = std::env::temp_dir().join(format!("docx-lok-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tile.bmp");
        mdc_lok::bmp::write_bmp(&file, &src, 2, 2).unwrap();
        let bytes = std::fs::read(&file).unwrap();
        let (w, h, bgra) = decode_bmp_bgra(&bytes).unwrap();
        assert_eq!((w, h), (2, 2));
        // write_bmp 写入的是「自上而下源、行序翻转落盘」——解码后必须
        // 还原为自上而下原序。
        assert_eq!(bgra, src);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 退化输入逐条拒绝（哨兵路径不 panic）。
    #[test]
    fn bmp_rejects_garbage() {
        assert!(decode_bmp_bgra(&[]).is_err());
        assert!(decode_bmp_bgra(b"XX").is_err());
        let mut v4 = vec![b'B', b'M'];
        v4.extend_from_slice(&[0u8; 12]); // size/reserved
        v4.extend_from_slice(&122u32.to_le_bytes()); // pixel offset
        v4.extend_from_slice(&108u32.to_le_bytes()); // V4 size
        v4.extend_from_slice(&2i32.to_le_bytes());
        v4.extend_from_slice(&1i32.to_le_bytes()); // h=1
        v4.extend_from_slice(&1u16.to_le_bytes()); // planes
        v4.extend_from_slice(&16u16.to_le_bytes()); // bpp=16 → 拒
        v4.extend_from_slice(&3u32.to_le_bytes());
        v4.resize(122 + 64, 0xEE);
        assert!(decode_bmp_bgra(&v4).is_err(), "bpp=16 必须拒绝");
    }

    /// 像素区截断拒绝。
    #[test]
    fn bmp_rejects_truncated_pixels() {
        let mut v4 = vec![b'B', b'M'];
        v4.extend_from_slice(&[0u8; 12]);
        v4.extend_from_slice(&122u32.to_le_bytes());
        v4.extend_from_slice(&108u32.to_le_bytes());
        v4.extend_from_slice(&4i32.to_le_bytes()); // w=4
        v4.extend_from_slice(&4i32.to_le_bytes()); // h=4
        v4.extend_from_slice(&1u16.to_le_bytes());
        v4.extend_from_slice(&32u16.to_le_bytes());
        v4.extend_from_slice(&3u32.to_le_bytes());
        v4.resize(122 + 8, 0x00); // 只给 8 字节像素（需 64）
        assert!(decode_bmp_bgra(&v4).is_err());
    }

    /// 阶段名/判据口径。
    #[test]
    fn error_stage_taxonomy() {
        assert_eq!(SessionError::NoRuntime("x".into()).stage(), "discover");
        assert!(SessionError::NoRuntime("x".into()).is_no_runtime());
        assert!(!SessionError::Boot("y".into()).is_no_runtime());
        assert_eq!(SessionError::Boot("y".into()).stage(), "boot");
        assert_eq!(
            SessionError::Op {
                op: "paint",
                message: "z".into()
            }
            .stage(),
            "paint"
        );
    }

    /// 会话目录：进程内唯一、落 `<tmp>/mdc-docx-lok/`。
    #[test]
    fn session_root_shape() {
        let a = session_root_for(1);
        let b = session_root_for(2);
        assert_ne!(a, b);
        assert!(a.starts_with(std::env::temp_dir().join("mdc-docx-lok")));
    }

    /// **真实运行时链（有内核才跑；无内核 SKIP 横幅——不假绿）**：
    /// open_blocking → Ready（尺寸非零 + 内核指纹）→ request_tile →
    /// Tile（像素非空 + stats 达冒烟口径）→ request_size（DocSession 契约
    /// 重查）→ close。
    ///
    /// worker 预部署仿真（DocSession::open 契约 = 同目录 `mdc-lok-worker`，
    /// 不暴露注入参——测试把构建产物拷到本测试二进制同目录，与部署
    /// 布局一致；`MDC_LOK_WORKER_BIN` 仅作拷贝源提示，不给产品路径）。
    ///
    /// 运行方式（Linux 真实证据，工作区根）：
    /// ```text
    /// cargo build -p mdc-lok            # 先出 worker 二进制
    /// cargo test -p mang-shell --lib docx_lok_session -- --nocapture --test-threads=1
    /// ```
    #[test]
    fn real_runtime_open_paint_close() {
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.docx");
        assert!(fixture.is_file(), "黄金例必须进仓：{fixture:?}");
        // 原文保真（smoke.rs 同纪律）：复制到临时工作副本——黄金例零
        // 直开（LO 文档锁不落夹具目录、任何意外写回不毁黄金例）。
        let work =
            std::env::temp_dir().join(format!("mdc-docx-lok-test-{}.docx", std::process::id()));
        std::fs::copy(&fixture, &work).unwrap_or_else(|e| panic!("复制夹具失败：{e}"));
        let rt = match runtime::discover() {
            Ok(rt) => rt,
            Err(reason) => {
                eprintln!("SKIP（无 LibreOffice 运行时，真链未跑）：{reason}");
                return;
            }
        };
        eprintln!(
            "runtime: kernel={} sha256={}",
            rt.kernel_lib.display(),
            rt.kernel_sha256
        );
        // worker 预部署：拷到本测试二进制同目录（DocSession::open 同目录
        // 契约）。`MDC_LOK_WORKER_BIN` = 拷贝源（工作区根 target/debug）；
        // 未设则拷工作区根 target/debug/mdc-lok-worker（cargo 标准布局）。
        let test_exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .expect("定位测试二进制目录失败");
        let staged = test_exe_dir.join("mdc-lok-worker");
        let src = std::env::var_os("MDC_LOK_WORKER_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/debug/mdc-lok-worker")
            });
        if !src.is_file() {
            eprintln!("SKIP（worker 构建产物不在 {src:?}——先 `cargo build -p mdc-lok`，真链未跑）");
            return;
        }
        if staged != src
            && (!staged.is_file()
                || std::fs::metadata(&staged).ok().map(|m| m.len())
                    != std::fs::metadata(&src).ok().map(|m| m.len()))
        {
            std::fs::copy(&src, &staged)
                .unwrap_or_else(|e| panic!("worker 预部署拷贝失败（{src:?} → {staged:?}）：{e}"));
        }
        let mut session =
            DocxLokSession::open_blocking(&work).expect("打开链必须成功（运行时在场）");
        // 打开链回执必须先于任何 tile；Failed/Died 快速失败——不白等
        // 180s（C1 遗留缺陷：失败回执被吞导致超时 panic 丢真实原因）。
        let mut ready = None;
        let mut open_error: Option<(String, String)> = None;
        let deadline = Instant::now() + Duration::from_secs(180);
        while ready.is_none() && open_error.is_none() && Instant::now() < deadline {
            if let SessionRecv::Reply(SessionReply::Ready {
                width_twips,
                height_twips,
                kernel_sha256,
                elapsed_ms,
            }) = session.try_recv_detailed()
            {
                assert!(width_twips > 0 && height_twips > 0, "文档尺寸必须非零");
                assert!(!kernel_sha256.is_empty(), "内核指纹必须留痕");
                eprintln!(
                    "ready: {width_twips}x{height_twips}tw kernel={kernel_sha256} \
                     open_ms={elapsed_ms}"
                );
                ready = Some((width_twips, height_twips));
            }
            if let SessionRecv::Reply(SessionReply::Failed { stage, message }) =
                session.try_recv_detailed()
            {
                open_error = Some((stage.to_string(), message));
            }
            if let SessionRecv::Reply(SessionReply::Died { detail }) = session.try_recv_detailed() {
                open_error = Some(("died".into(), detail));
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        if let Some((stage, message)) = open_error {
            panic!("打开链失败（{stage}）：{message}");
        }
        let (w_tw, h_tw) = ready.expect("180s 内必须 Ready");
        // 首块 tile：整页缩放进 512px 画布（smoke.rs 同款口径断言）。
        let seq = session.request_tile(512, 512, 0, 0, w_tw as i32, h_tw as i32);
        let mut tile = None;
        let deadline = Instant::now() + Duration::from_secs(60);
        while tile.is_none() && Instant::now() < deadline {
            match session.try_recv_detailed() {
                SessionRecv::Reply(SessionReply::Tile {
                    seq: s,
                    width,
                    height,
                    bgra,
                    sha256,
                    stats,
                    elapsed_ms,
                }) => {
                    assert_eq!(s, seq, "seq 必须对账");
                    assert_eq!(width * height * 4, bgra.len(), "BGRA 载荷长度");
                    assert!(stats.unique_colors >= 8, "真实渲染证据：{stats:?}");
                    assert!(stats.non_white_ratio > 0.005, "非空白页证据：{stats:?}");
                    eprintln!(
                        "tile: {width}x{height}px sha={sha256} paint_ms={elapsed_ms} stats={stats:?}"
                    );
                    tile = Some(stats);
                }
                SessionRecv::Reply(SessionReply::Failed { stage, message }) => {
                    panic!("tile 失败（{stage}）：{message}");
                }
                SessionRecv::Reply(SessionReply::Died { detail }) => {
                    panic!("worker 死亡：{detail}");
                }
                // 其余回执（Ready/Size/Callback——此阶段迟到者）与通道
                // 两态：机械跳过，下一拍继续。
                SessionRecv::Reply(_) | SessionRecv::Empty | SessionRecv::Disconnected => {}
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        assert!(tile.is_some(), "60s 内必须收到 tile 回执");
        // ---- C2：文档几何重查（DOCUMENT_SIZE_CHANGED 时机同款 wire 面）----
        session.request_size();
        let mut size_reply = None;
        let mut seen_callbacks: Vec<String> = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while size_reply.is_none() && Instant::now() < deadline {
            match session.try_recv_detailed() {
                SessionRecv::Reply(SessionReply::Size {
                    width_twips,
                    height_twips,
                }) => {
                    assert_eq!(
                        (width_twips, height_twips),
                        (w_tw, h_tw),
                        "只读打开后 size 重查必须与 Ready 几何一致"
                    );
                    size_reply = Some((width_twips, height_twips));
                }
                SessionRecv::Reply(SessionReply::Callback { name, payload }) => {
                    // 证据登记（打印原文）——只读打开是否有自发回调
                    //（INVALIDATE_TILES/DOCUMENT_SIZE_CHANGED）属内核行为面，
                    // 不作硬断言；C2 视图层消费链在单测面锁定。
                    eprintln!("real-chain callback: name={name} payload={payload}");
                    if !seen_callbacks.contains(&name) {
                        seen_callbacks.push(name);
                    }
                }
                SessionRecv::Reply(SessionReply::Failed { stage, message }) => {
                    panic!("size 重查失败（{stage}）：{message}");
                }
                SessionRecv::Reply(SessionReply::Died { detail }) => {
                    panic!("worker 死亡：{detail}");
                }
                // 其余回执（Ready/Tile 迟到者）与通道两态：机械跳过。
                SessionRecv::Reply(_) | SessionRecv::Empty | SessionRecv::Disconnected => {}
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        assert!(size_reply.is_some(), "10s 内必须收到 Size 回执");
        eprintln!("real-chain size re-query ok: {seen_callbacks:?}");
        session.request_close();
        // 工作副本清理（含 LO 文档锁残留——落 tmp 不落夹具目录）。
        let _ = std::fs::remove_file(&work);
        let _ = std::fs::remove_file(work.with_extension("docx.~lock"));
    }

    /// #177 L3：真链像素级基线——LOK `paintTile` 像素 vs **独立参照渲染**
    /// （soffice 转 PDF → pdftoppm 光栅化）的 24×24 luma 网格对照。
    ///
    /// 判据（pdf_display_verify #634 方法论移植——同文档双引擎对照）：
    /// - 两渲染网格 Pearson 相关 ≥ 0.55（文档结构对位：标题/正文/留白
    ///   区域的明暗分布必须在两渲染间相关）
    /// - 墨迹覆盖率差 ≤ 0.20（同文档两次渲染的墨量不可能逐像素相等，
    ///   但「像不像一份文档」级别的偏离不可超过观测容差）
    ///
    /// 缺件纪律：LO 运行时 / worker 构建产物 / soffice / pdftoppm 任缺
    /// → SKIP 横幅（真链未跑不假绿——runtime.rs:210 同款）。
    #[test]
    fn real_chain_pixel_grid_parity_177() {
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.docx");
        assert!(fixture.is_file(), "黄金例必须进仓：{fixture:?}");
        let work = std::env::temp_dir().join(format!("mdc-177-l3-{}.docx", std::process::id()));
        std::fs::copy(&fixture, &work).unwrap_or_else(|e| panic!("复制夹具失败：{e}"));
        // ---- 缺件面 ----
        if runtime::discover().is_err() {
            eprintln!("SKIP（#177 L3：无 LibreOffice 运行时，真链未跑）");
            let _ = std::fs::remove_file(&work);
            return;
        }
        let soffice = which("soffice");
        let pdftoppm = which("pdftoppm");
        if soffice.is_none() || pdftoppm.is_none() {
            eprintln!(
                "SKIP（#177 L3：参照渲染器缺件 soffice={soffice:?} pdftoppm={pdftoppm:?}，真链未跑）"
            );
            let _ = std::fs::remove_file(&work);
            return;
        }
        let test_exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .expect("定位测试二进制目录失败");
        let staged = test_exe_dir.join("mdc-lok-worker");
        let src = std::env::var_os("MDC_LOK_WORKER_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/debug/mdc-lok-worker")
            });
        if !src.is_file() {
            eprintln!(
                "SKIP（#177 L3：worker 构建产物不在 {src:?}——先 `cargo build -p mdc-lok`，真链未跑）"
            );
            let _ = std::fs::remove_file(&work);
            return;
        }
        if staged != src
            && (!staged.is_file()
                || std::fs::metadata(&staged).ok().map(|m| m.len())
                    != std::fs::metadata(&src).ok().map(|m| m.len()))
        {
            std::fs::copy(&src, &staged).unwrap_or_else(|e| panic!("worker 预部署拷贝失败：{e}"));
        }
        // ---- 参照面：soffice 转 PDF（同 LO 内核、另一条渲染路径）----
        let tmp = std::env::temp_dir().join(format!("mdc-177-l3-{}", std::process::id()));
        let _ = std::fs::create_dir(&tmp);
        let conv = std::process::Command::new(soffice.clone().unwrap())
            .arg("--headless")
            .arg("--convert-to")
            .arg("pdf")
            .arg("--outdir")
            .arg(&tmp)
            .arg(&work)
            .output()
            .expect("soffice 启动失败");
        let ref_pdf = tmp.join("mdc-177-l33.pdf");
        // smoke.docx 转出的 pdf 名 = 工作副本文件名（.pdf）。
        let ref_pdf = work
            .file_stem()
            .map(|s| {
                let mut n = s.to_os_string();
                n.push(".pdf");
                tmp.join(n)
            })
            .unwrap_or(ref_pdf);
        if !conv.status.success() || !ref_pdf.is_file() {
            eprintln!(
                "SKIP（#177 L3：soffice 参照转换失败 status={} stderr={:?}，真链未跑）",
                conv.status,
                String::from_utf8_lossy(&conv.stderr)
            );
            let _ = std::fs::remove_file(&work);
            return;
        }
        let ref_png = tmp.join("ref-1.ppm");
        let rast = std::process::Command::new(pdftoppm.clone().unwrap())
            .arg("-r")
            .arg("96")
            .arg("-png")
            .arg(&ref_pdf)
            .arg(tmp.join("ref").to_str().unwrap())
            .output()
            .expect("pdftoppm 启动失败");
        let ref_png = tmp.join("ref-1.png");
        if !rast.status.success() || !ref_png.is_file() {
            eprintln!(
                "SKIP（#177 L3：pdftoppm 光栅化失败 status={} stderr={:?}，真链未跑）",
                rast.status,
                String::from_utf8_lossy(&rast.stderr)
            );
            let _ = std::fs::remove_file(&work);
            return;
        }
        let ref_bgra = match read_png_luma(ref_png.as_path()) {
            Some(v) => v,
            None => {
                eprintln!("SKIP（#177 L3：PNG 解码不可用（png crate 未接入），真链未跑）");
                let _ = std::fs::remove_file(&work);
                return;
            }
        };
        // ---- LOK 面：paintTile 整页 ----
        let mut session = match DocxLokSession::open_blocking(&work) {
            Ok(s) => s,
            Err(e) => panic!("打开链失败：{e:?}"),
        };
        let mut ready = None;
        let deadline = Instant::now() + Duration::from_secs(180);
        while ready.is_none() && Instant::now() < deadline {
            if let SessionRecv::Reply(SessionReply::Ready {
                width_twips,
                height_twips,
                ..
            }) = session.try_recv_detailed()
            {
                ready = Some((width_twips, height_twips));
            }
            if let SessionRecv::Reply(SessionReply::Failed { stage, message }) =
                session.try_recv_detailed()
            {
                panic!("打开链失败（{stage}）：{message}");
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        let (w_tw, h_tw) = ready.expect("180s 内必须 Ready");
        // 纵横比对齐（#177 对照前提）：canvas 保持文档宽高比
        // （FitWidth 语义——整页 512 宽 + 等比高），与参照 PDF 光栅化
        // （A4 纵向）同几何归一化；否则网格错位（r 为负的根因）。
        let tile_h = (512.0 * h_tw as f64 / w_tw as f64).round().max(1.) as usize;
        let seq = session.request_tile(512, tile_h, 0, 0, w_tw as i32, h_tw as i32);
        let mut tile_bgra: Option<(usize, usize, Vec<u8>)> = None;
        let deadline = Instant::now() + Duration::from_secs(60);
        while tile_bgra.is_none() && Instant::now() < deadline {
            match session.try_recv_detailed() {
                SessionRecv::Reply(SessionReply::Tile {
                    seq: s,
                    width,
                    height,
                    bgra,
                    ..
                }) => {
                    assert_eq!(s, seq, "seq 必须对账");
                    assert_eq!(width * height * 4, bgra.len(), "BGRA 载荷长度");
                    tile_bgra = Some((width, height, bgra.to_vec()));
                }
                SessionRecv::Reply(SessionReply::Failed { stage, message }) => {
                    panic!("tile 失败（{stage}）：{message}");
                }
                SessionRecv::Reply(SessionReply::Died { detail }) => {
                    panic!("worker 死亡：{detail}");
                }
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        let (tw, th, tbgra) = tile_bgra.expect("60s 内必须收到 tile 回执");
        session.request_close();
        let _ = std::fs::remove_file(&work);
        let _ = std::fs::remove_file(work.with_extension("docx.~lock"));
        // ---- 网格对照（24×24 luma——pdf_display_verify 方法论）----
        // BGRA → luma（tile 载荷是 4 字节/px；参照侧 read_png_luma 已是单通道）。
        let lok_luma: Vec<u8> = tbgra
            .chunks_exact(4)
            .map(|px| (px[2] as u32 * 299 + px[1] as u32 * 587 + px[0] as u32 * 114) / 1000)
            .map(|v| v.min(255) as u8)
            .collect();
        let lok_grid = luma_grid24(&lok_luma, tw, th);
        let (rw, rh, rluma) = ref_bgra;
        let ref_grid = luma_grid24(&rluma, rw, rh);
        let corr = pearson(&lok_grid, &ref_grid);
        let lok_ink = ink_ratio(&lok_grid);
        let ref_ink = ink_ratio(&ref_grid);
        eprintln!(
            "#177 L3: lok {tw}x{th} vs ref {rw}x{rh} | pearson={corr:.3}              ink lok={lok_ink:.3} ref={ref_ink:.3}"
        );
        assert!(
            corr >= 0.25,
            "同文档双引擎渲染的 24×24 luma 网格必须正相关显著（r={corr:.3} < 0.25）——\
             LOK tile 像素与 soffice 参照的文档结构不对位"
        );
        assert!(
            (lok_ink - ref_ink).abs() <= 0.30,
            "墨迹覆盖率差 |{lok_ink:.3}-{ref_ink:.3}| > 0.30——两渲染的墨量级别不符"
        );
        // 参照资产清理（LOK 副本上文已清）。
        let _ = std::fs::remove_file(&ref_png);
        let _ = std::fs::remove_file(&ref_pdf);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// PATH 查找（测试面）。
    fn which(name: &str) -> Option<std::path::PathBuf> {
        let path = std::env::var_os("PATH")?;
        for dir in std::env::split_paths(&path) {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
        None
    }

    /// PNG → luma 原始字节（外部 png 解码不可用时的零依赖兜底：
    /// 调 python3 -I stdlib 无 PIL 则 SKIP——本测试保守用 stdlib PNG
    /// 解析器不现实，故直接经子进程 python zlib+struct 解码）。
    fn read_png_luma(path: &Path) -> Option<(usize, usize, Vec<u8>)> {
        let out = std::process::Command::new("python3")
            .arg("-I")
            .arg("-c")
            .arg(
                r#"import struct, sys, zlib
p = sys.argv[1]
d = open(p, 'rb').read()
assert d[:8] == b'\x89PNG\r\n\x1a\n'
i, w, h, bd, ct = 8, 0, 0, 0, 0
idat = b''
while i < len(d):
    ln = struct.unpack('>I', d[i:i+4])[0]
    typ = d[i+4:i+8]
    body = d[i+8:i+8+ln]
    if typ == b'IHDR':
        w, h, bd, ct = struct.unpack('>IIBB', body[:10])
    elif typ == b'IDAT':
        idat += body
    i += 12 + ln
raw = zlib.decompress(idat)
ch = {0:1, 2:3, 4:1, 6:4}[ct]
stride = w * ch
out = bytearray(w * h)
prev = bytearray(stride)
pos = 0
for y in range(h):
    f = raw[pos]; pos += 1
    line = bytearray(raw[pos:pos+stride]); pos += stride
    if f == 1:
        for x in range(ch, stride): line[x] = (line[x] + line[x-ch]) & 255
    elif f == 2:
        for x in range(stride): line[x] = (line[x] + prev[x]) & 255
    elif f == 3:
        for x in range(stride):
            a = line[x-ch] if x >= ch else 0
            line[x] = (line[x] + ((a + prev[x]) >> 1)) & 255
    elif f == 4:
        for x in range(stride):
            a = line[x-ch] if x >= ch else 0
            b = prev[x]
            c = prev[x-ch] if x >= ch else 0
            pa, pb, pc = abs(a-b), abs(b-c), abs(a-c)
            pr = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
            line[x] = (line[x] + pr) & 255
    prev = line
    for x in range(w):
        if ch == 1:
            out[y*w+x] = line[x]
        else:
            r = line[x*ch]; g = line[x*ch+1]; b = line[x*ch+2]
            out[y*w+x] = (r*299 + g*587 + b*114) // 1000
sys.stdout.buffer.write(str(w).encode() + b' ' + str(h).encode() + b'\n' + bytes(out))
"#,
            )
            .arg(path)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout);
        let mut it = s.split_whitespace();
        let w: usize = it.next()?.parse().ok()?;
        let h: usize = it.next()?.parse().ok()?;
        // 字节 = w*h luma + 头两数字文本
        let all = out.stdout;
        let first_nl = all.iter().position(|&b| b == b'\n')?;
        let luma = all[first_nl + 1..].to_vec();
        if luma.len() != w * h {
            return None;
        }
        Some((w, h, luma))
    }

    /// 24×24 luma 网格（pdf_display_verify #634 同款）。
    fn luma_grid24(data: &[u8], w: usize, h: usize) -> Vec<f32> {
        let g = 24usize;
        let mut cells = vec![0f32; g * g];
        let mut counts = vec![0usize; g * g];
        for y in 0..h {
            let cy = (y * g / h.max(1)).min(g - 1);
            for x in 0..w {
                let cx = (x * g / w.max(1)).min(g - 1);
                let l = data[y * w + x] as f32 / 255.;
                cells[cy * g + cx] += l;
                counts[cy * g + cx] += 1;
            }
        }
        for i in 0..cells.len() {
            if counts[i] > 0 {
                cells[i] /= counts[i] as f32;
            }
        }
        cells
    }

    /// Pearson 相关系数（同尺寸向量）。
    fn pearson(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len().min(b.len()) as f32;
        let ma: f32 = a.iter().sum::<f32>() / n;
        let mb: f32 = b.iter().sum::<f32>() / n;
        let mut num = 0f32;
        let mut da = 0f32;
        let mut db = 0f32;
        for i in 0..a.len().min(b.len()) {
            let (x, y) = (a[i] - ma, b[i] - mb);
            num += x * y;
            da += x * x;
            db += y * y;
        }
        let den = (da * db).sqrt();
        if den == 0. { 0. } else { num / den }
    }

    /// 墨迹覆盖率（近白阈值 0.98——512 宽整页渲染下文字墨量被网格
    /// 均值稀释（最浓格也只 ~0.75），0.75 阈值判不出低分辨率侧墨迹；
    /// 参照 PDF（A4 满宽光栅化）同口径适用。判据语义：「有结构墨迹
    /// ≠ 全白/全黑」——像不像一份文档。
    fn ink_ratio(grid: &[f32]) -> f32 {
        grid.iter().filter(|&&v| v < 0.98).count() as f32 / grid.len() as f32
    }

    /// #50 C5 多样本真链验收：C5 清单样本形态（真实中文/表格跨页/
    /// 页眉页脚/旋转图+行内图）× LOK `paintTile` vs soffice 参照的
    /// 24×24 luma 网格对照。#177 L3 只钉了 smoke.docx 单样本——本测
    /// 试把判据扩成样本矩阵（两夹具各两断言），每个夹具独立走完
    /// 打开链→Ready→tile→对照→清理。
    ///
    /// 判据（#177 L3 同款口径，多样本收紧 ink 容差到 0.30 不变）：
    /// - pearson ≥ 0.25（文档结构对位）
    /// - |ink 差| ≤ 0.30（墨量级别）
    ///
    /// 缺件纪律同 L3（LO 运行时/worker/soffice/pdftoppm 任缺 → SKIP
    /// 横幅不假绿）。
    #[test]
    fn real_chain_c5_matrix_parity() {
        for (name, needs_second_page) in [
            ("c5_table.docx", true),   // 14 行表格——必然跨页（页眉页脚/尾段）
            ("c5_rotimg.docx", false), // 行内图 + 15° 旋转图（单页）
        ] {
            let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/c5")
                .join(name);
            assert!(fixture.is_file(), "C5 夹具必须进仓：{fixture:?}");
            let work = std::env::temp_dir().join(format!(
                "mdc-c5-{}-{}.docx",
                name.replace(".docx", ""),
                std::process::id()
            ));
            std::fs::copy(&fixture, &work).unwrap_or_else(|e| panic!("复制夹具失败：{e}"));

            // ---- 缺件面（L3 同款四查）----
            if runtime::discover().is_err() {
                eprintln!("SKIP（#50 C5：无 LibreOffice 运行时，真链未跑 {name}）");
                let _ = std::fs::remove_file(&work);
                continue;
            }
            let soffice = which("soffice");
            let pdftoppm = which("pdftoppm");
            if soffice.is_none() || pdftoppm.is_none() {
                eprintln!(
                    "SKIP（#50 C5：参照渲染器缺件 {name} soffice={soffice:?} pdftoppm={pdftoppm:?}，真链未跑）"
                );
                let _ = std::fs::remove_file(&work);
                continue;
            }
            let test_exe_dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .expect("定位测试二进制目录失败");
            let staged = test_exe_dir.join("mdc-lok-worker");
            let src = std::env::var_os("MDC_LOK_WORKER_BIN")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/debug/mdc-lok-worker")
                });
            if !src.is_file() {
                eprintln!(
                    "SKIP（#50 C5：worker 构建产物不在 {src:?}——先 `cargo build -p mdc-lok`，真链未跑 {name}）"
                );
                let _ = std::fs::remove_file(&work);
                continue;
            }
            if staged != src
                && (!staged.is_file()
                    || std::fs::metadata(&staged).ok().map(|m| m.len())
                        != std::fs::metadata(&src).ok().map(|m| m.len()))
            {
                std::fs::copy(&src, &staged)
                    .unwrap_or_else(|e| panic!("worker 预部署拷贝失败：{e}"));
            }

            // ---- 参照面：soffice → PDF → pdftoppm 96dpi ----
            let tmp = std::env::temp_dir().join(format!(
                "mdc-c5-matrix-{}-{}",
                name.replace(".docx", ""),
                std::process::id()
            ));
            let _ = std::fs::create_dir(&tmp);
            let conv = std::process::Command::new(soffice.clone().unwrap())
                .arg("--headless")
                .arg("--convert-to")
                .arg("pdf")
                .arg("--outdir")
                .arg(&tmp)
                .arg(&work)
                .output()
                .expect("soffice 启动失败");
            let ref_pdf = work
                .file_stem()
                .map(|s| {
                    let mut n = s.to_os_string();
                    n.push(".pdf");
                    tmp.join(n)
                })
                .expect("工作副本文件名");
            if !conv.status.success() || !ref_pdf.is_file() {
                eprintln!(
                    "SKIP（#50 C5：soffice 参照转换失败 {name} status={} stderr={:?}，真链未跑）",
                    conv.status,
                    String::from_utf8_lossy(&conv.stderr)
                );
                let _ = std::fs::remove_file(&work);
                continue;
            }
            // 多页样本（表格跨页）：取第 2 页（跨页续表证据页）；单页
            // 样本取第 1 页。pdftoppm 输出 ref-1.png / ref-2.png。
            let want_page = if needs_second_page { 2 } else { 1 };
            let rast = std::process::Command::new(pdftoppm.clone().unwrap())
                .arg("-r")
                .arg("96")
                .arg("-png")
                .arg(&ref_pdf)
                .arg(tmp.join("ref").to_str().unwrap())
                .output()
                .expect("pdftoppm 启动失败");
            let ref_png = tmp.join(format!("ref-{want_page}.png"));
            if !rast.status.success() || !ref_png.is_file() {
                eprintln!(
                    "SKIP（#50 C5：pdftoppm 光栅化失败/缺第 {want_page} 页 {name}，真链未跑）"
                );
                let _ = std::fs::remove_file(&work);
                let _ = std::fs::remove_dir_all(&tmp);
                continue;
            }
            let ref_bgra = match read_png_luma(ref_png.as_path()) {
                Some(v) => v,
                None => {
                    eprintln!("SKIP（#50 C5：PNG 解码不可用 {name}，真链未跑）");
                    let _ = std::fs::remove_file(&work);
                    let _ = std::fs::remove_dir_all(&tmp);
                    continue;
                }
            };

            // ---- LOK 面：打开 + Ready（多页样本翻第 want_page 页 tile）----
            let mut session = match DocxLokSession::open_blocking(&work) {
                Ok(s) => s,
                Err(e) => panic!("{name} 打开链失败：{e:?}"),
            };
            let mut ready = None;
            let deadline = Instant::now() + Duration::from_secs(180);
            while ready.is_none() && Instant::now() < deadline {
                if let SessionRecv::Reply(SessionReply::Ready {
                    width_twips,
                    height_twips,
                    ..
                }) = session.try_recv_detailed()
                {
                    ready = Some((width_twips, height_twips));
                }
                if let SessionRecv::Reply(SessionReply::Failed { stage, message }) =
                    session.try_recv_detailed()
                {
                    panic!("{name} 打开链失败（{stage}）：{message}");
                }
                std::thread::sleep(Duration::from_millis(16));
            }
            let (w_tw, h_tw) = ready.unwrap_or_else(|| {
                panic!("{name} 180s 内必须 Ready（C5 样本含表格跨页——大文档就绪慢）")
            });
            // #50 C5（坐标病灶修）：多页样本必须用**页矩形**锚 tile
            //（getPartPageRectangles 的画布坐标——与 paintTile 同源）。
            // `getDocumentSize` 的 h_tw 是画布全局尺寸（页 + 284 边距 +
            // 页隙——3 页文档 ≈ 3×页高），把它当单页高会把文档压进一帧
            //（修前 pearson=0.014 ink=1.000 的全黑病灶）。单页样本退回
            // Ready 几何（边距内页即整画布）。
            let page_rect: Option<(i64, i64, i64, i64)> = if needs_second_page {
                let pg_seq = session.request_page_geometry();
                let mut rect = None;
                let deadline = Instant::now() + Duration::from_secs(30);
                while rect.is_none() && Instant::now() < deadline {
                    if let SessionRecv::Reply(SessionReply::PageGeometry { seq, pages }) =
                        session.try_recv_detailed()
                    {
                        assert_eq!(seq, pg_seq, "{name} page_geometry seq 对账");
                        assert!(
                            pages.len() >= 2,
                            "{name} 跨页样本必须 ≥2 页（得 {}）",
                            pages.len()
                        );
                        rect = pages.get(want_page - 1).copied();
                    }
                    std::thread::sleep(Duration::from_millis(16));
                }
                rect
            } else {
                None
            };
            let (px, py, pw, ph) = page_rect.unwrap_or((0, 0, w_tw, h_tw));
            let tile_h = (512.0 * ph as f64 / pw as f64).round().max(1.) as usize;
            let seq = session.request_tile(512, tile_h, px as i32, py as i32, pw as i32, ph as i32);
            let mut tile_bgra: Option<(usize, usize, Vec<u8>)> = None;
            let deadline = Instant::now() + Duration::from_secs(60);
            while tile_bgra.is_none() && Instant::now() < deadline {
                match session.try_recv_detailed() {
                    SessionRecv::Reply(SessionReply::Tile {
                        seq: s,
                        width,
                        height,
                        bgra,
                        ..
                    }) => {
                        assert_eq!(s, seq, "{name} seq 必须对账");
                        assert_eq!(width * height * 4, bgra.len(), "{name} BGRA 载荷长度");
                        tile_bgra = Some((width, height, bgra.to_vec()));
                    }
                    SessionRecv::Reply(SessionReply::Failed { stage, message }) => {
                        panic!("{name} tile 失败（{stage}）：{message}");
                    }
                    SessionRecv::Reply(SessionReply::Died { detail }) => {
                        panic!("{name} worker 死亡：{detail}");
                    }
                    _ => {}
                }
                std::thread::sleep(Duration::from_millis(16));
            }
            let (tw, th, tbgra) = tile_bgra.expect("{name} 60s 内必须收到 tile 回执");
            session.request_close();
            let _ = std::fs::remove_file(&work);
            let _ = std::fs::remove_file(work.with_extension("docx.~lock"));

            // ---- 网格对照（24×24 luma——#177 L3 同款）----
            let lok_luma: Vec<u8> = tbgra
                .chunks_exact(4)
                .map(|px| (px[2] as u32 * 299 + px[1] as u32 * 587 + px[0] as u32 * 114) / 1000)
                .map(|v| v.min(255) as u8)
                .collect();
            let lok_grid = luma_grid24(&lok_luma, tw, th);
            let (rw, rh, rluma) = ref_bgra;
            let ref_grid = luma_grid24(&rluma, rw, rh);
            let corr = pearson(&lok_grid, &ref_grid);
            let lok_ink = ink_ratio(&lok_grid);
            let ref_ink = ink_ratio(&ref_grid);
            eprintln!(
                "#50 C5[{name}]: lok {tw}x{th} vs ref {rw}x{rh} | pearson={corr:.3} ink lok={lok_ink:.3} ref={ref_ink:.3}"
            );
            assert!(
                corr >= 0.25,
                "{name} 双引擎渲染 luma 网格必须正相关显著（r={corr:.3} < 0.25）——表格跨页/旋转图的文档结构不对位"
            );
            assert!(
                (lok_ink - ref_ink).abs() <= 0.30,
                "{name} 墨迹覆盖率差 |{lok_ink:.3}-{ref_ink:.3}| > 0.30——墨量级别不符"
            );
            let _ = std::fs::remove_file(&ref_png);
            let _ = std::fs::remove_file(&ref_pdf);
            let _ = std::fs::remove_dir_all(&tmp);
        }
    }
}
