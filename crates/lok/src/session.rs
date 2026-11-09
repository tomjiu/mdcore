//! 宿主文档会话（#633）——#616 产品接线的直接消费面。
//!
//! 保存安全链（issue 任务 4）：
//! 1. 源文件路径全程只读（worker 侧另有 save_as 目标 ≠ 源的硬守卫）；
//! 2. `save_edits_temp`：saveAs 到**临时路径** → 结构校验（zip/条目/
//!    期望文本）→ 返回回执。**脏位不清**；
//! 3. `commit_replace`：宿主决定替换时，把已校验产物原子落位
//!    （同目录临时名 + rename），成功才清脏位；任一步失败脏位保留、
//!    源文件不动、错误明确；
//! 4. 超时 / worker 死亡 / 保存失败 → 脏位一律保留（验收面）。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use crate::client::{LokRuntimeConfig, LokWorker};
use crate::error::{ErrorCode, LokError, LokResult};
use crate::ffi::{lok_enums, path_to_file_url};
use crate::verify::{verify_saved_docx, VerifyReport, VerifySpec};

/// 会话级超时档（paint/save 长于默认）。
const TIMEOUT_INPUT: Duration = Duration::from_secs(15);
const TIMEOUT_PAINT: Duration = Duration::from_secs(60);
const TIMEOUT_LOAD: Duration = Duration::from_secs(120);
const TIMEOUT_SAVE: Duration = Duration::from_secs(180);

/// 宿主侧单文档会话。
pub struct DocumentSession {
    worker: LokWorker,
    source: PathBuf,
    dirty: bool,
    revision: u64,
    last_save_temp: Option<PathBuf>,
    cfg: LokRuntimeConfig,
    /// wait_for_event 期间收到但尚未交给调用方的事件（drain 时一并
    /// 返回——等待不丢事件）。
    stashed_events: Vec<crate::client::LokEventMsg>,
}

/// 一次 tile 渲染结果。
#[derive(Debug, Clone)]
pub struct Tile {
    pub width: i32,
    pub height: i32,
    /// 像素（4 字节/px；通道序 = meta.tile_mode：0=RGBA 1=BGRA）。
    pub pixels: Vec<u8>,
    pub tile_mode: i32,
    pub revision: u64,
}

impl Tile {
    /// 通道序归一为 RGBA（BGRA→RGBA 就地换 B/R；PNG 诊断输出用）。
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut px = self.pixels.clone();
        if self.tile_mode == lok_enums::TILEMODE_BGRA {
            for chunk in px.as_chunks_mut::<4>().0 {
                chunk.swap(0, 2);
            }
        }
        px
    }

    /// 非白像素计数（冒烟「非空实际像素」断言，循 xlsx_chart 墨迹
    /// 对照先例；1x1 占位冒充会被这里的阈值拒绝）。
    pub fn nonwhite_pixels(&self) -> usize {
        self.pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| !(p[0] > 240 && p[1] > 240 && p[2] > 240))
            .count()
    }
}

/// 保存回执（校验报告随附，冒烟日志用）。
pub struct SaveReceipt {
    pub temp_path: PathBuf,
    pub report: VerifyReport,
}

