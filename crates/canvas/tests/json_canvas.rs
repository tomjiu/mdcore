//! JSON Canvas 互通测试（#419——Obsidian 生态）。
//!
//! 验收对应：
//! 1. 导入 Obsidian 生成的 .canvas 文件（规范样例——四节点型 + 边）
//! 2. 导出后在 Obsidian 能打开（结构合规：必需字段齐 + 型正确——
//!    本环境无 Obsidian 实体，以规范结构断言替代，PR 如实声明）
//! 3. 往返一致（导出再导入，对象不丢）
//! 4. 未知字段忽略不报错（向前兼容）
//!
//! 附加钉定：未知节点型跳过、敌意面（缺字段/坏类型/非对象根/坏 JSON）、
//! 重复 id 首个胜、悬空边导出跳过、笔迹导出跳过（登记损失面）。

use mdc_canvas::session::ObjectKind;
use mdc_canvas::{export_json_canvas, import_json_canvas, CanvasObject};

/// Obsidian 规范样例（四节点型 + 带侧别/标签/调色板色的边）。
const SPEC_SAMPLE: &str = r##"{
  "nodes": [
    {"id":"n1","type":"text","text":"第一章 报价说明","x":0,"y":0,"width":260,"height":120,"color":"1"},
    {"id":"n2","type":"file","file":"报价/设备清单.md","x":320,"y":0,"width":300,"height":140},
    {"id":"n3","type":"link","url":"https://example.com/spec","x":320,"y":180,"width":300,"height":80},
    {"id":"g1","type":"group","label":"报价区","x":-40,"y":-60,"width":720,"height":420,"color":"5"}
  ],
  "edges": [
    {"id":"e1","fromNode":"n1","toNode":"n2","fromSide":"right","toSide":"left","label":"包含","color":"3"},
    {"id":"e2","fromNode":"n2","toNode":"n3","toSide":"top"}
  ]
}"##;

#[test]
fn import_spec_sample_four_node_types() {
    let objs = import_json_canvas(SPEC_SAMPLE).expect("规范样例应可导入");
    assert_eq!(objs.len(), 6, "4 节点 + 2 边");

    let by_id = |id: &str| {
        objs.iter()
            .find(|o| o.id == id)
            .unwrap_or_else(|| panic!("{id} 缺失"))
    };

    // text → Node + shape=text + text 正文。
    let n1 = by_id("n1");
    assert_eq!(n1.kind, ObjectKind::Node);
    assert_eq!((n1.x, n1.y, n1.w, n1.h), (0.0, 0.0, 260.0, 120.0));
    assert_eq!(n1.props.get("shape").map(String::as_str), Some("text"));
    assert_eq!(
        n1.props.get("text").map(String::as_str),
        Some("第一章 报价说明")
    );
    assert_eq!(n1.props.get("color").map(String::as_str), Some("1"));

    // file → Node + jc:type/jc:file + 文件名作可见文本。
    let n2 = by_id("n2");
    assert_eq!(n2.kind, ObjectKind::Node);
    assert_eq!(n2.props.get("jc:type").map(String::as_str), Some("file"));
    assert_eq!(
        n2.props.get("jc:file").map(String::as_str),
        Some("报价/设备清单.md")
    );
    assert_eq!(
        n2.props.get("text").map(String::as_str),
        Some("报价/设备清单.md")
    );

    // link → Node + jc:url。
    let n3 = by_id("n3");
    assert_eq!(n3.kind, ObjectKind::Node);
    assert_eq!(n3.props.get("jc:type").map(String::as_str), Some("link"));
    assert_eq!(
        n3.props.get("jc:url").map(String::as_str),
        Some("https://example.com/spec")
    );

    // group → Group + label。
    let g1 = by_id("g1");
    assert_eq!(g1.kind, ObjectKind::Group);
    assert_eq!(g1.props.get("label").map(String::as_str), Some("报价区"));

    // 边：端点 + 侧别 + 标签。
    let e1 = by_id("e1");
    assert_eq!(e1.kind, ObjectKind::Edge);
    assert_eq!(e1.from.as_deref(), Some("n1"));
    assert_eq!(e1.to.as_deref(), Some("n2"));
    assert_eq!(
        e1.props.get("jc:fromSide").map(String::as_str),
        Some("right")
    );
    assert_eq!(e1.props.get("jc:toSide").map(String::as_str), Some("left"));
    assert_eq!(e1.props.get("label").map(String::as_str), Some("包含"));

    // 可选字段缺省（e2 无 fromSide/label）→ props 无该键。
    let e2 = by_id("e2");
    assert!(!e2.props.contains_key("jc:fromSide"));
    assert_eq!(e2.props.get("jc:toSide").map(String::as_str), Some("top"));
}

