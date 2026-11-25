//! #47 巨石拆解批⑤：原 keymap.rs 内嵌 `tests` 纯迁出
//!（零行为变化——字节原样剥壳，#[cfg(test)] + #[path] 挂载在宿主文件）。

use super::*;
use Mod::{Any, Yes};

/// 查询构造（接线层 `KeyQuery` 的测试面）。
fn q(key: &str, ctrl: bool, shift: bool, alt: bool) -> KeyQuery {
    KeyQuery {
        key: key.to_string(),
        ctrl,
        shift,
        alt,
    }
}

/// 断言默认编辑器主表命中（无挂起）。
fn editor_hit(key: &str, ctrl: bool, shift: bool, alt: bool, want: EditorAction) {
    let store = KeymapStore::default_tables();
    let got = store.resolve_editor_main(&q(key, ctrl, shift, alt), None);
    assert_eq!(
        got,
        EditorOutcome::Matched(want),
        "键 {key}（ctrl={ctrl} shift={shift} alt={alt}）应命中 {want:?}，实得 {got:?}"
    );
}

/// 断言默认编辑器主表无命中（`_` 分支——可打印插入/冒泡面）。
fn editor_miss(key: &str, ctrl: bool, shift: bool, alt: bool) {
    let store = KeymapStore::default_tables();
    let got = store.resolve_editor_main(&q(key, ctrl, shift, alt), None);
    assert_eq!(
        got,
        EditorOutcome::None,
        "键 {key}（ctrl={ctrl} shift={shift} alt={alt}）应无命中"
    );
}

/// 断言默认 shell 表命中（无挂起）。
fn shell_hit(key: &str, ctrl: bool, shift: bool, alt: bool, want: ShellAction) {
    let store = KeymapStore::default_tables();
    let got = store.resolve_shell(&q(key, ctrl, shift, alt), None, Instant::now());
    assert_eq!(
        got,
        ShellOutcome::Matched(want),
        "shell 键 {key}（ctrl={ctrl} shift={shift} alt={alt}）应命中 {want:?}，实得 {got:?}"
    );
}

/// 断言默认 shell 表无命中。
fn shell_miss(key: &str, ctrl: bool, shift: bool, alt: bool) {
    let store = KeymapStore::default_tables();
    let got = store.resolve_shell(&q(key, ctrl, shift, alt), None, Instant::now());
    assert_eq!(
        got,
        ShellOutcome::None,
        "shell 键 {key}（ctrl={ctrl} shift={shift} alt={alt}）应无命中"
    );
}

// ─── #111 D1：行移动 Alt+↑/↓ 键位表 ────────────────────────────

/// #111 D1：Alt+↑/↓ 命中行移动（表接线面——编译红先行：
/// EditorAction::MoveLineUp/Down 变体未定义时本测试不可编译）。
#[test]
fn alt_up_down_hit_line_move() {
    editor_hit("up", false, false, true, EditorAction::MoveLineUp);
    // #111 P2 续批②：选区大小写转换键位（Ctrl+Shift+U/L/T）。
    editor_hit("u", true, true, false, EditorAction::UpperCaseSelection);
    editor_hit("j", true, true, false, EditorAction::LowerCaseSelection);
    editor_hit("t", false, true, true, EditorAction::TitleCaseSelection);
    editor_hit("down", false, false, true, EditorAction::MoveLineDown);
    // 修饰精确性：ctrl+alt+up 不应命中（Any/Yes 约束数——精确者胜，
    // 3 修饰键 = 纯 Alt↑ 条目 ctrl=Any 不匹配 ctrl=Yes 查询……
    // 不：Any 接受 Yes。表条目 (Any, Any, Yes) 对 ctrl+alt+up 也命中
    // ——参照 Q-IDE「alt 修饰 + 忽略 ctrl/shift」宽面同型。此处钉
    // 主形态纯 Alt；ctrl 叠加形态不另设条目（无二义）。
    editor_hit("up", false, false, true, EditorAction::MoveLineUp);
}

/// #111 D1：裸 ↑/↓ 仍命中光标移动（行移动不劫持光标键主形态——
/// alt=false 走 MoveKey 家族，回归面）。
#[test]
fn bare_up_down_still_cursor_move() {
    editor_hit(
        "up",
        false,
        false,
        false,
        EditorAction::MoveKey(MoveDir::Up),
    );
    editor_hit(
        "down",
        false,
        false,
        false,
        EditorAction::MoveKey(MoveDir::Down),
    );
}

// ─── 默认表自检 ─────────────────────────────────────────────────

/// 默认表无歧义绑定（同域相交不等的合法取舍 + Pass 豁免之外无冲突
/// ——check_conflicts 的豁免规则）。
#[test]
fn default_tables_conflict_free() {
    let store = KeymapStore::default_tables();
    let conflicts = store.check_conflicts();
    assert!(conflicts.is_empty(), "默认表存在歧义：{conflicts:?}");
}

/// 机器名全域唯一 + id/from_id 往返（用户覆盖解析的根据）。
#[test]
fn action_ids_unique_and_roundtrip() {
    let mut ids: Vec<&str> = Vec::new();
    for a in EDITOR_ACTION_IDS
        .iter()
        .copied()
        .map(Action::Editor)
        .chain(SHELL_ACTION_IDS.iter().copied().map(Action::Shell))
    {
        ids.push(a.id());
        assert_eq!(
            Action::from_id(a.id()),
            Some(a),
            "{} 反查应命中自身",
            a.id()
        );
    }
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(ids.len(), sorted.len(), "机器名不得重复：{ids:?}");
    assert_eq!(Action::from_id("no_such_action"), None);
    assert_eq!(Action::from_id(""), None);
}

// ─── 编辑器主层逐臂对照（E2-E32——默认行为零变化红线） ──────────

/// 移动家族 E4-E7：裸键/shift 选区/ctrl 词跳（修饰进执行层参数——
/// 表层只认键，Any 修饰全命中）。
#[test]
fn editor_move_family_all_modifier_combos() {
    for (key, dir) in [
        ("left", MoveDir::Left),
        ("right", MoveDir::Right),
        ("up", MoveDir::Up),
        ("down", MoveDir::Down),
    ] {
        for (ctrl, shift, alt) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (true, true, true),
        ] {
            editor_hit(key, ctrl, shift, alt, EditorAction::MoveKey(dir));
        }
    }
}

/// home/end 家族 E8-E9：ctrl 分流在执行层——表层全修饰命中同动作。
#[test]
fn editor_home_end_family() {
    for (ctrl, shift) in [(false, false), (true, false), (false, true), (true, true)] {
        editor_hit("home", ctrl, shift, false, EditorAction::HomeKey);
        editor_hit("end", ctrl, shift, false, EditorAction::EndKey);
    }
}

