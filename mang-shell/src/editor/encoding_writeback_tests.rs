//! #47 巨石拆解批②：原 editor/mod.rs 内嵌 `encoding_writeback_tests` 纯迁出（零行为变化——
//! 字节原样剥壳，挂载 #[cfg(test)] + #[path] 在 mod.rs）。

use super::*;
use crate::document::{self, OpenOutcome};
use gpui::Entity;
use std::path::Path;

/// "中文文档" GBK 字节（手工钉死，同 document.rs 探测红测）。
const GBK_ZHONGWEN: &[u8] = &[0xD6, 0xD0, 0xCE, 0xC4, 0xCE, 0xC4, 0xB5, 0xB5];
/// "测试文档" GBK 字节（手工钉死：测 B2E2 试 CAD4 文 CEC4 档 B5B5）。
const GBK_CESHI: &[u8] = &[0xB2, 0xE2, 0xCA, 0xD4, 0xCE, 0xC4, 0xB5, 0xB5];
/// "中文文档" UTF-16LE + BOM 字节（手工钉死，同 document.rs 红测）。
const UTF16LE_ZHONGWEN: &[u8] = &[0xFF, 0xFE, 0x2D, 0x4E, 0x87, 0x65, 0x87, 0x65, 0x63, 0x68];

fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mdc-111-g1-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 经真实打开链（document::open_editor）构造编辑器——编码探测
/// 必须走同一分派面，不绕开。
fn open_ed(path: &Path, cx: &mut gpui::TestAppContext) -> Entity<Editor> {
    match cx.update(|cx| document::open_editor(path, cx)) {
        OpenOutcome::Opened(ed) => ed,
        OpenOutcome::Unsupported { message, .. } | OpenOutcome::Failed { message, .. } => {
            panic!("应打开成功 {}: {message}", path.display())
        }
    }
}

/// 红 1：GBK 打开→编辑→保存 → 盘上仍是 GBK 字节。
#[gpui::test]
fn gbk_save_writes_back_original_encoding(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("gbk");
    let path = dir.join("gbk.md");
    std::fs::write(&path, GBK_ZHONGWEN).unwrap();
    let ed = open_ed(&path, cx);
    cx.update(|cx| {
        ed.update(cx, |ed, cx| {
            ed.insert_text_at(0, 0, "X", cx);
        })
    });
    assert_eq!(
        cx.update(|cx| ed.update(cx, |ed, _| ed.save())),
        SaveOutcome::Saved,
        "GBK 文档保存应成功"
    );
    let mut want = vec![b'X'];
    want.extend_from_slice(GBK_ZHONGWEN);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        want,
        "保存写回 GBK 字节（先红：现恒写 UTF-8——打开探测编码被丢弃）"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 红 2：UTF-8 BOM → 保存补回 BOM。
#[gpui::test]
fn utf8_bom_save_restores_bom(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("bom");
    let path = dir.join("bom.md");
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice("中文".as_bytes()); // e4 b8 ad e6 96 87
    std::fs::write(&path, &bytes).unwrap();
    let ed = open_ed(&path, cx);
    cx.update(|cx| {
        ed.update(cx, |ed, cx| {
            ed.insert_text_at(0, 0, "X", cx);
        })
    });
    assert_eq!(
        cx.update(|cx| ed.update(cx, |ed, _| ed.save())),
        SaveOutcome::Saved
    );
    let mut want = vec![0xEF, 0xBB, 0xBF, b'X'];
    want.extend_from_slice("中文".as_bytes());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        want,
        "BOM 文档保存补回 EF BB BF（先红：BOM 打开即剥、保存不再补）"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 红 3：UTF-16LE（BOM）→ 保存写 BOM + LE 单元。
#[gpui::test]
fn utf16le_save_writes_bom_and_units(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("u16");
    let path = dir.join("u16.txt");
    std::fs::write(&path, UTF16LE_ZHONGWEN).unwrap();
    let ed = open_ed(&path, cx);
    cx.update(|cx| {
        ed.update(cx, |ed, cx| {
            ed.insert_text_at(0, 0, "X", cx);
        })
    });
    assert_eq!(
        cx.update(|cx| ed.update(cx, |ed, _| ed.save())),
        SaveOutcome::Saved
    );
    let mut want = vec![0xFF, 0xFE, b'X', 0x00];
    want.extend_from_slice(&UTF16LE_ZHONGWEN[2..]);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        want,
        "UTF-16LE 保存写回 BOM + LE 单元（先红：现写 UTF-8）"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 红 4：GBK 不可映射字符 → 保存拒绝且盘上不动（不静默替换造乱码）。
#[gpui::test]
fn gbk_unmappable_save_rejected_disk_untouched(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("gbkemoji");
    let path = dir.join("emoji.md");
    std::fs::write(&path, GBK_ZHONGWEN).unwrap();
    let ed = open_ed(&path, cx);
    cx.update(|cx| {
        ed.update(cx, |ed, cx| {
            ed.insert_text_at(0, 0, "\u{1F600}", cx);
        })
    });
    let outcome = cx.update(|cx| ed.update(cx, |ed, _| ed.save()));
    assert_eq!(
        outcome,
        SaveOutcome::Failed,
        "GBK 装不下的字符拒绝静默替换（先红：现为 Saved + 改写 UTF-8）"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        GBK_ZHONGWEN,
        "保存失败盘上必须零改动（原 GBK 字节原封）"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 红 5：外部改盘 GBK → reload_from_disk 解码重载（#134 面的编码半）。
#[gpui::test]
fn reload_from_disk_decodes_gbk(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("gbkre");
    let path = dir.join("re.md");
    std::fs::write(&path, GBK_ZHONGWEN).unwrap();
    let ed = open_ed(&path, cx);
    // 外部进程把盘上换成另一份 GBK 内容（模拟 RPC write_file / 外部
    // 编辑器——磁盘是权威，Buffer 追上）。
    std::fs::write(&path, GBK_CESHI).unwrap();
    let ok = cx.update(|cx| ed.update(cx, |ed, cx| ed.reload_from_disk(cx)));
    assert!(
        ok,
        "GBK 外改盘应能解码重载（先红：from_utf8 拒——返回 false）"
    );
    let got = ed.read_with(cx, |ed, _| ed.buffer.line(0));
    assert_eq!(got, "测试文档", "重载后 Buffer 与磁盘一致");
    let _ = std::fs::remove_dir_all(&dir);
}
