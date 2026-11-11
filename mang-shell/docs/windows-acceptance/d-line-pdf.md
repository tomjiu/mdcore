# D 线（#51 PDF 阅读）Windows 真机验收清单（D5 交付物 → #48 走查用）

> 来源：#51 派工单 D5「给 #48 提供 Windows 操作/截图清单」。验收人：具备
> Windows GUI 环境的一方（Lead / #48 认领人）。A 线窗口/布局/生命周期清单见
> 同目录 [README.md](README.md)——本文件只覆盖 **D 线 PDF 阅读/批注/选择面**，
> 两清单合走即 #48 完整验收。
>
> 口径（#48 铁律）：不得用 mock、非空位图或跳过测试替代窗口验收；每项记录
> 通过 / 失败 / 未测并附截图或日志；真实缺陷单独建可领取 issue。Linux 侧
> （云沙箱，真 PDFium + 真 LibreOffice）自动化背书见每项末列；标注
> 「Windows 待验」的项必须真机走查后才可在 #48 记通过。

## 一、验收对象与启动方式

| 环境 | 命令 | 判定 |
|---|---|---|
| Windows（真机） | `cargo run --manifest-path mang-shell/Cargo.toml -- <文件>` | 人工走查 + 截图 |
| Linux 无人值守 | `./smoke.sh 5 tests/fixtures/smoke.pdf` | stdout 末行 `smoke: quit reached` |
| Linux 证据口径 | `cargo test --manifest-path mang-shell/Cargo.toml --lib pdf_display_verify -- --nocapture --test-threads=1`（`MDC_PDFIUM_LIB=<libpdfium.dll 路径>`） | 全绿 + `#51` 证据行 |

语料（`tests/fixtures/display_corpus/`，15 文件——中文嵌入/缺失字体、扫描图、
透明/裁剪、CropBox 偏移/内缩、旋转 90/180、矢量、横竖混合、120 页长档、
2000×2800pt 超常页、综合特征、坏档 ×2）。环境要求：`MDC_PDFIUM_LIB` 指向
`pdfium.dll`（chromium/8086 同轨）。

## 二、打开与首屏（D1/D2 面）

| # | 检查项 | 操作 | 期望 | Linux 自动化背书 |
|---|---|---|---|---|
| D-1 | 正常档打开 | 逐个打开 15 夹具 | 12 正常档位图首屏（PDF 长得像 PDF）；2 坏档 typed error 面板（不白屏不崩） | `product_acceptance_corpus_view_sweep`（15 夹具全景） |
| D-2 | 旋转页 | `rotated_90.pdf` / `rotated_180.pdf` | 旋转方向正确（90° 竖排/180° 倒置）；内容与 poppler 参照一致 | `engine_content_matches_poppler_reference`（15×25 探针页矩阵 + 活参照 NCC/IoU） |
| D-3 | CropBox | `cropbox_offset.pdf` / `cropbox_inset.pdf` | 裁剪域正确（偏移/内缩不漂移）；letterbox 留白居中不拉伸 | 同上（几何 ±0.75pt 对照 pypdf） |
| D-4 | 缺字体 | `zh_missing_font.pdf` | 回退字体渲染（不缺页不空白块） | 扫描（D-1） |
| D-5 | 扫描图 | `scanned_page.pdf` | 位图原样呈现（无「OCR 不支持」遮挡） | 扫描（D-1） |
| D-6 | 超常页 | `oversized_page.pdf`（2000×2800pt） | 正常渲染不拒开 | 扫描（D-1）+ 健壮性测试 |

## 三、阅读导航与缩放（D2/#421 面）

| # | 检查项 | 操作 | 期望 | Linux 自动化背书 |
|---|---|---|---|---|
| D-7 | 滚动 | 滚轮逐页滚 120 页长档 | 平滑滚动；变高槽（横竖混排各页各高） | `view_chain_mixed_geometry_slots_and_zoom_anchor` |
| D-8 | 跳页 | 状态栏/书签跳第 60 页 | 顶部页读回 60；位图就绪 | `view_chain_first_screen_scroll_zoom_close_reopen` |
| D-9 | 缩略图侧栏 | 右 dock 缩略图 | 点击跳页；高亮随滚动跟随 | `pdf_thumbs` 既有面（#420） |
| D-10 | 缩放四态 | Ctrl+0 适应页面 / Ctrl+1 实际大小 / Ctrl+Shift+0 适应宽度 / Ctrl+=·Ctrl+- 步进 | 档位切换重渲不跳页（页空间锚点）；百分比 chip 正确 | `view_chain_first_screen_scroll_zoom_close_reopen`（576→640 换档） |
| D-11 | Ctrl+滚轮缩放 | Ctrl+滚轮 | 窗口级拦截（列表不随滚）+ 步进 10% | 编译面 + 键域同构（真机手感待验） |
| D-12 | 高 DPI | 150%/200% 显示缩放后 Ctrl+= | 密度随 sf 升档（不发虚）；交互坐标仍逻辑域 | `hidpi_device_width_scales_display_stays_logical`（单元）+ `render_lifecycle_fast_zoom_stale_cancel` |

## 四、页内查找（#422）

| # | 检查项 | 操作 | 期望 | Linux 自动化背书 |
|---|---|---|---|---|
| D-13 | Ctrl+F 开条 | 页面视图内 Ctrl+F | 不切文本视图直接开条；黄色高亮 + n/m 计数 + Enter/Shift+Enter 导航 | `pdf_view_ctrl_f_opens_in_page_search`（shell e2e） |
| D-14 | 旋转页高亮 | `rotated_90.pdf` 查找命中 | 高亮 quad 页变换映射不错位 | `pdf_view_search_full_flow`（视图级） |