/// 翻页/编辑键 E10-E14。
#[test]
fn editor_paging_and_edit_keys() {
    for (ctrl, shift) in [(false, false), (true, false), (false, true)] {
        editor_hit("pageup", ctrl, shift, false, EditorAction::PageUpKey);
        editor_hit("pagedown", ctrl, shift, false, EditorAction::PageDownKey);
        editor_hit("enter", ctrl, shift, false, EditorAction::Newline);
        editor_hit("backspace", ctrl, shift, false, EditorAction::Backspace);
        editor_hit("delete", ctrl, shift, false, EditorAction::DeleteForward);
    }
}

/// E15：Tab 带 !ctrl 守卫——Ctrl+Tab 不命中（冒泡 shell 标签循环）。
#[test]
fn editor_tab_guard_bubbles_ctrl_tab() {
    editor_hit("tab", false, false, false, EditorAction::Indent);
    editor_hit("tab", false, true, false, EditorAction::Indent);
    editor_miss("tab", true, false, false); // Ctrl+Tab → shell
    editor_miss("tab", true, true, false); // Ctrl+Shift+Tab → shell
}

/// E16-E20：Esc / 选区 / 剪贴板。
#[test]
fn editor_escape_clipboard_select_all() {
    editor_hit(
        "escape",
        false,
        false,
        false,
        EditorAction::EscapeClearSelection,
    );
    editor_hit("a", true, false, false, EditorAction::SelectAll);
    editor_hit("c", true, true, false, EditorAction::Copy);
    editor_hit("x", true, false, true, EditorAction::Cut);
    editor_hit("v", true, true, true, EditorAction::Paste);
    // 无 ctrl 不命中（`_` 分支可打印插入）。
    editor_miss("a", false, false, false);
    editor_miss("c", false, false, false);
}

/// E21-E23：undo/redo 双键位——约束度排序复现 match 臂序。
#[test]
fn editor_undo_redo_disambiguation() {
    editor_hit("z", true, true, false, EditorAction::Redo);
    editor_hit("z", true, true, true, EditorAction::Redo); // alt Any
    editor_hit("z", true, false, false, EditorAction::Undo);
    editor_hit("y", true, false, false, EditorAction::RedoY);
    editor_hit("y", true, true, false, EditorAction::RedoY);
    editor_miss("z", false, false, false);
}

/// E24-E28：加同词光标 / Alt+F 折叠 / 折叠快捷键 / Ctrl+F 查找。
#[test]
fn editor_fold_word_match_and_find() {
    editor_hit("d", true, false, false, EditorAction::AddNextWordMatch);
    // E25：Alt+F（alt 守卫——ctrl/shift 任意；臂序在 "f" if ctrl 前，
    // ctrl+alt+f 同走 ToggleFold——现状 match 臂序精确复现）。
    editor_hit("f", false, false, true, EditorAction::ToggleFold);
    editor_hit("f", true, false, true, EditorAction::ToggleFold);
    editor_hit("f", false, true, true, EditorAction::ToggleFold);
    // E28：Ctrl+F 查找（无 alt）。
    editor_hit("f", true, false, false, EditorAction::OpenFind);
    editor_hit("f", true, true, false, EditorAction::OpenFind);
    editor_hit("[", true, true, false, EditorAction::FoldCurrent);
    editor_hit("[", true, true, true, EditorAction::FoldCurrent);
    editor_hit("]", true, true, false, EditorAction::UnfoldCurrent);
    editor_miss("[", false, false, false);
    editor_miss("[", true, false, false); // 无 shift 不命中
}

/// E29-E30：替换 / Ctrl+S·Ctrl+Shift+S 空臂放行（#386：保存分流
/// 归 shell 域——两键位全冒泡）。
#[test]
fn editor_replace_save_pass() {
    editor_hit("h", true, false, false, EditorAction::OpenReplace);
    editor_hit("h", true, true, true, EditorAction::OpenReplace);
    // #386：Ctrl+S 不再由编辑器自处理（保存分流归 Shell）——
    // 整键空臂放行冒泡（shell 域 ShellAction::Save 接管）。
    editor_hit("s", true, false, false, EditorAction::Pass);
    editor_hit("s", true, false, true, EditorAction::Pass);
    editor_hit("s", true, true, false, EditorAction::Pass); // 冒泡 shell
    editor_hit("s", true, true, true, EditorAction::Pass);
    editor_miss("s", false, false, false);
}

/// E0：前置层 palette toggle（查表点在 palette 模态之前）。
#[test]
fn editor_palette_toggle_pre_layer() {
    let store = KeymapStore::default_tables();
    assert_eq!(
        store.resolve_pre(Site::EditorPalette, &q("p", true, true, false)),
        Some(Action::Editor(EditorAction::PaletteToggle))
    );
    // alt Any：Ctrl+Shift+Alt+P 同样命中（现状 `ctrl && shift` 守卫）。
    assert_eq!(
        store.resolve_pre(Site::EditorPalette, &q("p", true, true, true)),
        Some(Action::Editor(EditorAction::PaletteToggle))
    );
    assert_eq!(
        store.resolve_pre(Site::EditorPalette, &q("p", true, false, false)),
        None
    );
    assert_eq!(
        store.resolve_pre(Site::EditorPalette, &q("p", false, true, false)),
        None
    );
}

/// E1：前置层 goto 开框（!shift 守卫；is_none 条件在执行层）。
#[test]
fn editor_goto_open_pre_layer() {
    let store = KeymapStore::default_tables();
    assert_eq!(
        store.resolve_pre(Site::EditorGoto, &q("g", true, false, false)),
        Some(Action::Editor(EditorAction::GotoOpen))
    );
    assert_eq!(
        store.resolve_pre(Site::EditorGoto, &q("g", true, false, true)),
        Some(Action::Editor(EditorAction::GotoOpen)) // alt Any
    );
    assert_eq!(
        store.resolve_pre(Site::EditorGoto, &q("g", true, true, false)),
        None
    );
    assert_eq!(
        store.resolve_pre(Site::EditorGoto, &q("g", false, false, false)),
        None
    );
}

