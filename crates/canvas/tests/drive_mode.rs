//! 三态驾驶闸（#353 / #44 步5）测试。
//!
//! 覆盖：User 模式拒 Ai 一切写入面（execute/execute_batch/undo/redo/
//! 切模式）/Human 永不被拒（安全默认）/Collaborative 缺省零回归/
//! Ai 态让出矩阵（→User、→Collaborative、幂等 Ai）/自我提权被拒
//! （Collaborative→Ai、User 下一切）/拒绝路径零 CRDT 字节·零事件·
//! 零撤销入栈/模式不入 CRDT（快照恢复回落 Collaborative）/set_drive_mode
//! 不能进批/事件订阅面闸后零派发。

use std::collections::BTreeMap;
use std::sync::Arc;

use mdc_canvas::{Actor, CanvasCommand, CanvasError, CanvasSession, DriveMode, SessionOptions};

fn session(tag: u64) -> CanvasSession {
    CanvasSession::open(SessionOptions {
        client_id: tag,
        clock_ms: Some(Arc::new(move || 1_700_000_000_000 + tag)),
        ..Default::default()
    })
    .unwrap()
}

fn add_node_cmd(id: &str, x: f64, y: f64) -> CanvasCommand {
    CanvasCommand::AddNode {
        id: Some(id.into()),
        x,
        y,
        w: 100.0,
        h: 50.0,
        props: BTreeMap::new(),
    }
}

/// 缺省 = Collaborative（Lead 裁定点 1：零破坏——既有 AI 路径消费者
/// 不回归；User 锁定由宿主显式开启）。
#[test]
fn default_mode_is_collaborative() {
    let s = session(1);
    assert_eq!(s.drive_mode(), DriveMode::Collaborative);
    // Ai 照常（缺省面零回归）。
    let mut s = s;
    s.execute(Actor::Ai, add_node_cmd("n1", 0.0, 0.0)).unwrap();
    assert!(s.find_object("n1").is_some());
}

/// User 模式：Ai 的 execute 被拒（ModeDenied 含当前模式与被拒 actor），
/// Human 照常。
#[test]
fn user_mode_denies_ai_execute_allows_human() {
    let mut s = session(2);
    s.set_drive_mode(Actor::Human, DriveMode::User).unwrap();
    assert_eq!(s.drive_mode(), DriveMode::User);

    let err = s
        .execute(Actor::Ai, add_node_cmd("n1", 0.0, 0.0))
        .unwrap_err();
    assert_eq!(
        err,
        CanvasError::ModeDenied {
            mode: DriveMode::User,
            actor: Actor::Ai,
        }
    );
    assert!(s.find_object("n1").is_none(), "拒绝路径读模型不变");

    // Human 命令照常。
    s.execute(Actor::Human, add_node_cmd("n1", 0.0, 0.0))
        .unwrap();
    assert!(s.find_object("n1").is_some());
}

/// User 模式：Ai 借撤销/重做面改状态同拒；Human 撤销照常。
#[test]
fn user_mode_denies_ai_undo_redo() {
    let mut s = session(3);
    s.execute(Actor::Ai, add_node_cmd("n1", 0.0, 0.0)).unwrap();
    s.execute(Actor::Human, add_node_cmd("n2", 1.0, 1.0))
        .unwrap();
    s.set_drive_mode(Actor::Human, DriveMode::User).unwrap();

    let err = s.undo(Actor::Ai).unwrap_err();
    assert_eq!(
        err,
        CanvasError::ModeDenied {
            mode: DriveMode::User,
            actor: Actor::Ai,
        }
    );
    let err = s.redo(Actor::Ai).unwrap_err();
    assert!(matches!(err, CanvasError::ModeDenied { .. }));

    // Ai 的栈没被动过（后续 Ai 态恢复后仍可撤）。
    assert_eq!(s.undo_depth(Actor::Ai), 1);

    // Human 撤销照常（撤自己那笔）。
    assert!(s.undo(Actor::Human).expect("human undo"));
    assert!(s.find_object("n2").is_none(), "Human 撤销生效");
    assert!(s.find_object("n1").is_some(), "Ai 的栈未被波及");
}

