//! B4 编辑与保存可靠性测试（#49 工作线 B）。
//!
//! 中文正文/单元格实际编辑 → saveAs 暂存产物 → **独立 DOCX 校验**
//! （`verify` 模块：不经 LO 重开，宿主单方面确认产物——C 线「保存完成」
//! 判据）；撤销重做与 dirty/重绘一致性；保存失败保脏与源文件不变。
//! 环境门：无 LibreOffice 运行时 SKIP（≠PASS）。

use mdc_lok::client::DocSession;
use std::path::PathBuf;
use std::time::Duration;

fn runtime_available() -> bool {
    mdc_lok::runtime::discover().is_ok()
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke-chars-img-table.docx")
}

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mdc-lok-b4-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn file_url(p: &std::path::Path) -> String {
    let canon = std::fs::canonicalize(p).unwrap();
    format!("file://{}", canon.to_string_lossy())
}

fn worker_bin_env() {
    if let Some(w) = option_env!("CARGO_BIN_EXE_mdc-lok-worker") {
        std::env::set_var("MDC_LOK_WORKER_BIN", w);
    }
}

fn sha256_of(p: &std::path::Path) -> String {
    mdc_lok::sha256::sha256_hex(&std::fs::read(p).unwrap())
}

/// LOK UNO SearchItem 参数（ExecuteSearch 定位）。
fn search_args(q: &str) -> String {
    serde_json::json!({
        "SearchItem.SearchString": {"type": "string", "value": q},
        "SearchItem.Backward": {"type": "boolean", "value": "false"},
        "SearchItem.Command": {"type": "unsigned short", "value": "0"},
        "SearchItem.SearchFlags": {"type": "long", "value": "0"},
        "SearchItem.TransliterateFlags": {"type": "long", "value": "0"},
        "SearchItem.AlgorithmType2": {"type": "short", "value": "1"},
    })
    .to_string()
}

/// 中文正文 + 单元格编辑 → 保存暂存产物 → **独立校验**（verify 模块，
/// 不经 LO）→ 源文件字节不变。这是 B 线给 C 线的完整保存安全链样本。
#[test]
fn edit_and_save_independent_verify_chain() {
    worker_bin_env();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("chain");
    let work = dir.join("work.docx");
    std::fs::copy(fixture(), &work).unwrap();
    let src_hash = sha256_of(&work);
    let saved_url = {
        let canon = std::fs::canonicalize(&dir).unwrap();
        format!("file://{}", canon.join("saved.docx").to_string_lossy())
    };
    let saved_path = dir.join("saved.docx");

    let mut s = DocSession::open(None, &dir.join("s"), &file_url(&work), None).expect("打开");

    // 正文编辑：定位锚点（ExecuteSearch 查询类）→ 键入替换（选区语义）
    s.uno_query(".uno:ExecuteSearch", Some(&search_args("锚点正文句甲")))
        .expect("定位正文锚点");
    s.type_text("正文新句二零二六").expect("正文替换");

    // 单元格编辑：定位表格单元格 → 键入替换
    s.uno_query(".uno:ExecuteSearch", Some(&search_args("表格单元格丙")))
        .expect("定位单元格锚点");
    s.type_text("单元格新文戊").expect("单元格替换");

    assert!(s.is_dirty(), "编辑后 dirty");
    let rev_before_save = s.revision();

    // 保存暂存产物
    s.save_as(&saved_url, "docx").expect("save_as 暂存产物");
    assert!(!s.is_dirty(), "保存成功清脏");
    assert_eq!(s.revision(), rev_before_save + 1, "保存推进 revision");

    // **独立校验**（C 线判据——不经 LO）：
    //   zip 结构 + document.xml 内容断言 + 图片条目在场
    let report = mdc_lok::verify::verify_saved_docx(
        &saved_path,
        &mdc_lok::verify::VerifySpec::new()
            .must_contain("正文新句二零二六")
            .must_contain("单元格新文戊")
            .must_contain("相邻段落乙") // 相邻内容保留
            .must_not_contain("锚点正文句甲")
            .must_not_contain("表格单元格丙")
            .must_have_entry("word/media/image1.png"), // 图片保留
    )
    .expect("独立校验（zip/文本/图片）");
    assert!(report.document_xml_len > 1000, "document.xml 非空");
    assert!(
        report.entry_count >= 8,
        "DOCX 条目结构完整（实际 {}）",
        report.entry_count
    );
    assert_eq!(report.checks_passed, 6, "六断言全过");

    // 源文件字节不变（隔离铁律——替换决策留给 C 线）
    assert_eq!(sha256_of(&work), src_hash, "源文件 hash 前后一致");

    // 产物重开（LO 面）与独立校验互证
    let mut v = DocSession::open(None, &dir.join("v"), &saved_url, None).expect("重开保存产物");
    v.uno(".uno:SelectAll", None).expect("全选");
    let (_, text) = v.text_selection("text/plain;charset=utf-8").expect("选区");
    assert!(text.contains("正文新句二零二六"), "LO 面重开含新正文");
    s.shutdown();
    v.shutdown();
}

