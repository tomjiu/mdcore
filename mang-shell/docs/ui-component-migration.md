# #385 阶段 1：UI 组件库迁移盘点与决策——对照表 + 裁定建议

> issue #385（P0：GPUI UI 组件库迁移）阶段 1 交付物。
> 基线：main `70ab769`（vendored 快照 commit `1663af41`，PROVENANCE.md 在档）。
> 姊妹篇：`docs/ui-zed-parity-289.md`（#289 六缺陷逐项对照——本文不重复
> 其内容，聚焦**组件级**迁移路径）。

## 一、结论速览

1. **mang-shell 已在 ui crate 上，但只用了「底层原语层」**——`Label` /
   `LabelSize` / `Color` / `IconName` / `IconButton`（全仓 208 处引用）。
   迁移的真实任务是「原语层 → 交互组件层」（菜单族 / Modal / List /
   TabBar），不是从零开始。
2. **手写按钮族是文档化的刻意自绘，不是无主技术债**（本盘点的重要
   修正——初稿曾把它们列为批 1 迁移目标）：`activity_bar.rs` 模块文档
   明确记载自绘依据（IconButton 尺寸由 IconSize 的 rem 机器推导，
   **无法钉定 Lead 硬规格 36×36**——ButtonSize 仅 Large=32px 档，
   无任意固定尺寸 API，icon_button.rs:218/button_like.rs:455 实证）；
   `settings_panel.rs` 的 choice_button/step_button 注明「bar_button
   同款视觉语系」——迁移会破坏规格与视觉一致性。**按钮族维持自绘**，
   迁移重心转向菜单族/Modal/TabBar/List。
3. **闭包内没有文本输入组件**——issue 建议的「find_bar.rs → ui::InputField」
   映射**不成立**：zed 的输入面在其 `editor` crate（闭包外），ui crate
   及全部 34 个闭包 crate 内零 TextInput 实现（rg 实证）。文本输入的
   迁移是「闭包扩张」级决策，需 Lead 单独裁定（见 §五）。
4. **AI 五组件删除建议与搬运闭包纪律冲突**——`third_party/zed` 是
   「源码零修改口径」的在档搬运（PROVENANCE.md）。删除 = 破坏升级
   可比性。三条处置路线供 Lead 裁定（见 §四，本文推荐 C：暂不删，
   走不适用清单隔离）。
5. **可立即迁移的高价值面**：`context_menu`（443+146 行，自研
   Point/Rect/height_units 几何与 ui::ContextMenu 职责重叠——真去重）
   与 `tab_bar` / `settings_modal` 外壳——目标 API 在闭包内在场。

## 二、zed ui crate 组件面盘点（闭包内在场 = 43 组件文件）

### B1 交互组件（迁移目标——mang 可消费）

| zed 组件 | API 入口（闭包内实证） | mang 适用面 |
|---|---|---|
| `Button`（button/button.rs） | `Button::new(id, label)` + `ButtonStyle::{Filled, Tinted, Subtle, Ghost, Danger}` + `ButtonSize` | 全壳按钮位 |
| `IconButton`（button/icon_button.rs） | `IconButton::new(id, IconName)` | activity_bar / right_dock / 工具条 |
| `ToggleButton` / `SplitButton` / `ButtonLink` / `CopyButton`（button/） | 同族 builder | 按需 |
| `ContextMenu`（context_menu.rs） | `ContextMenuEntry::new(label)` + `ContextMenu::new(side, render)` | 右键菜单（现手写 443+146 行） |
| `DropdownMenu`（dropdown_menu.rs） | `DropdownMenu::new(...)` | 表单下拉（xlsx 标注模式选择器同款形态） |
| `PopoverMenu` / `RightClickMenu`（popover_menu.rs / right_click_menu.rs） | 各自 builder | 菜单弹出位 |
| `Modal`（modal.rs） | `Modal::new(id, scroll_handle)` + `ModalHeader` / `ModalRow` | settings_modal（现手写模态） |
| `List`（list/） | `List::new()` + `ListItem` / `ListHeader` / `ListSeparator` | 大纲 / 属性面板列表 |
| `DataTable`（data_table/） | DataTable 族 | xlsx 网格远期（当前自研虚拟化面更贴） |
| `Tab` / `TabBar`（tab.rs / tab_bar.rs） | `TabBar::new(id)` | tab_bar.rs（现手写 218 行） |
| `Tooltip`（tooltip.rs） | Tooltip builder | 全壳 title 提示的统一化 |
| `Notification` / `Banner` / `Callout` | 各自 builder | 错误横幅 / 引导弹窗 |
| `Indicator` / `Progress` | 各自 builder | 后台任务态 |
| `Chip` / `ChoiceCard` / `Disclosure` / `Divider` / `CountBadge` | 各自 builder | 面板细节件 |

