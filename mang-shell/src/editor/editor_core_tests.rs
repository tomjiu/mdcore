//! #47 巨石拆解批②：原 editor/mod.rs 内嵌 `tests` 纯迁出（零行为变化——
//! 字节原样剥壳，挂载 #[cfg(test)] + #[path] 在 mod.rs）。

use super::*;

fn p(line: usize, col: usize) -> Point {
    Point::new(line, col)
}

// ---------------- G2：跳转到行（纯函数面） ----------------

#[test]
fn goto_parse_valid_one_based() {
    // 1 基输入 → 0 基目标行
    assert_eq!(parse_goto_line("1", 100), Some(0));
    assert_eq!(parse_goto_line("42", 100), Some(41));
    assert_eq!(parse_goto_line(" 7 ", 100), Some(6), "容许前后空白");
    assert_eq!(parse_goto_line("100", 100), Some(99), "末行恰好");
}

#[test]
fn goto_parse_out_of_range_clamps_not_crash() {
    // 越界 clamp 到末行（不崩溃）；提示行由 hint 承载
    assert_eq!(parse_goto_line("99999", 100), Some(99));
    assert_eq!(parse_goto_line("1000000", 1), Some(0));
}

#[test]
fn goto_parse_invalid_inputs() {
    assert_eq!(parse_goto_line("", 100), None, "空输入不跳");
    assert_eq!(parse_goto_line("0", 100), None, "行号从 1 起");
    assert_eq!(parse_goto_line("abc", 100), None, "非数字");
    assert_eq!(parse_goto_line("12x", 100), None, "混合字符");
    assert_eq!(parse_goto_line("-3", 100), None, "负号非法");
    assert_eq!(parse_goto_line("1.5", 100), None, "小数点非法");
}

#[test]
fn goto_hint_three_states() {
    let empty = goto_hint("", 100);
    assert!(empty.contains("1-100"), "空输入给范围提示：{empty}");
    let preview = goto_hint("42", 100);
    assert_eq!(preview, "Enter 跳到第 42 行");
    let over = goto_hint("999", 100);
    assert_eq!(over, "越界 → 跳到末行（第 100 行）");
    let bad = goto_hint("xyz", 100);
    assert_eq!(bad, "非数字——请输入行号");
    let zero = goto_hint("0", 100);
    assert_eq!(zero, "行号从 1 起");
}

// ---------------- G3：缩进/反缩进（纯函数面） ----------------

#[test]
fn indent_line_edit_inserts_unit_at_line_start() {
    let e = indent_line_edit(7, IndentUnit::Spaces2);
    assert_eq!(e, (p(7, 0), p(7, 0), "  ".to_string()));
    let e4 = indent_line_edit(0, IndentUnit::Spaces4);
    assert_eq!(e4.2, "    ");
    let et = indent_line_edit(3, IndentUnit::Tab);
    assert_eq!(et.2, "\t");
}

#[test]
fn outdent_line_edit_partial_and_full_removal() {
    let b = Buffer::from_plain_text("  two\n    four\nnone");
    // 前导 2，单位 4 → 删 min(2,4)=2
    assert_eq!(
        outdent_line_edit(&b, 0, IndentUnit::Spaces4),
        Some((p(0, 0), p(0, 2), String::new()))
    );
    // 前导 4，单位 4 → 删 4
    assert_eq!(
        outdent_line_edit(&b, 1, IndentUnit::Spaces4),
        Some((p(1, 0), p(1, 4), String::new()))
    );
    // 无前导 → None（不产生空编辑）
    assert_eq!(outdent_line_edit(&b, 2, IndentUnit::Spaces4), None);
}

#[test]
fn outdent_line_edit_prefers_leading_tab_char() {
    let b = Buffer::from_plain_text("\tcode");
    // Tab 单位：删 1 个 \t 字符
    assert_eq!(
        outdent_line_edit(&b, 0, IndentUnit::Tab),
        Some((p(0, 0), p(0, 1), String::new()))
    );
    // Spaces2 单位：行首是 \t → 仍删 1 char（Tab 字符优先——单位
    // 不改变 Tab 字符的删除粒度）
    assert_eq!(
        outdent_line_edit(&b, 0, IndentUnit::Spaces2),
        Some((p(0, 0), p(0, 1), String::new()))
    );
}

#[test]
fn outdent_line_edit_tab_unit_folds_spaces_by_width() {
    let b = Buffer::from_plain_text("      six");
    // 前导 6 空格，Tab 单位折算宽 4 → 删 4（留 2——渐进对齐语义）
    assert_eq!(
        outdent_line_edit(&b, 0, IndentUnit::Tab),
        Some((p(0, 0), p(0, 4), String::new()))
    );
    // Spaces2 单位 → 删 2
    assert_eq!(
        outdent_line_edit(&b, 0, IndentUnit::Spaces2),
        Some((p(0, 0), p(0, 2), String::new()))
    );
}

#[test]
fn utf16_roundtrip_ascii() {
    let b = Buffer::from_plain_text("hello\nworld");
    assert_eq!(point_to_utf16_offset(&b, p(0, 0)), 0);
    assert_eq!(point_to_utf16_offset(&b, p(0, 5)), 5);
    assert_eq!(point_to_utf16_offset(&b, p(1, 0)), 6, "跨行 +1（换行符）");
    assert_eq!(point_to_utf16_offset(&b, p(1, 5)), 11);
    // 逆映射
    assert_eq!(utf16_offset_to_point(&b, 0), p(0, 0));
    assert_eq!(utf16_offset_to_point(&b, 6), p(1, 0));
    assert_eq!(utf16_offset_to_point(&b, 11), p(1, 5));
}

