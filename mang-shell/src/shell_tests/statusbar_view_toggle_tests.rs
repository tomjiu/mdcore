//! #47 巨石拆解批①：原 shell.rs 内嵌 `statusbar_view_toggle_tests` 纯迁出（零行为变化——
//! 字节原样，挂载点 #[cfg(test)] + #[path] 在 shell.rs）。

use super::*;
use gpui::{
    AnyWindowHandle, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PlatformInput, point, px,
};

/// 宿主：shell 进渲染树（鼠标命中测试需要整树 + 命中盒——
/// pdf_search_wiring_tests WiringHost 同款；空 div 宿主会把状态栏
/// 挡在派发树外，点击无法路由）。
struct StatusHost(Entity<ShellApp>);

impl gpui::Render for StatusHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div().size_full().child(self.0.clone())
    }
}

/// 整树宿主 + 窗口（theme provider 双注册——#422 同款：整树鼠标
/// 派发需要 provider 在位）。
fn host_window(
    cx: &mut gpui::TestAppContext,
) -> (Entity<ShellApp>, gpui::WindowHandle<StatusHost>) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let mut slot: Option<Entity<ShellApp>> = None;
    let window = cx.add_window(|window, cx| {
        let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
        slot = Some(shell.clone());
        StatusHost(shell)
    });
    (slot.expect("宿主构造即置"), window)
}

/// 双视图态快照（pdf_view_mode, xlsx_grid_view_mode——私有字段
/// 同文件测试面直读）。
fn view_modes(shell: &Entity<ShellApp>, cx: &mut gpui::TestAppContext) -> (bool, bool) {
    cx.read(|app| {
        let s = shell.read(app);
        (s.pdf_view_mode, s.xlsx_grid_view_mode)
    })
}

