//! #525：Zed 按钮组件 36×36 硬规格适配层——方案 A 成立的实证交付。
//!
//! # 决策记录（2026-11-08，无头面证据链 + 源码实证）
//!
//! issue 的核心质疑成立：**「IconButton 没有 36×36 参数」≠「无法实现
//! 36×36」**。三条候选路径的实测/源码结论：
//!
//! | 路径 | 结论 | 证据 |
//! |---|---|---|
//! | ① 外层固定 36×36 容器 + IconButton 居中 | **不成立** | IconButton 自身 `.flex_none()`（button_like.rs render），外容器只改摆放不改按钮盒；命中区域 = 内按钮盒（默认约 24×22），边缘约 6px 环形死区点击不落——见 [`tests::fit_probes`] P1 |
//! | ② Square 形状 + `IconSize::Custom(rems_from_px(12.))` | **数值巧合，不采纳** | `square() = icon + 2×padding`，Custom 的 padding == icon 尺寸（上游 TODO 行为）→ 12+24=36 恰好命中；但图标退化 12px（现规格 16px）且依赖 quirk——上游修正即静默破坏。探针锁行为见 P5 |
//! | ③ IconButton + 渲染拦截 + 高度钉定（**本适配层**） | **成立** | IconButton 暴露 `.width()`（`FixedWidth` trait）但无 `.height()`；其 `render()` 是公开 trait 方法且签名 `-> ButtonLike`（icon_button.rs，公开具体类型）——拦截返回值后补 `.height(px(36.))`（ButtonLike 公开方法）。按钮盒精确 36×36 + 命中区域覆盖全盒（P3），IconButton 全语义保留（hover/active 精确色注入、tooltip、aria、toggle、disabled） |
//!
//! 备选：`ButtonLike` 直构（`.width(px(36.)) + .height(px(36.))` 双公开
//! 方法，零拦截）同样精确 36×36（P2 实证），但 ButtonLike 未暴露
//! `hover_background`/`active_background`/icon 色注入面——现 bar_button
//! 需要精确 hover 色注入，故主路径取 ③。
//!
//! # 已知边界（诚实登记）
//!
//! - **选中底色精确注入不可达**：ButtonStyle 无 Custom 变体（Tinted 仅
//!   Accent/Error/Warning/Success），`element_selected` 字面色无法经
//!   样式体系注入。zed 官方选中模式 = `selected_style(Tinted(Accent))`
//!   （ToggleButton 同款）。后续替换批次按 Lead 裁定取舍。
//! - **hover 色槽半档差（仅浅色主题）**：Subtle 的 hover 走
//!   `ghost_element_hover`，现自绘走 `element_hover`——JustBase 深色基线
//!   两槽同值（fallback dark：均 `dark_alpha().step_4()`，P7 锁证）；
//!   浅色派生主题只覆盖 `element_*` 17 槽（theme_switch.rs），
//!   `ghost_*` 未覆盖——浅色下有半档差，登记待浅色主题补覆盖时收口。
//! - **待真机项（AGENTS 诚实边界纪律）**：Windows 截图对比、键盘焦点
//!   视觉、tooltip 悬停行为——无头面不可测，未验证前不宣布迁移成功。
//! - **a11y 净增**：现 bar_button 零 a11y（activity_bar.rs 模块文档自认
//!   「记为后续 PR」）；适配层经 ButtonLike 机制自带 role=Button +
//!   aria_label（缺省回落 tooltip 文本）+ tab_index/焦点环。
//!
//! # 使用面
//!
//! 本模块只交付适配层与验证测试（#525 约束「只负责验证和决策，不直接
//! 扩大成整个按钮系统重写」）——bar_button 等自绘单元的**批量替换**是
//! 后续独立批次（对照表行 2/3/4 的解冻件），不在本 PR。
//!
//! 进 lib 树的理由：验证测试（`#[gpui::test]` 无头面）挂 bin 目标 =
//! CI 永不运行（假绿）；CI mang-shell job 只跑 `--lib`/`--tests`——
//! 与 document.rs 同纪律。

use gpui::{App, ClickEvent, Hsla, Pixels, RenderOnce, SharedString, Window, px};
use ui::prelude::*;
use ui::{ButtonLike, Tooltip};

/// 图标按钮边长（px，#63 Lead 硬规格 36×36）。
///
/// 与 `activity_bar::BUTTON_SIZE` 同值但独立声明：activity_bar 是 bin 树
/// 模块，本层进 lib 树（见模块文档），不反向依赖 bin。
pub const ICON_BUTTON_SIDE: Pixels = px(36.);

