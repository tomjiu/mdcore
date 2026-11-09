//! worker ↔ 宿主 JSON-lines 协议（issue 第 1 条：宿主通过请求 ID、帧长
//! 限制、超时与明确错误消费响应）。
//!
//! ## 帧形态（每行一帧，UTF-8 JSON，换行符 `\n`）
//!
//! - 请求（宿主 → worker）：`{"id":N,"op":"...","timeout_ms":可选,...}`
//! - 应答（worker → 宿主）：`{"id":N,"ok":true,...}` 或
//!   `{"id":N,"ok":false,"error":{"code":"...","message":"..."}}`
//! - 事件（worker → 宿主，回调/生命周期，无请求 ID）：
//!   `{"ev":"cb","type":N,"name":"...","payload":"..."}`
//!
//! ## 帧长上限
//!
//! [`crate::MAX_FRAME_BYTES`]（默认 2 MiB）。**读侧在分配前检查长度**
//! （ElyOffice `ely-ipc` 教训：先分配再检查会被恶意长度打爆内存）；
//! 写侧应答超限 → 回 `E_OVERSIZE` 错误帧且 worker 存活（错误文本提示
//! 缩小请求面，比如只取选区而不取全文档）；事件超限 → 截断 payload
//! 保事件名（诊断价值优先）。tile 像素一律走 side file，不走帧。
//!
//! ## 错误码（明确错误，不静默）
//!
//! | code | 含义 |
//! |---|---|
//! | `E_BAD_REQUEST` | 帧不是合法请求（字段缺/类型错） |
//! | `E_OVERSIZE` | 应答超帧限（worker 存活） |
//! | `E_NO_DOCUMENT` | 无打开文档（请求需要文档） |
//! | `E_LOK_LOAD` | 内核库加载 / hook 解析失败 |
//! | `E_LOK_INIT` | hook_2 初始化失败（LO 内部错误） |
//! | `E_LOAD` | documentLoad 失败（附 kit getError 原文） |
//! | `E_SAVE` | saveAs 失败（附 kit getError 原文） |
//! | `E_MEMBER` | ABI 成员缺失（nSize 守卫拦截——运行时太老） |
//! | `E_IO` | side file / 文件系统错误 |
//! | `E_INTERNAL` | worker 内部不变量被破坏 |
//!
//! 宿主侧错误（不出现在线上，client 内部产生）：`E_TIMEOUT`（超时，
//! worker 被杀）、`E_WORKER_DIED`（worker 异常退出，附 stderr 尾部）、
//! `E_OVERSIZE`（请求行超限拒发）。

use crate::MAX_FRAME_BYTES;
use serde_json::{json, Value};
use std::io::BufRead as _;

/// 错误码常量。
pub mod codes {
    pub const BAD_REQUEST: &str = "E_BAD_REQUEST";
    pub const OVERSIZE: &str = "E_OVERSIZE";
    pub const NO_DOCUMENT: &str = "E_NO_DOCUMENT";
    pub const LOK_LOAD: &str = "E_LOK_LOAD";
    pub const LOK_INIT: &str = "E_LOK_INIT";
    pub const LOAD: &str = "E_LOAD";
    pub const SAVE: &str = "E_SAVE";
    pub const MEMBER: &str = "E_MEMBER";
    pub const IO: &str = "E_IO";
    pub const INTERNAL: &str = "E_INTERNAL";
    // 宿主侧（client 本地产生，不在线上）
    pub const TIMEOUT: &str = "E_TIMEOUT";
    pub const WORKER_DIED: &str = "E_WORKER_DIED";
}

/// 读一行（帧）并施加帧长上限。
///
/// 返回 `Ok(None)` = 干净 EOF（仅在行边界才算干净——帧头读一半算异常，
/// ElyOffice 教训）；`Err(超限)` = 行超过 [`MAX_FRAME_BYTES`]（**分配前**
/// 拒绝）。
pub fn read_frame<R: std::io::BufRead>(r: &mut R) -> std::io::Result<Option<Vec<u8>>> {
    let mut buf = Vec::new();
    // 读侧分配前拒帧：take 上限 = MAX + 换行 + 越界探针。
    let mut reader = std::io::Read::take(&mut *r, MAX_FRAME_BYTES as u64 + 2);
    let n = reader.read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(None); // 干净 EOF（行边界）
    }
    // 剔除尾部换行后再量（恰好 MAX 内容 + 换行 = 合法临界帧）
    let mut content_end = buf.len();
    while content_end > 0 && (buf[content_end - 1] == b'\n' || buf[content_end - 1] == b'\r') {
        content_end -= 1;
    }
    if content_end > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("帧超过上限 {MAX_FRAME_BYTES} 字节（读侧分配前拒绝；tile 像素请走 side file）"),
        ));
    }
    buf.truncate(content_end);
    Ok(Some(buf))
}

/// 写一帧（自动施加帧长上限检查由调用方决定——见 [`answer_or_oversize`]）。
pub fn write_frame<W: std::io::Write>(w: &mut W, v: &Value) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(v)?;
    line.push(b'\n');
    w.write_all(&line)?;
    w.flush()
}

