//! #163 单源原语与行映射验收——围栏/标题单源化 + parse_with_lines 几何。
//!
//! 覆盖面：
//! - `fence_info`：``` / ~~~ / 前导空白 / lang 首词（渲染面语义对齐）；
//! - `heading_level`：`#` 独占行空标题（编辑器/CommonMark 超集）；
//! - `classify_line`：分支分发一致性；
//! - `parse_with_lines`：slot 几何（prefix/tail/text_off/text_len）与
//!   树先序对齐——mang-shell 换芯桥的正确性根基；
//! - `History::coalesce_ms`：实例级合并窗（0 = 禁用）+ 旧档 serde 兼容；
//! - `Core::Clone`：深拷贝语义。

use mdcore::markdown::{
    classify_line, fence_info, heading_level, parse_with_lines, LineClass, SlotClass,
};
use mdcore::op::Op;
use mdcore::{ApplyMeta, Core};

// ---------- fence_info（渲染面语义对齐） ----------

#[test]
fn fence_info_backtick_and_tilde() {
    let f = fence_info("```rust").expect("```rust");
    assert_eq!(f.fence_char, '`');
    assert_eq!(f.lang, "rust");
    assert_eq!(f.marker_end, 3, "标记结束位 = 3 个反引号后");
    let t = fence_info("~~~js").expect("~~~js");
    assert_eq!(t.fence_char, '~');
    assert_eq!(t.lang, "js");
}

#[test]
fn fence_info_leading_spaces_upto_three() {
    assert!(fence_info("   ```").is_some(), "≤3 前导空白是围栏");
    assert!(
        fence_info("    ```").is_none(),
        "4 空格非围栏（缩进代码语义面）"
    );
}

#[test]
fn fence_info_lang_first_word_only() {
    let f = fence_info("```rust, ignore extra").expect("lang 首词");
    assert_eq!(
        f.lang, "rust",
        "lang 收窄为首词（与编辑器 fence_lang_of 同口径）"
    );
}

#[test]
fn fence_info_requires_three() {
    assert!(fence_info("``").is_none());
    assert!(fence_info("~~").is_none());
    assert!(fence_info("````").is_some(), "≥3 均可");
    assert!(fence_info("`x`").is_none(), "单反引号行内代码不是围栏");
    assert!(
        fence_info("``` ```").is_none(),
        "info 含围栏字符非围栏（CommonMark）"
    );
}

// ---------- 解析器围栏扩展（渲染面对齐——漂移消灭的实证） ----------

#[test]
fn parser_accepts_tilde_fence() {
    let (tree, slots) = parse_with_lines("~~~\nabc\n~~~\n");
    assert_eq!(tree.top_level().len(), 1, "~~~ 围栏成块");
    let node = tree.node(tree.top_level()[0]);
    assert_eq!(node.text.to_string(), "abc");
    assert_eq!(slots[0].class, SlotClass::MarkerOnly);
    assert_eq!(slots[1].class, SlotClass::BodyCode);
    assert_eq!(slots[2].class, SlotClass::MarkerOnly);
}

#[test]
fn parser_accepts_indented_fence_open() {
    let (tree, _) = parse_with_lines("   ```rust\nx\n```\n");
    assert_eq!(tree.top_level().len(), 1, "≤3 前导空白围栏开行成块");
    let node = tree.node(tree.top_level()[0]);
    assert_eq!(
        node.attrs.get("lang").map(String::as_str),
        Some("rust"),
        "lang 语义保留"
    );
}

#[test]
fn parser_closes_fence_on_any_marker() {
    // 渲染面翻转语义：``` 开栏，~~~ 也闭栏（编辑器 FenceCursor 同款）。
    let (tree, _) = parse_with_lines("```\ncode\n~~~\nafter");
    assert_eq!(tree.top_level().len(), 2, "~~~ 闭栏，after 是独立段");
}

// ---------- heading_level（单源） ----------

#[test]
fn heading_level_bare_hash_is_empty_heading() {
    assert_eq!(heading_level("#"), Some(1), "# 独占行 = 空标题（超集）");
    assert_eq!(heading_level("# "), Some(1));
    assert_eq!(heading_level("###### x"), Some(6));
    assert_eq!(heading_level("####### x"), None, "7 个 # 非标题");
    assert_eq!(heading_level("#x"), None, "无空格非标题");
}

