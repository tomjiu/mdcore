//! #47 巨石拆解批①：原 shell.rs 内嵌 `tab_workflow_tests` 纯迁出（零行为变化——
//! 字节原样，挂载点 #[cfg(test)] + #[path] 在 shell.rs）。

use super::*;
use crate::editor::buffer::Point;
use crate::editor::commands::Cmd;

/// 测试宿主（见模块注释）。
#[allow(dead_code)]
struct ShellHost(Entity<ShellApp>);

impl gpui::Render for ShellHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div()
    }
}

/// 临时夹具目录（OS 临时区，进程号隔离；测试首尾各清一次——
/// 崩溃残留也只是临时区垃圾，不污染工作树）。
fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mdc-tab-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 写一个 md 夹具（返回绝对路径——对话框/文件树入参形态）。
fn write_md(dir: &Path, name: &str, content: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

/// 空态 ShellApp + 宿主窗口（返回实体与窗口句柄）。
fn shell_window(
    cx: &mut gpui::TestAppContext,
) -> (Entity<ShellApp>, gpui::WindowHandle<ShellHost>) {
    // 主题全局：真实应用在 main.rs 开窗前 theme::init(JustBase)——
    // 测试镜像同一序列（ShellApp::new 的 GlobalTheme::theme /
    // theme_switch::apply_mode 首访即需；TestPlatform 的
    // window_appearance 为固定 Light 桩，fallback 主题集不依赖
    // 资产源——JustBase 与 main 同款可无头初始化）。
    cx.update(|app| theme::init(theme::LoadThemes::JustBase, app));
    // ShellApp::new 需要 &mut Window（on_window_should_close 等），
    // 实体只能在 add_window 闭包内构造；槽位带出（FnOnce 一次性）。
    let mut slot: Option<Entity<ShellApp>> = None;
    let window = cx.add_window(|window, cx| {
        let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
        slot = Some(shell.clone());
        ShellHost(shell)
    });
    (slot.expect("宿主构造即置"), window)
}

/// 打开一个文档标签（open_document_tab 的薄包装——无 window 面）。
fn open(shell: &Entity<ShellApp>, cx: &mut Context<ShellHost>, path: PathBuf) {
    shell.update(cx, |shell, cx| {
        shell.open_document_tab(path, cx);
    });
}

/// #175：启动 FILE 路径构造（main.rs 初始文档同款——`document::
/// open_editor` → `ShellApp::new(Some(editor))`；区别于
/// `shell_window` 的空态构造与 `open` 的标签路径）。
fn startup_shell_window(
    cx: &mut gpui::TestAppContext,
    path: PathBuf,
) -> (Entity<ShellApp>, gpui::WindowHandle<ShellHost>) {
    let mut slot: Option<Entity<ShellApp>> = None;
    let window = cx.add_window(|window, cx| {
        let editor = match document::open_editor(&path, cx) {
            document::OpenOutcome::Opened(ed) => ed,
            document::OpenOutcome::Unsupported { message, .. } => {
                panic!("夹具类型必须支持（{}）：{message}", path.display())
            }
            document::OpenOutcome::Failed { message, .. } => {
                panic!("夹具必须可打开（{}）：{message}", path.display())
            }
        };
        let shell = cx.new(|cx| ShellApp::new(Some(editor), None, window, cx));
        slot = Some(shell.clone());
        ShellHost(shell)
    });
    (slot.expect("宿主构造即置"), window)
}

/// #175：启动 FILE 路径的页面视图接线——`ShellApp::new(Some(
/// editor))` 即建对应主显示面（此前仅 `open_document_tab` 标签
/// 路径调用三 ensure：双击/命令行直给文件静默落文本投影态 =
/// 「打开没渲染」真机症状）。三格式各验一例 + md 零行为变化。
/// 实体存在性即接线断言（打开链在视图内异步——Ready/Failed 属
/// 内核链面；真机双击 worker 进程/DPI 两档/同次运行日志属 #48
/// 验收面，不在单测范围）。
#[gpui::test]
fn startup_file_path_wires_page_views(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let dir = fixture_dir("startup-wire");

    // ① docx 启动 → LOK 页面视图实体在场（#175 旧实现恒 None）。
    let docx = write_docx_fixture(&dir, "startup.docx");
    let (shell, _window) = startup_shell_window(cx, docx);
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.tab_count(), 1, "启动标签在场");
        assert!(
            shell.docx_pages.is_some(),
            "启动路径 docx → 页面视图实体（#175）"
        );
        assert!(
            shell.pdf_pages.is_none() && shell.xlsx_grid.is_none(),
            "非 pdf/xlsx 不建对应实体"
        );
    });

    // ② pdf 启动 → 页视图实体在场（同一漏接线面——一并确认）。
    // #247③：pdfium 引擎环境卫——沙箱/无引擎开发机优雅跳过
    //（引擎装载失败的诊断文案已含取件指引；有引擎环境才断言
    // 页视图接线——引擎在位由装有 libpdfium 的门禁环境覆盖）。
    if mdc_pdf::engine_pdfium::PdfiumEngine::shared_handle().is_err() {
        eprintln!(
            "SKIP: pdfium 引擎不可用——段②（pdf 启动页视图接线）环境性跳过；段①③（docx/md）照跑"
        );
        return;
    }
    let pdf_src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.pdf");
    let pdf = dir.join("startup.pdf");
    std::fs::copy(&pdf_src, &pdf).expect("pdf 夹具在场");
    let (shell, _window) = startup_shell_window(cx, pdf);
    shell.read_with(cx, |shell, _| {
        assert!(shell.pdf_pages.is_some(), "启动路径 pdf → 页视图实体");
        assert!(
            shell.docx_pages.is_none() && shell.xlsx_grid.is_none(),
            "非 docx/xlsx 不建对应实体"
        );
    });

    // ③ md 启动 → 零行为变化（不建任何视图实体）。
    let md = write_md(&dir, "startup.md", "# 启动\n");
    let (shell, _window) = startup_shell_window(cx, md);
    shell.read_with(cx, |shell, _| {
        assert!(
            shell.docx_pages.is_none() && shell.pdf_pages.is_none() && shell.xlsx_grid.is_none(),
            "md 启动不建视图实体（零回归）"
        );
    });
}

