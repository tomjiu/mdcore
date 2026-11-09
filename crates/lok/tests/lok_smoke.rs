//! LOK 真实运行时 smoke（#633 验收链；环境门控：缺运行时 → SKIP 横幅，
//! 不算失败——CI 无 LibreOffice 时绿；有运行时的环境全链实测）。
//!
//! 复跑：`cargo test -p mdc-lok --test lok_smoke -- --nocapture`
//! 或独立跑：`cargo run -p mdc-lok --bin mdc-lok-smoke -- --out /tmp/lok-out`

use std::path::PathBuf;

#[test]
fn lok_smoke_full_chain() {
    let fixture: PathBuf = mdc_lok::smoke_fixture_path();

    // 环境门（AGENTS §13.1-13.4：探测 + SKIP 横幅 + 补救指引，不静默）
    let probe = mdc_lok::runtime::discover();
    if probe.is_err() {
        eprintln!(
            "[lok] SKIP lok_smoke_full_chain —— 本机无 LibreOffice 运行时（真实门未执行；\
             协议/超时/帧限/死亡面恒跑层见 tests/protocol.rs 全绿）。\
             补救：安装 LibreOffice 或设 MDC_LOK_HOME=<安装根>。原因：{}",
            probe.err().unwrap_or_default()
        );
        return;
    }
    if !fixture.is_file() {
        panic!(
            "夹具缺失：{fixture:?}（git lfs / 重新生成：\
             python3 crates/lok/tests/fixtures/gen_smoke_fixture.py）"
        );
    }

    let out = std::env::temp_dir().join(format!("mdc-lok-smoke-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let worker = option_env!("CARGO_BIN_EXE_mdc-lok-worker").map(PathBuf::from);
    let report = mdc_lok::smoke::run(&fixture, &out, worker.as_deref())
        .expect("smoke 链路本体不应崩（步骤失败由 report.ok 表达）");

    let failed: Vec<&str> = report
        .steps
        .iter()
        .filter(|s| !s.ok)
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        report.ok,
        "LOK smoke 有失败步骤：{:?}；诊断目录 {}（steps.jsonl / report.json / tile-*.bmp）",
        failed,
        report.out_dir.display()
    );
}
