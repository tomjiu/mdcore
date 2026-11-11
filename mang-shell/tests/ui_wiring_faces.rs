//! #168 / #196 收尾：界面层接线完整性——六面锚点（一面一测）。
//!
//! ## 与既有测试的分工
//!
//! | 文件 | 覆盖面 |
//! |---|---|
//! | `wiring_integrity.rs`（代理 B） | 模块内交互细节（无死闭包/菜单动作对账/基线计数） |
//! | `module_wiring.rs` | 模块挂载 + 派发安全 + perf 采集点 |
//! | `command_coverage.rs` | Cmd 枚举↔ALL↔执行臂静态对账（编辑器级键位） |
//! | **本文件** | **#196 任务 2 的六条接线面**——每面一条**全链路**锚点（入口 → 处理函数 → 状态/落盘），断言跨文件：任何一环被摘除即红 |
//!
//! ## 六面（#196 原文口径）
//!
//! 1. 菜单栏项 → 命令分发
//! 2. 快捷键 → Cmd 映射（本文件钉 **shell 级**键位——编辑器级键臂已由
//!    command_coverage 钉住，两级互补）
//! 3. 面板切换 → 状态变更
//! 4. 文件树点击 → 打开文件（实装为**单击**打开——双击语义从未实装，
//!    锚点按实装口径钉，防「声称双击实际没接」）
//! 5. 状态栏 → 数据源
//! 6. 设置项 → 生效
//!
//! ## 铁律
//!
//! - 只读源码断言，不改产线代码（git diff 只含 tests/）；
//! - 失败信息指明「哪面哪环断了」；
//! - 每条都验证过红（删锚点即红——见 PR 描述的红绿证据）。

use std::fs;
use std::path::{Path, PathBuf};

/// 读 `mang-shell/src/<rel>` 源码。缺失即 FAIL（源码进仓，不是环境件）。
fn src(rel: &str) -> String {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(rel);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("源码文件缺失/不可读（{}）：{e}", path.display()))
}

/// 提取函数体（`fn <decl>` 到配对大括号——与 module_wiring 同款口径）。
fn fn_body(file: &str, decl: &str) -> String {
    let start = file
        .find(decl)
        .unwrap_or_else(|| panic!("函数缺失：{decl}（接线被拆除或重命名——请同步锚点）"));
    let body_start = file[start..].find('{').expect("函数签名后无函数体") + start;
    let mut depth = 0usize;
    for (i, b) in file[body_start..].bytes().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return file[body_start..body_start + i + 1].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("函数体未闭合：{decl}");
}

/// 多文件全链断言（锚点缺失即失败，消息带面名与断环位置）。
fn chain(face: &str, links: &[(&str, &str, &str)]) {
    for (file_rel, needle, what) in links {
        let content = src(file_rel);
        assert!(
            content.contains(needle),
            "【面：{face}】断链——{file_rel} 缺「{what}」（锚点 `{needle}`）"
        );
    }
}

// ---------------------------------------------------------------------------
// 面 1：菜单栏项 → 命令分发
// ---------------------------------------------------------------------------

/// 菜单项从数据定义到执行的完整链路：
/// `context_menu.rs`（MenuItem::action 数据面）→ `context_menu_view.rs`
/// （点击回调）→ `shell.rs activate_context_menu_item`（分派体，按
/// MenuOrigin 区分 Editor/FileTree 两源）。
#[test]
fn face1_menu_items_reach_command_dispatch() {
    let menu = src("context_menu.rs");
    assert!(
        menu.contains("pub fn editor_menu(") && menu.contains("MenuItem::action("),
        "【面 1】菜单数据面缺失：context_menu.rs 无 editor_menu/MenuItem::action——\
         菜单项定义被拆除"
    );
    chain(
        "菜单栏项 → 命令分发",
        &[
            (
                "context_menu_view.rs",
                "activate_context_menu_item",
                "菜单点击回调 → shell 分派",
            ),
            (
                "shell.rs",
                "fn activate_context_menu_item",
                "菜单动作分派处理函数",
            ),
            ("shell.rs", "MenuOrigin::Editor", "编辑器菜单源分派臂"),
            ("shell.rs", "MenuOrigin::FileTree", "文件树菜单源分派臂"),
        ],
    );
}

// ---------------------------------------------------------------------------
// 面 2：快捷键 → Cmd 映射（shell 级键位——编辑器级由 command_coverage 钉）
// ---------------------------------------------------------------------------