/// E2/E3/E32：编辑器 chord 全流程——挂起/续接/作废转新起点/刷新
/// （对齐 `ctrl_k_pending` 状态机）。
#[test]
fn editor_chord_full_lifecycle() {
    let store = KeymapStore::default_tables();
    // 首键挂起（k 臂提前 return——接线层存本键快照）。
    let out = store.resolve_editor_main(&q("k", true, false, false), None);
    assert_eq!(out, EditorOutcome::Pending, "Ctrl+K 应挂起，实得 {out:?}");
    let pending = q("k", true, false, false);
    // 挂起中 0 → 全部折叠（续接键修饰 Any——Ctrl+K Ctrl+0 亦触发，
    // 现状怪癖照抄）。
    for (ctrl, shift) in [(false, false), (true, false), (false, true), (true, true)] {
        assert_eq!(
            store.resolve_editor_main(&q("0", ctrl, shift, false), Some(&pending)),
            EditorOutcome::Matched(EditorAction::FoldAll),
            "挂起中 0（ctrl={ctrl} shift={shift}）应全部折叠"
        );
        assert_eq!(
            store.resolve_editor_main(&q("j", ctrl, shift, false), Some(&pending)),
            EditorOutcome::Matched(EditorAction::UnfoldAll)
        );
    }
    // 挂起中 Ctrl+K → 刷新挂起（前缀作废后单键再命中首键）。
    assert_eq!(
        store.resolve_editor_main(&q("k", true, false, false), Some(&pending)),
        EditorOutcome::Pending
    );
    // 挂起中续接落空 → 挂起终结 + 该键当新起点（不吞键）：
    // Ctrl+F 照常打开查找。
    assert_eq!(
        store.resolve_editor_main(&q("f", true, false, false), Some(&pending)),
        EditorOutcome::Matched(EditorAction::OpenFind)
    );
    // 挂起中可打印字符 x → 无命中（接线层走 `_` 插入）。
    assert_eq!(
        store.resolve_editor_main(&q("x", false, false, false), Some(&pending)),
        EditorOutcome::None
    );
    // 无挂起时 0/J 是普通键（Ctrl+0 冒泡 shell 字号重置）。
    assert_eq!(
        store.resolve_editor_main(&q("0", true, false, false), None),
        EditorOutcome::None
    );
    editor_miss("j", false, false, false);
    // Shift+K 不挂起（!shift 守卫；ctrl=false 纯 Shift+K——
    // ctrl+shift+k 现归 #111 批⑦ DeleteLine）。
    assert_eq!(
        store.resolve_editor_main(&q("k", false, true, false), None),
        EditorOutcome::None
    );
    // Alt+K 挂起（alt Any——现状守卫不查 alt）。
    assert_eq!(
        store.resolve_editor_main(&q("k", true, false, true), None),
        EditorOutcome::Pending
    );
}

// ─── shell 层逐臂对照（S1-S11） ─────────────────────────────────

/// shell 全部单键分支（chord 除外）。
#[test]
fn shell_single_key_branches() {
    shell_hit("tab", true, false, false, ShellAction::NextTab);
    shell_hit("tab", true, true, false, ShellAction::PrevTab);
    shell_miss("tab", true, false, true); // alt 守卫
    shell_miss("tab", false, false, false); // 无 ctrl → 编辑器缩进
    shell_hit("w", true, false, false, ShellAction::CloseTab);
    shell_miss("w", true, true, false); // plain_ctrl
    shell_miss("w", true, false, true);
    // #386 S12：Ctrl+S 保存分流（编辑器 E30 旧键位移除后冒泡至此）。
    shell_hit("s", true, false, false, ShellAction::Save);
    shell_miss("s", true, false, true); // plain（!alt）守卫
    shell_miss("s", false, false, false); // 无 ctrl → 可打印字符
    shell_hit("s", true, true, false, ShellAction::PromptSaveAs);
    shell_miss("s", true, true, true); // alt 守卫（S5 带 !alt）
    shell_hit("=", true, false, false, ShellAction::FontBigger);
    shell_hit("=", true, true, false, ShellAction::FontBigger); // shift Any
    shell_miss("=", true, true, true); // alt 守卫（S6/S7/S8 带 !alt）
    shell_hit("+", true, true, false, ShellAction::FontBigger);
    shell_hit("-", true, false, false, ShellAction::FontSmaller);
    shell_hit("0", true, false, false, ShellAction::FontReset);
    shell_miss("0", true, true, false); // plain_ctrl
    shell_hit("t", true, false, false, ShellAction::CycleTheme);
    shell_hit("o", true, false, false, ShellAction::PromptOpenFile);
    // #108 P2③：Ctrl+Shift+O → GotoSymbol（原「无命中」断言随
    // 键位新增同步更新——行为扩展非回归）。
    shell_hit("o", true, true, false, ShellAction::GotoSymbol);
    shell_miss("o", false, false, false);
}

/// S1：shell chord 全流程——挂起/续接/过期作废/作废后继续分支
/// （对齐 `chord_until` 状态机）。
#[test]
fn shell_chord_full_lifecycle() {
    let store = KeymapStore::default_tables();
    let now = Instant::now();
    // 首键挂起（接线层存快照 + deadline）。
    let pending = q("k", true, false, false);
    assert_eq!(
        store.resolve_shell(&pending, None, now),
        ShellOutcome::Pending,
        "shell Ctrl+K 应挂起"
    );
    // 窗口期内 Ctrl+O → 打开工作区（吞掉，不落 Ctrl+O 打开文件）。
    let deadline = now + std::time::Duration::from_secs(2);
    assert_eq!(
        store.resolve_shell(
            &q("o", true, false, false),
            Some(&(pending.clone(), deadline)),
            now
        ),
        ShellOutcome::Matched(ShellAction::PromptOpenWorkspace)
    );
    // 过期 → 作废不吞键：Ctrl+O 落单键分支 = 打开文件。
    let expired = now - std::time::Duration::from_secs(1);
    assert_eq!(
        store.resolve_shell(
            &q("o", true, false, false),
            Some(&(pending.clone(), expired)),
            now
        ),
        ShellOutcome::Matched(ShellAction::PromptOpenFile)
    );
    // 窗口期内续接落空（Ctrl+W）→ 作废 + 该键继续后续分支 = 关标签
    // （现状「过期/非 O → 和弦作废」后分支序不变）。
    assert_eq!(
        store.resolve_shell(
            &q("w", true, false, false),
            Some(&(pending.clone(), deadline)),
            now
        ),
        ShellOutcome::Matched(ShellAction::CloseTab)
    );
    // 挂起中普通字符 → 无 shell 命中。
    assert_eq!(
        store.resolve_shell(
            &q("x", false, false, false),
            Some(&(pending.clone(), deadline)),
            now
        ),
        ShellOutcome::None
    );
    // 挂起中再按 Ctrl+K → 刷新挂起。
    assert_eq!(
        store.resolve_shell(&q("k", true, false, false), Some(&(pending, deadline)), now),
        ShellOutcome::Pending
    );
}

// ─── 键串解析 ───────────────────────────────────────────────────