impl DocumentSession {
    /// 打开会话：spawn worker → hello（运行时元数据）→ load 源文件
    /// → initializeForRendering（worker 侧）→ 默认缩放。
    pub fn open(cfg: &LokRuntimeConfig, path: &Path, mock: bool) -> LokResult<Self> {
        let mut worker = if mock {
            LokWorker::spawn_mock(cfg)?
        } else {
            LokWorker::spawn(cfg)?
        };
        // hello：运行时元数据（真实模式含 version_info/library/nSize）。
        let hello = worker.hello()?;
        let _runtime = hello
            .pointer("/meta/version_info")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "mock".to_string());
        // load：相对路径先规范化为绝对路径（LO 只收绝对 file: URL；
        // "type detection failed" 的常见成因就是相对 URL）。
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
        };
        let url = path_to_file_url(&abs);
        let _ = worker.call("load", &json!({"url": url}), Vec::new(), TIMEOUT_LOAD)?;
        // 默认缩放：1px = 15 twips（96dpi；ElyOffice 同款 TWIPS_PER_PIXEL）。
        let _ = worker.call(
            "set_client_zoom",
            &json!({"px_w": 256, "px_h": 256, "tw_w": 3840, "tw_h": 3840}),
            Vec::new(),
            TIMEOUT_INPUT,
        )?;
        Ok(DocumentSession {
            worker,
            source: path.to_path_buf(),
            dirty: false,
            revision: 0,
            last_save_temp: None,
            cfg: cfg.clone(),
            stashed_events: Vec::new(),
        })
    }

    pub fn source_path(&self) -> &Path {
        &self.source
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// worker 是否存活（宿主可据此决定重拉）。
    pub fn worker_alive(&self) -> bool {
        self.worker.is_alive()
    }

    /// 文档全幅（twips）。
    pub fn document_size(&mut self) -> LokResult<(i64, i64)> {
        let (v, _) = self
            .worker
            .call("doc_size", &Value::Null, Vec::new(), TIMEOUT_INPUT)?;
        Ok((
            v.get("width_twips").and_then(|x| x.as_i64()).unwrap_or(0),
            v.get("height_twips").and_then(|x| x.as_i64()).unwrap_or(0),
        ))
    }

    /// 渲染 tile（twips 矩形 → canvas 像素）。
    pub fn paint_tile(
        &mut self,
        canvas_w: i32,
        canvas_h: i32,
        tile_x: i32,
        tile_y: i32,
        tile_w: i32,
        tile_h: i32,
    ) -> LokResult<Tile> {
        let (v, pixels) = self.worker.call(
            "paint_tile",
            &json!({
                "canvas_w": canvas_w, "canvas_h": canvas_h,
                "tile_x": tile_x, "tile_y": tile_y,
                "tile_w": tile_w, "tile_h": tile_h,
            }),
            Vec::new(),
            TIMEOUT_PAINT,
        )?;
        self.revision = v
            .get("rev")
            .and_then(|x| x.as_u64())
            .unwrap_or(self.revision);
        Ok(Tile {
            width: v.get("width").and_then(|x| x.as_i64()).unwrap_or(0) as i32,
            height: v.get("height").and_then(|x| x.as_i64()).unwrap_or(0) as i32,
            tile_mode: v.get("tile_mode").and_then(|x| x.as_i64()).unwrap_or(0) as i32,
            pixels,
            revision: self.revision,
        })
    }

    /// 键事件（Unicode 字符直传；编辑后 dirty=true）。
    pub fn type_text(&mut self, text: &str) -> LokResult<()> {
        for ch in text.chars() {
            let (v, _) = self.worker.call(
                "post_key",
                &json!({"kind": lok_enums::KEYEVENT_KEYINPUT, "char": ch as i32, "keycode": 0}),
                Vec::new(),
                TIMEOUT_INPUT,
            )?;
            self.after_mutation(&v);
            let (v, _) = self.worker.call(
                "post_key",
                &json!({"kind": lok_enums::KEYEVENT_KEYUP, "char": ch as i32, "keycode": 0}),
                Vec::new(),
                TIMEOUT_INPUT,
            )?;
            self.after_mutation(&v);
        }
        Ok(())
    }

    /// 鼠标点击（twips；count=2 双击选词）。
    pub fn mouse_click(&mut self, x: i32, y: i32, count: i32) -> LokResult<()> {
        let (v, _) = self.worker.call(
            "post_mouse",
            &json!({
                "kind": lok_enums::MOUSEEVENT_BUTTON_DOWN,
                "x": x, "y": y, "count": count, "buttons": 1, "modifiers": 0,
            }),
            Vec::new(),
            TIMEOUT_INPUT,
        )?;
        self.after_mutation(&v);
        let (v, _) = self.worker.call(
            "post_mouse",
            &json!({
                "kind": lok_enums::MOUSEEVENT_BUTTON_UP,
                "x": x, "y": y, "count": count, "buttons": 1, "modifiers": 0,
            }),
            Vec::new(),
            TIMEOUT_INPUT,
        )?;
        self.after_mutation(&v);
        Ok(())
    }

    /// UNO 命令。
    pub fn post_uno(&mut self, command: &str, args: Option<&str>) -> LokResult<()> {
        let (v, _) = self.worker.call(
            "post_uno",
            &json!({"command": command, "args": args, "notify": false}),
            Vec::new(),
            TIMEOUT_INPUT,
        )?;
        self.after_mutation(&v);
        Ok(())
    }

    /// 当前选区文本。
    pub fn text_selection(&mut self) -> LokResult<Option<String>> {
        let (v, _) = self.worker.call(
            "get_text_selection",
            &json!({"mime": "text/plain;charset=utf-8"}),
            Vec::new(),
            TIMEOUT_INPUT,
        )?;
        Ok(v.get("text")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string()))
    }

    /// 取积压事件（含 wait_for_event 暂存与回调脏态/失效通知）。
    pub fn drain_events(&mut self) -> Vec<crate::client::LokEventMsg> {
        let mut out = std::mem::take(&mut self.stashed_events);
        out.extend(self.worker.drain_events());
        out
    }

    /// 轮询等待某类事件到达（LOK 的键鼠/UNO 命令即使 unipoll 同步
    /// 分发，事件帧也在响应帧之后写出；重画前等 invalidate_tiles
    /// 更稳——flaky 实测记录见 gates/633.md）。等待期间收到的**全部**
    /// 事件进暂存（drain_events 一并取回，不丢）；超时返回 false。
    pub fn wait_for_event(&mut self, kind: i32, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            self.stashed_events.extend(self.worker.drain_events());
            if self.stashed_events.iter().any(|e| e.kind == kind) {
                return true;
            }
            match deadline.checked_duration_since(std::time::Instant::now()) {
                Some(remain) if !remain.is_zero() => {
                    std::thread::sleep(remain.min(Duration::from_millis(25)));
                }
                _ => return false,
            }
        }
    }

    /// 杀死 worker（故障注入/超时恢复路径）。源文件与脏位不受影响；
    /// 本会话后续调用返回 WorkerDied，宿主应 `reopen` 或丢弃会话。
    pub fn kill_worker(&mut self) {
        self.worker.kill();
    }

    /// 用新 worker 重开同一源文件（编辑丢失到上次已保存/校验产物；
    /// 原型语义：宿主仍可操作——验收「worker 被终止宿主仍可操作」面）。
    pub fn reopen(&mut self) -> LokResult<()> {
        let fresh = Self::open(&self.cfg, &self.source, false)?;
        self.worker = fresh.worker;
        self.revision = 0;
        Ok(())
    }

    /// 另存临时产物 + 结构校验。**成功不清脏位**（commit 才清）。
    /// `spec` 断言编辑已进产物（期望文本已变等）。
    pub fn save_edits_temp(&mut self, spec: &VerifySpec) -> LokResult<SaveReceipt> {
        let dir = std::env::temp_dir().join(format!(
            "mdc-lok-save-{}-{}",
            std::process::id(),
            now_nanos()
        ));
        std::fs::create_dir_all(&dir).map_err(|e| {
            LokError::new(ErrorCode::SaveFailed, format!("保存临时目录创建失败：{e}"))
        })?;
        let temp = dir.join("saved.docx");
        let url = path_to_file_url(&temp);
        let (v, _) = self.worker.call(
            "save_as",
            &json!({"url": url, "format": "docx", "options": ""}),
            Vec::new(),
            TIMEOUT_SAVE,
        )?;
        if !v.get("saved").and_then(|s| s.as_bool()).unwrap_or(false) {
            return Err(LokError::new(
                ErrorCode::SaveFailed,
                format!("LO saveAs 失败：{v}"),
            ));
        }
        let report = verify_saved_docx(&temp, spec)?;
        self.last_save_temp = Some(temp.clone());
        Ok(SaveReceipt {
            temp_path: temp,
            report,
        })
    }

    /// 决定替换：把已校验产物原子落位到源路径（同目录临时名 +
    /// rename）。成功清脏位；失败脏位保留、源文件不动。
    pub fn commit_replace(&mut self, receipt: &SaveReceipt) -> LokResult<()> {
        if !receipt.temp_path.exists() {
            return Err(LokError::new(
                ErrorCode::SaveFailed,
                "已校验产物不存在（临时文件被外部清除？）——脏位保留",
            ));
        }
        let parent = self
            .source
            .parent()
            .ok_or_else(|| LokError::new(ErrorCode::InvalidRequest, "源路径无父目录"))?;
        let staged = parent.join(format!(
            ".{}.mdc-lok-commit",
            self.source
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        ));
        std::fs::copy(&receipt.temp_path, &staged).map_err(|e| {
            LokError::new(
                ErrorCode::SaveFailed,
                format!("落位复制失败（目录拒写？）：{e}——源文件未动，脏位保留"),
            )
        })?;
        std::fs::rename(&staged, &self.source).map_err(|e| {
            let _ = std::fs::remove_file(&staged);
            LokError::new(
                ErrorCode::SaveFailed,
                format!("原子替换失败：{e}——源文件未动，脏位保留"),
            )
        })?;
        self.dirty = false;
        Ok(())
    }

    /// 高级逃生口：绕过语义化方法直接调 worker 协议（边界测试的
    /// 故障注入与 #616 宿主接线用；常规操作请走 paint/输入/保存方法）。
    pub fn call_worker(
        &mut self,
        method: &str,
        params: &Value,
        timeout: Duration,
    ) -> LokResult<(Value, Vec<u8>)> {
        self.worker.call(method, params, Vec::new(), timeout)
    }

    /// 优雅关闭。
    pub fn close(self) -> LokResult<()> {
        self.worker.shutdown(TIMEOUT_INPUT)
    }

    fn after_mutation(&mut self, v: &Value) {
        self.dirty = true;
        if let Some(rev) = v.get("rev").and_then(|r| r.as_u64()) {
            self.revision = rev;
        }
    }
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}