/// shell 级键钩子的六大命令臂：Ctrl+Tab 循环 / Ctrl+W 关标签 / Ctrl+O
/// 打开 / Ctrl+Shift+S 另存为 / Ctrl+=/−/0 字号 / Ctrl+T 主题。
/// （编辑器级键臂 ↔ Cmd 声称的对账在 command_coverage.rs
/// `default_keys_reach_dispatch_key`——两级互补。）
///
/// #217：键位条件改 keymap 表驱动（哪个键绑哪个动作由 keymap.rs 内嵌
/// 单测逐臂钉定 + harness `#[path]` 双环境执行）——本面证据升级为
/// **动作变体 → 处理调用** 的执行臂对账（表动作无执行体 = 按键无反应）。
#[test]
fn face2_shell_shortcuts_reach_commands() {
    let shell = src("shell.rs");
    // 键钩子区域：`on_key_down(cx.listener(…))` 起、下一锚点止（不取
    // 整个 render 体——format! 字符串里的花括号会干扰函数体提取）。
    let hook_start = shell
        .find("on_key_down(cx.listener")
        .expect("shell 级键钩子缺失（on_key_down listener）——全部 shell 级快捷键失灵");
    let hook_end = shell[hook_start..]
        .find("fn dir_for_new")
        .map(|i| hook_start + i)
        .unwrap_or(shell.len());
    let key_block = &shell[hook_start..hook_end];
    for (variant, call, what) in [
        ("ShellAction::NextTab", "cycle_tab", "Ctrl+Tab 循环标签"),
        (
            "ShellAction::CloseTab",
            "close_active_tab",
            "Ctrl+W 关闭标签",
        ),
        (
            "ShellAction::PromptOpenFile",
            "prompt_open_file",
            "Ctrl+O 打开文件",
        ),
        (
            "ShellAction::PromptSaveAs",
            "prompt_save_as",
            "Ctrl+Shift+S 另存为",
        ),
        (
            "ShellAction::FontBigger",
            "apply_font_size",
            "Ctrl+= 字号放大",
        ),
        ("ShellAction::CycleTheme", "cycle_theme", "Ctrl+T 主题循环"),
    ] {
        assert!(
            key_block.contains(variant) && key_block.contains(call),
            "【面 2：快捷键 → Cmd 映射】shell 级键臂断链——{what}：\
             动作臂 `{variant}` 或处理调用 `{call}(` 不在键钩子内"
        );
    }
}

// ---------------------------------------------------------------------------
// 面 3：面板切换 → 状态变更
// ---------------------------------------------------------------------------

/// 图标条钮（文件树/git/设置）+ 右缘 dock 条钮（属性/来源）→ shell
/// toggle 方法 → 状态字段翻转 → `cx.notify()`（状态变更必须触发重绘，
/// 否则「点了没反应」）。
///
/// #289-⑤：设置钮打开**弹窗**（`toggle_settings` → `settings_open` 翻转），
/// 右栏开关移至右缘 dock 条（`right_dock.rs` 的 `dock-toggle-right`）——
/// 左图标条只管左栏。
#[test]
fn face3_panel_toggle_changes_state() {
    let bar = src("activity_bar.rs");
    for (id, method) in [
        ("act-file-tree", "toggle_left"),
        ("act-git", "toggle_git"),
        ("act-settings", "toggle_settings"),
    ] {
        assert!(
            bar.contains(id) && bar.contains(&format!("this.{method}(cx)")),
            "【面 3：面板切换 → 状态变更】图标条断链——{id} 未接 this.{method}(cx)"
        );
    }
    // 右缘 dock 条（#289-⑤）：右栏开关归右缘。
    let dock = src("right_dock.rs");
    assert!(
        dock.contains("dock-toggle-right") && dock.contains("this.toggle_right(cx)"),
        "【面 3：面板切换 → 状态变更】右缘 dock 条断链——dock-toggle-right 未接 this.toggle_right(cx)"
    );
    // 左图标条不再承载右栏开关（#289-⑤ 语义收口）。
    assert!(
        !bar.contains("act-toggle-right"),
        "【面 3：面板切换 → 状态变更】act-toggle-right 不应在左图标条（#289-⑤：右栏开关归右缘 dock 条）"
    );
    // 状态变更体：字段翻转 + notify（渲染层可见的前提）。#119：
    // git_view 布尔退役——左栏视图切换的形态断言改为 dock 激活协议
    // 接线（activate_left_view 单点 + 预物化意图注释）。
    let shell = src("shell.rs");
    for (decl, flip, what) in [
        (
            "fn toggle_left(",
            "let next = !dock.is_open();",
            "左栏开合经 dock 开合协议（#120 批②）",
        ),
        (
            "fn toggle_git(",
            "activate_left_view(crate::git_panel::GIT_PANEL_KEY, cx)",
            "git 视图经 dock 激活协议（#119）",
        ),
        (
            "fn toggle_settings(",
            "settings_open = !self.settings_open",
            "设置弹窗开合字段翻转",
        ),
        (
            "fn toggle_right(",
            "right_open = !self.right_open",
            "右栏开合字段翻转",
        ),
    ] {
        let body = fn_body(&shell, decl);
        assert!(
            body.contains(flip) && body.contains("cx.notify()"),
            "【面 3：面板切换 → 状态变更】{decl} 状态变更断链——缺「{what}」\
             或缺 cx.notify()（状态变了但不重绘 = 用户看不见）"
        );
    }
}