### B2 展示/布局件（低风险补充消费）

`Icon`（icons crate 枚举单源）/ `Label`（已在用）/ `Stack` / `Group` /
`Image` / `Scrollbar` / `KeybindingHint`（快捷键展示——菜单/速查表可消费）/
`Navigable`（列表键盘导航语义）/ `TreeViewItem`（文件树行远期）/
`IndentGuides`（编辑器缩进线）。

### B3 zed 专属件（mang 不适用——不迁）

`project_empty_state`（zed 空态）/ `diff_stat`（git 行数条——git_panel
远期可选）/ `redistributable_columns` / `facepile`+`avatar`+`collab/`
（协作面——mang 无协作）/ `ai/`（五文件，见 §四）/ `gradient_fade` /
`sticky_items`（长列表头粘滞——xlsx 网格远期可参考实现）。

### B4 缺件（闭包内无 → 迁移映射不成立）

| 需求 | issue 原建议 | 实证 | 影响 |
|---|---|---|---|
| 文本输入框 | find_bar/settings → `ui::InputField` | **闭包 34 crate 内无 TextInput/InputField**（zed 输入面在其 editor crate——闭包外） | find_bar（1218 行）/settings 输入位**维持手写**，或 Lead 裁定扩张闭包搬 editor crate（依赖面大：rope/sum_tree/languages… 在场，但 editor 本体 + 其 UI 依赖链未在场） |
| 富菜单栏（顶部） | menu_bar → `ui::Menu` | 闭包内 `menu` crate 仅是 **action 定义**（Cancel/Confirm/导航 actions），非菜单 widget；widget = ui::ContextMenu 族 | menu_bar.rs（1318 行手写）迁移目标 = ContextMenu/DropdownMenu 组合——可行但需重构键盘导航层 |

## 三、mang-shell 手写面 → 迁移对照表（阶段 2 分批依据）

> 「手写」判定口径：该文件的交互件由裸 `div().on_click(...)` 构建
> （非 ui 组件 builder）。难度：低 = 纯替换；中 = 需状态桥接；
> 高 = 语义重构。

