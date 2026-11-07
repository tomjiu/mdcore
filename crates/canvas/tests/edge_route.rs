//! 富 connector 测试（#406——吸附锚点 + 路由 + caption）。
//!
//! 验收对应：
//! 1. 边从指定边中点出发（side_anchor——缺省中心向后兼容）
//! 2. caption 渲染在边中点（from_objects 的 Text 发射 + tf.origin 锚）
//! 3. 移动节点时边跟随（锚点由节点几何现算——无存点，MoveNode 后
//!    渲染自动重算）
//!
//! 附加钉定：直角路由形状（stub 引出 + 单肘 + 共线折叠）、
//! SetEdgeRoute 命令语义（终态替换/None 清除/kind 门禁/空串拒收）、
//! 指纹、快照往返、wasm 桥。

use mdc_canvas::render::{CanvasRendering, Shape};
use mdc_canvas::{
    Actor, CanvasCommand, CanvasError, CanvasSession, ObjectKind, SessionOptions, Side,
};

fn session(id: u64) -> CanvasSession {
    CanvasSession::open(SessionOptions {
        client_id: id,
        ..Default::default()
    })
    .expect("open")
}

/// 两节点（左 0,0,100,100；右 300,0,100,100）+ 一条边。
fn two_nodes_with_edge(s: &mut CanvasSession) {
    for (id, x) in [("n1", 0.0), ("n2", 300.0)] {
        s.execute(
            Actor::Human,
            CanvasCommand::AddNode {
                id: Some(id.into()),
                x,
                y: 0.0,
                w: 100.0,
                h: 100.0,
                props: Default::default(),
            },
        )
        .unwrap();
    }
    s.execute(
        Actor::Human,
        CanvasCommand::AddEdge {
            id: Some("e1".into()),
            from: "n1".into(),
            to: "n2".into(),
            props: Default::default(),
        },
    )
    .unwrap();
}

fn set_route(
    s: &mut CanvasSession,
    id: &str,
    from_side: Option<Side>,
    to_side: Option<Side>,
    caption: Option<&str>,
) {
    s.execute(
        Actor::Human,
        CanvasCommand::SetEdgeRoute {
            id: id.into(),
            from_side,
            to_side,
            caption: caption.map(str::to_string),
        },
    )
    .expect("set edge route");
}

/// 验收 1：边从指定边中点出发（缺省中心——向后兼容）。
#[test]
fn edge_anchors_at_side_midpoints() {
    let mut s = session(7_306_001);
    two_nodes_with_edge(&mut s);
    // 缺省：中心直线（既有行为）。
    let r = CanvasRendering::from_objects(&s.objects());
    match &r.shapes[0].0 {
        Shape::Line { from, to, .. } => {
            assert_eq!((from.x, from.y), (50.0, 50.0), "缺省 from = 中心");
            assert_eq!((to.x, to.y), (350.0, 50.0), "缺省 to = 中心");
        }
        other => panic!("缺省应为直线，实际 {other:?}"),
    }

    // 指定侧：right→left 锚点 = 边中点。
    set_route(&mut s, "e1", Some(Side::Right), Some(Side::Left), None);
    let r = CanvasRendering::from_objects(&s.objects());
    match &r.shapes[0].0 {
        Shape::Path { cmds, .. } => {
            // 首点 = n1 右边中点 (100, 50)；末点 = n2 左边中点 (300, 50)。
            match cmds.first() {
                Some(mdc_canvas::render::PathCmd::MoveTo(p)) => {
                    assert_eq!((p.x, p.y), (100.0, 50.0), "from = 右边中点")
                }
                other => panic!("首命令应为 MoveTo，实际 {other:?}"),
            }
            match cmds.last() {
                Some(mdc_canvas::render::PathCmd::LineTo(p)) => {
                    assert_eq!((p.x, p.y), (300.0, 50.0), "to = 左边中点")
                }
                other => panic!("末命令应为 LineTo，实际 {other:?}"),
            }
        }
        other => panic!("双侧指定应为折线，实际 {other:?}"),
    }

    // 单侧指定：直线但从边中点出发。
    set_route(&mut s, "e1", Some(Side::Top), None, None);
    let r = CanvasRendering::from_objects(&s.objects());
    match &r.shapes[0].0 {
        Shape::Line { from, .. } => {
            assert_eq!((from.x, from.y), (50.0, 0.0), "单侧 from = 顶边中点")
        }
        other => panic!("单侧应为直线，实际 {other:?}"),
    }
}