/// 解析全形态（含 "+" 键名、大小写、chord、错误）。
#[test]
fn parse_binding_forms() {
    use Binding::*;
    assert_eq!(
        parse_binding("ctrl+k ctrl+o").unwrap(),
        Chord(
            KeyPattern::key("k").with(Yes, Any, Any),
            KeyPattern::key("o").with(Yes, Any, Any)
        )
    );
    assert_eq!(
        parse_binding("ctrl+shift+[").unwrap(),
        Single(KeyPattern::key("[").with(Yes, Yes, Any))
    );
    // "+" 键名（前缀扫描不误拆）。
    assert_eq!(
        parse_binding("ctrl++").unwrap(),
        Single(KeyPattern::key("+").with(Yes, Any, Any))
    );
    assert_eq!(
        parse_binding("ctrl+-").unwrap(),
        Single(KeyPattern::key("-").with(Yes, Any, Any))
    );
    // 大小写不敏感 + 修饰序任意。
    assert_eq!(
        parse_binding("SHIFT+Ctrl+A").unwrap(),
        Single(KeyPattern::key("a").with(Yes, Yes, Any))
    );
    assert_eq!(
        parse_binding("alt+f").unwrap(),
        Single(KeyPattern::key("f").with(Any, Any, Yes))
    );
    // 错误形态。
    assert!(parse_binding("").is_err());
    assert!(parse_binding("ctrl+k ctrl+o ctrl+j").is_err());
    assert!(parse_binding("ctrl+").is_err());
    assert!(parse_binding("ctrl+ctrl+k").is_err());
    // 不支持的修饰词（meta/cmd/…）显式报错——幻影键防线
    // （#218 review），逐词面见 parse_pattern_rejects_phantom_modifiers。
    assert!(parse_binding("meta+k").is_err());
}

/// 幻影修饰词防线（#218 review）：已知但不支持的修饰词前缀、
/// 混入支持修饰词后的不支持词、含 `+` 的未知词、裸修饰词作键名
/// ——全部 Err（dispatch 永不产生这些键名）；既有合法形态（含
/// "+" 键名的 `ctrl++`）回归钉定不受影响。
#[test]
fn parse_pattern_rejects_phantom_modifiers() {
    for bad in [
        // 已知但不支持的修饰词前缀（大小写不敏感）。
        "meta+k",
        "Meta+K",
        "META+k",
        "cmd+s",
        "command+s",
        "win+e",
        "windows+d",
        "super+x",
        "option+v",
        "altgr+n",
        "control+k",
        "Control+K",
        // 混入支持修饰词后（扫描逐轮检查——第二段照样抓）。
        "ctrl+meta+k",
        "ctrl+alt+cmd+k",
        "shift+win+e",
        // 未知词带 "+"（"foo+bar" 类）。
        "foo+bar",
        "ctrl+foo+bar",
        // 裸修饰词单独作键名。
        "meta",
        "Meta",
        "cmd",
        "ctrl",
        "Ctrl",
        "control",
        "shift",
        "alt",
        "Alt",
    ] {
        assert!(parse_binding(bad).is_err(), "\"{bad}\" 应报错（幻影键）");
    }
    // 既有合法形态回归钉定（"+" 键名必须继续合法——前缀扫描不误拆）。
    for ok in [
        "ctrl++",
        "ctrl+-",
        "ctrl+shift+[",
        "SHIFT+Ctrl+A",
        "alt+f",
        "ctrl+k ctrl+o",
        "ctrl+alt+home",
    ] {
        assert!(parse_binding(ok).is_ok(), "\"{ok}\" 应保持合法");
    }
    // 错误文案可读（警告直出 stderr 的口径）。
    let err = parse_binding("meta+k").unwrap_err().0;
    assert!(
        err.contains("meta") && err.contains("ctrl/shift/alt"),
        "{err}"
    );
    let err = parse_binding("meta").unwrap_err().0;
    assert!(
        err.contains("meta") && err.contains("不能单独作键名"),
        "{err}"
    );
}

/// 幻影键换绑的关键回归（#218 review 原例）：bind "meta+k" → save
/// 被拒且 **ctrl+s 默认绑定完好**（解析失败发生在移除旧绑定之前
/// ——apply 先 parse 后变异，结构性保证）；unbind 幻影键同样被拒
/// 且零移除。
#[test]
fn phantom_modifier_bind_rejected_keeps_default_save() {
    let mut store = KeymapStore::default_tables();
    let warnings = store.apply_user(&[
        UserOverride::Bind {
            bind: "meta+k".into(),
            action: "save".into(),
        },
        UserOverride::Unbind {
            bind: "meta+k".into(),
        },
    ]);
    assert_eq!(warnings.len(), 2, "两条都应被拒：{warnings:?}");
    assert!(
        warnings[0].contains("不支持的修饰词「meta」"),
        "bind 拒绝文案应指明修饰词：{}",
        warnings[0]
    );
    assert!(
        warnings[1].contains("不支持的修饰词「meta」"),
        "unbind 拒绝文案应指明修饰词：{}",
        warnings[1]
    );
    // #386：ctrl+s 默认形态 = 空臂放行冒泡（保存分流归 shell 域
    // ——E30 旧键位已移除；meta+k 被拒不得影响 s 键族默认形态）。
    assert_eq!(
        store.resolve_editor_main(&q("s", true, false, false), None),
        EditorOutcome::Matched(EditorAction::Pass),
        "meta+k 被拒不得影响 ctrl+s 默认（空臂放行）"
    );
    // 全表零移除：默认 chord 完好（unbind 幻影键零副作用）。
    let pending = q("k", true, false, false);
    assert_eq!(
        store.resolve_editor_main(&q("0", false, false, false), Some(&pending)),
        EditorOutcome::Matched(EditorAction::FoldAll)
    );
}

// ─── 标签生成（menu_bar 字面量对拍——格式零漂移） ───────────────

/// 默认表标签 = menu_bar 既有 16 对标签字面量（单一事实源转移，
/// 值零漂移——edit-find 与 view-side-search 共用 open_find 的标签）。
#[test]
fn default_labels_match_menu_bar_literals() {
    let store = KeymapStore::default_tables();
    let pinned: &[(&str, &str)] = &[
        ("prompt_open_file", "Ctrl+O"),
        ("prompt_open_workspace", "Ctrl+K Ctrl+O"),
        ("save", "Ctrl+S"),
        ("prompt_save_as", "Ctrl+Shift+S"),
        ("undo", "Ctrl+Z"),
        ("redo", "Ctrl+Shift+Z"),
        ("cut", "Ctrl+X"),
        ("copy", "Ctrl+C"),
        ("paste", "Ctrl+V"),
        ("open_find", "Ctrl+F"),
        ("open_replace", "Ctrl+H"),
        ("toggle_command_palette", "Ctrl+Shift+P"),
        ("font_bigger", "Ctrl+="),
        ("font_smaller", "Ctrl+-"),
        ("font_reset", "Ctrl+0"),
    ];
    for (id, label) in pinned {
        assert_eq!(
            store.label(id).as_deref(),
            Some(*label),
            "{id} 标签漂移（menu_bar 字面量对拍）"
        );
    }
    // redo 第二键位独立标签（表序：主键位在前）。
    assert_eq!(store.label("redo_y").as_deref(), Some("Ctrl+Y"));
    // 折叠/跳行家族标签格式（Cmd::default_key 同款口径对拍）。
    assert_eq!(store.label("fold_all").as_deref(), Some("Ctrl+K Ctrl+0"));
    assert_eq!(store.label("unfold_all").as_deref(), Some("Ctrl+K Ctrl+J"));
    assert_eq!(store.label("toggle_fold").as_deref(), Some("Alt+F"));
    assert_eq!(store.label("fold_current").as_deref(), Some("Ctrl+Shift+["));
    assert_eq!(
        store.label("unfold_current").as_deref(),
        Some("Ctrl+Shift+]")
    );
    assert_eq!(store.label("goto_line").as_deref(), Some("Ctrl+G"));
}

