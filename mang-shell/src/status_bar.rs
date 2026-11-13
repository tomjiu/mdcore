//! 底部状态栏（#63 Lead 布局 + #68 阶段 1 文档状态联动 + F2 工作区）。
//!
//! 左：文档状态（文件名 + 修改标记 ● + 行,列 + 字数——无文档时「就绪」）
//! + 工作区路径（F2——打开文件夹后显示，`~` 缩写/超长截前保尾）
//! + 选中字数（#188 接线——有选区才显示）
//! + 锁定行拒改提示（#178——锁定行敲字被拒时可见，Error 色）；
//! 右：git 分支 / 文件编码 / 换行符（#188 接线——各自可判定才显示）
//! + 版本与许可标识。背景色差（title_bar_background）区分主编辑区，
//! 无 1px 边框。
//!
//! #188 接线说明：四项信息的数据层在 [`crate::status_info`]（纯函数，
//! 各自带单测——detect_eol / detect_encoding / format_selection /
//! find_git_root + git_branch_at），本模块此前零消费（状态栏只用
//! Editor 自带标签）。接线保持纯函数不动，只在渲染期组串：
//!
//! | 项 | 数据来源 | 缺省（不显示） |
//! |---|---|---|
//! | 选中字数 | 编辑器选区（selection_points + selection_text） | 无选区 |
//! | git 分支 | 工作区根向上找 .git（无工作区回退文件目录） | 非 git 仓库 |
//! | 编码 | 源文件字节（md 态——容器格式无编码语义） | 读不到/判不出 |
//! | 换行符 | 源文件字节扫描（md 态） | 无换行 |

use gpui::{App, ClickEvent, Context, Entity, Pixels, Window, px};
use ui::prelude::*;
use ui::{Label, LabelSize};

use crate::editor::Editor;
use crate::file_tree::workspace_display;
use crate::shell::ShellApp;
use crate::status_info;

/// 状态栏高度（px）。
pub const STATUS_BAR_HEIGHT: Pixels = px(24.);

/// 状态栏文案只读面（#47 A5 剩余缺口：状态栏真实文案**运行时证据**的
/// 断言面——`outline_panel::row_labels` 先例同构：内容证据不依赖像素
/// 断言（像素级文本呈现归 #554）。[`status_bar`] 渲染与
/// [`status_bar_labels`] **同源**——本结构锁「渲染消费的计算面」。
#[derive(Debug, Default)]
pub(crate) struct StatusBarLabels {
    /// 左段文档串（文档态：文件名 + 修改标记 + 行,列 + 页码 + 字数；
    /// 无文档：就绪提示语）。
    pub left: String,
    /// 工作区段（`Some` = 「  ·  工作区 …」整段；未打开 `None`）。
    pub workspace: Option<String>,
    /// 选中字数段（#188——无/空选区 `None` 不占位）。
    pub selection: Option<String>,
    /// git 分支段（#188——非 git 仓库 `None`；`Some` 带前导空格）。
    pub branch: Option<String>,
    /// 编码段（#188——仅 md 态可判；容器格式无字节级语义 `None`）。
    pub encoding: Option<String>,
    /// 换行符段（#188——仅 md 态可判 `None`）。
    pub eol: Option<String>,
}

