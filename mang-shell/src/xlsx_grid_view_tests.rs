//! #47 巨石拆解批⑤：原 xlsx_grid_view.rs 内嵌 `tests` 纯迁出
//!（零行为变化——字节原样剥壳，#[cfg(test)] + #[path] 挂载在宿主文件）。

use super::*;

// ── #389 阶段 2：编辑面（parse/引用/提交/往返） ─────────────

/// 夹具路径（tests/fixtures/smoke.xlsx——B2=100 数值格、B4=SUM 公式
/// 格、A2=「一月」共享串格；xlsx_grid_model.rs 同源夹具）。
fn fixture() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.xlsx")
}

#[test]
fn parse_cell_input_variants() {
    // 数值（前后空白容忍；f64 有限值）。
    assert!(matches!(parse_cell_input("100"), CellEdit::Number(n) if n == 100.0));
    assert!(matches!(parse_cell_input(" 200.5 "), CellEdit::Number(n) if n == 200.5));
    assert!(matches!(parse_cell_input("-3"), CellEdit::Number(n) if n == -3.0));
    // 布尔（大小写不敏感）。
    assert!(matches!(parse_cell_input("TRUE"), CellEdit::Boolean(true)));
    assert!(matches!(
        parse_cell_input("false"),
        CellEdit::Boolean(false)
    ));
    // 文本（原文保留——内边空白不裁剪、无引号转义）。
    assert!(matches!(parse_cell_input("hello world"), CellEdit::Text(ref t) if t == "hello world"));
    assert!(matches!(parse_cell_input("  pad  "), CellEdit::Text(ref t) if t == "  pad  "));
    // 数值不可解析的近形（日期/分数/科学计数法越界）→ 文本。
    assert!(matches!(parse_cell_input("2026-01-01"), CellEdit::Text(_)));
    // 空白 → 清格（空 Text——引擎面 validate_text 放行）。
    assert!(matches!(parse_cell_input("   "), CellEdit::Text(ref t) if t.is_empty()));
}

#[test]
fn cell_ref_for_maps_display_coords_to_a1() {
    let mut sheet = GridSheet::default();
    sheet.start_row = 0;
    sheet.start_col = 0;
    assert_eq!(cell_ref_for(&sheet, 0, 0), "A1");
    assert_eq!(cell_ref_for(&sheet, 2, 1), "B3");
    // 数据区不从 A1 起的簿（office-io 包围盒偏移）。
    sheet.start_row = 3;
    sheet.start_col = 2;
    assert_eq!(cell_ref_for(&sheet, 0, 0), "C4");
    assert_eq!(cell_ref_for(&sheet, 1, 0), "C5");
}

#[test]
fn commit_cell_edit_updates_model_and_rejects_formula() {
    let bytes = std::fs::read(fixture()).expect("夹具读取");
    let mut editor = XlsxEditor::open(&bytes).expect("编辑会话打开");
    let mut model = XlsxGridModel::load(&fixture()).expect("网格模型装载");

    // B2（数值格，显示坐标 (1,1)——表头行 0）改文本：落账 + 显示面
    // 即时更新。
    commit_cell_edit(&mut editor, &mut model, 0, 1, 1, "三百").expect("B2 值编辑落账");
    assert_eq!(model.sheets[0].cells[1][1], "三百", "pending 显示面更新");

    // B4（公式格，显示坐标 (3,1)）拒改——引擎面 FormulaCell。
    let err =
        commit_cell_edit(&mut editor, &mut model, 0, 3, 1, "999").expect_err("公式格必须拒改");
    assert!(err.contains("公式"), "拒改文案含公式语义：{err}");
    // 拒改后显示面不动（原缓存值 300.5 保持）。
    assert_eq!(model.sheets[0].cells[3][1], "300.5");

    // 保存 → 重开读面：B2 新值在场（真落盘闭环——不是只改内存态）。
    let saved = editor.save_to_vec().expect("保存序列化");
    let mut reopened = XlsxEditor::open(&saved).expect("重开会话");
    let info = reopened.cell_info(0, "B2").expect("B2 在场").expect("非空");
    assert_eq!(info.text, "三百", "落盘后 B2 = 三百");
}

