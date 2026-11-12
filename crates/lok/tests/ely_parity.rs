//! #115 比对缺口三个 P1 小项的落地验收（ely-lok 同构移植，#49 B 线）：
//!
//! - **缺口 #7 关闭类加速键卫**：Ctrl+W/Q/F4 key_code 拒发（内核直发会
//!   **关内核自己的帧**——文档凭空消失、后续 paint 全黑）；无 Mod1 同键
//!   与普通键不拦（卫只拦关闭类组合键）。
//! - **缺口 #8 首屏布局落定**：load 后立刻 `.uno:Repaginate`（LOK-03——
//!   后续 paint 不吃半布局态）；响应带 `repaginated` 标志（best-effort：
//!   非 Writer 文档不挂 load）。
//! - **缺口 #9 save 选项白名单**：`TakeOwnership[,TerminateEditing]` 所有权
//!   族；未知选项 BAD_REQUEST（防把过滤器参数误当所有权语义）；失败保存
//!   不清脏位、成功才清。
//!
//! 环境门：无 LibreOffice 运行时 SKIP（≠PASS）。

use mdc_lok::client::{DocSession, WorkerClient};
use mdc_lok::protocol::codes;
use serde_json::json;
use std::path::PathBuf;
use std::time::Duration;

fn runtime_available() -> bool {
    mdc_lok::runtime::discover().is_ok()
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke-chars-img-table.docx")
}

/// 夹具复制到 tmp（**既有约定**：全部 LO 测试先复制再加载——LO 打开写
/// `.~lock` 锁；worker 硬杀（SIGKILL）后锁残留 → 同路径重开 = **只读
/// 模式**，postKeyEvent 回 ok 但编辑静默丢弃。本文件三测试并行跑，
/// 直接加载原夹具还会**互相锁**（两个 worker 同路径）——复制品各走
/// 各的 tmp，锁随目录清理。
/// **第二坑**：spawn 会清空 session_root——work 文件必须放 session_root
/// **之外**（session 一律 `dir.join("s")` 子目录，work 在 `dir` 下）。
fn work_fixture(dir: &std::path::Path) -> PathBuf {
    let work = dir.join("work.docx");
    std::fs::copy(fixture(), &work).expect("复制夹具");
    work
}

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mdc-lok-115parity-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn file_url(p: &std::path::Path) -> String {
    let canon = std::fs::canonicalize(p).unwrap();
    format!("file://{}", canon.to_string_lossy())
}

/// 保存目标 URL（目标尚不存在——canonicalize 目录再拼文件名，既有模式）。
fn target_url(dir: &std::path::Path, name: &str) -> String {
    let canon = std::fs::canonicalize(dir).unwrap();
    format!("file://{}", canon.join(name).to_string_lossy())
}

fn worker_bin_env() {
    if let Some(w) = option_env!("CARGO_BIN_EXE_mdc-lok-worker") {
        std::env::set_var("MDC_LOK_WORKER_BIN", w);
    }
}

/// 缺口 #7：关闭类加速键拒发——内核不得关自己的帧；拒发后 worker 与
/// 文档双活（普通编辑照常）；卫不过度拦截（无 Mod1 的 W 放行）。
#[test]
fn closing_accelerators_rejected_kernel_frame_stays() {
    worker_bin_env();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("closing");
    let rt = mdc_lok::runtime::discover().expect("discover（环境门已过）");
    let w = WorkerClient::spawn(Some(&rt), &dir.join("s"), None, None).expect("spawn");
    w.request_with_timeout(
        "load",
        json!({"url": file_url(&work_fixture(&dir))}),
        Duration::from_secs(30),
    )
    .expect("load 夹具");

    // Ctrl(Mod1=0x2000) + W(534)/Q(771)/F4(528)：三个关闭类加速键全拒
    for kc in [534i32, 771, 528] {
        let err = w
            .request_with_timeout(
                "post_key",
                json!({"char": 0, "key_code": 0x2000 | kc}),
                Duration::from_secs(10),
            )
            .expect_err("关闭类加速键必须拒发");
        assert_eq!(err.code(), codes::BAD_REQUEST, "key_code={kc}: {err}");
        let msg = format!("{err}");
        assert!(msg.contains("关闭类"), "拒发理由应点名关闭类：{msg}");
        assert!(!w.is_dead(), "拒发不杀 worker（key_code={kc}）");
    }

    // 拒发后文档仍可编辑：普通键直发照常（'a'）
    w.request_with_timeout(
        "post_key",
        json!({"char": 97, "key_code": 0}),
        Duration::from_secs(10),
    )
    .expect("普通键不拦（文档双活证据）");

    // 卫不过度拦截：无 Mod1 的裸 W（key_code=534）放行
    w.request_with_timeout(
        "post_key",
        json!({"char": 0, "key_code": 534}),
        Duration::from_secs(10),
    )
    .expect("无 Mod1 的 W 放行（卫只拦 Ctrl 组合）");

    w.kill();
}

