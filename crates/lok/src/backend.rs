//! 文档会话后端抽象（#633）。
//!
//! worker 只面向 [`DocSession`] trait 分发请求；真实实现
//! （[`RealSession`]，包 ffi 的 Lok/LokDocument）与测试替身
//! （[`crate::mock::MockSession`]）同构——协议/边界测试因此可以
//! 在无 LibreOffice 运行时的 CI 上真实跑 worker 子进程（spawn、
//! 帧往返、kill、超时、拒写），只有「真实渲染/编辑/保存」门需要
//! 实机运行时（tests/lok_real.rs 环境门 + examples/smoke.rs）。

use std::sync::mpsc::Receiver;

use crate::error::LokResult;
use crate::ffi::{Lok, LokDocument};

/// 一条 LOK 回调事件（type 见 ffi::lok_enums；payload 原样字符串）。
#[derive(Debug, Clone)]
pub struct LokEvent {
    pub kind: i32,
    pub payload: String,
}

/// 单文档会话能力面（worker 协议的直接后端）。
///
/// 线程纪律：真实实现内全部 LOK 调用必须来自创建它的分发线程；
/// `drain_events` 由 worker 在每次请求处理前后调用，把内核回调
/// （真实：回调蹦床入队；mock：直接注入）回传宿主。
pub trait DocSession {
    fn doc_type(&self) -> i32;
    fn parts(&self) -> i32;
    fn doc_size(&self) -> (i64, i64);
    fn tile_mode(&self) -> i32;

    #[allow(clippy::too_many_arguments)] // 参数面钉定 LOK paintTile ABI
    fn paint_tile(
        &mut self,
        buf: &mut [u8],
        canvas_w: i32,
        canvas_h: i32,
        tile_x: i32,
        tile_y: i32,
        tile_w: i32,
        tile_h: i32,
    ) -> LokResult<()>;

    fn post_key(&mut self, kind: i32, char_code: i32, key_code: i32) -> LokResult<()>;
    fn post_mouse(
        &mut self,
        kind: i32,
        x: i32,
        y: i32,
        count: i32,
        buttons: i32,
        modifiers: i32,
    ) -> LokResult<()>;
    fn post_uno(&mut self, command: &str, args: Option<&str>, notify: bool) -> LokResult<()>;
    fn get_command_values(&mut self, command: &str) -> LokResult<Option<String>>;
    fn get_text_selection(&mut self, mime: &str) -> LokResult<Option<String>>;
    fn set_client_zoom(&mut self, px_w: i32, px_h: i32, tw_w: i32, tw_h: i32) -> LokResult<()>;

    /// 另存（saveAs 布尔语义：非 0=成功）。实现负责把失败文本放进
    /// 事件流（ERROR 事件）。
    fn save_as(&mut self, url: &str, format: &str, options: &str) -> LokResult<i32>;

    /// 取走自上次 drain 以来内核/mock 产生的事件。
    fn drain_events(&mut self) -> Vec<LokEvent>;

    /// 显式销毁文档（进程退出前调用；LO 端 teardown 走 _exit 兜底）。
    fn destroy(&mut self) -> LokResult<()>;
}

/// 真实 LibreOfficeKit 会话。
pub struct RealSession {
    _lok: Lok,
    doc: Option<LokDocument>,
    /// 回调事件通道（kit/文档回调蹦床 → 分发线程 drain）。
    events_rx: Receiver<LokEvent>,
    /// Sender 副本（kit 回调闭包已持一份；load 时再克隆给文档回调）。
    events_tx: Option<std::sync::Arc<std::sync::mpsc::Sender<LokEvent>>>,
}

/// load 响应元数据。
#[derive(Debug, Clone, serde::Serialize)]
pub struct LoadInfo {
    pub doc_type: i32,
    pub parts: i32,
    pub doc_nsize: usize,
}

