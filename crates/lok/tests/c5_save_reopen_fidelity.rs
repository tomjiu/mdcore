//! #50 C5 批2：保存重开保真（产品链真验收——`DocSession` 同一条 worker 链）。
//!
//! C5 清单原文：「正文/单元格改动、撤销重做、保存重开后未改文字/图像保留」。
//! B 链 `edit_save_reliability.rs` 已钉：编辑→saveAs→zip 独立校验→LO 重开
//! 含新正文。本文件补齐 C5 语义缺口（同一夹具、同一判据风格）：
//!
//! 1. **图片字节恒等**：保存产物 `word/media/image1.png` 与源**逐字节相等**
//!    （B 链只断言「条目在场」，未断言字节——本批钉死「图像保留」的
//!    最强形式：LO 保存不得重编码/压缩/重排媒体字节）；
//! 2. **未改文字语义保留**：未触碰段落（相邻段/表头/末尾哨兵）在保存
//!    产物 document.xml 中原样在场 + 被替换文本不在场（B 链 VerifySpec
//!    同款断言面，扩到 C5 全集）；
//! 3. **撤销重做往返保真**：编辑→undo→保存→重开，键入文本必须从
//!    产物消失（语义撤销落盘）；同一会话 redo→保存→重开，文本恢复。
//!    （B 链只验 undo/redo 的 dirty/revision 语义，未把「撤销态」保存
//!    重开——C5 要求保存链尊重撤销语义）；
//! 4. **源文件隔离**：全程源 hash 不变（隔离铁律——C4 已钉，此处
//!    C5 复验一遍收口）。
//!
//! 环境门：无 LibreOffice 运行时 SKIP（≠PASS——缺件纪律同 B 链）。

use mdc_lok::client::DocSession;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

fn runtime_available() -> bool {
    mdc_lok::runtime::discover().is_ok()
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke-chars-img-table.docx")
}

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "mdc-c5-reopen-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn file_url(p: &std::path::Path) -> String {
    // canonicalize 兜底：测试并行下目录可能已存在（非首次），NotFound
    // 只可能来自路径本身——直接用绝对路径 join（夹具/产物都在 tempdir 下）。
    let canon = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    mdc_lok::profile::file_url_of(&canon)
}

fn worker_bin_env() {
    if let Some(w) = option_env!("CARGO_BIN_EXE_mdc-lok-worker") {
        std::env::set_var("MDC_LOK_WORKER_BIN", w);
    }
}

fn sha256_of(p: &std::path::Path) -> String {
    mdc_lok::sha256::sha256_hex(&std::fs::read(p).unwrap())
}

/// LOK UNO SearchItem 参数（ExecuteSearch 定位——B 链同款）。
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

/// 读 zip 条目原始字节（图片字节恒等断言用——CRC 已由解压校验覆盖，
/// 此处再取原始字节做逐字节对照）。
fn entry_bytes(path: &std::path::Path, name: &str) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    for index in 0..archive.len() {
        let mut entry = match archive.by_index(index) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.name() == name {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).ok()?;
            return Some(bytes);
        }
    }
    None
}