// ─── 用户覆盖（换绑/解绑/冲突——#217 验收面） ───────────────────

/// 换绑生效：新键触发原动作、旧键不再触发（issue 验收标准）。
#[test]
fn bind_replaces_old_binding() {
    let mut store = KeymapStore::default_tables();
    let warnings = store.apply_user(&[UserOverride::Bind {
        bind: "ctrl+alt+p".into(),
        action: "toggle_command_palette".into(),
    }]);
    assert!(warnings.is_empty(), "无冲突换绑不应警告：{warnings:?}");
    // 新键触发原动作（前置层查表点）。
    assert_eq!(
        store.resolve_pre(Site::EditorPalette, &q("p", true, true, true)),
        Some(Action::Editor(EditorAction::PaletteToggle))
    );
    // 旧键不再触发。
    assert_eq!(
        store.resolve_pre(Site::EditorPalette, &q("p", true, true, false)),
        None
    );
    // 标签跟随新绑定（menu_bar 单一事实源）。
    assert_eq!(
        store.label("toggle_command_palette").as_deref(),
        Some("Ctrl+Alt+P")
    );
}

/// 同键换动作（pattern 相等 → 替换——覆盖默认表最直觉路径）。
#[test]
fn bind_same_key_replaces_action() {
    let mut store = KeymapStore::default_tables();
    let warnings = store.apply_user(&[UserOverride::Bind {
        bind: "ctrl+f".into(),
        // #386：编辑器保存动作改名 editor_save（"save" 归 shell
        // 保存分流——本测试钉定编辑器动作仍可同键替换）。
        action: "editor_save".into(),
    }]);
    assert!(warnings.is_empty(), "同键替换应放行：{warnings:?}");
    // ctrl+f → save（替换了 open_find）。
    assert_eq!(
        store.resolve_editor_main(&q("f", true, false, false), None),
        EditorOutcome::Matched(EditorAction::Save)
    );
    // open_find 无绑定（标签消失）。
    assert_eq!(store.label("open_find"), None);
    // save 原键 ctrl+s 不再保存——E31 Pass 接管该键（不消费冒泡，
    // 接线面与 None 同效）。
    assert_eq!(
        store.resolve_editor_main(&q("s", true, false, false), None),
        EditorOutcome::Matched(EditorAction::Pass)
    );
}

/// 冲突拒绝：同域相交且不更精确（新条目永不触发）——拒绝时动作
/// **保持既有绑定**（先检测后改）。
///
/// #386 后冲突对换型：旧例（ctrl+alt+s × 编辑器 Save ctrl+!shift+s）
/// 随 E30 移除失效（编辑器表再无 No-约束 2 约束条目，同精度不相
/// 容相交在编辑器域不可构造）。现用 shell 域同构例：S8 `-`(ctrl,!alt)
/// × 新 ctrl+shift+-（同为 2 约束、互不覆盖——canonical 双命中，
/// 表序早者 S8 赢）→ 拒绝。
#[test]
fn bind_conflicting_key_rejected() {
    let mut store = KeymapStore::default_tables();
    // font_reset 换绑到 ctrl+shift+-（2 约束）：S8 FontSmaller
    // -（ctrl,!alt）同为 2 约束、互不覆盖，canonical 双命中 →
    // 表序早者 S8 赢 → 新条目永不触发 → 拒绝。
    let warnings = store.apply_user(&[UserOverride::Bind {
        bind: "ctrl+shift+-".into(),
        action: "font_reset".into(),
    }]);
    assert_eq!(warnings.len(), 1, "应恰好一条警告：{warnings:?}");
    assert!(
        warnings[0].contains("冲突"),
        "警告应说明冲突：{}",
        warnings[0]
    );
    // 拒绝 = 该条完全不生效：font_reset 保持 ctrl+0 原绑、S8
    // FontSmaller 原样（断言直查本 store，不用 shell_hit——它自
    // 建默认表，验不到拒绑后状态）。
    assert_eq!(
        store.resolve_shell(&q("0", true, false, false), None, Instant::now()),
        ShellOutcome::Matched(ShellAction::FontReset),
        "拒绑后 font_reset 原绑不动"
    );
    assert_eq!(
        store.resolve_shell(&q("-", true, true, false), None, Instant::now()),
        ShellOutcome::Matched(ShellAction::FontSmaller),
        "拒绑后 S8 原样（ctrl+shift+- 仍 FontSmaller——S8 的 shift Any）"
    );
    // save 不受影响（#386：shell 域 S12 ctrl+s 保存分流——与拒绑
    // 前同形态）。
    assert_eq!(
        store.resolve_shell(&q("s", true, false, false), None, Instant::now()),
        ShellOutcome::Matched(ShellAction::Save),
        "S12 保存分流不受拒绑影响"
    );
}

/// 跨域冲突拒绝：同键两域叠加执行（编辑器不拦冒泡）。
#[test]
fn bind_cross_domain_conflict_rejected() {
    let mut store = KeymapStore::default_tables();
    // ctrl+o 已被 shell prompt_open_file 占用——编辑器动作 bind 上去
    // 会被拒绝（#386：编辑器保存动作改名 editor_save）。
    let warnings = store.apply_user(&[UserOverride::Bind {
        bind: "ctrl+o".into(),
        action: "editor_save".into(),
    }]);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("shell"),
        "警告应指明跨域：{}",
        warnings[0]
    );
    // shell 的 ctrl+o 不受影响；#386：save 默认无键位（ctrl+s
    // 空臂放行——保存分流归 shell 域）。
    shell_hit("o", true, false, false, ShellAction::PromptOpenFile);
    assert_eq!(
        store.resolve_editor_main(&q("s", true, false, false), None),
        EditorOutcome::Matched(EditorAction::Pass)
    );
}