/// #219 验收①：至少 3 个文档并存，切换后各自内容、光标/选区与
/// dirty 状态保持正确（tab0 脏+光标、tab1 全选区、tab2 干净基线；
/// 循环切换双向 wrap 后逐标签回读）。
#[gpui::test]
fn three_tabs_switch_preserves_state(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("preserve");
    let a = write_md(&dir, "a.md", "A0\nA1\nA2\n");
    let b = write_md(&dir, "b.md", "B0\nB1\n");
    let c = write_md(&dir, "c.md", "C0\nC1\n");

    let (shell, window) = shell_window(cx);
    let _ = window.update(cx, |_host, _window, cx| {
        open(&shell, cx, a.clone());
        open(&shell, cx, b.clone());
        open(&shell, cx, c.clone());
        shell.update(cx, |shell, _cx| {
            assert_eq!(shell.tab_count(), 3, "三文档并存");
            assert_eq!(shell.active, 2, "新开即激活");
        });
    });

    // 各标签置入待保持的状态：tab0 脏 + 光标在插入点；tab1 全选；
    // tab2 干净。tab0 的编辑不得波及 tab1/tab2 的脏位。（闭包内赋值
    // 走 default 起始 + expect 收口——闭包捕获未初始化局部不可编译。）
    let mut cursor_in_tab0 = Point::default();
    let mut selection_in_tab1: Option<(Point, Point)> = None;
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            let ed0 = shell.tab_editor(0).unwrap().clone();
            cursor_in_tab0 = ed0
                .update(cx, |ed, cx| ed.insert_text_at(1, 2, "X", cx))
                .expect("可编辑文档插入返回光标点");
            assert!(shell.tab_is_modified(0, cx).unwrap(), "tab0 插入后脏");

            shell.activate_tab(1, window, cx);
            let ed1 = shell.tab_editor(1).unwrap().clone();
            ed1.update(cx, |ed, cx| ed.run_command(Cmd::SelectAll, window, cx));
            selection_in_tab1 = ed1.read(cx).selection_points();

            shell.activate_tab(2, window, cx);
            assert!(!shell.tab_is_modified(2, cx).unwrap(), "tab2 干净基线");
        });
    });
    let selection_in_tab1 = selection_in_tab1.expect("tab1 全选有选区");

    // 循环切换 2→0（wrap）→1→0（backward）——全程逐项回读保持。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.cycle_tab(true, window, cx);
            assert_eq!(shell.active, 0, "末位 wrap 回首（方向）");
            let ed0 = shell.tab_editor(0).unwrap().clone();
            let ed = ed0.read(cx);
            assert_eq!(ed.buffer_line(1), "A1X", "tab0 内容保持（编辑在）");
            assert_eq!(ed.cursor_point(), cursor_in_tab0, "tab0 光标保持");
            assert!(ed.is_modified(), "tab0 脏位跨切换保持");

            shell.cycle_tab(true, window, cx);
            assert_eq!(shell.active, 1);
            let ed1 = shell.tab_editor(1).unwrap().clone();
            assert_eq!(
                ed1.read(cx).selection_points(),
                Some(selection_in_tab1),
                "tab1 选区跨切换保持"
            );
            assert!(!ed1.read(cx).is_modified(), "tab1 不脏");

            shell.cycle_tab(false, window, cx);
            assert_eq!(shell.active, 0, "backward 方向");

            // 脏位隔离：只有 tab0 脏。
            assert!(shell.tab_is_modified(0, cx).unwrap());
            assert!(!shell.tab_is_modified(1, cx).unwrap());
            assert!(!shell.tab_is_modified(2, cx).unwrap());
        });
    });

    let _ = std::fs::remove_dir_all(&dir);
}

/// #219 验收②/边界：同一文件重复打开（CLI 相对形态 + 对话框绝对
/// 形态）激活既有标签，不产生重复会话。
///
/// 相对形态夹具必须落在 CWD 内（cargo test 的 CWD = mang-shell
/// workspace 根；Windows 上跨盘符构造不出相对路径）——目录名进
/// .gitignore，测试首尾自清。
#[gpui::test]
fn same_file_reopen_dedupes_to_existing_tab(cx: &mut gpui::TestAppContext) {
    let tag_dir = std::env::current_dir()
        .unwrap()
        .join(format!("mdc-tab-rel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tag_dir);
    std::fs::create_dir_all(&tag_dir).unwrap();
    // 相对形态（原生分隔符：Windows 下 join 产反斜杠，与
    // std::path::absolute 的 GetFullPathNameW 归一结果一致）。
    let rel_dir = PathBuf::from(tag_dir.file_name().unwrap());
    let rel_a = rel_dir.join("a.md");
    let abs_a = tag_dir.join("a.md");
    std::fs::write(&abs_a, "A\n").unwrap();
    let abs_b = {
        let p = tag_dir.join("b.md");
        std::fs::write(&p, "B\n").unwrap();
        p
    };

    let (shell, window) = shell_window(cx);
    let _ = window.update(cx, |_host, _window, cx| {
        // 相对形态首开：落一个标签，身份 = 绝对化结果。
        open(&shell, cx, rel_a.clone());
        shell.update(cx, |shell, _cx| {
            assert_eq!(shell.tab_count(), 1);
            assert_eq!(shell.tab_path(0), Some(abs_a.clone()), "相对入参绝对化");
        });

        // 绝对形态开 b → 两标签。
        open(&shell, cx, abs_b.clone());
        shell.update(cx, |shell, _cx| {
            assert_eq!(shell.tab_count(), 2);
            assert_eq!(shell.active, 1);
        });

        // 同文件绝对形态重开 → 激活既有标签，不增。
        open(&shell, cx, abs_a.clone());
        shell.update(cx, |shell, _cx| {
            assert_eq!(shell.tab_count(), 2, "绝对形态重开不重复开会话");
            assert_eq!(shell.active, 0, "重开 = 激活既有");
            assert_eq!(shell.tab_index_by_path(&abs_a), Some(0));
        });

        // 同文件相对形态重开 → 同样去重（两形态同身份）。
        open(&shell, cx, rel_a.clone());
        shell.update(cx, |shell, _cx| {
            assert_eq!(shell.tab_count(), 2, "相对形态重开不重复开会话");
            assert_eq!(shell.active, 0);
            assert_eq!(shell.tab_index_by_path(&rel_a), Some(0));
        });
    });

    let _ = std::fs::remove_dir_all(&tag_dir);
}

// ── #306：OCR 读面 + 可搜索写回（手势级 e2e） ──────────────────

/// 引擎仓扫描件夹具（跨 crate 同仓引用——pdf_annotate_e2e 的签名
/// 夹具同款纪律）。
fn scanned_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../crates/pdf/fixtures/scanned_mixed.pdf")
        .to_path_buf()
}

/// #306 ①（合成面）：开扫描件 → 前置态（暂不支持横幅/pending）→
/// 合成 OCR 结果落读面（`apply_pdf_ocr`——真实词来自真 tesseract 的
/// 链路见 [`Self::ocr_scan_gesture_e2e`]）→ 读面断言：OCR 横幅 +
/// CJK 连接（招标文件无空格）+ 拉丁空格 + find 引擎命中 + **不置脏**
/// （派生态非文件变更）+ 菜单门槛翻转。
///
/// 环境门（AGENTS 13.1）：libpdfium 缺席 → pdf 打开失败态 → SKIP
/// 横幅（本测试不依赖 tesseract——合成词）。
#[gpui::test]
fn ocr_payload_face_synthetic(cx: &mut gpui::TestAppContext) {
    let (shell, window) = shell_window(cx);
    let scanned = scanned_fixture();
    let _ = window.update(cx, |_host, _window, cx| {
        open(&shell, cx, scanned.clone());
    });
    let pending = window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                shell
                    .editor()
                    .is_some_and(|ed| ed.read(cx).pdf_ocr_pending())
            })
        })
        .expect("前置态读");
    if !pending {
        eprintln!(
            "[SKIP] ocr_payload_face_synthetic：扫描件未成待识别态 \
                 （libpdfium 缺席或夹具缺席——pdf 打开走失败态）"
        );
        return;
    }
    let _ = window.update(cx, |_host, _window, cx| {
        shell.update(cx, |shell, cx| {
            let ed = shell.editor().unwrap().clone();
            // 前置态：暂不支持横幅 + pending + 非 ready。
            {
                let e = ed.read(cx);
                assert!(e.pdf_ocr_pending(), "扫描件待识别");
                assert!(!e.pdf_ocr_ready());
                let text = e.buffer_text_for_probe();
                assert!(
                    text.contains("渲染需要 OCR，当前版本暂不支持"),
                    "扫描件横幅在位（得：{text:?}）"
                );
            }
            // 合成 OCR（CJK 词行 + 拉丁词行——pdf_doc::with_ocr 的
            // 词→行分组面由 pdf_doc 单测背书）。
            let words = vec![
                mdc_pdf::OcrWord {
                    text: "招标".into(),
                    bbox: [50.0, 700.0, 90.0, 730.0],
                    line: (1, 1, 1),
                },
                mdc_pdf::OcrWord {
                    text: "文件".into(),
                    bbox: [95.0, 700.0, 135.0, 730.0],
                    line: (1, 1, 1),
                },
                mdc_pdf::OcrWord {
                    text: "Specification".into(),
                    bbox: [50.0, 640.0, 150.0, 670.0],
                    line: (2, 1, 1),
                },
            ];
            let outcome = crate::editor::pdf_doc::PdfOcrOutcome {
                lang: "chi_sim+eng".into(),
                word_count: words.len(),
                pages: vec![words],
            };
            ed.update(cx, |ed, cx| {
                let payload = ed.pdf_payload().cloned().unwrap();
                ed.apply_pdf_ocr(payload.with_ocr(outcome), cx);
            });
            // 读面断言。
            let e = ed.read(cx);
            assert!(!e.pdf_ocr_pending(), "已识别：pending 翻转");
            assert!(e.pdf_ocr_ready(), "已识别：ready 置位");
            assert!(!e.is_modified(), "OCR 是派生态——不置脏（保存手势不被触发）");
            let text = e.buffer_text_for_probe();
            assert!(text.contains("【OCR】识别完成"), "OCR 横幅（得：{text:?}）");
            assert!(
                text.contains("招标文件"),
                "CJK 词无空格直连（得：{text:?}）"
            );
            assert!(text.contains("Specification"), "拉丁词在位（得：{text:?}）");
            assert!(!text.contains("暂不支持"), "扫描件横幅退役（得：{text:?}）");
            // find 面：产品同源 Buffer find 引擎命中（可搜索性的
            // 引擎级证明——查找条走同一 Buffer::find 面）。
            let probe = crate::editor::buffer::Buffer::from_plain_text(&text);
            let hits = probe.find_matches("招标文件");
            assert!(!hits.is_empty(), "识别文本可搜索（产品 find 引擎命中）");
            // 菜单门槛翻转（menu_context 快照）。
            let menu = shell.menu_context(cx);
            assert!(!menu.pdf_ocr_pending, "菜单：OCR 项灰显");
            assert!(menu.pdf_ocr_ready, "菜单：另存可搜索项点亮");
        });
    });
}

