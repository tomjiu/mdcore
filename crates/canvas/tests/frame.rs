//! frame 面测试（#404——命名区 + parent 树：Group 演化的容器原语）。
//!
//! 验收对应：
//! 1. 创建 frame，命名，把节点放进去（parent 挂接）
//! 2. reparent 后 ID 不变
//! 3. 设置环路 parent 被拒（A.parent=B, B.parent=A）
//! 4. 渲染时 frame 背景框在最底层（容器分层）
//!
//! 附加钉定：校验/错误面、级联删除 + 撤销原子恢复、快照往返、
//! 指纹语义、嵌套 frame、任意 kind 入 frame。

use mdc_canvas::render::CanvasRendering;
use mdc_canvas::{
    Actor, CanvasCommand, CanvasError, CanvasSession, ObjectKind, SessionOptions, Shape,
};

fn session(id: u64) -> CanvasSession {
    CanvasSession::open(SessionOptions {
        client_id: id,
        ..Default::default()
    })
    .expect("open")
}

fn add_node(s: &mut CanvasSession, id: &str) {
    s.execute(
        Actor::Human,
        CanvasCommand::AddNode {
            id: Some(id.into()),
            x: 10.0,
            y: 10.0,
            w: 100.0,
            h: 60.0,
            props: Default::default(),
        },
    )
    .expect("add node");
}

fn create_frame(s: &mut CanvasSession, id: &str, name: &str) {
    s.execute(
        Actor::Human,
        CanvasCommand::CreateFrame {
            id: Some(id.into()),
            name: name.into(),
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 300.0,
            parent: None,
        },
    )
    .expect("create frame");
}

fn set_parent(s: &mut CanvasSession, id: &str, parent: Option<&str>) {
    s.execute(
        Actor::Human,
        CanvasCommand::SetParent {
            id: id.into(),
            parent: parent.map(str::to_string),
        },
    )
    .expect("set parent");
}

/// 验收 1 + 2：创建命名 frame、节点入 frame、reparent 后 ID 不变。
#[test]
fn frame_create_name_and_reparent_keeps_id() {
    let mut s = session(7_304_001);
    add_node(&mut s, "n1");
    create_frame(&mut s, "f1", "第一章");
    create_frame(&mut s, "f2", "第二章");

    // frame 读模型：kind/name/几何。
    let f = s.find_object("f1").unwrap();
    assert_eq!(f.kind, ObjectKind::Frame);
    assert_eq!(f.name.as_deref(), Some("第一章"));
    assert_eq!((f.x, f.y, f.w, f.h), (0.0, 0.0, 400.0, 300.0));

    // 节点入 frame（验收 1：把节点放进去）。
    set_parent(&mut s, "n1", Some("f1"));
    let n = s.find_object("n1").unwrap();
    assert_eq!(n.parent.as_deref(), Some("f1"), "节点挂到 frame");

    // reparent 后 ID 不变（验收 2）。
    set_parent(&mut s, "n1", Some("f2"));
    let n = s.find_object("n1").unwrap();
    assert_eq!(n.id, "n1", "reparent 不换 ID");
    assert_eq!(n.parent.as_deref(), Some("f2"));

    // 提顶层（parent = None）。
    set_parent(&mut s, "n1", None);
    let n = s.find_object("n1").unwrap();
    assert_eq!(n.parent, None, "提顶层显式删键");

    // 重命名。
    s.execute(
        Actor::Human,
        CanvasCommand::RenameFrame {
            id: "f1".into(),
            name: "第一章（修订）".into(),
        },
    )
    .unwrap();
    assert_eq!(
        s.find_object("f1").unwrap().name.as_deref(),
        Some("第一章（修订）")
    );
}

/// 验收 3：环路 parent 被拒（A.parent=B, B.parent=A）+ 深链环 + 自指。
#[test]
fn frame_cycle_parents_rejected() {
    let mut s = session(7_304_002);
    create_frame(&mut s, "f1", "A");
    create_frame(&mut s, "f2", "B");
    create_frame(&mut s, "f3", "C");
    set_parent(&mut s, "f2", Some("f1")); // f2 → f1

    // A.parent=B, B.parent=A 的环路：f1 挂到 f2 会成环。
    let e = s
        .execute(
            Actor::Human,
            CanvasCommand::SetParent {
                id: "f1".into(),
                parent: Some("f2".into()),
            },
        )
        .unwrap_err();
    assert!(matches!(e, CanvasError::InvalidInput(_)), "{e:?}");
    assert_eq!(s.find_object("f1").unwrap().parent, None, "拒写后原值不变");

    // 自指。
    let e = s
        .execute(
            Actor::Human,
            CanvasCommand::SetParent {
                id: "f1".into(),
                parent: Some("f1".into()),
            },
        )
        .unwrap_err();
    assert!(matches!(e, CanvasError::InvalidInput(_)), "{e:?}");

    // 深链环：f3 → f2 → f1，再 f1 → f3 成环。
    set_parent(&mut s, "f3", Some("f2"));
    let e = s
        .execute(
            Actor::Human,
            CanvasCommand::SetParent {
                id: "f1".into(),
                parent: Some("f3".into()),
            },
        )
        .unwrap_err();
    assert!(matches!(e, CanvasError::InvalidInput(_)), "{e:?}");
}