#[test]
fn parser_bare_hash_heading_slot() {
    let (tree, slots) = parse_with_lines("#\n# x\n");
    assert_eq!(tree.top_level().len(), 2);
    assert_eq!(slots[0].class, SlotClass::Heading(1));
    assert_eq!(slots[0].prefix, 1, "空标题前缀 = 单 #（无空格）");
    assert_eq!(slots[0].text_len, 0);
    assert_eq!(slots[1].prefix, 2, "\"# x\" 前缀 = # + 空格");
    assert_eq!(slots[1].text_len, 1);
}

// ---------- classify_line ----------

#[test]
fn classify_line_dispatch() {
    assert_eq!(classify_line("```x"), LineClass::Fence);
    assert_eq!(classify_line("  ~~~"), LineClass::Fence);
    assert_eq!(classify_line("$$"), LineClass::MathFence);
    assert_eq!(classify_line("$$x$$"), LineClass::MathInline);
    assert_eq!(classify_line(""), LineClass::Blank);
    assert_eq!(classify_line("  "), LineClass::Blank);
    assert_eq!(classify_line("## h"), LineClass::Heading(2));
    assert_eq!(classify_line("---"), LineClass::Thematic);
    assert_eq!(classify_line("|a|b|"), LineClass::TableRow);
    assert_eq!(classify_line("> q"), LineClass::Quote(1));
    assert_eq!(classify_line(">> q"), LineClass::Quote(2));
    assert_eq!(classify_line("- i"), LineClass::ListItem(1));
    assert_eq!(classify_line("  - i"), LineClass::ListItem(2));
    assert_eq!(classify_line("plain"), LineClass::Paragraph);
}

// ---------- parse_with_lines 几何 ----------

#[test]
fn slots_geometry_paragraph_heading() {
    let (_, slots) = parse_with_lines("hello\n## title\n\nworld");
    assert_eq!(slots.len(), 4, "slot 逐行产出（含空行）");
    assert_eq!(slots[0].block, Some(0));
    assert_eq!(slots[0].class, SlotClass::Paragraph);
    assert_eq!(slots[0].prefix, 0);
    assert_eq!(slots[0].text_len, 5);
    assert_eq!(slots[1].class, SlotClass::Heading(2));
    assert_eq!(slots[1].prefix, 3);
    assert_eq!(slots[1].text_len, 5);
    assert_eq!(slots[2].class, SlotClass::Blank);
    assert_eq!(slots[2].block, None);
    assert_eq!(slots[3].block, Some(2));
}

#[test]
fn slots_geometry_quote_and_list_prefix() {
    let (_, slots) = parse_with_lines("> quoted\n- item\n  - nested");
    // > quoted → quote(0) 容器 + 段落子块(1)；行 slot 指向段落子块。
    assert_eq!(slots[0].block, Some(1));
    assert_eq!(slots[0].class, SlotClass::Quote(1));
    assert_eq!(slots[0].prefix, 2, "\"> \" 前缀");
    assert_eq!(slots[0].text_len, 6);
    // - item → ListItem(2)；前缀 = "- "。
    assert_eq!(slots[1].block, Some(2));
    assert_eq!(slots[1].class, SlotClass::ListItem(1));
    assert_eq!(slots[1].prefix, 2);
    // 嵌套列表项前缀 = 缩进 + 标记 + 空格。
    assert_eq!(slots[2].block, Some(3));
    assert_eq!(slots[2].class, SlotClass::ListItem(2));
    assert_eq!(slots[2].prefix, 4);
}

#[test]
fn slots_geometry_fence_body_offsets() {
    let (_, slots) = parse_with_lines("```rs\nab\ncd\n```\ntail");
    assert_eq!(slots[0].class, SlotClass::MarkerOnly);
    assert_eq!(slots[0].prefix, 5, "开行 prefix = 整行");
    assert_eq!(slots[1].class, SlotClass::BodyCode);
    assert_eq!(slots[1].text_off, 0);
    assert_eq!(slots[1].text_len, 2);
    assert_eq!(slots[2].text_off, 3, "体行连加：ab + \\n");
    assert_eq!(slots[2].text_len, 2);
    assert_eq!(slots[3].class, SlotClass::MarkerOnly);
    assert_eq!(slots[4].class, SlotClass::Paragraph);
}