| # | mang-shell 现状（行数） | zed ui 对应 | 难度 | 优先级 | 批次建议 | 备注 |
|---|---|---|---|---|---|---|
| 1 | `title_bar.rs`（123）窗口控制 | **已迁移**（`ui::IconButton` 在用——#289 已做） | — | — | — | 剩余：菜单钮位视觉打磨 |
| 2 | 各面板按钮（panels/git_panel/settings_panel/pdf 工具条的 `div().on_click`） | **维持自绘**（见结论 2：视觉语系一致性 + 精确几何控制；IconButton 无固定尺寸 API） | — | — | 不迁 | 零件级 ui::Tooltip 已在用；~~后续若 Lead 弃 36×36 规格再评估~~ **#525 裁定（2026-11-08）：36×36 不需弃——适配层成立**（`src/button_ext.rs` IconButton 渲染拦截路径：width 走 FixedWidth + render() 返回公开 ButtonLike 补 height——无头实测精确 36×36 全盒命中，外层容器方案死区证伪）；替换批次待 Lead 排期 |
| 3 | `activity_bar.rs`（139）视图切换钮 | **维持自绘**（模块文档在案：IconButton rem 推导无法钉 36×36 Lead 硬规格） | — | — | 不迁 | 自绘单元同时掌控 hover/selected 主题色 |
| 4 | `right_dock.rs`（52 恒在 dock 条） | **维持自绘**（复用 bar_button 单元——左右两条同语系） | — | — | 不迁 | 同上 |
| 5 | `tab_bar.rs`（218 手写 tab） | `ui::TabBar` + `ui::Tab` | 中 | P1 | **批 2** | 溢出横滚已用 gpui 原生；Tab 组件带关闭钮/激活态语义 |
| 6 | `context_menu.rs`（443）+ `context_menu_view.rs`（146） | `ui::ContextMenu` + `ContextMenuEntry` | 中 | **P0** | **批 1（修正后首批）** | 手写含锚点翻转/键盘导航——ContextMenu 自带；`Point/Rect/height_units` 自研几何可退役（真去重面） |
| 7 | `menu_bar.rs`（1318）+ `menu_bar_view.rs`（344） | `ui::DropdownMenu`/`PopoverMenu` 组合 | **高** | P1 | 批 4+ | ✅ 批 4 已交付（feat/385-b4-menu-bar）：下拉/子菜单/键盘导航/点外关闭 → `ui::ContextMenu` 实体（批 1 同构桥接：custom_entry/toggleable_entry/.submenu + anchored/deferred 部署）；自研 MenuBarState 状态机与 place_menu 几何消费面退役；标题行自绘保留（结论 2 语系）。已知降级：←→ 切换顶部菜单 → 组件语义（← 关/→ 进子菜单） | 
| 8 | `find_bar.rs`（1218，editor/） | **无对应件**（B4 缺件） | 高 | P2 | 待裁定 | 维持手写或闭包扩张（§五） |
| 9 | `settings_panel.rs`（378 模态弹窗） | `ui::Modal` + `ModalHeader`/`ModalRow` | 中 | P1 | 批 3 | #289 已改模态语义——外壳换 Modal 组件 |
| 10 | `palette_view.rs`（104 命令面板） | `ui::Popover`/`Modal` 组合 | 中 | P2 | 批 3+ | |
| 11 | `file_tree.rs`（659） | `ui::List` + `TreeViewItem` | 高 | P2 | 批 5 | 自研虚拟化/多选/右键——迁移面大。**批 5b 实落**：行渲染 → `ListItem`（TreeViewItem 实证不适用——单级缩进/无图标位）；uniform_list 虚拟化保留（ui::List 非虚拟化容器）——偏离全文见下方批次登记 |
| 12 | `panels.rs`（528 大纲/属性） | `ui::List` + `ListItem` | 中 | P2 | 批 5 | |
| 13 | `git_panel.rs`（560） | `ui::List` + `Chip`/`CountBadge` | 中 | P3 | 批 5+ | |
| 14 | `status_bar.rs`（235 标签条） | `ui::Label` 已在用 | — | — | — | 标签面无需组件化 |
| 15 | `pdf_pages.rs`/`xlsx_grid_view.rs`（自研虚拟化视图） | 无直接对应（DataTable 远期） | — | P3 | 远期 | 自研面更贴合（#289/#389 已建回归门） |
| 16 | `assets.rs` 图标（手工表 + SHELL_ICON_SET 回归门） | icons crate 枚举单源（已在用） | 低 | P1 | **批 2 并行** | #289 已补 4 枚 + 回归门；剩余风险 = 手工表漂移（门已锁）——维持现状可接受 |

### 分批 PR 顺序（阶段 2 执行版——每批一 PR + 截图对比 + 交互测试）

> 修正记录：初稿曾把「面板/图标条按钮 → ui::Button/IconButton」列为
> 批 1（低风险样板）——实最核实后撤回：按钮族是文档化刻意自绘
> （结论 2：36×36 硬规格 + 视觉语系一致性，IconButton 无固定尺寸
> API）。批序重排如下。

