//! Dock 实体与激活协议（#119 批①）——蓝本 zed `dock.rs:283-998`。
//!
//! 结构对齐蓝本 `Dock`（dock.rs:283-294）：每侧一个 dock 实体持
//! `Vec<PanelEntry>`（蓝本 :383-387），激活协议 `activate_panel`
//! （:936-961 去重 + 先去激后激）与 `first_enabled_panel_idx`
//! （:550-560）。蓝本 `DockRestoreState::Restoring{pending}`（:296-314）
//! 在本批泛化为「预物化激活意图」（面板未注册时记录 key、物化即激活）
//! ——批② 布局恢复将复用同一 pending 语义。

use std::sync::Arc;

use gpui::{App, Context, FocusHandle, Focusable, Subscription};

use super::handle::{PanelHandle, PanelId};
use super::panel::{Panel, PanelEvent};

/// dock 边（蓝本 dock.rs:322-327）。蓝本的 settings `From` 换位面归
/// 后续批次（PROVENANCE delta）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockPosition {
    Left,
    Bottom,
    Right,
}

impl DockPosition {
    /// 尺寸轴（蓝本 dock.rs:368-373——批③ drop zone 与批② 尺寸
    /// 持久化的共享裁决输入）。
    pub fn axis(self) -> gpui::Axis {
        match self {
            Self::Left | Self::Right => gpui::Axis::Horizontal,
            Self::Bottom => gpui::Axis::Vertical,
        }
    }
}

/// 激活协议纯状态机（去重/去抖——gpui-base `active.rs:20` 设计：
/// 同目标重激活零通知；换目标产出（去激，激）一对）。
///
/// 零 gpui 依赖可单测；[`Dock`] 持有它作为激活下标的唯一事实源。
#[derive(Default)]
pub struct ActiveTracker {
    current: Option<usize>,
}

impl ActiveTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前激活下标。
    pub fn current(&self) -> Option<usize> {
        self.current
    }

    /// 激活换位。返回 `(去激下标, 激活下标)`；同目标重入返回
    /// `(None, None)`（去重——重复激活零副作用）。
    pub fn transition(&mut self, target: Option<usize>) -> (Option<usize>, Option<usize>) {
        if self.current == target {
            return (None, None);
        }
        let deactivate = self.current;
        self.current = target;
        (deactivate, target)
    }

    /// 移除下标 `ix` 的条目后修正激活位（蓝本 dock.rs:904-915 的
    /// 索引语义：前移收缩、命中激活位清空、后位无关）。
    /// 返回 `Some(ix)` 当被移除的正是激活面板。
    pub fn remove_at(&mut self, ix: usize) -> Option<usize> {
        match self.current {
            Some(a) if a == ix => {
                self.current = None;
                Some(ix)
            }
            Some(a) if a > ix => {
                self.current = Some(a - 1);
                None
            }
            _ => None,
        }
    }
}

/// 面板条目（蓝本 dock.rs:383-387：面板句柄 + 订阅随条目存续——
/// 订阅随条目 Drop 自动退订，故仅供存续持有效不读）。
pub struct PanelEntry {
    panel: Arc<dyn PanelHandle>,
    #[allow(dead_code)] // 存续语义：Drop 即退订（蓝本同构）。
    subscriptions: Vec<Subscription>,
    /// #120（批②）：per-panel 记忆尺寸（px；None = 无记忆——
    /// 蓝本 dock.rs:377-381 `PanelSizeState` 的单 dock 简化面；
    /// 拖拽结束由宿主回写，渲染宽由宿主读激活面板值）。
    size: Option<f32>,
}

impl PanelEntry {
    /// 面板句柄（测试/宿主读取面）。
    pub fn panel(&self) -> &Arc<dyn PanelHandle> {
        &self.panel
    }

    /// #121（批③a）：拖拽中转态重建构造面（面板从源 dock 拔出后
    /// 原样落位目标侧——订阅与记忆尺寸随行，不退订不丢尺寸）。
    pub(crate) fn from_parts(
        panel: Arc<dyn PanelHandle>,
        subscriptions: Vec<Subscription>,
        size: Option<f32>,
    ) -> Self {
        Self {
            panel,
            subscriptions,
            size,
        }
    }

    /// #121（批③a）：拔出拆解面（`take_panel` 中转态构造用——记忆
    /// 尺寸随面板走，落位侧继承）。
    pub(crate) fn into_parts(self) -> (Arc<dyn PanelHandle>, Vec<Subscription>, Option<f32>) {
        (self.panel, self.subscriptions, self.size)
    }
}