/// 36×36 图标按钮构造器（#525 方案 A 适配层——主路径 ③）。
///
/// 组装式样（[`IconButton36::build`]）：
///
/// ```text
/// IconButton（全语义：Subtle 底 + 16px 图标 + toggle/aria/tooltip/
///            hover/active 精确色 + on_click）
///   .width(px(36.))                 ← IconButton 既有 FixedWidth
///   .render(window, cx)             ← 公开 trait 方法，返回 ButtonLike
///   .height(px(36.))                ← ButtonLike 公开方法，高度钉定
/// ```
///
/// 返回 [`ButtonLike`] 而非 `AnyElement`：调用方可继续链式精调
/// （ButtonLike 公开面全保留——如再 `.disabled()`、`.tab_index()`）。
pub struct IconButton36 {
    id: &'static str,
    icon: IconName,
    selected: bool,
    tooltip: Option<SharedString>,
    hover_background: Option<Hsla>,
    active_background: Option<Hsla>,
    selected_style: Option<ButtonStyle>,
    disabled: bool,
    aria_label: Option<SharedString>,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
}

/// 构造 36×36 图标按钮（见 [`IconButton36`]）。
pub fn icon_button_36(id: &'static str, icon: IconName) -> IconButton36 {
    IconButton36 {
        id,
        icon,
        selected: false,
        tooltip: None,
        hover_background: None,
        active_background: None,
        selected_style: None,
        disabled: false,
        aria_label: None,
        on_click: None,
    }
}

impl IconButton36 {
    /// 选中态（toggle_state）：图标切 `Color::Selected`；底色见
    /// [`Self::selected_style`]（缺省维持 Subtle 底——精确色注入边界
    /// 见模块文档「已知边界」）。
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// tooltip 文本（`ui::Tooltip::text`——与批 6 title_bar 窗口控制钮
    /// 同语系）。缺省时 aria_label 亦回落到该文本（icon-only 钮的
    /// 读屏基线）。
    pub fn tooltip_text(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    /// hover 底色精确注入（IconButton 既有 builder——经渲染拦截保留）。
    /// 现自绘 bar_button 的 `element_hover` 注入模式在此续用。
    pub fn hover_background(mut self, background: Hsla) -> Self {
        self.hover_background = Some(background);
        self
    }

    /// 按下（active）底色精确注入（同上）。
    pub fn active_background(mut self, background: Hsla) -> Self {
        self.active_background = Some(background);
        self
    }

    /// 选中底色的样式交换（zed 官方选中模式：
    /// `selected_style(ButtonStyle::Tinted(TintColor::Accent))`——
    /// ToggleButton 同款）。
    pub fn selected_style(mut self, style: ButtonStyle) -> Self {
        self.selected_style = Some(style);
        self
    }

    /// 禁用态（ButtonLike 机制：on_click 不注册 + 光标 not-allowed +
    /// 图标 `Color::Disabled`）。
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 读屏标签（缺省回落 tooltip 文本）。
    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = Some(label.into());
        self
    }

    /// 点击回调（命中区域 = 完整 36×36 按钮盒，P3 实证）。
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// 组装为 [`ButtonLike`]（消耗 self——链式收口）。
    ///
    /// 需要 `window`/`cx`：渲染拦截路径的 `render()` 签名所需
    /// （IconButton 的 Square 尺寸推导要读 window 的 rem_size——本路径
    /// Wide 形状实际不消费，但签名不可省）。
    pub fn build(self, window: &mut Window, cx: &mut App) -> ButtonLike {
        // aria 缺省回落 tooltip（icon-only 钮读屏基线——IconButton
        // 文档明示「should always set this」）。
        let aria_label = self.aria_label.or_else(|| self.tooltip.clone());

        let mut button = IconButton::new(self.id, self.icon)
            // Subtle 底：ghost_element_background（近透明）——现自绘
            // 「无底色」的最近似式样；hover 走注入色（见下）。
            .style(ButtonStyle::Subtle)
            // 16px 图标——现 bar_button 规格（IconSize::Medium）。
            .icon_size(IconSize::Medium)
            .toggle_state(self.selected)
            .selected_icon_color(Color::Selected)
            .when_some(self.tooltip, |this, text| this.tooltip(Tooltip::text(text)))
            .when_some(aria_label, |this, label| this.aria_label(label))
            .when_some(self.hover_background, |this, bg| this.hover_background(bg))
            .when_some(self.active_background, |this, bg| {
                this.active_background(bg)
            })
            .when_some(self.selected_style, |this, style| {
                this.selected_style(style)
            })
            .when(self.disabled, |this| this.disabled(true))
            .when_some(self.on_click, |this, handler| this.on_click(handler));

        // 路径 ③ 两步钉定：宽度走 IconButton 既有 FixedWidth；高度经
        // 渲染拦截（RenderOnce::render 公开方法 → ButtonLike 公开类型）
        // 在返回值上补 ButtonLike::height——IconButton 自身无高度 API
        // （ButtonSize enum 最大 Large=32px 且 rem 随 UI 缩放浮动，
        // 无法钉 36px 字面规格）。
        button = button.width(ICON_BUTTON_SIDE);
        button.render(window, cx).height(ICON_BUTTON_SIDE.into())
    }
}

