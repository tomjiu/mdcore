//! smoke —— #633 LOK 隔离 worker 原型全链冒烟（真实运行时）。
//!
//! 运行：`cargo run -p mdc-lok --example smoke [-- <输出目录>]`
//!（worker 二进制先 `cargo build -p mdc-lok`；或设 MDC_LOK_WORKER_BIN。）
//!
//! 链路 = #633 任务 3 的每一步，逐步落诊断文件（默认
//! `<workspace>/target/lok-smoke/`）：
//! 1. spawn + hello → 00-runtime.json（运行时/版本/nSize/库路径）
//! 2. load 真实 DOCX（中文 + 2x2 表格 + 内联 PNG 图）→ doc 元数据
//! 3. initializeForRendering + getDocumentSize → 尺寸日志
//! 4. setClientZoom + paintTile → 02-tile-before.png（非空白像素计数）
//! 5. 鼠标双击正文词 REPLACEME → 键入中文「换好了」→ 事件（失效回调）
//! 6. 等失效事件落地 → paintTile → 04-tile-after.png（与编辑前不同）
//! 7. 鼠标点击表格单元格 → 键入「改」 → 单元格编辑证据
//! 8. saveAs 临时 DOCX → 结构校验（文本已变/图片条目仍在）→ 05-save.json
//! 9. close → 重开已保存文件 → 06-reopen.png（编辑后内容真实可读）
//! 10. 源文件摘要前后对比（只读纪律）+ 故障注入面见 tests/boundary.rs
//!
//! 诚实边界：本冒烟在**本机 Linux LibreOffice 25.2** 实测；Windows
//! 实机门与运行时发现/补丁清单见 docs/lok-runtime.md。

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mdc_lok::client::LokRuntimeConfig;
use mdc_lok::ffi::lok_enums;
use mdc_lok::session::DocumentSession;
use mdc_lok::verify::VerifySpec;

fn main() {
    let out_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/lok-smoke"));
    if let Err(e) = run(&out_dir) {
        eprintln!("SMOKE FAIL: {e}");
        std::process::exit(1);
    }
    println!("SMOKE PASS —— 诊断文件在 {}", out_dir.display());
}

