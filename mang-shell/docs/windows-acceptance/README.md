# Windows 窗口统一验收清单（工作线 A 阶段 A5 交付物 → #48 真机走查用）

> 来源：#47（工作线 A）阶段 A5 要求「向 #48 提供统一窗口验收清单和可启动程序，
> 不用只断言 label 的测试代替操作」。验收人：具备 Windows GUI 环境的一方
> （Lead / #48 认领人）。
>
> 口径（#48 铁律）：不得用 mock、非空位图或跳过测试替代窗口验收；每项记录
> 通过 / 失败 / 未测并附截图或日志；真实缺陷单独建可领取 issue，不在本文件
> 里堆积。Linux 侧（无 root 云沙箱）已先行验证程序可启动、四格式文档可渲染
> （§六 证据）；标注「Windows 待验」的项必须真机走查后才可在 #48 记通过。

## 一、验收对象与启动方式

| 环境 | 命令 | 判定 |
|---|---|---|
| Windows（真机） | `cargo run --manifest-path mang-shell/Cargo.toml -- <文件>` | 人工走查 + 截图 |
| Linux 无人值守 | `./smoke.sh 5 tests/fixtures/smoke.md` | stdout 末行 `smoke: quit reached` |
| Linux 截图取证 | `./screenshot.sh <out.png> 6 <文件>` | 1600×1000 PNG 落盘 |

夹具（`tests/fixtures/`）：`smoke.md` / `smoke.pdf` / `smoke.xlsx` /
`smoke.docx` / `smoke.canvas` / `mouse_smoke.md` / `thumbs_100p.pdf`。
安装包与文件关联面归旧 #270 范围，不在本清单。

## 二、A. 窗口与布局（阶段 A1）

| # | 检查项 | 操作 | 期望 | 自动化背书（Linux） |
|---|---|---|---|---|
| A-1 | 标题栏 | 观察窗口顶部 | 自绘标题栏在顶、可拖拽移动窗口 | 手工 |
| A-2 | 菜单栏贴左 | 观察菜单栏 | 紧贴标题栏下方左侧（旧 #297 已修——回归确认） | `workspace_layout_tests::git_and_menu_controls_keep_visible_bounds_with_document_open` |
| A-3 | 活动条 | 左缘图标条 | 36×36 按钮 / 间距 4 / 边距 8（实宽 52px）；hover / 选中态 | 同上（`act-git` 精确 36×36 断言 + 点击切视图） |
| A-4 | 文件树 | 左栏 | 260px 可折叠；根行下为文件；上方无横贯窗口的文件条 | `tabs_start_at_editor_column_and_never_above_explorer` |
| A-5 | 标签条 | 编辑区上沿 | 标签只在编辑区上沿（起 x 与编辑区同列） | 同上（1280 / 900 两宽断言） |
| A-6 | 右 dock | 右缘 | 导航与文档信息分离生命周期；关信息不关导航 | `right_dock_navigation_and_document_information_have_separate_lifetimes` |
| A-7 | 状态栏 | 底部 | 真实文案：文件名 + 修改标记 + 行列字数 + 工作区 + 选区 + git 分支 + 编码 + 换行 + 页码 | `ui_wiring_faces::face5_status_bar_binds_data_sources` |
| A-8 | 窗口宽高变化 | 拖拽边缘 1280→760（最小窗宽）、最大化 / 还原 | 布局不错位、编辑区保 ≥240px 可用宽（**右 dock 在窗宽 <900px 时渲染期让位，不改写用户开关态，宽度恢复后回归**） | 1280/900/760×480 已自动化（`right_dock_yields_below_critical_width_and_returns_when_restored` / `layout_holds_at_enforced_minimum_window_size`）；最大化 / 还原手工（Windows 待验） |
| A-9 | 高 DPI | 150% / 200% 显示缩放 | 布局不错位（全壳物理 px 自绘，源内零 rem 混用） | 无——#48 真机专有 |

## 三、B. 文档生命周期（阶段 A2）