#[test]
fn utf16_roundtrip_cjk() {
    // 中文：每 char 3 字节 / 1 个 UTF16 码元；emoji "😀" 是 2 个码元。
    let b = Buffer::from_plain_text("中文😀\nx");
    // 行 0：中(1)文(1)😀(2) = 4 码元。
    assert_eq!(
        point_to_utf16_offset(&b, p(0, 3)),
        4,
        "3 个 char = 4 个 UTF16 码元"
    );
    assert_eq!(point_to_utf16_offset(&b, p(1, 0)), 5);
    assert_eq!(point_to_utf16_offset(&b, p(1, 1)), 6);
    // 逆：码元 3 落在 emoji 内 → clamp 到其后的 char 边界（col 3）。
    assert_eq!(utf16_offset_to_point(&b, 3), p(0, 3));
    assert_eq!(utf16_offset_to_point(&b, 5), p(1, 0));
    assert_eq!(utf16_offset_to_point(&b, 6), p(1, 1));
}

#[test]
fn utf16_clamp_to_document_end() {
    let b = Buffer::from_plain_text("ab");
    // 越界偏移 clamp 到文尾。
    assert_eq!(utf16_offset_to_point(&b, 999), p(0, 2));
}

/// 状态标签语义（构造 Editor 全字段有 App 依赖，标签逻辑钉定在
/// Buffer 纯逻辑层——此处验证组合格式）。
#[test]
fn label_format_semantics() {
    let mut b = Buffer::from_plain_text("中文字");
    let label = |b: &Buffer| {
        let c = b.cursor();
        format!("{}, {}", c.line + 1, c.col + 1)
    };
    assert_eq!(label(&b), "1, 1");
    b.set_cursor(p(0, 2));
    assert_eq!(label(&b), "1, 3");
    assert_eq!(format!("{} 字符", b.char_count()), "3 字符");
    assert!(!b.is_modified());
    b.insert("新增");
    assert!(b.is_modified());
}

// ---------------- #132-3：折叠 UI 入口（纯函数面） ----------------

#[test]
fn heading_level_parses_atx_forms() {
    assert_eq!(heading_level_of("# 一级"), Some(1));
    assert_eq!(heading_level_of("### 三级"), Some(3));
    assert_eq!(heading_level_of("###### 六级"), Some(6));
    assert_eq!(heading_level_of("#"), Some(1), "裸 # 也是标题");
    assert_eq!(heading_level_of("####### 七级"), None, "7 个 # 不是标题");
    assert_eq!(heading_level_of("#无空格"), None, "# 后须有空格");
    assert_eq!(heading_level_of("正文 # 不是标题"), None, "行中 # 不算");
    assert_eq!(heading_level_of(""), None);
    assert_eq!(heading_level_of("  # 缩进"), None, "缩进 # 不认");
}

#[test]
fn heading_span_covers_subtree_until_same_level() {
    let lines = ["# A", "正文", "正文2", "# B", "后文"];
    assert_eq!(
        heading_span_in(&lines, 0),
        Some((0, 3)),
        "到下一个同级标题前"
    );
    assert_eq!(heading_span_in(&lines, 3), Some((3, 5)), "末尾标题到文末");
    assert_eq!(heading_span_in(&lines, 1), None, "正文不可折叠");
}

#[test]
fn heading_span_includes_deeper_subheadings() {
    let lines = ["# A", "## B", "b-body", "### C", "c-body", "# D"];
    assert_eq!(
        heading_span_in(&lines, 0),
        Some((0, 5)),
        "一级标题吃掉所有下级小节"
    );
    assert_eq!(heading_span_in(&lines, 1), Some((1, 5)), "二级吃到 # D 前");
    assert_eq!(heading_span_in(&lines, 3), Some((3, 5)), "三级同");
}

#[test]
fn heading_with_no_body_is_not_foldable() {
    // 连续两个标题：前一个没有正文 → 区间 [0,1) 只有 1 行，不折。
    let lines = ["# A", "# B", "body"];
    assert_eq!(heading_span_in(&lines, 0), None, "无正文不折叠（无意义）");
    assert_eq!(heading_span_in(&lines, 1), Some((1, 3)));
}

#[test]
fn top_level_spans_skip_nested_blocks() {
    // A 小节含子标题 B：顶层只有 A，B 不单独出现（避免重叠区间）。
    let lines = ["# A", "## B", "b-body", "# C", "c-body"];
    let fenced = vec![false; lines.len()];
    assert_eq!(
        top_level_fold_spans(&lines, &fenced),
        vec![(0, 3), (3, 5)],
        "顶层两块（A 吃 B）"
    );
}

#[test]
fn top_level_spans_handle_fenced_code() {
    // fenced[i] = 行 i 行首在围栏内；开行/闭行行首 false。
    // 行号：0=intro 1=开``` 2=code1 3=code2 4=闭``` 5=tail
    // 区间 = [1, 4)：开行(1) 是折叠头行、闭行(4) 是**末可见行**，
    // 被藏起来的是 2..3 两行代码。
    let lines = ["intro", "```", "code1", "code2", "```", "tail"];
    let fenced = vec![false, false, true, true, false, false];
    assert_eq!(
        top_level_fold_spans(&lines, &fenced),
        vec![(1, 4)],
        "围栏块 [开行, 闭行] —— 开行折叠头、闭行末可见行"
    );
}

#[test]
fn top_level_spans_mix_headings_and_fences() {
    // 0=# A 1=text 2=开``` 3=code 4=闭``` 5=# B 6=b
    // A 小节 = [0, 5)（到 # B 前）；B 小节 = [5, 7)（到文末）。
    // A 内的代码块不单独出现（顶层语义）。
    let lines = ["# A", "text", "```", "code", "```", "# B", "b"];
    let fenced = vec![false, false, false, true, false, false, false];
    assert_eq!(
        top_level_fold_spans(&lines, &fenced),
        vec![(0, 5), (5, 7)],
        "A 小节整体（代码块在其内）；B 小节其后"
    );
}