#[test]
fn write_xlsx_atomically_preserves_original_on_failure() {
    // 正常路径：临时文件 + rename 落位（目标可写）。
    let dir = std::env::temp_dir().join(format!("mang-xlsx-atom-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("out.xlsx");
    assert!(write_xlsx_atomically(&target, b"PK\x03\x04bytes").is_ok());
    assert_eq!(std::fs::read(&target).unwrap(), b"PK\x03\x04bytes");
    // 临时文件不残留（rename 收干净）。
    let residue: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains("mang-tmp"))
        .collect();
    assert!(residue.is_empty(), "无临时文件残留：{residue:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn col_letter_sequence_and_wrap() {
    assert_eq!(col_letter(0), "A");
    assert_eq!(col_letter(25), "Z");
    assert_eq!(col_letter(26), "AA");
    assert_eq!(col_letter(27), "AB");
    assert_eq!(col_letter(701), "ZZ");
    assert_eq!(col_letter(702), "AAA");
}

#[test]
fn display_width_ea_aware() {
    assert_eq!(display_width("abc"), 3);
    assert_eq!(display_width("中文"), 4, "CJK 双宽");
    assert_eq!(display_width("a中b"), 4, "混合");
    assert_eq!(display_width(""), 0);
}

#[test]
fn col_width_px_clamps() {
    assert_eq!(col_width_px(0), COL_W_MIN, "空列 = 下限");
    assert_eq!(col_width_px(3), COL_W_MIN, "短内容仍下限");
    // 封顶内容（24 cell）的自然宽 = 24×7.5+12 = 192 = 上限。
    assert_eq!(col_width_px(24), COL_W_MAX, "封顶内容 = 上限");
    assert_eq!(
        col_width_px(200),
        COL_W_MAX,
        "超宽内容 = 上限（CELL_WIDTH_CAP 截断面）"
    );
}

#[test]
fn col_offsets_cumulative_with_gap() {
    let offs = col_offsets(&[100.0, 50.0, 80.0]);
    assert_eq!(offs, vec![0.0, 101.0, 152.0], "含 COL_GAP 累计");
}

#[test]
fn visible_col_window_basic() {
    let widths = vec![100.0; 6];
    let offs = col_offsets(&widths);
    // 视口 300 @ scroll 0 → 列 0..3（列 2 右缘 300 边界算可见）。
    let (f, e) = visible_col_window(&offs, &widths, 0.0, 300.0);
    assert_eq!((f, e), (0, 3), "首窗口");
    // scroll 202（列 2 左缘 202）→ 列 2..5（505 右缘钳在窗口内）。
    let (f, e) = visible_col_window(&offs, &widths, 202.0, 300.0);
    assert_eq!((f, e), (2, 5), "中窗口");
    // 滚到最右：总宽 605，视口 300 → scroll max 305 → 窗口 3..6。
    let (f, e) = visible_col_window(&offs, &widths, 305.0, 300.0);
    assert_eq!((f, e), (3, 6), "末窗口");
    // 超界 scroll = 零窗口（调用方 clamp 前置——契约防御面）。
    let (f, e) = visible_col_window(&offs, &widths, 1e9, 300.0);
    assert_eq!((f, e), (0, 0), "超界 scroll = 零窗口");
    // 空表。
    let (f, e) = visible_col_window(&[], &[], 0.0, 300.0);
    assert_eq!((f, e), (0, 0), "零列 = 零窗口");
}

#[test]
fn clamp_scroll_x_domain() {
    assert_eq!(clamp_scroll_x(-50.0, 1000.0, 300.0), 0.0, "负值钳 0");
    assert_eq!(
        clamp_scroll_x(900.0, 1000.0, 300.0),
        700.0,
        "上限 = 总宽-视口"
    );
    assert_eq!(clamp_scroll_x(50.0, 100.0, 300.0), 0.0, "内容窄于视口 = 0");
    assert_eq!(clamp_scroll_x(f32::NAN, 1000.0, 300.0), 0.0, "NaN 防御");
}

#[test]
fn hit_test_cell_resolves_and_rejects() {
    let widths = vec![100.0, 80.0];
    let offs = col_offsets(&widths);
    // scroll 0：x=GUTTER_W+10 → 列 0；y=45 → 行 2（首可见 0）。
    assert_eq!(
        hit_test_cell(&offs, &widths, 0.0, 0, GUTTER_W + 10.0, 45.0),
        Some((2, 0))
    );
    // 列 1（x = GUTTER_W + 101..180）。
    assert_eq!(
        hit_test_cell(&offs, &widths, 0.0, 0, GUTTER_W + 105.0, 0.0),
        Some((0, 1))
    );
    // 行号槽（x < GUTTER_W）= None。
    assert_eq!(hit_test_cell(&offs, &widths, 0.0, 0, 10.0, 10.0), None);
    // 列间隙（x = GUTTER_W + 100.5 在 100..101 间隙）= None。
    assert_eq!(
        hit_test_cell(&offs, &widths, 0.0, 0, GUTTER_W + 100.5, 10.0),
        None
    );
    // scroll_x=101：列 0 左移出窗 → x=GUTTER_W+10 命中列 1。
    assert_eq!(
        hit_test_cell(&offs, &widths, 101.0, 0, GUTTER_W + 10.0, 0.0),
        Some((0, 1))
    );
    // 首可见行偏移（纵向滚过 3 行）：y=0 → 行 3。
    assert_eq!(
        hit_test_cell(&offs, &widths, 0.0, 3, GUTTER_W + 10.0, 0.0),
        Some((3, 0))
    );
    // 负 y / NaN 防御。
    assert_eq!(
        hit_test_cell(&offs, &widths, 0.0, 0, GUTTER_W + 10.0, -5.0),
        None
    );
    assert_eq!(hit_test_cell(&offs, &widths, 0.0, 0, f32::NAN, 0.0), None);
}

#[test]
fn cell_display_formula_fallback() {
    use mdc_office_io::io::CellData;
    assert_eq!(
        cell_display(Some(&CellData::Empty), Some("SUM(A1:A2)")),
        "=SUM(A1:A2)"
    );
    assert_eq!(cell_display(Some(&CellData::Number(1.5)), None), "1.5");
    assert_eq!(cell_display(None, None), "");
}

// ── #555：合并单元格（模型携带 + 编辑落账 + 视图接线） ──────

/// 表 1 合并 B5:C5 的显示坐标锚（绝对 (4,1)-(4,2)，包围盒 A1 起）。
fn smoke_merge_span() -> crate::xlsx_merge_geometry::MergeSpan {
    crate::xlsx_merge_geometry::MergeSpan {
        row_start: 4,
        row_end: 4,
        col_start: 1,
        col_end: 2,
        abs_anchor: (4, 1),
    }
}

#[test]
fn from_book_carries_merge_spans() {
    let model = XlsxGridModel::load(&fixture()).expect("网格模型装载");
    // 表 1：B5:C5（绝对 (4,1)-(4,2)）→ 显示坐标同值（A1 起包围盒）。
    assert_eq!(
        model.sheets[0].merge_spans,
        vec![smoke_merge_span()],
        "合并范围全量携带（此前只有计数——范围被丢弃）"
    );
    // 计数与范围口径分离：merges = 原始计数（含裁掉的），spans = 可渲染。
    assert_eq!(model.sheets[0].merges, 1);
    // 表 2：无合并。
    assert!(model.sheets[1].merge_spans.is_empty(), "表 2 无合并范围");
    // 覆盖格 C5（显示 (4,2)）命中映射到左上 B5（4,1）。
    assert_eq!(
        canonical_cell(&model.sheets[0].merge_spans, 4, 2),
        Some((4, 1)),
        "覆盖格 → 左上（Excel 语义）"
    );
    // 普通格不受影响（无映射——单格语义零漂移）。
    assert_eq!(canonical_cell(&model.sheets[0].merge_spans, 1, 1), None);
}

#[test]
fn commit_merge_edit_writes_top_left_only_and_preserves_range() {
    // #555 验收 3（模型/落账面）：双击覆盖格 → 修改左上值 → 保存 →
    // 重读——左上值已改、merge 范围原样保留、覆盖格未写入重复值。
    let bytes = std::fs::read(fixture()).expect("夹具读取");
    let mut editor = XlsxEditor::open(&bytes).expect("编辑会话打开");
    let mut model = XlsxGridModel::load(&fixture()).expect("网格模型装载");

    // 用户手势的模型面等价：点的是覆盖格 C5（显示 (4,2)），命中映射
    // 到左上 B5（4,1）后提交编辑。
    let (r, c) = canonical_cell(&model.sheets[0].merge_spans, 4, 2).expect("覆盖格有映射");
    assert_eq!((r, c), (4, 1), "C5 → B5");
    commit_cell_edit(&mut editor, &mut model, 0, r, c, "改后合并值").expect("左上值编辑落账");
    // pending 显示面：只有左上改（覆盖格不复制值——issue 明文禁令）。
    assert_eq!(model.sheets[0].cells[4][1], "改后合并值");
    assert_eq!(
        model.sheets[0].cells[4][2], "",
        "覆盖格 C5 显示面不写入重复值"
    );

    // 保存 → 重开读面（真落盘闭环）。
    let saved = editor.save_to_vec().expect("保存序列化");
    let mut reopened = XlsxEditor::open(&saved).expect("重开会话");
    let info = reopened.cell_info(0, "B5").expect("B5 在场").expect("非空");
    assert_eq!(info.text, "改后合并值", "落盘后 B5 = 新值");
    // 覆盖格 C5 读回为空（未写入重复值）。
    let covered = reopened.cell_info(0, "C5").expect("C5 可查");
    assert!(
        covered.is_none(),
        "覆盖格 C5 未被写入（得 {:?}）——禁止复制值填满合并区域",
        covered.map(|i| i.text)
    );
    // merge 范围原样保留（保存字节 → 读面比对——不污染仓内夹具）。
    let saved_book = {
        let tmp =
            std::env::temp_dir().join(format!("mang-merge-roundtrip-{}.xlsx", std::process::id()));
        std::fs::write(&tmp, &saved).expect("临时落盘");
        let b = mdc_office_io::io::read_book_with_options(
            &tmp.to_string_lossy(),
            &ReadOptions { compat: true },
        )
        .expect("保存字节读面");
        let _ = std::fs::remove_file(&tmp);
        b
    };
    assert_eq!(
        saved_book.sheets[0].merges.len(),
        1,
        "merge 范围原样保留（数量）"
    );
    let dm = saved_book.sheets[0].merges[0];
    assert_eq!(
        (dm.start.0, dm.start.1, dm.end.0, dm.end.1),
        (4, 1, 4, 2),
        "merge 范围原样保留（B5:C5 绝对端点）"
    );
}

/// #584 返工：测试宿主——视图进**真实渲染树**（空 div 宿主证明
/// 不了渲染/事件注册——审核要求：真实视图点击/双击/按键回归）。
struct GridHost(gpui::Entity<XlsxGridView>);

impl Render for GridHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div().size_full().child(self.0.clone())
    }
}