/// User 模式：Ai 的 execute_batch 同拒（沙箱副本零成本早退）。
#[test]
fn user_mode_denies_ai_batch() {
    let mut s = session(4);
    s.execute(Actor::Ai, add_node_cmd("n1", 0.0, 0.0)).unwrap();
    s.set_drive_mode(Actor::Human, DriveMode::User).unwrap();

    let err = s
        .execute_batch(Actor::Ai, vec![add_node_cmd("n2", 1.0, 1.0)])
        .unwrap_err();
    assert!(matches!(err, CanvasError::ModeDenied { .. }));
    assert!(s.find_object("n2").is_none());
}

/// User 模式：Ai 连切模式本身也被拒（不能自救解锁——解锁权只在人）。
#[test]
fn user_mode_denies_ai_mode_switch() {
    let mut s = session(5);
    s.set_drive_mode(Actor::Human, DriveMode::User).unwrap();

    // 会话方法面。
    let err = s
        .set_drive_mode(Actor::Ai, DriveMode::Collaborative)
        .unwrap_err();
    assert!(matches!(err, CanvasError::ModeDenied { .. }));
    // 命令面（同一门）。
    let err = s
        .execute(
            Actor::Ai,
            CanvasCommand::SetDriveMode {
                mode: DriveMode::Collaborative,
            },
        )
        .unwrap_err();
    assert!(matches!(err, CanvasError::ModeDenied { .. }));
    assert_eq!(s.drive_mode(), DriveMode::User, "模式未被 Ai 改动");
}

/// 拒绝路径零 CRDT 字节产出（无事件/无 undo 入栈）。
#[test]
fn denial_path_zero_bytes_zero_events_zero_undo() {
    let mut s = session(6);
    s.execute(Actor::Ai, add_node_cmd("n1", 0.0, 0.0)).unwrap();
    let bytes_before = s.full_update_len();
    let ai_depth_before = s.undo_depth(Actor::Ai);

    // 事件缓冲：订阅后拒绝，断言零派发。
    let seen = Arc::new(std::sync::Mutex::new(0usize));
    let sink = seen.clone();
    let _sub = s.subscribe(move |_| {
        *sink.lock().unwrap() += 1;
    });

    s.set_drive_mode(Actor::Human, DriveMode::User).unwrap();
    let _ = s
        .execute(Actor::Ai, add_node_cmd("n2", 1.0, 1.0))
        .unwrap_err();
    let _ = s
        .execute_batch(Actor::Ai, vec![add_node_cmd("n3", 2.0, 2.0)])
        .unwrap_err();
    let _ = s.undo(Actor::Ai).unwrap_err();
    let _ = s.redo(Actor::Ai).unwrap_err();

    assert_eq!(s.full_update_len(), bytes_before, "零 CRDT 字节产出");
    assert_eq!(s.undo_depth(Actor::Ai), ai_depth_before, "零撤销入栈");
    assert_eq!(*seen.lock().unwrap(), 0, "拒绝路径零事件派发");
}

/// Human 全转移矩阵任意态可切（含切回）；Ai 态让出矩阵。
#[test]
fn transition_matrix() {
    let mut s = session(7);

    // Human：任意态（User→Ai→Collaborative→User 全通）。
    for m in [
        DriveMode::User,
        DriveMode::Ai,
        DriveMode::Collaborative,
        DriveMode::User,
    ] {
        s.set_drive_mode(Actor::Human, m).unwrap();
        assert_eq!(s.drive_mode(), m);
    }

    // 命令面同语义（Human）。
    s.execute(
        Actor::Human,
        CanvasCommand::SetDriveMode {
            mode: DriveMode::Ai,
        },
    )
    .unwrap();
    assert_eq!(s.drive_mode(), DriveMode::Ai);

    // Ai 态：让出 → User / Collaborative 均可；幂等 Ai 也可。
    s.set_drive_mode(Actor::Ai, DriveMode::User).unwrap();
    s.set_drive_mode(Actor::Human, DriveMode::Ai).unwrap();
    s.set_drive_mode(Actor::Ai, DriveMode::Collaborative)
        .unwrap();
    s.set_drive_mode(Actor::Human, DriveMode::Ai).unwrap();
    s.set_drive_mode(Actor::Ai, DriveMode::Ai).unwrap(); // 幂等
    assert_eq!(s.drive_mode(), DriveMode::Ai);

    // Collaborative 态：Ai 切任何模式都拒（含降权 User——「仅能从
    // Ai 态让出」的字面语义；提权 Ai 更拒）。
    s.set_drive_mode(Actor::Human, DriveMode::Collaborative)
        .unwrap();
    for m in [DriveMode::User, DriveMode::Ai, DriveMode::Collaborative] {
        let err = s.set_drive_mode(Actor::Ai, m).unwrap_err();
        assert!(
            matches!(err, CanvasError::ModeDenied { .. }),
            "Collaborative 态 Ai 切 {m:?} 应拒"
        );
    }
    assert_eq!(s.drive_mode(), DriveMode::Collaborative);
}