/// #306 ①+②（真链路主锁）：开扫描件 → 菜单动作 OCR（真 tesseract
/// 逐页识别——专用线程 + mpsc + 轮询）→ advance_clock 驱动 Done
/// 消费 → 真识别词断言（中文词条级）→ 另存核心（`write_searchable_to`
/// ——真写回 + 重开验证）。
///
/// 环境门（AGENTS 13.1）：tesseract/chi_sim/libpdfium 任一缺席 →
/// SKIP 横幅（真链路在 pdff4_ocr 锁 7 有双引擎面背书；本测试是
/// 壳面手势链——从菜单动作到 payload 落位的全链）。
#[gpui::test]
fn ocr_scan_gesture_e2e(cx: &mut gpui::TestAppContext) {
    // tessdata 注入（pdff4_ocr 同款——本地携带目录在位时）。
    // SAFETY（edition 2024）：仅测试面、单线程序（此刻 OCR 线程尚未
    // spawn、TestAppContext 无并发读 env 面）；设置先于一切识别调用。
    let local = Path::new(env!("CARGO_MANIFEST_DIR")).join("../test-artifacts/ocr-tessdata");
    if std::env::var_os("MDC_OCR_TESSDATA").is_none() && local.join("chi_sim.traineddata").exists()
    {
        unsafe { std::env::set_var("MDC_OCR_TESSDATA", &local) };
    }
    let ocr_env = mdc_pdf::ocr::ocr_available();
    if !ocr_env.binary_ok || !ocr_env.has_lang("chi_sim") {
        eprintln!(
            "[SKIP] ocr_scan_gesture_e2e：tesseract/chi_sim 缺席 \
                 （binary={} langs={:?}——取件见 test-artifacts/ocr-tessdata/README.md）",
            ocr_env.binary_ok, ocr_env.languages
        );
        return;
    }
    let (shell, window) = shell_window(cx);
    let scanned = scanned_fixture();
    let _ = window.update(cx, |_host, _window, cx| {
        open(&shell, cx, scanned.clone());
    });
    let pending = window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                shell
                    .editor()
                    .is_some_and(|ed| ed.read(cx).pdf_ocr_pending())
            })
        })
        .expect("前置态读");
    if !pending {
        eprintln!("[SKIP] ocr_scan_gesture_e2e：libpdfium 缺席（扫描件打开失败态）");
        return;
    }
    // 手势：菜单动作（真实菜单同一分派键——activate_menu_action）。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.activate_menu_action(crate::menu_bar::MenuAction::OcrPdf, window, cx);
        });
    });
    // 真线程 + 虚拟时钟轮询：advance_clock 驱动 16ms 定时器，真实
    // tesseract 需要真实墙钟（页均秒级）——虚进 + 实睡混合直到
    // Done 消费（payload.ocr 落位）；上限 180s 实钟（防挂死）。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    let mut applied = false;
    while std::time::Instant::now() < deadline {
        cx.background_executor
            .advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        applied = cx.update(|app| {
            shell
                .read(app)
                .editor()
                .is_some_and(|ed| ed.read(app).pdf_ocr_ready())
        });
        if applied {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(applied, "OCR Done 应在实钟内消费（真 tesseract 识别）");

    // 读面断言（真识别词——词条级锚点，与 pdff4_ocr 同口径）。
    let outcome = window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                let ed = shell.editor().unwrap().clone();
                {
                    let e = ed.read(cx);
                    assert!(e.pdf_ocr_ready());
                    assert!(!e.is_modified(), "识别结果不置脏");
                    let text = e.buffer_text_for_probe();
                    assert!(text.contains("【OCR】识别完成"), "横幅在位");
                    assert!(
                        text.contains("招标"),
                        "真识别中文词（得头：{:?}）",
                        text.lines().take(8).collect::<Vec<_>>()
                    );
                    let probe = crate::editor::buffer::Buffer::from_plain_text(&text);
                    assert!(!probe.find_matches("招标").is_empty(), "真识别文本可搜索");
                }
                ed.read(cx)
                    .pdf_payload()
                    .and_then(|p| p.ocr.clone())
                    .unwrap()
            })
        })
        .expect("读面断言");
    // ② 另存核心（真写回 + 重开验证——对话框层是薄壳，
    // prompt_for_new_path 在 TestAppContext 不可用）。
    let dir = fixture_dir("ocr-save");
    let out_path = dir.join("scanned_可搜索.pdf");
    let (words, chars) = ShellApp::write_searchable_to(&scanned, &out_path, &outcome)
        .expect("另存核心链路（写回 + 重开验证）");
    assert!(words >= 10, "注入词量级（得 {words}）");
    assert!(chars >= 20, "重开验证文本量级（得 {chars}）");
    // 重开验证第二断言：产品打开面读到非扫描态。
    let written = std::fs::read(&out_path).unwrap();
    let payload = crate::editor::pdf_doc::load_from_bytes(written).unwrap();
    assert!(!payload.props.scanned, "写出档有文本层（非扫描态）");
    assert!(payload.text.contains("招标"), "产品打开面读到识别词");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #219 验收③/边界：关闭邻位规则（右邻优先/末位回左）、关非活动