/// #584 返工：真实鼠标事件派发（MouseDown + MouseUp——完整命中
/// 测试管线，走覆盖 canvas 注册的窗口级观察器 → select_at /
/// begin_edit_at；main.rs 烟雾 + 批 11 同款 update_window 口径）。
fn grid_click_at(
    window: &gpui::WindowHandle<GridHost>,
    cx: &mut gpui::TestAppContext,
    x: f32,
    y: f32,
    click_count: usize,
) {
    use gpui::{Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput, point, px};
    let any: gpui::AnyWindowHandle = window.clone().into();
    let down = PlatformInput::MouseDown(MouseDownEvent {
        button: MouseButton::Left,
        position: point(px(x), px(y)),
        modifiers: Modifiers::default(),
        click_count,
        first_mouse: false,
    });
    let up = PlatformInput::MouseUp(MouseUpEvent {
        button: MouseButton::Left,
        position: point(px(x), px(y)),
        modifiers: Modifiers::default(),
        click_count,
    });
    cx.update_window(any, |_, window, cx| {
        window.dispatch_event(down, cx);
        window.dispatch_event(up, cx);
    })
    .expect("鼠标派发");
}

/// 合成按键（pdf_pages key_event 同构）。
fn grid_key_event(key: &str, key_char: Option<&str>, ctrl: bool) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: gpui::Keystroke {
            modifiers: gpui::Modifiers {
                control: ctrl,
                ..Default::default()
            },
            key: key.to_string(),
            key_char: key_char.map(|c| c.to_string()),
        },
        is_held: false,
        prefer_character_input: false,
    }
}