```
批 1（真去重样板）：context_menu → ui::ContextMenu（自研 Point/Rect/height_units
                     几何退役——443+146 行与 ui 组件职责重叠的真去重面）
                     ✅ 批 1×4 收口已交付：几何层整体退役（place_menu/
                     place_submenu/menu_size/estimate_width/height_units +
                     Point/Rect 自研结构——批 4 后仅剩测试自引用的死代码，
                     PR #519 预登记的合并后收口）；`MenuOpener`/open_* 签名
                     换 `gpui::Point<Pixels>` 直传（event.position 原样，
                     消 f32 往返）；补 context_menu 生命周期交互测试
                     （TestAppContext 无头面——阶段 3 质量门 1 同款，
                     menu_bar_lifecycle_tests 模式）。
批 2：tab_bar → ui::TabBar/Tab
批 3：settings_panel 外壳 → ui::Modal；palette → Popover 组合
批 4：menu_bar → DropdownMenu/PopoverMenu 组合（键盘导航重构——最重）
                     ✅ 批 4 已交付（feat/385-b4-menu-bar）：下拉 → `ui::ContextMenu` 实体
                     （组件自带键盘导航/一层子菜单/点外关闭；自研状态机与几何退役）；
                     标题 trigger 自绘保留（按钮族语系）；←→ 切顶部菜单降级为组件
                     语义（← 关/→ 进子菜单），PR 描述已登记
批 5：panels/file_tree/git_panel → ui::List 族（虚拟化/多选桥接）
                     ✅ 批 5a 已交付（PR #482 已合）：panels.rs 行渲染迁
                     List/ListHeader/ListItem（hover/选中/缩进语义统一）；
                     键盘导航（Navigable）坦白未接——panels 无状态渲染架构
                     需先立焦点/选中状态面，挂后续批次。
                     ✅ 批 7 已交付（#571，feat/571-outline-keyboard-nav）：
                     键盘导航挂账兑现——heading_tree 内联渲染退役，迁
                     OutlinePanel 实体（行焦点句柄按行号差量复用）+
                     ui::Navigable 接线（↑↓ 焦点行移动 / Enter 激活 =
                     跳行 + #289-④ 页滚 / Esc 焦点还编辑区）；md 大纲
                     补齐（md_outline 首次入面板——主格式此前无结构树）。
                     ✅ 批 5b 已合并（PR #498）：file_tree 行渲染迁 ListItem
                     （indent_level×14px 深度缩进 + start_slot 折叠箭头 +
                     on_click/on_secondary_mouse_down 左右键 + toggle_state
                     活动高亮 + aria_role(TreeItem)）。两处实最偏离（对行
                     11 原映射）：① TreeViewItem 不适用（单级缩进线/无图标
                     位/28px 行高）；② uniform_list 虚拟化保留（ui::List 非
                     虚拟化容器——「1000+ 文件只布局可视行」不变量不动）。
                     锚点同步：wiring_integrity `file_tree_row_clicks_wired`
                     + ui_wiring_faces `face4`（MouseButton 字面量迁组件内，
                     锚点改钉 .on_click/.on_secondary_mouse_down）。
                     ✅ 批 5c 已合并（PR #483）：git_panel 改动/历史行迁
                     ListItem + 状态码 Chip + 计数 CountBadge + 空态
                     List::empty_message；视图切换 tab 钮不在行 13 映射内未动。
                     批 5 三件套仅剩 5a（本 PR）待合并。批 6+：Tooltip 统一化 / Chip / CountBadge 等细节件按需
每批验收：既有单测全绿 + 新增交互测试（不是只测函数返回值）+ 截图对比
（迁移前/后——沙箱无 GUI 时登记待真机，AGENTS 诚实边界纪律）
不迁清单：按钮族（bar_button/choice_button/step_button 语系）、
          find_bar（缺件——§五）、pdf_pages/xlsx_grid 自研虚拟化视图
```

## 四、AI 五组件处置建议（Lead 裁定点）

**实证**（本盘点）：`components/ai/` 五文件（agent_setup_button /
ai_setting_item / configured_api_card / delta_illustration / thread_item）
依赖面 = `gpui` + `std` + `itertools`（itertools 已是 ui crate 直接依赖，
Cargo.toml:20 在档）——**零 AI 外部依赖**，issue 的「删了不影响编译」
判断成立。

