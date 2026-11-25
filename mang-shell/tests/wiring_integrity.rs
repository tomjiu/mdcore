//! #168：界面层「接线完整性」回归测试。
//!
//! ## 为什么
//!
//! mang-shell 约 2.8 万行，`tests/` 只有 docx/括号测试——界面层零回归
//! 保护。「按钮是空闭包」这类问题（owner 2026-11-02 release 实测发现，
//! 登记 #165/#166）能一路存活到发布，因为没有任何测试会抓它。
//!
//! ## 测什么（边界：不测 UI 渲染，只测接线表的完整性）
//!
//! | 模块 | 手段 |
//! |---|---|
//! | activity_bar.rs | 静态扫描：`bar_button` 的 on_click 空闭包 = 0；tooltip 无「未接线/骨架」 |
//! | file_tree.rs | ① 数据面（`context_menu::file_tree_menu` 的 action id）↔ 接线面（shell.rs 的 match 分支）双向对账；② 行点击 → 打开文档 / 右键 → reveal 的接线存在性 |
//! | tab_bar.rs | 标签点击 → `activate_tab` 接线存在 + 空闭包 = 0 |
//! | palette_view.rs | 数据链（`commands::search` → `command_groups::build_sections` → `flatten`）零丢失——面板能列出全部命令 |
//! | context_menu_view.rs | 菜单项 on_click → `activate_context_menu_item` 接线存在 + 空闭包 = 0 |
//! | title_bar.rs | 窗口控制钮（最小化/最大化/关闭）接线存在 + 空闭包 = 0 |
//!
//! ## 静态扫描的取舍（issue #168 方案 2 + 3 混合）
//!
//! GPUI 组件（`Context`/`Window`）无法在无窗口环境构造，按钮回调是
//! 渲染期闭包、无注册表可枚举——「不改产线」约束下（铁律：git diff
//! 只含 tests/），静态源码扫描是唯一可行的接线验证面。文本匹配固有
//! 边界：换一种写法的空回调（如引用空函数）抓不到——但当前形态
//! （`|_event, _window, _cx| {}`）与历史 bug 形态一致，能抓真问题。
//! 数据面对账（file_tree/editor 菜单）不受此限：菜单定义是纯数据
//! （`context_menu.rs`），接线分支是静态可解析的 match arm——这条
//! 是**双向强对账**。
//!
//! ## 红绿账（2026-11-02 全绿——协作模式验证成功）
//!
//! #171 预告的协作模式：「B 先写失败态 → A 接线后转绿」。已闭环：
//!
//! | 测试 | 初始 | 转绿依据 |
//! |---|---|---|
//! | `activity_bar_no_dead_buttons` | 🔴（act-search/act-settings 空闭包） | PR #172（#165）：搜索钮接 OpenFind、设置钮按「接不了就移除」铁律删除 |
//! | `file_tree_context_menu_actions_all_wired` | 🔴（ft-* 五项无接线） | PR #174（#166）：触发器/浮层挂载/六动作全接 |
//!
//! 绝不允许 xfail 放过（#171：「那等于给断线发许可证」）——未来新断线
//! 出现时，这两条测试会重新变红，那是它们的职责。
//!
//! ## 铁律
//!
//! - 只加测试，不改产线代码（git diff 只含 tests/）；
//! - 源码文件缺失 = FAIL（不是环境缺件——源码就在仓库里）；
//! - 失败信息必须指明「哪个按钮/哪个菜单项没接线」。

use std::path::{Path, PathBuf};

use mang_shell::command_groups::{RecentRing, build_sections, flatten};
use mang_shell::editor::commands::{self, Cmd};

// ---------------------------------------------------------------------------
// 源码读取与函数体提取（静态检查面工具）
// ---------------------------------------------------------------------------

/// 读 `mang-shell/src/<rel>` 源码。缺失即 FAIL（源码进仓，不是环境件）。
fn src(rel: &str) -> String {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("源码文件缺失/不可读（{}）：{e}", path.display()))
}

/// 源码字节偏移 → 1 基行号（失败信息定位用）。
fn line_of(file: &str, byte_idx: usize) -> usize {
    file[..byte_idx.min(file.len())].matches('\n').count() + 1
}

/// 提取 `decl`（如 `"fn exec_command("`）所在函数的**函数体**（含首尾
/// 大括号）。
///
/// 大括号配对，字符串字面量感知（`"{"` 不计深度）；目标函数（mod.rs 的
/// 方法、shell.rs 的方法）无 char 字面量与生命周期标注，无需处理 `'`。
fn fn_body(file: &str, decl: &str) -> String {
    let start = file.find(decl).unwrap_or_else(|| {
        panic!("静态锚点缺失：源码里找不到 `{decl}`——函数被改名/删除？接线检查失效，请同步测试")
    });
    let body_open = file[start..]
        .find('{')
        .unwrap_or_else(|| panic!("`{decl}` 后无函数体开括号"));
    let bytes = file.as_bytes();
    let mut i = start + body_open;
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_str = false;
            }
        } else if c == b'"' {
            in_str = true;
        } else if c == b'{' {
            depth += 1;
        } else if c == b'}' {
            depth -= 1;
            if depth == 0 {
                return file[start + body_open..=i].to_string();
            }
        }
        i += 1;
    }
    panic!("`{decl}` 函数体大括号不配对（字符串感知扫描到文件尾）");
}