/// Dock 实体：单边面板列表 + 开合态 + 激活协议（蓝本 dock.rs:283）。
pub struct Dock {
    position: DockPosition,
    entries: Vec<PanelEntry>,
    is_open: bool,
    tracker: ActiveTracker,
    /// 预物化激活意图（key 未注册时 `activate_key` 记录；物化即激活；
    /// 用户实际换面时丢弃——蓝本 dock.rs:938 discard_pending 同语义）。
    pending_activation: Option<&'static str>,
    /// #120（批②）：预物化尺寸意图（restore 时未注册面板的记忆宽——
    /// 面板物化时兑现；与 pending_activation 对称。真实流：重启后
    /// 「打开文件夹」建树 → 上次的树宽恢复）。
    pending_sizes: std::collections::HashMap<String, f32>,
    focus_handle: FocusHandle,
}

impl Focusable for Dock {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Dock {
    pub fn new(position: DockPosition, cx: &mut Context<Self>) -> Self {
        Self {
            position,
            entries: Vec::new(),
            is_open: true,
            tracker: ActiveTracker::new(),
            pending_activation: None,
            pending_sizes: std::collections::HashMap::new(),
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn position(&self) -> DockPosition {
        self.position
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    /// 开合（蓝本 dock.rs:583-592：变更是前提；激活面板随开合收
    /// `set_active`；去重——同值零通知）。
    pub fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if open != self.is_open {
            self.is_open = open;
            if let Some(active) = self.tracker.current()
                && let Some(entry) = self.entries.get(active)
            {
                entry.panel.set_active(open, cx);
            }
            cx.notify();
        }
    }

    pub fn panels_len(&self) -> usize {
        self.entries.len()
    }

    pub fn active_index(&self) -> Option<usize> {
        self.tracker.current()
    }

    /// 激活面板句柄（无激活面板 = None）。
    pub fn active_panel(&self) -> Option<&Arc<dyn PanelHandle>> {
        self.tracker
            .current()
            .and_then(|ix| self.entries.get(ix))
            .map(|entry| &entry.panel)
    }

    pub fn panel_at(&self, ix: usize) -> Option<&Arc<dyn PanelHandle>> {
        self.entries.get(ix).map(|entry| &entry.panel)
    }

    /// 按键查位（panel_key 与 persistent_name 双口径——蓝本
    /// dock.rs:525-535 两个查询的合一便捷面）。
    pub fn panel_index_for_key(&self, key: &str) -> Option<usize> {
        self.entries.iter().position(|entry| {
            entry.panel.panel_key() == key || entry.panel.persistent_name() == key
        })
    }

    /// 按实体 id 查位（蓝本 dock.rs:537-541 proto_id 查的解绑版——
    /// PanelId 为 EntityId 派生）。
    pub fn panel_index_for_id(&self, id: PanelId) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.panel.panel_id() == id)
    }

    /// 当前激活面板键——**预物化意图优先**（面板未注册时 `activate_key`
    /// 的记录就是用户当前视图选择，渲染面据此呈现占位视图；物化后面板
    /// 实际激活，语义无缝衔接）。
    pub fn active_key(&self) -> Option<&'static str> {
        self.pending_activation.or_else(|| {
            self.tracker
                .current()
                .and_then(|ix| self.entries.get(ix))
                .map(|entry| entry.panel.panel_key())
        })
    }

    /// 视图归属裁决（宿主渲染面的便捷判定）。
    pub fn active_is(&self, key: &str) -> bool {
        self.active_key() == Some(key)
    }