/// 校验/错误面：空名、非 frame 目标、不存在目标、非 frame 重命名。
#[test]
fn frame_validation_and_errors() {
    let mut s = session(7_304_003);
    add_node(&mut s, "n1");
    create_frame(&mut s, "f1", "A");

    // 空名。
    let e = s
        .execute(
            Actor::Human,
            CanvasCommand::CreateFrame {
                id: None,
                name: "".into(),
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
                parent: None,
            },
        )
        .unwrap_err();
    assert!(matches!(e, CanvasError::InvalidInput(_)), "{e:?}");

    // parent 目标必须是 frame（节点不是）。
    let e = s
        .execute(
            Actor::Human,
            CanvasCommand::SetParent {
                id: "n1".into(),
                parent: Some("n1".into()), // 自指也先撞自指校验
            },
        )
        .unwrap_err();
    assert!(matches!(e, CanvasError::InvalidInput(_)), "{e:?}");
    let e = s
        .execute(
            Actor::Human,
            CanvasCommand::CreateFrame {
                id: Some("fx".into()),
                name: "X".into(),
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
                parent: Some("n1".into()), // 节点不是 frame
            },
        )
        .unwrap_err();
    assert!(matches!(e, CanvasError::InvalidInput(_)), "{e:?}");

    // 不存在的 parent / 不存在的对象。
    let e = s
        .execute(
            Actor::Human,
            CanvasCommand::SetParent {
                id: "n1".into(),
                parent: Some("nope".into()),
            },
        )
        .unwrap_err();
    assert!(matches!(e, CanvasError::ObjectNotFound { .. }), "{e:?}");
    let e = s
        .execute(
            Actor::Human,
            CanvasCommand::SetParent {
                id: "nope".into(),
                parent: Some("f1".into()),
            },
        )
        .unwrap_err();
    assert!(matches!(e, CanvasError::ObjectNotFound { .. }), "{e:?}");

    // 非 frame 重命名 → KindMismatch。
    let e = s
        .execute(
            Actor::Human,
            CanvasCommand::RenameFrame {
                id: "n1".into(),
                name: "X".into(),
            },
        )
        .unwrap_err();
    assert!(matches!(e, CanvasError::KindMismatch { .. }), "{e:?}");

    // NaN 几何。
    let e = s
        .execute(
            Actor::Human,
            CanvasCommand::CreateFrame {
                id: None,
                name: "X".into(),
                x: f64::NAN,
                y: 0.0,
                w: 1.0,
                h: 1.0,
                parent: None,
            },
        )
        .unwrap_err();
    assert!(matches!(e, CanvasError::NonFiniteGeometry { .. }), "{e:?}");
}

/// 验收 4：渲染时 frame 背景框在最底层（容器分层）+ 标题发射。
#[test]
fn frame_renders_at_bottom_layer_with_title() {
    let mut s = session(7_304_004);
    // 构造：frame 在对象序中排最后（若不分层，其背景会画在内容之上）。
    add_node(&mut s, "n1");
    create_frame(&mut s, "f1", "第一章");

    let rendering = CanvasRendering::from_objects(&s.objects());
    assert!(rendering.shapes.len() >= 3, "背景框 + 标题 + 节点");
    // 第一个形状 = frame 背景框（Rect，虚线边框）。
    match &rendering.shapes[0].0 {
        Shape::Rect { rect, style } => {
            assert_eq!((rect.x, rect.y, rect.w, rect.h), (0.0, 0.0, 400.0, 300.0));
            assert!(style.fill.is_some(), "背景框有底色");
            assert_eq!(style.stroke.as_ref().unwrap().dash.dash, 6, "虚线边框");
        }
        other => panic!("首形状应为 frame 背景框，实际 {other:?}"),
    }
    // 第二个形状 = 标题文本（名称）。
    match &rendering.shapes[1].0 {
        Shape::Text { text, .. } => assert_eq!(text, "第一章"),
        other => panic!("次形状应为标题，实际 {other:?}"),
    }
    // 内容（节点）在 frame 图元之后。
    match &rendering.shapes[2].0 {
        Shape::Rect { rect, .. } => assert_eq!((rect.x, rect.y), (10.0, 10.0), "节点图元"),
        other => panic!("第三形状应为节点，实际 {other:?}"),
    }

    // shape_from_object：frame 不作为内容图元（容器由分层发射）。
    assert!(mdc_canvas::render::shape_from_object(&s.find_object("f1").unwrap(), &[]).is_none());
}

