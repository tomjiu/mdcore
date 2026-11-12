//! LOK 回调事件 typed 解析（#115 比对缺口 #11；C 线 C3 选区高亮/光标
//! 绘制的前置数据面）。
//!
//! 蓝本：ely-lok `events/parse.rs`（L45-131——cursor_of / selection_of /
//! state_of / found_of 四解析器）同构移植。**落点选宿主侧纯函数**（#115
//! 表内两候选之一——worker 事件帧 enrichment 或宿主解析）：循
//! `docx_geom` 解析 INVALIDATE_TILES 的先例（宿主侧单测可覆盖、零 FFI；
//! worker 帧面保持透传原文——enrichment 会让 worker 成为解析语义的
//! 单点，与「worker 薄、语义在宿主」的分工相悖）。
//!
//! 解析对象（真内核 payload 形态，25.2.3.2）：
//! - `INVALIDATE_VISIBLE_CURSOR`（type 1）：裸矩形串 `"x, y, w, h"` 或
//!   JSON `{"relrect": "...", "refpoint": "..."}` / `{"rectangle": "..."}`
//!   ——`relrect` 是**相对当前视口的坐标**（Ely 同款语义：两类坐标并存，
//!   anchor 在场即 relrect 面）；
//! - `TEXT_SELECTION`（type 2）：`";"` 分隔多矩形 + 可选 `"::x, y"` 尾锚
//!   （print-twips 锚点——选区起点的绝对坐标）；
//! - `STATE_CHANGED`（type 8）：`".uno:Cmd=v"` 或 JSON
//!   `{"commandName", "state", "dontcare"}`；
//! - `SEARCH_RESULT_SELECTION`（type 15）：JSON `searchResultSelection`
//!   数组，每命中取其首矩形。
//!
//! 纪律：解析失败返回 `Err`/`None` **不 panic**——内核 payload 是外部
//! 输入（版本间可变），宿主消费面按「无数据 = 不绘制」降级。

use serde_json::Value;

/// 矩形（print twips）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// print-twips 锚点（内核给出才在场——Ely `Anchor` 同构）。
pub type Anchor = Option<(i32, i32)>;

/// 解析失败（payload 非法——消费面降级不绘制，不 panic）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unreadable;

/// 裸矩形串 `"x, y, w, h"` → `Rect`（Ely rect_of 同构；空串 = None）。
fn rect_of(text: &str) -> Result<Option<Rect>, Unreadable> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let mut parts = text.split(',');
    let mut next = || -> Result<i32, Unreadable> {
        parts
            .next()
            .ok_or(Unreadable)?
            .trim()
            .parse()
            .map_err(|_| Unreadable)
    };
    let rect = Rect {
        x: next()?,
        y: next()?,
        w: next()?,
        h: next()?,
    };
    if parts.next().is_some() {
        return Err(Unreadable);
    }
    Ok(Some(rect))
}

/// `"x, y"` → 点（Ely point_of 同构）。
fn point_of(text: &str) -> Result<(i32, i32), Unreadable> {
    let mut parts = text.split(',');
    let mut next = || -> Result<i32, Unreadable> {
        parts
            .next()
            .ok_or(Unreadable)?
            .trim()
            .parse()
            .map_err(|_| Unreadable)
    };
    let point = (next()?, next()?);
    if parts.next().is_some() {
        return Err(Unreadable);
    }
    Ok(point)
}

/// 可见光标（`INVALIDATE_VISIBLE_CURSOR` payload）→（矩形，锚点）。
///
/// 两形态（Ely cursor_of 同构）：
/// - 裸串 `"x, y, w, h"`（无锚）；
/// - JSON：`relrect`+`refpoint`（**相对视口坐标 + 锚**）或 `rectangle`
///   （无锚）。
pub fn cursor_of(text: &str) -> Result<(Option<Rect>, Anchor), Unreadable> {
    if !text.trim_start().starts_with('{') {
        return Ok((rect_of(text)?, None));
    }
    let json: Value = serde_json::from_str(text).map_err(|_| Unreadable)?;
    if json.get("relrect").is_some() || json.get("refpoint").is_some() {
        let rect = rect_of(json["relrect"].as_str().ok_or(Unreadable)?)?.ok_or(Unreadable)?;
        let anchor = point_of(json["refpoint"].as_str().ok_or(Unreadable)?)?;
        Ok((Some(rect), Some(anchor)))
    } else {
        Ok((
            rect_of(json["rectangle"].as_str().ok_or(Unreadable)?)?,
            None,
        ))
    }
}

/// 文本选区（`TEXT_SELECTION` payload）→（矩形集，锚点）。
///
/// `";"` 分隔多矩形；`"::x, y"` 尾部可选锚（print-twips；Ely
/// selection_of 同构）。空矩形段跳过（内核可发空段）。
pub fn selection_of(text: &str) -> Result<(Vec<Rect>, Anchor), Unreadable> {
    let (text, anchor) = match text.split_once("::") {
        Some((rects, point)) => (rects, Some(point_of(point)?)),
        None => (text, None),
    };
    let rects = text
        .split(';')
        .map(rect_of)
        .collect::<Result<Vec<_>, _>>()?;
    Ok((rects.into_iter().flatten().collect(), anchor))
}

