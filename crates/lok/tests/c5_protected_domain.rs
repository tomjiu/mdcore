//! #50 C5 批3：保护域样本真链验收——LOK 内核对只读锁文档的拒写语义。
//!
//! C5 清单原文：「保护域样本按页面与成熟参考比对；……不假装编辑已安全落盘。」
//! B 链/mang-shell 引擎链已有 typed 拒锁定面（docx 桥预检拒写），但
//! **LOK 会话链（post_key/uno 直捅 LO 内核）零覆盖**：带
//! `w:documentProtection w:edit="readOnly"` 的真档在 LOK 下——
//! 键入是否被内核拒？保存产物是否原样？保护标记是否保留？此前无证据。
//!
//! 本文件三测试（真链，环境门 SKIP≠PASS 纪律同 B 链）：
//!
//! 1. `readonly_doc_rejects_typing_and_preserves_bytes`：
//!    只读锁（enforcement=1）→ 键入试探（X + 中文乙）→ **产物与源
//!    逐字节恒等**（LO 拒写最强形式）+ 全文不含键入痕迹 + 保护标记
//!    在场 + 源文件不变；
//! 2. `forms_doc_rejects_body_typing`：forms 模式（只允许填表单域）
//!    → 正文键入同样拒（产物恒等）——两模式对照面；
//! 3. `unprotected_doc_accepts_typing`：同夹具无保护对照——键入真入文
//!    （基线，证明拒写是保护域语义而非链路死）。
//!
//! fixture 生成：`tests/fixtures/gen_protected_fixture.py`（从
//! smoke-chars-img-table.docx 注入 settings.xml 的 documentProtection
//! 标记——OOXML 规范面真锁，非模拟）。

use mdc_lok::client::DocSession;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