fn fnv1a(bytes: &[u8]) -> String {
    // 非加密摘要（快照对比用途：证明源文件字节未动，非安全场景）。
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn write_png(path: &Path, tile: &mdc_lok::session::Tile) -> std::io::Result<()> {
    let bmp = mdc_canvas::render::Bitmap {
        width: tile.width as u32,
        height: tile.height as u32,
        pixels: tile.to_rgba(),
    };
    let png = mdc_canvas::encode_png(&bmp).expect("PNG 编码");
    std::fs::write(path, png)
}

fn run(out_dir: &Path) -> mdc_lok::LokResult<()> {
    std::fs::create_dir_all(out_dir).ok();
    let mut log = String::new();
    let mut step = |name: &str, msg: String| {
        println!("[{name}] {msg}");
        log.push_str(&format!("[{name}] {msg}\n"));
    };

    // worker 二进制定位：env → 当前 exe 同目录（examples 的上级）。
    let worker_bin = std::env::var("MDC_LOK_WORKER_BIN")
        .map(PathBuf::from)
        .ok()
        .filter(|p| p.exists())
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|e| e.parent()?.parent().map(|d| d.join("mdc-lok-worker")))
                .filter(|p| p.exists())
        })
        .ok_or_else(|| {
            mdc_lok::LokError::new(
                mdc_lok::ErrorCode::RuntimeNotFound,
                "找不到 mdc-lok-worker 二进制：先 cargo build -p mdc-lok，\
                 或设 MDC_LOK_WORKER_BIN=<路径>",
            )
        })?;
    let cfg = LokRuntimeConfig {
        worker_bin: Some(worker_bin),
        ..Default::default()
    };

    // 夹具副本（源文件只读纪律；诊断目录里留档）。
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lok_smoke_source.docx");
    let src = out_dir.join("source-copy.docx");
    std::fs::copy(&fixture, &src)?;
    let src_before = std::fs::read(&src)?;
    step(
        "00",
        format!(
            "源文件副本 {}（{}B，fnv1a={}）",
            src.display(),
            src_before.len(),
            fnv1a(&src_before)
        ),
    );

    // 1-2. 打开（spawn+hello+load+initForRendering+setClientZoom）。
    let t0 = std::time::Instant::now();
    let mut s = DocumentSession::open(&cfg, &src, false)?;
    step(
        "01",
        format!("open 完成（{:?}，worker=真实 LOK unipoll）", t0.elapsed()),
    );

    // hello 元数据落盘（runtime/version/nSize）。
    let hello = s
        .call_worker("hello", &serde_json::Value::Null, Duration::from_secs(10))?
        .0;
    std::fs::write(
        out_dir.join("00-runtime.json"),
        serde_json::to_vec_pretty(&hello).unwrap(),
    )?;
    step(
        "02",
        format!(
            "runtime: {}",
            hello
                .pointer("/meta/version_info")
                .and_then(|v| v.as_str())
                .unwrap_or("(mock?)")
        ),
    );

    // 3. 文档尺寸。
    let (w, h) = s.document_size()?;
    step("03", format!("document size: {w} x {h} twips"));

    // 4. 编辑前 tile（页首区域：正文标题 + 第二段落）。
    let tile_before = s.paint_tile(400, 300, 0, 0, 6000, 4500)?;
    let nw_before = tile_before.nonwhite_pixels();
    write_png(&out_dir.join("02-tile-before.png"), &tile_before)?;
    step(
        "04",
        format!(
            "paintTile 编辑前 400x300 tileMode={} 非白像素={nw_before}",
            tile_before.tile_mode
        ),
    );
    if nw_before < 500 {
        return Err(mdc_lok::LokError::new(
            mdc_lok::ErrorCode::VerifyFailed,
            format!("tile 非空白像素过少（{nw_before}）——疑似空白渲染，拒绝以占位冒充"),
        ));
    }

    // 5. 双击 REPLACEME（第二段落；估算坐标 x≈4300 y≈2050 twips）→ 键入中文。
    s.mouse_click(4300, 2050, 2)?;
    let sel = s.text_selection()?.unwrap_or_default();
    step("05", format!("双击后选区文本：{sel:?}"));
    if !sel.contains("REPLACEME") {
        return Err(mdc_lok::LokError::new(
            mdc_lok::ErrorCode::VerifyFailed,
            format!("双击未选中 REPLACEME（选区={sel:?}）——鼠标定位或双击语义未生效"),
        ));
    }
    s.type_text("换好了")?;
    // 失效事件帧在响应帧之后写出：等待（暂存不丢）后取全量计数。
    let settled = s.wait_for_event(lok_enums::CALLBACK_INVALIDATE_TILES, Duration::from_secs(3));
    let events = s.drain_events();
    let invalidations = events
        .iter()
        .filter(|e| e.kind == lok_enums::CALLBACK_INVALIDATE_TILES)
        .count();
    let cursor_events = events
        .iter()
        .filter(|e| e.kind == lok_enums::CALLBACK_INVALIDATE_VISIBLE_CURSOR)
        .count();
    step(
        "06",
        format!(
            "键入「换好了」：事件共 {}（invalidate_tiles={invalidations}, cursor={cursor_events}，settled={settled}），dirty={} rev={}",
            events.len(),
            s.is_dirty(),
            s.revision()
        ),
    );
    if invalidations == 0 {
        return Err(mdc_lok::LokError::new(
            mdc_lok::ErrorCode::VerifyFailed,
            "编辑后未收到 LOK_CALLBACK_INVALIDATE_TILES——脏态回调链未接通",
        ));
    }

    // 6. 编辑后 tile（失效已落地）。
    let tile_after = s.paint_tile(400, 300, 0, 0, 6000, 4500)?;
    write_png(&out_dir.join("04-tile-after.png"), &tile_after)?;
    let pixels_changed = tile_before.to_rgba() != tile_after.to_rgba();
    step(
        "07",
        format!(
            "paintTile 编辑后非白像素={}，像素与编辑前不同={pixels_changed}",
            tile_after.nonwhite_pixels()
        ),
    );
    if !pixels_changed {
        return Err(mdc_lok::LokError::new(
            mdc_lok::ErrorCode::VerifyFailed,
            "编辑后 tile 像素与编辑前相同——渲染未反映编辑",
        ));
    }

    // 7. 表格单元格编辑：点击单元格甲（表格在第二段后，首行 y≈2600 起）。
    s.mouse_click(2500, 2700, 1)?;
    s.type_text("改")?;
    let events2 = s.drain_events();
    step(
        "08",
        format!("单元格点击+键入「改」：事件 {} 条", events2.len()),
    );

    // 8. saveAs 临时产物 + 校验。
    let spec = VerifySpec::new()
        .must_contain("换好了")
        .must_contain("改")
        .must_not_contain("REPLACEME")
        .must_have_entry("word/media/image1.png");
    let receipt = s.save_edits_temp(&spec)?;
    std::fs::write(
        out_dir.join("05-save.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "temp_path": receipt.temp_path.to_string_lossy(),
            "report": receipt.report,
        }))
        .unwrap(),
    )?;
    std::fs::copy(&receipt.temp_path, out_dir.join("05-saved-copy.docx"))?;
    step(
        "09",
        format!(
            "saveAs+校验通过：条目 {} 个，document.xml {}B，断言 {} 条全过",
            receipt.report.entry_count,
            receipt.report.document_xml_len,
            receipt.report.checks_passed
        ),
    );

    // 9. 源文件只读纪律。
    let src_after = std::fs::read(&src)?;
    let digest_after = fnv1a(&src_after);
    step(
        "10",
        format!(
            "源文件事后 fnv1a={digest_after}（与事前相等={}）",
            digest_after == fnv1a(&src_before)
        ),
    );
    if src_after != src_before {
        return Err(mdc_lok::LokError::new(
            mdc_lok::ErrorCode::VerifyFailed,
            "源文件字节在会话期间被改动——违反只读纪律",
        ));
    }

    // 10. close → 重开已保存文件 → 渲染证明内容真实可读。
    s.close()?;
    let mut reopened = DocumentSession::open(&cfg, &receipt.temp_path, false)?;
    let (rw, rh) = reopened.document_size()?;
    let tile_reopen = reopened.paint_tile(400, 300, 0, 0, 6000, 4500)?;
    write_png(&out_dir.join("06-reopen.png"), &tile_reopen)?;
    step(
        "11",
        format!(
            "重开已保存文件：size {rw}x{rh}，tile 非白像素 {}",
            tile_reopen.nonwhite_pixels()
        ),
    );
    if tile_reopen.nonwhite_pixels() < 500 {
        return Err(mdc_lok::LokError::new(
            mdc_lok::ErrorCode::VerifyFailed,
            "重开文件渲染近乎空白——保存产物可疑",
        ));
    }
    reopened.close()?;

    log.push_str("SMOKE PASS\n");
    let mut f = std::fs::File::create(out_dir.join("07-summary.log"))?;
    f.write_all(log.as_bytes())?;
    Ok(())
}