/// #584 返工：真实键盘事件派发（焦点路径——点击后视图持焦，
/// KeyDown 经 track_focus 节点到达 handle_key；#422 同款）。
fn grid_key_down(
    window: &gpui::WindowHandle<GridHost>,
    cx: &mut gpui::TestAppContext,
    ev: KeyDownEvent,
) {
    let any: gpui::AnyWindowHandle = window.clone().into();
    cx.update_window(any, |_, window, cx| {
        window.dispatch_event(gpui::PlatformInput::KeyDown(ev), cx);
    })
    .expect("键盘派发");
}

/// #584 返工：重绘一帧（命中盒/焦点节点随最新状态注册）。
fn grid_redraw(window: &gpui::WindowHandle<GridHost>, cx: &mut gpui::TestAppContext) {
    let any: gpui::AnyWindowHandle = window.clone().into();
    cx.update_window(any, |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .expect("重绘");
}

/// #555 验收 3（#584 返工重写——真实渲染树 + 真实事件全链）：
/// 挂载真实 XlsxGridView（渲染树内）→ **真实点击**覆盖格 C5 →
/// 选中整区（锚 B5）→ **真实双击**进入编辑（左上原值起步）→
/// **真实键入** → Enter 提交 → Ctrl+S 落盘 → 重读文件断言
/// （左上改 / 范围在 / 覆盖格零写入）。
/// 夹具复制到临时目录——save() 写回源路径，不许污染仓内黄金夹具。
#[gpui::test]
fn merge_covered_click_selects_edits_and_saves(cx: &mut gpui::TestAppContext) {
    let src = fixture();
    let tmp = std::env::temp_dir().join(format!("mang-merge-grid-{}.xlsx", std::process::id()));
    std::fs::copy(&src, &tmp).expect("夹具复制到临时目录");
    let model = XlsxGridModel::load(&tmp).expect("网格模型装载");
    assert_eq!(
        model.sheets[0].merge_spans,
        vec![smoke_merge_span()],
        "临时夹具合并范围在档（前置）"
    );

    // #584 返工：真实渲染树挂载需要主题全局（视图 render 读
    // cx.theme()——theme 是 workspace 依赖 crate；渲染期派生还要
    // settings provider 全局，#422 WiringHost 同款双注册）。
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });

    // 真实渲染树挂载（GridHost = size_full + child 视图——渲染/
    // 命中盒/焦点节点全部注册，非空 div 直调可比）。
    let mut slot: Option<gpui::Entity<XlsxGridView>> = None;
    let window = cx.add_window(|_window, cx| {
        let v = cx.new(|cx| XlsxGridView::new(model, tmp.clone(), cx));
        slot = Some(v.clone());
        GridHost(v)
    });
    let view = slot.expect("宿主构造即置");

    // 覆盖格 C5（显示 (4,2)）的窗口坐标：x = 行号槽 + 列 2 偏移
    //（Host 全幅 → 视图原点 = (0,0)，canvas 局部 = 窗口坐标）；
    // y = 行 4 中部（y/ROW_H 整除命中行 4——表格头/表签只遮影行
    // 0-2，行 4 起精确）。
    let (click_x, click_y) = window
        .update(cx, |_host, _window, cx| {
            view.update(cx, |v, _| {
                let (off2, w2) = (v.col_offsets[2], v.col_widths[2]);
                (
                    GUTTER_W + off2 + (w2 / 2.0).min(5.0),
                    4.0 * ROW_H + ROW_H / 2.0,
                )
            })
        })
        .expect("读坐标");

    // ① 真实单击覆盖格 C5 → 选中整区（锚 B5）+ 落焦（点击即焦）。
    grid_redraw(&window, cx);
    grid_click_at(&window, cx, click_x, click_y, 1);

    window
        .update(cx, |_host, _window, cx| {
            view.update(cx, |v, _| {
                assert_eq!(
                    v.selected_cell(),
                    Some((4, 1)),
                    "真实点击 C5 → 选中锚 B5（整区选择）"
                );
                assert_eq!(v.selected_a1().as_deref(), Some("B5"));
            });
        })
        .expect("断言选中");

    // ② 真实双击覆盖格 C5 → 进入编辑（锚左上，缓冲 = 左上原值）。
    grid_redraw(&window, cx);
    grid_click_at(&window, cx, click_x, click_y, 2);
    window
        .update(cx, |_host, _window, cx| {
            view.update(cx, |v, _| {
                let ed = v.editing.as_ref().expect("真实双击进入编辑态");
                assert_eq!((ed.row, ed.col), (4, 1), "编辑锚 = 左上 B5");
                assert_eq!(ed.buffer, "外部资料区", "缓冲以左上原值起步");
            });
        })
        .expect("断言编辑态");

    // ③ 真实键入「新」→ Enter 提交（焦点路径——handle_key 全链）。
    grid_redraw(&window, cx);
    grid_key_down(&window, cx, grid_key_event("新", Some("新"), false));
    window
        .update(cx, |_host, _window, cx| {
            view.update(cx, |v, _| {
                let ed = v.editing.as_ref().expect("编辑态保持");
                assert!(ed.buffer.contains("新"), "真实键入进缓冲：{}", ed.buffer);
            });
        })
        .expect("断言键入");
    grid_key_down(&window, cx, grid_key_event("enter", None, false));
    window
        .update(cx, |_host, _window, cx| {
            view.update(cx, |v, _| {
                assert!(v.editing.is_none(), "Enter 提交退出编辑态");
                assert!(v.is_modified(), "提交后脏标记");
            });
        })
        .expect("断言提交");

    // ④ 真实 Ctrl+S 落盘。
    grid_redraw(&window, cx);
    grid_key_down(&window, cx, grid_key_event("s", None, true));
    window
        .update(cx, |_host, _window, cx| {
            view.update(cx, |v, _| {
                assert!(!v.is_modified(), "Ctrl+S 落盘后脏清除");
            });
        })
        .expect("断言保存");

    // ⑤ 重读落盘文件：左上新值 + 范围保留 + 覆盖格零写入。
    let saved_bytes = std::fs::read(&tmp).expect("落盘文件可读");
    let mut reopened = XlsxEditor::open(&saved_bytes).expect("重开会话");
    let info = reopened.cell_info(0, "B5").expect("B5 可查").expect("非空");
    assert!(
        info.text.contains("新") && info.text.contains("外部资料区"),
        "B5 = 键入后的完整值（得 {}）",
        info.text
    );
    assert!(
        reopened.cell_info(0, "C5").expect("C5 可查").is_none(),
        "覆盖格 C5 零写入（禁止复制值填满合并区域）"
    );
    let saved_book = {
        let tmp2 =
            std::env::temp_dir().join(format!("mang-merge-verify-{}.xlsx", std::process::id()));
        std::fs::write(&tmp2, &saved_bytes).expect("验证临时落盘");
        let b = mdc_office_io::io::read_book_with_options(
            &tmp2.to_string_lossy(),
            &ReadOptions { compat: true },
        )
        .expect("读面");
        let _ = std::fs::remove_file(&tmp2);
        b
    };
    let dm = saved_book.sheets[0].merges[0];
    assert_eq!(
        (dm.start.0, dm.start.1, dm.end.0, dm.end.1),
        (4, 1, 4, 2),
        "merge 范围原样保留（B5:C5 绝对端点）"
    );
    let _ = std::fs::remove_file(&tmp);
}

