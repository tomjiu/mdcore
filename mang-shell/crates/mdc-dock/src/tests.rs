//! mdc-dock 批①测试面（#119 验收 1：trait 契约 + 激活去重）。
//!
//! 纯逻辑（`ActiveTracker`）零依赖单测；实体激活协议经 `#[gpui::test]`
//! 无头验证（订阅/去重/预物化意图/移除语义/开合通知）。

use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, AppContext, Context, TestAppContext};

use crate::dock::{ActiveTracker, Dock, DockPosition};
use crate::handle::PanelId;
use crate::panel::{Panel, PanelEvent};
use crate::registry::PanelRegistry;
use crate::state::{DOCK_LAYOUT_VERSION, DockAreaState, DockLayoutVersionMismatch, PanelState};

// ---------------------------------------------------------------------------
// 纯逻辑：ActiveTracker
// ---------------------------------------------------------------------------

#[test]
fn tracker_transition_dedups_same_target() {
    let mut t = ActiveTracker::new();
    assert_eq!(t.current(), None);
    // None → Some(0)：仅激。
    assert_eq!(t.transition(Some(0)), (None, Some(0)));
    // 同位重入：零副作用（去重——gpui-base active.rs 去抖契约）。
    assert_eq!(t.transition(Some(0)), (None, None));
    // 换面：先去激后激一对。
    assert_eq!(t.transition(Some(1)), (Some(0), Some(1)));
    // 归零：仅去激。
    assert_eq!(t.transition(None), (Some(1), None));
    assert_eq!(t.current(), None);
}

#[test]
fn tracker_remove_at_adjusts_indices() {
    let mut t = ActiveTracker::new();
    t.transition(Some(1));
    // 前位移收缩：激活位 1 → 0。
    assert_eq!(t.remove_at(0), None);
    assert_eq!(t.current(), Some(0));
    // 命中激活位：清空并上报。
    assert_eq!(t.remove_at(0), Some(0));
    assert_eq!(t.current(), None);
    // 后位无关。
    t.transition(Some(0));
    assert_eq!(t.remove_at(5), None);
    assert_eq!(t.current(), Some(0));
}

// ---------------------------------------------------------------------------
// 测试面板型（persistent_name/panel_key 是静态身份——一型一面，
// 与蓝本 per-type 身份一致；GhostPanel 携带可关/可启差异面）
// ---------------------------------------------------------------------------

type Log = Rc<std::cell::RefCell<Vec<bool>>>;

struct TreePanel {
    focus: gpui::FocusHandle,
    log: Log,
}

struct GitPanel {
    focus: gpui::FocusHandle,
    log: Log,
}

struct GhostPanel {
    focus: gpui::FocusHandle,
    log: Log,
    closable: bool,
    enabled: bool,
}

impl TreePanel {
    const KEY: &'static str = "file-tree";
}
impl GitPanel {
    const KEY: &'static str = "git";
}
impl GhostPanel {
    const KEY: &'static str = "ghost";
}

macro_rules! impl_test_panel {
    ($ty:ty, $key:expr) => {
        impl gpui::Render for $ty {
            fn render(
                &mut self,
                _: &mut gpui::Window,
                _: &mut Context<Self>,
            ) -> impl gpui::IntoElement {
                gpui::div()
            }
        }
        impl gpui::EventEmitter<PanelEvent> for $ty {}
        impl gpui::Focusable for $ty {
            fn focus_handle(&self, _: &App) -> gpui::FocusHandle {
                self.focus.clone()
            }
        }
        impl Panel for $ty {
            fn persistent_name() -> &'static str {
                $key
            }
            fn panel_key() -> &'static str {
                $key
            }
            fn position(&self) -> DockPosition {
                DockPosition::Left
            }
            fn position_is_valid(&self, position: DockPosition) -> bool {
                position == DockPosition::Left
            }
            fn starts_open(&self) -> bool {
                $key == "file-tree"
            }
            fn set_active(&mut self, active: bool, _cx: &mut Context<Self>) {
                self.log.borrow_mut().push(active);
            }
        }
    };
}

impl_test_panel!(TreePanel, TreePanel::KEY);
impl_test_panel!(GitPanel, GitPanel::KEY);