#[test]
fn fold_persist_key_is_empty_for_unnamed() {
    assert_eq!(fold_persist_key(None), "");
    assert_eq!(
        fold_persist_key(Some(std::path::Path::new("/w/notes.md"))),
        "/w/notes.md"
    );
}

// ---------------- #132-3：折叠 Editor 级行为面（#416 草稿抢救移植） ----------------
// 来源：根目录草稿 `_foldtests.txt`（#132/#134 折叠轮 rebase 散佚件，
// 见 #416 抢救判定）——原稿基于未落地的 `tests_support::make_editor`
// 设施，此处按仓内 gpui::TestAppContext 习惯重写装载面；断言与
// 语义照原稿。原稿第 10 条（docx 无折叠语义）需要真实 docx 编辑器
// 夹具，属 docx 编辑器测试面，不在本次移植范围（见 #416 PR 说明）。

/// 构造一个 md 编辑器，供折叠行为测试驱动（直接调方法面——不经
/// 命令分发，不触发 persist_folds 落盘）。
/// #111 P2：软换行开关行为（关 = 已有断点清空 + 后续回填段跳过；
/// 开 = 照常。显示行数随断点集回落真实行数）。
#[gpui::test]
fn soft_wrap_toggle_clears_and_restores_breaks(cx: &mut gpui::TestAppContext) {
    let ed = md_editor(cx, "长行甲\n长行乙");
    // 人工注入断点（模拟渲染回填后的 wraps 态——行 0/1 各一断点，
    // 显示行 = 2 行 ×2 段 = 4）。
    ed.update(cx, |e, _| {
        e.display_map.set_wrap_points(&e.buffer, 0, vec![2]);
        e.display_map.set_wrap_points(&e.buffer, 1, vec![2]);
        assert_eq!(
            e.display_map.display_row_count(&e.buffer),
            4,
            "两断点在：2 行 ×2 段"
        );
    });
    // 关软换行：断点清空（显示行回落 2）+ 开关位 false。
    ed.update(cx, |e, _| {
        e.set_soft_wrap(false);
        assert_eq!(e.soft_wrap_enabled(), false, "开关位已关");
        assert_eq!(
            e.display_map.display_row_count(&e.buffer),
            2,
            "断点清空：显示行回落真实行数"
        );
    });
    // 再开：开关位 true（断点回填由渲染层下一帧照常——此处只验
    // 开关语义与幂等）。
    ed.update(cx, |e, _| {
        e.set_soft_wrap(true);
        assert_eq!(e.soft_wrap_enabled(), true, "再开");
        e.set_soft_wrap(true); // 幂等不炸
    });
}

/// #111 P2 续批②：选区大小写转换三态（Upper/Lower/Title）+
/// 无选区 no-op + CJK 恒等（无大小写变换——不产生编辑历史）。
#[gpui::test]
fn case_transform_upper_lower_title_roundtrip(cx: &mut gpui::TestAppContext) {
    let ed = md_editor(cx, "hello WORLD 世界");
    // Upper：选区 "hello"（0-5）→ "HELLO"。
    ed.update(cx, |e, _| {
        e.buffer.set_selection(p(0, 0), p(0, 5));
        assert!(e.transform_selection_case(CaseMode::Upper));
        assert_eq!(e.buffer.line(0), "HELLO WORLD 世界", "Upper");
    });
    // Lower：选区 "WORLD"（6-11）→ "world"。
    ed.update(cx, |e, _| {
        e.buffer.set_selection(p(0, 6), p(0, 11));
        assert!(e.transform_selection_case(CaseMode::Lower));
        assert_eq!(e.buffer.line(0), "HELLO world 世界", "Lower");
    });
    // Title：整行——词首大写（"HELLO world" → "Hello World"；
    // CJK「世界」非词字符恒等）。
    ed.update(cx, |e, _| {
        e.buffer.set_selection(p(0, 0), p(0, 17));
        assert!(e.transform_selection_case(CaseMode::Title));
        assert_eq!(e.buffer.line(0), "Hello World 世界", "Title");
    });
}

/// #111 P2 续批②：边界——无选区 no-op（false 不改文本）+ 纯 CJK
/// 选区恒等（false 不产生编辑历史）。
#[gpui::test]
fn case_transform_no_selection_and_cjk_noop(cx: &mut gpui::TestAppContext) {
    let ed = md_editor(cx, "世界 abc");
    // 无选区（裸光标）→ no-op。
    ed.update(cx, |e, _| {
        e.buffer.set_cursor(p(0, 0));
        assert!(!e.transform_selection_case(CaseMode::Upper), "无选区 no-op");
        assert_eq!(e.buffer.line(0), "世界 abc", "文本未动");
    });
    // 纯 CJK 选区 → 恒等 false（to_uppercase 天然不动的 Unicode 语义）。
    ed.update(cx, |e, _| {
        e.buffer.set_selection(p(0, 0), p(0, 2));
        assert!(
            !e.transform_selection_case(CaseMode::Upper),
            "CJK 恒等 no-op"
        );
        assert_eq!(e.buffer.line(0), "世界 abc", "文本未动");
    });
    // Title 里 CJK 邻接词（"abc世界" 单词串——世界非词字符重置词界）。
    let ed2 = md_editor(cx, "abc世界");
    ed2.update(cx, |e, _| {
        e.buffer.set_selection(p(0, 0), p(0, 5));
        assert!(e.transform_selection_case(CaseMode::Upper));
        assert_eq!(e.buffer.line(0), "ABC世界", "CJK 混选 Upper 只动拉丁");
    });
}