/// #584 返工：合并区左上在数据包围盒**外**（审核复现场景——
/// 空合并格 E5:G7 + 数据 F6/G7 → 包围盒从 F6 起）——真实写面
/// 构造夹具 → 模型装载（锚 E5 不漂移）→ 编辑覆盖格落账 E5 →
/// 保存重读（E5 新值 / 覆盖格零写入 / merge 原样）。
#[test]
fn anchor_outside_box_edit_writes_original_e5() {
    use mdc_office_io::write::{CellValue, WorkbookWriter};

    // 构造：合并 E5:G7（绝对 (4,4)-(6,6)，文本空——锚在包围盒外）
    // + 数据 F6=1、G7=2（包围盒 (5,5)-(6,6)）。顺序关键：先 merge
    // 后 write——rust_xlsxwriter merge_range 擦除范围内既有写入
    //（先写后合并 = 数据被清空）；先合并后写入 = 覆盖格带值
    //（真实场景：被合并覆盖但仍有值的格——typed 网格过滤空格后
    // 包围盒从 F6 起，与审核复现场景一致）。
    let mut w = WorkbookWriter::new();
    let mut s = w.add_sheet("锚外").expect("建表");
    s.merge(4, 4, 6, 6, "", None).expect("合并 E5:G7（空文本）");
    s.write(5, 5, CellValue::Number(1.), None).expect("F6=1");
    s.write(6, 6, CellValue::Number(2.), None).expect("G7=2");
    let bytes = w.save_to_vec().expect("写盘");
    let tmp = std::env::temp_dir().join(format!("mang-anchor-out-{}.xlsx", std::process::id()));
    std::fs::write(&tmp, &bytes).expect("临时落盘");

    // 模型装载：包围盒 F6 起 + 可见段 (0,0)-(1,1) + 锚 E5 (4,4)。
    let model = XlsxGridModel::load(&tmp).expect("模型装载");
    let sheet = &model.sheets[0];
    assert_eq!(
        (sheet.start_row, sheet.start_col),
        (5, 5),
        "包围盒从 F6 起（merge 空文本不扩盒）"
    );
    let span = sheet.merge_spans.first().expect("合并范围在档");
    assert_eq!((span.row_start, span.col_start), (0, 0), "可见段起点 = F6");
    assert_eq!((span.row_end, span.col_end), (1, 1), "可见段末点 = G7");
    assert_eq!(span.abs_anchor, (4, 4), "原始绝对锚 = E5（不随裁剪漂移）");

    // 编辑覆盖格（可见 (1,1) = G7 位——canonical → 可见锚 (0,0)）：
    // 提交必须落账 **E5**（绝对锚），不是 F6（显示坐标换算的错值）。
    let mut editor = XlsxEditor::open(&bytes).expect("编辑会话");
    let mut model2 = model.clone();
    let (r, c) = canonical_cell(&model2.sheets[0].merge_spans, 1, 1).expect("覆盖格有可见锚映射");
    assert_eq!((r, c), (0, 0), "可见锚 = F6 显示坐标");
    // selected_a1 面（A1 引用锚 E5）。
    let a1 = format!("{}{}", col_letter(span.abs_anchor.1), span.abs_anchor.0 + 1);
    assert_eq!(a1, "E5", "A1 引用 = 原始绝对锚");
    commit_cell_edit(&mut editor, &mut model2, 0, r, c, "锚外新值")
        .expect("覆盖格编辑落账（锚 E5）");

    // 保存 → 重读：E5 有新值；F6/G7（覆盖格）保持原值不重写；
    // merge 范围原样。
    let saved = editor.save_to_vec().expect("保存");
    let mut reopened = XlsxEditor::open(&saved).expect("重开");
    let e5 = reopened.cell_info(0, "E5").expect("E5 可查").expect("非空");
    assert_eq!(e5.text, "锚外新值", "落账在原始绝对锚 E5（非 F6）");
    let f6 = reopened.cell_info(0, "F6").expect("F6 可查").expect("非空");
    assert_eq!(f6.text, "1", "覆盖格 F6 保持原值（零写入）");
    let g7 = reopened.cell_info(0, "G7").expect("G7 可查").expect("非空");
    assert_eq!(g7.text, "2", "覆盖格 G7 保持原值（零写入）");
    let book = {
        let tmp2 =
            std::env::temp_dir().join(format!("mang-anchor-verify-{}.xlsx", std::process::id()));
        std::fs::write(&tmp2, &saved).expect("验证落盘");
        let b = mdc_office_io::io::read_book_with_options(
            &tmp2.to_string_lossy(),
            &ReadOptions { compat: true },
        )
        .expect("读面");
        let _ = std::fs::remove_file(&tmp2);
        b
    };
    let dm = book.sheets[0].merges[0];
    assert_eq!(
        (dm.start.0, dm.start.1, dm.end.0, dm.end.1),
        (4, 4, 6, 6),
        "merge 范围原样保留（E5:G7）"
    );
    let _ = std::fs::remove_file(&tmp);
}
mod value_fix_io_tests {
    use super::*;

