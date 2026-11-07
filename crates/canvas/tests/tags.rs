//! 标签面测试（#405——跨区分类标签：props 之外的一等公民）。
//!
//! 钉定面：
//! - 命令语义：AddTag/RemoveTag 幂等（重复加/删不存在的 tag =
//!   零写入零事件零撤销栈变化）；
//! - 读模型：tags 排序去重（确定性）、list_by_tag 过滤面（ID 字典序）；
//! - 事件面：tag 变更经既有事件中枢派发（Command 源 + touched 含
//!   对象 ID——嵌套事件路径首段提取）；
//! - 载体面：嵌套 Map（tag → true）联合合并——并发加不同 tag
//!   双活（对比 Any::Array 整值替换的丢失面）；
//! - 全 kind 适用（node/edge/group/stroke——标签是对象级元数据，
//!   不限 kind）；
//! - 指纹面：标签集入指纹（排序后——同集不同序同指纹）。

use std::sync::{Arc, Mutex};

use mdc_canvas::{
    Actor, CanvasChangeSource, CanvasCommand, CanvasError, CanvasSession, ObjectKind,
    SessionOptions, StrokeTool,
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
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 60.0,
            props: Default::default(),
        },
    )
    .expect("add node");
}

fn add_tag(s: &mut CanvasSession, id: &str, tag: &str) -> mdc_canvas::CommandReceipt {
    s.execute(
        Actor::Human,
        CanvasCommand::AddTag {
            id: id.into(),
            tag: tag.into(),
        },
    )
    .expect("add tag")
}

fn remove_tag(s: &mut CanvasSession, id: &str, tag: &str) -> mdc_canvas::CommandReceipt {
    s.execute(
        Actor::Human,
        CanvasCommand::RemoveTag {
            id: id.into(),
            tag: tag.into(),
        },
    )
    .expect("remove tag")
}

#[derive(Clone, Default)]
struct Recorder {
    events: Arc<Mutex<Vec<mdc_canvas::CanvasEvent>>>,
}

impl Recorder {
    fn new() -> Self {
        Self::default()
    }
    fn attach(&self, s: &mut CanvasSession) -> mdc_canvas::Subscription {
        let ev = Arc::clone(&self.events);
        s.subscribe(move |e: &mdc_canvas::CanvasEvent| {
            ev.lock().unwrap().push(e.clone());
        })
    }
    fn take(&self) -> Vec<mdc_canvas::CanvasEvent> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }
}

/// 验收 1+2：加 tag 可查 + 同一对象多 tag + 读模型排序。
#[test]
fn tag_add_list_multiple_and_sorted_read_model() {
    let mut s = session(7_305_001);
    add_node(&mut s, "n1");
    add_node(&mut s, "n2");

    add_tag(&mut s, "n1", "风险");
    add_tag(&mut s, "n1", "报价");
    add_tag(&mut s, "n1", "核心");

    // 读模型：多 tag 全在 + 排序（字典序确定性）。
    let o = s.find_object("n1").unwrap();
    assert_eq!(
        o.tags,
        vec!["报价", "核心", "风险"],
        "多 tag 排序读出（码点序）"
    );

    // list_by_tag 查到（验收 1）。
    let hit = s.list_by_tag("风险");
    assert_eq!(hit.len(), 1);
    assert_eq!(hit[0].id, "n1");

    // 另一对象加同 tag → 两条都查出（跨对象分类）。
    add_tag(&mut s, "n2", "风险");
    let hit = s.list_by_tag("风险");
    assert_eq!(
        hit.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(),
        vec!["n1", "n2"],
        "list_by_tag 按 ID 字典序"
    );

    // 无 tag 的标签 → 空。
    assert!(s.list_by_tag("不存在").is_empty());

    // 移除后不再查出。
    remove_tag(&mut s, "n1", "风险");
    let hit = s.list_by_tag("风险");
    assert_eq!(
        hit.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(),
        vec!["n2"]
    );
    let o = s.find_object("n1").unwrap();
    assert_eq!(o.tags, vec!["报价", "核心"], "移除后其余 tag 不受影响");
}