fn md_editor(cx: &mut gpui::TestAppContext, text: &str) -> gpui::Entity<Editor> {
    cx.update(|cx| cx.new(|cx| Editor::new(Some("notes.md".into()), text.to_string(), cx)))
}

/// #111 批②：括号自动闭合——键入开括号插入对、光标居中；
/// 有选区 = 包裹语义。先红：键入 "(" 现状单插无闭合。
#[gpui::test]
fn auto_close_pairs_basic(cx: &mut gpui::TestAppContext) {
    let ed = md_editor(cx, "");
    ed.update(cx, |e, _| {
        e.insert_text_multi("(");
        assert_eq!(e.buffer.to_plain_text(), "()", "开括号自动闭合");
        assert_eq!(e.buffer.cursor().col, 1, "光标居括号对间");

        // over-type：光标右侧已是对应闭括号 → 跳过不插。
        e.insert_text_multi(")");
        assert_eq!(e.buffer.to_plain_text(), "()", "闭括号跳过");
        assert_eq!(e.buffer.cursor().col, 2, "光标跳过闭合括号");
    });
}

/// #111 批②：选区包裹——有选区键入开括号 = 选区被对围住。
#[gpui::test]
fn auto_close_pairs_wraps_selection(cx: &mut gpui::TestAppContext) {
    let ed = md_editor(cx, "abc");
    ed.update(cx, |e, _| {
        e.buffer.set_selection(p(0, 1), p(0, 3));
        e.insert_text_multi("(");
        assert_eq!(e.buffer.to_plain_text(), "a(bc)", "选区被括号包裹");
    });
}

/// #111 批②：配对表纯函数面（含引号对称——既 open 又 close）。
#[test]
fn auto_close_pair_table_pinned() {
    assert_eq!(auto_close_pair_of('('), Some(')'));
    assert_eq!(auto_close_pair_of('['), Some(']'));
    assert_eq!(auto_close_pair_of('{'), Some('}'));
    assert_eq!(auto_close_pair_of('"'), Some('"'));
    assert_eq!(auto_close_pair_of('\u{300c}'), Some('\u{300d}')); // 「」
    assert_eq!(auto_close_pair_of('\u{ff08}'), Some('\u{ff09}')); // （）
    assert_eq!(auto_close_pair_of('x'), None, "普通字符不在配对表");
    assert!(is_close_pair_char(')'));
    assert!(is_close_pair_char('"'));
    assert!(!is_close_pair_char('('), "开括号非闭合侧");
}

/// #111 批②：配对表全覆盖（方/花/引号/全角括号）+ 非配对键
/// 原样单插（回归面）。
#[gpui::test]
fn auto_close_pairs_table_and_plain_chars(cx: &mut gpui::TestAppContext) {
    let ed = md_editor(cx, "");
    ed.update(cx, |e, _| {
        // 非配对字符：原样单插（回归面）。
        e.insert_text_multi("x");
        assert_eq!(e.buffer.to_plain_text(), "x", "普通字符原样单插");
        // 方括号对。
        e.buffer.set_cursor(p(0, 1));
        e.insert_text_multi("[");
        assert_eq!(e.buffer.to_plain_text(), "x[]", "方括号对闭合");
    });
}

/// #111 G2 批⑧：围栏内切换注释——跨行批加（`// ` 前缀）→
/// 再切幂等往返（去）回原文。
#[gpui::test]
fn toggle_comment_roundtrip_in_rust_fence(cx: &mut gpui::TestAppContext) {
    let ed = md_editor(cx, "```rust\nlet a = 1;\nlet b = 2;\n```\ntail");
    // 选区跨 fence 内两行（行 1-2）。
    ed.update(cx, |e, _| {
        e.buffer.set_selection(p(1, 0), p(2, 7));
        e.sync_cursors_from_buffer();
    });
    ed.update(cx, |e, _| e.toggle_comment());
    ed.update(cx, |e, _| {
        assert_eq!(e.buffer.line(1), "// let a = 1;", "跨行批同加");
        assert_eq!(e.buffer.line(2), "// let b = 2;");
        assert_eq!(e.buffer.line(4), "tail", "围栏外不动");
    });
    // 再切（重设选区——首次切后选区存续不属本断言语义）——
    // 全注释 → 去（幂等往返回原文）。
    ed.update(cx, |e, _| {
        e.buffer.set_selection(p(1, 0), p(2, 10));
        e.sync_cursors_from_buffer();
    });
    ed.update(cx, |e, _| e.toggle_comment());
    ed.update(cx, |e, _| {
        assert_eq!(e.buffer.line(1), "let a = 1;", "往返回原文");
        assert_eq!(e.buffer.line(2), "let b = 2;");
    });
}

fn toggle_comment_python_hash_and_md_body_noop(cx: &mut gpui::TestAppContext) {
    let ed = md_editor(cx, "正文段\n```python\nx = 1\n```");
    ed.update(cx, |e, _| {
        e.buffer.set_cursor(p(0, 0));
        e.sync_cursors_from_buffer();
    });
    // md 正文（围栏外）——D2 未裁：无操作。
    ed.update(cx, |e, _| e.toggle_comment());
    ed.update(cx, |e, _| {
        assert_eq!(e.buffer.line(0), "正文段", "md 正文不动（D2 待裁）");
    });
    // python 围栏行 2——`#` 注释符。
    ed.update(cx, |e, _| {
        e.buffer.set_cursor(p(2, 0));
        e.sync_cursors_from_buffer();
    });
    ed.update(cx, |e, _| e.toggle_comment());
    ed.update(cx, |e, _| {
        assert_eq!(e.buffer.line(2), "# x = 1", "python 围栏 # 注释");
    });
    // 再切（重设选区——首次切后选区存续不属本断言语义）——
    // 全注释 → 去（幂等往返回原文）。
    ed.update(cx, |e, _| {
        e.buffer.set_selection(p(1, 0), p(2, 10));
        e.sync_cursors_from_buffer();
    });
    ed.update(cx, |e, _| e.toggle_comment());
    ed.update(cx, |e, _| {
        assert_eq!(e.buffer.line(1), "let a = 1;", "往返回原文");
        assert_eq!(e.buffer.line(2), "let b = 2;");
    });
}