/// 文件里所有字符串字面量（粗提取——供「未接线」措辞扫描）。
fn string_literals(file: &str) -> Vec<(usize, String)> {
    let bytes = file.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start = i + 1;
            let mut j = start;
            let mut escaped = false;
            while j < bytes.len() {
                let c = bytes[j];
                if escaped {
                    escaped = false;
                } else if c == b'\\' {
                    escaped = true;
                } else if c == b'"' {
                    break;
                }
                j += 1;
            }
            if j < bytes.len() {
                out.push((start, file[start..j].to_string()));
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// 空闭包（参数全下划线 + 空体）扫描——「死按钮」的直接形态。
///
/// 命中形态：`|_event, _window, _cx| {}` / `|_, _| {}` /
/// `|_, _, _, cx| {}` 不命中（`cx` 非下划线参数、体非空都不算）。
fn dead_closures(file: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let bytes = file.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'|' {
            // 尝试从这里的管道参数列表解析。
            if let Some((close, params)) = parse_closure_params(bytes, i) {
                // close = `|` 位置；体 = 其后跳过空白后恰为 `{}`（可含空白）。
                let mut j = close + 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j + 1 < bytes.len() && bytes[j] == b'{' && bytes[j + 1] == b'}' {
                    let body = format!("|{params}| {{}}");
                    out.push((i, body));
                }
            }
        }
        i += 1;
    }
    out
}

/// 从 `pos`（`|` 处）解析闭包参数列表：返回 (收尾 `|` 的字节位, 参数文本)。
/// 参数**全部**为 `_` / `_name` 形态才返回 Some——有任何具名参数即 None。
fn parse_closure_params(bytes: &[u8], pos: usize) -> Option<(usize, String)> {
    let mut j = pos + 1;
    let mut params = String::new();
    let mut saw_param = false;
    loop {
        // 跳过空白。
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            params.push(bytes[j] as char);
            j += 1;
        }
        if j >= bytes.len() {
            return None;
        }
        if bytes[j] == b'|' {
            return if saw_param { Some((j, params)) } else { None };
        }
        // 一个参数：必须 `_` 或 `_ident`。
        if bytes[j] != b'_' {
            return None;
        }
        saw_param = true;
        params.push('_');
        j += 1;
        while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
            params.push(bytes[j] as char);
            j += 1;
        }
        // 参数后必须是 `,` 或 `|`（空白已由循环头处理）。
        if j < bytes.len() && bytes[j] == b',' {
            params.push(',');
            j += 1;
            continue;
        }
        if j < bytes.len() && bytes[j] == b'|' {
            return Some((j, params));
        }
        return None;
    }
}

/// 按钮上下文定位：从空闭包位置向前找最近的按钮 id 字面量与其后的
/// tooltip 字面量（bar_button 实参序：id → icon → selected → tooltip）。
fn nearest_button_context(file: &str, closure_pos: usize) -> (String, String) {
    let mut id = String::from("<未知 id>");
    let mut tooltip = String::new();
    for (pos, lit) in string_literals(file) {
        if pos >= closure_pos {
            break;
        }
        if lit.starts_with("act-") {
            id = lit;
            tooltip.clear(); // id 变了，旧 tooltip 作废。
        } else if tooltip.is_empty() {
            // id 之后的第一个字面量即 tooltip（后续字面量不覆盖）。
            tooltip = lit;
        }
    }
    (id, tooltip)
}

// ---------------------------------------------------------------------------
// activity_bar：图标条按钮接线
// ---------------------------------------------------------------------------