/// 精确度豁免：更精确的用户条目与家族条目并存（alt+left 叠在
/// 方向键家族的 Any 条目上——运行时精确者胜）。
#[test]
fn bind_more_specific_coexists_with_family() {
    let mut store = KeymapStore::default_tables();
    let warnings = store.apply_user(&[UserOverride::Bind {
        bind: "alt+left".into(),
        action: "open_replace".into(),
    }]);
    assert!(warnings.is_empty(), "更精确条目应放行：{warnings:?}");
    // alt+left → 新动作（约束数 1 > 家族 0）。
    assert_eq!(
        store.resolve_editor_main(&q("left", false, false, true), None),
        EditorOutcome::Matched(EditorAction::OpenReplace)
    );
    // 裸方向键/ctrl/shift 组合仍走家族（执行层移动语义不变）。
    assert_eq!(
        store.resolve_editor_main(&q("left", false, false, false), None),
        EditorOutcome::Matched(EditorAction::MoveKey(MoveDir::Left))
    );
    assert_eq!(
        store.resolve_editor_main(&q("left", true, true, false), None),
        EditorOutcome::Matched(EditorAction::MoveKey(MoveDir::Left))
    );
}

/// chord 跨域冲突：续接键相同的两域 chord（Ctrl+K Ctrl+O 双执行）
/// 被拒绝。
#[test]
fn bind_chord_cross_domain_rejected() {
    let mut store = KeymapStore::default_tables();
    // 编辑器 bind [ctrl+k ctrl+o]→editor_save：与 shell S1 [ctrl+k
    // ctrl+o] 首键相交且续接键相交 → 跨域拒绝。
    let warnings = store.apply_user(&[UserOverride::Bind {
        bind: "ctrl+k ctrl+o".into(),
        action: "editor_save".into(),
    }]);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("叠加执行"), "{}", warnings[0]);
}

/// 同域新 chord 与默认 chord 续接分流并存（[ctrl+k x] 与 [ctrl+k 0]）。
#[test]
fn bind_new_chord_coexists_by_followup() {
    let mut store = KeymapStore::default_tables();
    let warnings = store.apply_user(&[UserOverride::Bind {
        bind: "ctrl+k x".into(),
        action: "open_replace".into(),
    }]);
    assert!(warnings.is_empty(), "续接分流应放行：{warnings:?}");
    // 挂起快照 = 默认首键（Ctrl+K 竞争中约束数 2 胜过用户首键 1）。
    let pending = q("k", true, false, false);
    // 续接 x → 新动作（用户和弦首键 Any 变体经快照可达）。
    assert_eq!(
        store.resolve_editor_main(&q("x", true, false, false), Some(&pending)),
        EditorOutcome::Matched(EditorAction::OpenReplace)
    );
    // 默认续接 0/j 不受影响。
    assert_eq!(
        store.resolve_editor_main(&q("0", false, false, false), Some(&pending)),
        EditorOutcome::Matched(EditorAction::FoldAll)
    );
}

/// 解绑：移除该键实际触发的绑定（chord 写全两键；拼写修饰宽于
/// 默认守卫也能命中——按 canonical 按键解析赢家）。
#[test]
fn unbind_removes_exact_binding() {
    let mut store = KeymapStore::default_tables();
    let warnings = store.apply_user(&[UserOverride::Unbind {
        bind: "ctrl+k ctrl+0".into(),
    }]);
    assert!(warnings.is_empty(), "{warnings:?}");
    // ctrl+k 仍挂起（ctrl+k ctrl+j 还在）。
    assert_eq!(
        store.resolve_editor_main(&q("k", true, false, false), None),
        EditorOutcome::Pending
    );
    // 挂起后 0 不再折叠——当新起点（无单键 0 绑定 → None）。
    let pending = q("k", true, false, false);
    assert_eq!(
        store.resolve_editor_main(&q("0", false, false, false), Some(&pending)),
        EditorOutcome::None
    );
    // j 仍触发全部展开。
    assert_eq!(
        store.resolve_editor_main(&q("j", false, false, false), Some(&pending)),
        EditorOutcome::Matched(EditorAction::UnfoldAll)
    );
    // 无此绑定 → 警告。
    let warnings = store.apply_user(&[UserOverride::Unbind {
        bind: "ctrl+f9".into(),
    }]);
    assert_eq!(warnings.len(), 1);
}

/// chord 解绑赢家镜像 dispatch（#218 review 原例）：表同时有
/// `ctrl+shift+l x` 与 `ctrl+l ctrl+x` 时，序列（ctrl+shift+l,
/// ctrl+x）dispatch 触发**后者**（续接尾键精确度 1>0）——
/// unbind "ctrl+shift+l ctrl+x" 必须恰移除后者（原 `(首键, 尾键)`
/// 字典序按头优先移除了前者——dispatch 不会触发的那条），另一条
/// 完好。
#[test]
fn unbind_chord_winner_mirrors_dispatch() {
    let mut store = KeymapStore::default_tables();
    let warnings = store.apply_user(&[
        UserOverride::Bind {
            bind: "ctrl+shift+l x".into(),
            action: "open_replace".into(),
        },
        UserOverride::Bind {
            bind: "ctrl+l ctrl+x".into(),
            action: "open_find".into(),
        },
    ]);
    assert!(warnings.is_empty(), "两条 chord 换绑均应放行：{warnings:?}");
    // dispatch 前提对拍：序列（ctrl+shift+k, ctrl+x）触发 open_find
    // 的 chord（首键竞争 ctrl+shift+k 头 spec 2 胜出挂起；续接
    // ctrl+x 尾 spec 1 > open_replace 裸 x 尾 spec 0）。
    let first = q("l", true, true, false);
    assert_eq!(
        store.resolve_editor_main(&first, None),
        EditorOutcome::Pending
    );
    assert_eq!(
        store.resolve_editor_main(&q("x", true, false, false), Some(&first)),
        EditorOutcome::Matched(EditorAction::OpenFind),
        "dispatch 应触发 ctrl+k ctrl+x（尾键精确者胜）"
    );
    // 反向序列（ctrl+shift+k, 裸 x）触发 open_replace 的 chord。
    assert_eq!(
        store.resolve_editor_main(&q("x", false, false, false), Some(&first)),
        EditorOutcome::Matched(EditorAction::OpenReplace)
    );
    // 解绑 dispatch 会触发的那条（review 原例的键串）。
    let warnings = store.apply_user(&[UserOverride::Unbind {
        bind: "ctrl+shift+l ctrl+x".into(),
    }]);
    assert!(warnings.is_empty(), "应恰命中一条移除：{warnings:?}");
    // 另一条完好：同序列现在触发 open_replace 的 chord（若移错了
    // 条，此解析会变 None/Cut——旧字典序赢家恰在此暴露）。
    assert_eq!(
        store.resolve_editor_main(&q("x", true, false, false), Some(&first)),
        EditorOutcome::Matched(EditorAction::OpenReplace),
        "解绑后应由幸存的 ctrl+shift+k x 接管该序列"
    );
    assert_eq!(
        store.resolve_editor_main(&q("x", false, false, false), Some(&first)),
        EditorOutcome::Matched(EditorAction::OpenReplace)
    );
    // 被解绑的 chord 不再触发：ctrl+k 挂起后续接 ctrl+x 落空 →
    // 本键当新起点（ctrl+x = Cut——挂起终结语义）。
    let pending = q("k", true, false, false);
    assert_eq!(
        store.resolve_editor_main(&q("x", true, false, false), Some(&pending)),
        EditorOutcome::Matched(EditorAction::Cut)
    );
}

