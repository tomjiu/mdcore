//! stdio 帧协议（#633）。
//!
//! 宿主 ↔ worker 双向帧，二进制定长头 + JSON 头 + 可选二进制尾：
//!
//! ```text
//! magic "MDCLK1"(6B) | kind u8 | flags u8 | req_id u64 LE
//!                    | json_len u32 LE | bin_len u32 LE
//!                    | json(json_len B, UTF-8) | bin(bin_len B)
//! ```
//!
//! 设计依据（ElyOffice ely-ipc 参考实现 MIT + mang-shell JSON Lines
//! id 回显先例，模式参考不整包搬运）：
//! - **请求 ID 关联**：响应帧回显请求 id；worker 主动推送（LOK
//!   callback 事件 / 日志）用 id=0（UNASKED）。
//! - **帧长上限**：JSON 头 ≤ 1 MiB、二进制尾 ≤ 64 MiB；读端在
//!   分配前拒绝超限帧（防恶意/失控 worker 撑爆宿主内存）。
//! - **二进制尾**：paint_tile 的像素原样直传（不 base64 膨胀）。
//! - magic/版本前置：坏流早失败，错误信息含两侧期望值。

use std::io::{Read, Write};

use crate::error::{ErrorCode, LokError, LokResult};

pub const MAGIC: &[u8; 6] = b"MDCLK1";
/// 帧头定长：magic(6) + kind(1) + flags(1) + id(8) + json_len(4) + bin_len(4)。
pub const HEADER_LEN: usize = 24;
/// JSON 头上限（请求 / 响应元数据 / 事件载荷）。
pub const MAX_JSON_LEN: u32 = 1024 * 1024;
/// 二进制尾上限（单块 tile 像素；256×256 BGRA=256KB，整页 2×A4@1x 亦覆盖）。
pub const MAX_BIN_LEN: u32 = 64 * 1024 * 1024;
/// 主动推送（事件/日志）帧的固定请求 ID。
pub const UNASKED: u64 = 0;

/// 帧类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    Request = 1,
    Response = 2,
    Event = 3,
    Log = 4,
}

impl FrameKind {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(FrameKind::Request),
            2 => Some(FrameKind::Response),
            3 => Some(FrameKind::Event),
            4 => Some(FrameKind::Log),
            _ => None,
        }
    }
}

/// 编码后的帧（拥有数据）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameBuf {
    pub kind: FrameKind,
    pub id: u64,
    pub json: Vec<u8>,
    pub bin: Vec<u8>,
}

impl FrameBuf {
    pub fn new(kind: FrameKind, id: u64, json: Vec<u8>, bin: Vec<u8>) -> Self {
        FrameBuf {
            kind,
            id,
            json,
            bin,
        }
    }

    pub fn from_json_value(
        kind: FrameKind,
        id: u64,
        value: &serde_json::Value,
        bin: Vec<u8>,
    ) -> Self {
        let json = serde_json::to_vec(value).expect("serde_json::to_vec 不可失败");
        FrameBuf {
            kind,
            id,
            json,
            bin,
        }
    }