    /// 首个启用面板位（蓝本 dock.rs:550-560；无启用面板 = None——
    /// 蓝本为 anyhow Result，本批宿主自行缺省，PROVENANCE delta）。
    pub fn first_enabled_panel_idx(&self, cx: &App) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.panel.enabled(cx))
    }

    /// 注册面板（蓝本 dock.rs AddPanel 区：订阅事件 + 入列）。若此前
    /// `activate_key` 记录过本面板的预物化意图——立即激活（意图兑现）。
    pub fn add_panel<T: Panel>(
        &mut self,
        panel: &gpui::Entity<T>,
        cx: &mut Context<Self>,
    ) -> usize {
        let subscription = cx.subscribe(panel, |this, entity, event: &PanelEvent, cx| {
            let id = PanelId::from(entity.entity_id());
            let Some(ix) = this.panel_index_for_id(id) else {
                return;
            };
            match event {
                PanelEvent::Activate => this.activate_panel(ix, cx),
                // 可关门裁决：不可关面板的 Close 请求落到 `_` 忽略
                //（蓝本 closable 二次裁决同款）。
                PanelEvent::Close if this.entries[ix].panel.closable(cx) => {
                    this.remove_panel(ix, cx);
                }
                _ => {}
            }
        });
        self.entries.push(PanelEntry {
            panel: Arc::new(panel.clone()),
            subscriptions: vec![subscription],
            size: None,
        });
        let ix = self.entries.len() - 1;
        // #120（批②）：预物化尺寸意图兑现（restore 时面板未注册——
        // 物化时吃回）。
        if let Some(size) = self.pending_sizes.remove(T::panel_key()) {
            self.entries[ix].size = Some(size);
        }
        if self
            .pending_activation
            .is_some_and(|k| k == T::panel_key() || k == T::persistent_name())
        {
            self.pending_activation = None;
            self.activate_panel(ix, cx);
        }
        cx.notify();
        ix
    }

    /// 移除面板（蓝本 dock.rs:893-924：激活位命中则清空并把 dock
    /// 收起；前移位收缩；后位无关）。
    pub fn remove_panel(&mut self, ix: usize, cx: &mut Context<Self>) -> bool {
        if ix >= self.entries.len() {
            return false;
        }
        if let Some(removed_active) = self.tracker.remove_at(ix) {
            if let Some(entry) = self.entries.get(removed_active) {
                entry.panel.set_active(false, cx);
            }
            self.set_open(false, cx);
        }
        self.entries.remove(ix);
        cx.notify();
        true
    }

    /// 按键移除（宿主换工作区重建面板列的便捷面——键在列则真移）。
    pub fn remove_panel_by_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        match self.panel_index_for_key(key) {
            Some(ix) => self.remove_panel(ix, cx),
            None => false,
        }
    }

    /// 激活协议（蓝本 dock.rs:936-961：去重（同位零副作用）、用户实际
    /// 换面丢弃预物化意图、先去激后激、notify）。
    pub fn activate_panel(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.entries.len() || self.tracker.current() == Some(ix) {
            return;
        }
        self.pending_activation = None;
        let (deactivate, activate) = self.tracker.transition(Some(ix));
        if let Some(prev) = deactivate
            && let Some(entry) = self.entries.get(prev)
        {
            entry.panel.set_active(false, cx);
        }
        if let Some(now) = activate
            && let Some(entry) = self.entries.get(now)
        {
            entry.panel.set_active(true, cx);
        }
        cx.notify();
    }

    /// 按键激活：面板在列 → 激活返回 `true`；未注册 → 记录预物化意图
    /// 返回 `false`（渲染面据 `active_key` 呈现占位视图——面板物化时
    /// 由 `add_panel` 自动兑现）。
    pub fn activate_key(&mut self, key: &'static str, cx: &mut Context<Self>) -> bool {
        match self.panel_index_for_key(key) {
            Some(ix) => {
                self.activate_panel(ix, cx);
                true
            }
            None => {
                self.pending_activation = Some(key);
                cx.notify();
                false
            }
        }
    }

    /// #120（批②）：按键读记忆尺寸（无面板/无记忆 = None）。
    pub fn panel_size(&self, key: &str) -> Option<f32> {
        self.panel_index_for_key(key)
            .and_then(|ix| self.entries[ix].size)
    }

    /// #120：按键写记忆尺寸（面板在列才存——未注册键丢弃；
    /// 新值与旧值相等零通知）。
    pub fn set_panel_size(&mut self, key: &str, size: f32, cx: &mut Context<Self>) {
        if let Some(ix) = self.panel_index_for_key(key) {
            if self.entries[ix].size != Some(size) {
                self.entries[ix].size = Some(size);
                cx.notify();
            }
        }
    }

    /// #120：restore 内部写入面（恢复期不逐面板 notify——末尾统一
    /// set_open/activate 通知）。
    fn set_panel_size_at(&mut self, ix: usize, size: Option<f32>) {
        if let Some(entry) = self.entries.get_mut(ix) {
            entry.size = size;
        }
    }

    /// #120：清全部面板记忆尺寸（重置语义——宿主「重置默认宽」调用）。
    pub fn clear_panel_sizes(&mut self, cx: &mut Context<Self>) {
        let had = self.entries.iter().any(|entry| entry.size.is_some());
        if had {
            for entry in &mut self.entries {
                entry.size = None;
            }
            cx.notify();
        }
    }

    /// #120（批②）：布局快照（版本号写 [`DOCK_LAYOUT_VERSION`]；
    /// 激活键走 `active_key()`——预物化意图一并携带；面板尺寸按条目
    /// 记忆值）。serde 数据面见 [`super::state`]。
    pub fn dump(&self) -> super::state::DockAreaState {
        super::state::DockAreaState {
            version: super::state::DOCK_LAYOUT_VERSION,
            open: self.is_open,
            active: self.active_key().map(str::to_owned),
            panels: self
                .entries
                .iter()
                .map(|entry| super::state::PanelState {
                    key: entry.panel.panel_key().to_owned(),
                    size: entry.size,
                })
                .collect(),
        }
    }

    /// #120（批②）：布局恢复。**版本不符 → 整份丢弃回默认**（返回
    /// `Err` 交宿主静默重建，Q-IDE layout.rs:518-521 裁决同款；坏
    /// JSON 由宿主 serde_json 解析层拦——本层只见合法结构）。
    ///
    /// 恢复面：开合态（`set_open` 协议）；激活键（仅当下面板真激活
    /// ——未注册键静默丢弃，不灌预物化意图：恢复键集仅当下面板，
    /// 坏键不崩）；面板尺寸（按键匹配写入记忆值——未注册键忽略，
    /// 越界值由宿主 clamp 收口）。
    pub fn restore(
        &mut self,
        state: super::state::DockAreaState,
        cx: &mut Context<Self>,
    ) -> Result<(), super::state::DockLayoutVersionMismatch> {
        if state.version != super::state::DOCK_LAYOUT_VERSION {
            return Err(super::state::DockLayoutVersionMismatch {
                found: state.version,
                expected: super::state::DOCK_LAYOUT_VERSION,
            });
        }
        self.set_open(state.open, cx);
        for panel in &state.panels {
            if let Some(ix) = self.panel_index_for_key(&panel.key) {
                self.set_panel_size_at(ix, panel.size);
            } else if let Some(size) = panel.size {
                // 面板未物化：记入预物化尺寸意图（物化时兑现——
                // 真实流：重启后打开文件夹才建树）。
                self.pending_sizes.insert(panel.key.clone(), size);
            }
        }
        if let Some(key) = state.active {
            if let Some(ix) = self.panel_index_for_key(&key) {
                self.activate_panel(ix, cx);
            }
        }
        Ok(())
    }

    /// #121（批③a）：拔出面板（拖拽协议——面板离开源 dock，进拖拽
    /// 中转态 [`super::drag::PanelDetachment`]）。
    ///
    /// 与 [`Self::remove_panel`] 的分界：**不收起 dock**（remove 的
    /// 激活位命中收起语义是「关闭面板」；拖拽换侧不是用户关栏意图）
    /// ——激活位修正：命中则切首个启用面板（无则清空），订阅随行不
    /// 退订，记忆尺寸随行。返回 `None`：键不在列（拖起与落位间的
    /// 竞态防御）。
    pub fn take_panel(
        &mut self,
        key: &str,
        cx: &mut Context<Self>,
    ) -> Option<super::drag::PanelDetachment> {
        let ix = self.panel_index_for_key(key)?;
        let was_active = self.tracker.current() == Some(ix);
        let (panel, subscriptions, size) = self.entries.remove(ix).into_parts();
        // 激活位修正（remove_at 的索引语义：命中清空/前移收缩）。
        self.tracker.remove_at(ix);
        if was_active && let Some(next) = self.first_enabled_panel_idx(cx) {
            // 拔的是激活面板：切首个启用面板保视图（无启用面 = 无激活，
            // dock 保持开——拖拽不是关栏）。
            self.tracker.transition(Some(next));
            if let Some(entry) = self.entries.get(next) {
                entry.panel.set_active(true, cx);
            }
        }
        cx.notify();
        Some(super::drag::PanelDetachment {
            panel,
            subscriptions,
            size,
        })
    }

    /// #121（批③a）：落位接收（拖拽中转态进目标 dock）。
    ///
    /// 落位即激活——**挤掉原槽位语义**（Q-IDE `app/dock.rs:550-567`：
    /// 拖到已占用侧 = 被拖面板成为该侧激活面板，原激活面板让位留列
    /// 可切换）。落位侧打开。返回新激活下标。
    pub fn receive_panel(
        &mut self,
        detachment: super::drag::PanelDetachment,
        cx: &mut Context<Self>,
    ) -> usize {
        let super::drag::PanelDetachment {
            panel,
            subscriptions,
            size,
        } = detachment;
        self.entries
            .push(PanelEntry::from_parts(panel, subscriptions, size));
        let ix = self.entries.len() - 1;
        self.activate_panel(ix, cx);
        self.set_open(true, cx);
        ix
    }
}