#[test]
fn slots_geometry_math_inline_delimiters() {
    let (_, slots) = parse_with_lines("  $$E=mc^2$$  ");
    assert_eq!(slots[0].class, SlotClass::MathInline);
    assert_eq!(slots[0].prefix, 4, "2 空白 + 开界 $$");
    assert_eq!(slots[0].tail, 4, "收界 $$ + 2 空白");
    assert_eq!(slots[0].text_len, 6);
}

#[test]
fn slots_geometry_table_rows() {
    let (_, slots) = parse_with_lines("|a|b|\n|c|d|\npara");
    assert_eq!(slots[0].class, SlotClass::TableRow);
    assert_eq!(slots[0].text_off, 0);
    assert_eq!(slots[1].text_off, 6, "行 0 长(5) + \\n");
    assert_eq!(slots[2].class, SlotClass::Paragraph);
    assert_eq!(slots[2].block, Some(1));
}

#[test]
fn slots_block_index_is_preorder() {
    // 创建序 == 先序 == 文档序：quote 容器(0) > 段落(1)，再根段落(2)。
    let (tree, slots) = parse_with_lines("> a\nb");
    let ids = tree.preorder_block_ids();
    assert_eq!(ids.len(), 3, "quote + 段落子块 + 根段落");
    assert_eq!(
        slots[0].block,
        Some(1),
        "行 slot 指向段落子块（容器无独立行）"
    );
    assert_eq!(slots[1].block, Some(2));
    // 同一 slot 的块可通过先序表 + id_index 直接定位。
    let para_id = ids[1];
    let node_id = tree.lookup_id(para_id).expect("id 可寻");
    assert_eq!(tree.node(node_id).kind, mdcore::tree::BlockKind::Paragraph);
}

// ---------- History 合并窗实例化 ----------

#[test]
fn coalesce_window_zero_disables_merging() {
    let mut core = Core::from_markdown("hello");
    core.set_coalesce_ms(0);
    for _ in 0..3 {
        core.apply_ops(
            vec![Op::ReplaceText {
                path: vec![0],
                start: 5,
                end: 5,
                insert: "x".into(),
            }],
            ApplyMeta::view("test"),
        )
        .unwrap();
    }
    assert_eq!(core.undo_depth(), 3, "禁合并：3 事务 3 条目");
}

#[test]
fn coalesce_window_old_history_json_compat() {
    // 旧档（无 coalesce_ms 字段）反序列化 → 缺省 1000。
    let old = r#"{
        "format_version": 2,
        "undo": [],
        "redo": [],
        "last_at_ms": 0
    }"#;
    let hist = mdcore::history::History::from_json(old).expect("旧档可读");
    let json = hist.to_json();
    assert!(json.contains("coalesce_ms"), "新档写出窗宽字段: {json}");
}

#[test]
fn core_clone_is_full_snapshot() {
    let mut core = Core::from_markdown("a\n\nb");
    core.apply_ops(
        vec![Op::ReplaceText {
            path: vec![0],
            start: 0,
            end: 0,
            insert: "x".into(),
        }],
        ApplyMeta::view("t"),
    )
    .unwrap();
    let mut snapshot = core.clone();
    core.undo();
    assert_eq!(core.undo_depth(), 0);
    assert_eq!(snapshot.undo_depth(), 1, "克隆体持有完整历史");
    // 克隆体可独立 undo（互不影响源）。
    assert!(snapshot.undo().is_some());
    assert_eq!(core.undo_depth(), 0, "源的栈不受克隆体 undo 影响");
}

#[test]
fn core_clear_redo_and_last_undo_ops() {
    let mut core = Core::from_markdown("hello");
    core.set_coalesce_ms(0);
    core.apply_ops(
        vec![Op::ReplaceText {
            path: vec![0],
            start: 0,
            end: 0,
            insert: "x".into(),
        }],
        ApplyMeta::view("t"),
    )
    .unwrap();
    assert_eq!(core.last_undo_ops_count(), 1);
    core.undo();
    assert_eq!(core.redo_depth(), 1);
    core.clear_redo();
    assert_eq!(core.redo_depth(), 0);
    assert_eq!(core.last_undo_ops_count(), 0, "栈空 = 0");
}