| # | 检查项 | 操作 | 期望 | 自动化背书（Linux） |
|---|---|---|---|---|
| B-1 | md 全环 | 打开→编辑→保存→关闭→重开 | 内容往返一致 | `--bin` 面多测试 + `docx_edit_roundtrip` 同构管线 |
| B-2 | 打开入口 | 「打开文件」按钮 / 菜单 / 命令面板三入口 | 文件选择器弹出→打开成功 | 手工（Windows 待验） |
| B-3 | 关闭取消 | 改脏→点标签关闭→「取消」 | 脏标签保留 | `tab_close_button_closes_document_and_honors_dirty_cancel` |
| B-4 | 关闭三分支 | 改脏→关闭→取消 / 保存 / 放弃 | 三分支各自正确 | `dirty_close_confirm_cancel_save_discard` |
| B-5 | 保存失败 | 只读文档改脏→保存 | 示警不静默、拒关标签 | `editor_save_outcome` 三态 + `xlsx_save_failure_keeps_dirty_and_blocks_close` |
| B-6 | 退出协调 | 多脏标签（md / pdf / xlsx）→ 关窗 | 逐标签确认；批注脏计入 | `pdf_annotate_dirty_blocks_quit_confirm`（Linux 真跑）/ `xlsx_dirty_quit_confirm_blocks_and_saves_all` / `xlsx_quit_write_failure_keeps_retry_confirmation` |
| B-7 | pdf 批注 | 打开→批注→保存→重开 | 批注往返（页号不变） | `pdf_annotate_e2e` 10 项 + `pdf_gesture_dirty_close_confirm_save_persists` |
| B-8 | xlsx 网格 | 网格编辑→Ctrl+S→关闭→重开 | 四路统一分流不丢数据 | `xlsx_close_confirm_save_persists_cancel_keeps_dirty` + `value_fix_io_tests` |
| B-9 | docx 编辑 | docx 改→保存→重开 | 走 `save_docx` 专用通道（非行编辑面） | `docx_edit_roundtrip` 7 项 |
| B-10 | 多标签 | 开 3 标签轮换 / 同文件重开 | 状态保持 / 去重 | `three_tabs_switch_preserves_state` / `same_file_reopen_dedupes_to_existing_tab` |

## 四、C. 工作区/文件树/Git（A3）+ D. 菜单/命令/输入/焦点（A4）

| # | 检查项 | 期望 | 自动化背书（Linux） |
|---|---|---|---|
| C-1 | 打开文件夹 / 刷新 / 键盘按路径选中 | 选中稳定、子目录仓库根统一 | `git_diff_real_repo` 24 项（真实临时仓库） |
| C-2 | 差异 / 历史定位 | 定位准确 | 同上 |
| C-3 | 文件树点击开文档 | 真实 shell 无实体重入 | `file_tree_open_and_reopen_use_real_shell_without_entity_reentry` |
| D-1 | 菜单开 / 执行 / 关闭（键盘 + 点击） | 键盘导航、点外关闭 | `face1_menu_items_reach_command_dispatch` |
| D-2 | 命令面板 / 设置 / 查找框 | 打开、执行、Esc 恢复焦点 | `face2_shell_shortcuts_reach_commands` / `face6_settings_changes_take_effect` |
| D-3 | 面板切换 | 状态联动 | `face3_panel_toggle_changes_state` |
| D-4 | 中文 IME / 选区 / 粘贴 / 快捷键冲突 | 不互相吞 | **无自动化——Windows 真机专有（#48 核心）** |

## 五、E. 主题/缩放/一致性（A5）+ 按钮六问（#48 原始用户反馈）