/// 反向序列对称用例：unbind "ctrl+k ctrl+x" 恰移除 dispatch 在
/// （ctrl+k, ctrl+x）序列上触发的那条；ctrl+shift+k x 不受影响。
#[test]
fn unbind_chord_reverse_sequence_symmetric() {
    let mut store = KeymapStore::default_tables();
    let warnings = store.apply_user(&[
        UserOverride::Bind {
            bind: "ctrl+shift+l x".into(),
            action: "open_replace".into(),
        },
        UserOverride::Bind {
            bind: "ctrl+k ctrl+x".into(),
            action: "open_find".into(),
        },
    ]);
    assert!(warnings.is_empty(), "{warnings:?}");
    let pending = q("k", true, false, false);
    // dispatch 前提：序列（ctrl+k, ctrl+x）触发 open_find 的 chord
    //（首键默认 chord 头 spec 2 胜出挂起；续接只它匹配 ctrl+x）。
    assert_eq!(
        store.resolve_editor_main(&q("x", true, false, false), Some(&pending)),
        EditorOutcome::Matched(EditorAction::OpenFind)
    );
    // 解绑该序列实际触发的绑定。
    let warnings = store.apply_user(&[UserOverride::Unbind {
        bind: "ctrl+k ctrl+x".into(),
    }]);
    assert!(warnings.is_empty(), "{warnings:?}");
    // 同序列不再触发 open_find：续接落空 → 本键当新起点（Cut）。
    assert_eq!(
        store.resolve_editor_main(&q("x", true, false, false), Some(&pending)),
        EditorOutcome::Matched(EditorAction::Cut)
    );
    // open_find 无绑定（默认 ctrl+f 已在换绑时让位，chord 又被解绑）。
    assert_eq!(store.label("open_find"), None);
    // 另一条不受影响：序列（ctrl+shift+l, x）照常触发 open_replace
    //（原例键 ctrl+shift+k 现归 #111 批⑦ DeleteLine——换 l 保语义）。
    let first = q("l", true, true, false);
    assert_eq!(
        store.resolve_editor_main(&q("x", false, false, false), Some(&first)),
        EditorOutcome::Matched(EditorAction::OpenReplace)
    );
}

/// 单键 bind 被 chord 首键抢赢 → 拒绝（原实现 canon[1] 越界
/// panic——单键绑定无第二键；#218 review 附加审查）。dispatch 语义
/// 对拍：ctrl+k 的赢家是 chord 首（spec 2 > 新单键 1）→ 恒 Pending，
/// 新单键绑定永不触发 → canonical 赢家策略拒绝，既有绑定保持。
#[test]
fn bind_single_key_stolen_by_chord_head_rejected() {
    let mut store = KeymapStore::default_tables();
    let warnings = store.apply_user(&[UserOverride::Bind {
        bind: "ctrl+k".into(),
        action: "fold_all".into(),
    }]);
    assert_eq!(warnings.len(), 1, "应恰好一条冲突警告：{warnings:?}");
    assert!(
        warnings[0].contains("冲突") && warnings[0].contains("unfold_all"),
        "警告应指明抢占者（Ctrl+K Ctrl+J → unfold_all）：{}",
        warnings[0]
    );
    // 拒绝 = 该条完全不生效：默认 chord 全部保持。
    assert_eq!(
        store.resolve_editor_main(&q("k", true, false, false), None),
        EditorOutcome::Pending
    );
    let pending = q("k", true, false, false);
    assert_eq!(
        store.resolve_editor_main(&q("0", false, false, false), Some(&pending)),
        EditorOutcome::Matched(EditorAction::FoldAll),
        "fold_all 默认 chord 完好（拒绝不落任何变更）"
    );
}

/// 格式错误：未知动作 / 坏键串——单条拒绝 + 可读警告，其余生效。
#[test]
fn malformed_overrides_rejected_with_readable_warnings() {
    let mut store = KeymapStore::default_tables();
    let warnings = store.apply_user(&[
        UserOverride::Bind {
            bind: "ctrl+alt+f12".into(),
            action: "no_such_action".into(),
        },
        UserOverride::Bind {
            bind: "bad key str".into(), // 空格分 chord → 三段超限
            action: "save".into(),
        },
        UserOverride::Bind {
            bind: "alt+home".into(),
            action: "open_find".into(),
        },
    ]);
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings[0].contains("未知动作"));
    assert!(warnings[1].contains("两键"));
    // 第三条好配置照常生效（alt+home 更精确于 home 家族 Any 条目
    // ——canonical 赢家检测放行，运行时精确者胜）。
    assert_eq!(
        store.resolve_editor_main(&q("home", false, false, true), None),
        EditorOutcome::Matched(EditorAction::OpenFind)
    );
}

/// 覆盖后的表通过冲突自检（check_conflicts 与 apply_user 策略一致）。
#[test]
fn store_after_overrides_still_conflict_checked() {
    let mut store = KeymapStore::default_tables();
    store.apply_user(&[
        UserOverride::Bind {
            bind: "ctrl+alt+p".into(),
            action: "toggle_command_palette".into(),
        },
        UserOverride::Bind {
            bind: "ctrl+alt+left".into(),
            action: "open_replace".into(),
        },
    ]);
    assert!(
        store.check_conflicts().is_empty(),
        "合法覆盖后不应有歧义：{:?}",
        store.check_conflicts()
    );
}

// ─── 只读门禁（动作级兜底——#218 review 修复） ─────────────────

/// readonly_allows 镜像夹具（**镜像自 docx_doc.rs:1101，两处需同步
/// 改**——keymap 层纯 std 红线不依赖 docx_doc，单测用誊抄副本对拍；
/// docx_doc 侧有同名单测钉定其自身）。
fn readonly_allows_mirror(key: &str, ctrl: bool, alt: bool) -> bool {
    match key {
        // #111 D1：Alt+↑/↓ 行移动（编辑命令）——镜像同步拒。
        "up" | "down" if alt => false,
        "left" | "right" | "up" | "down" | "home" | "end" | "pageup" | "pagedown" | "escape" => {
            true
        }
        "a" | "c" | "f" | "tab" | "=" | "+" | "-" | "0" | "t" | "g" => ctrl,
        _ => false,
    }
}