/// 状态栏文案计算面（渲染零逻辑的单一事实源）。
pub(crate) fn status_bar_labels(
    editor: Option<&Entity<Editor>>,
    workspace: Option<std::path::PathBuf>,
    pdf_page_label: Option<String>,
    cx: &App,
) -> StatusBarLabels {
    let workspace_label = workspace
        .as_deref()
        .map(workspace_display)
        .map(|ws| format!("  ·  工作区 {ws}"));
    // #188：选区标签（无选区 = None 不占位）。
    let selection_label = match editor {
        Some(ed) => {
            let e = ed.read(cx);
            let info = e.selection_points().and_then(|(start, end)| {
                status_info::selection_info(&e.selection_text(), start.line, end.line)
            });
            status_info::format_selection(info)
        }
        None => None,
    };
    // #188：git 分支（工作区优先；无工作区回退文件所在目录——单文件
    // 打开也能看见分支）。find_git_root 只读目录不启子进程。
    let branch_label = {
        let start = workspace
            .as_deref()
            .map(std::path::Path::to_path_buf)
            .or_else(|| {
                editor
                    .and_then(|ed| ed.read(cx).file_path())
                    .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
            });
        // #281（审计 R-e）：git 分支走 HEAD mtime 缓存——状态栏随
        // ShellApp 每次重渲（每键/每拖拽）被调，此前每帧重读 .git/HEAD。
        start
            .and_then(|start| status_info::find_git_root(&start))
            .and_then(|root| status_info::git_branch_at_cached(&root))
            .map(|branch| format!(" {branch}"))
    };
    // #188：编码 + 换行符（md 态源文件——docx/pdf/xlsx/canvas 是容器
    // 格式，字节级编码/换行语义不适用，不显示；未落盘草稿无字节面）。
    // #281（审计 R-e）：探测走 mtime+size 缓存——此前每帧全文件读盘
    //（大文件持续磁盘 churn），现命中仅一次 stat。
    let (encoding_label, eol_label) = match editor {
        Some(ed) => {
            let e = ed.read(cx);
            if e.doc_kind() == "md" {
                match e
                    .file_path()
                    .and_then(status_info::detect_encoding_eol_cached)
                {
                    Some((enc, eol)) => (
                        enc.map(|enc| enc.label().to_string()),
                        eol.map(|eol| eol.label().to_string()),
                    ),
                    None => (None, None),
                }
            } else {
                (None, None)
            }
        }
        None => (None, None),
    };
    let left = match editor {
        Some(ed) => {
            let ed = ed.read(cx);
            let modified = if ed.is_read_only() {
                " · 只读"
            } else if ed.is_modified() {
                " ●"
            } else {
                ""
            };
            // 页码标签（批 4：pdf 态「第 N / M 页」——光标行经页表映射；
            // md/docx 态无页语义，不占位）。#51 D4b：**页面视图态改由
            // 页视图 top_visible_page 驱动**（Shell 传入——emit 订阅
            // 活更新；此前文本光标映射在页面态读到的是隐形编辑器的
            // 旧光标位——「当前页状态不一致」本修）。文本态 None
            // 沿用编辑器映射（零变化）。
            let page = pdf_page_label
                .or_else(|| ed.page_label().map(|l| l.to_string()))
                .map(|l| format!("  ·  {l}"))
                .unwrap_or_default();
            format!(
                "{}{modified}  ·  行, 列 {}{page}  ·  {}",
                ed.display_name(),
                ed.cursor_label(),
                ed.char_label()
            )
        }
        None => "就绪 —— Ctrl+O 打开文件 · Ctrl+K Ctrl+O 打开文件夹".to_string(),
    };
    StatusBarLabels {
        left,
        workspace: workspace_label,
        selection: selection_label,
        branch: branch_label,
        encoding: encoding_label,
        eol: eol_label,
    }
}

