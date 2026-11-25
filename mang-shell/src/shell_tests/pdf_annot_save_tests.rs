//! #47 巨石拆解批③：原 shell.rs 内嵌 `pdf_annot_save_tests` 纯迁出（零行为变化）。

use super::*;

/// 测试宿主（与 tab_workflow_tests 同款——ShellApp 不作根视图，
/// 空 div 宿主开窗只取真实 `Window`）。
struct ShellHost(Entity<ShellApp>);

impl gpui::Render for ShellHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div()
    }
}

/// smoke.pdf 夹具字节（两页——与 #336/#386 视图级测试同源夹具）。
fn smoke_pdf_bytes() -> Vec<u8> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.pdf");
    std::fs::read(&p).unwrap_or_else(|e| panic!("夹具不可读（{}）：{e}", p.display()))
}

/// #386 全链：手势脏 → 关闭确认 → 「保存」→ 标签关 + 盘上批注
/// 在档（增量前缀 + 独立读回 1 条——验收 1/3/5 的 Shell 面）。
#[gpui::test]
fn pdf_gesture_dirty_close_confirm_save_persists(cx: &mut gpui::TestAppContext) {
    let dir = std::env::temp_dir().join(format!("mdc-386-shell-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let original = smoke_pdf_bytes();
    let pdf = dir.join("x.pdf");
    std::fs::write(&pdf, &original).unwrap();

    cx.update(|app| theme::init(theme::LoadThemes::JustBase, app));
    let mut slot: Option<Entity<ShellApp>> = None;
    let window = cx.add_window(|window, cx| {
        let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
        slot = Some(shell.clone());
        ShellHost(shell)
    });
    let shell = slot.expect("宿主构造即置");

    // 开 pdf 标签（缺省页视图模式）+ 构建页视图（生产路径 = 渲染
    // 期 ensure_pdf_pages；测试直调同一函数——同一调用面）。
    let _ = window.update(cx, |_host, _window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_document_tab(pdf.clone(), cx);
            shell.ensure_pdf_pages(cx);
        });
    });
    // 环境门控：无 libpdfium → 页视图 error 态（SKIP 横幅）；其他
    // 失败 = 夹具/真缺陷（panic，不放过）。
    let view_err = window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                shell
                    .pdf_pages
                    .as_ref()
                    .map(|(_, e)| e.read(cx).engine_error().map(str::to_owned))
            })
        })
        .expect("窗口更新");
    match view_err {
        Some(Some(e)) if e.contains("pdfium") => {
            eprintln!(
                "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                     本机无 libpdfium——#386 Shell 分流 e2e 未真跑。\n  \
                     有 libpdfium 的环境（MDC_PDFIUM_LIB=<库路径> \
                     cargo test）全量执行。\n═══════════════════════════\n"
            );
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        Some(Some(e)) => {
            panic!("页视图构造失败（夹具/真缺陷，非环境缺件）：{e}")
        }
        Some(None) => {} // 页视图在位——全量真跑。
        // 槽缺位 = 打开本身失败（无标签可挂，open_document_tab 走
        // Failed 分支）——失败文案在 notice：libpdfium 定位/装载
        // = 环境缺件（SKIP 横幅，同上口径，AGENTS 13.1 门序）；
        // 其他 = 夹具/真缺陷（panic，不放过）。
        None => {
            let notice = window
                .update(cx, |_host, _window, cx| {
                    shell.update(cx, |shell, _| {
                        shell.notice_board.message().map(str::to_owned)
                    })
                })
                .expect("窗口更新");
            match notice {
                Some(msg) if msg.contains("libpdfium") => {
                    eprintln!(
                        "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                             本机无 libpdfium——#386 Shell 分流 e2e 未真跑\n  \
                             （pdf 打开面即失败：引擎绑定缺库）。\n  \
                             有 libpdfium 的环境（MDC_PDFIUM_LIB=<库路径> \
                             cargo test）全量执行。\n═══════════════════════════\n"
                    );
                    let _ = std::fs::remove_dir_all(&dir);
                    return;
                }
                other => {
                    panic!("pdf 标签未开（夹具/真缺陷，非环境缺件）：{other:?}")
                }
            }
        }
    }

    // ① 手势驱动页视图（生产入口 = finish_drag_annotation 收尾同款
    //    commit_annotation——不跳过视图层；缺省工具 = 高亮）。
    let _ = window.update(cx, |_host, _window, cx| {
        shell.update(cx, |shell, cx| {
            let (_, entity) = shell.pdf_pages.clone().expect("槽在位");
            entity.update(cx, |v, _| {
                v.commit_annotation(0, 40., 60., 200., 120.)
                    .expect("手势提交");
            });
        });
    });
    // ② 批注脏计入标签脏判定（tab_is_modified 同源——标题栏脏标/
    //    关闭确认/RPC 同一函数）；文本面自身不脏（只读编辑器）。
    let _ = window.update(cx, |_host, _window, cx| {
        shell.update(cx, |shell, cx| {
            assert!(
                shell.tab_is_modified(0, cx).unwrap(),
                "#386：批注脏计入标签脏判定"
            );
            let ed = shell.tab_editor(0).unwrap();
            assert!(
                !ed.read(cx).is_modified(),
                "只读文本面无未存修改（脏来自页视图批注——语义分层）"
            );
        });
    });
    // ③ 关脏标签 → 确认框拦截（不静默丢——验收 3）。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.close_active_tab(window, cx);
            let confirm = shell.confirm.as_ref().expect("批注脏关闭必弹确认");
            assert!(
                confirm.message.contains("x.pdf"),
                "文案含文件名（得 {:?}）",
                confirm.message
            );
            assert_eq!(confirm.buttons, CONFIRM_BUTTONS, "保存/不保存/取消三选");
        });
    });
    // ④ 确认框选「保存」→ 标签关 + 批注增量落盘（save_tab 分流）。
    let _ = window.update(cx, |_host, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.resolve_confirm(0, window, cx);
            assert_eq!(shell.tab_count(), 0, "保存后关标签");
            assert!(shell.pdf_pages.is_none(), "标签关 → 页视图槽随之清空");
        });
    });
    // ⑤ 盘上：原前缀保持（增量非重写——验收 2）+ 独立读回 1 条
    //    （mdc-pdf 读面第二意见——验收 1 的落盘形态）。
    let after = std::fs::read(&pdf).expect("盘上文件");
    assert!(
        after.starts_with(&original),
        "#386 验收 2：盘上保持原字节前缀（增量）"
    );
    assert!(after.len() > original.len(), "批注修订段非空");
    let (annots, report) =
        mdc_pdf::annotate::read_annotations(after, mdc_pdf::security::Limits::default())
            .expect("独立读回");
    assert_eq!(report.parsed, 1, "重开应见 1 条批注：{report:?}");
    assert_eq!(annots.len(), 1);
    assert_eq!(annots[0].page.0, 0, "页号往返");

    let _ = std::fs::remove_dir_all(&dir);
}