/// 撤销/重做与 dirty/重绘一致性：undo 不清脏（仍是文档变更），
/// 每步编辑伴随 INVALIDATE_TILES（重绘信号）。
#[test]
fn undo_redo_dirty_and_repaint_consistency() {
    worker_bin_env();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("undo");
    let work = dir.join("work.docx");
    std::fs::copy(fixture(), &work).unwrap();

    let mut s = DocSession::open(None, &dir.join("s"), &file_url(&work), None).expect("打开");
    let events = s.take_events().expect("事件通道");

    // 定位 + 键入
    s.uno_query(".uno:ExecuteSearch", Some(&search_args("相邻段落乙")))
        .expect("定位");
    s.type_text("临时编辑甲").expect("键入");
    assert!(s.is_dirty(), "键入后 dirty");
    let rev_after_type = s.revision();

    // undo：文档仍处于修改态（相对源），dirty 必须保持
    s.uno(".uno:Undo", None).expect("undo");
    assert!(
        s.is_dirty(),
        "undo 后 dirty 保持（LO undo 是文档变更，不回写源文件）"
    );
    assert_eq!(s.revision(), rev_after_type + 1, "undo 推进 revision");

    // redo：恢复编辑
    s.uno(".uno:Redo", None).expect("redo");
    assert!(s.is_dirty(), "redo 后 dirty 保持");
    assert_eq!(s.revision(), rev_after_type + 2, "redo 推进 revision");

    // 语义闭环：undo 后键入的文本应消失，redo 后恢复
    s.uno(".uno:Undo", None).expect("第二次 undo（撤 redo）");
    s.uno(".uno:SelectAll", None).expect("全选");
    let mut text = String::new();
    for _ in 0..3 {
        let (_, t) = s.text_selection("text/plain;charset=utf-8").unwrap();
        text = t;
        if !text.contains("临时编辑甲") {
            break;
        }
        s.uno(".uno:SelectAll", None).unwrap();
    }
    assert!(
        !text.contains("临时编辑甲"),
        "undo 语义生效（键入文本被撤销；实际片段：{:?}）",
        &text[..text.len().min(60)]
    );

    // 重绘一致性：编辑期间 INVALIDATE_TILES 事件持续到达
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut invalidate_count = 0;
    while std::time::Instant::now() < deadline {
        match events.recv_timeout(Duration::from_millis(200)) {
            Ok(mdc_lok::client::Event::Callback { name, .. }) if name == "INVALIDATE_TILES" => {
                invalidate_count += 1;
                if invalidate_count >= 2 {
                    break;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert!(
        invalidate_count >= 2,
        "编辑/undo/redo 全程 INVALIDATE_TILES ≥2 次（重绘信号一致；实际 {invalidate_count}）"
    );

    s.shutdown();
}

/// 保存失败保脏 + 源文件不变（DocSession 面：拒写目录）。
#[test]
fn save_refusal_keeps_dirty_and_source_intact() {
    worker_bin_env();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("refuse");
    let work = dir.join("work.docx");
    std::fs::copy(fixture(), &work).unwrap();
    let src_hash = sha256_of(&work);
    // 拒写目标：父路径是普通文件（任何用户都建不出目标——root 下
    // chmod 拒写不可靠）
    let blocker = dir.join("blocker.txt");
    std::fs::write(&blocker, b"not a dir").unwrap();
    let blocked_url = {
        let canon = std::fs::canonicalize(&blocker).unwrap();
        format!("file://{}", canon.join("child.docx").to_string_lossy())
    };

    let mut s = DocSession::open(None, &dir.join("s"), &file_url(&work), None).expect("打开");
    s.uno_query(".uno:ExecuteSearch", Some(&search_args("相邻段落乙")))
        .expect("定位");
    s.type_text("编辑后保存失败").expect("编辑");
    assert!(s.is_dirty());
    let rev = s.revision();

    let err = s
        .save_as(&blocked_url, "docx")
        .expect_err("拒写目标必须失败");
    eprintln!("拒写错误（可操作样本）：{err}");
    assert!(s.is_dirty(), "保存失败保脏位（不静默成功）");
    assert_eq!(s.revision(), rev, "保存失败不推进 revision");
    assert_eq!(sha256_of(&work), src_hash, "源文件不变");

    // 失败后会话仍可用（不因保存失败假死）——可再次保存到合法目标
    let ok_url = {
        let canon = std::fs::canonicalize(&dir).unwrap();
        format!("file://{}", canon.join("recovered.docx").to_string_lossy())
    };
    s.save_as(&ok_url, "docx").expect("失败后恢复保存");
    assert!(!s.is_dirty(), "恢复保存清脏");
    s.shutdown();
}