// GhostPanel：closable/enabled 可变——Close 门与 enabled 跳过的裁决面。
impl gpui::Render for GhostPanel {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div()
    }
}
impl gpui::EventEmitter<PanelEvent> for GhostPanel {}
impl gpui::Focusable for GhostPanel {
    fn focus_handle(&self, _: &App) -> gpui::FocusHandle {
        self.focus.clone()
    }
}
impl Panel for GhostPanel {
    fn persistent_name() -> &'static str {
        GhostPanel::KEY
    }
    fn panel_key() -> &'static str {
        GhostPanel::KEY
    }
    fn position(&self) -> DockPosition {
        DockPosition::Left
    }
    fn position_is_valid(&self, position: DockPosition) -> bool {
        position == DockPosition::Left
    }
    fn closable(&self, _cx: &App) -> bool {
        self.closable
    }
    fn enabled(&self, _cx: &App) -> bool {
        self.enabled
    }
    fn set_active(&mut self, active: bool, _cx: &mut Context<Self>) {
        self.log.borrow_mut().push(active);
    }
}

fn tree(cx: &mut TestAppContext) -> gpui::Entity<TreePanel> {
    cx.update(|app| {
        app.new(|cx| TreePanel {
            focus: cx.focus_handle(),
            log: Rc::new(std::cell::RefCell::new(Vec::new())),
        })
    })
}

fn git(cx: &mut TestAppContext) -> gpui::Entity<GitPanel> {
    cx.update(|app| {
        app.new(|cx| GitPanel {
            focus: cx.focus_handle(),
            log: Rc::new(std::cell::RefCell::new(Vec::new())),
        })
    })
}

fn ghost(cx: &mut TestAppContext, closable: bool, enabled: bool) -> gpui::Entity<GhostPanel> {
    cx.update(|app| {
        app.new(|cx| GhostPanel {
            focus: cx.focus_handle(),
            log: Rc::new(std::cell::RefCell::new(Vec::new())),
            closable,
            enabled,
        })
    })
}

fn log_of<T: ActivityLog + 'static>(entity: &gpui::Entity<T>, cx: &TestAppContext) -> Vec<bool> {
    entity.read_with(cx, |p, _| p.activity_log().borrow().clone())
}

/// 测试面板共性访问面（log_of 的类型约束）。
trait ActivityLog {
    fn activity_log(&self) -> &Log;
}

impl ActivityLog for TreePanel {
    fn activity_log(&self) -> &Log {
        &self.log
    }
}

impl ActivityLog for GitPanel {
    fn activity_log(&self) -> &Log {
        &self.log
    }
}

impl ActivityLog for GhostPanel {
    fn activity_log(&self) -> &Log {
        &self.log
    }
}

fn left_dock(cx: &mut TestAppContext) -> gpui::Entity<Dock> {
    cx.update(|app| app.new(|cx| Dock::new(DockPosition::Left, cx)))
}

// ---------------------------------------------------------------------------
// 激活协议（gpui::test 无头实体面）
// ---------------------------------------------------------------------------

#[gpui::test]
fn activate_key_resolves_and_dedups_notifications(cx: &mut TestAppContext) {
    let dock = left_dock(cx);
    let tree = tree(cx);
    let git = git(cx);

    dock.update(cx, |d, cx| {
        d.add_panel(&tree, cx);
        d.add_panel(&git, cx);
        assert_eq!(d.active_index(), None, "注册不自动激活——宿主显式激活");
        assert!(d.activate_key("git", cx));
    });
    assert_eq!(
        dock.read_with(cx, |d, _| d.active_key()),
        Some("git"),
        "按键激活生效"
    );
    // 去抖：重复激活零通知。
    dock.update(cx, |d, cx| {
        assert!(d.activate_key("git", cx));
    });
    assert_eq!(
        log_of(&git, cx),
        vec![true],
        "同位重激零新增 set_active 通知（去重）"
    );
    assert_eq!(log_of(&tree, cx), Vec::<bool>::new());
}

#[gpui::test]
fn switching_panels_deactivates_before_activates(cx: &mut TestAppContext) {
    let dock = left_dock(cx);
    let tree = tree(cx);
    let git = git(cx);
    dock.update(cx, |d, cx| {
        d.add_panel(&tree, cx);
        d.add_panel(&git, cx);
        d.activate_key("git", cx);
        d.activate_key("file-tree", cx);
    });
    assert_eq!(log_of(&git, cx), vec![true, false], "git 先激后退");
    assert_eq!(log_of(&tree, cx), vec![true], "tree 仅收激一次");
}