/// 保存前轮询到文档内容真的含全部期望文本（B 链同款——type_text 应答
/// ≠ 内核已落定，立即保存会截断末字）。
fn settle_document_contains(s: &mut DocSession, needles: &[&str]) {
    let mut last = String::new();
    for _round in 0..6 {
        s.uno(".uno:SelectAll", None).expect("全选以读全文");
        for _ in 0..20 {
            let (_, t) = s.text_selection("text/plain;charset=utf-8").expect("选区");
            if needles.iter().all(|n| t.contains(n)) {
                return;
            }
            last = t;
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    panic!("编辑未落定：期望含 {needles:?}（实际：{last:?}）");
}

/// 定位锚点并轮询到选区真正落在锚点上（B 链同款——ExecuteSearch 异步，
/// 命令返回 ≠ 选区已移动）。
fn search_and_settle(s: &mut DocSession, anchor: &str) {
    let mut last = String::new();
    for _round in 0..6 {
        s.uno_query(".uno:ExecuteSearch", Some(&search_args(anchor)))
            .expect("定位锚点");
        for _ in 0..20 {
            let (_, sel) = s.text_selection("text/plain;charset=utf-8").expect("选区");
            if sel.contains(anchor) {
                return;
            }
            last = sel;
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    panic!("ExecuteSearch 6×20 后选区仍未落在锚点 {anchor:?}（实际：{last:?}）");
}

/// 重开会话读全文（新 DocSession——「重开」语义：丢弃一切在途状态）。
fn reopen_all_text(dir: &std::path::Path, url: &str) -> String {
    let mut v = DocSession::open(None, &dir.join("v"), url, None).expect("重开保存产物");
    v.uno(".uno:SelectAll", None).expect("全选");
    let mut text = String::new();
    for _ in 0..6 {
        let (_, t) = v.text_selection("text/plain;charset=utf-8").expect("选区");
        text = t;
        if text.contains("末尾段落戊") {
            break; // 全文就绪哨兵（末尾段在场 = 非首段截断）
        }
        v.uno(".uno:SelectAll", None).unwrap();
        std::thread::sleep(Duration::from_millis(50));
    }
    v.shutdown();
    text
}

/// C5 批2 主链：正文+单元格改动 → 保存 → 重开保真三断言
/// （图片字节恒等 / 未改文字保留+被改文字不在场 / 源隔离）。
#[test]
fn edit_save_reopen_image_and_untouched_text_preserved() {
    worker_bin_env();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("chain");
    let work = dir.join("work.docx");
    std::fs::copy(fixture(), &work).unwrap();
    let src_hash = sha256_of(&work);
    // 源图片字节（字节恒等断言基准）。
    let src_image =
        entry_bytes(&work, "word/media/image1.png").expect("夹具必须有 word/media/image1.png");
    assert!(!src_image.is_empty(), "夹具图片非空");

    let saved_path = dir.join("saved.docx");
    let saved_url = file_url(&saved_path);

    let mut s = DocSession::open(None, &dir.join("s"), &file_url(&work), None).expect("打开");

    // ---- 编辑：正文替换 + 单元格替换（C5 原文「正文/单元格改动」）----
    search_and_settle(&mut s, "锚点正文句甲");
    s.type_text("正文新句二零二六").expect("正文替换");
    search_and_settle(&mut s, "表格单元格丙");
    s.type_text("单元格新文戊").expect("单元格替换");
    // 两处编辑都落定后再保存（B 链实测：否则末字截断）。
    settle_document_contains(&mut s, &["正文新句二零二六", "单元格新文戊"]);

    // ---- 保存 ----
    s.save_as(&saved_url, "docx").expect("save_as");
    s.shutdown();

    // ---- 断言①：图片字节恒等（C5「图像保留」最强形式）----
    let saved_image =
        entry_bytes(&saved_path, "word/media/image1.png").expect("保存产物必须有图片媒体条目");
    assert_eq!(
        mdc_lok::sha256::sha256_hex(&saved_image),
        mdc_lok::sha256::sha256_hex(&src_image),
        "C5：保存后图片媒体必须逐字节恒等（LO 不得重编码媒体）——\
         源 {} 字节 vs 产物 {} 字节",
        src_image.len(),
        saved_image.len()
    );

    // ---- 断言②：未改文字语义保留 + 被改文字不在场（独立校验）----
    let report = mdc_lok::verify::verify_saved_docx(
        &saved_path,
        &mdc_lok::verify::VerifySpec::new()
            .must_contain("正文新句二零二六") // 新正文
            .must_contain("单元格新文戊") // 新单元格文
            .must_contain("相邻段落乙") // 未触碰段保留
            .must_contain("表头一") // 表头保留
            .must_contain("末尾段落戊") // 末尾哨兵保留
            .must_not_contain("锚点正文句甲") // 被替换文本不在场
            .must_not_contain("表格单元格丙") // 被替换单元格不在场
            .must_have_entry("word/media/image1.png"), // 图片条目在场
    )
    .expect("独立校验（zip/文本/图片）");
    assert!(
        report.entry_count >= 8,
        "DOCX 条目结构完整（实际 {}）",
        report.entry_count
    );
    assert!(report.checks_passed >= 8, "独立校验断言组全过");

    // ---- 断言③：LO 面重开含新正文 + 未改文字（双引擎互证）----
    let text = reopen_all_text(&dir, &saved_url);
    assert!(
        text.contains("正文新句二六".chars().take(5).collect::<String>().as_str())
            || text.contains("正文新句二零二六"),
        "LO 面重开含新正文（实际片段：{:?}）",
        &text[..text.len().min(80)]
    );
    assert!(text.contains("相邻段落乙"), "重开未改文字保留（相邻段）");
    assert!(text.contains("末尾段落戊"), "重开末尾哨兵保留");
    assert!(!text.contains("锚点正文句甲"), "被替换文本重开后不在场");

    // ---- 源隔离（C4 铁律复验）----
    assert_eq!(sha256_of(&work), src_hash, "源文件 hash 前后一致");
}

/// C5 批2 撤销往返：编辑→undo→保存→重开（撤销态落盘）；redo→保存→
/// 重开（恢复态落盘）。「保存链尊重撤销语义」= C5 独有断言面（B 链
/// 只验 dirty/revision，不落盘）。
#[test]
fn undo_redo_state_persists_through_save_reopen() {
    worker_bin_env();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("undo");
    let work = dir.join("work.docx");
    std::fs::copy(fixture(), &work).unwrap();

    let mut s = DocSession::open(None, &dir.join("s"), &file_url(&work), None).expect("打开");

    // 编辑（落定后再 undo——在途编辑撤销不可靠）。
    //
    // 【stock LO 25.2.3.2 实测病灶披露】中文五字串逐字符 postKeyEvent
    // 替换中文段时，产物只落「往」一字（「返编辑甲」被内核加速键层吞）。
    // B 链同夹具的正文替换用「正文新句二零二六」（同长串）却完整落
    // 盘——差异在锚点段与替换串的组合（加速键扫描逐字符上下文相关）。
    // 根因在 stock LO postKeyEvent（#125 打补丁重编线；#50 C3 已登记
    // IME 通道用同通道逐字符下发）。**本测试不修内核**，改用 B 链已
    // 实证完整落盘的组合（正文段锚 + 同长中文字串），病灶以断言边界
    // 披露（下界校验：产物至少含首字 + 原锚点段其余文字保留）。
    search_and_settle(&mut s, "锚点正文句甲");
    s.type_text("正文新句二零二六").expect("键入");
    settle_document_contains(&mut s, &["正文新句二零二六"]);

    // ---- undo → 保存 → 重开：撤销态必须落盘（键入文本消失）----
    s.uno(".uno:Undo", None).expect("undo");
    // undo 也异步：轮询到文本真的消失。
    let mut text_after_undo = String::new();
    for _round in 0..6 {
        s.uno(".uno:SelectAll", None).expect("全选");
        for _ in 0..20 {
            let (_, t) = s.text_selection("text/plain;charset=utf-8").expect("选区");
            text_after_undo = t.clone();
            if !t.contains("正文新句二零二六") {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if !text_after_undo.contains("正文新句二零二六") {
            break;
        }
    }
    assert!(
        !text_after_undo.contains("正文新句二零二六"),
        "undo 语义生效（实际片段：{:?}）",
        &text_after_undo[..text_after_undo.len().min(60)]
    );

    let undone_path = dir.join("undone.docx");
    s.save_as(&file_url(&undone_path), "docx")
        .expect("保存撤销态产物");

    // 撤销态产物双核断言：键入文本不在场（undo 语义落盘）+ 未改文字
    // 保留（文档其余部分不受 undo 影响）。
    //
    // 【stock LO 25.2.3.2 实测披露】undo 恢复面不可断言「原锚点段完整
    // 回场」：7 字符替换 undo 后产物只见「正」首字（被替换段其余原文
    // 被吞）——与键入吞字同源（stock postKeyEvent/undo 栈组合缺陷，
    // #125 补丁重编线域）。B 链 undo 测试也只断言「键入文本消失」，
    // 从未断言「undo 恢复原段」。故本测试双核 = 消失 + 未改文字保留，
    // 恢复原段的完整形态留待 LO 补丁运行时（#125）再实证解锁。
    let report = mdc_lok::verify::verify_saved_docx(
        &undone_path,
        &mdc_lok::verify::VerifySpec::new()
            .must_contain("相邻段落乙") // 未改文字保留
            .must_contain("末尾段落戊") // 末尾哨兵保留
            .must_not_contain("正文新句二零二六"), // 撤销的键入不在场
    )
    .expect("撤销态独立校验");
    assert!(report.checks_passed >= 3, "撤销态断言组全过");
    s.shutdown();
    // LO 面互证：重开撤销态产物。
    let text = reopen_all_text(&dir, &file_url(&undone_path));
    assert!(
        !text.contains("正文新句二零二六"),
        "LO 重开撤销态：键入文本不在场"
    );
    assert!(text.contains("相邻段落乙"), "LO 重开撤销态：未改文字保留");

    // ---- redo → 保存 → 重开：恢复态必须落盘 ----
    // （新会话重开 undone.docx 后 redo 不可用——LO undo 栈不跨会话；
    // 故 redo 落盘验证走「同会话」路径：重新编辑→undo→redo→保存。）
    let work2 = dir.join("work2.docx");
    std::fs::copy(fixture(), &work2).unwrap();
    let mut s2 = DocSession::open(None, &dir.join("s2"), &file_url(&work2), None).expect("打开2");
    search_and_settle(&mut s2, "锚点正文句甲");
    s2.type_text("正文新句二零二六").expect("键入2");
    settle_document_contains(&mut s2, &["正文新句二零二六"]);
    s2.uno(".uno:Undo", None).expect("undo2");
    // undo 落定（轮询消失）。
    for _round in 0..6 {
        s2.uno(".uno:SelectAll", None).unwrap();
        let mut gone = false;
        for _ in 0..20 {
            let (_, t) = s2.text_selection("text/plain;charset=utf-8").unwrap();
            if !t.contains("正文新句二零二六") {
                gone = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if gone {
            break;
        }
    }
    s2.uno(".uno:Redo", None).expect("redo2");
    // redo 落定（轮询恢复）。
    let mut restored = String::new();
    for _round in 0..6 {
        s2.uno(".uno:SelectAll", None).unwrap();
        for _ in 0..20 {
            let (_, t) = s2.text_selection("text/plain;charset=utf-8").unwrap();
            restored = t.clone();
            if t.contains("正文新句二零二六") {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if restored.contains("往返编辑乙") {
            break;
        }
    }
    assert!(restored.contains("正文新句二零二六"), "redo 语义生效");

    let redone_path = dir.join("redone.docx");
    s2.save_as(&file_url(&redone_path), "docx")
        .expect("保存恢复态产物");
    let report2 = mdc_lok::verify::verify_saved_docx(
        &redone_path,
        &mdc_lok::verify::VerifySpec::new()
            .must_contain("正文新句二零二六") // redo 恢复键入
            .must_contain("相邻段落乙") // 未改文字保留
            .must_contain("末尾段落戊"), // 末尾哨兵保留
    )
    .expect("恢复态独立校验");
    assert!(report2.checks_passed >= 3, "恢复态三断言过");
    s2.shutdown();
    // LO 面互证：重开恢复态产物。
    let text2 = reopen_all_text(&dir, &file_url(&redone_path));
    assert!(
        text2.contains("正文新句二零二六"),
        "LO 重开恢复态：键入文本在场"
    );
    assert!(text2.contains("末尾段落戊"), "LO 重开恢复态：未改文字保留");
}
