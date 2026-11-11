//! smoke 全链路（issue 第 3/4 条 + 验收四条）。
//!
//! 链路：发现运行时 → 复制夹具（原文只读不动）→ worker1 打开 → 初始化渲染
//! → 页面尺寸 → paintTile（非空像素证明）→ 搜索锚文本（LOK UNO）→
//! 键盘替换正文 + 表格单元格 → 回调脏态（INVALIDATE_TILES）与重绘 →
//! 全文校验 → 只读位保存失败（脏位保留）→ 另存新 DOCX（脏位清零）→
//! 双 worker 并行（独立 profile 不互锁）→ worker2 重开校验（文本已变 /
//! 相邻内容与图片仍在）→ 超时看门狗 → worker 被杀 → 宿主仍可操作。
//!
//! 诊断产物（out_dir）：`steps.jsonl`（逐步结果）、`report.json`（总报）、
//! `tile-*.bmp`（真实像素，人眼可验）、`original.sha256`（原文指纹前后对照）。

use crate::client::{DocHandle, Event, LokError, WorkerClient};
use crate::protocol::codes;
use crate::runtime::{self, LokRuntime};
use serde_json::{json, Value};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// twips → 像素换算基准（LO 标准 15 twips/px @ 96dpi）。
pub const TWIPS_PER_PX: i64 = 15;
/// 全页 paint 的画布上限（像素）。
pub const MAX_CANVAS_PX: i64 = 1024;

/// 逐步记录。
#[derive(Debug, Clone)]
pub struct SmokeStep {
    pub name: String,
    pub ok: bool,
    pub detail: Value,
}

/// smoke 总报。
pub struct SmokeReport {
    pub ok: bool,
    pub runtime: Option<LokRuntime>,
    pub steps: Vec<SmokeStep>,
    pub out_dir: PathBuf,
}

struct Logger {
    steps: Vec<SmokeStep>,
    steps_file: std::fs::File,
    out_dir: PathBuf,
}

impl Logger {
    fn new(out_dir: &Path) -> Result<Logger, String> {
        let _ = std::fs::create_dir_all(out_dir);
        let steps_file =
            std::fs::File::create(out_dir.join("steps.jsonl")).map_err(|e| e.to_string())?;
        Ok(Logger {
            steps: Vec::new(),
            steps_file,
            out_dir: out_dir.to_path_buf(),
        })
    }

    fn step(&mut self, name: &str, ok: bool, detail: Value) -> bool {
        println!(
            "[smoke] {} {} — {}",
            if ok { "OK  " } else { "FAIL" },
            name,
            serde_json::to_string(&detail).unwrap_or_default()
        );
        let step = SmokeStep {
            name: name.to_string(),
            ok,
            detail: detail.clone(),
        };
        let line = json!({"step": name, "ok": ok, "detail": detail});
        let _ = writeln!(self.steps_file, "{line}");
        self.steps.push(step);
        ok
    }

    /// 记录并断言（FAIL 即返回 false，由 run 汇总）。
    fn check(&mut self, name: &str, ok: bool, detail: Value) -> bool {
        self.step(name, ok, detail)
    }
}