#[gpui::test]
fn unregistered_key_records_intent_and_materializes_on_add(cx: &mut TestAppContext) {
    let dock = left_dock(cx);

    // 预物化：git 面未注册 → false + 意图记录。
    let resolved = dock.update(cx, |d, cx| d.activate_key("git", cx));
    assert!(!resolved);
    assert_eq!(dock.read_with(cx, |d, _| d.active_key()), Some("git"));

    // 兑现：git 面注册 → 自动激活、意图清空。
    let git = git(cx);
    dock.update(cx, |d, cx| {
        d.add_panel(&git, cx);
    });
    assert_eq!(dock.read_with(cx, |d, _| d.active_index()), Some(0));
    assert_eq!(log_of(&git, cx), vec![true]);
    // 意图已兑现——active_key 走实体口径。
    assert_eq!(dock.read_with(cx, |d, _| d.active_key()), Some("git"));

    // 追加的面板不吃旧意图。
    let tree = tree(cx);
    dock.update(cx, |d, cx| {
        d.add_panel(&tree, cx);
    });
    assert_eq!(
        dock.read_with(cx, |d, _| d.active_key()),
        Some("git"),
        "后续注册不抢激活"
    );
}

#[gpui::test]
fn user_activation_supersedes_pending_intent(cx: &mut TestAppContext) {
    let dock = left_dock(cx);
    let tree = tree(cx);
    dock.update(cx, |d, cx| {
        d.add_panel(&tree, cx);
        // ghost 未注册 → 意图。
        assert!(!d.activate_key("ghost", cx));
        assert!(d.active_is("ghost"));
        // 用户实际换面（tree 在列）→ 意图丢弃（蓝本 discard_pending）。
        d.activate_key("file-tree", cx);
        assert!(d.active_is("file-tree"));
        assert_eq!(d.active_key(), Some("file-tree"), "意图被实际换面覆盖");
    });
}

#[gpui::test]
fn remove_active_panel_closes_dock_and_notifies_deactivation(cx: &mut TestAppContext) {
    let dock = left_dock(cx);
    let tree = tree(cx);
    let git = git(cx);
    dock.update(cx, |d, cx| {
        d.add_panel(&tree, cx);
        d.add_panel(&git, cx);
        d.activate_key("git", cx);
        assert!(d.remove_panel_by_key("git", cx));
        assert!(!d.remove_panel_by_key("git", cx), "已移除再移除为假");
    });
    assert_eq!(log_of(&git, cx), vec![true, false], "移除前先去激");
    assert_eq!(dock.read_with(cx, |d, _| d.active_index()), None);
    assert!(
        !dock.read_with(cx, |d, _| d.is_open()),
        "激活面板移除收起 dock（蓝本 :911）"
    );
}

#[gpui::test]
fn remove_before_active_shifts_index(cx: &mut TestAppContext) {
    let dock = left_dock(cx);
    let tree = tree(cx);
    let git = git(cx);
    dock.update(cx, |d, cx| {
        let ix_tree = d.add_panel(&tree, cx);
        d.add_panel(&git, cx);
        d.activate_key("git", cx);
        assert_eq!(ix_tree, 0);
        // 移除前面的 tree：git 的位次前移，激活身份不漂。
        d.remove_panel(0, cx);
        assert_eq!(d.active_index(), Some(0));
        assert!(d.active_is("git"));
    });
}

#[gpui::test]
fn first_enabled_index_skips_disabled(cx: &mut TestAppContext) {
    let dock = left_dock(cx);
    let tree = tree(cx);
    let ghost_disabled = ghost(cx, false, false);
    let ghost_enabled = ghost(cx, false, true);
    dock.update(cx, |d, cx| {
        let _ = d.add_panel(&ghost_disabled, cx);
        let _ = d.add_panel(&ghost_enabled, cx);
        let _ = d.add_panel(&tree, cx);
        assert_eq!(
            d.first_enabled_panel_idx(cx),
            Some(1),
            "禁用面板被 first_enabled 跳过（蓝本 :550）"
        );
    });
}