**但**：`third_party/zed` 是「源码零修改口径」在档搬运闭包
（mang-shell/Cargo.toml 头注释 + PROVENANCE.md）。删除 = 修改搬运体，
破坏与上游 commit `1663af41` 的可比性（未来升级 = 整闭包重同步时，
本地删除会丢失/冲突）。

| 路线 | 做法 | 得失 |
|---|---|---|
| A. 删除（issue 建议） | 删 `components/ai/` + `ai.rs` 的 mod/pub use 两行 | ui crate 干净（死代码 -5 文件）；**破坏零修改口径**——PROVENANCE 需登记偏差，升级重同步成本 +1 |
| B. 保留 | 现状不动 | 零纪律成本；死代码留在闭包（不进 mang 二进制的活跃路径——编译进 rlib 但链接器可弃） |
| **C. 保留 + 隔离声明（本文推荐）** | 保留文件；在 `mang-shell/docs/ui-component-migration.md`（本文）登记「不适用清单」（§B3 已列）——后续任何 `use ui::ai_*` 误引入在 review 面可见 | 零口径破坏 + 误用防线（文档层）；死代码维护面 ≈ 0（闭包随上游升级整体换代） |

**推荐 C 的理由**：AI 组件的死代码成本几乎为零（编译器层面的未用
pub 项不进最终二进制活跃路径），而口径破坏的升级成本是长期的。若
Lead 坚持 A，需同步修改 PROVENANCE.md 登记偏差行（升级时重放）。

## 五、文本输入缺件的处置建议（Lead 裁定点）

find_bar（1218 行手写输入面）/ settings 数字输入 / xlsx 批注评语输入
（未来）均无闭包内迁移目标。两条路线：

| 路线 | 做法 | 得失 |
|---|---|---|
| **维持手写（本文推荐·近期）** | find_bar 现状已有 IME/选区/历史回归门（find_history_tests 等在案） | 零风险；但「手写输入件」的交互 bug 面继续自扛（用户报的「点击不好使」类问题一部分来自此） |
| 闭包扩张（远期裁定） | 从上游同 commit 搬 zed `editor` crate（输入面真源） | 闭包 +1 大 crate（其依赖 rope/sum_tree/text 已在场，但 editor 本体依赖链需全量核）；升级面 +1；**输入面质量与 zed 同源** |

建议：阶段 2 六批迁移完成后再评估扩张（那时手写输入件的 bug 面数据
更充分）。

## 六、阶段 3 质量门（登记——阶段 2 各批逐步落）

1. **交互测试门**：每批迁移必须带「点击 → 状态变化」级测试（非仅
   函数返回值断言）——GPUI 无头面可测状态桥（`TestAppContext` 已在
   editor/mod.rs 用例在案），真机手势待开发机；
   落地登记：
   - menu_bar（批 4 交付时带）：`menu_bar_lifecycle_tests`——开/切换/
     关/可重开 + Cancel → DismissEvent → 清态链；
   - context_menu（批 1×4 收口带）：`context_menu_lifecycle_tests`——
     四态开合 + 切换重建 + Cancel 链 + 执行动作不悬挂；
   - palette/goto-box（阶段 3 批 2）：`palette_goto_interaction_tests`
     ——Ctrl+Shift+P/Ctrl+G 真实键位入口 + 键入过滤 + ↑↓ 选中 +
     Enter 执行闭环（OpenFind → 查找条开 + #183 A-3 最近使用记账
     置顶）+ goto 跳转/clamp/非法不跳不关/Esc；
   - tab 族：`tab_workflow_tests`（#219 在案，5 例状态桥——先于
     迁移存在，批 2 迁移后回归通过）；
2. **截图回归门**：`ui_snapshot.rs`（迁移前基线截图进仓 + 对比）——
   沙箱无 GUI，登记为真机/CI 侧任务；
3. **SHELL_ICON_SET 门**（已在案）与组件迁移并行不悖。

## 七、本盘点的不做清单

- 不动任何产线代码（阶段 1 = 纯盘点文档交付）；
- 不删除 third_party 任何文件（§四路线 C 推荐——Lead 裁定后另 PR 执行）；
- 不启动阶段 2 任何批次（每批独立认领/独立 PR）。