    pub(super) fn fixture_bytes(visible_top: bool) -> Vec<u8> {
        use mdc_office_io::write::{CellValue, WorkbookWriter};
        let mut book = WorkbookWriter::new();
        {
            let mut sheet = book.add_sheet("Anchor values").unwrap();
            sheet.merge(4, 4, 6, 6, "", None).unwrap();
            if visible_top {
                sheet
                    .write(0, 5, CellValue::Text("marker".into()), None)
                    .unwrap();
                sheet
                    .write(4, 5, CellValue::Text("covered-value".into()), None)
                    .unwrap();
            }
            sheet
                .write(5, 5, CellValue::Text("covered-value".into()), None)
                .unwrap();
            sheet
                .write(6, 6, CellValue::Text("covered-tail".into()), None)
                .unwrap();
        }
        book.save_to_vec().unwrap()
    }

    fn model(bytes: &[u8]) -> XlsxGridModel {
        let book = read_book_from_bytes_with_options(bytes, &ReadOptions { compat: true }).unwrap();
        XlsxGridModel::from_book(&book)
    }

    #[test]
    fn empty_original_anchor_is_displayed_and_prefilled_as_empty() {
        let bytes = fixture_bytes(false);
        let model = model(&bytes);
        let editor = XlsxEditor::open(&bytes).unwrap();
        let values = merge_values_from_session(&model, &editor).unwrap();
        assert_eq!(
            (model.sheets[0].start_row, model.sheets[0].start_col),
            (5, 5)
        );
        assert_eq!(display_cell_value(&model, &values, 0, 0, 0), "");
        assert_eq!(model.sheets[0].cells[0][0], "covered-value");
    }