#[gpui::test]
fn panel_events_reach_dock_protocol(cx: &mut TestAppContext) {
    let dock = left_dock(cx);
    let tree = tree(cx);
    dock.update(cx, |d, cx| {
        d.add_panel(&tree, cx);
    });
    // Activate 上行 → dock 激活该面板。
    tree.update(cx, |_, cx| cx.emit(PanelEvent::Activate));
    assert_eq!(dock.read_with(cx, |d, _| d.active_index()), Some(0));

    // Close 上行但面板不可关 → 保留（closable 门二次裁决）。
    let ghost_locked = ghost(cx, false, true);
    dock.update(cx, |d, cx| {
        d.add_panel(&ghost_locked, cx);
        d.activate_key("ghost", cx);
    });
    ghost_locked.update(cx, |_, cx| cx.emit(PanelEvent::Close));
    assert_eq!(
        dock.read_with(cx, |d, _| d.panel_index_for_key("ghost")),
        Some(1),
        "不可关面板 Close 请求被拒"
    );

    // 可关面板 Close 上行 → 移除（连带激活位语义）。
    let ghost_open = ghost(cx, true, true);
    dock.update(cx, |d, cx| {
        d.remove_panel_by_key("ghost", cx);
        d.add_panel(&ghost_open, cx);
        d.activate_key("ghost", cx);
    });
    ghost_open.update(cx, |_, cx| cx.emit(PanelEvent::Close));
    assert_eq!(
        dock.read_with(cx, |d, _| d.panel_index_for_key("ghost")),
        None,
        "可关面板 Close 上行移除"
    );
    assert_eq!(
        dock.read_with(cx, |d, _| d.active_index()),
        None,
        "蓝本 ：911——移除的正是激活面板：激活位清空、dock 收起，无自动接管"
    );
    assert!(!dock.read_with(cx, |d, _| d.is_open()));
}

#[gpui::test]
fn set_open_toggles_active_panel_notification_idempotently(cx: &mut TestAppContext) {
    let dock = left_dock(cx);
    let tree = tree(cx);
    dock.update(cx, |d, cx| {
        d.add_panel(&tree, cx);
        d.activate_key("file-tree", cx);
        d.set_open(false, cx);
        d.set_open(false, cx); // 同值零通知。
        d.set_open(true, cx);
    });
    assert_eq!(
        log_of(&tree, cx),
        vec![true, false, true],
        "开/关/重开各一次通知，同值去重"
    );
    assert!(dock.read_with(cx, |d, _| d.is_open()));
}

#[gpui::test]
fn panel_id_and_position_contract(cx: &mut TestAppContext) {
    let dock = left_dock(cx);
    let tree = tree(cx);
    dock.update(cx, |d, cx| {
        let ix = d.add_panel(&tree, cx);
        let panel = d.panel_at(ix).expect("句柄在场");
        assert_eq!(panel.panel_key(), "file-tree");
        assert_eq!(panel.persistent_name(), "file-tree");
        assert_eq!(panel.position(cx), DockPosition::Left);
        assert!(panel.position_is_valid(DockPosition::Left, cx));
        assert!(!panel.position_is_valid(DockPosition::Right, cx));
        assert!(panel.starts_open(cx));
        assert_eq!(panel.panel_id(), PanelId::from(tree.entity_id()));
        assert_eq!(
            d.panel_index_for_id(PanelId::from(tree.entity_id())),
            Some(ix)
        );
    });
    assert_eq!(dock.read_with(cx, |d, _| d.position()), DockPosition::Left);
    assert_eq!(DockPosition::Left.axis(), gpui::Axis::Horizontal);
    assert_eq!(DockPosition::Bottom.axis(), gpui::Axis::Vertical);
}

// ---------------------------------------------------------------------------
// 批②持久化面（#120 验收 1/2/3：round-trip 全等 / 版本迁移 / 坏值容错）
// ---------------------------------------------------------------------------

#[gpui::test]
fn dump_restore_round_trips_all_fields(cx: &mut TestAppContext) {
    let dock = left_dock(cx);
    let tree_e = tree(cx);
    let git_e = git(cx);
    dock.update(cx, |d, cx| {
        d.add_panel(&tree_e, cx);
        d.add_panel(&git_e, cx);
        d.activate_key("git", cx);
        d.set_panel_size("file-tree", 312.5, cx);
        d.set_panel_size("git", 268.0, cx);
        d.set_open(false, cx);
    });
    // dump → serde 往返 → restore 到新 dock 实体——全等。
    let state = dock.read_with(cx, |d, _| d.dump());
    let json = serde_json::to_string(&state).expect("serde 序列化");
    let parsed: DockAreaState = serde_json::from_str(&json).expect("serde 反序列化");
    let dock2 = left_dock(cx);
    let tree2 = tree(cx);
    let git2 = git(cx);
    dock2.update(cx, |d, cx| {
        d.add_panel(&tree2, cx);
        d.add_panel(&git2, cx);
        d.restore(parsed, cx).expect("同版恢复");
    });
    let state2 = dock2.read_with(cx, |d, _| d.dump());
    assert_eq!(state, state2, "round-trip：dump→restore→dump 全等");
    assert_eq!(state.version, DOCK_LAYOUT_VERSION);
    assert_eq!(state.active.as_deref(), Some("git"));
    assert!(!state.open, "开合态随快照");
    assert_eq!(
        state.panels,
        vec![
            PanelState {
                key: "file-tree".into(),
                size: Some(312.5)
            },
            PanelState {
                key: "git".into(),
                size: Some(268.0)
            },
        ],
        "面板集与尺寸逐条快照"
    );
}