/// 标签的 active 校正、关最后一个标签 → 空态不 panic、空态再关
/// 幂等。
#[gpui::test]
fn close_neighbor_rule_and_last_tab_empty_state(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("close");
    let a = write_md(&dir, "a.md", "A\n");
    let b = write_md(&dir, "b.md", "B\n");
    let c = write_md(&dir, "c.md", "C\n");

    let (shell, window) = shell_window(cx);
    let _ = window.update(cx, |_host, _window, cx| {
        open(&shell, cx, a.clone());
        open(&shell, cx, b.clone());
        open(&shell, cx, c.clone());
    });

    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            // [A,B,C] 激活 1，关 B → C 前移补位，active 仍 1。
            shell.activate_tab(1, window, cx);
            shell.close_active_tab(window, cx);
            assert_eq!(shell.tab_count(), 2);
            assert_eq!(shell.active, 1, "右邻前移补位");
            assert_eq!(shell.tab_path(1), Some(c.clone()), "C 补位到 1");

            // [A,C] active 1，关非活动左邻 A（idx 0）→ active 校正 0。
            shell.close_tab_at(0, window, cx);
            assert_eq!(shell.tab_count(), 1);
            assert_eq!(shell.active, 0, "关左侧非活动标签 active 随减");

            // 关最后一个标签 → 空态（editor() None，active 归零）。
            shell.close_active_tab(window, cx);
            assert_eq!(shell.tab_count(), 0, "关最后一个标签 → 空态");
            assert_eq!(shell.active, 0);
            assert!(shell.editor().is_none());

            // 空态再关：幂等不 panic。
            shell.close_active_tab(window, cx);
            assert_eq!(shell.tab_count(), 0);
        });
    });

    // 空态后可继续开新文档（生命周期闭环）。
    let _ = window.update(cx, |_host, _window, cx| {
        open(&shell, cx, a.clone());
        shell.update(cx, |shell, _cx| {
            assert_eq!(shell.tab_count(), 1, "空态后再开正常");
        });
    });

    let _ = std::fs::remove_dir_all(&dir);
}

/// #219 验收④：脏标签关闭确认三选——取消（原标签与内容不变）、
/// 保存（落盘后关）、不保存（直接关、盘上内容不动）。干净标签
/// 关闭无确认框。
#[gpui::test]
fn dirty_close_confirm_cancel_save_discard(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("confirm");
    let a = write_md(&dir, "a.md", "L0\n");

    let (shell, window) = shell_window(cx);
    let _ = window.update(cx, |_host, _window, cx| {
        open(&shell, cx, a.clone());
    });

    // 干净标签：直接关，无确认框。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.close_active_tab(window, cx);
            assert!(shell.confirm.is_none(), "干净标签关闭不弹确认");
            assert_eq!(shell.tab_count(), 0);
        });
    });

    // 重开并置脏（L0 → L0X）。
    let _ = window.update(cx, |_host, _window, cx| {
        open(&shell, cx, a.clone());
        shell.update(cx, |shell, cx| {
            shell.tab_editor(0).unwrap().update(cx, |ed, cx| {
                ed.insert_text_at(0, 2, "X", cx);
            });
            assert!(shell.tab_is_modified(0, cx).unwrap());
        });
    });

    // 关脏标签 → 确认框（文案/按钮/动作/默认选中）。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.close_active_tab(window, cx);
            let confirm = shell.confirm.as_ref().expect("脏标签关闭弹确认");
            assert!(confirm.message.contains("a.md"), "文案含文件名");
            assert_eq!(
                confirm.buttons,
                CONFIRM_BUTTONS.to_vec(),
                "保存/不保存/取消三选"
            );
            assert!(matches!(
                confirm.action,
                ConfirmAction::CloseTab { idx: 0, .. }
            ));
            assert_eq!(confirm.selected, 0, "默认停在保存侧");
        });
    });

    // 取消：原标签与内容原样保留，确认框清除。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.resolve_confirm(2, window, cx);
            assert!(shell.confirm.is_none(), "取消清确认框");
            assert_eq!(shell.tab_count(), 1, "取消不关标签");
            let ed = shell.tab_editor(0).unwrap().clone();
            assert_eq!(ed.read(cx).buffer_line(0), "L0X", "取消内容不变");
            assert!(ed.read(cx).is_modified(), "取消后仍脏");
        });
    });

    // 保存（choice 0）：落盘 + 关标签；盘上带 X。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.close_active_tab(window, cx);
            assert!(shell.confirm.is_some());
            shell.resolve_confirm(0, window, cx);
            assert_eq!(shell.tab_count(), 0, "保存后关");
            assert_eq!(
                std::fs::read_to_string(&a).unwrap(),
                "L0X\n",
                "保存 = 编辑落盘"
            );
        });
    });

    // 重开：内容带 X 且干净；再置脏（L0X → L0XY）后不保存（choice 1）：
    // 直接关、盘上仍是无 Y 的版本。
    let _ = window.update(cx, |_host, _window, cx| {
        open(&shell, cx, a.clone());
        shell.update(cx, |shell, cx| {
            let ed = shell.tab_editor(0).unwrap().clone();
            assert_eq!(ed.read(cx).buffer_line(0), "L0X", "重开 = 盘上内容");
            assert!(!ed.read(cx).is_modified(), "重开干净");
            ed.update(cx, |ed, cx| {
                ed.insert_text_at(0, 3, "Y", cx);
            });
        });
    });
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.close_active_tab(window, cx);
            shell.resolve_confirm(1, window, cx);
            assert_eq!(shell.tab_count(), 0, "不保存直接关");
            assert_eq!(
                std::fs::read_to_string(&a).unwrap(),
                "L0X\n",
                "放弃 = 盘上内容不动（Y 未落盘）"
            );
        });
    });

    let _ = std::fs::remove_dir_all(&dir);
}