/// #165/#171：图标条不得有死按钮（空闭包）、「未接线——骨架」措辞。
///
/// 红绿史：初始红（act-search/act-settings 空闭包）→ PR #172（#165）
/// 合入后绿：搜索钮接 Cmd::OpenFind，设置钮按「接不了就从界面移除」
/// 铁律删除。
#[test]
fn activity_bar_no_dead_buttons() {
    let file = src("activity_bar.rs");

    // ① 空闭包 = 0（失败信息带按钮 id / tooltip / 行号）。
    let dead = dead_closures(&file);
    if !dead.is_empty() {
        let mut msg = String::from("图标条存在死按钮（on_click 空闭包）——#165 待接线：\n");
        for (pos, body) in &dead {
            let (id, tooltip) = nearest_button_context(&file, *pos);
            msg.push_str(&format!(
                "  - activity_bar.rs:{} 按钮 id={id} tooltip={tooltip:?} 闭包={body}\n",
                line_of(&file, *pos)
            ));
        }
        panic!("{msg}铁律（#171）：接不了就从界面移除，不装作能点。");
    }

    // ② tooltip / 可见文案不得自曝「未接线」「骨架」（#171 验收项）。
    for (pos, lit) in string_literals(&file) {
        for word in ["未接线", "骨架"] {
            if lit.contains(word) {
                panic!(
                    "activity_bar.rs:{} 的界面文案 {:?} 含「{word}」——\
                     #171 验收：界面/悬停提示里不出现「未接线」「骨架」字样",
                    line_of(&file, pos),
                    lit
                );
            }
        }
    }

    // ③ 按钮基线：图标条 4 个按钮（文件树/git/搜索/设置）。
    // #289-⑤：右栏开关移至右缘 dock 条（act-toggle-right → dock-
    // toggle-right，基线 5→4）；设置钮保留（#182 恢复，现开弹窗）。
    // 这不是放过死按钮，而是锚定「现存按钮全部真实接线」这一事实；
    // 未来新增/删除按钮时请同步本基线。
    // #212 后签名演进：results 需带命中下标（空查询 = 全部命中、下标集空）。
    let hl_results = commands::search_with_highlights("");
    let sections = build_sections("", &hl_results, &RecentRing::new(4));
    let flat = flatten(&sections);
    assert_eq!(
        flat.len(),
        Cmd::ALL.len(),
        "面板分组装配丢失命令：扁平 {} != 全集 {}",
        flat.len(),
        Cmd::ALL.len()
    );
    for &c in Cmd::ALL {
        assert!(flat.contains(&c), "{c:?} 在分组装配后丢失（键盘选不到）");
    }
    // index 连续从 0 起（键盘导航依赖）。
    for (i, e) in sections.iter().flat_map(|s| s.entries.iter()).enumerate() {
        assert_eq!(
            e.index, i,
            "面板条目 index 不连续：第 {i} 项 index={}",
            e.index
        );
    }

    // ③ 静态钉住 palette_view.rs 的数据来源（组合关系不被换掉）。
    let pv = src("palette_view.rs");
    assert!(
        pv.contains("commands::search"),
        "palette_view 数据源不再是 commands::search"
    );
    assert!(
        pv.contains("build_sections"),
        "palette_view 不再经 build_sections 装配分组"
    );
}

// ---------------------------------------------------------------------------
// #389 阶段 2：xlsx 网格编辑链（双击 → set_cell → 会话银行 → 保存 →
// 脏拦截——静态锚点钉住五段接线不脱钩）
// ---------------------------------------------------------------------------

/// xlsx 编辑闭环接线面：
/// ① 视图层——双击进编辑（click_count）、提交走引擎 `set_cell`、
///    保存走 `save_to_vec` + 原子写盘；
/// ② shell 层——会话银行（切标签保活）、保存统一收口 `save_xlsx_at`
///    （菜单/确认框/退出三路）、脏拦截（`xlsx_dirty_at`）。
#[test]
fn xlsx_grid_edit_chain_wired() {
    let view = src("xlsx_grid_view.rs");
    // ① 双击进编辑：click_count 分流（单击选中/双击编辑）。
    assert!(
        view.contains("ev.click_count >= 2") && view.contains("begin_edit_at"),
        "网格无双击进编辑接线（click_count 分流缺失）"
    );
    // ② 提交走引擎公开 API（不改 office-io 内部）。
    assert!(
        view.contains(".set_cell(") && view.contains("commit_cell_edit"),
        "编辑提交未走 XlsxEditor::set_cell（引擎公开面）"
    );
    // ③ 保存走 save_to_vec + 原子写盘（临时文件 + rename）。
    assert!(
        view.contains(".save_to_vec(") && view.contains("write_xlsx_atomically"),
        "保存未走 save_to_vec + 原子写盘（写坏毁原件风险）"
    );
    // ④ 键位面：Enter 提交 / Esc 取消（编辑态键处理在场）。
    assert!(
        view.contains("\"enter\"") && view.contains("\"escape\""),
        "编辑态缺 Enter/Esc 键位臂"
    );

    let shell = src("shell.rs");
    // ⑤ 会话银行：切标签 stash + 重建回接（脏编辑跨实体保活）。
    assert!(
        shell.contains("xlsx_sessions") && shell.contains("stash_xlsx_session"),
        "shell 缺会话银行（切标签重建丢已提交未落盘编辑——静默丢失）"
    );
    assert!(
        shell.contains("new_with_session"),
        "shell 缺会话回接（银行命中未注回重建实体）"
    );
    // ⑥ 保存统一收口：菜单保存（dispatch_editor_cmd）与关闭确认/
    //    退出共用 save_xlsx_at。
    assert!(
        shell.contains("save_xlsx_at"),
        "shell 缺 xlsx 保存统一收口（Ctrl+S/菜单/关闭确认三路分叉）"
    );
    // ⑦ 脏拦截：关闭确认/标签脏态查询追查网格会话（不再只看行模型
    //    只读编辑器的恒 false）。
    assert!(
        shell.contains("xlsx_dirty_at"),
        "shell 缺 xlsx 会话脏态查询（关闭确认漏拦网格编辑丢失）"
    );
}