/// 运行 smoke 全链路。返回总报（ok=false 表示有 FAIL 步）。
pub fn run(
    fixture: &Path,
    out_dir: &Path,
    worker_bin: Option<&Path>,
) -> Result<SmokeReport, String> {
    let mut log = Logger::new(out_dir)?;
    let mut overall = true;

    // ---- 0. 运行时发现（版本/来源/hash 必须留痕）----
    let rt = match runtime::discover() {
        Ok(rt) => {
            log.step(
                "runtime.discover",
                true,
                json!({
                    "program_dir": rt.program_dir.to_string_lossy(),
                    "kernel_lib": rt.kernel_lib.to_string_lossy(),
                    "kernel_sha256": rt.kernel_sha256,
                }),
            );
            rt
        }
        Err(e) => {
            log.step("runtime.discover", false, json!({"error": e}));
            return Ok(SmokeReport {
                ok: false,
                runtime: None,
                steps: log.steps,
                out_dir: log.out_dir,
            });
        }
    };

    // ---- 1. 原文保真：复制夹具，全程只编辑副本 ----
    let work = out_dir.join("work.docx");
    std::fs::copy(fixture, &work).map_err(|e| format!("复制夹具失败：{e}"))?;
    let original_sha_before = crate::sha256::sha256_hex_file(fixture).map_err(|e| e.to_string())?;
    std::fs::write(out_dir.join("original.sha256"), &original_sha_before).ok();
    log.step(
        "fixture.copy",
        true,
        json!({
            "fixture": fixture.to_string_lossy(),
            "work_copy": work.to_string_lossy(),
            "original_sha256": original_sha_before,
        }),
    );

    let work_url = {
        let canon = std::fs::canonicalize(&work).map_err(|e| e.to_string())?;
        crate::profile::file_url_of(&canon)
    };

    // ---- 2. worker1 spawn + ready 事件 ----
    let s1 = out_dir.join("session-1");
    let w1 = WorkerClient::spawn(Some(&rt), &s1, worker_bin, None)
        .map_err(|e| format!("spawn worker1 失败：{e}"))?;
    let ev1 = w1.take_events().ok_or("worker1 事件通道被占")?;
    let mut ev1 = EventTap {
        rx: ev1,
        seen: Vec::new(),
    };
    let ready = wait_ready(&w1, &mut ev1, Duration::from_secs(180));
    overall &= log.check(
        "worker1.ready",
        ready.is_some(),
        json!({"ready_detail": ready.clone().unwrap_or(json!({"error": "180s 内未见 ready 事件（boot-failed？）"}))}),
    );
    let ready = match ready {
        Some(v) => v,
        None => {
            w1.kill();
            return Ok(SmokeReport {
                ok: false,
                runtime: Some(rt),
                steps: log.steps,
                out_dir: log.out_dir,
            });
        }
    };

    // ---- 3. 版本（LOK getVersionInfo 原文）----
    let version = w1.request("version", json!({})).ok().and_then(|v| {
        v.get("version_info")
            .and_then(Value::as_str)
            .map(str::to_owned)
    });
    let version_ok = version.as_deref().is_some_and(|v| !v.is_empty());
    overall &= log.check(
        "worker1.version",
        version_ok,
        json!({
            "version_info": version.clone().unwrap_or_default(),
            "kernel_sha256": rt.kernel_sha256,
            "ready": ready,
        }),
    );

    // ---- 4. 打开副本（documentLoadWithOptions）----
    let doc = match DocHandle::load(&w1, &work_url, Some("DeviceFormFactor=desktop")) {
        Ok(d) => d,
        Err(e) => {
            log.check("doc.load", false, json!({"error": e.to_string()}));
            w1.kill();
            return Ok(SmokeReport {
                ok: false,
                runtime: Some(rt),
                steps: log.steps,
                out_dir: log.out_dir,
            });
        }
    };
    overall &= log.step("doc.load", true, json!({"url": work_url}));

    // ---- 5. 初始化渲染 ----
    match doc.init_rendering(None) {
        Ok(_) => overall &= log.step("doc.init_rendering", true, json!({})),
        Err(e) => {
            overall &= log.check("doc.init_rendering", false, json!({"error": e.to_string()}));
        }
    }

    // ---- 6. 页面尺寸（twips，必须非零）----
    let (w_tw, h_tw) = doc
        .size()
        .map_err(|e| format!("getDocumentSize 失败：{e}"))?;
    overall &= log.check(
        "doc.size",
        w_tw > 0 && h_tw > 0,
        json!({"width_twips": w_tw, "height_twips": h_tw}),
    );

    // ---- 6b. 页几何（getPartPageRectangles：跳页/页码数据源，C 线缺口）----
    // 25.2.3.2 实测语义：页矩形在**画布坐标系**（与 paintTile 同源），
    // 画布 = 页 + 四周 284 twips 边距、页隙 284。验收口径：非空、每页
    // 宽高为正、y 单调不重叠、统一 x（单栏）、页在画布内、边距对称、
    // A4 页尺寸（夹具确定性）。
    match doc.page_geometry() {
        Ok(pages) => {
            let all_positive = pages.iter().all(|p| p.w > 0 && p.h > 0);
            let monotonic = pages.windows(2).all(|w| w[1].y >= w[0].y + w[0].h);
            let uniform_x = pages.iter().map(|p| p.x).min() == pages.iter().map(|p| p.x).max();
            let within_canvas = pages.iter().all(|p| p.x + p.w <= w_tw)
                && pages.last().is_some_and(|p| p.y + p.h <= h_tw);
            let first = pages.first();
            let last = pages.last();
            let border_symmetric = first.is_some_and(|f| {
                f.x == w_tw - f.x - f.w && last.is_some_and(|l| f.y == h_tw - l.y - l.h)
            });
            let a4 = first.is_some_and(|f| f.w == 11906 && f.h == 16838);
            overall &= log.check(
                "doc.page_geometry",
                !pages.is_empty()
                    && all_positive
                    && monotonic
                    && uniform_x
                    && within_canvas
                    && border_symmetric
                    && a4,
                json!({
                    "page_count": pages.len(),
                    "pages": pages.iter().map(|p|
                        json!({"x": p.x, "y": p.y, "w": p.w, "h": p.h})
                    ).collect::<Vec<_>>(),
                    "checks": {
                        "non_empty": !pages.is_empty(),
                        "all_positive": all_positive,
                        "y_monotonic_nonoverlap": monotonic,
                        "uniform_x": uniform_x,
                        "within_canvas": within_canvas,
                        "border_symmetric": border_symmetric,
                        "a4_page": a4,
                    },
                    "doc_size_twips": [w_tw, h_tw],
                }),
            );
        }
        Err(e) => {
            overall &= log.check("doc.page_geometry", false, json!({"error": e.to_string()}));
        }
    }

    // ---- 7. paintTile 首块（左上 256x256；非空像素证明）----
    let t1 = doc
        .paint(
            256,
            256,
            0,
            0,
            (256 * TWIPS_PER_PX as usize) as i32,
            (256 * TWIPS_PER_PX as usize) as i32,
        )
        .map_err(|e| format!("paintTile 失败：{e}"))?;
    let s1_unique = t1["stats"]["unique_colors"].as_u64().unwrap_or(0);
    let s1_nonwhite = t1["stats"]["non_white_ratio"].as_f64().unwrap_or(0.0);
    overall &= log.check(
        "doc.paint_first_tile",
        s1_unique >= 8 && s1_nonwhite > 0.005,
        json!({
            "tile": t1["tile_path"], "sha256_bgra": t1["sha256_bgra"],
            "unique_colors": s1_unique, "non_white_ratio": s1_nonwhite,
            "note": "unique>=8 且 non_white>0.5% 证明真实渲染（非 1x1 占位/空白）",
        }),
    );

    // ---- 8. paintTile 全页（画布映射整页；中文/图片/表格必须实际展示）----
    let scale_tw = TWIPS_PER_PX;
    let canvas_w = (w_tw / scale_tw).clamp(16, MAX_CANVAS_PX) as usize;
    let canvas_h = (h_tw / scale_tw).clamp(16, MAX_CANVAS_PX) as usize;
    let t2 = doc
        .paint(canvas_w, canvas_h, 0, 0, w_tw as i32, h_tw as i32)
        .map_err(|e| format!("全页 paintTile 失败：{e}"))?;
    let colored = t2["stats"]["colored_ratio"].as_f64().unwrap_or(0.0);
    overall &= log.check(
        "doc.paint_full_page",
        colored >= 0.0005,
        json!({
            "canvas": [canvas_w, canvas_h],
            "tile": t2["tile_path"],
            "colored_ratio": colored,
            "unique_colors": t2["stats"]["unique_colors"],
            "note": "colored>=0.05% 证明彩色内容（夹具红蓝探针图片）实际渲染",
        }),
    );

    // ---- 9. 正文编辑：搜索锚文本（LOK UNO）→ 键盘替换 ----
    let anchor = "锚点正文句甲";
    let replacement_body = "新正文句二零二六";
    let search_args = json!({
        "SearchItem.SearchString": {"type": "string", "value": anchor},
        "SearchItem.Backward": {"type": "boolean", "value": "false"},
        "SearchItem.Command": {"type": "unsigned short", "value": "0"},
        "SearchItem.SearchFlags": {"type": "long", "value": "0"},
        "SearchItem.TransliterateFlags": {"type": "long", "value": "0"},
        "SearchItem.AlgorithmType2": {"type": "short", "value": "1"},
    })
    .to_string();
    doc.uno_query(".uno:ExecuteSearch", Some(&search_args))
        .map_err(|e| format!("ExecuteSearch 失败：{e}"))?;
    let found = ev1.wait_callback(
        crate::abi::callbacks::SEARCH_RESULT_SELECTION,
        Duration::from_secs(15),
    );
    let not_found = ev1.saw(crate::abi::callbacks::SEARCH_NOT_FOUND);
    overall &= log.check(
        "doc.search_anchor",
        found && !not_found,
        json!({
            "anchor": anchor,
            "SEARCH_RESULT_SELECTION_seen": found,
            "SEARCH_NOT_FOUND_seen": not_found,
        }),
    );

    doc.type_text(replacement_body)
        .map_err(|e| format!("键盘输入正文失败：{e}"))?;
    overall &= log.check(
        "doc.type_body_replacement",
        doc.is_dirty(),
        json!({"replacement": replacement_body, "dirty": doc.is_dirty()}),
    );

    // ---- 10. 回调脏态：键盘编辑后必须有 INVALIDATE_TILES（重绘需要）----
    let invalidated = ev1.wait_callback(
        crate::abi::callbacks::INVALIDATE_TILES,
        Duration::from_secs(20),
    );
    overall &= log.check(
        "cb.invalidate_tiles_after_edit",
        invalidated,
        json!({
            "seen_types": ev1.seen_names(),
            "note": "编辑后必须收到 INVALIDATE_TILES（脏态重绘信号）",
        }),
    );

    // ---- 11. 表格单元格编辑：搜索单元格锚 → 键盘替换 ----
    let cell_anchor = "表格单元格丙";
    let replacement_cell = "新单元格戊";
    let search_args2 = json!({
        "SearchItem.SearchString": {"type": "string", "value": cell_anchor},
        "SearchItem.Backward": {"type": "boolean", "value": "false"},
        "SearchItem.Command": {"type": "unsigned short", "value": "0"},
        "SearchItem.SearchFlags": {"type": "long", "value": "0"},
        "SearchItem.TransliterateFlags": {"type": "long", "value": "0"},
        "SearchItem.AlgorithmType2": {"type": "short", "value": "1"},
    })
    .to_string();
    doc.uno_query(".uno:ExecuteSearch", Some(&search_args2))
        .map_err(|e| format!("单元格搜索失败：{e}"))?;
    let cell_found = ev1.wait_callback(
        crate::abi::callbacks::SEARCH_RESULT_SELECTION,
        Duration::from_secs(15),
    );
    doc.type_text(replacement_cell)
        .map_err(|e| format!("键盘输入单元格失败：{e}"))?;
    overall &= log.check(
        "doc.type_cell_replacement",
        cell_found,
        json!({"cell_anchor": cell_anchor, "replacement": replacement_cell, "search_seen": cell_found}),
    );

    // ---- 12. 保存前全文校验（SelectAll + getTextSelection）----
    // Writer 经典行为：表格内首个 SelectAll 只选单元格→第 2 次选表→
    // 第 3 次才全文；重试到含全部期望标记（上限 4 次）。
    let (plain_all, html_all) = select_all_full_text(&doc, replacement_body, replacement_cell)
        .map_err(|e| format!("全文选取失败：{e}"))?;
    let pre_ok = plain_all.contains(replacement_body)
        && plain_all.contains(replacement_cell)
        && plain_all.contains("相邻段落乙")
        && html_all.contains("<img")
        && html_all.contains("<table");
    overall &= log.check(
        "doc.pre_save_fulltext",
        pre_ok,
        json!({
            "plain_len": plain_all.len(),
            "has_replacement_body": plain_all.contains(replacement_body),
            "has_replacement_cell": plain_all.contains(replacement_cell),
            "has_neighbor_para": plain_all.contains("相邻段落乙"),
            "html_has_img": html_all.contains("<img"),
            "html_has_table": html_all.contains("<table"),
        }),
    );

    // ---- 13. 保存目录拒写：目标是「父路径为普通文件」的非法目录 ----
    //    （chmod 拒写在 root 下不可靠；父为文件时任何用户都建不出目标）
    let blocker = out_dir.join("blocker.txt");
    std::fs::write(&blocker, b"not a dir").map_err(|e| e.to_string())?;
    let blocked_url = {
        let canon = std::fs::canonicalize(&blocker).map_err(|e| e.to_string())?;
        crate::profile::file_url_of(&canon.join("child.docx"))
    };
    let save_denied = match doc.save_as(&blocked_url, "docx") {
        Err(LokError::Lok { code, message }) if code == codes::SAVE => {
            json!({"code": code, "message": message, "dirty_after": doc.is_dirty()})
        }
        other => json!({
            "unexpected": match &other {
                Ok(v) => serde_json::to_string(v).unwrap_or_default(),
                Err(e) => e.to_string(),
            },
            "dirty_after": doc.is_dirty(),
        }),
    };
    let denied_ok = save_denied.get("code").and_then(Value::as_str) == Some(codes::SAVE);
    let dirty_kept = doc.is_dirty();
    overall &= log.check(
        "save.denied_keeps_dirty",
        denied_ok && dirty_kept,
        save_denied,
    );

    // ---- 14. 另存新 DOCX（成功才清脏位）----
    let edited = out_dir.join("edited.docx");
    let edited_url = {
        // canonicalize 要求存在：用父目录 canonicalize + 文件名拼（url 唯一即可）
        let parent = std::fs::canonicalize(out_dir).map_err(|e| e.to_string())?;
        crate::profile::file_url_of(&parent.join("edited.docx"))
    };
    match doc.save_as(&edited_url, "docx") {
        Ok(v) => {
            let dirty_cleared = !doc.is_dirty();
            let file_exists = edited.is_file()
                && std::fs::metadata(&edited)
                    .map(|m| m.len() > 1000)
                    .unwrap_or(false);
            overall &= log.check(
                "save.as_new_docx",
                dirty_cleared && file_exists,
                json!({"saved": v, "dirty_cleared": dirty_cleared, "file": edited.to_string_lossy(), "bytes": std::fs::metadata(&edited).map(|m| m.len()).unwrap_or(0)}),
            );
        }
        Err(e) => {
            overall &= log.check(
                "save.as_new_docx",
                false,
                json!({"error": e.to_string(), "dirty_kept": doc.is_dirty()}),
            );
        }
    }

    // ---- 15. 原文保真终检（全程只动副本）----
    let original_sha_after = crate::sha256::sha256_hex_file(fixture).map_err(|e| e.to_string())?;
    overall &= log.check(
        "fixture.untouched",
        original_sha_before == original_sha_after,
        json!({"before": original_sha_before, "after": original_sha_after}),
    );

    // ---- 16. 双 worker 并行（独立 profile 不互锁）+ worker2 重开校验 ----
    let s2 = out_dir.join("session-2");
    let w2 = WorkerClient::spawn(Some(&rt), &s2, worker_bin, None)
        .map_err(|e| format!("spawn worker2 失败（并行会话）：{e}"))?;
    let ev2 = w2.take_events().ok_or("worker2 事件通道被占")?;
    let mut ev2 = EventTap {
        rx: ev2,
        seen: Vec::new(),
    };
    let ready2 = wait_ready(&w2, &mut ev2, Duration::from_secs(180));
    overall &= log.check(
        "worker2.parallel_ready",
        ready2.is_some(),
        json!({
            "ready": ready2,
            "note": "worker1 存活期间 worker2 用独立 profile 启动——多会话不互锁",
        }),
    );
    let edited_url2 = {
        let parent = std::fs::canonicalize(out_dir).map_err(|e| e.to_string())?;
        crate::profile::file_url_of(&parent.join("edited.docx"))
    };
    let doc2 = match DocHandle::load(&w2, &edited_url2, Some("DeviceFormFactor=desktop")) {
        Ok(d) => d,
        Err(e) => {
            overall &= log.check("reopen.load_edited", false, json!({"error": e.to_string()}));
            w1.kill();
            w2.kill();
            return Ok(SmokeReport {
                ok: overall,
                runtime: Some(rt),
                steps: log.steps,
                out_dir: log.out_dir,
            });
        }
    };
    overall &= log.step("reopen.load_edited", true, json!({"url": edited_url2}));
    doc2.init_rendering(None)
        .map_err(|e| format!("worker2 init 失败：{e}"))?;
    let (plain2, html2) = select_all_full_text(&doc2, replacement_body, replacement_cell)
        .map_err(|e| format!("重开全文选取失败：{e}"))?;
    let reopen_ok = plain2.contains(replacement_body)
        && plain2.contains(replacement_cell)
        && plain2.contains("相邻段落乙")
        && !plain2.contains(anchor)
        && !plain2.contains(cell_anchor)
        && html2.contains("<img")
        && html2.contains("<table");
    overall &= log.check(
        "reopen.verify_text_img_table",
        reopen_ok,
        json!({
            "replacement_body_kept": plain2.contains(replacement_body),
            "replacement_cell_kept": plain2.contains(replacement_cell),
            "neighbor_para_kept": plain2.contains("相邻段落乙"),
            "old_anchor_gone": !plain2.contains(anchor),
            "old_cell_gone": !plain2.contains(cell_anchor),
            "img_kept": html2.contains("<img"),
            "table_kept": html2.contains("<table"),
        }),
    );
    // worker2 全页重绘（图片在保存往返后仍实际渲染）
    let (w2t, h2t) = doc2.size().unwrap_or((0, 0));
    if w2t > 0 && h2t > 0 {
        let cw = (w2t / TWIPS_PER_PX).clamp(16, MAX_CANVAS_PX) as usize;
        let ch = (h2t / TWIPS_PER_PX).clamp(16, MAX_CANVAS_PX) as usize;
        if let Ok(t3) = doc2.paint(cw, ch, 0, 0, w2t as i32, h2t as i32) {
            let colored2 = t3["stats"]["colored_ratio"].as_f64().unwrap_or(0.0);
            overall &= log.check(
                "reopen.paint_full_page",
                colored2 >= 0.0005,
                json!({"colored_ratio": colored2, "tile": t3["tile_path"]}),
            );
        }
    }

    // ---- 17. worker1 友好关机（此时 worker2 仍活着——并行证明完成）----
    w1.shutdown();
    overall &= log.step(
        "worker1.shutdown",
        true,
        json!({"note": "worker2 存活期间关机"}),
    );

    // ---- 18. 超时看门狗：1ns 超时 → Timeout 错误 + worker 被杀 + 原文不变 ----
    let timeout_result = w2.request_with_timeout("ping", json!({}), Duration::from_nanos(1));
    let timeout_ok = matches!(&timeout_result, Err(LokError::Timeout { .. }));
    overall &= log.check(
        "watchdog.timeout_kills_worker",
        timeout_ok && w2.is_dead(),
        json!({
            "result": match &timeout_result {
                Ok(v) => serde_json::to_string(v).unwrap_or_default(),
                Err(e) => e.to_string(),
            },
            "worker_dead": w2.is_dead(),
        }),
    );
    let original_sha_final = crate::sha256::sha256_hex_file(fixture).map_err(|e| e.to_string())?;
    overall &= log.check(
        "watchdog.source_untouched",
        original_sha_before == original_sha_final,
        json!({"before": original_sha_before, "after": original_sha_final}),
    );

    // ---- 19. worker 被杀（外部终止）→ 明确错误 + 宿主仍可操作 ----
    let s3 = out_dir.join("session-3");
    let w3 = WorkerClient::spawn(Some(&rt), &s3, worker_bin, None)
        .map_err(|e| format!("spawn worker3 失败：{e}"))?;
    {
        // 打开 + 编辑一笔（脏位宿主侧语义在 DocHandle；此处杀进程看错误面）
        let parent = std::fs::canonicalize(out_dir).map_err(|e| e.to_string())?;
        let url3 = crate::profile::file_url_of(&parent.join("work.docx"));
        if let Ok(doc3) = DocHandle::load(&w3, &url3, Some("DeviceFormFactor=desktop")) {
            let _ = doc3.type_text("杀前临时编辑");
        }
    }
    let w3_pid = w3.session_pid();
    w3.kill(); // 外部终止（模拟内核崩溃 / OOM kill）
    let died_result = w3.request("ping", json!({}));
    let died_ok = matches!(&died_result, Err(LokError::WorkerDied { .. }));
    overall &= log.check(
        "crash.worker_died_clear_error",
        died_ok,
        json!({
            "result": match &died_result {
                Ok(v) => serde_json::to_string(v).unwrap_or_default(),
                Err(e) => e.to_string(),
            },
            "killed_pid": w3_pid,
        }),
    );

    // ---- 20. 宿主仍可操作：新会话 ping ----
    let s4 = out_dir.join("session-4");
    let w4 = WorkerClient::spawn(Some(&rt), &s4, worker_bin, None)
        .map_err(|e| format!("spawn worker4 失败：{e}"))?;
    let host_alive = w4.request("ping", json!({})).is_ok();
    overall &= log.check(
        "host.still_operable",
        host_alive,
        json!({"ping": host_alive}),
    );
    w4.shutdown();

    // ---- 总报 ----
    let failed: Vec<&str> = log
        .steps
        .iter()
        .filter(|s| !s.ok)
        .map(|s| s.name.as_str())
        .collect();
    let report = json!({
        "ok": overall,
        "runtime": {
            "program_dir": rt.program_dir.to_string_lossy(),
            "kernel_lib": rt.kernel_lib.to_string_lossy(),
            "kernel_sha256": rt.kernel_sha256,
        },
        "steps_total": log.steps.len(),
        "steps_failed": failed,
        "out_dir": log.out_dir.to_string_lossy(),
        "note": "Windows/macOS 实测门未执行（见 crate lib.rs 真实门声明）",
    });
    std::fs::write(
        log.out_dir.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap_or_default(),
    )
    .map_err(|e| e.to_string())?;
    println!(
        "\n[smoke] 总报：{}（{} 步，失败 {}）",
        if overall { "PASS" } else { "FAIL" },
        log.steps.len(),
        failed.len()
    );
    if !failed.is_empty() {
        println!("[smoke] 失败步骤：{}", failed.join("、"));
    }

    Ok(SmokeReport {
        ok: overall,
        runtime: Some(rt),
        steps: log.steps,
        out_dir: log.out_dir,
    })
}