#[gpui::test]
fn version_mismatch_discards_and_rebuilds_default(cx: &mut TestAppContext) {
    let dock = left_dock(cx);
    let tree = tree(cx);
    dock.update(cx, |d, cx| {
        d.add_panel(&tree, cx);
        d.activate_key("file-tree", cx);
        // 旧版本号（0）——升级迁移：整份丢弃。
        let old = DockAreaState {
            version: 0,
            open: false,
            active: Some("file-tree".into()),
            panels: vec![PanelState {
                key: "file-tree".into(),
                size: Some(999.),
            }],
        };
        let err = d.restore(old, cx).expect_err("版本不符应拒");
        assert_eq!(
            err,
            DockLayoutVersionMismatch {
                found: 0,
                expected: DOCK_LAYOUT_VERSION
            },
            "诊断面携带实得/期望两值"
        );
    });
    // 丢弃语义：旧面板尺寸/激活态不残留（重建默认——面板集在、
    // 尺寸无、激活保持 restore 前状态）。
    dock.update(cx, |d, cx| {
        assert_eq!(d.panel_size("file-tree"), None, "旧尺寸未写入");
        d.set_open(true, cx);
    });
    let fresh = dock.read_with(cx, |d, _| d.dump());
    assert_eq!(
        fresh,
        DockAreaState {
            version: DOCK_LAYOUT_VERSION,
            open: true,
            // 激活态不受被拒恢复影响（restore 在写入前已返回）。
            active: Some("file-tree".into()),
            panels: vec![PanelState {
                key: "file-tree".into(),
                size: None
            }],
        },
        "丢弃语义：旧尺寸不残留，存量状态不被污染"
    );
}

#[gpui::test]
fn restore_tolerates_unknown_keys_and_missing_panels(cx: &mut TestAppContext) {
    let dock = left_dock(cx);
    let tree = tree(cx);
    dock.update(cx, |d, cx| {
        d.add_panel(&tree, cx);
        // 未来面板键 + 未知键 + 未物化激活键——全静默、不崩。
        d.restore(
            DockAreaState {
                version: DOCK_LAYOUT_VERSION,
                open: true,
                active: Some("future-panel".into()),
                panels: vec![
                    PanelState {
                        key: "file-tree".into(),
                        size: Some(301.),
                    },
                    PanelState {
                        key: "no-such-panel".into(),
                        size: Some(42.),
                    },
                ],
            },
            cx,
        )
        .expect("未知键不阻恢复");
        assert_eq!(d.panel_size("file-tree"), Some(301.), "在列面板尺寸生效");
        assert_eq!(d.panel_size("no-such-panel"), None, "未知键丢弃");
        assert_eq!(d.active_index(), None, "未物化激活键静默丢弃（不灌意图）");
    });
}

#[test]
fn polluted_json_is_host_layer_default_bound() {
    // 坏值容错的宿主层契约：截断/污染 JSON 在 serde_json 解析层即拒
    //（宿主 settings 捕获后回默认——见 mang-shell settings.rs 测试面）。
    // 本测锁结构形状：字段名冻结 + 坏结构拒绝。
    let good = r#"{"version":1,"open":true,"active":null,"panels":[]}"#;
    let parsed: Result<DockAreaState, _> = serde_json::from_str(good);
    assert!(parsed.is_ok(), "合法结构应解");
    for bad in [
        "{",
        "not json at all",
        r#"{"version":1,"open":"yes","active":null,"panels":[]}"#,
        r#"{"panels":[]}"#,
    ] {
        let bad_parsed: Result<DockAreaState, _> = serde_json::from_str(bad);
        assert!(bad_parsed.is_err(), "坏值应拒：{bad}");
    }
}

// ---------------------------------------------------------------------------
// 注册表（批② 布局恢复的消费契约）
// ---------------------------------------------------------------------------

