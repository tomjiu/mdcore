//! Mock 后端（#633）——无 LibreOffice 运行时的协议/边界测试面。
//!
//! 诚实边界（AGENTS：SKIP/PASS 不混报）：mock 只证明**协议、进程
//! 隔离、超时、脏位语义**，不证明任何渲染/编辑真实行为——真实门
//! 在 tests/lok_real.rs（环境门）与 examples/smoke.rs（实机冒烟）。
//!
//! 行为模型：
//! - 文本 = 若干行；`post_mouse` 按 y 坐标（行高 300 twips）定位
//!   当前行；`post_key` KEYINPUT 追加/替换字符；`.uno:SelectAll`
//!   选中当前行（此后输入替换整行）；
//! - `paint_tile` 生成确定性像素（文本 hash 驱动的彩带 + 坐标网格），
//!   保证「非空白」与「编辑后像素变化」两断言可判；
//! - `save_as` 写**真实最小 zip**（stored 条目），宿主 verify.rs
//!   可读回文本——保存链路的宿主侧校验因此也进 CI；
//! - 变更操作后注入 `INVALIDATE_TILES` 事件（真实内核同款回调号），
//!   宿主事件断言无需运行时。
//!
//! 注入开关（per-worker，由宿主 LokRuntimeConfig.mock_hang_method /
//! MDC_LOK_MOCK_SAVE_FAIL env 设置——真实 spawn 的子进程 env 隔离，
//! 并行测试互不污染）：
//! - `MDC_LOK_MOCK_HANG=<method>`：该方法处理时永久睡眠（超时门）；
//! - `MDC_LOK_MOCK_SAVE_FAIL=1`：save_as 返回失败码（保存失败门）。

use std::io::Write;

use crate::backend::{DocSession, LokEvent};
use crate::error::{ErrorCode, LokError, LokResult};
use crate::ffi::lok_enums;

pub struct MockSession {
    lines: Vec<String>,
    current_line: usize,
    selected: bool,
    tile_mode: i32,
    doc_size: (i64, i64),
    events: Vec<LokEvent>,
    pub loaded_url: Option<String>,
}

impl MockSession {
    pub fn new() -> Self {
        MockSession {
            lines: vec![
                "LOK mock 标题行".to_string(),
                "第二段落：正文锚点词 REPLACEME 待替换。".to_string(),
                "单元格甲".to_string(),
                "单元格乙".to_string(),
            ],
            current_line: 0,
            selected: false,
            tile_mode: lok_enums::TILEMODE_BGRA,
            doc_size: (11906, 16838),
            events: Vec::new(),
            loaded_url: None,
        }
    }

    fn hang_if_requested(&self, method: &str) {
        if std::env::var("MDC_LOK_MOCK_HANG").as_deref() == Ok(method) {
            // 永久睡眠：宿主必须在超时后 kill 才能恢复——这正是被测面。
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
    }

    fn mutate(&mut self) {
        self.events.push(LokEvent {
            kind: lok_enums::CALLBACK_INVALIDATE_TILES,
            payload: "{\"rectangle\":\"0, 0, 11906, 16838\"}".to_string(),
        });
    }

    /// mock 文档全文本（save_as 落盘与 verify 断言的源）。
    pub fn full_text(&self) -> String {
        self.lines.join("\n")
    }

    fn write_zip(path: &str, lines: &[String]) -> LokResult<()> {
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<w:document>\
             <w:body>{}</w:body></w:document>",
            lines
                .iter()
                .map(|l| format!("<w:p><w:r><w:t>{}</w:t></w:r></w:p>", l))
                .collect::<String>()
        );
        let entries: Vec<(&str, Vec<u8>)> = vec![
            ("[Content_Types].xml", b"<Types/>".to_vec()),
            ("word/document.xml", xml.into_bytes()),
            (
                "word/media/image1.png",
                vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
            ),
        ];
        write_stored_zip(path, &entries)
    }
}