/// #111 P2 批⑦：复制行——行块复制到自身下方（光标不动；undo
/// 一键复原）。先红面：copy_line/delete_line 方法不存在 = 编译红。
#[gpui::test]
fn copy_line_duplicates_below_keeps_cursor(cx: &mut gpui::TestAppContext) {
    let ed = md_editor(cx, "aaa\nbbb\nccc");
    ed.update(cx, |e, _| {
        // 单行复制：bbb（行 1）复制到行 2；光标原地不动。
        e.buffer.set_cursor(p(1, 2));
        assert!(e.copy_line(), "中间行可复制");
        assert_eq!(
            e.buffer.to_plain_text(),
            "aaa\nbbb\nbbb\nccc",
            "bbb 复制到自身下方"
        );
        assert_eq!(e.buffer.cursor().line, 1, "光标行不动");
        assert_eq!(e.buffer.cursor().col, 2, "光标列不动");

        // 多行块（选区覆盖 aaa+bbb）复制：块整体重复到下方。
        e.buffer.set_selection(p(0, 0), p(2, 0));
        assert!(e.copy_line(), "块复制成功");
        assert_eq!(
            e.buffer.to_plain_text(),
            "aaa\nbbb\naaa\nbbb\nbbb\nccc",
            "块整体重复"
        );
    });
}

/// #111 P2 批⑦：删除行——整行删除含换行；单行文档保留语义
/// （不删到空——返回 false）。
#[gpui::test]
fn delete_line_removes_with_newline(cx: &mut gpui::TestAppContext) {
    let ed = md_editor(cx, "aaa\nbbb\nccc");
    ed.update(cx, |e, _| {
        // 单行删除：bbb（行 1）整行含换行消失。
        e.buffer.set_cursor(p(1, 1));
        assert!(e.delete_line(), "中间行可删除");
        assert_eq!(e.buffer.to_plain_text(), "aaa\nccc", "bbb 整行含换行删除");

        // 多行块（选区 p(0,0)→p(2,0) 覆盖行 0-1）删除：余 ccc。
        e.buffer.set_selection(p(0, 0), p(2, 0));
        assert!(e.delete_line(), "块删除成功");
        assert_eq!(
            e.buffer.to_plain_text(),
            "ccc",
            "行 0-1 块删除（选区尾行不含——line_block_range 口径）"
        );
    });
}

/// #111 D1：行移动 Alt+↑/↓（Q-IDE editing.rs:60-116 移植）。
/// 三语义：单行/多行块移动、首行拒移（buffer 不动）、光标列保持
/// 与行内位置复位。先红面：move_line_up/down 方法不存在 = 编译红。
#[gpui::test]
fn move_line_up_down_semantics(cx: &mut gpui::TestAppContext) {
    let ed = md_editor(cx, "aaa\nbbb\nccc\nddd");
    ed.update(cx, |e, _| {
        // 单行上移：bbb（行 1）→ 行 0；光标随行。
        e.buffer.set_cursor(p(1, 1));
        assert!(e.move_line_up(), "中间行可上移");
        assert_eq!(
            e.buffer.to_plain_text(),
            "bbb\naaa\nccc\nddd",
            "bbb 整行上移"
        );
        assert_eq!(e.buffer.cursor().line, 0, "光标随移动行");
        assert_eq!(e.buffer.cursor().col, 1, "光标列保持");

        // 首行拒移：语义「边界拒移」——返回 false、buffer 零变更。
        assert!(!e.move_line_up(), "首行拒移返回 false");
        assert_eq!(
            e.buffer.to_plain_text(),
            "bbb\naaa\nccc\nddd",
            "首行拒移 buffer 不动"
        );
    });
}

/// #111 D1：多行块下移 + undo 一键复原（编辑事务进撤销栈——
/// 同 Q-IDE「移动也是编辑」语义）。
#[gpui::test]
fn move_line_block_down_and_undo(cx: &mut gpui::TestAppContext) {
    let ed = md_editor(cx, "aaa\nbbb\nccc\nddd");
    ed.update(cx, |e, _| {
        // 多行块（选区覆盖 aaa+bbb）下移：与 ccc 换位。
        e.buffer.set_selection(p(0, 0), p(2, 0));
        assert!(e.move_line_down(), "块下移成功");
        assert_eq!(
            e.buffer.to_plain_text(),
            "ccc\naaa\nbbb\nddd",
            "aaa+bbb 块与 ccc 换位"
        );
        // undo 一键复原（行移动是编辑事务——可撤销）。
        assert!(e.buffer.undo(), "行移动进撤销栈");
        assert_eq!(
            e.buffer.to_plain_text(),
            "aaa\nbbb\nccc\nddd",
            "undo 复原块移动"
        );
    });
}