| # | 检查项 | 期望 |
|---|---|---|
| E-1 | 明 / 暗主题两态 | 侧栏无透明按钮；主题色 / 尺寸 / 圆角 / 焦点环统一。**Linux 侧自动化背书（#47 批次 4）**：`theme_visual_smoke.sh` → `theme_visual_check.py` 11 项像素断言（双态底色分向 / 切换真实生效 / 侧栏随主题 / 活动条墨迹可见性），证据 PNG `theme-light.png` / `theme-dark.png`；「无透明按钮」人工观感与四态统一仍属真机验收 |
| E-2 | hover / 选中 / 焦点 / 禁用态 | 四态可见且一致。**Linux 侧自动化背书（#47 批次 5+6，hover/选中/禁用三面）**：`--smoke-hover-state` 真实鼠标事件管线三帧断言（hover 底色 light Δ-14.2/dark Δ+24.5、选中图标蓝度 +19.5、文件树取消选中 -10.5、视图切换 3.6% 变化像素）+ `--smoke-disabled-state` 双面断言（**产品行为：git 面板「刷新」钮在不可用态改禁用渲染**——无 hover/无监听/Color::Disabled；禁用面悬停+点击零响应逐字节验证，对照面 hover/点击生效+`git: ` 证据行增量），证据 `hover-light/dark.png`、`ds-disabled/control.png`；**焦点环暂无确定性夹具**（渲染依赖键盘 focus_visible 语义）——待后续功能批补夹具，完整四态人工观感归 #48 真机 |
| 按钮六问 | （用户原话逐条） | ① 各面板按钮点击有响应（含 hover）②「打开文件」按钮→选择器→成功 ③ 侧栏无透明按钮（明暗两态）④ 菜单栏贴左与 zed/vscode 同款位 ⑤ 左右栏布局合理 ⑥ 迁移组件真机行为：右键菜单 / 菜单栏 / 命令面板 / Ctrl+G 跳行 / tab 条 / settings 模态 / panels·file_tree·git_panel 列表。**⑥ 菜单栏 GUI 级背书（#47 批次 7）**：`--smoke-menu` 点击「文件」开下拉（B 帧下拉区 54.45% 像素变化——浮层+菜单项真实渲染）→ 再点击 toggle 关（C 帧与 A 帧逐字节相等 + `menu: open/close` 双证据行），证据 `menu-light.png`；Esc 关闭链与菜单项执行派发归头单测（menu_bar_lifecycle_tests / ui_wiring_faces）。**⑥ 右键菜单 GUI 级背书（#47 批次 8）**：`--smoke-context-menu` 右键编辑区开浮层（B 帧菜单区 7.26% 像素变化 + `context-menu: open` 证据行）→ 点外关（C 帧变化 0.00%——插入符容差内），证据 `cm-light.png`；菜单项执行派发归 ui_wiring_faces。**⑥ 命令面板 GUI 级背书（#47 批次 9）**：`--smoke-palette` Ctrl+Shift+P 完整键派发链开面板（面板区 8.48% 像素变化——覆盖盒+段标题/条目真实渲染）→ Esc 关净（0.00%，内联键分支无焦点注入依赖），证据 `pal-light.png` |

## 六、Linux 侧启动证据（2026-11-10，本批实测）

- 基线：main `2e9d233`；工具链 rust 1.99.0（rust-toolchain.toml 钉住）。
- 主题两态视觉面（#47 批次 4，E-1）：`./theme_visual_smoke.sh`（仓库根级）
  Xvfb 1600×1000 + llvmpipe 双态截帧 → 11 项断言全过
  （实测：light 全帧 244.6 / dark 37.4 / MAD 132.8 / 活动条墨迹 1.05%・0.95%），
  证据 `theme-light.png` / `theme-dark.png`（本目录）。
- 门禁（全部本地实跑）：`cargo check --all-targets` 0 error；
  `cargo test --lib` 582 passed / 0 failed / 5 ignored（ignored = 夹具生成器 + 1 性能面）；
  `cargo test --bin mang-shell` 679 passed / 0 failed；
  集成面 `git_diff_real_repo` 24 / `ui_wiring_faces` 7 / `command_coverage` 6 /
  `docx_edit_roundtrip` 7 / `pdf_annotate_e2e` 10 / `module_wiring` 9 /
  `wiring_integrity` 10——全绿。
- libpdfium 在场：依赖 pdfium 的退出面 e2e 真跑通过（无 SKIP 横幅）。
- GUI 启动：Xvfb + llvmpipe（LLVM 19.1.7）Vulkan 软渲染；`smoke.sh` 判定行
  `smoke: quit reached` 达成；RPC 事件泵启动、recent 列表装载正常。
- 截图（1600×1000，xwd→netpbm 链，像素统计验证非空白）：
  `linux-md.png` / `linux-pdf.png` / `linux-xlsx.png` / `linux-docx.png`。
- 环境：无 root Debian 云沙箱；系统依赖经用户态 sysroot
  （`smoke.sh` 的 `MANG_SYSROOT` 机制）补齐 xkbcommon / lavapipe / 截图链。

## 七、Windows 未测项（诚实登记，待 #48 真机）

IME 输入面（中文输入 / 光标选区 / 粘贴 / 快捷键冲突）、高 DPI（150% / 200%）、
真机窗口装饰与边缘拖拽缩放、明暗主题实机观感、安装包链（旧 #270 范围）。
以上在本 Linux 侧无对应证据，不得记为通过。