    /// 序列化为线上字节（头 + JSON + bin）。
    pub fn encode(&self) -> LokResult<Vec<u8>> {
        if self.json.len() > MAX_JSON_LEN as usize {
            return Err(LokError::new(
                ErrorCode::BadFrame,
                format!("帧 JSON 头超限：{} > {}", self.json.len(), MAX_JSON_LEN),
            ));
        }
        if self.bin.len() > MAX_BIN_LEN as usize {
            return Err(LokError::new(
                ErrorCode::BadFrame,
                format!("帧二进制尾超限：{} > {}", self.bin.len(), MAX_BIN_LEN),
            ));
        }
        let mut out = Vec::with_capacity(HEADER_LEN + self.json.len() + self.bin.len());
        out.extend_from_slice(MAGIC);
        out.push(self.kind as u8);
        out.push(0); // flags：保留，恒 0
        out.extend_from_slice(&self.id.to_le_bytes());
        out.extend_from_slice(&(self.json.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.json);
        out.extend_from_slice(&self.bin);
        Ok(out)
    }
}

/// 从流中读取一帧（阻塞）。EOF 于帧首字节处返回 `Ok(None)`（对端正常
/// 关闭）；EOF 于帧中间或任何头违规返回 `Err(BadFrame)`。
pub fn read_frame<R: Read>(r: &mut R) -> LokResult<Option<FrameBuf>> {
    let mut header = [0u8; HEADER_LEN];
    match r.read_exact(&mut header) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    if &header[0..6] != MAGIC {
        return Err(LokError::new(
            ErrorCode::BadFrame,
            format!("帧 magic 不符：期望 {:?} 实得 {:?}", MAGIC, &header[0..6]),
        ));
    }
    let kind = FrameKind::from_u8(header[6])
        .ok_or_else(|| LokError::new(ErrorCode::BadFrame, format!("未知帧类别 {}", header[6])))?;
    if header[7] != 0 {
        return Err(LokError::new(
            ErrorCode::BadFrame,
            format!("帧 flags 非 0：{}", header[7]),
        ));
    }
    let id = u64::from_le_bytes(header[8..16].try_into().unwrap());
    let json_len = u32::from_le_bytes(header[16..20].try_into().unwrap());
    let bin_len = u32::from_le_bytes(header[20..24].try_into().unwrap());
    if json_len > MAX_JSON_LEN {
        return Err(LokError::new(
            ErrorCode::BadFrame,
            format!("帧 JSON 头超限：{json_len} > {MAX_JSON_LEN}"),
        ));
    }
    if bin_len > MAX_BIN_LEN {
        return Err(LokError::new(
            ErrorCode::BadFrame,
            format!("帧二进制尾超限：{bin_len} > {MAX_BIN_LEN}"),
        ));
    }
    let mut json = vec![0u8; json_len as usize];
    let mut bin = vec![0u8; bin_len as usize];
    r.read_exact(&mut json)
        .map_err(|e| LokError::new(ErrorCode::BadFrame, format!("帧 JSON 段截断：{e}")))?;
    r.read_exact(&mut bin)
        .map_err(|e| LokError::new(ErrorCode::BadFrame, format!("帧二进制段截断：{e}")))?;
    Ok(Some(FrameBuf {
        kind,
        id,
        json,
        bin,
    }))
}

/// 写一帧并 flush。
pub fn write_frame<W: Write>(w: &mut W, frame: &FrameBuf) -> LokResult<()> {
    let bytes = frame.encode()?;
    w.write_all(&bytes)?;
    w.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(frame: &FrameBuf) -> FrameBuf {
        let bytes = frame.encode().unwrap();
        let mut cur = std::io::Cursor::new(bytes);
        read_frame(&mut cur).unwrap().unwrap()
    }

    #[test]
    fn frame_roundtrip_with_binary_tail() {
        let f = FrameBuf::new(
            FrameKind::Response,
            42,
            br#"{"ok":true}"#.to_vec(),
            vec![1, 2, 3, 255],
        );
        let g = roundtrip(&f);
        assert_eq!(f, g);
    }

    #[test]
    fn unasked_id_for_events() {
        let f = FrameBuf::new(FrameKind::Event, UNASKED, br#"{"type":0}"#.to_vec(), vec![]);
        assert_eq!(roundtrip(&f), f);
    }

    #[test]
    fn eof_at_boundary_is_none() {
        let mut cur = std::io::Cursor::new(Vec::<u8>::new());
        assert_eq!(read_frame(&mut cur).unwrap(), None);
    }

    #[test]
    fn bad_magic_rejected() {
        let mut bytes = FrameBuf::new(FrameKind::Request, 1, b"{}".to_vec(), vec![])
            .encode()
            .unwrap();
        bytes[0] = b'X';
        let mut cur = std::io::Cursor::new(bytes);
        let err = read_frame(&mut cur).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadFrame);
    }

    #[test]
    fn oversized_json_header_rejected_before_alloc() {
        let mut header = [0u8; HEADER_LEN];
        header[0..6].copy_from_slice(MAGIC);
        header[6] = 1;
        header[16..20].copy_from_slice(&(MAX_JSON_LEN + 1).to_le_bytes());
        let mut cur = std::io::Cursor::new(header.to_vec());
        let err = read_frame(&mut cur).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadFrame);
        assert!(err.message.contains("超限"));
    }

    #[test]
    fn oversized_bin_rejected_before_alloc() {
        let mut header = [0u8; HEADER_LEN];
        header[0..6].copy_from_slice(MAGIC);
        header[6] = 1;
        header[20..24].copy_from_slice(&(MAX_BIN_LEN + 1).to_le_bytes());
        let mut cur = std::io::Cursor::new(header.to_vec());
        assert_eq!(read_frame(&mut cur).unwrap_err().code, ErrorCode::BadFrame);
    }

    #[test]
    fn truncated_payload_rejected() {
        let bytes = FrameBuf::new(FrameKind::Request, 7, br#"{"a":1}"#.to_vec(), vec![9; 10])
            .encode()
            .unwrap();
        let mut cur = std::io::Cursor::new(&bytes[..bytes.len() - 5]);
        assert_eq!(read_frame(&mut cur).unwrap_err().code, ErrorCode::BadFrame);
    }

    #[test]
    fn unknown_kind_rejected() {
        let mut bytes = FrameBuf::new(FrameKind::Request, 1, b"{}".to_vec(), vec![])
            .encode()
            .unwrap();
        bytes[6] = 9;
        let mut cur = std::io::Cursor::new(bytes);
        assert_eq!(read_frame(&mut cur).unwrap_err().code, ErrorCode::BadFrame);
    }
}