/// 验收 2：导出结构合规（必需字段齐 + 型正确 + 顶层 nodes/edges）。
#[test]
fn export_shape_conforms_to_spec() {
    let objs = import_json_canvas(SPEC_SAMPLE).unwrap();
    let exported = export_json_canvas(&objs);
    let v: serde_json::Value = serde_json::from_str(&exported).expect("导出应为合法 JSON");
    let nodes = v
        .get("nodes")
        .and_then(|n| n.as_array())
        .expect("顶层 nodes");
    let edges = v
        .get("edges")
        .and_then(|e| e.as_array())
        .expect("顶层 edges");
    assert_eq!(nodes.len(), 4);
    assert_eq!(edges.len(), 2);
    for n in nodes {
        for key in ["id", "type", "x", "y", "width", "height"] {
            assert!(n.get(key).is_some(), "节点缺必需字段 {key}: {n}");
        }
    }
    for e in edges {
        for key in ["id", "fromNode", "toNode"] {
            assert!(e.get(key).is_some(), "边缺必需字段 {key}: {e}");
        }
    }
    // 型还原：n2=file 带 file 字段；n3=link 带 url；g1=group 带 label。
    let n2 = nodes.iter().find(|n| n["id"] == "n2").unwrap();
    assert_eq!(n2["type"], "file");
    assert_eq!(n2["file"], "报价/设备清单.md");
    let n3 = nodes.iter().find(|n| n["id"] == "n3").unwrap();
    assert_eq!(n3["type"], "link");
    assert_eq!(n3["url"], "https://example.com/spec");
    let g1 = nodes.iter().find(|n| n["id"] == "g1").unwrap();
    assert_eq!(g1["type"], "group");
    assert_eq!(g1["label"], "报价区");
    // 边侧别/标签往返。
    let e1 = edges.iter().find(|e| e["id"] == "e1").unwrap();
    assert_eq!(e1["fromSide"], "right");
    assert_eq!(e1["label"], "包含");
}

/// 验收 3：往返一致（导入 → 导出 → 再导入，对象不丢 + 核心字段稳定）。
#[test]
fn roundtrip_objects_survive() {
    let first = import_json_canvas(SPEC_SAMPLE).unwrap();
    let second = import_json_canvas(&export_json_canvas(&first)).unwrap();
    assert_eq!(first.len(), second.len(), "对象数不丢");
    for a in &first {
        let b = second
            .iter()
            .find(|o| o.id == a.id)
            .unwrap_or_else(|| panic!("往返丢失对象 {}", a.id));
        assert_eq!(a.kind, b.kind, "{} kind", a.id);
        assert_eq!((a.x, a.y, a.w, a.h), (b.x, b.y, b.w, b.h), "{} 几何", a.id);
        assert_eq!(a.from, b.from, "{} from", a.id);
        assert_eq!(a.to, b.to, "{} to", a.id);
        // 语义 props 稳定（text/label/color/引用）。
        for key in ["text", "label", "color", "jc:file", "jc:url", "jc:type"] {
            assert_eq!(a.props.get(key), b.props.get(key), "{} props[{key}]", a.id);
        }
    }
    // 二次往返幂等（第二次起完全稳定）。
    let third = import_json_canvas(&export_json_canvas(&second)).unwrap();
    assert_eq!(second.len(), third.len());
}

/// 验收 4：未知字段忽略不报错（节点/边/顶层三级）。
#[test]
fn unknown_fields_ignored() {
    let doc = r##"{
      "schemaVersion": "9.9",
      "futureTopLevel": {"anything": true},
      "nodes": [
        {"id":"n1","type":"text","text":"hi","x":0,"y":0,"width":10,"height":10,
         "futureField": "unknown", "anotherFuture": [1,2,3]}
      ],
      "edges": [
        {"id":"e1","fromNode":"n1","toNode":"n1","futureEdgeField": 42}
      ]
    }"##;
    let objs = import_json_canvas(doc).expect("未知字段应忽略");
    assert_eq!(objs.len(), 2);
}

/// 未知节点型跳过（向前兼容——不造伪对象不报错）。
#[test]
fn unknown_node_type_skipped() {
    let doc = r#"{
      "nodes": [
        {"id":"n1","type":"text","text":"ok","x":0,"y":0,"width":10,"height":10},
        {"id":"nx","type":"futureType","x":0,"y":0,"width":10,"height":10}
      ],
      "edges": []
    }"#;
    let objs = import_json_canvas(doc).unwrap();
    assert_eq!(objs.len(), 1, "未知型节点跳过");
    assert_eq!(objs[0].id, "n1");
}

