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
// 注册表（批② 布局恢复的消费契约）
// ---------------------------------------------------------------------------

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