/// 写一个最小合法 zip（全 stored 条目）。字节布局逐字段精确构造：
/// LFH(30B)：sig|ver|flags|method|time|date|crc|csize|usize|nlen|elen；
/// CD(46B)：…|clen|disk|iattr|eattr|offset；EOCD(22B)。
pub(crate) fn write_stored_zip(path: &str, entries: &[(&str, Vec<u8>)]) -> LokResult<()> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    for (name, data) in entries {
        let offset = out.len() as u32;
        // local file header（30 字节定长 + name + data）
        out.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]);
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // method=stored
        out.extend_from_slice(&0u16.to_le_bytes()); // mtime
        out.extend_from_slice(&0u16.to_le_bytes()); // mdate
        out.extend_from_slice(&0u32.to_le_bytes()); // crc32（mock 不校验）
        out.extend_from_slice(&(data.len() as u32).to_le_bytes()); // csize
        out.extend_from_slice(&(data.len() as u32).to_le_bytes()); // usize
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        // central directory 记录（46 字节定长 + name）
        central.extend_from_slice(&[0x50, 0x4B, 0x01, 0x02]);
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0u16.to_le_bytes()); // flags
        central.extend_from_slice(&0u16.to_le_bytes()); // method
        central.extend_from_slice(&0u16.to_le_bytes()); // mtime
        central.extend_from_slice(&0u16.to_le_bytes()); // mdate
        central.extend_from_slice(&0u32.to_le_bytes()); // crc32
        central.extend_from_slice(&(data.len() as u32).to_le_bytes()); // csize
        central.extend_from_slice(&(data.len() as u32).to_le_bytes()); // usize
        central.extend_from_slice(&(name.len() as u16).to_le_bytes()); // nlen @28
        central.extend_from_slice(&0u16.to_le_bytes()); // extra len @30
        central.extend_from_slice(&0u16.to_le_bytes()); // comment len @32
        central.extend_from_slice(&0u16.to_le_bytes()); // disk start @34
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attr @36
        central.extend_from_slice(&0u32.to_le_bytes()); // external attr @38
        central.extend_from_slice(&offset.to_le_bytes()); // LFH offset @42
        central.extend_from_slice(name.as_bytes()); // @46
    }
    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);
    // EOCD（22 字节）
    out.extend_from_slice(&[0x50, 0x4B, 0x05, 0x06]);
    out.extend_from_slice(&0u16.to_le_bytes()); // 本盘号
    out.extend_from_slice(&0u16.to_le_bytes()); // CD 起始盘号
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes()); // 本盘条目数
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes()); // 总条目数
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // 注释长
    let mut f = std::fs::File::create(path)
        .map_err(|e| LokError::new(ErrorCode::SaveFailed, format!("mock save 创建失败：{e}")))?;
    f.write_all(&out)
        .map_err(|e| LokError::new(ErrorCode::SaveFailed, format!("mock save 写入失败：{e}")))?;
    Ok(())
}

impl Default for MockSession {
    fn default() -> Self {
        Self::new()
    }
}

impl DocSession for MockSession {
    fn doc_type(&self) -> i32 {
        0 // LOK_DOCTYPE_TEXT（钉定头语义：Writer）
    }

    fn parts(&self) -> i32 {
        1
    }

    fn doc_size(&self) -> (i64, i64) {
        self.doc_size
    }

    fn tile_mode(&self) -> i32 {
        self.tile_mode
    }

    fn paint_tile(
        &mut self,
        buf: &mut [u8],
        canvas_w: i32,
        canvas_h: i32,
        tile_x: i32,
        tile_y: i32,
        _tile_w: i32,
        _tile_h: i32,
    ) -> LokResult<()> {
        self.hang_if_requested("paint_tile");
        let need = canvas_w as usize * canvas_h as usize * 4;
        if buf.len() < need {
            return Err(LokError::new(ErrorCode::InvalidRequest, "缓冲不足"));
        }
        // 确定性伪渲染：行文本 hash 驱动色带 + 坐标网格 + 白底。
        let text = self.full_text();
        let hash = text
            .bytes()
            .fold(0x811cu32, |h, b| (h ^ b as u32).wrapping_mul(0x01000193));
        for y in 0..canvas_h as usize {
            for x in 0..canvas_w as usize {
                let i = (y * canvas_w as usize + x) * 4;
                let (r, g, b) = if (x + y) % 24 == 0 {
                    (0x30u8, 0x30, 0x30) // 网格线
                } else {
                    let band = (y / 40) as u32;
                    let v = hash.wrapping_mul(band + 1) as u8;
                    let base: u8 = if ((x / 16) + (y / 16)) % 2 == 0 {
                        0xF0
                    } else {
                        0xE0
                    };
                    (base / 4 + v % 48, base.saturating_sub(v % 32), base)
                };
                buf[i] = b;
                buf[i + 1] = g;
                buf[i + 2] = r;
                buf[i + 3] = 0xFF;
            }
        }
        // 标记 tile 起点，证明坐标参数真实参与了渲染。
        if tile_x >= 0 && tile_y >= 0 && canvas_w > 4 && canvas_h > 4 {
            for k in 0..4 {
                let i = (k * canvas_w as usize + k) * 4;
                buf[i] = 0x00;
                buf[i + 1] = 0xC0;
                buf[i + 2] = 0x00;
                buf[i + 3] = 0xFF;
            }
        }
        Ok(())
    }

    fn post_key(&mut self, kind: i32, char_code: i32, key_code: i32) -> LokResult<()> {
        self.hang_if_requested("post_key");
        if kind == lok_enums::KEYEVENT_KEYUP {
            return Ok(());
        }
        if key_code != 0 {
            return Ok(()); // 控制键：mock 不模拟
        }
        let ch = char::from_u32(char_code as u32).unwrap_or('\u{FFFD}');
        let line = &mut self.lines[self.current_line];
        if self.selected {
            line.clear();
            self.selected = false;
        }
        line.push(ch);
        self.mutate();
        Ok(())
    }