/// 仓内 docx 夹具拷入临时目录（不触仓内文件——e2e/docx_edit_e2e
/// 同款夹具源：tests/fixtures/smoke.docx，gen_smoke_docx 生成）。
fn write_docx_fixture(dir: &Path, name: &str) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.docx");
    let path = dir.join(name);
    std::fs::copy(&src, &path).expect("夹具在场（cargo test gen_smoke_docx -- --ignored 生成）");
    path
}

/// 极简 zip 条目名扫描（EOCD → 中央目录 → 文件名表；与
/// tests/docx_edit_e2e.rs 同口径——独立于被测代码的裸字节判读）。
fn zip_entry_names(bytes: &[u8]) -> Vec<String> {
    let n = bytes.len();
    let mut eocd = None;
    let mut i = n.saturating_sub(22);
    loop {
        if bytes[i..i + 4] == [0x50, 0x4b, 0x05, 0x06] {
            eocd = Some(i);
            break;
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    let eocd = eocd.expect("EOCD 在场（zip 尾）");
    let count = u16::from_le_bytes([bytes[eocd + 10], bytes[eocd + 11]]) as usize;
    let cd_off = u32::from_le_bytes([
        bytes[eocd + 16],
        bytes[eocd + 17],
        bytes[eocd + 18],
        bytes[eocd + 19],
    ]) as usize;
    let mut names = Vec::new();
    let mut p = cd_off;
    for _ in 0..count {
        let nlen = u16::from_le_bytes([bytes[p + 28], bytes[p + 29]]) as usize;
        let xlen = u16::from_le_bytes([bytes[p + 30], bytes[p + 31]]) as usize;
        let clen = u16::from_le_bytes([bytes[p + 32], bytes[p + 33]]) as usize;
        names.push(String::from_utf8_lossy(&bytes[p + 46..p + 46 + nlen]).to_string());
        p += 46 + nlen + xlen + clen;
    }
    names
}

/// #47 A2：docx 标签生命周期全链——页面态保存卫（Unmodified 零
/// 写盘）→ 文本态编辑脏 → 关闭确认三选（取消保留 / 保存 = **合法
/// docx 字节**（PK zip + word/document.xml + 编辑段落在场），不是
/// 行模型文本覆写进 .docx）→ 放弃 = 盘上不动 → 重开 = 盘上内容
/// （编辑在场 + 干净）。夹具锚点段与 docx_edit_e2e 同款。
#[gpui::test]
fn docx_tab_lifecycle_close_confirm_saves_docx_bytes(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("docx-life");
    let docx = write_docx_fixture(&dir, "life.docx");
    let before = std::fs::read(&docx).expect("夹具已拷入");

    let (shell, window) = shell_window(cx);

    // ① 打开（页面视图缺省）→ 页面态保存卫：Unmodified + 零写盘。
    let _ = window.update(cx, |_host, _window, cx| {
        open(&shell, cx, docx.clone());
    });
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            assert_eq!(shell.tab_count(), 1, "docx 标签打开");
            assert!(shell.docx_view_mode, "页面视图缺省");
            let outcome = shell.save_tab(0, cx);
            assert_eq!(
                outcome,
                SaveOutcome::Unmodified,
                "页面态只读渲染 → 保存 no-op（不走行编辑保存路径）"
            );
            assert_eq!(
                std::fs::read(&docx).unwrap(),
                before,
                "页面态保存零写盘（字节逐位不动）"
            );
            // 干净关闭：无确认框（页面态无编辑面）。
            shell.close_active_tab(window, cx);
            assert!(shell.confirm.is_none(), "干净 docx 不弹确认");
            assert_eq!(shell.tab_count(), 0);
        });
    });

    // ② 重开 → 切文本态（拆 LOK 会话）→ 找锚点段编辑 → 脏 →
    //    关闭确认 → 取消保留。
    let _ = window.update(cx, |_host, window, cx| {
        open(&shell, cx, docx.clone());
        shell.update(cx, |shell, cx| {
            assert!(shell.docx_pages.is_some(), "页面态缺省建页视图");
            shell.toggle_docx_view_mode(window, cx);
            assert!(!shell.docx_view_mode, "切文本态成功");
            assert!(
                shell.docx_pages.is_none(),
                "文本态拆 LOK 会话（行投影 = 唯一编辑面）"
            );
            // 锚点段（与 tests/docx_edit_e2e.rs 同款夹具锚点）。
            let ed = shell.tab_editor(0).unwrap().clone();
            let target = (0..ed.read(cx).line_count())
                .find(|&i| {
                    ed.read(cx)
                        .buffer_line(i)
                        .starts_with("平台采用云原生微服务架构")
                })
                .expect("锚点段在场（smoke.docx 夹具）");
            ed.update(cx, |ed, cx| {
                ed.insert_text_at(target, 2, "【改】", cx);
            });
            assert!(shell.tab_is_modified(0, cx).unwrap(), "文本态编辑 → 标签脏");
            assert!(
                ed.read(cx).buffer_line(target).contains("【改】"),
                "行模型带编辑标记"
            );
            // 文本态下切走再切回：不重建隐藏 LOK 会话（#47 A2 修复面）。
            shell.activate_tab(0, window, cx);
            assert!(
                shell.docx_pages.is_none(),
                "文本态激活不重建 LOK 会话（模式卫）"
            );
        });
    });

    // ③ 脏关闭 → 确认框三面 → 取消：标签/内容/脏全保留。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.close_active_tab(window, cx);
            let confirm = shell.confirm.as_ref().expect("脏 docx 关闭弹确认");
            assert!(
                confirm.message.contains("life.docx"),
                "文案含文件名：{}",
                confirm.message
            );
            assert_eq!(confirm.buttons, CONFIRM_BUTTONS.to_vec(), "三选按钮");
            assert!(matches!(
                confirm.action,
                ConfirmAction::CloseTab { idx: 0, .. }
            ));
            shell.resolve_confirm(2, window, cx);
            assert_eq!(shell.tab_count(), 1, "取消不关标签");
            let ed = shell.tab_editor(0).unwrap().clone();
            assert!(
                ed.read(cx)
                    .buffer_line(
                        (0..ed.read(cx).line_count())
                            .find(|&i| ed.read(cx).buffer_line(i).contains("【改】"))
                            .expect("编辑行在场")
                    )
                    .contains("【改】"),
                "取消内容不变"
            );
            assert!(shell.tab_is_modified(0, cx).unwrap(), "取消仍脏");
        });
    });

    // ④ 保存（choice 0）：关标签 + 盘上是**合法 docx 字节**——
    //    PK zip 头 + word/document.xml 条目 + 锚点段带编辑标记；
    //    绝不是把行模型文本覆写进 .docx。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.close_active_tab(window, cx);
            assert!(shell.confirm.is_some());
            shell.resolve_confirm(0, window, cx);
            assert_eq!(shell.tab_count(), 0, "保存后关标签");
            let bytes = std::fs::read(&docx).unwrap();
            assert!(
                bytes.starts_with(b"PK\x03\x04"),
                "保存产物 = docx 容器（PK zip 头），不是行模型文本"
            );
            assert_ne!(bytes, before, "编辑已落盘（字节变化）");
            let names = zip_entry_names(&bytes);
            assert!(
                names.iter().any(|n| n == "word/document.xml"),
                "zip 条目含 word/document.xml：{names:?}"
            );
            assert!(
                !bytes.starts_with(b"\xef\xbb\xbf") && !bytes.starts_with(b"\xe5\xb9\xb3"),
                "不是 UTF-8 文本覆写（首字节非 BOM/中文行首）"
            );
        });
    });

    // ⑤ 重开 = 盘上内容：编辑标记在场 + 干净；再脏后放弃（choice 1）
    //    = 盘上不动。
    let _ = window.update(cx, |_host, window, cx| {
        open(&shell, cx, docx.clone());
        shell.update(cx, |shell, cx| {
            assert!(!shell.docx_view_mode, "文本态跨关闭保持（shell 全局）");
            assert!(shell.docx_pages.is_none(), "重开不建隐藏会话（文本态）");
            let ed = shell.tab_editor(0).unwrap().clone();
            let target = (0..ed.read(cx).line_count())
                .find(|&i| ed.read(cx).buffer_line(i).contains("【改】"))
                .expect("保存落盘的编辑在重开行模型里");
            assert!(!shell.tab_is_modified(0, cx).unwrap(), "重开干净");
            let after_save = std::fs::read(&docx).unwrap();
            ed.update(cx, |ed, cx| {
                ed.insert_text_at(target, 2, "【弃】", cx);
            });
            assert!(shell.tab_is_modified(0, cx).unwrap());
            shell.close_active_tab(window, cx);
            shell.resolve_confirm(1, window, cx);
            assert_eq!(shell.tab_count(), 0, "放弃直接关");
            assert_eq!(
                std::fs::read(&docx).unwrap(),
                after_save,
                "放弃 = 盘上不动（【弃】未落盘）"
            );
        });
    });

    let _ = std::fs::remove_dir_all(&dir);
}