/// 渲染状态栏。`editor`：打开的编辑器实体（`None` = 无文档就绪态）；
/// `workspace`：工作区根（F2——`None` = 未打开）；`pdf_view_mode`：
/// #289-④ pdf 页面视图开关态（仅 pdf 标签活跃时消费——切换钮挂
/// 右侧信息区）；`xlsx_grid_view_mode`：#389 阶段 1 xlsx 网格视图
/// 开关态（仅 xlsx 标签活跃时消费——网格/文本切换钮，pdf 先例同构）；
/// `docx_view_mode`：#50 C1 docx LOK 页面视图开关态（仅 docx 标签活
/// 跃时消费——页面/文本切换钮）。
pub fn status_bar(
    editor: Option<&Entity<Editor>>,
    workspace: Option<std::path::PathBuf>,
    pdf_view_mode: bool,
    xlsx_grid_view_mode: bool,
    docx_view_mode: bool,
    pdf_page_label: Option<String>,
    cx: &mut Context<ShellApp>,
) -> impl IntoElement {
    let colors = cx.theme().colors().clone();
    // #47 A5：文案计算面单一事实源（渲染零逻辑——运行时证据测试直接
    // 断言 [`status_bar_labels`]，与本渲染同源）。
    let labels = status_bar_labels(editor, workspace, pdf_page_label, cx);
    let left = {
        let mut left = labels.left;
        if let Some(ws) = &labels.workspace {
            left.push_str(ws);
        }
        // #188：选区串在左段尾部（与字数同族——文档内容面）。
        if let Some(sel) = &labels.selection {
            left.push_str(&format!("  ·  {sel}"));
        }
        left
    };
    let locked_notice = editor.and_then(|ed| ed.read(cx).locked_notice().cloned());
    // #289-④：pdf 标签 → 页面/文本视图切换钮（右侧信息区，git 分支
    // 前——文档形态面）。非 pdf 态不占位。
    // #585 残余①：文案经 status_info 纯函数（状态 → 文案的单一事实源
    // ——穷尽两态映射在 status_info 单测锁，此处零内联字面量）。
    let is_pdf = editor.is_some_and(|ed| ed.read(cx).doc_kind() == "pdf");
    let pdf_toggle_label = status_info::pdf_toggle_label(pdf_view_mode);
    // #389 阶段 1：xlsx 标签 → 网格/文本视图切换钮（pdf 先例同构——
    // 非 xlsx 态不占位；#585 残余①同源收口）。
    let is_xlsx = editor.is_some_and(|ed| ed.read(cx).doc_kind() == "xlsx");
    let xlsx_toggle_label = status_info::xlsx_toggle_label(xlsx_grid_view_mode);
    // #50 C1：docx 标签 → 页面/文本视图切换钮（pdf/xlsx 先例同构——
    // 非 docx 态不占位；#585 零内联字面量纪律同源）。
    let is_docx = editor.is_some_and(|ed| ed.read(cx).doc_kind() == "docx");
    let docx_toggle_label = status_info::docx_toggle_label(docx_view_mode);
    let hover = colors.element_hover;

    h_flex()
        .id("status-bar")
        .h(STATUS_BAR_HEIGHT)
        .flex_none()
        .w_full()
        .bg(colors.title_bar_background)
        .px_3()
        .justify_between()
        .child(
            Label::new(left)
                .size(LabelSize::XSmall)
                .color(ui::Color::Muted),
        )
        .child(
            h_flex()
                .gap_3()
                // #178：锁定行拒改提示（Error 色——「此行受保护，
                // 不可编辑」+ 具体原因；无拒改不占位）。
                .when_some(locked_notice, |el, notice| {
                    el.child(
                        Label::new(notice.label())
                            .size(LabelSize::XSmall)
                            .color(ui::Color::Error),
                    )
                })
                // #289-④：pdf 页面/文本视图切换（真实行为钮——
                // toggle_pdf_view_mode；非 pdf 标签不占位）。
                .when(is_pdf, |el| {
                    el.child(
                        div()
                            .id("pdf-view-toggle")
                            .px_2()
                            .rounded_sm()
                            .hover(move |el| el.bg(hover))
                            .on_click(cx.listener(
                                |this, _ev: &ClickEvent, window: &mut Window, cx| {
                                    this.toggle_pdf_view_mode(window, cx);
                                },
                            ))
                            .child(
                                Label::new(pdf_toggle_label)
                                    .size(LabelSize::XSmall)
                                    .color(ui::Color::Muted),
                            ),
                    )
                })
                // #389 阶段 1：xlsx 网格/文本视图切换（pdf 先例同构——
                // 非 xlsx 标签不占位）。
                .when(is_xlsx, |el| {
                    el.child(
                        div()
                            .id("xlsx-view-toggle")
                            .px_2()
                            .rounded_sm()
                            .hover(move |el| el.bg(hover))
                            .on_click(cx.listener(
                                |this, _ev: &ClickEvent, window: &mut Window, cx| {
                                    this.toggle_xlsx_view_mode(window, cx);
                                },
                            ))
                            .child(
                                Label::new(xlsx_toggle_label)
                                    .size(LabelSize::XSmall)
                                    .color(ui::Color::Muted),
                            ),
                    )
                })
                // #50 C1：docx 页面/文本视图切换（LOK 内核页视图 ↔ 行投
                // 影；非 docx 标签不占位）。
                .when(is_docx, |el| {
                    el.child(
                        div()
                            .id("docx-view-toggle")
                            .px_2()
                            .rounded_sm()
                            .hover(move |el| el.bg(hover))
                            .on_click(cx.listener(
                                |this, _ev: &ClickEvent, window: &mut Window, cx| {
                                    this.toggle_docx_view_mode(window, cx);
                                },
                            ))
                            .child(
                                Label::new(docx_toggle_label)
                                    .size(LabelSize::XSmall)
                                    .color(ui::Color::Muted),
                            ),
                    )
                })
                // #188：git 分支（如  main）——非 git 目录不占位。
                .when_some(labels.branch, |el, branch| {
                    el.child(
                        Label::new(branch)
                            .size(LabelSize::XSmall)
                            .color(ui::Color::Muted),
                    )
                })
                // #188：文件编码（UTF-8 / GBK / …）——判不出不占位。
                .when_some(labels.encoding, |el, enc| {
                    el.child(
                        Label::new(enc)
                            .size(LabelSize::XSmall)
                            .color(ui::Color::Muted),
                    )
                })
                // #188：换行符（LF / CRLF / 混合换行）——无换行不占位。
                .when_some(labels.eol, |el, eol| {
                    el.child(
                        Label::new(eol)
                            .size(LabelSize::XSmall)
                            .color(ui::Color::Muted),
                    )
                })
                .child(
                    Label::new("mdcore 0.1.0")
                        .size(LabelSize::XSmall)
                        .color(ui::Color::Muted),
                )
                .child(
                    Label::new("GPL-3.0-or-later")
                        .size(LabelSize::XSmall)
                        .color(ui::Color::Muted),
                ),
        )
}
