//! JSON Canvas 互通（#419——Obsidian 生态）。
//!
//! # 定位
//!
//! JSON Canvas 是 Obsidian 发起的开放画布格式（节点/连线 + 平面几何，
//! 结构简单全公开）。本模块把 `.canvas` 文件与 [`CanvasObject`] 读模型
//! 互译——**数据层转换**（不入会话；调用方按需经命令面装载）。
//!
//! # 映射表
//!
//! | JSON Canvas | mdc-canvas | 说明 |
//! |---|---|---|
//! | node type=text | `Node` + props{shape:text, text} | 渲染协议直读 |
//! | node type=file | `Node` + props{jc:file, text} | 文件引用进 props 通道；文件名作可见文本 |
//! | node type=link | `Node` + props{jc:url, text} | URL 同上 |
//! | node type=group | `Group` + props{label} | 空间区域语义（Group 无显式成员——与 JSON Canvas 同为空间隐含）；#404 frame 合入后可升级映射 |
//! | edge | `Edge` + props{label/color/jc:fromSide/jc:toSide} | 端点悬空合法（读模型既有纪律） |
//! | stroke | （导出跳过） | JSON Canvas 无笔迹原语——导出损失面，登记 |
//! | 边 caption | `Edge` + `jc:caption` 键 | #61 裁决落定案 B：专键往返（typed 单通道——导出读 caption 字段、导入还原字段），字节不丢；互操作可见性走手工单向转 label（可逆），label 语义（#447 渲染消费）不污染。旧损失面登记见 #636 |
//!
//! # 兼容纪律
//!
//! - **未知字段忽略不报错**（验收 4——规范向前兼容面）；
//! - **未知节点型跳过**（同上——不造伪对象，导出侧无法还原）；
//! - **缺必需字段的节点/连线跳过**（敌意/陈旧面防御——不 panic）；
//! - 重复 id：首个胜（确定性）；
//! - 根解析失败 / 根非对象 / 缺 nodes 数组 = `Err`（文件级错误，调用
//!   方可提示用户）；
//! - 非 mdc 属性（fill/shape 等样式自由键）导出即丢——JSON Canvas
//!   无承载面，对象本体（id/kind/几何/端点）不丢（验收 3 口径）。
//!
//! # props 通道键（`jc:` 前缀 = JSON Canvas 专有）
//!
//! 与样式属性区隔；`jc:type` 保留原始节点型（file/link——text/group
//! 由 kind 本身承载），AI 检索可凭 `props["jc:type"]` 判来源。

use std::collections::{BTreeMap, HashSet};

use serde_json::{json, Value};

use crate::session::{CanvasObject, ObjectKind, Side};
use crate::CanvasError;

/// 原始节点型标记（file / link）。
const P_TYPE: &str = "jc:type";
/// file 节点的库内路径。
const P_FILE: &str = "jc:file";
/// link 节点的 URL。
const P_URL: &str = "jc:url";
/// 边起点侧（left/right/top/bottom）。
const P_FROM_SIDE: &str = "jc:fromSide";
/// 边终点侧。
const P_TO_SIDE: &str = "jc:toSide";
/// #61 案 B：边说明专键（caption 一等字段的 JSON Canvas 承载面
/// ——规范边仅 label 无 caption 原语，专键往返保信息不丢）。
const P_JC_CAPTION: &str = "jc:caption";
/// 标签（节点 label / 边 label 共用键——渲染协议既有键）。
const P_LABEL: &str = "label";
/// Obsidian 调色板色（"1"-"6"）。
const P_COLOR: &str = "color";