/// 验收 2：caption 渲染在边中点（Text 发射于 tf.origin）。
#[test]
fn caption_renders_at_path_midpoint() {
    let mut s = session(7_306_002);
    two_nodes_with_edge(&mut s);
    set_route(&mut s, "e1", None, None, Some("包含"));
    let r = CanvasRendering::from_objects(&s.objects());
    // 第二个图元 = caption Text（主图元之后——ID 序 e1 先于 n1/n2）。
    assert!(r.shapes.len() >= 4, "主图元 + caption");
    match &r.shapes[1].0 {
        Shape::Text { text, pos, .. } => {
            assert_eq!(text, "包含");
            assert_eq!((pos.x, pos.y), (200.0, 50.0), "caption 在线段中点");
        }
        other => panic!("应为 caption Text，实际 {other:?}"),
    }
    // 主图元仍是 Line（缺省路由）。
    assert!(matches!(&r.shapes[0].0, Shape::Line { .. }));

    // 折线路由的 caption：在折线长度中点上（此例为肘点邻近段上）。
    set_route(
        &mut s,
        "e1",
        Some(Side::Right),
        Some(Side::Left),
        Some("路"),
    );
    let r = CanvasRendering::from_objects(&s.objects());
    match &r.shapes[1].0 {
        Shape::Text { pos, .. } => {
            // 对称场景（right→left 等高）折线 = (100,50)→(124,50)→(276,50)→(300,50)
            // 共线折叠为直线——长度中点 = (200, 50)。
            assert_eq!((pos.x, pos.y), (200.0, 50.0), "折线 caption 在长度中点");
        }
        other => panic!("折线 caption 应为 Text，实际 {other:?}"),
    }
}

/// 验收 3：移动节点时边跟随（锚点现算——MoveNode 后重渲染自动跟）。
#[test]
fn edge_follows_node_movement() {
    let mut s = session(7_306_003);
    two_nodes_with_edge(&mut s);
    set_route(&mut s, "e1", Some(Side::Right), Some(Side::Left), None);

    s.execute(
        Actor::Human,
        CanvasCommand::MoveNode {
            id: "n2".into(),
            x: 500.0,
            y: 200.0,
        },
    )
    .unwrap();
    let r = CanvasRendering::from_objects(&s.objects());
    match &r.shapes[0].0 {
        Shape::Path { cmds, .. } => {
            match cmds.last() {
                Some(mdc_canvas::render::PathCmd::LineTo(p)) => {
                    // n2 移到 (500,200)——左边中点 = (500, 250)。
                    assert_eq!(
                        (p.x, p.y),
                        (500.0, 250.0),
                        "移动后 to 锚跟随（from_side 吸附保持）"
                    );
                }
                other => panic!("{other:?}"),
            }
            // from 锚不动（n1 未动）。
            match cmds.first() {
                Some(mdc_canvas::render::PathCmd::MoveTo(p)) => {
                    assert_eq!((p.x, p.y), (100.0, 50.0));
                }
                other => panic!("{other:?}"),
            }
        }
        other => panic!("移动后仍应折线（双侧指定保持），实际 {other:?}"),
    }
    // 模型面：sides 不随移动变（吸附保持）。
    let e = s.find_object("e1").unwrap();
    assert_eq!(e.from_side, Some(Side::Right));
    assert_eq!(e.to_side, Some(Side::Left));
}

/// 直角路由形状钉定：stub 引出 + 单肘 + 共线折叠。
#[test]
fn orthogonal_route_shape() {
    let mut s = session(7_306_004);
    two_nodes_with_edge(&mut s);
    // 非对称场景：bottom→top（垂直对齐时折叠为直线，错位时单肘）。
    set_route(&mut s, "e1", Some(Side::Bottom), Some(Side::Top), None);
    let r = CanvasRendering::from_objects(&s.objects());
    match &r.shapes[0].0 {
        Shape::Path { cmds, .. } => {
            // n1 bottom 中点 (50,100) stub→(50,124)；n2 top 中点 (350,0) stub→(350,-24)。
            // from 垂直 → 肘 (50,-24) → (350,-24) → (350,0)。
            let pts: Vec<(f64, f64)> = cmds
                .iter()
                .filter_map(|c| match c {
                    mdc_canvas::render::PathCmd::MoveTo(p) => Some((p.x, p.y)),
                    mdc_canvas::render::PathCmd::LineTo(p) => Some((p.x, p.y)),
                    // 富 connector 路由只产 MoveTo/LineTo（QuadTo/CubicTo/
                    // Close 是 props path 面的字面量——此处不达）。
                    _ => None,
                })
                .collect();
            assert_eq!(
                pts,
                // (50,124) 与 (50,-24) 共线折叠——干净折线（三段）：
                // 垂直引出 → 水平跨接 → 垂直到点。
                vec![(50.0, 100.0), (50.0, -24.0), (350.0, -24.0), (350.0, 0.0)],
                "stub 引出 + 单肘 + 到点（共线折叠）：{pts:?}"
            );
        }
        other => panic!("应为折线，实际 {other:?}"),
    }
}