/// 重绘一帧（命中盒随最新状态重注册——update_window 口径）。
fn redraw(window: &gpui::WindowHandle<StatusHost>, cx: &mut gpui::TestAppContext) {
    let any: AnyWindowHandle = window.clone().into();
    cx.update_window(any, |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .expect("重绘一帧");
}

/// 真实单击：MouseDown + MouseUp 同点派发（update_window 口径——
/// 不包根视图 update，shell 级监听器不重入；main.rs 烟雾同构）。
fn click_at(
    window: &gpui::WindowHandle<StatusHost>,
    cx: &mut gpui::TestAppContext,
    x: f32,
    y: f32,
) {
    let any: AnyWindowHandle = window.clone().into();
    let down = PlatformInput::MouseDown(MouseDownEvent {
        button: MouseButton::Left,
        position: point(px(x), px(y)),
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    let up = PlatformInput::MouseUp(MouseUpEvent {
        button: MouseButton::Left,
        position: point(px(x), px(y)),
        modifiers: Modifiers::default(),
        click_count: 1,
    });
    cx.update_window(any, |_, window, cx| {
        window.dispatch_event(down, cx);
        window.dispatch_event(up, cx);
    })
    .expect("单击派发");
}

/// 状态栏右区命中扫描：从右缘（viewport 宽 - px_3 内边距）向左
/// 步进 8px 单击至中线，直到**任一**视图态翻转。返回 (命中 x, 翻转后
/// 双态)；扫完零翻转 = None（该形态无交互钮——md 态零占位的
/// 运行时证据）。
///
/// y = 状态栏中线（viewport 高 - 12：状态栏 = 底部 24px——
/// 垂直方向只可能与状态栏命中盒相交，编辑区/面板/标签栏均在其上；
/// 测试平台 display 为 1920×1080——坐标从 viewport_size 动态推
/// 导，不硬编码）。
fn sweep_statusbar_right(
    shell: &Entity<ShellApp>,
    window: &gpui::WindowHandle<StatusHost>,
    cx: &mut gpui::TestAppContext,
) -> Option<(f32, (bool, bool))> {
    let (right_edge, mid_x, status_y) = window
        .update(cx, |_host, window, _cx| {
            let size = window.viewport_size();
            (
                f32::from(size.width) - 12.0,
                f32::from(size.width) / 2.0,
                f32::from(size.height) - 12.0,
            )
        })
        .expect("读 viewport 尺寸");
    redraw(window, cx);
    let mut x = right_edge;
    while x >= mid_x {
        let before = view_modes(shell, cx);
        click_at(window, cx, x, status_y);
        let after = view_modes(shell, cx);
        if after != before {
            return Some((x, after));
        }
        x -= 8.0;
    }
    None
}

/// smoke.xlsx 夹具路径（只读打开——本测试面不保存，零污染）。
fn smoke_xlsx() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.xlsx")
}

/// 状态栏中线 y（viewport 高 - 12——底部 24px 状态栏的垂直中心；
/// 回切重击前的坐标重取）。
fn statusbar_y(window: &gpui::WindowHandle<StatusHost>, cx: &mut gpui::TestAppContext) -> f32 {
    window
        .update(cx, |_host, window, _cx| {
            f32::from(window.viewport_size().height) - 12.0
        })
        .expect("读 viewport 高")
}

/// xlsx 面：网格态起（初始 true）→ 扫描命中切换钮 → 翻转到文本态
/// （false）→ 同点再击回切网格态（true）→ pdf 态全程不受扰
/// （两钮互斥在场，点击不串道）。
#[gpui::test]
fn xlsx_view_toggle_click_flips_and_restores(cx: &mut gpui::TestAppContext) {
    let (shell, window) = host_window(cx);
    window
        .update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_document(smoke_xlsx(), window, cx);
            });
        })
        .expect("开 xlsx 标签");
    cx.run_until_parked();
    let (pdf0, xlsx0) = view_modes(&shell, cx);
    assert!(xlsx0, "xlsx 态初始 = 网格视图（ShellApp 默认 true）");
    assert!(pdf0, "pdf 态默认 true（未开 pdf 标签不受扰）");

    // 扫描命中（Some = 右区存在交互钮且点击真实翻转了状态）。
    // #585 残余①：文案联动前置（网格态 → 钮显示「去文本」）。
    assert_eq!(
        crate::status_info::xlsx_toggle_label(xlsx0),
        "切换到文本视图"
    );

    let (hit_x, (pdf1, xlsx1)) =
        sweep_statusbar_right(&shell, &window, cx).expect("xlsx 标签活跃：右区应命中切换钮");
    assert!(!xlsx1, "第一击：网格 → 文本（true → false）");
    assert!(pdf1 == pdf0, "xlsx 钮点击不串扰 pdf 态");
    // #585 残余①：真实点击翻转 ⇒ 钮文案随纯函数翻转（渲染消费
    // 同一 `xlsx_toggle_label`——状态与文案闭环）。
    assert_eq!(
        crate::status_info::xlsx_toggle_label(xlsx1),
        "切换到网格视图",
        "翻转后钮文案应翻转为「切换到网格视图」"
    );

    // 同点回切（文案两态等宽——按钮不位移；仍重绘保命中盒新鲜）。
    let status_y = statusbar_y(&window, cx);
    redraw(&window, cx);
    click_at(&window, cx, hit_x, status_y);
    let (pdf2, xlsx2) = view_modes(&shell, cx);
    assert!(xlsx2, "第二击：文本 → 网格（false → true，回切闭环）");
    assert!(pdf2 == pdf0, "回切仍不串扰 pdf 态");
}