// ---------------------------------------------------------------------------
// 面 4：文件树点击 → 打开文件（实装为单击打开）
// ---------------------------------------------------------------------------

/// 文件树行左键 → `open_document_tab`（shell 通道）→ shell 方法存在且
/// 被 F1（Ctrl+O 对话框回调）共用——「从树里打开文件」与「对话框打开」
/// 同一条链。
///
/// #385 批 5b 演进：行渲染迁 `ui::ListItem`，左键经组件 `on_click` 接
/// 线（MouseButton 枚举在组件内部——锚点改钉组件方法名）。
#[test]
fn face4_file_tree_click_opens_document() {
    // #575 锚点更新：钩子化后行点击链 = render_row 的 ListItem.on_click
    // → hooks.open_file（file_tree.rs 面）→ shell.rs file_tree_hooks 的
    // open_document_tab 调用（bin 面）——三段两文件锚定（原单文件
    // 锚点随 lib 化拆两处）。
    let row = fn_body(&src("file_tree.rs"), "fn render_row(");
    assert!(
        row.contains(".on_click(") && row.contains("hooks.open_file"),
        "【面 4：文件树点击 → 打开文件】行点击断链——render_row 缺 ListItem.on_click 或\
         open_file 钩子调用（实装口径：单击打开；#575 lib 化后经钩子）"
    );
    let hooks = fn_body(&src("shell.rs"), "fn file_tree_hooks(");
    assert!(
        hooks.contains("open_document_tab"),
        "【面 4：文件树点击 → 打开文件】bin 面断链——file_tree_hooks 的 open_file \
         闭包缺 open_document_tab 调用（#575 钩子注入点）"
    );
    chain(
        "文件树点击 → 打开文件",
        &[
            (
                "shell.rs",
                "pub fn open_document_tab(",
                "打开文件入标签的 shell 方法（文件树与 Ctrl+O 共用通道）",
            ),
            (
                "shell.rs",
                "fn open_document(",
                "对话框回调同通道（F1）——两条入口汇聚",
            ),
        ],
    );
}

// ---------------------------------------------------------------------------
// 面 5：状态栏 → 数据源
// ---------------------------------------------------------------------------

/// 状态栏四项活数据（选中字数 / git 分支 / 编码 / 换行符）全部经
/// `status_info` 纯函数计算 + shell 渲染状态栏（消费 Editor 现场态）。
///
/// #281（审计 R-e）：编码/EOL 与 git 分支改经 **mtime 失效缓存面**
/// 消费（`detect_encoding_eol_cached` / `git_branch_at_cached`——
/// 状态栏随 ShellApp 每次重渲被调，直连探测 = 每帧整文件读盘）；
/// 缓存面内部仍调底层纯探测函数（status_info 单测锁），数据链
/// 绑定语义不变。
#[test]
fn face5_status_bar_binds_data_sources() {
    let sb = src("status_bar.rs");
    for (call, what) in [
        ("status_info::selection_info", "选中字数（现场 selection）"),
        ("status_info::find_git_root", "git 仓库探测"),
        (
            "status_info::git_branch_at_cached",
            "git 分支名（#281：HEAD mtime 缓存面）",
        ),
        (
            "status_info::detect_encoding_eol_cached",
            "编码+换行符探测（#281：mtime+size 缓存面）",
        ),
    ] {
        assert!(
            sb.contains(call),
            "【面 5：状态栏 → 数据源】数据项断链——{what}（{call}）未被状态栏消费"
        );
    }
    chain(
        "状态栏 → 数据源",
        &[(
            "shell.rs",
            "status_bar::status_bar(",
            "shell 渲染状态栏（Editor 现场态注入）",
        )],
    );
}