/// 幂等纪律：重复加/删不存在的 tag = 零事件 + 零撤销栈变化。
#[test]
fn tag_idempotent_no_write_no_event() {
    let mut s = session(7_305_002);
    add_node(&mut s, "n1");
    add_tag(&mut s, "n1", "风险");
    let depth_after_first = s.undo_depth(Actor::Human);

    let rec = Recorder::new();
    let _sub = rec.attach(&mut s);

    // 重复加同 tag → 回执 TagAdded 但零事件零栈变化。
    let r = add_tag(&mut s, "n1", "风险");
    assert!(matches!(
        r.outcome,
        mdc_canvas::CommandOutcome::TagAdded { .. }
    ));
    assert_eq!(
        s.undo_depth(Actor::Human),
        depth_after_first,
        "幂等加不入撤销栈"
    );
    assert!(rec.take().is_empty(), "幂等加零事件");

    // 删不存在的 tag → 同幂等纪律。
    let r = remove_tag(&mut s, "n1", "不存在");
    assert!(matches!(
        r.outcome,
        mdc_canvas::CommandOutcome::TagRemoved { .. }
    ));
    assert_eq!(
        s.undo_depth(Actor::Human),
        depth_after_first,
        "幂等删不入撤销栈"
    );
    assert!(rec.take().is_empty(), "幂等删零事件");

    // 真加一个新 tag → 有事件（对照面——证明上面的空不是订阅坏了）。
    add_tag(&mut s, "n1", "报价");
    let events = rec.take();
    assert_eq!(events.len(), 1, "真写入恰一事件");
}

/// 验收 3：tag 变更触发事件（Command 源 + touched 含对象 ID）。
#[test]
fn tag_change_emits_event_with_touched() {
    let mut s = session(7_305_003);
    add_node(&mut s, "n1");
    let rec = Recorder::new();
    let _sub = rec.attach(&mut s);

    add_tag(&mut s, "n1", "风险");
    let events = rec.take();
    assert_eq!(events.len(), 1, "加 tag 恰一事件");
    assert_eq!(
        events[0].source,
        CanvasChangeSource::Command {
            actor: Actor::Human
        }
    );
    assert_eq!(
        events[0].touched,
        vec!["n1".to_string()],
        "touched 含对象 ID"
    );
    assert!(!events[0].updates.is_empty(), "有 CRDT 字节");

    remove_tag(&mut s, "n1", "风险");
    let events = rec.take();
    assert_eq!(events.len(), 1, "删 tag 恰一事件");
    assert_eq!(events[0].touched, vec!["n1".to_string()]);

    // AI 源标注（AI 检索/过滤消费面的事件溯源）。
    s.execute(
        Actor::Ai,
        CanvasCommand::AddTag {
            id: "n1".into(),
            tag: "ai-标记".into(),
        },
    )
    .unwrap();
    let events = rec.take();
    assert_eq!(
        events[0].source,
        CanvasChangeSource::Command { actor: Actor::Ai }
    );
}

/// 校验/错误面：空 tag 拒收；对象不存在。
#[test]
fn tag_validation_and_errors() {
    let mut s = session(7_305_004);
    add_node(&mut s, "n1");

    let e = s
        .execute(
            Actor::Human,
            CanvasCommand::AddTag {
                id: "n1".into(),
                tag: "".into(),
            },
        )
        .unwrap_err();
    assert!(matches!(e, CanvasError::InvalidInput(_)), "{e:?}");

    let e = s
        .execute(
            Actor::Human,
            CanvasCommand::RemoveTag {
                id: "n1".into(),
                tag: "".into(),
            },
        )
        .unwrap_err();
    assert!(matches!(e, CanvasError::InvalidInput(_)), "{e:?}");

    let e = s
        .execute(
            Actor::Human,
            CanvasCommand::AddTag {
                id: "nope".into(),
                tag: "风险".into(),
            },
        )
        .unwrap_err();
    assert!(matches!(e, CanvasError::ObjectNotFound { .. }), "{e:?}");
}

/// 全 kind 适用：标签是对象级元数据（node/edge/group/stroke 都能挂）。
#[test]
fn tags_apply_to_all_kinds() {
    let mut s = session(7_305_005);
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
            points: vec![(0.0, 0.0), (10.0, 10.0)],
            color: "#000".into(),
            width: 2.0,
            tool: StrokeTool::Pen,
        },
    )
    .unwrap();

    for (id, kind) in [
        ("n1", ObjectKind::Node),
        ("e1", ObjectKind::Edge),
        ("g1", ObjectKind::Group),
        ("s1", ObjectKind::Stroke),
    ] {
        add_tag(&mut s, id, "跨区");
        let o = s.find_object(id).unwrap();
        assert_eq!(o.kind, kind);
        assert_eq!(o.tags, vec!["跨区".to_string()], "{id} 应挂上标签");
    }
    assert_eq!(s.list_by_tag("跨区").len(), 4);
}

/// 撤销/重做：加 tag 可撤销（标签消失）、重做恢复。
#[test]
fn tag_undo_redo() {
    let mut s = session(7_305_006);
    add_node(&mut s, "n1");
    add_tag(&mut s, "n1", "风险");
    assert_eq!(s.find_object("n1").unwrap().tags, vec!["风险"]);

    assert!(s.undo(Actor::Human), "撤销加 tag");
    assert_eq!(
        s.find_object("n1").unwrap().tags,
        Vec::<String>::new(),
        "撤销后标签消失"
    );

    assert!(s.redo(Actor::Human), "重做加 tag");
    assert_eq!(
        s.find_object("n1").unwrap().tags,
        vec!["风险"],
        "重做后标签回归"
    );

    // 删 tag 也可撤销。
    remove_tag(&mut s, "n1", "风险");
    assert!(s.undo(Actor::Human), "撤销删 tag");
    assert_eq!(
        s.find_object("n1").unwrap().tags,
        vec!["风险"],
        "撤销删 = 标签回归"
    );
}

