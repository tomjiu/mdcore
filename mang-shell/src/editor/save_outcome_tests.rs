//! #47 巨石拆解批②：原 editor/mod.rs 内嵌 `save_outcome_tests` 纯迁出（零行为变化——
//! 字节原样剥壳，挂载 #[cfg(test)] + #[path] 在 mod.rs）。

use super::*;
use std::path::PathBuf;

/// 临时夹具目录（进程号隔离；首尾各清一次）。
fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mdc-386-ed-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// #386：只读态保存不再假装成功——干净只读 = `Unmodified`（no-op
/// 的诚实口径），且文本面恒不脏（pdf 的「脏」来自页视图批注，
/// 归 Shell 分流——语义分层各管各的）。
#[gpui::test]
fn readonly_save_is_unmodified_not_fake_success(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("ro");
    let path = dir.join("doc.pdf");
    std::fs::write(&path, b"%PDF-fixture").unwrap();
    let ed = cx.update(|cx| {
        cx.new(|cx| Editor::new_readonly(Some(path.clone()), "渲染态文本".into(), cx))
    });
    // 前置：只读 + 不脏（正常路径——只读门禁拦下一切编辑）。
    assert!(cx.update(|cx| ed.read(cx).is_read_only()));
    assert!(!cx.update(|cx| ed.read(cx).is_modified()));
    // 旧实现在此 `return true`（保存「成功」的谎——调用方无从区分
    // 「真存了」与「什么都没做」，PDF 批注静默丢失的根因）。
    let outcome = cx.update(|cx| ed.update(cx, |ed, _| ed.save()));
    assert_eq!(
        outcome,
        SaveOutcome::Unmodified,
        "#386：只读干净 = Unmodified（no-op），不是 Saved"
    );
    // 盘上不动（只读文本面无写盘语义——「另存为」才迁移内容）。
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"%PDF-fixture",
        "只读保存零写盘"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// #386：可编辑文档三态——未命名 `Failed`（交调用方引导另存）/
/// 有名干净 `Unmodified`（零写盘）/ 有名脏 `Saved`（落盘 + 脏标清）。
#[gpui::test]
fn editable_save_three_states(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("rw");
    let named = dir.join("a.md");
    std::fs::write(&named, "L0\n").unwrap();

    // ① 未命名（新文档无路径）→ Failed（不是静默 false——调用方
    //    引导「另存为」）。
    let unnamed = cx.update(|cx| cx.new(|cx| Editor::new(None, "新文档".into(), cx)));
    assert_eq!(
        cx.update(|cx| unnamed.update(cx, |ed, _| ed.save())),
        SaveOutcome::Failed,
        "未命名保存 = Failed（无路径可写）"
    );

    // ② 有名干净 → Unmodified（零写盘——盘上不动）。
    let clean = cx.update(|cx| cx.new(|cx| Editor::new(Some(named.clone()), "L0\n".into(), cx)));
    assert_eq!(
        cx.update(|cx| clean.update(cx, |ed, _| ed.save())),
        SaveOutcome::Unmodified,
        "干净保存 = no-op"
    );
    assert_eq!(std::fs::read_to_string(&named).unwrap(), "L0\n");

    // ③ 有名脏 → Saved：盘上 = 行模型文本 + is_modified 清零。
    cx.update(|cx| {
        clean.update(cx, |ed, cx| {
            ed.insert_text_at(0, 2, "X", cx);
        })
    });
    assert!(
        cx.update(|cx| clean.read(cx).is_modified()),
        "前置：插入后脏"
    );
    assert_eq!(
        cx.update(|cx| clean.update(cx, |ed, _| ed.save())),
        SaveOutcome::Saved,
        "脏保存 = Saved"
    );
    assert_eq!(
        std::fs::read_to_string(&named).unwrap(),
        "L0X\n",
        "Saved = 行模型文本落盘"
    );
    assert!(
        !cx.update(|cx| clean.read(cx).is_modified()),
        "保存成功后脏标清零"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