/// SelectAll + getTextSelection：重试直到选取覆盖全部期望标记
/// （Writer 表格内首个 SelectAll 只选单元格——实测行为）。
fn select_all_full_text(
    doc: &DocHandle,
    replacement_body: &str,
    replacement_cell: &str,
) -> Result<(String, String), String> {
    for attempt in 1..=4 {
        doc.uno_query(".uno:SelectAll", None)
            .map_err(|e| format!("SelectAll(#{attempt}) 失败：{e}"))?;
        let (_, plain) = doc
            .text_selection("text/plain;charset=utf-8")
            .map_err(|e| format!("getTextSelection(plain #{attempt}) 失败：{e}"))?;
        if plain.contains(replacement_body)
            && plain.contains(replacement_cell)
            && plain.contains("相邻段落乙")
        {
            let (_, html) = doc
                .text_selection("text/html")
                .map_err(|e| format!("getTextSelection(html) 失败：{e}"))?;
            return Ok((plain, html));
        }
    }
    Err("SelectAll 重试 4 次仍未覆盖全文（期望标记缺失）".to_string())
}

/// 等 ready / boot-failed 事件。
fn wait_ready(w: &WorkerClient, tap: &mut EventTap, dur: Duration) -> Option<Value> {
    let deadline = std::time::Instant::now() + dur;
    while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
        match tap
            .rx
            .recv_timeout(remaining.min(Duration::from_millis(500)))
        {
            Ok(Event::Life { kind, detail }) => match kind.as_str() {
                "ready" => return Some(detail),
                "boot-failed" => return Some(json!({"boot-failed": detail})),
                _ => continue,
            },
            Ok(Event::Callback { .. }) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // worker 死亡提前返回（不傻等满 180s）
                if w.is_dead() {
                    return Some(json!({
                        "worker_died": w.death_detail(),
                        "stderr_tail": w.stderr_tail(10),
                    }));
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Some(json!({
                    "worker_died": w.death_detail(),
                    "stderr_tail": w.stderr_tail(10),
                }));
            }
        }
    }
    None
}

/// 事件抽取器（记录 seen，供断言/诊断）。
pub struct EventTap {
    pub rx: std::sync::mpsc::Receiver<Event>,
    pub seen: Vec<(i32, String)>,
}

impl EventTap {
    /// 等指定回调类型出现（期间事件全部记录）。
    fn wait_callback(&mut self, type_id: i32, dur: Duration) -> bool {
        let deadline = std::time::Instant::now() + dur;
        while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
            match self.rx.recv_timeout(remaining) {
                Ok(Event::Callback {
                    type_id: t, name, ..
                }) => {
                    self.seen.push((t, name.clone()));
                    if t == type_id {
                        return true;
                    }
                }
                Ok(Event::Life { .. }) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return false,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return false,
            }
        }
        false
    }

    fn saw(&self, type_id: i32) -> bool {
        self.seen.iter().any(|(t, _)| *t == type_id)
    }

    fn seen_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.seen.iter().map(|(_, n)| n.clone()).collect();
        names.sort();
        names.dedup();
        names
    }
}