/// 敌意/陈旧面：缺必需字段的节点与边跳过；文件级错误 Err。
#[test]
fn hostile_inputs_skip_or_error() {
    let doc = r#"{
      "nodes": [
        {"id":"ok","type":"text","text":"ok","x":0,"y":0,"width":10,"height":10},
        {"type":"text","text":"缺 id","x":0,"y":0,"width":10,"height":10},
        {"id":"no-type","x":0,"y":0,"width":10,"height":10},
        {"id":"bad-geo","type":"text","x":null,"y":0,"width":10,"height":10},
        "纯字符串节点项",
        {"id":"dup","type":"text","text":"首个","x":0,"y":0,"width":10,"height":10},
        {"id":"dup","type":"text","text":"第二个","x":9,"y":9,"width":10,"height":10}
      ],
      "edges": [
        {"id":"e-ok","fromNode":"ok","toNode":"ok"},
        {"id":"e-no-to","fromNode":"ok"}
      ]
    }"#;
    let objs = import_json_canvas(doc).expect("节点/边级问题应跳过不报错");
    // 存活：ok 节点 + dup（首个胜）+ e-ok。
    assert_eq!(
        objs.len(),
        3,
        "{:?}",
        objs.iter().map(|o| o.id.clone()).collect::<Vec<_>>()
    );
    assert!(objs.iter().all(|o| o.id != "not-obj"));
    let dup = objs.iter().find(|o| o.id == "dup").unwrap();
    assert_eq!(
        dup.props.get("text").map(String::as_str),
        Some("首个"),
        "重复 id 首个胜"
    );

    // 文件级错误。
    assert!(import_json_canvas("{ 坏 JSON").is_err(), "坏 JSON");
    assert!(import_json_canvas("[1,2,3]").is_err(), "根非对象");
    assert!(
        import_json_canvas(r#"{"edges":[]}"#).is_err(),
        "缺 nodes 数组"
    );
    // 空 nodes 合法（空画布）。
    assert_eq!(import_json_canvas(r#"{"nodes":[]}"#).unwrap().len(), 0);
    // edges 缺省合法（规范可选）。
    assert_eq!(
        import_json_canvas(
            r#"{"nodes":[{"id":"n","type":"text","text":"","x":0,"y":0,"width":1,"height":1}]}"#
        )
        .unwrap()
        .len(),
        1
    );
}

/// 导出损失面登记：笔迹跳过 + 悬空边跳过；其余对象照导。
#[test]
fn export_lossy_edges_registered() {
    let objs = vec![
        CanvasObject {
            id: "s1".into(),
            kind: ObjectKind::Stroke,
            points: vec![(0.0, 0.0), (5.0, 5.0)],
            color: Some("#000".into()),
            width: Some(2.0),
            tool: Some(mdc_canvas::StrokeTool::Pen),
            ..Default::default()
        },
        CanvasObject {
            id: "dangling".into(),
            kind: ObjectKind::Edge,
            from: None,
            to: Some("n1".into()),
            ..Default::default()
        },
        CanvasObject {
            id: "n1".into(),
            kind: ObjectKind::Node,
            x: 1.0,
            y: 2.0,
            w: 3.0,
            h: 4.0,
            ..Default::default()
        },
    ];
    let exported = export_json_canvas(&objs);
    let v: serde_json::Value = serde_json::from_str(&exported).unwrap();
    let nodes = v["nodes"].as_array().unwrap();
    let edges = v["edges"].as_array().unwrap();
    assert_eq!(nodes.len(), 1, "笔迹跳过、节点照导");
    assert_eq!(edges.len(), 0, "悬空边跳过");
    // 整值几何整型化（Obsidian 常见形态）。
    assert_eq!(v["nodes"][0]["x"], 1);
    // 非整几何保持浮点。
    let objs2 = vec![CanvasObject {
        id: "f".into(),
        kind: ObjectKind::Node,
        x: 1.5,
        y: 2.5,
        w: 3.5,
        h: 4.5,
        ..Default::default()
    }];
    let v2: serde_json::Value = serde_json::from_str(&export_json_canvas(&objs2)).unwrap();
    assert_eq!(v2["nodes"][0]["x"], 1.5);
}

/// mdc 自有属性导出丢失属格式边界——对象本体（id/kind/几何）不丢。
#[test]
fn export_drops_freeform_props_but_keeps_identity() {
    let mut props = std::collections::BTreeMap::new();
    props.insert("shape".to_string(), "rect".to_string());
    props.insert("fill".to_string(), "#ff0000".to_string());
    let objs = vec![CanvasObject {
        id: "n9".into(),
        kind: ObjectKind::Node,
        x: 0.0,
        y: 0.0,
        w: 100.0,
        h: 50.0,
        props,
        ..Default::default()
    }];
    let back = import_json_canvas(&export_json_canvas(&objs)).unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].id, "n9");
    assert_eq!(back[0].kind, ObjectKind::Node);
    assert_eq!((back[0].x, back[0].w), (0.0, 100.0));
    // 自由样式键不往返（JSON Canvas 无承载面——对象不丢的口径内）。
    assert_eq!(back[0].props.get("fill"), None);
}