#[gpui::test]
fn fold_current_collapses_heading_block(cx: &mut gpui::TestAppContext) {
    let ed = md_editor(cx, "# A\npara1\npara2\n# B\nafter");
    ed.update(cx, |e, _| {
        // 光标在标题行本身 → 折叠整个小节。
        e.buffer.set_cursor(p(0, 0));
        assert!(e.fold_current(), "光标在标题行应折叠该小节");
        assert_eq!(e.display_map.fold_heads(), vec![0], "折叠头 = 标题行 0");
        assert_eq!(
            e.display_map.fold_ranges(),
            vec![(0, 3)],
            "区间到下一个同级标题前（#B 行 3 可见）"
        );
        // #428：光标在小节正文行 → 回溯折所属小节（原稿语义；
        // 断言随 fix/428 从「现行为钉死 false」翻转为正）。
        e.unfold_all();
        e.buffer.set_cursor(p(1, 0));
        assert!(e.fold_current(), "光标在小节内应折叠该小节（跳到折叠头）");
        assert_eq!(e.display_map.fold_heads(), vec![0], "折叠头 = 标题行 0");
        assert_eq!(e.display_map.fold_ranges(), vec![(0, 3)]);
    });
}

#[gpui::test]
fn fold_current_walkup_takes_innermost_section(cx: &mut gpui::TestAppContext) {
    // #428：嵌套小节正文行回溯取最内层（### C 而非 # A）。
    let ed = md_editor(cx, "# A\n## B\nb-body\n### C\nc-body\n# D");
    ed.update(cx, |e, _| {
        e.buffer.set_cursor(p(4, 0)); // c-body（### C 小节内）
        assert!(e.fold_current());
        assert_eq!(e.display_map.fold_ranges(), vec![(3, 5)], "折最内层 ### C");
    });
}

#[gpui::test]
fn fold_current_walkup_refuses_text_above_first_heading(cx: &mut gpui::TestAppContext) {
    // #428：首个标题之前的正文——之上没有覆盖它的标题块 → false
    //（尾部正文会被末节区间覆盖到文末，故负例取头部）。
    let ed = md_editor(cx, "前言\n# A\nbody");
    ed.update(cx, |e, _| {
        e.buffer.set_cursor(p(0, 0));
        assert!(!e.fold_current(), "之上无标题块 → 不假装支持");
    });
}

#[gpui::test]
fn fold_current_collapses_fenced_code_block(cx: &mut gpui::TestAppContext) {
    let ed = md_editor(cx, "intro\n```rust\nlet a = 1;\nlet b = 2;\n```\ntail");
    ed.update(cx, |e, _| {
        e.buffer.set_cursor(p(2, 0)); // 代码块内部
        assert!(e.fold_current());
        assert_eq!(e.display_map.fold_ranges(), vec![(1, 5)], "含开闭 ``` 行");
    });
}

#[gpui::test]
fn fold_current_refuses_plain_paragraph(cx: &mut gpui::TestAppContext) {
    let ed = md_editor(cx, "普通正文\n第二行\n\n第三段");
    ed.update(cx, |e, _| {
        e.buffer.set_cursor(p(0, 0));
        assert!(!e.fold_current(), "非标题非围栏 → 不可折叠（不假装支持）");
        assert!(e.display_map.fold_heads().is_empty());
    });
}

#[gpui::test]
fn unfold_current_restores_rows(cx: &mut gpui::TestAppContext) {
    let ed = md_editor(cx, "# A\npara1\npara2\n# B");
    ed.update(cx, |e, _| {
        e.buffer.set_cursor(p(0, 0));
        assert!(e.fold_current());
        assert_eq!(e.display_map.fold_ranges().len(), 1);
        assert!(e.unfold_current(), "展开当前");
        assert!(e.display_map.fold_ranges().is_empty());
    });
}

#[gpui::test]
fn fold_all_collapses_top_level_only(cx: &mut gpui::TestAppContext) {
    // A 小节含子标题 B：顶层只有 A（B 已含在 A 区间内）。
    let ed = md_editor(cx, "# A\n## B\nb-body\n# C\nc-body");
    ed.update(cx, |e, _| {
        let n = e.fold_all();
        assert_eq!(n, 2, "顶层两块：[A...]、[C...]");
        assert_eq!(e.display_map.fold_ranges(), vec![(0, 3), (3, 5)]);
    });
}

#[gpui::test]
fn unfold_all_clears_every_fold(cx: &mut gpui::TestAppContext) {
    let ed = md_editor(cx, "# A\nbody\n# B\nbody2");
    ed.update(cx, |e, _| {
        e.fold_all();
        assert_eq!(e.display_map.fold_ranges().len(), 2);
        assert!(e.unfold_all(), "全部展开返回 true");
        assert!(e.display_map.fold_ranges().is_empty());
        assert!(!e.unfold_all(), "已无折叠 → false（幂等）");
    });
}

#[gpui::test]
fn fold_then_restore_by_head_lines(cx: &mut gpui::TestAppContext) {
    // 持久化语义：存头行号 → 按当前文本重算区间尾（restore_folds 的
    // 单步切片——整链落盘/读回在 settings.rs folds_roundtrip_per_file）。
    let ed = md_editor(cx, "# A\nbody\n# B\nbody2");
    let heads = ed.update(cx, |e, _| {
        e.fold_all();
        e.display_map.fold_heads()
    });
    assert_eq!(heads, vec![0, 2]);
    // 模拟「重启」：新编辑器载同一文本，按头行恢复。
    let ed2 = md_editor(cx, "# A\nbody\n# B\nbody2");
    ed2.update(cx, |e2, _| {
        for h in &heads {
            if let Some((s, en)) = e2.foldable_span_at(*h) {
                e2.display_map
                    .fold(&e2.buffer, Point::new(s, 0)..Point::new(en, 0));
            }
        }
    });
    let (a, b) = (
        ed.update(cx, |e, _| e.display_map.fold_ranges()),
        ed2.update(cx, |e, _| e.display_map.fold_ranges()),
    );
    assert_eq!(b, a, "按头行恢复应得到相同折叠集");
}