    fn post_mouse(
        &mut self,
        kind: i32,
        x: i32,
        y: i32,
        count: i32,
        _buttons: i32,
        _modifiers: i32,
    ) -> LokResult<()> {
        self.hang_if_requested("post_mouse");
        if kind != lok_enums::MOUSEEVENT_BUTTON_DOWN {
            return Ok(());
        }
        if x < 0 || y < 0 {
            return Ok(());
        }
        let line = (y / 300) as usize;
        self.current_line = line.min(self.lines.len().saturating_sub(1));
        if count >= 2 {
            self.selected = true; // 双击 = 选中当前行
            self.mutate();
        }
        Ok(())
    }

    fn post_uno(&mut self, command: &str, _args: Option<&str>, _notify: bool) -> LokResult<()> {
        self.hang_if_requested("post_uno");
        match command {
            ".uno:SelectAll" => {
                self.selected = true;
                Ok(())
            }
            ".uno:GoToEndOfDoc" => {
                self.current_line = self.lines.len() - 1;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn get_command_values(&mut self, command: &str) -> LokResult<Option<String>> {
        Ok(Some(format!("{{\"command\":\"{command}\",\"mock\":true}}")))
    }

    fn get_text_selection(&mut self, _mime: &str) -> LokResult<Option<String>> {
        Ok(Some(if self.selected {
            self.lines[self.current_line].clone()
        } else {
            String::new()
        }))
    }

    fn set_client_zoom(&mut self, _px_w: i32, _px_h: i32, _tw_w: i32, _tw_h: i32) -> LokResult<()> {
        Ok(())
    }

    fn save_as(&mut self, url: &str, _format: &str, _options: &str) -> LokResult<i32> {
        self.hang_if_requested("save_as");
        if std::env::var("MDC_LOK_MOCK_SAVE_FAIL").is_ok() {
            self.events.push(LokEvent {
                kind: lok_enums::CALLBACK_ERROR,
                payload: "{\"mock\":\"save-fail-injected\"}".to_string(),
            });
            return Err(LokError::new(ErrorCode::SaveFailed, "mock 注入：保存失败"));
        }
        let path = url.strip_prefix("file://").unwrap_or(url);
        let path = url_to_path(path);
        Self::write_zip(&path, &self.lines)?;
        Ok(1) // 与真实 saveAs 布尔语义一致：非 0 = 成功
    }

    fn drain_events(&mut self) -> Vec<LokEvent> {
        std::mem::take(&mut self.events)
    }

    fn destroy(&mut self) -> LokResult<()> {
        self.loaded_url = None;
        Ok(())
    }
}

/// file: URL → 系统路径（mock save 用；百分号解码）。
fn url_to_path(url: &str) -> String {
    let mut out = Vec::new();
    let bytes = url.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_paint_is_nonblank_and_edit_sensitive() {
        let mut m = MockSession::new();
        let mut buf = vec![0u8; 64 * 48 * 4];
        m.paint_tile(&mut buf, 64, 48, 0, 0, 11906, 16838).unwrap();
        let nonwhite = buf
            .chunks(4)
            .filter(|p| !(p[0] > 240 && p[1] > 240 && p[2] > 240))
            .count();
        assert!(nonwhite > 30, "mock tile 应有非空白像素");
        let before = buf.clone();
        m.post_key(lok_enums::KEYEVENT_KEYINPUT, 'X' as i32, 0)
            .unwrap();
        m.paint_tile(&mut buf, 64, 48, 0, 0, 11906, 16838).unwrap();
        assert_ne!(before, buf, "编辑后 mock tile 像素应变化");
        assert_eq!(m.drain_events().len(), 1, "键入产生一次失效事件");
    }

    #[test]
    fn mock_double_click_selects_then_typing_replaces() {
        let mut m = MockSession::new();
        m.post_mouse(lok_enums::MOUSEEVENT_BUTTON_DOWN, 100, 350, 2, 1, 0)
            .unwrap(); // 第 2 行双击
        let sel = m.get_text_selection("text/plain").unwrap().unwrap();
        assert!(sel.contains("REPLACEME"));
        for ch in "换好了".chars() {
            m.post_key(lok_enums::KEYEVENT_KEYINPUT, ch as i32, 0)
                .unwrap();
        }
        assert!(m.full_text().contains("换好了"));
        assert!(!m.full_text().contains("REPLACEME"));
    }

    #[test]
    fn mock_save_writes_real_zip() {
        let mut m = MockSession::new();
        let dir = std::env::temp_dir().join(format!("mdc-lok-mock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mock-save.docx");
        let rc = m
            .save_as(&format!("file://{}", path.to_string_lossy()), "docx", "")
            .unwrap();
        assert_ne!(rc, 0, "saveAs 布尔语义：非 0=成功");
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..2], b"PK");
        assert!(bytes.windows(4).any(|w| w == b"docu"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