/// docx 标签置脏的共用前奏：开 → 切文本态 → 锚点段插入标记 →
/// 断言脏（返回锚点行号——落盘断言复用）。
fn docx_dirty_tab(
    cx: &mut gpui::TestAppContext,
    shell: &Entity<ShellApp>,
    window: &gpui::WindowHandle<ShellHost>,
    docx: &Path,
    marker: &str,
) -> usize {
    window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_document_tab(docx.to_path_buf(), cx);
            });
        })
        .expect("开 docx 标签");
    window
        .update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.toggle_docx_view_mode(window, cx);
                let ed = shell.tab_editor(0).unwrap().clone();
                let target = (0..ed.read(cx).line_count())
                    .find(|&i| {
                        ed.read(cx)
                            .buffer_line(i)
                            .starts_with("平台采用云原生微服务架构")
                    })
                    .expect("锚点段在场（smoke.docx 夹具）");
                ed.update(cx, |ed, cx| {
                    ed.insert_text_at(target, 2, marker, cx);
                });
                assert!(shell.tab_is_modified(0, cx).unwrap(), "置脏前置");
                target
            })
        })
        .expect("置脏")
}

/// #47 A2：docx 退出协调——脏 docx 文本编辑 → 真实关闭钮回调链
/// （simulate_close）→ 拦截 + 退出确认（Quit）→「保存」逐标签落盘
/// = **合法 docx 字节**（PK zip + 锚点段带标记），不是行模型文本
/// 覆写。测试平台 quit() 无操作——盘上字节为本断言面。
#[gpui::test]
fn docx_dirty_quit_confirm_saves_docx_bytes(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("docx-quit");
    let docx = write_docx_fixture(&dir, "quit.docx");

    let (shell, window) = shell_window(cx);
    let target = docx_dirty_tab(cx, &shell, &window, &docx, "【退】");
    cx.run_until_parked();

    // 真实关闭钮回调链：脏 → 拦截 + 退出确认。
    let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
    let closed = vcx.simulate_close();
    assert!(!closed, "脏 docx → should-close 返回 false（退出被拦）");
    vcx.update(|_window, cx| {
        shell.update(cx, |shell, _cx| {
            let confirm = shell.confirm.as_ref().expect("退出前确认弹出");
            assert!(
                matches!(confirm.action, ConfirmAction::Quit),
                "确认动作 = Quit（得 {:?}）",
                confirm.action
            );
            assert!(
                confirm.message.contains('1'),
                "文案含未保存计数（得 {:?}）",
                confirm.message
            );
            assert_eq!(shell.tab_count(), 1, "被拦 = 标签还在");
        });
    });

    // 「保存」（choice 0）→ Quit 分支逐标签 save_tab → docx 字节落盘。
    vcx.update(|_window, cx| {
        shell.update(cx, |shell, cx| {
            shell.resolve_confirm(0, _window, cx);
        });
    });
    let bytes = std::fs::read(&docx).expect("退出保存后文件在场");
    assert!(
        bytes.starts_with(b"PK\x03\x04"),
        "退出保存 = docx 容器（PK zip 头），不是行模型文本"
    );
    assert!(
        zip_entry_names(&bytes)
            .iter()
            .any(|n| n == "word/document.xml"),
        "zip 条目含 word/document.xml"
    );
    // 落盘内容含锚点段标记（docx_doc 装载回读——与打开链同一函数）。
    let reloaded = crate::editor::docx_doc::load_editable(&docx)
        .expect("退出保存产物可重开")
        .payload;
    let line = reloaded.text.split('\n').nth(target).expect("锚点行在场");
    assert!(
        line.contains("【退】"),
        "退出保存落盘锚点段带标记（得 {line}）"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// #47 A2：docx 保存失败协调——删夹具目录（写盘临时文件 ENOENT
/// ——root 也拦不住的真实 IO 失败）→ 关闭确认「保存」→ 保存失败
/// **不关标签**（脏态保留不静默丢）→ 恢复目录后重试 → 保存成功关
/// 标签 + 盘上 = 合法 docx 字节（PK zip）带标记。
#[gpui::test]
fn docx_save_failure_keeps_dirty_then_recovers(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("docx-fail");
    let docx = write_docx_fixture(&dir, "fail.docx");

    let (shell, window) = shell_window(cx);
    docx_dirty_tab(cx, &shell, &window, &docx, "【败】");

    // 造真实 IO 失败：删夹具目录（save_docx 的临时文件写 ENOENT）。
    std::fs::remove_dir_all(&dir).expect("删目录");

    window
        .update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.close_active_tab(window, cx);
                assert!(shell.confirm.is_some(), "脏标签关闭弹确认（保存失败前置）");
                shell.resolve_confirm(0, window, cx);
                assert_eq!(shell.tab_count(), 1, "保存失败不关标签（脏态保留不静默丢）");
                assert!(
                    shell.tab_is_modified(0, cx).unwrap(),
                    "失败后仍脏（可重试）"
                );
            });
        })
        .expect("失败拒关");

    // 恢复目录 + 夹具字节 → 重试关闭 → 保存成功关标签 + PK 字节。
    std::fs::create_dir_all(&dir).expect("恢复目录");
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.docx");
    std::fs::copy(&src, &docx).expect("恢复夹具");
    window
        .update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.close_active_tab(window, cx);
                shell.resolve_confirm(0, window, cx);
                assert_eq!(shell.tab_count(), 0, "恢复后保存成功关标签");
            });
        })
        .expect("恢复后关闭");
    let bytes = std::fs::read(&docx).expect("恢复保存后文件在场");
    assert!(
        bytes.starts_with(b"PK\x03\x04"),
        "重试保存 = docx 容器（PK zip 头）"
    );
    let reloaded = crate::editor::docx_doc::load_editable(&docx)
        .expect("重试保存产物可重开")
        .payload;
    assert!(reloaded.text.contains("【败】"), "重试保存落盘锚点段带标记");

    let _ = std::fs::remove_dir_all(&dir);
}