#[gpui::test]
fn restore_pending_size_consumed_on_materialization(cx: &mut TestAppContext) {
    let dock = left_dock(cx);
    // 恢复时无任何面板——尺寸记入预物化意图（真实流：重启后
    // 「打开文件夹」才建树）。
    dock.update(cx, |d, cx| {
        d.restore(
            DockAreaState {
                version: DOCK_LAYOUT_VERSION,
                open: true,
                active: None,
                panels: vec![PanelState {
                    key: "file-tree".into(),
                    size: Some(298.),
                }],
            },
            cx,
        )
        .expect("恢复");
    });
    // 物化树面板——意图兑现。
    let tree_e = tree(cx);
    dock.update(cx, |d, cx| {
        d.add_panel(&tree_e, cx);
    });
    assert_eq!(
        dock.read_with(cx, |d, _| d.panel_size("file-tree")),
        Some(298.),
        "预物化尺寸意图在面板注册时兑现"
    );
    assert_eq!(
        dock.read_with(cx, |d, _| d.dump()).panels[0].size,
        Some(298.),
        "dump 出已兑现值（快照不依赖意图）"
    );
}

#[gpui::test]
fn registry_rebuilds_by_persistent_name(cx: &mut TestAppContext) {
    cx.update(|app| {
        PanelRegistry::register_global(
            "file-tree",
            Arc::new(|cx: &mut App| {
                let entity: gpui::Entity<TreePanel> = cx.new(|cx| TreePanel {
                    focus: cx.focus_handle(),
                    log: Default::default(),
                });
                entity.into()
            }),
            app,
        );
    });
    let view = cx.update(|app| PanelRegistry::build_global("file-tree", app));
    assert!(
        view.is_some(),
        "按 persistent_name 重建实体（批② 恢复面契约）"
    );
    let missing = cx.update(|app| PanelRegistry::build_global("no-such-panel", app));
    assert!(missing.is_none(), "未知名静默 None——坏布局跳过的裁决入口");
    let names = cx.update(|app| {
        app.try_global::<PanelRegistry>()
            .map(|r| r.names())
            .unwrap_or_default()
    });
    assert_eq!(names, vec!["file-tree"]);
}

// ---------------------------------------------------------------------------
// #121 批③a：拖拽停靠数据面（drop zone 几何 / 落位裁决 / take-receive
// 协议 / 挤掉原槽位 / 记忆尺寸随行 / round-trip）
// ---------------------------------------------------------------------------

use crate::drag::{
    drop_is_intended, drop_zone_at, placement_for, DockDropZone, DraggedPanel, Placement,
};

/// 可停靠三侧的面板（拖拽换侧的裁决面——区别于 Left-only 的
/// TreePanel/GitPanel）。
struct RoverPanel {
    focus: gpui::FocusHandle,
}

impl RoverPanel {
    const KEY: &'static str = "rover";
}

impl gpui::Render for RoverPanel {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div()
    }
}
impl gpui::EventEmitter<PanelEvent> for RoverPanel {}
impl gpui::Focusable for RoverPanel {
    fn focus_handle(&self, _: &App) -> gpui::FocusHandle {
        self.focus.clone()
    }
}
impl Panel for RoverPanel {
    fn persistent_name() -> &'static str {
        RoverPanel::KEY
    }
    fn panel_key() -> &'static str {
        RoverPanel::KEY
    }
    fn position(&self) -> DockPosition {
        // 偏好左（「两侧占满时替换偏好侧」的裁决输入）。
        DockPosition::Left
    }
    fn position_is_valid(&self, position: DockPosition) -> bool {
        matches!(
            position,
            DockPosition::Left | DockPosition::Right | DockPosition::Bottom
        )
    }
}

fn rover(cx: &mut TestAppContext) -> gpui::Entity<RoverPanel> {
    cx.update(|app| {
        app.new(|cx| RoverPanel {
            focus: cx.focus_handle(),
        })
    })
}

fn window_bounds() -> gpui::Bounds<gpui::Pixels> {
    // 900×600 假想窗口（分区线：x 300/600，y 400）。
    gpui::Bounds {
        origin: gpui::point(gpui::px(0.), gpui::px(0.)),
        size: gpui::size(gpui::px(900.), gpui::px(600.)),
    }
}

