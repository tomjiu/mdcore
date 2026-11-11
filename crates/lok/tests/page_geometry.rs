//! page_geometry 多页契约（#49 B 线 → #50 C 线跳页/页码数据源）。
//!
//! 三页夹具（three-pages.docx，显式分页）在**真实 LOK 运行时**下锁定
//! 25.2.3.2 实测语义（2026-11-11 取证）：
//! - 页矩形在**画布坐标系**（与 paintTile 的文档坐标同源）；
//! - 画布 = 页 + 四周 284 twips 边距，页间隙同为 284；
//! - `page_count == 3`、页序单调、页隙 == 边距（跨版本漂移哨兵——
//!   LO 26.x 若改变此行为，本测试亮黄牌触发重实证，防 C 线跳页错位）。
//!
//! 环境门（AGENTS §13 纪律）：无 LibreOffice 运行时时打印横幅 SKIP。

use mdc_lok::client::DocSession;
use std::path::PathBuf;

fn runtime_available() -> bool {
    mdc_lok::runtime::discover().is_ok()
}

fn fixture() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/three-pages.docx")
}

#[test]
fn three_page_geometry_contract() {
    if let Some(w) = option_env!("CARGO_BIN_EXE_mdc-lok-worker") {
        std::env::set_var("MDC_LOK_WORKER_BIN", w);
    }
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时（page_geometry 多页契约未执行）===");
        eprintln!("=== 补救：安装 LibreOffice 或设 MDC_LOK_HOME=<安装根> ===");
        return;
    }
    let dir = std::env::temp_dir().join(format!("mdc-lok-pg3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let work = dir.join("work.docx");
    std::fs::copy(fixture(), &work).unwrap();
    let canon = std::fs::canonicalize(&work).unwrap();
    let url = format!("file://{}", canon.to_string_lossy());

    let s = DocSession::open(None, &dir.join("s"), &url, None).expect("DocSession::open");
    let (canvas_w, canvas_h) = s.size().expect("size（画布尺寸）");
    let pages = s.page_geometry().expect("page_geometry");

    // ---- 页数与页尺寸 ----
    assert_eq!(pages.len(), 3, "三页夹具必须报 3 页（实测 {pages:?}）");
    for (i, p) in pages.iter().enumerate() {
        assert_eq!(
            (p.w, p.h),
            (11906, 16838),
            "第 {} 页 A4 尺寸（实测 {:?}）",
            i + 1,
            p
        );
    }

    // ---- 画布坐标系：边距 284（四周）+ 页隙 284（25.2.3.2 实测）----
    const BORDER: i64 = 284;
    assert_eq!(pages[0].x, BORDER, "首页左边距 = 284");
    assert_eq!(pages[0].y, BORDER, "首页顶边距 = 284");
    assert_eq!(
        canvas_w,
        BORDER + pages[0].w + BORDER,
        "画布宽 = 左边距 + 页宽 + 右边距"
    );
    assert_eq!(
        canvas_h,
        pages.last().unwrap().y + pages.last().unwrap().h + BORDER,
        "画布高 = 末页底 + 下边距"
    );
    for w in pages.windows(2) {
        assert_eq!(
            w[1].y - (w[0].y + w[0].h),
            BORDER,
            "页隙 = 284（跳页滚动锚定的间隙参数）"
        );
    }

    // ---- 页序（跳页目标 y 的直接数据源）----
    let ys: Vec<i64> = pages.iter().map(|p| p.y).collect();
    assert_eq!(ys, vec![284, 17406, 34528], "三页 y 序列（实证值回归）");

    // ---- 与 getDocumentSize 的自洽（C 线不做第二次几何推导的依据）----
    assert!(pages.iter().all(|p| p.x + p.w <= canvas_w));
    assert!(
        pages.last().unwrap().y + pages.last().unwrap().h <= canvas_h,
        "末页底在画布内"
    );

    s.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