## 五、文本选择与复制（D4a——本批新增）

| # | 检查项 | 操作 | 期望 | Linux 自动化背书 |
|---|---|---|---|---|
| D-15 | 拖选 | 左键拖选一段文字 | 选择蓝高亮实时跟随拖选（与查找黄并存） | `pdf_view_ctrl_c_copies_text_selection`（shell e2e，真实鼠标管线） |
| D-16 | 复制 | 选中后 Ctrl+C → 记事本 Ctrl+V | 逐字一致（含跨行换行）；无选择 Ctrl+C 不吞 | 同上（测试平台剪贴板逐字断言） |
| D-17 | Esc 清选择 | 选中后 Esc | 选择清零；再拖选即时高亮（层保留） | 同上 |

## 六、批注（#302/#386 + D4c）

| # | 检查项 | 操作 | 期望 | Linux 自动化背书 |
|---|---|---|---|---|
| D-18 | 右键拖框高亮 | 右键拖框（默认高亮工具） | 高亮落点 = 拖框所见；右键单击循环切工具（横幅提示） | `pdf_view_right_drag_annotate_rotated_and_cropbox`（旋转/CropBox 双夹具真鼠标链） |
| D-19 | 下划线/便签 | 单击切工具后再拖框 | `/Underline` / `/Text` 两手势同链 | `pdf_annotate_e2e`（10 测：三手势/往返/签名保持） |
| D-20 | 落盘与重开 | 批注后 Ctrl+S → 关标签重开 | 增量落盘（原字节前缀 + 签名保持）；重开批注可见 | 同 D-18（前缀 + `/Highlight` 物证 + 重开渲染） |
| D-21 | 旋转/CropBox 落点 | `rotated_90.pdf` / `cropbox_offset.pdf` 上拖框 | 用户空间四角准确（所见即所落） | 同 D-18 + `rotated_page_geometry_and_landing`（控制器数学面） |

## 七、状态一致性（D4b——本批新增）

| # | 检查项 | 操作 | 期望 | Linux 自动化背书 |
|---|---|---|---|---|
| D-22 | 页码活更新 | 滚动/跳页 | 状态栏「第 N / M 页」随顶部可见页实时更新（不是旧光标位） | `pdf_view_state_snapshot_and_page_label`（emit 订阅） |
| D-23 | 切标签回原位 | A 档（页 2 + ActualSize）→ 切 B → 切回 A | 页 2 + 实际大小档原样回放（快照） | 同上（四段全链） |
| D-24 | 关标签重开 | 关 A → 重新打开 A | 同回放（关标签路径快照） | 同上 |

## 八、异常（D5）

| # | 检查项 | 操作 | 期望 | Linux 自动化背书 |
|---|---|---|---|---|
| D-25 | 坏档 | 打开 `damaged_garbage.pdf` / `damaged_truncated.pdf` | typed error 面板明示；窗口继续可操作（切文本视图可读字节面） | `robustness_damaged_oversized_multi_tab` |
| D-26 | 缺库 | 移走 pdfium.dll 再开 pdf | 「pdfium 未绑定」面板明示不白屏 | 同上（缺库 SKIP 横幅机制） |
| D-27 | 多标签并发 | 双文档交替切换/快速关开 | 互不锁死；关 B 后 A 继续可操作 | 同上 |

## 九、截图清单（每项一张，文件名建议）

```
d01-open-mixed.png      d10-zoom-actual.png     d18-annotate-highlight.png
d02-rotated90.png       d11-ctrl-wheel.png      d19-underline.png
d03-cropbox.png         d12-hidpi-200.png       d20-reopen-annot.png
d04-missing-font.png    d13-find-bar.png        d21-cropbox-annot.png
d05-scanned.png         d14-rotated-find.png    d22-status-page.png
d06-oversized.png       d15-drag-select.png     d23-tab-back.png
d07-scroll-120p.png     d16-copy-paste.png      d25-damaged-panel.png
d08-jump-60.png         d17-esc-clear.png      d27-two-tabs.png
```

## 十、Linux 侧证据摘要（先行验证口径）

- 全景：`product_acceptance_corpus_view_sweep`（15 夹具：13 正常档
  open→首屏→跳页→缩放→预算全链 + 2 坏档 typed 态）；
- 显示矩阵：`pdfr_display_matrix` 7/7（lopdf/PDFium 双面，真库）；
- 视图链/生命周期/健壮性：`pdf_display_verify` 全组（634 lib 测试全绿）；
- 批注 e2e：`pdf_annotate_e2e` 10/10；
- shell 键域/鼠标管线 e2e：Ctrl+F / Ctrl+C / 快照回放 / 右键拖框链
  （`pdf_search_wiring_tests` 组）。
- 命令（派工单结束条件，Linux 已跑通的三条原文）：
  `cargo test -p mdc-pdf --features engine-lopdf,engine-pdfium --test pdfr_display_matrix -- --nocapture --test-threads=1`；
  `cargo test --manifest-path mang-shell/Cargo.toml --lib pdf_display_verify -- --nocapture --test-threads=1`；
  `cargo test --manifest-path mang-shell/Cargo.toml --test pdf_annotate_e2e -- --test-threads=1`。