/// 级联删除：删 frame 直系子对象提顶层（同事务原子，撤销恢复）。
#[test]
fn frame_removal_cascades_children_atomically() {
    let mut s = session(7_304_005);
    add_node(&mut s, "n1");
    add_node(&mut s, "n2");
    create_frame(&mut s, "f1", "A");
    set_parent(&mut s, "n1", Some("f1"));
    set_parent(&mut s, "n2", Some("f1"));

    // 删 frame → 两子提顶层。
    s.execute(
        Actor::Human,
        CanvasCommand::RemoveObject { id: "f1".into() },
    )
    .unwrap();
    assert!(s.find_object("f1").is_none());
    assert_eq!(s.find_object("n1").unwrap().parent, None, "级联提顶层");
    assert_eq!(s.find_object("n2").unwrap().parent, None, "级联提顶层");
    assert!(
        s.find_object("n1").is_some() && s.find_object("n2").is_some(),
        "子对象不动"
    );

    // 撤销恢复：frame 回归 + 子对象 parent 回归（原子恢复）。
    assert!(s.undo(Actor::Human), "撤销删 frame");
    assert_eq!(s.find_object("f1").unwrap().name.as_deref(), Some("A"));
    assert_eq!(
        s.find_object("n1").unwrap().parent.as_deref(),
        Some("f1"),
        "撤销恢复挂接"
    );
    assert_eq!(
        s.find_object("n2").unwrap().parent.as_deref(),
        Some("f1"),
        "撤销恢复挂接"
    );
}

/// 嵌套 frame + 任意 kind 入 frame。
#[test]
fn nested_frames_and_all_kinds_can_parent() {
    let mut s = session(7_304_006);
    create_frame(&mut s, "outer", "外");
    create_frame(&mut s, "inner", "内");
    set_parent(&mut s, "inner", Some("outer"));
    assert_eq!(
        s.find_object("inner").unwrap().parent.as_deref(),
        Some("outer"),
        "嵌套 frame"
    );

    // 节点/边/分组/笔迹皆可入 frame。
    add_node(&mut s, "n1");
    add_node(&mut s, "n2");
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
    s.execute(
        Actor::Human,
        CanvasCommand::GroupObjects {
            group_id: Some("g1".into()),
            members: vec!["n1".into(), "n2".into()],
        },
    )
    .unwrap();
    s.execute(
        Actor::Human,
        CanvasCommand::AddStroke {
            id: Some("s1".into()),
            points: vec![(0.0, 0.0), (5.0, 5.0)],
            color: "#000".into(),
            width: 2.0,
            tool: mdc_canvas::StrokeTool::Pen,
        },
    )
    .unwrap();
    for id in ["n1", "e1", "g1", "s1"] {
        set_parent(&mut s, id, Some("inner"));
        assert_eq!(
            s.find_object(id).unwrap().parent.as_deref(),
            Some("inner"),
            "{id} 入嵌套 frame"
        );
    }
}

/// 快照往返 + 存量对象缺省（无 name/parent 字段 = None）。
#[test]
fn frame_snapshot_roundtrip_and_legacy_default() {
    let mut s = session(7_304_007);
    create_frame(&mut s, "f1", "第一章");
    add_node(&mut s, "n1");
    set_parent(&mut s, "n1", Some("f1"));

    let snap = s.authoritative_snapshot().unwrap();
    let r = CanvasSession::restore(
        SessionOptions {
            client_id: 7_304_007,
            ..Default::default()
        },
        &snap,
    )
    .unwrap();
    assert_eq!(r.find_object("f1").unwrap().name.as_deref(), Some("第一章"));
    assert_eq!(r.find_object("n1").unwrap().parent.as_deref(), Some("f1"));
}

/// 指纹语义：重命名 / reparent = 语义变更 → 指纹变。
#[test]
fn frame_fingerprint_semantics() {
    let mut s = session(7_304_008);
    add_node(&mut s, "n1");
    create_frame(&mut s, "f1", "A");
    let fp_n0 = s.object_fingerprint("n1").unwrap();
    let fp_f0 = s.object_fingerprint("f1").unwrap();

    // reparent 改节点指纹。
    set_parent(&mut s, "n1", Some("f1"));
    let fp_n1 = s.object_fingerprint("n1").unwrap();
    assert_ne!(fp_n0, fp_n1, "reparent → 节点指纹变");

    // 重命名改 frame 指纹。
    s.execute(
        Actor::Human,
        CanvasCommand::RenameFrame {
            id: "f1".into(),
            name: "B".into(),
        },
    )
    .unwrap();
    let fp_f1 = s.object_fingerprint("f1").unwrap();
    assert_ne!(fp_f0, fp_f1, "重命名 → frame 指纹变");
}