// ---------------------------------------------------------------------------
// #525 验证测试（最小验证代码——不批量替换现有按钮）。
//
// 无头面可测三件：布局尺寸（debug_selector → debug_bounds 实测）、
// 命中区域（simulate_click → 计数器断言——「点击 → 状态变化」级，
// 对照表 §六 质量门 1）、主题色槽等值。真机项（截图/键盘焦点视觉/
// tooltip 悬停）按 AGENTS 诚实边界纪律登记待真机。
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Bounds, Modifiers, Point, TestAppContext, VisualTestContext, px, size};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 探针计数器组（Arc 共享进闭包；断言读计数——状态变化级断言）。
    #[derive(Clone, Default)]
    struct Counters {
        p1_outer_container: Arc<AtomicUsize>,
        p2_button_like_direct: Arc<AtomicUsize>,
        p3_adapter: Arc<AtomicUsize>,
        p4_disabled: Arc<AtomicUsize>,
    }

    /// 探针宿主（Host 模式——同 shell.rs menu_bar_lifecycle_tests：
    /// 独立小宿主开真窗口，不牵 ShellApp）。
    struct ProbeHost {
        counters: Counters,
    }

    fn click_logger(
        counter: &Arc<AtomicUsize>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        let counter = counter.clone();
        move |_, _, _| {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// bounds 内偏移点（窗口坐标系——simulate_click 的入参坐标系）。
    fn at(bounds: &Bounds<Pixels>, dx: f32, dy: f32) -> Point<Pixels> {
        Point::new(bounds.origin.x + px(dx), bounds.origin.y + px(dy))
    }

    fn click(visual: &mut VisualTestContext, position: Point<Pixels>) {
        visual.simulate_click(position, Modifiers::default());
    }

    impl gpui::Render for ProbeHost {
        fn render(
            &mut self,
            window: &mut Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            let counters = self.counters.clone();
            // items_start：防 v_flex 默认 cross 轴 stretch 拉宽自适应宽
            // 的 P2 shrink-wrap 探针（测量前提）。
            v_flex()
                .id("probe-root")
                .size_full()
                .items_start()
                .gap_2()
                .p_1()
                // P1【issue 优先验证 1】外层固定 36×36 容器 + 默认
                // IconButton 居中——外容器不放大内按钮（flex_none），
                // 命中区域 = 内按钮盒。
                .child(
                    div()
                        .debug_selector(|| "P1-SLOT".to_string())
                        .size(ICON_BUTTON_SIDE)
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .child(
                            IconButton::new("p1-inner", IconName::FileTree)
                                .on_click(click_logger(&counters.p1_outer_container)),
                        ),
                )
                // P2【备选路径】ButtonLike 直构（width+height 双公开方法
                // 零拦截）——shrink-wrap 宿主测精确盒 + 角点击。
                .child(
                    div()
                        .debug_selector(|| "P2-WRAP".to_string())
                        .flex_none()
                        .child(
                            ButtonLike::new("p2-direct")
                                .width(ICON_BUTTON_SIDE)
                                .height(ICON_BUTTON_SIDE.into())
                                .style(ButtonStyle::Subtle)
                                .child(Icon::new(IconName::Close))
                                .on_click(click_logger(&counters.p2_button_like_direct)),
                        ),
                )
                // P3【主路径】适配层：40×40 宿主 flex 居中——按钮盒若为
                // 精确 36×36 则占据 (2,2)-(38,38)，角内/盒外各 1px 的
                // 点击判据完全确定。
                .child(
                    div()
                        .debug_selector(|| "P3-SLOT".to_string())
                        .size(px(40.))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .child(
                            icon_button_36("p3-adapted", IconName::MagnifyingGlass)
                                .tooltip_text("P3 适配层 36×36")
                                .hover_background(gpui::black())
                                .on_click(click_logger(&counters.p3_adapter))
                                .build(window, cx),
                        ),
                )
                // P4【禁用门】disabled → 点击不落（ButtonLike render 的
                // on_click.filter(!disabled) 行为证据）。
                .child(
                    icon_button_36("p4-disabled", IconName::Settings)
                        .disabled(true)
                        .tooltip_text("P4 禁用态")
                        .on_click(click_logger(&counters.p4_disabled))
                        .build(window, cx),
                )
                // P5【路径 ② 探针】Square + Custom(rems_from_px(12)) 的
                // 数值巧合（12 + 2×12 = 36）——锁上游 quirk：若上游修正
                // Custom 的 padding 语义，此断言红 = 适配前提变化的哨兵。
                .child(
                    IconButton::new("p5-quirk", IconName::Maximize)
                        .shape(ui::IconButtonShape::Square)
                        .icon_size(IconSize::Custom(rems_from_px(12_f32))),
                )
                // P6【enum 上限证据】ButtonSize::Large = 32px——IconButton
                // 尺寸 API 的最大档 < 36（rem 机器推导不可达字面规格）。
                .child(IconButton::new("p6-large", IconName::GitBranch).size(ButtonSize::Large))
        }
    }

    /// #525 验证主测：三条路径的布局/命中区域/禁用门 + enum 上限。
    #[gpui::test]
    fn fit_probes(cx: &mut TestAppContext) {
        cx.update(|app| {
            theme::init(theme::LoadThemes::JustBase, app);
            // 主题设置提供器（main.rs 同款序列）：ButtonLike 渲染的
            // DynamicSpacing（间距是设置驱动——Base04/Base08）首访即需
            // GlobalThemeSettingsProvider，缺则 panic。
            theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
        });
        let counters = Counters::default();
        let host = ProbeHost {
            counters: counters.clone(),
        };
        let window = cx.open_window(size(px(120.), px(340.)), |_, _| host);
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));

        // ---- P1 外层容器适配（issue 优先验证 1）：不成立 ----------------
        let p1_slot = visual
            .debug_bounds("P1-SLOT")
            .expect("P1 外容器 selector 在场");
        assert_eq!(
            (p1_slot.size.width, p1_slot.size.height),
            (px(36.), px(36.)),
            "P1 外容器确为 36×36"
        );
        // IconButton 自带 debug selector（ICON-{icon:?}，icon_button.rs
        // 构造器注入）——内按钮盒直接可测。
        let p1_inner = visual
            .debug_bounds("ICON-FileTree")
            .expect("IconButton 自动 selector 在场");
        assert!(
            p1_inner.size.height < px(36.) || p1_inner.size.width < px(36.),
            "P1 内按钮盒 {:?} 未填满 36×36——外容器只改摆放不改按钮盒（flex_none）",
            p1_inner.size
        );
        // 命中区域实证：外角（外容器内、内按钮外）点击不落。
        click(&mut visual, at(&p1_slot, 1., 1.));
        assert_eq!(
            counters.p1_outer_container.load(Ordering::SeqCst),
            0,
            "P1 外角点击不触发——环形死区（issue 验收 3 不满足，路径 ① 不成立）"
        );
        // 对照：内按钮中心点击正常落。
        let inner_cx = f32::from(p1_inner.size.width) / 2.;
        let inner_cy = f32::from(p1_inner.size.height) / 2.;
        click(&mut visual, at(&p1_inner, inner_cx, inner_cy));
        assert_eq!(
            counters.p1_outer_container.load(Ordering::SeqCst),
            1,
            "P1 内按钮中心点击触发（对照面——按钮本身功能正常）"
        );

        // ---- P2 ButtonLike 直构（备选路径）：精确 36×36 -----------------
        let p2_wrap = visual
            .debug_bounds("P2-WRAP")
            .expect("P2 shrink-wrap 宿主 selector 在场");
        assert_eq!(
            (p2_wrap.size.width, p2_wrap.size.height),
            (px(36.), px(36.)),
            "P2 shrink-wrap 实测 = 36×36——ButtonLike 直构精确钉定成立"
        );
        click(&mut visual, at(&p2_wrap, 1., 1.));
        assert_eq!(
            counters.p2_button_like_direct.load(Ordering::SeqCst),
            1,
            "P2 角点击触发——命中区域覆盖全盒"
        );

        // ---- P3 适配层（主路径）：精确 36×36 + 全盒命中 ------------------
        let p3_slot = visual
            .debug_bounds("P3-SLOT")
            .expect("P3 宿主 selector 在场");
        assert_eq!(
            (p3_slot.size.width, p3_slot.size.height),
            (px(40.), px(40.)),
            "P3 宿主确为 40×40（居中判据前提）"
        );
        let p3_btn = visual
            .debug_bounds("ICON-MagnifyingGlass")
            .expect("P3 适配层按钮自动 selector 在场");
        assert_eq!(
            (p3_btn.size.width, p3_btn.size.height),
            (px(36.), px(36.)),
            "P3 适配层按钮盒精确 36×36（width 预设 + height 渲染拦截补钉）"
        );
        // 居中于 40×40：起点 = 宿主原点 + (2,2)。
        assert_eq!(
            p3_btn.origin,
            Point::new(p3_slot.origin.x + px(2.), p3_slot.origin.y + px(2.)),
            "P3 按钮盒在 40×40 宿主正中（(40-36)/2=2）"
        );
        // 盒角内 1px（= 宿主 (3,3)）：触发——36×36 全盒命中。
        click(&mut visual, at(&p3_slot, 3., 3.));
        assert_eq!(
            counters.p3_adapter.load(Ordering::SeqCst),
            1,
            "P3 角点击触发——命中区域覆盖完整 36×36（issue 验收 3 满足）"
        );
        // 盒外 1px（= 宿主 (39,39)，仍在宿主内）：不触发——精确边界。
        click(&mut visual, at(&p3_slot, 39., 39.));
        assert_eq!(
            counters.p3_adapter.load(Ordering::SeqCst),
            1,
            "P3 盒外 1px 点击不触发——按钮盒边界精确 36×36"
        );

        // ---- P4 禁用门：disabled → 点击不落 ------------------------------
        let p4_btn = visual
            .debug_bounds("ICON-Settings")
            .expect("P4 禁用钮自动 selector 在场");
        click(&mut visual, at(&p4_btn, 18., 18.));
        assert_eq!(
            counters.p4_disabled.load(Ordering::SeqCst),
            0,
            "P4 禁用态点击不触发（issue 验收 4：Disabled 状态行为）"
        );

        // ---- P5 Square+Custom 数值巧合（路径 ②）：锁 quirk --------------
        let p5_btn = visual
            .debug_bounds("ICON-Maximize")
            .expect("P5 quirk 探针自动 selector 在场");
        assert_eq!(
            (p5_btn.size.width, p5_btn.size.height),
            (px(36.), px(36.)),
            "P5 Square+Custom(12px) 恰为 36×36（12+2×12 上游 quirk）——\
             不采纳：图标退化 12px + 依赖 TODO 行为（哨兵断言）"
        );

        // ---- P6 ButtonSize enum 上限（路径 ② 的前半证据）-----------------
        let p6_btn = visual
            .debug_bounds("ICON-GitBranch")
            .expect("P6 enum 探针自动 selector 在场");
        assert_eq!(
            p6_btn.size.height,
            px(32.),
            "P6 ButtonSize::Large = 32px——enum 最大档 < 36（IconButton \
             尺寸 API 不可达字面规格的直接证据）"
        );
    }

    /// #525 色槽等值（P7）：JustBase 深色基线下 element_hover 与
    /// ghost_element_hover 同值——Subtle 式样的 hover 与旧自绘
    /// element_hover 视觉等价（fallback dark：均 dark_alpha().step_4()）。
    /// 选中槽同理（step_5 同值）。浅色主题的 ghost_* 未覆盖属已知边界
    /// （模块文档「已知边界」第二节）。
    #[gpui::test]
    fn theme_slot_equivalence_justbase(cx: &mut TestAppContext) {
        cx.update(|app| theme::init(theme::LoadThemes::JustBase, app));
        cx.update(|app| {
            let colors = app.theme().colors();
            assert_eq!(
                colors.element_hover, colors.ghost_element_hover,
                "JustBase：hover 两槽同值（Subtle hover ≡ 旧自绘 hover）"
            );
            assert_eq!(
                colors.element_selected, colors.ghost_element_selected,
                "JustBase：选中两槽同值"
            );
        });
    }
}