/// md 面：非 pdf/xlsx 标签——右区全扫零翻转（双钮零占位的运行时
/// 证据：用户点了没反应；占位渲染与 toggle 守卫同谓词，见模块
/// 头「条件占位断言边界」）。
#[gpui::test]
fn md_tab_statusbar_right_zone_is_inert(cx: &mut gpui::TestAppContext) {
    let dir = std::env::temp_dir().join(format!("mdc-b11-md-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let md = dir.join("note.md");
    std::fs::write(&md, "批 11\n状态栏零占位\n").unwrap();

    let (shell, window) = host_window(cx);
    window
        .update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_document(md.clone(), window, cx);
            });
        })
        .expect("开 md 标签");
    cx.run_until_parked();

    let before = view_modes(&shell, cx);
    let swept = sweep_statusbar_right(&shell, &window, cx);
    assert!(
        swept.is_none(),
        "md 标签：右区全扫零翻转（得 {:?}——存在不应在场的交互钮）",
        swept.map(|(x, _)| x)
    );
    assert_eq!(view_modes(&shell, cx), before, "双视图态纹丝不动");

    let _ = std::fs::remove_dir_all(&dir);
}

/// pdf 面：pdf 标签活跃 → 扫描命中 → 翻转回切（xlsx 态不串扰）。
/// 引擎门控：无 libpdfium 时开档即失败（open 走 Failed 分支）→
/// SKIP 横幅（AGENTS 13.1——不伪装通过）；#422 pdf_view_ctrl_f
/// 同口径。
#[gpui::test]
fn pdf_view_toggle_click_flips_and_restores(cx: &mut gpui::TestAppContext) {
    let pdf_bytes =
        std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.pdf"))
            .unwrap_or_else(|e| panic!("夹具不可读：{e}"));
    let dir = std::env::temp_dir().join(format!("mdc-b11-pdf-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pdf = dir.join("s.pdf");
    std::fs::write(&pdf, pdf_bytes).unwrap();

    let (shell, window) = host_window(cx);
    window
        .update(cx, |_host, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_document(pdf.clone(), window, cx);
            });
        })
        .expect("开 pdf 标签");
    cx.run_until_parked();
    // 引擎门控：无 libpdfium → pdf_doc::load 失败 → 无标签 + notice。
    let open_ok = window
        .update(cx, |_host, _window, cx| {
            shell.update(cx, |shell, _| shell.tab_count() == 1)
        })
        .expect("查标签数");
    if !open_ok {
        let notice = window
            .update(cx, |_host, _window, cx| {
                shell.update(cx, |shell, _| {
                    shell.notice_board.message().map(str::to_owned)
                })
            })
            .expect("查 notice");
        if notice.as_deref().is_some_and(|m| m.contains("libpdfium")) {
            eprintln!(
                "\n══════ SKIP（环境缺件，不伪装通过）══════\n  \
                     本机无 libpdfium——批 11 pdf 视图切换钮 e2e 未真跑\n  \
                     （xlsx/md 两面已全量真跑——覆盖同一接线模式）。\n  \
                     有 libpdfium 的环境全量执行。\n═════════════════════════\n"
            );
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        panic!("pdf 标签开档失败（夹具/真缺陷，非环境缺件）：{notice:?}");
    }

    // #585 残余③：PDFium 真跑的显式日志证据（bin/lib/libpdfium.so
    // 在位——开档成功 = 引擎已绑定，本测试全量执行非 SKIP）。
    eprintln!(
        "\n══════ PDFIUM 真跑（非 SKIP）══════\n               pdf 标签开档成功 = libpdfium 引擎已绑定（定位序 \
             MDC_PDFIUM_LIB → bin/lib 取件位 → 系统）。\n               下方翻转/回切为真实引擎态下的完整点击链。\n═══════════════════════════\n"
    );
    let (pdf0, xlsx0) = view_modes(&shell, cx);
    assert!(pdf0, "pdf 态初始 = 页面视图（ShellApp 默认 true）");
    // ① 文案联动前置：钮上文案 = 状态的纯函数映射（渲染消费同一
    // 函数——翻转前为「去文本」）。
    assert_eq!(crate::status_info::pdf_toggle_label(pdf0), "切换到文本视图");

    let (hit_x, (pdf1, xlsx1)) =
        sweep_statusbar_right(&shell, &window, cx).expect("pdf 标签活跃：右区应命中切换钮");
    assert!(!pdf1, "第一击：页面 → 文本（true → false）");
    assert!(xlsx1 == xlsx0, "pdf 钮点击不串扰 xlsx 态");
    // #585 残余①：真实点击翻转状态 ⇒ 钮文案随纯函数翻转（渲染
    // 消费同一 `pdf_toggle_label`——状态与文案在同一事实源上闭环）。
    assert_eq!(
        crate::status_info::pdf_toggle_label(pdf1),
        "切换到页面视图",
        "翻转后钮文案应翻转为「切换到页面视图」"
    );

    let status_y = statusbar_y(&window, cx);
    redraw(&window, cx);
    click_at(&window, cx, hit_x, status_y);
    let (pdf2, _) = view_modes(&shell, cx);
    assert!(pdf2, "第二击：文本 → 页面（false → true，回切闭环）");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 整树宿主 + **显尺寸窗口**（A5 布局敏感面——`cx.open_window` 布局
/// 控制口径，add_window 是最大化尺寸不可控）。
fn host_window_sized(
    cx: &mut gpui::TestAppContext,
    w: f32,
    h: f32,
) -> (Entity<ShellApp>, gpui::WindowHandle<StatusHost>) {
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let mut slot: Option<Entity<ShellApp>> = None;
    let window = cx.open_window(gpui::size(px(w), px(h)), |window, cx| {
        let shell = cx.new(|cx| ShellApp::new(None, None, window, cx));
        slot = Some(shell.clone());
        StatusHost(shell)
    });
    (slot.expect("宿主构造即置"), window)
}

/// 仅 MouseDown（命中探针——down 侧监听器即置 `panel_drag` 态；
/// up 免发：拖拽会话由测试直接清态，不经持久化路径）。
fn mouse_down_at(
    window: &gpui::WindowHandle<StatusHost>,
    cx: &mut gpui::TestAppContext,
    x: f32,
    y: f32,
) {
    let any: AnyWindowHandle = window.clone().into();
    let down = PlatformInput::MouseDown(MouseDownEvent {
        button: MouseButton::Left,
        position: point(px(x), px(y)),
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    cx.update_window(any, |_, window, cx| window.dispatch_event(down, cx))
        .expect("MouseDown 派发");
}

/// 右栏拖拽热区 x（右缘锚定：dock 条 52 + 右栏宽 right_width——
/// 热区 4px 在右栏左缘，取中点 2px）。探针断言依据：**热区元素
/// 只在 `right_open_effective` 臂内渲染**（GPUI `when(false, f)` 不
/// 应用 f——元素/命中盒不存在），MouseDown 命中与否即让位证据。
fn right_hotzone_x(right_width: f32, w: f32) -> f32 {
    w - 52. - right_width - 2.
}

/// #121（批③b）：全量 mouse 事件派发（down/move/up 一族——探针
/// 面 mouse_down_at 只发 down；拖拽链需要 move>2px 激活 + up 落点）。
fn mouse_event_at(
    window: &gpui::WindowHandle<StatusHost>,
    cx: &mut gpui::TestAppContext,
    kind: &str,
    x: f32,
    y: f32,
) {
    let any: AnyWindowHandle = window.clone().into();
    let pos = point(px(x), px(y));
    cx.update_window(any, |_, window, cx| {
        let event = match kind {
            "down" => PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position: pos,
                modifiers: Modifiers::default(),
                click_count: 1,
                first_mouse: false,
            }),
            "move" => PlatformInput::MouseMove(MouseMoveEvent {
                position: pos,
                pressed_button: Some(MouseButton::Left),
                modifiers: Modifiers::default(),
            }),
            "up" => PlatformInput::MouseUp(MouseUpEvent {
                button: MouseButton::Left,
                position: pos,
                modifiers: Modifiers::default(),
                click_count: 1,
            }),
            _ => unreachable!("kind ∈ down/move/up"),
        };
        window.dispatch_event(event, cx);
    })
    .expect("mouse 事件派发");
}

/// #121（批③b）：真实鼠标事件链 e2e——GitPanel 左→右换侧。
///
/// 链路：标题手柄 down → move（>2px 激活 drag 会话）→ move 步进
/// 右分区（x > 2/3 窗宽）→ 覆盖层 on_drop（up 命中）→
/// finish_panel_drop：take（左 dock 拔出）→ receive（右 dock 落位
/// 即激活 = 挤掉原槽位）→ set_position(Right) → persist。
/// 验收 1（真实鼠标事件链）+ 2/3（换侧落位 + 挤掉原槽位）+ 4
/// （持久化 round-trip——右 dock dump 键在场）。
#[gpui::test]
fn drag_git_panel_to_right_via_real_mouse(cx: &mut gpui::TestAppContext) {
    let (shell, window) = host_window_sized(cx, 1000., 600.);
    let any: AnyWindowHandle = window.clone().into();
    let mut visual = gpui::VisualTestContext::from_window(any, cx);
    // 临时 git 工作区（GitPanel 物化 + 注册左 dock——与真实打开
    // 工作区同构；git 缺席环境跳过不假绿）。
    let ws = std::env::temp_dir().join(format!("mdc-121b-e2e-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&ws);
    let git_ok = std::process::Command::new("git")
        .arg("init")
        .arg(&ws)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !git_ok {
        eprintln!("SKIP（git 不可用——真实拖拽链未跑）");
        return;
    }
    cx.update(|app| {
        let shell = shell.clone();
        app.new(|cx| {
            let _ = &shell; // 实体已建——走 shell 方法面
        });
    });
    shell.update(cx, |this, cx| {
        this.open_workspace(ws.clone(), cx);
    });
    // 打开 git 面板（物化 + 左 dock 注册 + 激活）。
    shell.update(cx, |this, cx| {
        this.left_dock.update(cx, |dock, cx| {
            dock.activate_key(crate::git_panel::GIT_PANEL_KEY, cx)
        });
    });
    // 泵一帧 + debug_bounds 定位手柄真实几何（树内偏移自然计入——
    // 手拍坐标不可靠的教训修正）。
    visual.update(|window, cx| window.draw(cx).clear(cx));
    let handle_bounds = visual
        .debug_bounds("panel-drag-git")
        .expect("git 标题手柄真实渲染（debug_selector 观测面）");
    let (hx, hy) = (
        f32::from(handle_bounds.origin.x) + f32::from(handle_bounds.size.width) / 2.,
        f32::from(handle_bounds.origin.y) + f32::from(handle_bounds.size.height) / 2.,
    );

    // GitPanel 物化校验（未物化 = 测试环境局限，如实跳过）。
    let git_materialized = shell.read_with(cx, |this, cx| {
        this.left_dock
            .read(cx)
            .panel_position(crate::git_panel::GIT_PANEL_KEY, cx)
    });
    let Some(before_pos) = git_materialized else {
        eprintln!("SKIP（GitPanel 未物化（无工作区语义）——真实拖拽链未跑）");
        return;
    };
    assert_eq!(
        before_pos,
        mdc_dock::DockPosition::Left,
        "git 面板初始位 = 左"
    );
    // 真实鼠标链：down 手柄中心 → move >2px 激活 drag 会话 →
    // move 步进右分区（x > 2/3 × 1000 = 667）。
    mouse_event_at(&window, cx, "down", hx, hy);
    mouse_event_at(&window, cx, "move", hx + 10., hy + 4.);
    // move 步进到右分区（x > 2/3 × 1000 = 667）。
    mouse_event_at(&window, cx, "move", 850., 300.);
    // 拖拽中：drag_zone = Right（分区追踪断言——覆盖层状态面）。
    let zone_now = shell.read_with(cx, |this, _| this.drag_zone);
    assert_eq!(
        zone_now,
        Some(mdc_dock::DockDropZone::Right),
        "右分区追踪（on_drag_move → drag_zone）"
    );
    // up 落点右分区 → on_drop → finish_panel_drop。
    mouse_event_at(&window, cx, "up", 850., 300.);
    // ---- 落位断言（验收 2/3：换侧 + 挤掉原槽位）----
    let (left_active, right_active, right_len) = shell.read_with(cx, |this, cx| {
        (
            this.left_dock.read(cx).active_key(),
            this.right_dock.read(cx).active_key(),
            this.right_dock.read(cx).panels_len(),
        )
    });
    assert_eq!(
        right_active,
        Some(crate::git_panel::GIT_PANEL_KEY),
        "git 面板落右 dock 且激活（挤掉原槽位 = receive 即激活）"
    );
    assert_eq!(right_len, 1, "右 dock 恰收 1 面板");
    assert_ne!(
        left_active,
        Some(crate::git_panel::GIT_PANEL_KEY),
        "左 dock 不再持有 git 面板（take 拔出）"
    );
    // set_position 生效（面板实体位 = Right）。
    let after_pos = shell.read_with(cx, |this, cx| {
        this.right_dock
            .read(cx)
            .panel_position(crate::git_panel::GIT_PANEL_KEY, cx)
    });
    assert_eq!(
        after_pos,
        Some(mdc_dock::DockPosition::Right),
        "set_position(Right) 生效"
    );
    // 右栏开合联动。
    let right_open = shell.read_with(cx, |this, _| this.right_open);
    assert!(right_open, "落右侧时右栏开");
    // 拖拽终态清理。
    let zone_after = shell.read_with(cx, |this, _| this.drag_zone);
    assert_eq!(zone_after, None, "drop 后分区态清空");
    // ---- 验收 4：持久化 round-trip（右 dock dump 键在场）----
    let right_dump = shell.read_with(cx, |this, cx| this.right_dock.read(cx).dump());
    assert!(
        right_dump
            .panels
            .iter()
            .any(|p| p.key == crate::git_panel::GIT_PANEL_KEY),
        "右 dock dump 含 git 键（persist 面数据源）"
    );
    // 工作副本清理。
    let _ = std::fs::remove_dir_all(&ws);
}

/// #121（批③b）：Center 分区 = Cancel（拖回中心无动作——面板留源）。
/// 事件链同上但 up 落中央（x/y 均在中段）。
#[gpui::test]
fn drag_center_zone_cancels(cx: &mut gpui::TestAppContext) {
    let (shell, window) = host_window_sized(cx, 1000., 600.);
    let ws = std::env::temp_dir().join(format!("mdc-121b-ctr-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&ws);
    let git_ok = std::process::Command::new("git")
        .arg("init")
        .arg(&ws)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !git_ok {
        eprintln!("SKIP（git 不可用——Center 取消链未跑）");
        return;
    }
    shell.update(cx, |this, cx| {
        this.open_workspace(ws.clone(), cx);
    });
    shell.update(cx, |this, cx| {
        this.left_dock.update(cx, |dock, cx| {
            dock.activate_key(crate::git_panel::GIT_PANEL_KEY, cx)
        });
    });
    let any: AnyWindowHandle = window.clone().into();
    cx.update_window(any, |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .expect("预绘帧");
    let git_materialized = shell.read_with(cx, |this, cx| {
        this.left_dock
            .read(cx)
            .panel_position(crate::git_panel::GIT_PANEL_KEY, cx)
    });
    if git_materialized.is_none() {
        eprintln!("SKIP（GitPanel 未物化——Center 取消链未跑）");
        return;
    }
    // 标题 down → 激活 move → 中央 up。
    mouse_event_at(&window, cx, "down", 100., 58.);
    mouse_event_at(&window, cx, "move", 110., 24.);
    mouse_event_at(&window, cx, "move", 500., 300.);
    mouse_event_at(&window, cx, "up", 500., 300.);
    // Cancel：面板仍在左 dock 且激活位不动。
    let (left_has_git, right_len, zone_after) = shell.read_with(cx, |this, cx| {
        (
            this.left_dock
                .read(cx)
                .active_key()
                .is_some_and(|k| k == crate::git_panel::GIT_PANEL_KEY),
            this.right_dock.read(cx).panels_len(),
            this.drag_zone,
        )
    });
    assert!(left_has_git, "Center = Cancel：git 面板留左 dock 激活位");
    assert_eq!(right_len, 0, "右 dock 不收面板");
    assert_eq!(zone_after, None, "drop 后分区态清空");
    let _ = std::fs::remove_dir_all(&ws);
}

/// #121（批④）：呈现面统一——拖 git 到右侧后右栏渲染 GitPanel 本体。
/// 真实鼠标链（批③b e2e 同构）+ debug_selector 观测面。
#[gpui::test]
fn drag_git_to_right_renders_panel_body(cx: &mut gpui::TestAppContext) {
    let (shell, window) = host_window_sized(cx, 1000., 600.);
    let any: AnyWindowHandle = window.clone().into();
    let mut visual = gpui::VisualTestContext::from_window(any, cx);
    let ws = std::env::temp_dir().join(format!("mdc-121b4-e2e-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&ws);
    let git_ok = std::process::Command::new("git")
        .arg("init")
        .arg(&ws)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !git_ok {
        eprintln!("SKIP（git 不可用——批④呈现链未跑）");
        return;
    }
    shell.update(cx, |this, cx| {
        this.open_workspace(ws.clone(), cx);
    });
    shell.update(cx, |this, cx| {
        this.left_dock.update(cx, |dock, cx| {
            dock.activate_key(crate::git_panel::GIT_PANEL_KEY, cx)
        });
    });
    visual.update(|window, cx| window.draw(cx).clear(cx));
    let handle_bounds = visual
        .debug_bounds("panel-drag-git")
        .expect("git 标题手柄真实渲染（拖前在左栏）");
    let (hx, hy) = (
        f32::from(handle_bounds.origin.x) + f32::from(handle_bounds.size.width) / 2.,
        f32::from(handle_bounds.origin.y) + f32::from(handle_bounds.size.height) / 2.,
    );
    // 拖前：右栏是旧视图（无 right-dock-content 选择器）。
    assert!(
        visual.debug_bounds("right-dock-content").is_none(),
        "拖前右栏走旧 right_panel 兜底"
    );
    assert!(
        shell.read_with(cx, |this, cx| this.git_active(cx)),
        "拖前 git_active=true（左 dock 激活）"
    );
    // 真实拖拽链：down → move 激活 → 右分区 → up。
    mouse_event_at(&window, cx, "down", hx, hy);
    mouse_event_at(&window, cx, "move", hx + 10., hy + 4.);
    mouse_event_at(&window, cx, "move", 850., 300.);
    mouse_event_at(&window, cx, "up", 850., 300.);
    // 落位后重绘（呈现面换新——drop 后 notify 已触发，泵一帧）。
    visual.update(|window, cx| window.draw(cx).clear(cx));
    // ---- 呈现面断言（批④核心）----
    let body = visual
        .debug_bounds("right-dock-content")
        .expect("右栏呈现 right_dock 激活面板（GitPanel 本体）");
    let body_w = f32::from(body.size.width);
    assert!(body_w > 100., "右栏面板体宽度合理（实得 {body_w}px）");
    // git_active 跨 dock 合查仍 true（批④修的菜单态真源）。
    assert!(
        shell.read_with(cx, |this, cx| this.git_active(cx)),
        "拖后 git_active=true（right dock 激活——两 dock 合查）"
    );
    // 右 dock 激活 git + 手柄可再拖（呈现面手柄在场）。
    let right_active = shell.read_with(cx, |this, cx| this.right_dock.read(cx).active_key());
    assert_eq!(
        right_active,
        Some(crate::git_panel::GIT_PANEL_KEY),
        "right dock 激活 git"
    );
    let handle_after = visual
        .debug_bounds("panel-drag-git")
        .expect("右栏标题手柄在场（可再拖回左侧）");
    // 手柄应在右半区（回拖起点真实在场）。
    assert!(
        f32::from(handle_after.origin.x) > 600.,
        "手柄已随面板到右栏（x 实得 {:.0}）",
        f32::from(handle_after.origin.x)
    );
    // git_panel 实体在场（呈现的是本体非占位）。
    let git_present = shell.read_with(cx, |this, _| this.git_panel.is_some());
    assert!(git_present, "GitPanel 实体在场（本体呈现）");
    let _ = std::fs::remove_dir_all(&ws);
}

/// A5（#47 批次 13）：**实际窗口宽变化**下的右 dock 渲染期让位与
/// 恢复——真实尺寸事件（`simulate_window_resize`）+ 真实 MouseDown
/// 命中管线（拖拽热区探针）：
/// - 宽窗（1000px ≥ 900px 临界）：热区在场 → MouseDown 置
///   `panel_drag`（Some(Right)）——右栏体真实渲染；
/// - 窄窗（760px < 900px 临界）：右栏让位 → 热区**不渲染** →
///   MouseDown 零命中（`panel_drag` 保持 None），且 `right_open`
///   用户开关态**不被改写**（让位是渲染期判定）；
/// - 恢复宽窗：热区回归 → 命中恢复（按原开关态自动回归）。
#[gpui::test]
fn narrow_window_right_dock_yields_and_restores(cx: &mut gpui::TestAppContext) {
    let (shell, window) = host_window_sized(cx, 1000., 600.);

    // ① 宽窗：右栏在场（right_open 缺省开 + 1000 ≥ 900 临界）。
    redraw(&window, cx);
    let (right_open0, right_width0) =
        shell.read_with(cx, |s, _| (s.right_open, f32::from(s.right_width)));
    assert!(right_open0, "前置：right_open 用户态开（缺省）");
    assert!(
        right_width0 > 0. && right_width0 < 900.,
        "右栏宽在位（实得 {right_width0}px）"
    );
    let hot_x = right_hotzone_x(right_width0, 1000.);
    mouse_down_at(&window, cx, hot_x, 300.);
    let drag0 = shell.read_with(cx, |s, _| s.panel_drag.as_ref().map(|d| (d.side, d.last_x)));
    assert!(
        matches!(drag0, Some((PanelSide::Right, _))),
        "宽窗热区命中 → panel_drag = Some(Right)（实得 {drag0:?}）"
    );
    // 探针复位（不经 finish——零持久化副作用）。
    shell.update(cx, |s, _| {
        s.panel_drag = None;
    });

    // ② 窄窗（760px < 900px 临界）：右栏让位——热区不渲染，MouseDown
    //    零命中；right_open 用户态不被改写。
    let any: AnyWindowHandle = window.clone().into();
    cx.simulate_window_resize(any, gpui::size(px(760.), px(600.)));
    redraw(&window, cx);
    let viewport_w = window
        .update(cx, |_, window, _| f32::from(window.viewport_size().width))
        .expect("视口宽");
    assert_eq!(viewport_w, 760., "尺寸事件生效（viewport 760px）");
    let hot_x_narrow = right_hotzone_x(right_width0, 760.);
    mouse_down_at(&window, cx, hot_x_narrow, 300.);
    let (drag1, right_open1) = shell.read_with(cx, |s, _| (s.panel_drag.clone(), s.right_open));
    assert!(
        drag1.is_none(),
        "窄窗让位 → 热区不渲染 → MouseDown 零命中（panel_drag 仍 None）"
    );
    assert!(
        right_open1,
        "让位不改写用户开关态（right_open 保持 true——宽度恢复后按原态回归）"
    );

    // ③ 恢复宽窗：热区回归 → 命中恢复（同 x 同 y）。
    cx.simulate_window_resize(any, gpui::size(px(1000.), px(600.)));
    redraw(&window, cx);
    mouse_down_at(&window, cx, hot_x, 300.);
    let drag2 = shell.read_with(cx, |s, _| s.panel_drag.clone());
    assert!(
        matches!(
            drag2,
            Some(PanelDragState {
                side: PanelSide::Right,
                ..
            })
        ),
        "恢复宽窗 → 热区回归 → 命中恢复（panel_drag = Some(Right)）"
    );
}
