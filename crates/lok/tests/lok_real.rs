//! 真实 LibreOffice 运行时门（#633）——环境门后运行，缺运行时 SKIP。
//!
//! 诚实边界（AGENTS §13.3：SKIP ≠ PASS）：本文件证明「真实渲染 /
//! 真实键鼠编辑 / 真实 saveAs / 重开」。缺 LibreOffice 时打印横幅
//! 跳过；协议/边界语义已在 tests/boundary.rs 无运行时覆盖。
//! Windows 实机复跑清单：docs/lok-runtime.md。

use std::path::{Path, PathBuf};

use mdc_lok::client::LokRuntimeConfig;
use mdc_lok::ffi::lok_enums;
use mdc_lok::session::DocumentSession;
use mdc_lok::verify::VerifySpec;

fn runtime_program_dir() -> Option<PathBuf> {
    if let Some(dir) = mdc_lok::ffi::default_program_dirs()
        .into_iter()
        .find(|d| mdc_lok::ffi::pick_library(d).is_ok())
    {
        return Some(dir);
    }
    eprintln!(
        "[SKIP] 无 LibreOffice 运行时（MDC_LOK_DIR 未设且平台候选无库）——\
         真实渲染/编辑/保存门未执行（非 PASS）；发现方式见 docs/lok-runtime.md"
    );
    None
}

fn cfg(program_dir: &Path) -> LokRuntimeConfig {
    LokRuntimeConfig {
        program_dir: Some(program_dir.to_path_buf()),
        worker_bin: Some(PathBuf::from(env!("CARGO_BIN_EXE_mdc-lok-worker"))),
        ..Default::default()
    }
}

fn fixture_copy(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mdc-lok-real-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dst = dir.join("source.docx");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lok_smoke_source.docx"),
        &dst,
    )
    .unwrap();
    dst
}

#[test]
fn real_open_paint_edit_save_reopen_chain() {
    let Some(program_dir) = runtime_program_dir() else {
        return;
    };
    let src = fixture_copy("chain");
    let before = std::fs::read(&src).unwrap();

    let mut s = DocumentSession::open(&cfg(&program_dir), &src, false).unwrap();
    let (w, h) = s.document_size().unwrap();
    assert!(w > 5000 && h > 5000, "A4 文档尺寸异常：{w}x{h}");

    // 真实渲染（探针基线：400x300 非白 ≈1.97 万像素）。
    let tile = s.paint_tile(400, 300, 0, 0, 6000, 4500).unwrap();
    assert!(
        tile.nonwhite_pixels() > 500,
        "真实 tile 非空白像素过少（{}）——拒绝占位冒充",
        tile.nonwhite_pixels()
    );

    // 真实键鼠：双击选中 REPLACEME（中文正文中的 ASCII 词），键入中文。
    s.mouse_click(4300, 2050, 2).unwrap();
    let sel = s.text_selection().unwrap().unwrap_or_default();
    assert!(
        sel.contains("REPLACEME"),
        "双击未选中 REPLACEME（选区={sel:?}）"
    );
    s.type_text("换好了").unwrap();
    // 键入的失效事件帧在响应帧之后写出：等待（暂存不丢），再取全量。
    assert!(
        s.wait_for_event(
            lok_enums::CALLBACK_INVALIDATE_TILES,
            std::time::Duration::from_secs(3)
        ),
        "3s 内未见失效事件——键入未被应用？"
    );
    let events = s.drain_events();
    assert!(
        events
            .iter()
            .any(|e| e.kind == lok_enums::CALLBACK_INVALIDATE_TILES),
        "暂存事件里应含失效回调（wait_for_event 已确认在场）"
    );
    let tile2 = s.paint_tile(400, 300, 0, 0, 6000, 4500).unwrap();
    assert_ne!(tile.to_rgba(), tile2.to_rgba(), "编辑后像素应变化");

    // 单元格编辑 + 真实 saveAs + 结构校验（文本已变、图片条目仍在）。
    s.mouse_click(2500, 2700, 1).unwrap();
    s.type_text("改").unwrap();
    let receipt = s
        .save_edits_temp(
            &VerifySpec::new()
                .must_contain("换好了")
                .must_contain("改")
                .must_not_contain("REPLACEME")
                .must_have_entry("word/media/image1.png"),
        )
        .unwrap();
    assert!(s.is_dirty(), "临时保存后脏位保持");

    // 源文件只读纪律。
    assert_eq!(before, std::fs::read(&src).unwrap(), "源文件字节不得变化");

    // close → 重开已保存文件 → 真实渲染仍在。
    s.close().unwrap();
    let mut reopened =
        DocumentSession::open(&cfg(&program_dir), &receipt.temp_path, false).unwrap();
    let tile3 = reopened.paint_tile(400, 300, 0, 0, 6000, 4500).unwrap();
    assert!(tile3.nonwhite_pixels() > 500, "重开渲染近乎空白");
    reopened.close().unwrap();
}