#[gpui::test]
fn restore_drops_heads_that_no_longer_fold(cx: &mut gpui::TestAppContext) {
    // 头行漂移/不再是标题 → 该条丢弃（宁缺毋错）。
    let ed = md_editor(cx, "普通正文\n# 真标题\nbody");
    ed.update(cx, |e, _| {
        assert_eq!(e.foldable_span_at(0), None, "行 0 不是标题");
        assert_eq!(e.foldable_span_at(1), Some((1, 3)));
    });
}

#[gpui::test]
fn heading_fold_runs_to_document_end(cx: &mut gpui::TestAppContext) {
    // 末尾标题折叠到文末。
    let ed = md_editor(cx, "# A\nx\n# Z\nz1\nz2");
    ed.update(cx, |e, _| {
        e.buffer.set_cursor(p(2, 0));
        assert!(e.fold_current());
        assert_eq!(e.display_map.fold_ranges(), vec![(2, 5)], "到文末");
    });
}

// ------------------------------------------------------------------
// #169：Cmd::ALL 运行时执行面（真实 GPUI TestAppContext——实体/上下文
// 全真，无头窗口）。静态三方对账（枚举↔ALL↔执行臂+桩检测）在
// tests/command_coverage.rs；本组钉「每条都真的能跑且有可见效果」。
// ------------------------------------------------------------------

/// 运行时测试共用文档：两个同级一级标题（折叠域——fold_all 收同级
/// 块）+ 嵌套二级标题 + 多行正文（移动/选区域）。
const CMD_TEST_DOC: &str =
    "# 一级标题甲\n第一行正文内容\n# 一级标题乙\n第二行正文内容\n## 二级小节\n尾部正文一行\n";

/// 窗口宿主：Editor 非根视图（ShellApp 才 Render），测试以最小宿主
/// 开窗取真实 `Window`（exec_command 的焦点面需要——与 rpc.rs
/// `run_command` 同通道：`ed.update(cx, |ed, cx| ed.run_command(cmd, window, cx))`）。
struct EditorHost(gpui::Entity<Editor>);

impl gpui::Render for EditorHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div()
    }
}

/// 建编辑器 + 宿主窗口（返回编辑器实体与窗口句柄）。
fn editor_window(
    cx: &mut gpui::TestAppContext,
    path: Option<std::path::PathBuf>,
) -> (gpui::Entity<Editor>, gpui::WindowHandle<EditorHost>) {
    let ed = cx.update(|cx| cx.new(|cx| Editor::new(path, CMD_TEST_DOC.to_string(), cx)));
    let ed_for_host = ed.clone();
    let window = cx.add_window(move |_window, _cx| EditorHost(ed_for_host.clone()));
    (ed, window)
}

/// #281（审计 R-f）：字数缓存 revision 守卫——`char_count` 是全文
/// Rope→String 物化（O(doc) 分配），状态栏/属性面板每帧读；缓存键
/// = Buffer revision：未变更读命中（键值对不动）、变更事务后首读
/// 重算（值与键同步跟进）、纯光标移动零重算（revision 不变——
/// 结构性保证：缓存键等于当前 revision 即必命中）。
#[gpui::test]
fn char_count_cache_follows_revision(cx: &mut gpui::TestAppContext) {
    let (ed, _window) = editor_window(cx, None);
    // 初读：构造期 (u64::MAX, 0) 恒 miss → 填充。
    let n0 = cx.update(|cx| ed.read(cx).char_count());
    assert!(n0 > 0, "前置：测试文档非空");
    let (rev0, cached0) = cx.update(|cx| *ed.read(cx).char_count_cache.borrow());
    assert_eq!(cached0, n0, "缓存值 = 物化值");
    assert_eq!(
        rev0,
        cx.update(|cx| ed.read(cx).buffer.revision()),
        "缓存键对齐当前 revision"
    );
    // 未变更复读：命中（键值对不动——纯重渲/纯光标移动的帧级语义）。
    let _ = cx.update(|cx| ed.read(cx).char_label());
    let (rev1, cached1) = cx.update(|cx| *ed.read(cx).char_count_cache.borrow());
    assert_eq!((rev1, cached1), (rev0, cached0), "revision 未变零重算");
    // 变更事务（buffer 单点入口）：revision +1 → 首读重算跟进。
    let inserted = "追加的五个字";
    cx.update(|cx| {
        ed.update(cx, |ed, _| {
            let last = Point::new(ed.buffer.line_count() - 1, 0);
            assert!(ed.buffer.replace_range(last, last, inserted));
        })
    });
    let n2 = cx.update(|cx| ed.read(cx).char_count());
    assert_eq!(n2, n0 + inserted.chars().count(), "编辑后计数 = 原 + 插入");
    let (rev2, cached2) = cx.update(|cx| *ed.read(cx).char_count_cache.borrow());
    assert_eq!(cached2, n2, "缓存值跟进编辑后计数");
    assert_eq!(rev2, rev0 + 1, "缓存键跟进新 revision（恰 +1）");
    // char_label 行为面（状态栏消费口径）。
    let label = cx.update(|cx| ed.read(cx).char_label());
    assert!(
        label.contains(&format!("{n2} 字符")),
        "标签含最新计数：{label}"
    );
}