/// #386 退出面：批注脏标签计入退出前确认（on_window_should_close
/// 的 dirty 收集同源——含页视图未落盘批注）。仅钉「计入 + 拦截」
/// 这一环（保存分流本体已由上一条钉定；不重复）。
#[gpui::test]
fn pdf_annotate_dirty_blocks_quit_confirm(cx: &mut gpui::TestAppContext) {
    let dir = std::env::temp_dir().join(format!("mdc-386-quit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let original = smoke_pdf_bytes();
    let pdf = dir.join("q.pdf");
    std::fs::write(&pdf, &original).unwrap();

    cx.update(|app| theme::init(theme::LoadThemes::JustBase, app));
    let mut slot: Option<Entity<ShellApp>> = None;
    let window = cx.add_window(|window, cx| {
        let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
        slot = Some(shell.clone());
        ShellHost(shell)
    });
    let shell = slot.expect("宿主构造即置");

    let _ = window.update(cx, |_host, _window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_document_tab(pdf.clone(), cx);
            shell.ensure_pdf_pages(cx);
        });
    });
    let view_err = window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, cx| {
                shell
                    .pdf_pages
                    .as_ref()
                    .map(|(_, e)| e.read(cx).engine_error().map(str::to_owned))
            })
        })
        .expect("窗口更新");
    match view_err {
        Some(Some(e)) if e.contains("pdfium") => {
            eprintln!(
                "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                     本机无 libpdfium——#386 退出面 e2e 未真跑。\n═════\
                     ════════════════════════════\n"
            );
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        Some(Some(e)) => {
            panic!("页视图构造失败（夹具/真缺陷，非环境缺件）：{e}")
        }
        Some(None) => {}
        // 槽缺位 = 打开本身失败（同上一条口径：libpdfium 缺件 SKIP
        // 横幅；其他失败 panic 不放过——两处门文案见上）。
        None => {
            let notice = window
                .update(cx, |_host, _window, cx| {
                    shell.update(cx, |shell, _| {
                        shell.notice_board.message().map(str::to_owned)
                    })
                })
                .expect("窗口更新");
            match notice {
                Some(msg) if msg.contains("libpdfium") => {
                    eprintln!(
                        "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                             本机无 libpdfium——#386 退出面 e2e 未真跑\n  \
                             （pdf 打开面即失败：引擎绑定缺库）。\n  \
                             有 libpdfium 的环境（MDC_PDFIUM_LIB=<库路径> \
                             cargo test）全量执行。\n═══════════════════════════\n"
                    );
                    let _ = std::fs::remove_dir_all(&dir);
                    return;
                }
                other => {
                    panic!("pdf 标签未开（夹具/真缺陷，非环境缺件）：{other:?}")
                }
            }
        }
    }

    // 手势置脏 → 模拟用户点窗口关闭钮（真实 should-close 回调链）
    //——确认框弹出 + 关闭被拦（批注未落盘不许静默退出）。
    let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
    vcx.update(|_window, cx| {
        shell.update(cx, |shell, cx| {
            let (_, entity) = shell.pdf_pages.clone().expect("槽在位");
            entity.update(cx, |v, _| {
                v.commit_annotation(0, 40., 60., 200., 120.)
                    .expect("手势提交");
            });
        });
    });
    let closed = vcx.simulate_close();
    assert!(
        !closed,
        "#386：批注脏 → should-close 返回 false（窗口关闭被拦）"
    );
    vcx.update(|_window, cx| {
        shell.update(cx, |shell, _cx| {
            let confirm = shell.confirm.as_ref().expect("退出前确认弹出");
            assert!(
                matches!(confirm.action, ConfirmAction::Quit),
                "确认动作 = Quit（得 {:?}）",
                confirm.action
            );
            assert!(
                confirm.message.contains("1"),
                "文案含未保存计数（得 {:?}）",
                confirm.message
            );
            assert_eq!(shell.tab_count(), 1, "被拦 = 标签还在（批注未静默丢）");
        });
    });

    let _ = std::fs::remove_dir_all(&dir);
}