impl RealSession {
    /// 初始化 LOK（program_dir / profile_dir 语义见 ffi::lok_init）
    /// 并注册 kit 级回调。
    pub fn new(program_dir: &std::path::Path, profile_dir: &std::path::Path) -> LokResult<Self> {
        let lok = crate::ffi::lok_init(program_dir, profile_dir)?;
        // Sender 用 Arc 包裹：kit 回调闭包与文档回调闭包各持一份克隆，
        // 本体留在会话里供 load 时再克隆（mpsc Sender move 语义下直接
        // 交出会丢失后续注册能力）。
        let (tx, rx) = std::sync::mpsc::channel::<LokEvent>();
        let events_tx = std::sync::Arc::new(tx);
        let keep = std::sync::Arc::clone(&events_tx);
        unsafe extern "C" fn trampoline(
            kind: i32,
            payload: *const std::ffi::c_char,
            data: *mut std::ffi::c_void,
        ) {
            let cb = &*(data as *const Box<dyn Fn(i32, String) + Send>);
            let text = if payload.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(payload)
                    .to_string_lossy()
                    .into_owned()
            };
            cb(kind, text);
        }
        let cb: Box<dyn Fn(i32, String) + Send> = Box::new(move |kind, payload| {
            let _ = events_tx.send(LokEvent { kind, payload });
        });
        let data = Box::into_raw(Box::new(cb)) as *mut std::ffi::c_void;
        lok.register_kit_callback(trampoline, data)?;
        Ok(RealSession {
            _lok: lok,
            doc: None,
            events_rx: rx,
            events_tx: Some(keep),
        })
    }

    pub fn version_info(&self) -> LokResult<Option<String>> {
        self._lok.version_info()
    }

    pub fn kit_nsize(&self) -> usize {
        self._lok.kit_nsize()
    }

    /// 内核 kit 指针（unipoll runLoop 集成：serve 尾部进入主循环）。
    pub fn kit_ptr(&self) -> *mut crate::ffi::LibreOfficeKit {
        self._lok.kit_ptr()
    }

    fn doc(&self) -> LokResult<&LokDocument> {
        self.doc.as_ref().ok_or_else(|| {
            crate::error::LokError::new(
                crate::error::ErrorCode::NoDocument,
                "文档未加载（单文档 worker：先 load 后操作）",
            )
        })
    }

    /// 加载文档：initializeForRendering + 注册文档回调。
    pub fn load(&mut self, url: &str) -> LokResult<LoadInfo> {
        if self.doc.is_some() {
            return Err(crate::error::LokError::new(
                crate::error::ErrorCode::DocumentAlreadyOpen,
                "已有文档打开（原型单文档：先 close 再 load）",
            ));
        }
        let doc = self._lok.document_load(url)?;
        doc.initialize_for_rendering("")?;
        // 文档级回调：与 kit 通道共用（蹦床同款）。
        if let Some(tx) = &self.events_tx {
            let tx = std::sync::Arc::clone(tx);
            let cb: Box<dyn Fn(i32, String) + Send> = Box::new(move |kind, payload| {
                let _ = tx.send(LokEvent { kind, payload });
            });
            let data = Box::into_raw(Box::new(cb)) as *mut std::ffi::c_void;
            unsafe extern "C" fn trampoline(
                kind: i32,
                payload: *const std::ffi::c_char,
                data: *mut std::ffi::c_void,
            ) {
                let cb = &*(data as *const Box<dyn Fn(i32, String) + Send>);
                let text = if payload.is_null() {
                    String::new()
                } else {
                    std::ffi::CStr::from_ptr(payload)
                        .to_string_lossy()
                        .into_owned()
                };
                cb(kind, text);
            }
            doc.register_callback(trampoline, data)?;
        }
        let info = LoadInfo {
            doc_type: doc.document_type()?,
            parts: doc.parts()?,
            doc_nsize: doc.doc_nsize(),
        };
        self.doc = Some(doc);
        Ok(info)
    }
}

impl DocSession for RealSession {
    fn doc_type(&self) -> i32 {
        self.doc
            .as_ref()
            .and_then(|d| d.document_type().ok())
            .unwrap_or(-1)
    }

    fn parts(&self) -> i32 {
        self.doc.as_ref().and_then(|d| d.parts().ok()).unwrap_or(0)
    }

    fn doc_size(&self) -> (i64, i64) {
        self.doc
            .as_ref()
            .and_then(|d| d.document_size().ok())
            .unwrap_or((0, 0))
    }

    fn tile_mode(&self) -> i32 {
        self.doc
            .as_ref()
            .and_then(|d| d.tile_mode().ok())
            .unwrap_or(0)
    }

    fn paint_tile(
        &mut self,
        buf: &mut [u8],
        canvas_w: i32,
        canvas_h: i32,
        tile_x: i32,
        tile_y: i32,
        tile_w: i32,
        tile_h: i32,
    ) -> LokResult<()> {
        self.doc()?
            .paint_tile(buf, canvas_w, canvas_h, tile_x, tile_y, tile_w, tile_h)
    }

    fn post_key(&mut self, kind: i32, char_code: i32, key_code: i32) -> LokResult<()> {
        self.doc()?.post_key_event(kind, char_code, key_code)
    }

    fn post_mouse(
        &mut self,
        kind: i32,
        x: i32,
        y: i32,
        count: i32,
        buttons: i32,
        modifiers: i32,
    ) -> LokResult<()> {
        self.doc()?
            .post_mouse_event(kind, x, y, count, buttons, modifiers)
    }

    fn post_uno(&mut self, command: &str, args: Option<&str>, notify: bool) -> LokResult<()> {
        self.doc()?.post_uno_command(command, args, notify)
    }

    fn get_command_values(&mut self, command: &str) -> LokResult<Option<String>> {
        self.doc()?.get_command_values(command)
    }

    fn get_text_selection(&mut self, mime: &str) -> LokResult<Option<String>> {
        self.doc()?.get_text_selection(mime)
    }

    fn set_client_zoom(&mut self, px_w: i32, px_h: i32, tw_w: i32, tw_h: i32) -> LokResult<()> {
        self.doc()?.set_client_zoom(px_w, px_h, tw_w, tw_h)
    }

    fn save_as(&mut self, url: &str, format: &str, options: &str) -> LokResult<i32> {
        self.doc()?.save_as(url, format, options)
    }

    fn drain_events(&mut self) -> Vec<LokEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.events_rx.try_recv() {
            out.push(ev);
        }
        out
    }

    fn destroy(&mut self) -> LokResult<()> {
        if let Some(doc) = self.doc.take() {
            doc.destroy()?;
        }
        Ok(())
    }
}