#[test]
fn drop_zone_geometry_covers_three_sides_and_center() {
    let b = window_bounds();
    // 左右条带（底条带之上的两翼）。
    assert_eq!(
        drop_zone_at(b, gpui::point(gpui::px(150.), gpui::px(100.))),
        DockDropZone::Left
    );
    assert_eq!(
        drop_zone_at(b, gpui::point(gpui::px(750.), gpui::px(100.))),
        DockDropZone::Right
    );
    // 底条带全宽优先（左中右水平位置都落底）。
    for x in [50., 450., 850.] {
        assert_eq!(
            drop_zone_at(b, gpui::point(gpui::px(x), gpui::px(500.))),
            DockDropZone::Bottom,
            "y > 2/3 全宽落底（x={x}）"
        );
    }
    // 中心合并区（中三分之一 × 底条带之上）。
    assert_eq!(
        drop_zone_at(b, gpui::point(gpui::px(450.), gpui::px(100.))),
        DockDropZone::Center
    );
    // 越界点 clamp 后判定（窗口外 drop 不发生，防御口径）。
    assert_eq!(
        drop_zone_at(b, gpui::point(gpui::px(-50.), gpui::px(-50.))),
        DockDropZone::Left
    );
    assert_eq!(
        drop_zone_at(b, gpui::point(gpui::px(9999.), gpui::px(0.))),
        DockDropZone::Right
    );
}

#[test]
fn placement_respects_validity_and_preference() {
    use DockPosition::{Bottom, Left, Right};
    let all_valid = |_: DockPosition| true;
    // 中心区 → 取消。
    assert_eq!(
        placement_for(DockDropZone::Center, Left, all_valid),
        Placement::Cancel
    );
    // 边分区合法 → 落位该侧。
    assert_eq!(
        placement_for(DockDropZone::Right, Left, all_valid),
        Placement::Dock(Right)
    );
    assert_eq!(
        placement_for(DockDropZone::Bottom, Left, all_valid),
        Placement::Dock(Bottom)
    );
    // 分区非法 → 回偏好位（Left-only 面板拖到右分区 → 回左）。
    let left_only = |p: DockPosition| p == Left;
    assert_eq!(
        placement_for(DockDropZone::Right, Left, left_only),
        Placement::Dock(Left)
    );
    // 分区与偏好位都非法 → 取消（面板声明自相矛盾的保守兜底）。
    let nothing_valid = |_: DockPosition| false;
    assert_eq!(
        placement_for(DockDropZone::Right, Left, nothing_valid),
        Placement::Cancel
    );
}

#[test]
fn drop_is_intended_rejects_same_side_drop() {
    let a = gpui::EntityId::from(1u64);
    let b = gpui::EntityId::from(2u64);
    let dragged = DraggedPanel {
        source_dock: a,
        key: "rover",
    };
    assert!(drop_is_intended(dragged, b), "跨侧移动正常落位");
    assert!(
        !drop_is_intended(dragged, a),
        "拖回源侧 = 取消语义（不落位）"
    );
}

#[gpui::test]
fn take_panel_activates_next_and_keeps_dock_open(cx: &mut TestAppContext) {
    let dock = left_dock(cx);
    let tree = tree(cx);
    let git = git(cx);
    dock.update(cx, |d, cx| {
        d.add_panel(&tree, cx);
        d.add_panel(&git, cx);
        d.activate_key("git", cx);
        d.set_panel_size("git", 240., cx);
    });
    // 拔出激活面板 git：tree 接管激活，dock 保持开（拖拽不是关栏——
    // 与 remove_panel 收起语义的分界裁决）。
    let detached = dock
        .update(cx, |d, cx| d.take_panel("git", cx))
        .expect("激活面板在列可拔");
    assert_eq!(detached.panel.panel_key(), "git");
    assert_eq!(detached.size, Some(240.), "记忆尺寸随面板走");
    dock.read_with(cx, |d, _| {
        assert_eq!(d.panels_len(), 1);
        assert_eq!(d.active_key(), Some("file-tree"), "次面板接管激活");
        assert!(d.is_open(), "拔出激活面板不收起 dock");
    });
    // 拔非激活面板：激活不变。
    let tree_detached = dock
        .update(cx, |d, cx| d.take_panel("file-tree", cx))
        .expect("非激活面板在列可拔");
    dock.read_with(cx, |d, _| {
        assert_eq!(d.panels_len(), 0);
        assert_eq!(d.active_key(), None);
        assert!(d.is_open());
    });
    // 键不在列：None（拖起与落位间竞态防御）。
    let missing = dock.update(cx, |d, cx| d.take_panel("git", cx));
    assert!(missing.is_none());
    std::mem::forget(tree_detached);
}

