//! IME 中文输入契约（#49 B 线 → #50 C 线）。
//!
//! ## 实证结论（LibreOffice 25.2.3.2，2026-11-11，六流探针取证）
//!
//! **内核 preedit 通道（槽 #40 postWindowExtTextInputEvent）在 stock
//! 25.2.3.2 上语义不可靠，不作为产品 IME 通道**：
//! - 「进程首个 LOK 会话」：TEXTINPUT 组合串被当**定稿文本**插入
//!   （preedit 语义破缺），END 载荷被忽略；
//! - 「后续会话」：事件被**静默丢弃**（零效果）；
//! - 两种行为都不符合 IME 契约（组合不下定稿、定稿替换组合）。
//!   ElyOffice ely-lok 的可靠 postText 依赖其补丁版 LO（比对报告
//!   #115 §3 patches/ 发现），stock 内核不可复现。
//!
//! **产品 IME 通道 = A 线批次 12（#90）架构**：preedit 由 GPUI 宿主
//! 渲染（组合期键直输卫已就位），定稿串经 **type_text（postKeyEvent
//! 逐字符）** 下发——本测试锁定该通道的中文落盘全链（smoke 已证
//! 编辑链，此处锁 IME 语义面：定稿 = 编辑类，dirty/revision 前进、
//! 保存重开仍在）。
//!
//! `post_text` op 保留（机械面已验证：帧到达内核、守卫生效、编辑类
//! 语义正确）——待 LO 版本升级或补丁运行时重实证后在契约文档 §10
//! 解锁。环境门（AGENTS §13）：无运行时 SKIP 横幅。

use mdc_lok::client::DocSession;
use std::path::PathBuf;

fn runtime_available() -> bool {
    mdc_lok::runtime::discover().is_ok()
}

fn fixture() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/smoke-chars-img-table.docx")
}

/// 选全文文本（表格内首 SelectAll 可能只选单元格——session_contract
/// 同款重试）。
fn full_text(s: &mut DocSession, needle: &str) -> String {
    let mut text = String::new();
    for _ in 0..4 {
        s.uno(".uno:SelectAll", None).expect("SelectAll");
        let (_, t) = s
            .text_selection("text/plain;charset=utf-8")
            .expect("text_selection");
        text = t;
        if text.contains(needle) {
            break;
        }
    }
    text
}

#[test]
fn ime_commit_via_type_text_lands_and_persists() {
    if let Some(w) = option_env!("CARGO_BIN_EXE_mdc-lok-worker") {
        std::env::set_var("MDC_LOK_WORKER_BIN", w);
    }
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时（IME 契约未执行）===");
        eprintln!("=== 补救：安装 LibreOffice 或设 MDC_LOK_HOME=<安装根> ===");
        return;
    }
    let dir = std::env::temp_dir().join(format!("mdc-lok-ime-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let work = dir.join("work.docx");
    std::fs::copy(fixture(), &work).unwrap();
    let canon = std::fs::canonicalize(&work).unwrap();
    let url = mdc_lok::profile::file_url_of(&canon);

    let mut s = DocSession::open(None, &dir.join("s"), &url, None).expect("DocSession::open");
    let rev0 = s.revision();

    // ---- IME 定稿下发（产品通道：GPUI preedit 定稿 → type_text）----
    s.type_text("拼音定稿中文文本").expect("type_text 定稿串");

    // ---- 编辑类语义：dirty 置位、revision 前进 ----
    assert!(s.is_dirty(), "IME 定稿是编辑：dirty=true");
    assert!(
        s.revision() > rev0,
        "IME 定稿 revision 前进（{} > {}）",
        s.revision(),
        rev0
    );

    // ---- 定稿文本落进正文 ----
    let text = full_text(&mut s, "拼音定稿中文文本");
    assert!(
        text.contains("拼音定稿中文文本"),
        "IME 定稿文本必须落进正文（实际片段：{:?}）",
        text.chars().take(60).collect::<String>()
    );

    // ---- 保存重开：落盘链含定稿文本 ----
    let saved_url = {
        let dir_canon = std::fs::canonicalize(&dir).unwrap();
        mdc_lok::profile::file_url_of(&dir_canon.join("saved.docx"))
    };
    s.save_as(&saved_url, "docx").expect("save_as");
    let mut s2 = DocSession::open(None, &dir.join("s2"), &saved_url, None).expect("重开");
    let text2 = full_text(&mut s2, "拼音定稿中文文本");
    assert!(
        text2.contains("拼音定稿中文文本"),
        "保存重开后 IME 定稿仍在"
    );
    s.shutdown();
    s2.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

/// post_text（槽 #40）机械面：op 到达内核、编辑类语义正确、worker 存活。
/// **不**断言文本落地（stock 25.2.3.2 语义不可靠——见模块文档实证记录；
/// 文本落地断言待 LO 升级/补丁运行时重实证后恢复）。
#[test]
fn post_text_wire_mechanics_and_edit_semantics() {
    if let Some(w) = option_env!("CARGO_BIN_EXE_mdc-lok-worker") {
        std::env::set_var("MDC_LOK_WORKER_BIN", w);
    }
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时（post_text 机械面未执行）===");
        return;
    }
    let dir = std::env::temp_dir().join(format!("mdc-lok-pt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let work = dir.join("work.docx");
    std::fs::copy(fixture(), &work).unwrap();
    let canon = std::fs::canonicalize(&work).unwrap();
    let url = mdc_lok::profile::file_url_of(&canon);

    let mut s = DocSession::open(None, &dir.join("s"), &url, None).expect("DocSession::open");
    let rev0 = s.revision();

    // 两帧都应答成功（组合 + 提交）——内核接收面与守卫面工作正常
    s.post_text(false, "preedit").expect("post_text 组合帧");
    s.post_text(true, "commit").expect("post_text 提交帧");

    // 编辑类语义（宿主侧保守置位——契约 §5）
    assert!(s.is_dirty());
    assert!(s.revision() > rev0);

    // worker 存活且可继续服务（无成员缺失/崩溃）
    let (w, h) = s.size().expect("post_text 后 worker 仍可服务");
    assert!(w > 0 && h > 0);
    s.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