/// 导入 JSON Canvas（`.canvas` 文件正文）→ 对象读模型。
///
/// 节点/连线级问题（缺字段/未知型/非有限几何）逐个跳过不报错；文件
/// 级问题（坏 JSON / 根非对象 / 缺 nodes）返回 `Err`。
pub fn import_json_canvas(json: &str) -> Result<Vec<CanvasObject>, CanvasError> {
    let root: Value = serde_json::from_str(json)
        .map_err(|e| CanvasError::Decode(format!("JSON Canvas 根解析失败：{e}")))?;
    let Some(nodes) = root.get("nodes").and_then(|v| v.as_array()) else {
        return Err(CanvasError::Decode(
            "JSON Canvas 根缺 nodes 数组（规范必填）".into(),
        ));
    };
    let edges = root
        .get("edges")
        .and_then(|v| v.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    let mut out = Vec::with_capacity(nodes.len() + edges.len());
    let mut seen: HashSet<String> = HashSet::new();
    for n in nodes {
        if let Some(obj) = import_node(n) {
            if seen.insert(obj.id.clone()) {
                out.push(obj);
            }
        }
    }
    for e in edges {
        if let Some(obj) = import_edge(e) {
            if seen.insert(obj.id.clone()) {
                out.push(obj);
            }
        }
    }
    Ok(out)
}

/// 单节点导入（None = 跳过：未知型/缺字段/非有限几何）。
fn import_node(v: &Value) -> Option<CanvasObject> {
    let obj = v.as_object()?;
    // 必需字段（规范口径）：id/type/x/y/width/height。
    let id = obj.get("id")?.as_str()?.to_string();
    let ty = obj.get("type")?.as_str()?;
    let x = obj.get("x")?.as_f64()?;
    let y = obj.get("y")?.as_f64()?;
    let w = obj.get("width")?.as_f64()?;
    let h = obj.get("height")?.as_f64()?;
    if !(x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite()) {
        return None;
    }
    let mut props = BTreeMap::new();
    if let Some(color) = obj.get("color").and_then(|v| v.as_str()) {
        props.insert(P_COLOR.into(), color.into());
    }
    let (kind, text) = match ty {
        "text" => (
            ObjectKind::Node,
            obj.get("text").and_then(|v| v.as_str()).map(str::to_string),
        ),
        "file" => {
            let file = obj.get("file").and_then(|v| v.as_str()).map(str::to_string);
            props.insert(P_TYPE.into(), "file".into());
            if let Some(f) = &file {
                props.insert(P_FILE.into(), f.clone());
            }
            (ObjectKind::Node, file)
        }
        "link" => {
            let url = obj.get("url").and_then(|v| v.as_str()).map(str::to_string);
            props.insert(P_TYPE.into(), "link".into());
            if let Some(u) = &url {
                props.insert(P_URL.into(), u.clone());
            }
            (ObjectKind::Node, url)
        }
        "group" => {
            if let Some(label) = obj.get("label").and_then(|v| v.as_str()) {
                props.insert(P_LABEL.into(), label.into());
            }
            (ObjectKind::Group, None)
        }
        // 未知节点型：向前兼容跳过（不造伪对象）。
        _ => return None,
    };
    if kind == ObjectKind::Node {
        // 渲染协议：shape=text 读 text/label 键——导入统一落 text
        //（正文 / 文件引用 / URL 作可见内容）。
        props.insert("shape".into(), "text".into());
        props.insert("text".into(), text.unwrap_or_default());
    }
    Some(CanvasObject {
        id,
        kind,
        x,
        y,
        w,
        h,
        props,
        ..Default::default()
    })
}

/// 单边导入（None = 跳过：缺 id/fromNode/toNode）。端点不校验存在性
/// （悬空边在读模型合法——渲染跳过，同 Edge 既有纪律）。
///
/// #602：fromSide/toSide 双写——①typed 字段（渲染/路由读的一面，
/// `Side::from_wire` 解析）；②props `jc:fromSide`/`jc:toSide` 原串
/// （旧快照回落面 + 未知值不丢的往返保真）。非法 side：typed None
/// （渲染回落中心——既有容错约定），props 保留原串（向前兼容：未来
/// 规范新值不丢）。
fn import_edge(v: &Value) -> Option<CanvasObject> {
    let obj = v.as_object()?;
    let id = obj.get("id")?.as_str()?.to_string();
    let from = obj.get("fromNode")?.as_str()?.to_string();
    let to = obj.get("toNode")?.as_str()?.to_string();
    let mut props = BTreeMap::new();
    if let Some(l) = obj.get("label").and_then(|v| v.as_str()) {
        props.insert(P_LABEL.into(), l.into());
    }
    if let Some(c) = obj.get("color").and_then(|v| v.as_str()) {
        props.insert(P_COLOR.into(), c.into());
    }
    if let Some(s) = obj.get("fromSide").and_then(|v| v.as_str()) {
        props.insert(P_FROM_SIDE.into(), s.into());
    }
    if let Some(s) = obj.get("toSide").and_then(|v| v.as_str()) {
        props.insert(P_TO_SIDE.into(), s.into());
    }
    let from_side = obj
        .get("fromSide")
        .and_then(|v| v.as_str())
        .and_then(Side::from_wire);
    let to_side = obj
        .get("toSide")
        .and_then(|v| v.as_str())
        .and_then(Side::from_wire);
    // #61 案 B：jc:caption 专键 → caption 一等字段（typed 单通道，
    // 与导出侧对称——不进 props，免双源分叉）。
    let caption = obj
        .get(P_JC_CAPTION)
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some(CanvasObject {
        id,
        kind: ObjectKind::Edge,
        from: Some(from),
        to: Some(to),
        from_side,
        to_side,
        caption,
        props,
        ..Default::default()
    })
}

/// 导出为 JSON Canvas（`.canvas` 文件正文，pretty JSON）。
///
/// - 笔迹对象跳过（JSON Canvas 无笔迹原语——登记损失面）；
/// - 悬空边（from/to 缺失）跳过（规范端点必非空——导出无效边不如
///   不导）；
/// - 边 caption 走 `jc:caption` 专键（#61 案 B——见上方映射表）；
/// - 非 mdc 样式自由键不导（格式无承载面；对象本体不丢）。
pub fn export_json_canvas(objects: &[CanvasObject]) -> String {
    let mut nodes: Vec<Value> = Vec::new();
    let mut edges: Vec<Value> = Vec::new();
    for o in objects {
        match o.kind {
            ObjectKind::Edge => {
                let (Some(from), Some(to)) = (&o.from, &o.to) else {
                    continue; // 悬空边不导
                };
                let mut e = serde_json::Map::new();
                e.insert("id".into(), json!(o.id));
                e.insert("fromNode".into(), json!(from));
                e.insert("toNode".into(), json!(to));
                for (jk, pk) in [("label", P_LABEL), ("color", P_COLOR)] {
                    if let Some(v) = o.props.get(pk) {
                        e.insert(jk.into(), json!(v));
                    }
                }
                // #61 案 B（裁决落定）：边 caption 走 `jc:caption` 专键
                // ——typed 单通道（导出读一等字段，不进 props；caption
                // 无 #444 时代快照包袱，且纯 string 无「未知枚举值
                // 保真」问题，无需 side 那样的双通道）。互操作可见性
                // 缺口走手工单向转 label（可逆）；反向（label 回填
                // caption）不可逆，故不取。旧损失面登记见 #636。
                if let Some(c) = &o.caption {
                    e.insert(P_JC_CAPTION.into(), json!(c));
                }
                // #602：side 双通道导出——typed 值优先（SetEdgeRoute 后的
                // 新值不被旧 props 覆盖）；typed None + 旧快照 props 有值
                // 时回落（#444 时代快照兼容面）；双缺省不导键（不造伪值）。
                for (typed, pk, jk) in [
                    (o.from_side, P_FROM_SIDE, "fromSide"),
                    (o.to_side, P_TO_SIDE, "toSide"),
                ] {
                    let wire = typed
                        .map(|s| s.wire().to_string())
                        .or_else(|| o.props.get(pk).cloned());
                    if let Some(w) = wire {
                        e.insert(jk.into(), json!(w));
                    }
                }
                edges.push(Value::Object(e));
            }
            ObjectKind::Group => {
                let mut n = base_node_json(o);
                n.insert("type".into(), json!("group"));
                if let Some(l) = o.props.get(P_LABEL) {
                    n.insert("label".into(), json!(l));
                }
                if let Some(c) = o.props.get(P_COLOR) {
                    n.insert("color".into(), json!(c));
                }
                nodes.push(Value::Object(n));
            }
            // #404：frame 无 JSON Canvas 原语——按 group 语义降级导出
            // （几何 + 成员 id 保留；命名入 label）。损失面登记在模块文档。
            ObjectKind::Frame | ObjectKind::Node => {
                let mut n = base_node_json(o);
                match o.props.get(P_TYPE).map(String::as_str) {
                    Some("file") => {
                        n.insert("type".into(), json!("file"));
                        n.insert(
                            "file".into(),
                            json!(o.props.get(P_FILE).cloned().unwrap_or_default()),
                        );
                    }
                    Some("link") => {
                        n.insert("type".into(), json!("link"));
                        n.insert(
                            "url".into(),
                            json!(o.props.get(P_URL).cloned().unwrap_or_default()),
                        );
                    }
                    _ => {
                        // 缺省导出为 text 节点（JSON Canvas 无 rect 原语
                        // ——文本可为空，几何与 id 保留）。
                        n.insert("type".into(), json!("text"));
                        n.insert(
                            "text".into(),
                            json!(o.props.get("text").cloned().unwrap_or_default()),
                        );
                    }
                }
                if let Some(c) = o.props.get(P_COLOR) {
                    n.insert("color".into(), json!(c));
                }
                nodes.push(Value::Object(n));
            }
            // 笔迹：JSON Canvas 无原语——跳过（导出损失面，模块文档登记）。
            ObjectKind::Stroke => continue,
        }
    }
    let doc = json!({"nodes": nodes, "edges": edges});
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{\"nodes\":[],\"edges\":[]}".into())
}

/// 节点公共字段（id/x/y/width/height——整型化输出：规范常见形态，
/// 非整几何保持浮点）。
fn base_node_json(o: &CanvasObject) -> serde_json::Map<String, Value> {
    let mut n = serde_json::Map::new();
    n.insert("id".into(), json!(o.id));
    n.insert("x".into(), num_json(o.x));
    n.insert("y".into(), num_json(o.y));
    n.insert("width".into(), num_json(o.w));
    n.insert("height".into(), num_json(o.h));
    n
}

/// 数值 JSON 化：整值落整型（Obsidian 生态常见形态），非整保持浮点。
fn num_json(v: f64) -> Value {
    if v.fract() == 0. && v.abs() < 9.007_199_254_740_992e15 {
        json!(v as i64)
    } else {
        json!(v)
    }
}