#[gpui::test]
fn receive_panel_takes_the_slot_and_inherits_size(cx: &mut TestAppContext) {
    // 右侧 dock 已有激活面板 rover（占槽）→ 接收左拖来的 git =
    // 挤掉原槽位（git 落位即激活，rover 让位留列）。
    let left = left_dock(cx);
    let right = cx.update(|app| app.new(|cx| Dock::new(DockPosition::Right, cx)));
    let rover = rover(cx);
    let git = git(cx);
    left.update(cx, |d, cx| {
        d.add_panel(&git, cx);
        d.activate_key("git", cx);
        d.set_panel_size("git", 240., cx);
    });
    right.update(cx, |d, cx| {
        d.add_panel(&rover, cx);
        d.activate_key("rover", cx);
    });
    // 宿主 drop 收尾序列（批③b 渲染面照此接线）：
    // 1) placement 裁决（右分区 × rover 合法性——此处 git 拖到右分区，
    //    git 的 position_is_valid 由面板型声明，测试面板 GitPanel 是
    //    Left-only → 裁决回偏好位左。**换 RoverPanel 做被拖面板**才
    //    有跨侧合法性——本用例走 rover 拖到右已是右，改由 git 先
    //    落到右 dock 的替代路径：直接协议面验证（take → receive），
    //    几何/裁决由上面纯函数测试覆盖。
    // 2) take（源）→ receive（目标）。
    let detached = left
        .update(cx, |d, cx| d.take_panel("git", cx))
        .expect("git 在左 dock 在列");
    right.update(cx, |d, cx| {
        let ix = d.receive_panel(detached, cx);
        assert_eq!(ix, 1, "追加在 rover 之后");
        assert_eq!(d.active_key(), Some("git"), "落位即激活（挤掉原槽位）");
        assert!(d.is_open(), "落位侧打开");
    });
    // 记忆尺寸继承：落位侧 dump 带原尺寸（round-trip 尺寸面）。
    let snapshot = right.update(cx, |d, _| d.dump());
    assert_eq!(
        snapshot
            .panels
            .iter()
            .find(|p| p.key == "git")
            .and_then(|p| p.size),
        Some(240.),
        "面板记忆尺寸跨 dock 随行"
    );
    // 源侧快照不再含 git（双向一致性）。
    let left_snapshot = left.update(cx, |d, _| d.dump());
    assert!(
        !left_snapshot.panels.iter().any(|p| p.key == "git"),
        "拔出后源侧 dump 不再含该面板"
    );
}

#[gpui::test]
fn drag_round_trip_survives_dump_restore(cx: &mut TestAppContext) {
    // 验收「拖拽后布局持久化 round-trip」：rover 从左 dock 拖到右
    // dock（真实序列：placement 裁决 → take → receive → set_position
    // 语义由宿主面板承担）后 dump → restore 进全新右 dock，激活
    // 与尺寸无损。
    let left = left_dock(cx);
    let right = cx.update(|app| app.new(|cx| Dock::new(DockPosition::Right, cx)));
    let rover_entity = rover(cx);
    left.update(cx, |d, cx| {
        d.add_panel(&rover_entity, cx);
        d.activate_key("rover", cx);
        d.set_panel_size("rover", 320., cx);
    });
    // 拖拽序列：裁决（右分区合法）→ take → receive。
    let placement = placement_for(
        DockDropZone::Right,
        DockPosition::Left,
        |p| p != DockPosition::Bottom,
    );
    assert_eq!(placement, Placement::Dock(DockPosition::Right));
    let detached = left.update(cx, |d, cx| d.take_panel("rover", cx)).unwrap();
    right.update(cx, |d, cx| {
        d.receive_panel(detached, cx);
    });
    // round-trip：dump 右 → restore 进新右 dock。批② restore 语义
    // 只兑现当下面板（未注册键静默丢弃）——fresh 里先物化 rover，
    // dump 的激活/尺寸才能经 restore 无损兑现（真实流：重启后
    // PanelRegistry 按键重建 → restore）。
    let snapshot = right.update(cx, |d, _| d.dump());
    let fresh = cx.update(|app| app.new(|cx| Dock::new(DockPosition::Right, cx)));
    let rover_here = rover(cx);
    fresh.update(cx, |d, cx| {
        d.add_panel(&rover_here, cx);
        d.restore(snapshot.clone(), cx).unwrap();
        assert_eq!(
            d.active_key(),
            Some("rover"),
            "restore 兑现拖拽后的激活面"
        );
        assert_eq!(
            d.panel_size("rover"),
            Some(320.),
            "restore 兑现拖拽后的记忆尺寸"
        );
    });
    assert_eq!(
        snapshot.active.as_deref(),
        Some("rover"),
        "dump 携带拖拽后的激活键"
    );
}