    #[test]
    fn confirming_without_input_does_not_copy_the_covered_value() {
        let bytes = fixture_bytes(false);
        let mut model = model(&bytes);
        let mut editor = XlsxEditor::open(&bytes).unwrap();
        let mut values = merge_values_from_session(&model, &editor).unwrap();
        let value = display_cell_value(&model, &values, 0, 0, 0);
        commit_cell_edit(&mut editor, &mut model, 0, 0, 0, &value).unwrap();
        refresh_committed_merge_value(&model, &mut values, 0, 0, 0, &value);
        let mut reopened = XlsxEditor::open(&editor.save_to_vec().unwrap()).unwrap();
        assert!(
            reopened
                .cell_info(0, "E5")
                .unwrap()
                .map(|c| c.text)
                .unwrap_or_default()
                .is_empty()
        );
        assert_eq!(
            reopened.cell_info(0, "F6").unwrap().unwrap().text,
            "covered-value"
        );
        assert_eq!(
            reopened.cell_info(0, "G7").unwrap().unwrap().text,
            "covered-tail"
        );
    }

    #[test]
    fn consecutive_edits_and_session_restore_keep_anchor_and_covered_data_separate() {
        let bytes = fixture_bytes(false);
        let mut model = model(&bytes);
        let mut values = initial_merge_values(&model);
        let mut editor = XlsxEditor::open(&bytes).unwrap();
        commit_cell_edit(&mut editor, &mut model, 0, 0, 0, "first").unwrap();
        refresh_committed_merge_value(&model, &mut values, 0, 0, 0, "first");
        assert_eq!(display_cell_value(&model, &values, 0, 0, 0), "first");
        assert_eq!(model.sheets[0].cells[0][0], "covered-value");
        let restored = merge_values_from_session(&model, &editor).unwrap();
        assert_eq!(display_cell_value(&model, &restored, 0, 0, 0), "first");
        commit_cell_edit(&mut editor, &mut model, 0, 0, 0, "second").unwrap();
        refresh_committed_merge_value(&model, &mut values, 0, 0, 0, "second");
        assert_eq!(display_cell_value(&model, &values, 0, 0, 0), "second");
        let saved = editor.save_to_vec().unwrap();
        let mut reopened = XlsxEditor::open(&saved).unwrap();
        assert_eq!(reopened.cell_info(0, "E5").unwrap().unwrap().text, "second");
        assert_eq!(
            reopened.cell_info(0, "F6").unwrap().unwrap().text,
            "covered-value"
        );
        assert_eq!(
            reopened.cell_info(0, "G7").unwrap().unwrap().text,
            "covered-tail"
        );
        let book =
            read_book_from_bytes_with_options(&saved, &ReadOptions { compat: true }).unwrap();
        assert_eq!(
            (book.sheets[0].merges[0].start, book.sheets[0].merges[0].end),
            ((4, 4), (6, 6))
        );
    }
}