/// #47 A1：右栏展示与当前视图相关——大纲面板随活动标签换绑
/// （sync_outline 的 stale 判定 → 重建）+ 行集内容跟随当前文档
/// 标题。开（新标签激活）/切回/关邻位/关末标签四面。
#[gpui::test]
fn right_panel_outline_follows_active_tab(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("outline-follow");
    let a = write_md(&dir, "a.md", "# A1\n正文\n## A2\n尾段\n");
    let b = write_md(&dir, "b.md", "# B1\n正文\n");

    let (shell, window) = shell_window(cx);
    let _ = window.update(cx, |_host, _window, cx| {
        open(&shell, cx, a.clone());
        open(&shell, cx, b.clone());
    });

    // ① 开双标签（b 激活）→ 大纲绑定 b 编辑器 + 行集 = B1 单行。
    let _ = window.update(cx, |_host, _window, cx| {
        shell.update(cx, |shell, cx| {
            let ed_b = shell.tab_editor(1).unwrap().clone();
            let panel = shell
                .outline
                .as_ref()
                .expect("右栏大纲在场（激活标签有标题）")
                .clone();
            assert_eq!(
                panel.read(cx).editor(),
                &ed_b,
                "开新标签 → 大纲绑定 = 活动标签（b）编辑器"
            );
            assert_eq!(
                panel.read(cx).row_labels(),
                vec!["H1 · B1"],
                "行集内容 = 当前文档（b）标题"
            );
        });
    });

    // ② 切回 a → 换绑 a + 行集 = A1/A2。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.activate_tab(0, window, cx);
            let ed_a = shell.tab_editor(0).unwrap().clone();
            let panel = shell.outline.as_ref().expect("切回后面板在场").clone();
            assert_eq!(panel.read(cx).editor(), &ed_a, "切回 a → 大纲换绑 a 编辑器");
            assert_eq!(
                panel.read(cx).row_labels(),
                vec!["H1 · A1", "H2 · A2"],
                "换绑后行集 = 当前文档（a）标题"
            );
        });
    });

    // ③ 关 a（活动标签，干净）→ 大纲随新活动标签（b）换绑。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.close_active_tab(window, cx);
            assert_eq!(shell.tab_count(), 1, "关 a 后剩 b");
            let ed_b = shell.tab_editor(0).unwrap().clone();
            let panel = shell.outline.as_ref().expect("关后面板在场").clone();
            assert_eq!(
                panel.read(cx).editor(),
                &ed_b,
                "关标签 → 大纲随新活动标签（b）换绑"
            );
            assert_eq!(
                panel.read(cx).row_labels(),
                vec!["H1 · B1"],
                "关标签后行集跟随新活动文档"
            );
        });
    });

    // ④ 关末标签（b）→ 空态：大纲随清（无标签无绑定）。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.close_active_tab(window, cx);
            assert_eq!(shell.tab_count(), 0);
            assert!(
                shell.outline.is_none(),
                "末标签关闭 → 大纲空态清（右栏不挂游离编辑器）"
            );
        });
    });

    let _ = std::fs::remove_dir_all(&dir);
}

/// #47 A4：切换标签/关标签/切显示模式后焦点恢复——docx 页面态主
/// 显示面是 LOK 页视图，`focus_active` 必须落页视图句柄（编辑器
/// 不在树内，游离句柄 = 键事件死在合成根——#422 同款病灶；本批
/// 修复面：关标签落 docx / toggle 切回页面态两条路径）。
#[gpui::test]
fn focus_restore_lands_on_docx_page_view(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("docx-focus");
    let docx = write_docx_fixture(&dir, "focus.docx");
    let md = write_md(&dir, "m.md", "M\n");

    let (shell, window) = shell_window(cx);
    let _ = window.update(cx, |_host, _window, cx| {
        open(&shell, cx, docx.clone());
        open(&shell, cx, md.clone());
    });

    // ① 关活动 md（干净）→ docx 成活动标签（页面态）→ 焦点落
    //    LOK 页视图（修复前：落游离编辑器句柄——键事件死）。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.close_active_tab(window, cx);
            assert_eq!(shell.tab_count(), 1, "关 md 后剩 docx");
            let (_, view) = shell
                .docx_pages
                .as_ref()
                .expect("docx 页视图在场（页面态缺省）")
                .clone();
            let view_focus = view.read(cx).focus_handle(cx);
            assert!(
                view_focus.contains_focused(window, cx),
                "关 md 落 docx → 焦点落 LOK 页视图（focus_active 修复面）"
            );
            let ed_focus = shell.tab_editor(0).unwrap().read(cx).focus_handle().clone();
            assert!(
                !ed_focus.contains_focused(window, cx),
                "游离编辑器句柄不承担焦点（页面态编辑器不在树内）"
            );
        });
    });

    // ② 切文本态（焦点落编辑器）→ 切回页面态（ensure 重建页视图）
    //    → 焦点落重建的页视图（toggle 路径同修复）。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.toggle_docx_view_mode(window, cx);
            let ed_focus = shell.tab_editor(0).unwrap().read(cx).focus_handle().clone();
            assert!(
                ed_focus.contains_focused(window, cx),
                "文本态焦点落编辑器（文本投影在树内）"
            );
            shell.toggle_docx_view_mode(window, cx);
            let (_, view) = shell
                .docx_pages
                .as_ref()
                .expect("页视图重建（切回页面态）")
                .clone();
            let view_focus = view.read(cx).focus_handle(cx);
            assert!(
                view_focus.contains_focused(window, cx),
                "切回页面态 → 焦点落重建的 LOK 页视图（toggle 路径修复面）"
            );
        });
    });

    let _ = std::fs::remove_dir_all(&dir);
}