/// 分类表逐变体钉定（29 变体全覆盖——接线层兜底的判定面）。
#[test]
fn readonly_safe_classification_pinned() {
    let safe: Vec<EditorAction> = vec![
        EditorAction::PaletteToggle,
        EditorAction::GotoOpen,
        EditorAction::MoveKey(MoveDir::Left),
        EditorAction::MoveKey(MoveDir::Right),
        EditorAction::MoveKey(MoveDir::Up),
        EditorAction::MoveKey(MoveDir::Down),
        EditorAction::HomeKey,
        EditorAction::EndKey,
        EditorAction::PageUpKey,
        EditorAction::PageDownKey,
        EditorAction::EscapeClearSelection,
        EditorAction::SelectAll,
        EditorAction::Copy,
        EditorAction::OpenFind,
        EditorAction::Pass,
    ];
    let unsafe_: Vec<EditorAction> = vec![
        EditorAction::Newline,
        EditorAction::Backspace,
        EditorAction::DeleteForward,
        EditorAction::Indent,
        EditorAction::Cut,
        EditorAction::Paste,
        EditorAction::Redo,
        EditorAction::Undo,
        EditorAction::RedoY,
        EditorAction::AddNextWordMatch,
        EditorAction::Save,
        EditorAction::OpenReplace,
        EditorAction::ToggleComment,
        EditorAction::ToggleFold,
        EditorAction::FoldCurrent,
        EditorAction::UnfoldCurrent,
        EditorAction::FoldAll,
        EditorAction::UnfoldAll,
    ];
    for a in &safe {
        assert!(a.readonly_safe(), "{a:?} 应为只读安全（浏览类）");
    }
    for a in &unsafe_ {
        assert!(!a.readonly_safe(), "{a:?} 应为只读不安全（变更类）");
    }
    // 全集对账：EDITOR_ACTION_IDS 29 变体恰好二分（新增变体漏分类
    // 即在此爆——分类表不得有漏网）。
    assert_eq!(
        safe.len() + unsafe_.len(),
        EDITOR_ACTION_IDS.len(),
        "分类表必须覆盖全部变体"
    );
    for a in EDITOR_ACTION_IDS {
        let classified = safe.contains(a) || unsafe_.contains(a);
        assert!(classified, "{}（{:?}）未入分类表", a.id(), a);
    }
}

/// 默认表不变量：凡 canonical 键能过只读物理守卫（readonly_allows
/// 白名单）的条目，其动作必须 readonly_safe——证明动作级兜底对
/// **默认表**零行为变化（两层守卫在默认键位上从不冲突：物理守卫
/// 放行的键解析出的动作，兜底层必然放行）。
#[test]
fn default_table_readonly_guard_invariant() {
    let store = KeymapStore::default_tables();
    for (entries, _) in store.domains() {
        for e in entries {
            let Action::Editor(a) = e.action else {
                continue; // shell 域动作不经编辑器只读门禁
            };
            let canon = canonical_queries(&e.binding);
            let passes = match canon.as_slice() {
                // 单键：物理守卫放行该键 → 动作必须 safe。
                [c1] => readonly_allows_mirror(&c1.key, c1.ctrl, c1.alt),
                // chord：首键过守卫（挂起）且尾键过守卫（续接执行）
                // → 动作必须 safe（默认表首键 ctrl+k 不过白名单，
                // 恒 false——断言对默认表恒空转，用户换绑后生效）。
                [c1, c2] => {
                    readonly_allows_mirror(&c1.key, c1.ctrl, c1.alt)
                        && readonly_allows_mirror(&c2.key, c2.ctrl, c2.alt)
                }
                _ => false,
            };
            if passes {
                assert!(
                    a.readonly_safe(),
                    "默认条目 \"{}\" → {} 过物理守卫但动作非只读安全（默认行为漂移）",
                    label_binding(&e.binding),
                    a.id()
                );
            }
        }
    }
}

/// 绕道回归：ctrl+c（物理守卫放行键）换绑 delete 后——物理守卫仍
/// 放行（白名单镜像对拍），但解析出 DeleteForward 非只读安全 →
/// 接线层兜底条件成立（read_only && !readonly_safe → 拒绝）；
/// 安全动作（copy）换绑不受影响。
#[test]
fn readonly_bypass_blocked_at_action_level() {
    // 物理守卫对 ctrl+c 放行（绕道前提——镜像夹具对拍）。
    assert!(readonly_allows_mirror("c", true, false));
    assert!(!readonly_allows_mirror("c", false, false));
    let mut store = KeymapStore::default_tables();
    let warnings = store.apply_user(&[UserOverride::Bind {
        bind: "ctrl+c".into(),
        action: "delete".into(),
    }]);
    assert!(warnings.is_empty(), "无冲突换绑不应警告：{warnings:?}");
    // 换绑生效：ctrl+c 解析出 DeleteForward（物理守卫放行该键）……
    assert_eq!(
        store.resolve_editor_main(&q("c", true, false, false), None),
        EditorOutcome::Matched(EditorAction::DeleteForward)
    );
    // ……且 DeleteForward 非只读安全 → 新接线层必拒（兜底条件成立）。
    assert!(!EditorAction::DeleteForward.readonly_safe());
    // 其余默认放行键不受换绑影响（select_all 完好）。
    assert_eq!(
        store.resolve_editor_main(&q("a", true, false, false), None),
        EditorOutcome::Matched(EditorAction::SelectAll)
    );
    // 安全动作换绑不受影响：ctrl+alt+c → copy 照常解析且安全。
    let mut store2 = KeymapStore::default_tables();
    let warnings = store2.apply_user(&[UserOverride::Bind {
        bind: "ctrl+alt+c".into(),
        action: "copy".into(),
    }]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        store2.resolve_editor_main(&q("c", true, false, true), None),
        EditorOutcome::Matched(EditorAction::Copy)
    );
    assert!(EditorAction::Copy.readonly_safe());
}

// ─── 全局装配 ───────────────────────────────────────────────────

/// store() 未 init 时 = 默认表（单测/harness 默认面——测试不碰全局
/// 写接口，进程内并行测试互不污染）。
#[test]
fn global_store_defaults_without_init() {
    let label = store().label("save").expect("默认表应可查");
    assert_eq!(label, "Ctrl+S");
}

/// #108 P2③：Ctrl+Shift+O 解析到 GotoSymbol（shell 域——
/// 菜单/键位同链；Ctrl+G 归 EditorGoto 不抢；Ctrl+O 开文件不抢
///——plain("o") 的 Any 修饰面在 best_single 特异性排序下让位
/// 给 ctrl+shift 特定约束）。
#[test]
fn goto_symbol_ctrl_shift_o_resolves() {
    shell_hit("o", true, true, false, ShellAction::GotoSymbol);
    // 守卫边界：Ctrl+O 归 PromptOpenFile（plain("o") Any 修饰域）——
    // goto_symbol 的 ctrl+shift 特定约束不吞它。
    shell_hit("o", true, false, false, ShellAction::PromptOpenFile);
}
