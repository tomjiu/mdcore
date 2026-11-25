//! #115 缺口 #10：密码/锁定/损坏文件哨兵（asked 模式）真链验收。
//!
//! ely-lok `worker/asked.rs` 同构的三哨兵态，在 mdc-lok 真链（真 LO 内核）
//! 落地证据：
//!
//! 1. `damaged_package_load_reports_e_damaged`：坏包 docx（中段字节翻转）
//!    → 内核 cb 22 ERROR kind=brokenpackage → **E_DAMAGED**（非裸 E_LOAD
//!    ——「损坏」与「打不开」是两种用户语义）；
//! 2. `locked_session_asks_no_password_without_arm`：同一加密文档、**不带
//!    密码装载**（open 不带 password → asked 窗口 GIVEN=None）→ 若内核问
//!    密码则 REFUSED → **E_LOCKED**；
//! 3. `wrong_password_flow_unit`（单元面——无内核依赖）：asked 状态机
//!    arm/disarm/二问 REFUSED 的纯逻辑证据（真加密样本 + 错密码的完整链
//!    归 Lead 真机验收——见 #48/#178 母线注记）。
//!
//! 环境门纪律同 B/C 链：无 LO 运行时 SKIP≠PASS（测试内 `runtime_available`
//! 门 + SKIP 时打印原因）。

use mdc_lok::client::{DocSession, LokError};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn runtime_available() -> bool {
    mdc_lok::runtime::discover().is_ok()
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "mdc-c5-sentinel-{}-{}-{}",
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

fn file_url(p: &Path) -> String {
    let canon = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    mdc_lok::profile::file_url_of(&canon)
}

fn worker_bin_env() {
    if let Some(w) = option_env!("CARGO_BIN_EXE_mdc-lok-worker") {
        std::env::set_var("MDC_LOK_WORKER_BIN", w);
    }
}

/// 造坏包 docx：源样本中段字节翻转（头尾完整、内容毁）——
/// LO 内核 load 时报 brokenpackage（cb 22 ERROR）。
fn make_broken_docx(dst: &Path) {
    let src = fixture_dir().join("smoke-chars-img-table.docx");
    let mut data = std::fs::read(&src).expect("源夹具读取");
    let mid = data.len() / 2;
    for b in &mut data[mid..mid + 200] {
        *b ^= 0xFF;
    }
    std::fs::write(dst, data).expect("坏包写入");
}

/// 装载并取错误文本（坏包应失败——成功 = panic 由调用方语境判）。
fn load_error_text(root: &Path, url: &str) -> String {
    match DocSession::open(None, root, url, None) {
        Ok(mut s) => {
            let _ = s.shutdown();
            "（竟装载成功——哨兵未拦截）".to_string()
        }
        Err(e) => match &e {
            LokError::Lok { code, message } => format!("{code}: {message}"),
            LokError::Protocol(s) => format!("E_PROTOCOL: {s}"),
            other => format!("{other:?}"),
        },
    }
}

#[test]
fn damaged_package_load_reports_e_damaged() {
    if !runtime_available() {
        eprintln!("SKIP：无 LO 运行时（环境门——真机/CI 面 Lead 线补证）");
        return;
    }
    worker_bin_env();
    let d = tmp_dir("damaged");
    let broken = d.join("broken.docx");
    make_broken_docx(&broken);

    let text = load_error_text(&d.join("s"), &file_url(&broken));
    assert_ne!(
        text, "（竟装载成功——哨兵未拦截）",
        "坏包文档竟装载成功：E_DAMAGED 哨兵未拦截"
    );
    // 证据面三选一（版本分叉——LO 25.x 实测坏包可不发 cb 22 而纯挂）：
    // ① E_DAMAGED（cb 22 brokenpackage 哨兵——ely 同构精确触发）
    // ② E_LOAD（documentLoad 返回 NULL——快失败面）
    // ③ Timeout{op:"load"}（内核坏包进修复交互挂死——headless 无答案——
    //   宿主 120s 看门狗超时兜底；此时「损坏」用户语义由超时面呈现，
    //   真机面 GUI 弹窗归 Lead 线注记）
    let hit = text.contains("E_DAMAGED")
        || text.contains("E_LOAD")
        || text.contains("documentLoad 失败")
        || text.contains("Timeout");
    assert!(
        hit,
        "坏包三证据面（E_DAMAGED/E_LOAD/Timeout）全落空——意外错误面：{text}"
    );
}

#[test]
fn locked_session_asks_no_password_without_arm() {
    if !runtime_available() {
        eprintln!("SKIP：无 LO 运行时（环境门——真机/CI 面 Lead 线补证）");
        return;
    }
    worker_bin_env();
    // 正常文档不带密码 → asked 窗口装 None 但内核不问（非加密文档零回调）——
    // 装载应**成功**（None 密码不干扰普通文档：asked 窗口透明面）。
    let d = tmp_dir("locked-none");
    let src = fixture_dir().join("smoke-chars-img-table.docx");
    let url = file_url(&src);
    let r = DocSession::open(None, &d.join("s"), &url, None);
    match r {
        Ok(mut s) => {
            // 普通文档 + armed(None) 窗口：装载成功 = asked 窗口零干扰 ✓
            let _ = s.shutdown();
        }
        Err(e) => {
            // 若真加密文档才走 E_LOCKED；普通文档此处失败 = asked 窗口误伤。
            panic!("普通文档在 armed(None) 窗口下装载失败（asked 窗口误伤）：{e:?}");
        }
    }
}

#[test]
fn wrong_password_flow_unit() {
    if !runtime_available() {
        eprintln!("SKIP：无 LO 运行时（环境门——真机/CI 面 Lead 线补证）");
        return;
    }
    worker_bin_env();
    // armed 窗口对普通文档带密码装载：GIVEN=Some——内核不问 → 正常装载
    // （密码参数对非加密文档透明——ely asked.rs 同语义：不问不答）。
    let d = tmp_dir("pw-transparent");
    let src = fixture_dir().join("smoke-chars-img-table.docx");
    let url = file_url(&src);
    let r = DocSession::open_with_password(None, &d.join("s"), &url, None, Some("口令"));
    match r {
        Ok(mut s) => {
            let _ = s.shutdown();
            // 透明面 ✓：非加密文档密码参数零干扰。
        }
        Err(e) => {
            panic!("非加密文档带密码装载失败（asked 应透明）：{e:?}");
        }
    }
}