/// 缺口 #8：load 响应带 `repaginated` 标志（首屏布局落定，LOK-03）。
#[test]
fn load_repaginates_initial_layout() {
    worker_bin_env();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("repaginate");
    let rt = mdc_lok::runtime::discover().expect("discover（环境门已过）");
    let w = WorkerClient::spawn(Some(&rt), &dir.join("s"), None, None).expect("spawn");
    let v = w
        .request_with_timeout(
            "load",
            json!({"url": file_url(&work_fixture(&dir))}),
            Duration::from_secs(30),
        )
        .expect("load 夹具");
    assert!(
        v.get("repaginated")
            .and_then(serde_json::Value::as_bool)
            .is_some(),
        "load 响应必须带 repaginated 标志：{v}"
    );
    assert_eq!(
        v["repaginated"],
        json!(true),
        "Writer docx 的 Repaginate 应成功（stock 25.2.3.2）：{v}"
    );
    // 后续 paint 链照常（布局落定不破坏正常渲染面）
    w.request_with_timeout("ping", json!({}), Duration::from_secs(10))
        .expect("load 后 worker 照常服务");
    w.kill();
}

/// 缺口 #9：save 选项白名单——未知选项 BAD_REQUEST + 脏位保留；
/// TakeOwnership 成功清脏位；空串与无选项等价默认。
#[test]
fn save_options_whitelist_take_ownership() {
    worker_bin_env();
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时 ===");
        return;
    }
    let dir = tmp_dir("saveopts");
    let mut s = DocSession::open(None, &dir.join("s"), &file_url(&work_fixture(&dir)), None)
        .expect("open");
    s.type_text("白名单").expect("编辑置脏");
    assert!(s.is_dirty(), "编辑后必脏");

    // 未知选项：拒 + 脏位保留 + worker 活
    let bogus = dir.join("bogus.docx");
    let err = s
        .save_as_with_options(
            &target_url(&dir, "bogus.docx"),
            "docx",
            Some("CSV 44,34,76,1,,0,false,true,true"),
        )
        .expect_err("未知 save 选项必须拒");
    assert_eq!(err.code(), codes::BAD_REQUEST, "{err}");
    assert!(s.is_dirty(), "失败保存不清脏位");
    assert!(!bogus.exists(), "拒发即无产物");

    // TakeOwnership：成功 + 清脏位 + 产物落盘
    let owned = dir.join("owned.docx");
    s.save_as_with_options(
        &target_url(&dir, "owned.docx"),
        "docx",
        Some("TakeOwnership"),
    )
    .expect("TakeOwnership 是白名单档");
    assert!(!s.is_dirty(), "成功保存清脏位");
    assert!(owned.exists(), "产物真实落盘");

    // 空串 = 默认档；无选项（save_as 裸面）回归
    s.type_text("二").expect("再编辑");
    let empty = dir.join("empty.docx");
    s.save_as_with_options(&target_url(&dir, "empty.docx"), "docx", Some(""))
        .expect("空串允许（= 默认档）");
    assert!(empty.exists());
    s.type_text("三").expect("三编辑");
    let plain = dir.join("plain.docx");
    s.save_as(&target_url(&dir, "plain.docx"), "docx")
        .expect("裸 save_as 回归");
    assert!(!s.is_dirty());
    assert!(plain.exists());

    s.shutdown();
}