/// 状态变更（`STATE_CHANGED` payload）→（命令名, 状态值）。
///
/// 两形态（Ely state_of 同构）：`".uno:Cmd=v"` 裸串；JSON
/// `{"commandName", "state", "dontcare"}`（dontcare → 状态即 MIXED 语义
/// 上层定——此处如实透传原值）。判不出 None（消费面忽略）。
pub fn state_of(text: &str) -> Option<(String, String)> {
    if text.starts_with('{') {
        let json: Value = serde_json::from_str(text).ok()?;
        let name = json["commandName"].as_str()?.to_string();
        if json["dontcare"] == Value::Bool(true) {
            return Some((name, "mixed".to_string()));
        }
        let state = match &json["state"] {
            Value::String(state) => state.clone(),
            other => other.to_string(),
        };
        return Some((name, state));
    }
    let (name, state) = text.split_once('=')?;
    Some((name.to_string(), state.to_string()))
}

/// 搜索命中（`SEARCH_RESULT_SELECTION` payload）→ 每命中的首矩形集。
///
/// JSON `searchResultSelection[].rectangles` 首段（Ely found_of 同构）；
/// 判不出 None。
pub fn found_of(text: &str) -> Option<Vec<Rect>> {
    let json: Value = serde_json::from_str(text).ok()?;
    json["searchResultSelection"]
        .as_array()?
        .iter()
        .map(|found| rect_of(found["rectangles"].as_str()?.split(';').next()?).ok()?)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 裸矩形光标串（25.2.3.2 实测形态）。
    #[test]
    fn cursor_bare_rect() {
        let (rect, anchor) = cursor_of("2840, 1290, 226, 226").unwrap();
        assert_eq!(
            rect,
            Some(Rect {
                x: 2840,
                y: 1290,
                w: 226,
                h: 226
            })
        );
        assert_eq!(anchor, None, "裸串无锚");
    }

    /// JSON relrect+refpoint（相对视口 + 锚）。
    #[test]
    fn cursor_json_relrect_refpoint() {
        let payload = r#"{"relrect": "10, 20, 226, 226", "refpoint": "2840, 1290"}"#;
        let (rect, anchor) = cursor_of(payload).unwrap();
        assert_eq!(
            rect,
            Some(Rect {
                x: 10,
                y: 20,
                w: 226,
                h: 226
            }),
            "relrect = 视口相对坐标"
        );
        assert_eq!(anchor, Some((2840, 1290)), "refpoint = 绝对锚");
    }

    /// JSON rectangle（无锚形态）。
    #[test]
    fn cursor_json_rectangle() {
        let payload = r#"{"rectangle": "1, 2, 3, 4"}"#;
        let (rect, anchor) = cursor_of(payload).unwrap();
        assert_eq!(
            rect,
            Some(Rect {
                x: 1,
                y: 2,
                w: 3,
                h: 4
            })
        );
        assert_eq!(anchor, None);
    }

    /// 坏输入：非法串 Err、非法 JSON Err（不 panic）。
    #[test]
    fn cursor_unreadable_not_panic() {
        assert!(cursor_of("abc").is_err());
        assert!(cursor_of("1, 2, 3").is_err(), "缺段");
        assert!(cursor_of("1, 2, 3, 4, 5").is_err(), "多段");
        assert!(cursor_of(r#"{"relrect": "bad"}"#).is_err());
        assert!(cursor_of(r#"{"nope": 1}"#).is_err(), "无 rectangle 键");
    }

    /// 多矩形 + 尾锚选区串。
    #[test]
    fn selection_multi_rect_anchor() {
        let (rects, anchor) =
            selection_of("100, 200, 300, 40; 100, 240, 300, 40::5680, 1122").unwrap();
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0].y, 200);
        assert_eq!(rects[1].y, 240);
        assert_eq!(anchor, Some((5680, 1122)), ":: 尾锚");
    }

    /// 选区空段/空串（内核可发）——空集不 Err。
    #[test]
    fn selection_empty_segments() {
        let (rects, anchor) = selection_of("").unwrap();
        assert!(rects.is_empty());
        assert_eq!(anchor, None);
        let (rects, _) = selection_of(";;").unwrap();
        assert!(rects.is_empty(), "空段全跳过");
    }

    /// STATE_CHANGED 两形态 + 坏输入 None。
    #[test]
    fn state_changed_two_shapes() {
        assert_eq!(
            state_of(".uno:DocModified=true"),
            Some((".uno:DocModified".into(), "true".into()))
        );
        let payload = r#"{"commandName": ".uno:Bold", "state": "true"}"#;
        assert_eq!(state_of(payload), Some((".uno:Bold".into(), "true".into())));
        // dontcare → mixed（Ely MIXED 语义位）
        let payload = r#"{"commandName": ".uno:Bold", "state": "true", "dontcare": true}"#;
        assert_eq!(
            state_of(payload),
            Some((".uno:Bold".into(), "mixed".into()))
        );
        // 非字符串 state → JSON 序列化形态
        let payload = r#"{"commandName": ".uno:PageCount", "state": 3}"#;
        assert_eq!(
            state_of(payload),
            Some((".uno:PageCount".into(), "3".into()))
        );
        assert_eq!(state_of("没有等号"), None);
        assert_eq!(state_of(r#"{"commandName": 5}"#), None);
    }

    /// 搜索命中 JSON：多命中各取首矩形。
    #[test]
    fn search_found_rects() {
        let payload = r#"{"searchResultSelection": [
            {"part": 0, "rectangles": "100, 200, 300, 40; 500, 600, 100, 40", "matchHighlighted": true},
            {"part": 0, "rectangles": "700, 800, 300, 40"}
        ], "searchString": "锚点"}"#;
        let found = found_of(payload).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].x, 100, "首命中取首矩形");
        assert_eq!(found[1].x, 700);
        assert!(found_of(r#"{"searchResultSelection": []}"#).is_some());
        assert!(found_of("垃圾").is_none());
        assert!(found_of(r#"{"searchResultSelection": [{"rectangles": "坏"}]}"#).is_none());
    }
}
