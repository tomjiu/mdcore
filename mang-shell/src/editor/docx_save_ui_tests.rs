use super::*;
use mdc_office_io::docx::{DocxWriter, RunSpec};
use mdc_office_io::docx_edit::DocxEditor;

struct Fixture(PathBuf);

impl Fixture {
    fn new(tag: &str, bytes: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!(
            "mdcore-629-ui-{tag}-{}-{}.docx",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, bytes).unwrap();
        Self(path)
    }

    fn open(&self, cx: &mut gpui::TestAppContext) -> gpui::Entity<Editor> {
        let item = docx_doc::load_editable(&self.0).unwrap();
        cx.update(|cx| cx.new(|cx| Editor::new_docx_editable(self.0.clone(), item, cx)))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn setup_find(editor: &mut Editor, query: &str, replacement: &str) {
    let mut find = FindState::default();
    find.open_replace(&editor.buffer);
    find.set_query(query, &editor.buffer);
    find.set_focus_replacement(true);
    // #629 测试姿势修复：含 \n 的多行替换文本不走 handle_key 注入——
    // 真实键盘路径按字符逐个送达且 \n 走 enter 分支（ReplaceCurrent），
    // key_char 内含控制字符会被 is_control 整串拒绝。改走 set_replacement
    // 直设器（与 set_query 对称的真实注入口，渲染层粘贴同口径）。
    find.set_replacement(replacement);
    assert_eq!(find.replacement(), replacement);
    editor.find = Some(find);
}

#[gpui::test]
fn find_all_with_newlines_saves_the_right_paragraphs_and_undo_preserves_xml(
    cx: &mut gpui::TestAppContext,
) {
    let mut writer = DocxWriter::new();
    writer.paragraph_text("目标一");
    writer.paragraph_text("未改二");
    writer.paragraph_text("目标三");
    let source = writer.save_to_vec().unwrap();
    let fixture = Fixture::new("find", &source);
    let editor = fixture.open(cx);
    cx.update(|cx| {
        editor.update(cx, |editor, _| {
            setup_find(editor, "目标", "新增\n目标");
            editor.replace_find(true);
            assert_eq!(editor.save(), SaveOutcome::Saved);
            let saved = DocxEditor::open(&std::fs::read(&fixture.0).unwrap()).unwrap();
            assert_eq!(saved.paragraph_text(0), "新增\n目标一");
            assert_eq!(saved.paragraph_text(1), "未改二");
            assert_eq!(saved.paragraph_text(2), "新增\n目标三");
            assert!(editor.undo_edit());
            assert_eq!(editor.save(), SaveOutcome::Saved);
        })
    });
    let after = std::fs::read(&fixture.0).unwrap();
    // #629 收尾裁定：撤销→保存后与原文做「部件级一致性」比对——全部条目
    // 同名同序、内容逐字节恒等（含 word/document.xml），另做语义层重开
    // 断言。**不做整包字节恒等**：zip 8.6 raw_copy_file 会把 docx-rs 的
    // Unix 目录条目头（vmade 0x314 / eattr 0x41ed0000）归一化为
    // 0x30a / 0x81ed0000，重写部件也按 zip 缺省 deflate（原 stored）——
    // 容器头形态不是产品契约（office-io 既有口径
    // core_untouched_bytes_and_regions_identical /
    // table_cells_is_read_only_and_save_stays_identical 均为部件内容恒等，
    // 非整包字节）。
    assert_docx_parts_identical(&source, &after);
    let saved = DocxEditor::open(&after).unwrap();
    assert_eq!(saved.paragraph_text(0), "目标一");
    assert_eq!(saved.paragraph_text(1), "未改二");
    assert_eq!(saved.paragraph_text(2), "目标三");
}

/// 两个 DOCX 包的部件级一致性：条目同名同序 + 每部件解压内容逐字节相等。
/// （#629：e2e 面 tests/docx_bid_e2e.rs::zip_entries 的同构精简版——
/// EOCD→中央目录→本地头定位数据；stored 直取 / deflate 走 flate2。）
fn assert_docx_parts_identical(a: &[u8], b: &[u8]) {
    let pa = zip_parts(a);
    let pb = zip_parts(b);
    assert_eq!(
        pa.len(),
        pb.len(),
        "部件数一致（{}/{}）",
        pa.len(),
        pb.len()
    );
    for ((na, da), (nb, db)) in pa.iter().zip(pb.iter()) {
        assert_eq!(na, nb, "部件名同序");
        assert_eq!(da, db, "部件 {na} 解压内容逐字节恒等");
    }
}

/// 解包全部条目为 (名, 解压字节)。仅测试面；不处理 zip64/加密（夹具
/// 与保存产物都不含）。
fn zip_parts(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    fn eocd(b: &[u8]) -> usize {
        let mut i = b.len().saturating_sub(22);
        loop {
            if b[i..i + 4] == [0x50, 0x4b, 0x05, 0x06] {
                return i;
            }
            assert!(i > 0, "EOCD 不在场（非 ZIP？）");
            i -= 1;
        }
    }
    let b = bytes;
    let e = eocd(b);
    let count = u16::from_le_bytes([b[e + 10], b[e + 11]]) as usize;
    let cd = u32::from_le_bytes([b[e + 16], b[e + 17], b[e + 18], b[e + 19]]) as usize;
    let mut out = Vec::with_capacity(count);
    let mut p = cd;
    for _ in 0..count {
        assert_eq!(&b[p..p + 4], &[0x50, 0x4b, 0x01, 0x02], "中央目录头");
        let method = u16::from_le_bytes([b[p + 10], b[p + 11]]);
        let csize = u32::from_le_bytes([b[p + 20], b[p + 21], b[p + 22], b[p + 23]]) as usize;
        let nlen = u16::from_le_bytes([b[p + 28], b[p + 29]]) as usize;
        let elen = u16::from_le_bytes([b[p + 30], b[p + 31]]) as usize;
        let clen = u16::from_le_bytes([b[p + 32], b[p + 33]]) as usize;
        let lho = u32::from_le_bytes([b[p + 42], b[p + 43], b[p + 44], b[p + 45]]) as usize;
        let name = String::from_utf8_lossy(&b[p + 46..p + 46 + nlen]).into_owned();
        let lnlen = u16::from_le_bytes([b[lho + 26], b[lho + 27]]) as usize;
        let lelen = u16::from_le_bytes([b[lho + 28], b[lho + 29]]) as usize;
        let data = &b[lho + 30 + lnlen + lelen..][..csize];
        let plain = if method == 0 {
            data.to_vec()
        } else {
            let mut v = Vec::new();
            std::io::Read::read_to_end(&mut flate2::read::DeflateDecoder::new(data), &mut v)
                .expect("deflate 解压");
            v
        };
        out.push((name, plain));
        p += 46 + nlen + elen + clen;
    }
    out
}

#[gpui::test]
fn find_cannot_replace_a_locked_page_field(cx: &mut gpui::TestAppContext) {
    let mut writer = DocxWriter::new();
    writer.paragraph_text("前文");
    // #629 测试姿势修复：锁定行夹具改混合段（text + field_page + text，
    // 与 footer 夹具同构）。纯 PAGE 域段落在 office-io 模型级钉定下读回
    // 文本为空（域结果文本不入读回文本——域值是渲染期计算面），
    // buffer.line(locked) 为空使 :100 的非空断言失败；混合段锁定行
    // 非空（"第  页"），仍完整覆盖 typed 拒改 + replace_find 拒改 +
    // is_modified==false 的全链语义。
    writer.paragraph(&[
        RunSpec::text("第 "),
        RunSpec::field_page(),
        RunSpec::text(" 页"),
    ]);
    writer.paragraph_text("后文");
    let source = writer.save_to_vec().unwrap();
    let fixture = Fixture::new("field", &source);
    let editor = fixture.open(cx);
    cx.update(|cx| {
        editor.update(cx, |editor, _| {
            let original = editor.buffer.to_plain_text();
            let locked = editor
                .docx_edit
                .as_ref()
                .unwrap()
                .borrow()
                .bridge
                .rows()
                .iter()
                .position(|row| matches!(row, LineBlock::Locked(_)))
                .unwrap();
            let query = editor.buffer.line(locked);
            assert!(!query.is_empty());
            for all in [false, true] {
                setup_find(editor, &query, "破坏");
                editor.replace_find(all);
                assert_eq!(editor.buffer.to_plain_text(), original);
                assert!(!editor.is_modified());
            }
        })
    });
    assert_eq!(std::fs::read(&fixture.0).unwrap(), source);
}

#[gpui::test]
fn indent_undo_keeps_the_previous_edit_and_saves_without_mapping_failure(
    cx: &mut gpui::TestAppContext,
) {
    let mut writer = DocxWriter::new();
    writer.paragraph_text("第一段");
    writer.paragraph_text("第二段");
    let fixture = Fixture::new("indent", &writer.save_to_vec().unwrap());
    let editor = fixture.open(cx);
    cx.update(|cx| {
        editor.update(cx, |editor, _| {
            assert!(
                editor
                    .buffer
                    .replace_range(Point::new(0, 0), Point::new(0, 0), "已有编辑")
            );
            let before_indent = editor.buffer.to_plain_text();
            editor
                .buffer
                .set_selection(Point::new(0, 0), Point::new(1, editor.buffer.line_len(1)));
            editor.sync_cursors_from_buffer();
            editor.indent_key(false);
            assert_ne!(editor.buffer.to_plain_text(), before_indent);
            assert!(editor.undo_edit());
            assert_eq!(editor.buffer.to_plain_text(), before_indent);
            assert_eq!(editor.save(), SaveOutcome::Saved);
            let saved = DocxEditor::open(&std::fs::read(&fixture.0).unwrap()).unwrap();
            assert_eq!(saved.paragraph_text(0), "已有编辑第一段");
            assert_eq!(saved.paragraph_text(1), "第二段");
        })
    });
}