/// 快照往返：标签随权威快照走；存量对象（无 tags 字段）= 空集。
#[test]
fn tag_snapshot_roundtrip_and_legacy_default() {
    let mut s = session(7_305_007);
    add_node(&mut s, "n1");
    add_node(&mut s, "n2"); // 不加 tag（存量形状）
    add_tag(&mut s, "n1", "风险");
    add_tag(&mut s, "n1", "报价");

    let snap = s.authoritative_snapshot().unwrap();
    let r = CanvasSession::restore(
        SessionOptions {
            client_id: 7_305_007,
            ..Default::default()
        },
        &snap,
    )
    .unwrap();
    assert_eq!(
        r.find_object("n1").unwrap().tags,
        vec!["报价", "风险"],
        "标签随快照恢复（排序）"
    );
    assert_eq!(
        r.find_object("n2").unwrap().tags,
        Vec::<String>::new(),
        "无标签对象缺省空集"
    );
    assert_eq!(r.list_by_tag("风险").len(), 1);
}

/// 载体裁决钉定：嵌套 Map 联合合并——并发加不同 tag 双端收敛两笔全活。
#[test]
fn concurrent_add_tag_union_merge() {
    let mut a = session(7_305_008);
    let mut b = session(7_305_009);
    add_node(&mut a, "n1");
    // 同步基础对象到 b。
    let sv_b = b.state_vector();
    let diff_a = a.encode_diff(&sv_b).unwrap();
    b.apply_update(&diff_a).unwrap();

    // 并发：a 加「风险」、b 加「报价」（不同 tag）。
    add_tag(&mut a, "n1", "风险");
    add_tag(&mut b, "n1", "报价");

    // 双向同步。
    let sv_b = b.state_vector();
    let sv_a = a.state_vector();
    let diff_a = a.encode_diff(&sv_b).unwrap();
    let diff_b = b.encode_diff(&sv_a).unwrap();
    b.apply_update(&diff_a).unwrap();
    a.apply_update(&diff_b).unwrap();

    // 联合合并：两笔都活（Any::Array 整值替换会丢一笔——反面对照）。
    let expect = vec!["报价".to_string(), "风险".to_string()];
    assert_eq!(a.find_object("n1").unwrap().tags, expect, "a 端两 tag 全活");
    assert_eq!(b.find_object("n1").unwrap().tags, expect, "b 端两 tag 全活");

    // 并发加同 tag：幂等收敛（一 tag 不重复）。
    add_tag(&mut a, "n1", "同标");
    add_tag(&mut b, "n1", "同标");
    let sv_b = b.state_vector();
    let sv_a = a.state_vector();
    b.apply_update(&a.encode_diff(&sv_b).unwrap()).unwrap();
    a.apply_update(&b.encode_diff(&sv_a).unwrap()).unwrap();
    let oa = a.find_object("n1").unwrap().tags;
    let ob = b.find_object("n1").unwrap().tags;
    assert_eq!(oa, ob);
    assert_eq!(
        oa.iter().filter(|t| t.as_str() == "同标").count(),
        1,
        "同 tag 不重复"
    );
}

/// 指纹面：标签变更 = 语义变更 → 指纹变；同集不同序同指纹（排序语义）。
#[test]
fn tag_fingerprint_semantics() {
    let mut s = session(7_305_010);
    add_node(&mut s, "n1");
    let fp0 = s.object_fingerprint("n1").unwrap();

    add_tag(&mut s, "n1", "风险");
    let fp1 = s.object_fingerprint("n1").unwrap();
    assert_ne!(fp0, fp1, "加 tag → 指纹变（证据 Stale 语义）");

    remove_tag(&mut s, "n1", "风险");
    let fp2 = s.object_fingerprint("n1").unwrap();
    assert_eq!(fp0, fp2, "移除回空集 → 指纹回到初值");

    // 排序语义：不同写入顺序得到同一集 → 同指纹。
    let mut x = session(7_305_011);
    let mut y = session(7_305_012);
    add_node(&mut x, "n1");
    add_node(&mut y, "n1");
    add_tag(&mut x, "n1", "甲");
    add_tag(&mut x, "n1", "乙");
    add_tag(&mut y, "n1", "乙");
    add_tag(&mut y, "n1", "甲");
    assert_eq!(
        x.object_fingerprint("n1").unwrap(),
        y.object_fingerprint("n1").unwrap(),
        "集语义：同集不同序同指纹"
    );
}