/// #169 2.1 全量执行：遍历 `Cmd::ALL`，每条在真实编辑器上执行一遍
/// （Save 走临时文件），断言全程不 panic——「面板只列可执行的」
/// 声称的运行时面。
#[gpui::test]
fn runtime_every_cmd_in_all_executes(cx: &mut gpui::TestAppContext) {
    let dir = std::env::temp_dir().join(format!("mdc-cmd-all-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cmds.md");
    std::fs::write(&path, CMD_TEST_DOC).unwrap();

    let (ed, window) = editor_window(cx, Some(path.clone()));

    // 顺序 = ALL 声明序；面板类命令（find/replace/goto）开后续命令
    // 不受干扰（exec 直达，不走键路由）；折叠类先 fold 后 unfold
    // 自愈；Save 无前置（未脏时写当前内容亦安全）。
    for &cmd in Cmd::ALL {
        window.update(cx, |_host, window, cx| {
            ed.update(cx, |ed, cx| ed.run_command(cmd, window, cx));
        });
    }

    // 收尾存活（缓冲仍可读 = 未 panic 未崩溃）+ 清理。
    window.update(cx, |_host, _window, cx| {
        ed.update(cx, |ed, _| {
            assert!(ed.buffer.cursor().line < ed.buffer.line_count());
        });
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// #169 2.1 状态变化断言——导航/选区类：光标真动、选区真选。
#[gpui::test]
fn runtime_navigation_and_selection_change_state(cx: &mut gpui::TestAppContext) {
    let (ed, window) = editor_window(cx, None);

    window.update(cx, |_host, window, cx| {
        ed.update(cx, |ed, cx| {
            ed.run_command(Cmd::DocStart, window, cx);
            assert_eq!(
                (ed.buffer.cursor().line, ed.buffer.cursor().col),
                (0, 0),
                "DocStart 应回文首"
            );
            ed.run_command(Cmd::DocEnd, window, cx);
            assert_eq!(
                ed.buffer.cursor().line,
                ed.buffer.line_count() - 1,
                "DocEnd 应到末行"
            );
            // 行内移动：回到首行行尾（非空行 col>0）→ MoveLeft 左移一格
            //（无条件断言——防条件跳过形成覆盖洞）。
            ed.run_command(Cmd::DocStart, window, cx);
            ed.run_command(Cmd::MoveEnd, window, cx);
            let col_end = ed.buffer.cursor().col;
            assert!(col_end > 0, "前置：首行（标题）非空——MoveEnd 后 col>0");
            ed.run_command(Cmd::MoveLeft, window, cx);
            assert_eq!(ed.buffer.cursor().col, col_end - 1, "MoveLeft 应左移一格");
            // 下移/上移往返。
            ed.run_command(Cmd::DocStart, window, cx);
            ed.run_command(Cmd::MoveDown, window, cx);
            assert_eq!(ed.buffer.cursor().line, 1, "MoveDown 应下移一行");
            ed.run_command(Cmd::MoveUp, window, cx);
            assert_eq!(ed.buffer.cursor().line, 0, "MoveUp 应上移一行");

            // 全选：选区文本 = 全文（SelectAll 真生效）。
            ed.run_command(Cmd::SelectAll, window, cx);
            assert_eq!(ed.buffer.selection_text(), CMD_TEST_DOC.to_string());
        });
    });
}

/// #169 2.1 状态变化断言——折叠/面板/撤销/保存类：每类一个可见效果。
#[gpui::test]
fn runtime_fold_panels_undo_save_change_state(cx: &mut gpui::TestAppContext) {
    let dir = std::env::temp_dir().join(format!("mdc-cmd-cls-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("edit.md");
    std::fs::write(&path, CMD_TEST_DOC).unwrap();

    let (ed, window) = editor_window(cx, Some(path.clone()));

    // ① 折叠：FoldAll 收起两个同级一级标题块 → heads=2；UnfoldAll → 0
    //    （二级小节嵌套在一级块内，不另立 head——heading_span 语义）。
    window.update(cx, |_host, window, cx| {
        ed.update(cx, |ed, cx| {
            ed.run_command(Cmd::FoldAll, window, cx);
            assert_eq!(
                ed.display_map.fold_heads().len(),
                2,
                "FoldAll 应收起两个一级标题块"
            );
            ed.run_command(Cmd::UnfoldAll, window, cx);
            assert_eq!(ed.display_map.fold_heads().len(), 0, "UnfoldAll 应全部展开");
        });
    });

    // ② 面板：OpenFind / OpenReplace / GotoLine 打开对应态。
    window.update(cx, |_host, window, cx| {
        ed.update(cx, |ed, cx| {
            ed.run_command(Cmd::OpenFind, window, cx);
            assert!(ed.find.is_some(), "OpenFind 应打开查找条");
            ed.run_command(Cmd::OpenReplace, window, cx);
            assert!(ed.find.is_some(), "OpenReplace 应打开替换条（find 态承载）");
            ed.run_command(Cmd::GotoLine, window, cx);
            assert!(ed.goto.is_some(), "GotoLine 应打开跳行框");
        });
    });

    // ③ 撤销/重做：真实编辑 → Undo 复原 → Redo 重做。
    // ④ 保存：脏内容落盘（文件 == 当前文本）。
    window.update(cx, |_host, window, cx| {
        ed.update(cx, |ed, cx| {
            let before = ed.buffer.to_plain_text();
            ed.insert_text_at(0, 0, "X", cx);
            assert_ne!(ed.buffer.to_plain_text(), before, "前置：插入应改变文本");
            ed.run_command(Cmd::Undo, window, cx);
            assert_eq!(ed.buffer.to_plain_text(), before, "Undo 应复原插入");
            ed.run_command(Cmd::Redo, window, cx);
            assert_ne!(ed.buffer.to_plain_text(), before, "Redo 应重做插入");

            let now = ed.buffer.to_plain_text();
            ed.run_command(Cmd::Save, window, cx);
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                now,
                "Save 应把当前文本落盘"
            );
        });
    });

    let _ = std::fs::remove_dir_all(&dir);
}