/// #585 残余①+②：视图切换钮的**文案接线**与**条件占位结构**。
///
/// ① 文案链：两枚切换钮（`pdf-view-toggle` / `xlsx-view-toggle`）的
/// `Label::new` 消费 `status_info::pdf_toggle_label` /
/// `xlsx_toggle_label` 纯函数（状态 → 文案的单一事实源——映射在
/// status_info 单测穷尽锁死，此处锁「渲染真的消费它」）。
///
/// ② 零占位结构：钮构造（id + Label）必须**整体位于**
/// `.when(is_pdf, …)` / `.when(is_xlsx, …)` 条件臂内——GPUI 的
/// `when(false, f)` 不应用 f（文档语义），故非该形态标签时按钮
/// 元素/命中盒/占位**不存在**（不止「点击无效」——节点本身不在
/// 树内；md 态运行时全扫零翻转见 shell.rs 批 11 测试）。
#[test]
fn face5_view_toggle_buttons_wire_labels_and_occupy_conditionally() {
    let sb = src("status_bar.rs");

    // ① 文案接线：钮标签 = status_info 纯函数（非内联字面量）。
    for (call, btn, what) in [
        (
            "status_info::pdf_toggle_label(pdf_view_mode)",
            "pdf-view-toggle",
            "pdf 切换钮文案（#585 ①）",
        ),
        (
            "status_info::xlsx_toggle_label(xlsx_grid_view_mode)",
            "xlsx-view-toggle",
            "xlsx 切换钮文案（#585 ①）",
        ),
    ] {
        assert!(
            sb.contains(call),
            "【面 5：状态栏 → 数据源】{btn} 文案断链——{what}（`{call}`）未被消费"
        );
    }

    // ② 条件占位：按钮 id 只允许出现在对应 when 臂内（结构性零占位）。
    // when 臂块提取：`.when(is_pdf, |el| { … }` 起的配对大括号。
    for (guard, btn) in [
        ("is_pdf", "pdf-view-toggle"),
        ("is_xlsx", "xlsx-view-toggle"),
    ] {
        let arm = when_arm(&sb, guard);
        assert!(
            arm.contains(&format!("\"{btn}\"")),
            "【面 5：状态栏 → 数据源】{btn} 构造不在 `.when({guard}, …)` 臂内——             条件占位被破坏（非{guard}标签时按钮仍渲染/占位）"
        );
        // 全文件唯一性：id 出现次数 = when 臂内出现次数（臂外零构造点）。
        let total = sb.matches(&format!("\"{btn}\"")).count();
        let in_arm = arm.matches(&format!("\"{btn}\"")).count();
        assert_eq!(
            total, in_arm,
            "【面 5：状态栏 → 数据源】{btn} 在 `.when({guard})` 臂外另有构造点——             存在绕过条件占位的渲染路径"
        );
    }
}

/// 提取 `.when(<guard>, |el| { … }` 的配对大括号块（首个匹配——
/// status_bar.rs 中每守卫仅一臂）。
fn when_arm(file: &str, guard: &str) -> String {
    let needle = format!(".when({guard}, |el| ");
    let start = file
        .find(&needle)
        .unwrap_or_else(|| panic!("`.when({guard}, |el| …)` 条件臂缺失——条件占位结构被拆除"));
    let body_start = file[start..].find('{').expect("when 臂后无块体") + start;
    let mut depth = 0usize;
    for (i, b) in file[body_start..].bytes().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return file[body_start..body_start + i + 1].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("when 臂块未闭合：{guard}");
}

// ---------------------------------------------------------------------------
// 面 6：设置项 → 生效
// ---------------------------------------------------------------------------

/// 设置面板四项（主题/字号/缩进/面板宽度）→ shell 生效方法 → 持久化
/// （save_* 落盘）。「改了设置界面有变化 + 重启还在」的完整链。
#[test]
fn face6_settings_changes_take_effect() {
    let panel = src("settings_panel.rs");
    for (call, what) in [
        ("this.set_theme_mode(", "主题三态钮"),
        ("this.apply_font_size(", "字号 −/＋ 步进"),
        ("this.set_indent_unit_all(", "缩进单位三态钮"),
        ("this.reset_panel_widths(", "面板宽度重置钮"),
    ] {
        assert!(
            panel.contains(call),
            "【面 6：设置项 → 生效】设置面板断链——{what} 未接 {call}…)"
        );
    }
    // 生效体：运行时应用 + 落盘（重启还在）。
    let shell = src("shell.rs");
    for (decl, apply, persist, what) in [
        (
            "fn set_theme_mode(",
            "theme_switch::apply_mode",
            "save_theme_mode",
            "主题",
        ),
        (
            "fn apply_font_size(",
            "set_font_size",
            "save_font_size",
            "字号",
        ),
        (
            "fn set_indent_unit_all(",
            "set_indent_unit",
            "save_indent_unit",
            "缩进单位",
        ),
        (
            "fn reset_panel_widths(",
            "LEFT_PANEL_WIDTH",
            "save_panel_widths",
            "面板宽度",
        ),
    ] {
        let body = fn_body(&shell, decl);
        assert!(
            body.contains(apply) && body.contains(persist),
            "【面 6：设置项 → 生效】{what}生效断链——{decl} 缺运行时应用\
             （{apply}）或缺持久化（{persist}——重启丢设置）"
        );
    }
}