/// #47 A2：md 保存失败协调（xlsx/docx 已有同款，md 缺席——A2
/// 「保存失败及退出协调正确」的 md 面收口）：删夹具目录（写盘
/// ENOENT——root 也拦不住的真实 IO 失败）→ 关闭确认「保存」→ 失败
/// **不关标签**（脏保留可重试）→ 恢复后重试 → 保存成功关标签。
#[gpui::test]
fn md_save_failure_keeps_dirty_then_recovers(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("md-fail");
    let a = write_md(&dir, "a.md", "L0\n");

    let (shell, window) = shell_window(cx);
    let _ = window.update(cx, |_host, _window, cx| {
        open(&shell, cx, a.clone());
        shell.update(cx, |shell, cx| {
            shell.tab_editor(0).unwrap().update(cx, |ed, cx| {
                ed.insert_text_at(0, 2, "X", cx);
            });
            assert!(shell.tab_is_modified(0, cx).unwrap(), "置脏前置");
        });
    });

    // 造真实 IO 失败：删夹具目录（fs::write ENOENT）。
    std::fs::remove_dir_all(&dir).expect("删目录");

    // 确认「保存」→ 保存失败不关标签（脏保留）。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.close_active_tab(window, cx);
            assert!(shell.confirm.is_some(), "脏标签关闭弹确认");
            shell.resolve_confirm(0, window, cx);
            assert_eq!(
                shell.tab_count(),
                1,
                "md 保存失败不关标签（脏态保留不静默丢）"
            );
            assert!(
                shell.tab_is_modified(0, cx).unwrap(),
                "失败后仍脏（可重试）"
            );
        });
    });

    // 恢复目录 + 夹具 → 重试 → 保存成功关标签 + 盘上 = 编辑后内容。
    std::fs::create_dir_all(&dir).expect("恢复目录");
    std::fs::write(&a, "L0\n").expect("恢复夹具");
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.close_active_tab(window, cx);
            shell.resolve_confirm(0, window, cx);
            assert_eq!(shell.tab_count(), 0, "恢复后保存成功关标签");
        });
    });
    assert_eq!(
        std::fs::read_to_string(&a).unwrap(),
        "L0X\n",
        "重试保存落盘 = 编辑后内容"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// #47 A2：md 退出保存失败保留确认框（重试语义——xlsx 有同款，md
/// 缺席）：真实关闭钮链 + 删目录真 IO 失败 → Quit 确认「保存」→
/// 失败 → **确认框保留**（不静默退出丢编辑）。
#[gpui::test]
fn md_quit_write_failure_keeps_retry_confirmation(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("md-quit-fail");
    let a = write_md(&dir, "a.md", "Q0\n");

    let (shell, window) = shell_window(cx);
    let _ = window.update(cx, |_host, _window, cx| {
        open(&shell, cx, a.clone());
        shell.update(cx, |shell, cx| {
            shell.tab_editor(0).unwrap().update(cx, |ed, cx| {
                ed.insert_text_at(0, 2, "Y", cx);
            });
            assert!(shell.tab_is_modified(0, cx).unwrap(), "置脏前置");
        });
    });

    // 真实关闭钮回调链：脏 → 拦截 + 退出确认。
    let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
    let closed = vcx.simulate_close();
    assert!(!closed, "脏 md → should-close 返回 false（退出被拦）");
    vcx.update(|_window, cx| {
        shell.update(cx, |shell, _cx| {
            assert!(
                matches!(
                    shell.confirm.as_ref().map(|c| &c.action),
                    Some(ConfirmAction::Quit)
                ),
                "前置：退出确认在场"
            );
        });
    });

    // 删目录造真 IO 失败 → 「保存」→ 失败 → 确认框保留（重试语义）。
    std::fs::remove_dir_all(&dir).expect("删目录");
    vcx.update(|_window, cx| {
        shell.update(cx, |shell, cx| {
            shell.resolve_confirm(0, _window, cx);
            assert_eq!(shell.tab_count(), 1, "失败不关标签");
            assert!(
                shell.confirm.is_some(),
                "退出保存失败 → 确认框保留（不静默退出丢编辑——重试语义）"
            );
            assert!(
                matches!(
                    shell.confirm.as_ref().map(|c| &c.action),
                    Some(ConfirmAction::Quit)
                ),
                "保留的确认动作仍是 Quit（重试同一决策面）"
            );
        });
    });

    // 恢复 → 重试「保存」→ 落盘 + 退出链走通（quit 无操作，盘上为断言面）。
    std::fs::create_dir_all(&dir).expect("恢复目录");
    std::fs::write(&a, "Q0\n").expect("恢复夹具");
    vcx.update(|_window, cx| {
        shell.update(cx, |shell, cx| {
            shell.resolve_confirm(0, _window, cx);
        });
    });
    assert_eq!(
        std::fs::read_to_string(&a).unwrap(),
        "Q0Y\n",
        "重试退出保存落盘 = 编辑后内容"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// #219 边界：循环切换在空态/单标签为 no-op；越界激活安全不改
/// 状态（标签点击竞态/程序化调用的防御面）。
#[gpui::test]
fn cycle_noop_edges_and_out_of_range_activate(cx: &mut gpui::TestAppContext) {
    let dir = fixture_dir("edges");
    let a = write_md(&dir, "a.md", "A\n");
    let b = write_md(&dir, "b.md", "B\n");

    let (shell, window) = shell_window(cx);

    // 空态循环：no-op 不 panic。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.cycle_tab(true, window, cx);
            shell.cycle_tab(false, window, cx);
            assert_eq!(shell.tab_count(), 0);
            assert_eq!(shell.active, 0);
        });
    });

    let _ = window.update(cx, |_host, window, cx| {
        open(&shell, cx, a.clone());
        // 单标签循环：不变（len < 2 短路——shell 级方法验证短路
        // 真的生效，焦点面不炸）。
        shell.update(cx, |shell, cx| {
            shell.cycle_tab(true, window, cx);
            assert_eq!(shell.tab_count(), 1);
            assert_eq!(shell.active, 0, "单标签循环不变");
        });

        // 两标签（active 1 = b）：循环两周回原位（1→0→1——cycle 的
        // eprintln 为 1 基展示，与本处 0 基状态勿混）；越界激活安全
        // 不改状态（标签点击竞态/程序化调用的防御面）。
        open(&shell, cx, b.clone());
        shell.update(cx, |shell, cx| {
            shell.cycle_tab(true, window, cx);
            shell.cycle_tab(true, window, cx);
            assert_eq!(shell.active, 1, "两标签循环两周回原位（b→a→b）");

            shell.activate_tab(7, window, cx);
            assert_eq!(shell.active, 1, "越界激活不改 active");
            shell.activate_tab(0, window, cx);
            assert_eq!(shell.active, 0);
            shell.activate_tab(2, window, cx);
            assert_eq!(shell.active, 0, "越界激活不改 active（再证）");
        });
    });

    let _ = std::fs::remove_dir_all(&dir);
}