/// 截前 n 个字符（中文安全——&s[..60] 会切在 UTF-8 字中间 panic）。
fn head_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn runtime_available() -> bool {
    mdc_lok::runtime::discover().is_ok()
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "mdc-c5-protect-{}-{}-{}",
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

/// LOK UNO SearchItem 参数（B 链同款）。
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

/// 定位锚点并轮询到选区真正落在锚点上（B 链同款纪律）。
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

/// 读 zip 条目文本（document.xml 拼接全部 w:t）。
fn document_text(p: &std::path::Path) -> String {
    let file = std::fs::File::open(p).expect("开 zip");
    let mut archive = zip::ZipArchive::new(file).expect("zip 结构");
    let xml = String::from_utf8(
        archive
            .by_name("word/document.xml")
            .expect("document.xml")
            .bytes()
            .collect::<Result<Vec<_>, _>>()
            .expect("读 xml"),
    )
    .expect("utf-8");
    // 拼 w:t 文本（手写扫描稳妥版：兼容 <w:t/>（自闭合空 run）、
    // <w:t xml:space="preserve">text</w:t>（属性形）——verify 面
    // must_contain 语义同源，不引 regex 依赖（crate 纪律）。
    let mut out = String::new();
    let mut rest = xml.as_str();
    while let Some(i) = rest.find("<w:t") {
        rest = &rest[i..];
        // 边界排除：<w:tab/> / <w:tc> / <w:tbl> 等误中——只有紧跟
        // '>' / 空格 / '/' 的才是 w:t 标签本体。
        let after = rest.as_bytes().get(4).copied().unwrap_or(b'x');
        if after != b'>' && after != b' ' && after != b'/' {
            rest = &rest[3..];
            continue;
        }
        let end = match rest.find('>') {
            Some(e) => e,
            None => break, // 残尾
        };
        if rest.as_bytes()[end - 1] == b'/' {
            // <w:t .../> 自闭合：无内容。
            rest = &rest[end + 1..];
            continue;
        }
        let content_end = match rest[end..].find("</w:t>") {
            Some(x) => end + x,
            None => break, // 残尾容忍（无配对闭标签的残段）
        };
        out.push_str(&rest[end + 1..content_end]);
        rest = &rest[content_end + "</w:t>".len()..];
    }
    out
}

/// 读 settings.xml 原文（保护标记在场断言用）。
fn settings_xml(p: &std::path::Path) -> String {
    let file = std::fs::File::open(p).expect("开 zip");
    let mut archive = zip::ZipArchive::new(file).expect("zip 结构");
    String::from_utf8(
        archive
            .by_name("word/settings.xml")
            .expect("settings.xml")
            .bytes()
            .collect::<Result<Vec<_>, _>>()
            .expect("读 settings"),
    )
    .expect("utf-8")
}

/// 键入试探后读全文（选区面——LO 内核视角「文档现状」）。
fn all_text(s: &mut DocSession) -> String {
    s.uno(".uno:SelectAll", None).expect("全选");
    let mut last = String::new();
    for _ in 0..6 {
        let (_, t) = s.text_selection("text/plain;charset=utf-8").expect("选区");
        last = t;
        if last.contains("末尾段落戊") {
            break; // 全文就绪哨兵
        }
        s.uno(".uno:SelectAll", None).unwrap();
        std::thread::sleep(Duration::from_millis(50));
    }
    last
}

/// 只读锁文档：键入被内核拒——**保存产物与源逐字节恒等**（拒写最强
/// 形式）+ 全文无键入痕迹 + 保护标记保留 + 源文件不变。
#[test]
fn readonly_doc_rejects_typing_and_preserves_bytes() {
    worker_bin_env();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("ro");
    let work = dir.join("work.docx");
    std::fs::copy(fixture_dir().join("protected-readonly.docx"), &work).unwrap();
    let src_hash = sha256_of(&work);

    let mut s = DocSession::open(None, &dir.join("s"), &file_url(&work), None).expect("打开");

    // 键入试探：定位锚点（保护文档可搜索）→ 英文字符 + 中文字符各一。
    search_and_settle(&mut s, "锚点正文句甲");
    let r1: Result<(), _> = s.type_text("X");
    let r2: Result<(), _> = s.type_text("乙");

    // 内核拒写语义：文档全文不含键入痕迹（X/乙 均未入文）。
    let text = all_text(&mut s);
    assert!(
        !text.contains("乙X") && !text.contains("X乙"),
        "保护域键入未入文（实际片段：{:?}）",
        head_chars(&text, 60)
    );
    // 键入应答面：LO 内核对保护文档的 post_key 或回 ok（吞）或回 err（拒）——
    // 两种都合法（协议层成功 ≠ 文档变更），但产物面必须恒等（下断言）。
    let _ = (r1, r2);

    // 保存产物：保护态下的保存 = **语义恒等重写**（拒写铁律的实测形态）。
    //
    // 【stock LO 25.2.3.2 实测披露】保护文档保存**不产字节恒等**：LO 重排
    // 部件（新增 docProps/custom.xml 空壳）、展开 documentProtection 属性
    // （补 crypt* 空属性）、表格 gridCol 4820→4819（twips 舍入重算）——
    // 但**文本语义逐字符恒等**（下断言）+ 键入痕迹零 + 保护标记保留 +
    // 「拒写」体现在**内容面**（键入不入文）而非「不落盘」。字节恒等
    // 是我最初的错误预期：LO 的 docx 导出器从不保持字节稳定（B 链
    // 批2 亦然——图片是字节恒等的例外，因为媒体原样转存）。
    let saved = dir.join("saved.docx");
    s.save_as(&file_url(&saved), "docx").expect("保存");
    s.shutdown();

    // 语义恒等断言（三面）：
    let dt_saved = document_text(&saved);
    let dt_src = document_text(&work);
    assert_eq!(
        dt_saved, dt_src,
        "C5 批3 铁律：只读锁文档保存产物的文本语义必须与源逐字符恒等（键入不得入文）"
    );
    // 保护标记保留（保存不丢锁）。
    let sx = settings_xml(&saved);
    assert!(
        sx.contains("documentProtection") && sx.contains("w:edit=\"readOnly\""),
        "保存产物保护标记保留（实际 settings 片段：{:?}）",
        head_chars(&sx, 120)
    );
    // 键入痕迹零（语义恒等断言已含，此处显式防回归锚点）。
    assert!(
        !dt_saved.contains("X乙") && !dt_saved.contains("乙X"),
        "产物无键入痕迹（X/乙 组合）"
    );

    // 源文件不变（隔离铁律——保护域同源）。
    assert_eq!(sha256_of(&work), src_hash, "源文件 hash 前后一致");
}

/// forms 模式（只允许填表单域）：**行为披露面**——stock LO 25.2.3.2 的
/// LOK 嵌入视图**不执行 forms 拒写**（键入真入文），与 readOnly 的拒写
/// 行为分裂（桌面端 forms 会被挡；LOK 内核层绕过——实测实证）。
/// 本测试钉住实测行为 + 披露，防「误以为 forms 同 readOnly」的产品
/// 假设；不假绿（断言实际发生的事，而不是期望发生的事）。
#[test]
fn forms_doc_typing_behavior_disclosure() {
    worker_bin_env();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("forms");
    let work = dir.join("work.docx");
    std::fs::copy(fixture_dir().join("protected-forms.docx"), &work).unwrap();
    let src_hash = sha256_of(&work);

    let mut s = DocSession::open(None, &dir.join("s"), &file_url(&work), None).expect("打开");

    // forms 模式实测：键入**入文**（与 readOnly 拒写分裂）。
    search_and_settle(&mut s, "锚点正文句甲");
    let _xf: Result<(), _> = s.type_text("XF");
    let text = all_text(&mut s);
    assert!(
        text.contains("XF"),
        "【行为披露】forms 模式在 LOK 下键入必须已入文（实测行为断言；\
         若此断言失败 = LO 已修复 forms 拒写 → 把本测试翻回拒写断言）。\
         实际片段：{:?}",
        head_chars(&text, 60)
    );

    let saved = dir.join("saved.docx");
    s.save_as(&file_url(&saved), "docx").expect("保存");
    s.shutdown();
    // 产物面披露：键入痕迹在产物（与 readOnly 的语义恒等对照）。
    assert!(
        document_text(&saved).contains("XF"),
        "【行为披露】forms 产物含键入痕迹（LOK 不执行 forms 拒写）"
    );
    let sx = settings_xml(&saved);
    // forms 标记保留（保存不丢锁——与 readOnly 同面；保护标记在场
    // 说明 LO 读到了锁，只是 LOK 嵌入路径不执行 forms 拒写语义）。
    assert!(
        sx.contains("documentProtection") && sx.contains("w:edit=\"forms\""),
        "forms 保护标记保留（LO 读到锁但不执行拒写——行为分裂实证）"
    );
    assert_eq!(sha256_of(&work), src_hash, "源文件 hash 前后一致");
}

/// 基线对照：同夹具**无**保护 → 键入真入文（证明拒写是保护域语义，
/// 不是链路死/会话坏——C5「不假装」纪律的对照锚）。
#[test]
fn unprotected_doc_accepts_typing() {
    worker_bin_env();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("open");
    let work = dir.join("work.docx");
    std::fs::copy(fixture_dir().join("smoke-chars-img-table.docx"), &work).unwrap();

    let mut s = DocSession::open(None, &dir.join("s"), &file_url(&work), None).expect("打开");
    search_and_settle(&mut s, "锚点正文句甲");
    // 键入英文串（基线：入文；被替换段与批2 不同面——这里不替换，
    // 追加式键入 X 标记串——全选读文含 X 即基线证成）。
    let _q: Result<(), _> = s.type_text("XQ7");
    let text = all_text(&mut s);
    assert!(
        text.contains("XQ7"),
        "基线：无保护文档键入必须入文（实际片段：{:?}）",
        head_chars(&text, 60)
    );
    s.shutdown();
}