/// 序列化应答；超限则改回 `E_OVERSIZE` 错误帧（worker 存活）。
pub fn answer_or_oversize(id: u64, ok_value: Value) -> Value {
    if serde_json::to_vec(&ok_value)
        .map(|v| v.len())
        .unwrap_or(usize::MAX)
        <= MAX_FRAME_BYTES
    {
        return ok_value;
    }
    err_frame(
        id,
        codes::OVERSIZE,
        &format!(
            "应答序列化后超过 {MAX_FRAME_BYTES} 字节上限（缩小请求面后重试，\
             例如只取选区而非全文档）"
        ),
    )
}

/// 成功应答帧：`{"id":N,"ok":true,...}`。
pub fn ok_frame(id: u64, extra: Value) -> Value {
    let mut v = json!({"id": id, "ok": true});
    if let (Some(obj), Some(ex)) = (v.as_object_mut(), extra.as_object()) {
        for (k, val) in ex {
            obj.insert(k.clone(), val.clone());
        }
    }
    v
}

/// 错误应答帧：`{"id":N,"ok":false,"error":{"code","message"}}`。
pub fn err_frame(id: u64, code: &str, message: &str) -> Value {
    json!({"id": id, "ok": false, "error": {"code": code, "message": message}})
}

/// 回调事件帧（payload 超限截断到字符边界，事件名保留）。
pub fn cb_event_frame(type_id: i32, name: &str, payload: &str) -> Value {
    let budget = MAX_FRAME_BYTES / 2;
    let truncated = payload.len() > budget;
    let shown = if truncated {
        let mut end = budget;
        while end > 0 && !payload.is_char_boundary(end) {
            end -= 1;
        }
        &payload[..end]
    } else {
        payload
    };
    json!({
        "ev": "cb",
        "type": type_id,
        "name": name,
        "payload": shown,
        "payload_truncated": truncated,
    })
}

/// 生命周期事件帧（hello / bye / died 等）。
pub fn life_event_frame(kind: &str, detail: Value) -> Value {
    json!({"ev": kind, "detail": detail})
}

/// 从请求帧取字段（worker 侧校验用）。
pub fn req_str(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

pub fn req_u64(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(Value::as_u64)
}

pub fn req_i32(v: &Value, key: &str) -> Option<i32> {
    v.get(key)
        .and_then(Value::as_i64)
        .and_then(|x| i32::try_from(x).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let mut buf = std::io::Cursor::new(b"{\"id\":1,\"op\":\"ping\"}\n".to_vec());
        let frame = read_frame(&mut buf).unwrap().unwrap();
        let v: Value = serde_json::from_slice(&frame).unwrap();
        assert_eq!(v["op"], "ping");
        assert_eq!(v["id"], 1);
    }

    #[test]
    fn clean_eof_only_at_line_boundary() {
        let mut buf = std::io::Cursor::new(b"".to_vec());
        assert!(read_frame(&mut buf).unwrap().is_none());
        // 帧读一半（无换行但有字节）→ 有内容但无换行：返回该行（可能非法
        // JSON，由解析层报 E_BAD_REQUEST）；只有再次读返回 None 才算干净 EOF
        let mut half = std::io::Cursor::new(b"{\"id\"".to_vec());
        assert!(read_frame(&mut half).unwrap().is_some());
        assert!(read_frame(&mut half).unwrap().is_none());
    }

    #[test]
    fn oversize_rejected_before_alloc() {
        // 构造 > MAX_FRAME_BYTES 的单行：take 上限读满即拒
        let big = vec![b'a'; MAX_FRAME_BYTES + 10];
        let mut buf = std::io::Cursor::new(big);
        let err = read_frame(&mut buf).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("上限"));
    }

    #[test]
    fn ok_and_err_frames() {
        assert_eq!(ok_frame(7, json!({"pong": true}))["ok"], true);
        assert_eq!(ok_frame(7, json!({"pong": true}))["pong"], true);
        let e = err_frame(8, codes::SAVE, "boom");
        assert_eq!(e["id"], 8);
        assert_eq!(e["error"]["code"], "E_SAVE");
        assert_eq!(e["error"]["message"], "boom");
    }

    #[test]
    fn answer_oversize_downgrades_to_error_frame() {
        let huge = "字".repeat(MAX_FRAME_BYTES / 2 + 64);
        let v = answer_or_oversize(1, json!({"text": huge}));
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"]["code"], "E_OVERSIZE");
    }

    #[test]
    fn cb_event_truncates() {
        let huge = "a".repeat(MAX_FRAME_BYTES);
        let f = cb_event_frame(0, "INVALIDATE_TILES", &huge);
        assert_eq!(f["payload_truncated"], true);
        assert!(f["payload"].as_str().unwrap().len() < MAX_FRAME_BYTES);
        let normal = cb_event_frame(16, "UNO_COMMAND_RESULT", r#"{"success":true}"#);
        assert_eq!(normal["payload_truncated"], false);
    }
}
