//! B2 产品会话契约测试（#49 工作线 B → #50 C 线消费）。
//!
//! 验证 [`DocSession`]（可拥有会话）的公开契约在**真实 LOK 运行时**下的
//! 实际行为：打开 → 几何 → tile → 编辑 → 撤销/重做 → 选区 → 保存 →
//! 重开 → 双会话并行。undo/redo 是 C 线（#50）产品关闭/编辑链的硬依赖，
//! 按 #49 派工单「API 以真实 LOK 能能为准」实测断言，不以协议面替代。
//!
//! 环境门（AGENTS §13 纪律）：无 LibreOffice 运行时时打印横幅 SKIP
//! （SKIP ≠ PASS）。

use mdc_lok::client::DocSession;
use std::time::Duration;

fn runtime_available() -> bool {
    mdc_lok::runtime::discover().is_ok()
}

fn fixture() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/smoke-chars-img-table.docx")
}

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("mdc-lok-b2-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn file_url(p: &std::path::Path) -> String {
    let canon = std::fs::canonicalize(p).unwrap();
    mdc_lok::profile::file_url_of(&canon)
}

#[test]
fn session_contract_full_chain() {
    // cargo 集成测试：worker 在 CARGO_BIN_EXE_mdc-lok-worker（deps 同级），
    // 测试进程在 deps/ 下——经 env 注入（spawn 默认值链第二优先级）。
    if let Some(w) = option_env!("CARGO_BIN_EXE_mdc-lok-worker") {
        std::env::set_var("MDC_LOK_WORKER_BIN", w);
    }
    if !runtime_available() {
        eprintln!("=== SKIP：无 LibreOffice 运行时（MDC_LOK_HOME / 系统安装均未发现）===");
        eprintln!("=== 契约测试需要真实内核（open/undo/redo/saveAs/reopen 全链）===");
        return;
    }
    let dir = tmp_dir("full");
    // 工作副本（源文件不动——保存守卫禁止覆盖源）
    let work = dir.join("work.docx");
    std::fs::copy(fixture(), &work).unwrap();
    let saved = dir.join("saved.docx");

    // ---- 契约 1：open 一步到位（spawn+load+init_rendering）----
    let mut s1 = DocSession::open(None, &dir.join("s1"), &file_url(&work), None)
        .expect("DocSession::open（发现序+load+init_rendering）");
    assert!(!s1.is_dirty());
    assert_eq!(s1.revision(), 0);
    assert!(!s1.is_dead());

    // ---- 契约 2：几何（getDocumentSize，twips）----
    let (w, h) = s1.size().expect("size");
    assert!(w > 0 && h > 0, "文档几何非零（实测 {w}x{h} twips）");

    // ---- 契约 2b：页几何（page_geometry：C 线跳页/页码数据源）----
    // 25.2.3.2 实测语义：画布坐标系（与 paintTile 同源），画布 = 页 +
    // 对称边距（实测 284 twips），页隙 284。断言按实测语义锁：
    // 非空、宽高为正、A4 页尺寸、边距对称（左右相等、上下相等）。
    let pages = s1.page_geometry().expect("page_geometry");
    assert!(!pages.is_empty(), "页矩形非空（Writer 分页模式每页一条）");
    assert!(
        pages.iter().all(|p| p.w > 0 && p.h > 0),
        "每页宽高为正（实测 {pages:?}）"
    );
    let first = pages[0];
    let last = *pages.last().unwrap();
    assert_eq!(
        (first.w, first.h),
        (11906, 16838),
        "A4 页尺寸（夹具确定性；画布边距不含在内）"
    );
    assert_eq!(
        first.x,
        w - first.x - first.w,
        "左右画布边距相等（跳页滚动锚定的几何自洽；实测边距 284 twips）"
    );
    assert_eq!(
        first.y,
        h - last.y - last.h,
        "上下画布边距相等（首顶 == 末底余量；实测 284 twips）"
    );
    assert!(
        pages.windows(2).all(|pw| pw[1].y >= pw[0].y + pw[0].h),
        "y 单调不重叠（页序即视口序）"
    );

    // ---- 契约 3：tile（side-file BMP；tile_w/h 为文档 twips，1px=15twips）----
    let tile = s1.paint(256, 256, 0, 0, 256 * 15, 256 * 15).expect("paint");
    let non_white = tile
        .get("stats")
        .and_then(|st| st.get("non_white_ratio"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    assert!(non_white > 0.005, "tile 非空白（non_white={non_white}）");

    // ---- 契约 4：编辑（type_text 置脏 + revision+1）----
    s1.type_text("契约测试文本").expect("type_text");
    assert!(s1.is_dirty(), "编辑后 dirty=true");
    assert_eq!(s1.revision(), 1, "编辑后 revision=1");

    // ---- 契约 5：undo（.uno:Undo——真实能力验证，非协议模拟）----
    s1.uno(".uno:Undo", None).expect("uno Undo");
    assert_eq!(s1.revision(), 2, "undo 也是变更：revision+1");
    // ---- 契约 6：redo（.uno:Redo）----
    s1.uno(".uno:Redo", None).expect("uno Redo");

    // ---- 契约 7：选区全文（SelectAll + getTextSelection）----
    //
    // 【2026-11-12 Windows 实测】`postUnoCommand` 是**异步投递**：投递后
    // 立刻 getTextSelection 读到的是旧选区（后续为上一状态的空串），会
    // 被误判成「SelectAll 未生效」而重复投递——每次重复投递都会让
    // revision 多 +1，破坏契约 8 的精确计数（实测 left=6 vs right=5 即
    // 此因）。因此这里改为：**只投递一次**，随后轮询选区直到出现期望
    // 标记（等价于 wait-for-effect，不重复触发变更）。
    s1.uno(".uno:SelectAll", None).expect("SelectAll");
    let mut text = String::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        let (_, t) = s1
            .text_selection("text/plain;charset=utf-8")
            .expect("text_selection");
        text = t;
        if text.contains("契约测试文本") {
            break;
        }
    }
    assert!(
        text.contains("契约测试文本"),
        "redo 后全文含编辑文本（undo→redo 语义闭环；实际片段：{text:?}）"
    );

    // ---- 契约 8：save_as（成功清脏 + revision+1；产物可被第二会话重开）----
    // URL 先构造（目标文件此刻尚不存在——canonicalize 只对目录做）
    let saved_url = {
        let dir_canon = std::fs::canonicalize(&dir).unwrap();
        mdc_lok::profile::file_url_of(&dir_canon.join("saved.docx"))
    };
    s1.save_as(&saved_url, "docx").expect("save_as");
    assert!(!s1.is_dirty(), "保存成功后 dirty=false");
    assert_eq!(
        s1.revision(),
        5,
        "save 后 revision=5（type/undo/redo/selectAll×?/save——编辑类单调递增）"
    );
    assert!(saved.is_file(), "保存产物存在");

    // ---- 契约 9：双会话并行（独立 profile 不互锁）----
    let s2 =
        DocSession::open(None, &dir.join("s2"), &saved_url, None).expect("第二会话重开保存产物");
    assert!(!s1.is_dead(), "会话 1 在会话 2 存活期间不死");
    let (w2, _h2) = s2.size().expect("s2 size");
    assert!(w2 > 0);

    // ---- 契约 10：事件通道（回调到达）----
    //
    // 【2026-11-12 Windows 实测矩阵】tile 失效回调（INVALIDATE_TILES）在
    // 本构建上**不是**普通编辑的产物。直连真内核、每次全新 profile、步骤间
    // 用 ping 往返让 LO 主线程消费异步命令，八组对照实测：
    //
    //   不画 + 编辑+undo+redo                    → 0
    //   paint + post_text                        → 0
    //   paint + ExecuteSearch + post_text        → 0
    //   paint + post_text + undo                 → 0
    //   paint + post_text + undo + redo          → 0
    //   paint + ExecuteSearch + 逐字符 post_key   → 0
    //   paint + 逐字符 post_key                   → 0
    //   paint + ExecutionSearch + 编辑 + undo+redo+undo → 1（payload `EMPTY, 0, 0`）
    //
    // 即：单次编辑（无论 post_text 还是 type_text 的逐字符 post_key）都不触发
    // tile 失效；只有「搜索建立选区 + 编辑 + 撤销/重做往复」这一整串才会以
    // 整视图失效（`EMPTY`）的形式合并上报一次。因此本契约按**事件通道可用性**
    // 断言（channel 确实是通的、编辑类回调确实到达），不绑定某个特定回调名。
    // tile 失效本身由 smoke 的 `cb.invalidate_tiles_after_edit` 步骤端到端覆盖
    // （该步走完整链路并通过）。
    let events = s2.take_events().expect("事件通道");
    let mut s2 = s2;
    let (w2p, h2p) = s2.size().expect("s2 尺寸");
    let cw2 = (w2p / 15).min(1024) as usize;
    let ch2 = (h2p / 15).min(1024) as usize;
    s2.paint(cw2, ch2, 0, 0, cw2 as i32, ch2 as i32)
        .expect("s2 首帧绘制");
    // 排空首帧绘制产生的回调（events 已被 take_events 取走，必须用它本身）。
    while events.try_recv().is_ok() {}
    s2.type_text("第二会话编辑").expect("s2 编辑（产生回调）");
    // 编辑类回调至少到达一条，且必须是「编辑确实发生了」的那几种之一。
    const EDIT_CALLBACKS: [&str; 4] = [
        "TEXT_SELECTION",
        "INVALIDATE_VISIBLE_CURSOR",
        "STATE_CHANGED",
        "MODIFY_CHANGED",
    ];
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut seen: Vec<String> = Vec::new();
    let mut hit_edit_cb = false;
    while std::time::Instant::now() < deadline {
        match events.recv_timeout(Duration::from_millis(200)) {
            Ok(mdc_lok::client::Event::Callback { name, .. }) => {
                if EDIT_CALLBACKS.contains(&name.as_str()) {
                    hit_edit_cb = true;
                    break;
                }
                seen.push(name);
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert!(
        hit_edit_cb,
        "编辑后事件通道应到达编辑类回调（TEXT_SELECTION/INVALIDATE_VISIBLE_CURSOR/\
         STATE_CHANGED/MODIFY_CHANGED 之一）；实际先收到：{seen:?}"
    );

    // ---- 契约 11：关闭纪律（shutdown 不 panic、产物不回收）----
    s1.shutdown();
    s2.shutdown();
    assert!(
        saved.is_file(),
        "shutdown 后保存产物仍在（落盘只认 saveAs）"
    );
}