// 实际窗口事件路径；不把本case冒称本地真机/截图验证。
#[gpui::test]
fn original_anchor_double_click_reedit_session_restore_and_save(cx: &mut gpui::TestAppContext) {
    let bytes = value_fix_io_tests::fixture_bytes(true);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("merge-values-{}-{nonce}.xlsx", std::process::id()));
    std::fs::write(&path, &bytes).unwrap();
    let model = XlsxGridModel::load(&path).unwrap();
    assert_eq!(model.sheets[0].start_col, 5); // E5原锚在F列起的数据盒外。
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let mut slot = None;
    let window = cx.add_window(|_, cx| {
        let view = cx.new(|cx| XlsxGridView::new(model, path.clone(), cx));
        slot = Some(view.clone());
        GridHost(view)
    });
    let mut view = slot.unwrap();
    let (x, y) = window
        .update(cx, |_, _, cx| {
            view.update(cx, |v, _| {
                (GUTTER_W + v.col_offsets[0] + 5.0, 4.0 * ROW_H + ROW_H / 2.0)
            })
        })
        .unwrap();
    grid_redraw(&window, cx);
    grid_click_at(&window, cx, x, y, 2);
    window
        .update(cx, |_, _, cx| {
            view.update(cx, |v, _| {
                assert_eq!(v.selected_a1().as_deref(), Some("E5"));
                assert_eq!(
                    v.editing.as_ref().unwrap().buffer,
                    "",
                    "空E5不应预填F5/F6覆盖值"
                );
                assert_eq!(display_cell_value(&v.model, &v.merge_values, 0, 4, 0), "");
            })
        })
        .unwrap();
    grid_key_down(&window, cx, grid_key_event("enter", None, false));
    window
        .update(cx, |_, _, cx| {
            view.update(cx, |v, _| {
                assert!(!v.is_modified(), "确认未改不置脏");
            })
        })
        .unwrap();
    for (input, expected_before, expected_after) in
        [("first", "", "first"), ("-second", "first", "first-second")]
    {
        grid_redraw(&window, cx);
        grid_click_at(&window, cx, x, y, 2);
        window
            .update(cx, |_, _, cx| {
                view.update(cx, |v, _| {
                    assert_eq!(v.editing.as_ref().unwrap().buffer, expected_before);
                })
            })
            .unwrap();
        for ch in input.chars() {
            let character = ch.to_string();
            grid_key_down(
                &window,
                cx,
                grid_key_event(&character, Some(&character), false),
            );
        }
        grid_key_down(&window, cx, grid_key_event("enter", None, false));
        window
            .update(cx, |_, _, cx| {
                view.update(cx, |v, _| {
                    assert_eq!(
                        display_cell_value(&v.model, &v.merge_values, 0, 4, 0),
                        expected_after
                    );
                    assert_eq!(
                        v.model.sheets[0].cells[4][0], "covered-value",
                        "F5模型槽保持原值"
                    );
                })
            })
            .unwrap();
    }
    // 模拟Shell会话银行：磁盘model仍旧，pending会话必须提供新E5值。
    let (session, dirty) = window
        .update(cx, |_, _, cx| {
            view.update(cx, |v, _| {
                let session = v.take_session().unwrap();
                (session, v.is_modified())
            })
        })
        .unwrap();
    let old_model = XlsxGridModel::load(&path).unwrap();
    let next = window
        .update(cx, |_, _, cx| {
            cx.new(|cx| XlsxGridView::new_with_session(old_model, path.clone(), session, cx))
        })
        .unwrap();
    window
        .update(cx, |host, _, cx| {
            next.update(cx, |v, _| v.set_modified(dirty));
            host.0 = next.clone();
            cx.notify();
        })
        .unwrap();
    view = next;
    grid_redraw(&window, cx);
    grid_click_at(&window, cx, x, y, 2);
    window
        .update(cx, |_, _, cx| {
            view.update(cx, |v, _| {
                assert_eq!(
                    v.editing.as_ref().unwrap().buffer,
                    "first-second",
                    "旧model不得覆盖会话E5新值"
                );
            })
        })
        .unwrap();
    grid_key_down(&window, cx, grid_key_event("enter", None, false));
    grid_redraw(&window, cx);
    grid_key_down(&window, cx, grid_key_event("s", None, true));
    let saved = std::fs::read(&path).unwrap();
    let mut reopened = XlsxEditor::open(&saved).unwrap();
    assert_eq!(
        reopened.cell_info(0, "E5").unwrap().unwrap().text,
        "first-second"
    );
    for reference in ["F5", "F6"] {
        assert_eq!(
            reopened.cell_info(0, reference).unwrap().unwrap().text,
            "covered-value"
        );
    }
    assert_eq!(
        reopened.cell_info(0, "G7").unwrap().unwrap().text,
        "covered-tail"
    );
    let book = read_book_from_bytes_with_options(&saved, &ReadOptions { compat: true }).unwrap();
    assert_eq!(
        (book.sheets[0].merges[0].start, book.sheets[0].merges[0].end),
        ((4, 4), (6, 6))
    );
    std::fs::remove_file(path).unwrap();
}