/// Ai 模式：Ai 命令照常，Human 夺回不受限（安全默认）。
#[test]
fn ai_mode_ai_works_human_never_denied() {
    let mut s = session(8);
    s.set_drive_mode(Actor::Human, DriveMode::Ai).unwrap();

    // Ai 照常。
    s.execute(Actor::Ai, add_node_cmd("n1", 0.0, 0.0)).unwrap();
    assert!(s.find_object("n1").is_some());

    // Human 夺回：Ai 态下 Human 命令/撤销/切模式全不受限。
    s.execute(Actor::Human, add_node_cmd("n2", 1.0, 1.0))
        .unwrap();
    assert!(s.undo(Actor::Human).expect("human undo"));
    s.set_drive_mode(Actor::Human, DriveMode::User).unwrap();
    assert_eq!(s.drive_mode(), DriveMode::User);
}

/// 模式不入 CRDT：快照恢复的会话回落 Collaborative（模式是会话本地
/// 策略——两端策略由宿主各自决定）。
#[test]
fn mode_not_in_crdt_snapshot_restores_collaborative() {
    let mut s = session(9);
    s.execute(Actor::Human, add_node_cmd("n1", 0.0, 0.0))
        .unwrap();
    s.set_drive_mode(Actor::Human, DriveMode::User).unwrap();

    let snap = s.authoritative_snapshot().unwrap();
    let mut restored = CanvasSession::restore(
        SessionOptions {
            client_id: 901,
            clock_ms: Some(Arc::new(|| 1_700_000_000_901)),
            ..Default::default()
        },
        &snap,
    )
    .unwrap();
    // 对象在（CRDT 面照常）；模式回落缺省（非载体字节）。
    assert!(restored.find_object("n1").is_some());
    assert_eq!(restored.drive_mode(), DriveMode::Collaborative);
    // 恢复会话的 User 锁定由宿主重新显式开启。
    restored
        .set_drive_mode(Actor::Human, DriveMode::User)
        .unwrap();
    assert_eq!(restored.drive_mode(), DriveMode::User);
}

/// set_drive_mode 不能进批：会话本地策略在沙箱内静默丢失 = 回执谎报
/// ——批入口显式拒绝（Human 也拒——这不是权限问题，是语义面）。
#[test]
fn set_drive_mode_rejected_in_batch() {
    let mut s = session(10);
    let err = s
        .execute_batch(
            Actor::Human,
            vec![
                add_node_cmd("n1", 0.0, 0.0),
                CanvasCommand::SetDriveMode {
                    mode: DriveMode::User,
                },
            ],
        )
        .unwrap_err();
    assert!(matches!(err, CanvasError::InvalidInput(ref m) if m.contains("set_drive_mode")));
    // 原子性：整批未落地。
    assert!(s.find_object("n1").is_none());
}

/// 成功切换零 CRDT 字节·零事件·零入栈（模式是本地策略——切换本身
/// 不该动载体，也不该占用撤销栈）。
#[test]
fn successful_switch_zero_carrier_bytes() {
    let mut s = session(11);
    s.execute(Actor::Human, add_node_cmd("n1", 0.0, 0.0))
        .unwrap();
    let bytes_before = s.full_update_len();
    let depth_before = s.undo_depth(Actor::Human);

    let seen = Arc::new(std::sync::Mutex::new(0usize));
    let sink = seen.clone();
    let _sub = s.subscribe(move |_| {
        *sink.lock().unwrap() += 1;
    });

    s.set_drive_mode(Actor::Human, DriveMode::Ai).unwrap();
    assert_eq!(s.full_update_len(), bytes_before, "切换零载体字节");
    assert_eq!(s.undo_depth(Actor::Human), depth_before, "切换零入栈");
    assert_eq!(*seen.lock().unwrap(), 0, "切换零事件（裁定点 2）");
}