/// SetEdgeRoute 命令语义：终态替换 / None 清除 / kind 门禁 / 空串拒收。
#[test]
fn set_edge_route_command_semantics() {
    let mut s = session(7_306_005);
    two_nodes_with_edge(&mut s);

    set_route(
        &mut s,
        "e1",
        Some(Side::Top),
        Some(Side::Bottom),
        Some("关系"),
    );
    let e = s.find_object("e1").unwrap();
    assert_eq!(e.from_side, Some(Side::Top));
    assert_eq!(e.to_side, Some(Side::Bottom));
    assert_eq!(e.caption.as_deref(), Some("关系"));

    // 终态替换 + None 清除。
    set_route(&mut s, "e1", Some(Side::Left), None, None);
    let e = s.find_object("e1").unwrap();
    assert_eq!(e.from_side, Some(Side::Left));
    assert_eq!(e.to_side, None, "None 删键清除");
    assert_eq!(e.caption, None, "caption 清除");

    // kind 门禁：非 Edge 对象 KindMismatch。
    let err = s
        .execute(
            Actor::Human,
            CanvasCommand::SetEdgeRoute {
                id: "n1".into(),
                from_side: Some(Side::Top),
                to_side: None,
                caption: None,
            },
        )
        .unwrap_err();
    assert!(matches!(err, CanvasError::KindMismatch { .. }), "{err:?}");

    // 不存在。
    let err = s
        .execute(
            Actor::Human,
            CanvasCommand::SetEdgeRoute {
                id: "nope".into(),
                from_side: None,
                to_side: None,
                caption: None,
            },
        )
        .unwrap_err();
    assert!(matches!(err, CanvasError::ObjectNotFound { .. }), "{err:?}");

    // 空 caption 拒收（清除用 None）。
    let err = s
        .execute(
            Actor::Human,
            CanvasCommand::SetEdgeRoute {
                id: "e1".into(),
                from_side: None,
                to_side: None,
                caption: Some(String::new()),
            },
        )
        .unwrap_err();
    assert!(matches!(err, CanvasError::InvalidInput(_)), "{err:?}");

    // 撤销/重做。
    set_route(&mut s, "e1", Some(Side::Right), Some(Side::Left), Some("x"));
    assert!(s.undo(Actor::Human));
    assert_eq!(s.find_object("e1").unwrap().caption, None, "撤销 caption");
    assert!(s.redo(Actor::Human));
    assert_eq!(s.find_object("e1").unwrap().caption.as_deref(), Some("x"));
}

/// 指纹语义：改侧别/改标题 = 语义变更；快照往返。
#[test]
fn route_fingerprint_and_snapshot_roundtrip() {
    let mut s = session(7_306_006);
    two_nodes_with_edge(&mut s);
    let fp0 = s.object_fingerprint("e1").unwrap();

    set_route(&mut s, "e1", Some(Side::Right), Some(Side::Left), None);
    let fp1 = s.object_fingerprint("e1").unwrap();
    assert_ne!(fp0, fp1, "改锚侧 → 指纹变");

    set_route(
        &mut s,
        "e1",
        Some(Side::Right),
        Some(Side::Left),
        Some("标注"),
    );
    let fp2 = s.object_fingerprint("e1").unwrap();
    assert_ne!(fp1, fp2, "改标题 → 指纹变");

    let snap = s.authoritative_snapshot().unwrap();
    let r = CanvasSession::restore(
        SessionOptions {
            client_id: 7_306_006,
            ..Default::default()
        },
        &snap,
    )
    .unwrap();
    let e = r.find_object("e1").unwrap();
    assert_eq!(e.from_side, Some(Side::Right));
    assert_eq!(e.to_side, Some(Side::Left));
    assert_eq!(e.caption.as_deref(), Some("标注"));

    // 存量边（无路由键）缺省 None——向后兼容。
    let mut legacy = session(7_306_007);
    two_nodes_with_edge(&mut legacy);
    let e = legacy.find_object("e1").unwrap();
    assert_eq!(e.from_side, None);
    assert_eq!(e.to_side, None);
    assert_eq!(e.caption, None);
}

/// 非 Edge 对象的锚侧字段恒 None（读模型面）。
#[test]
fn non_edge_objects_have_no_route_fields() {
    let mut s = session(7_306_008);
    two_nodes_with_edge(&mut s);
    for id in ["n1", "n2"] {
        let o = s.find_object(id).unwrap();
        assert_eq!(o.kind, ObjectKind::Node);
        assert_eq!(o.from_side, None);
        assert_eq!(o.to_side, None);
        assert_eq!(o.caption, None);
    }
}
