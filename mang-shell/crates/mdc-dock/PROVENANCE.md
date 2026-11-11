# PROVENANCE —— mdc-dock 溯源与 delta 三栏

> Lead 裁决：#105 评论（2026-11-11）批准方案 C（zed dock.rs 蓝本移植自建
> mdc-dock）；比对报告与路径分析见 #106。

## 一、蓝本来源

| 项 | 值 |
|---|---|
| 上游仓库 | https://github.com/zed-industries/zed |
| 蓝本文件 | `crates/workspace/src/dock.rs` |
| 蓝本 commit | `1663af412c625417759561c0d15ab5a1668aafa1`（与 third_party/zed 闭包同 rev——`third_party/zed/PROVENANCE.md`） |
| 许可 | zed 仓库闭包同源（GPL 面与本 crate `GPL-3.0-or-later` 一致） |
| 移植方式 | **蓝本逐函数对照**（非物理搬运）——锚点逐项列于 §二 |

## 二、逐区对照（蓝本锚点 → 本 crate 落点）

| 蓝本区 | 蓝本锚点 | 本 crate | 说明 |
|---|---|---|---|
| `PanelEvent` | dock.rs:27-32 | `panel.rs` | ZoomIn/ZoomOut 本批仅定义（zoom 后续批） |
| `PanelId` | dock.rs:34（`pub use proto::PanelId`） | `handle.rs` | **解绑①**：协作 protobuf → `EntityId` 派生新类型（仿 gpui-base `layout/node.rs:33` `From<EntityId>`） |
| `Panel` trait | dock.rs:36-104 | `panel.rs` | 保留面：persistent_name/panel_key/activation_focus_handle/position 群/enabled/set_active/activation_priority/starts_open；**解绑②**：`pane()`/`remote_id()`/`hide_button_setting()`/`is_agent_panel()`（Pane 挂钩归批③、协作面由 mdc-collab 承担——Lead #105 裁决说明）；`icon`/`toggle_action` 由宿主 activity_bar 既有按钮承载，不入 trait |
| `PanelHandle` | dock.rs:106-281 | `handle.rs` | 对象安全镜像 + `impl PanelHandle for Entity<T: Panel>` blanket（蓝本 :154 模式）；to_any → `AnyView::from`（闭包 gpui view.rs:30 同源） |
| `DockPosition` | dock.rs:322-374 | `dock.rs` | Left/Bottom/Right + `axis()`；**delta**：settings `From`/`TerminalDockPosition` 换位面归后续批（设置驱动的位置迁移） |
| `Dock` struct | dock.rs:283-294 | `dock.rs` | position/entries/is_open/active/focus 一致；**解绑③**：`workspace: WeakEntity<Workspace>`/`modal_layer`/`zoom_layer_open` 不入本批（持久化钩子批②补、zoom 后续批） |
| `DockRestoreState` | dock.rs:296-314 | `dock.rs` | **泛化**为 `pending_activation: Option<&'static str>`（预物化激活意图：面板未注册时记 key、物化即激活、用户实际换面丢弃——蓝本 restore-pending 同语义；批②布局恢复复用同一字段） |
| `first_enabled_panel_idx` | dock.rs:550-560 | `dock.rs` | 返回 `Option`（蓝本 anyhow Result——本批宿主自缺省，delta） |
| `panel_index_for_*` | dock.rs:519-548 | `dock.rs` | proto_id 查位 → PanelId 查位（解绑①对齐）；persistent_name/type 双查合一便捷面 |
| `set_open` | dock.rs:576-592 | `dock.rs` | 一致：变更前提 + 激活面板随开合收 set_active + 去重 |
| `activate_panel` | dock.rs:936-961 | `dock.rs` | 一致：同位去重（蓝本 :949）+ discard pending（:938）+ 先去激后激 + notify |
| `remove_panel` | dock.rs:893-924 | `dock.rs` | 一致：激活位命中清空 + dock 收起（:911）、前移位收缩、后位无关 |
| `PanelEntry` | dock.rs:383-387 | `dock.rs` | panel + subscriptions 随条目存续 |
| `add_panel`（订阅/入列） | zed workspace crate（AddPanel 群） | `dock.rs` | 事件订阅（Activate→激活；Close→closable 门移除）+ pending 兑现 + 注册不自动激活（宿主显式） |
| `PanelRegistry` | Q-IDE `app/dock.rs:95-121`；gpui-base `registry.rs:59` | `registry.rs` | persistent_name → 构造器全局注册表（批② 布局恢复消费面） |
| `ActiveTracker` | gpui-base `active.rs:20` 设计 | `dock.rs` | 去重/去抖纯状态机（同目标零通知；去激-激成对产出） |
| `DockAreaState`/版本迁移 | Q-IDE `layout.rs:20-22/:525-662/:518-521` | `state.rs` | #120 批②：serde 结构化快照（字段名冻结）+ `DOCK_LAYOUT_VERSION` + 不符整份丢弃；per-panel 尺寸参照 zed `dock.rs:998-1160`（单 dock 绝对像素宽——无分树份额缩放，delta） |
| `dump`/`restore` | zed 序列化钩子 + Q-IDE `save_layout`/`restore_session` | `dock.rs` | #120 批②：快照/恢复协议；**预物化尺寸意图**（restore 时未注册面板记 pending，物化即兑现——与批① pending_activation 对称，重启后打开文件夹才建树的真实流） |

## 三、纪律说明

- **非零修改口径**：蓝本是 zed workspace crate 的深缠模块（`client::proto`/
  `db::kvp`/`Workspace`/`ModalLayer` 纠缠——#106 报告 §5 D1-B 12 处清单），
  物理零修改不可行；Lead 裁决的蓝本口径 = 本文件 §二 对照表 + 蓝本锚点
  内嵌于各源文件文档注释。
- 蓝本 `set_position`/`position` 携带 `window`/settings 参——本批静态位置
  签名（delta），设置驱动换位（蓝本 dock.rs:329-357）归后续批次。
- 蓝本 `ActivationFocusHandle` 语义（面板子树归属焦点）原样保留于 trait
  文档（panel.rs）；mang-shell 面板本批用树根/面板根句柄（file_tree 的
  tree_focus / git_panel 的 panel_focus）。
