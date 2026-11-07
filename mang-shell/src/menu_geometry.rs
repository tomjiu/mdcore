//! 菜单几何（#63 Lead 打磨项 → #385 批 1 迁移后唯一消费者 = menu_bar）。
//!
//! # 迁移状态（#385 批 1，2026-11-08）
//!
//! 右键菜单（context_menu）已迁 `ui::ContextMenu`：摆放/边缘翻转/夹紧
//! 由 gpui `Anchored`（`AnchoredFitMode::SwitchAnchor` + 窗口边夹紧）
//! 承担，宽度由真实文本测量承担——**右键菜单路径不再消费本模块**。
//!
//! 本模块存续的唯一理由：顶部菜单栏（`menu_bar.rs` / `menu_bar_view.rs`，
//! #385 批 4 迁 `ui::DropdownMenu` 组合）仍是手写下拉，其锚定/翻转子菜单
//! 飞出仍用这套纯函数几何。**批 4 落地后本模块整体退役删除。**
//!
//! # 为什么几何是纯函数
//!
//! 菜单最容易出的 bug 不是「不显示」，而是**在屏幕右下角弹到屏幕外**——
//! 用户看不见但程序一切正常。摆放算法必须能脱离窗口单测、对着边界写
//! 断言才抓得住（见本文件测试 `place_menu_*` / `place_submenu_*` 一族）。

use crate::context_menu::MenuItem;

/// 屏幕/窗口坐标点（逻辑像素，f32——与 gpui 的 `Point<Pixels>` 同域，
/// 但本模块保持零 GPUI 依赖，menu_bar 模型层可在无窗口环境单测）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// 矩形（左上角 + 宽高）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
}

/// 单行高度（px）。
pub const ITEM_HEIGHT: f32 = 26.;
/// 分隔线占位（行高倍数）。
pub const SEPARATOR_UNITS: f32 = 0.4;
/// 菜单上下内边距（px）。
pub const MENU_PADDING_Y: f32 = 4.;
/// 菜单左右内边距（px）。
pub const MENU_PADDING_X: f32 = 4.;
/// 菜单最小宽度（px）——再窄标签会折行。
pub const MENU_MIN_WIDTH: f32 = 160.;
/// 宽度估算：每字符像素（中文按 1.6 倍算，见 [`estimate_width`]）。
const CHAR_W: f32 = 7.5;

/// 文本宽度粗估（不加载字体——菜单宽度只需要「够不够放」，不需要精确）。
///
/// CJK 字符按 1.6 倍宽度估：菜单标签大量是中文，按拉丁字符宽估会窄一半，
/// 渲染时标签被截断。两侧各留 `PADDING` 余量。
pub fn estimate_width(text: &str) -> f32 {
    text.chars()
        .map(|c| if c.is_ascii() { CHAR_W } else { CHAR_W * 1.6 })
        .sum()
}

/// 菜单几何：给定项集，算出菜单自身应有尺寸。
pub fn menu_size(items: &[MenuItem]) -> (f32, f32) {
    let mut w = MENU_MIN_WIDTH;
    let mut h = MENU_PADDING_Y * 2.;
    for item in items {
        h += ITEM_HEIGHT * item.height_units();
        let need = match item {
            MenuItem::Action {
                label, shortcut, ..
            } => {
                // 标签 + 间隔 + 快捷键 + 左右内边距
                let sc = shortcut.as_deref().unwrap_or("");
                estimate_width(label)
                    + if sc.is_empty() {
                        0.
                    } else {
                        32. + estimate_width(sc)
                    }
                    + MENU_PADDING_X * 2.
                    + 24.
            }
            MenuItem::Submenu { label, .. } => estimate_width(label) + 40. + MENU_PADDING_X * 2.,
            MenuItem::Separator => 0.,
        };
        w = w.max(need);
    }
    (w, h)
}

/// 把菜单摆到屏幕内：优先在点击点右下方向展开；越界则翻转/夹紧。
///
/// `anchor` = 点击点（屏幕坐标），`size` = 菜单尺寸，`screen` = 可用区域
/// （窗口客户区）。返回菜单左上角坐标。
///
/// 规则（顺序有讲究）：
/// 1. 默认左上角 = 点击点；
/// 2. 右边越界 → 左翻（菜单右边缘对齐点击点）；
/// 3. 下边越界 → 上翻；
/// 4. 翻转后仍越界（菜单比屏幕还大）→ 夹紧到 [0, screen - size]，
///    保证菜单**至少完整可见地贴在屏幕内**，不会一半在屏幕外。
///
/// （#385 批 1 注：右键菜单路径已由 gpui `Anchored` 的 SwitchAnchor +
/// 窗口边夹紧承担同一语义；本函数现仅服务 menu_bar 批 4 前的手写下拉。）
pub fn place_menu(anchor: Point, size: (f32, f32), screen: Rect) -> Point {
    let (w, h) = size;
    let mut x = anchor.x;
    let mut y = anchor.y;

    // 右越界 → 左翻
    if x + w > screen.right() {
        x = anchor.x - w;
    }
    // 下越界 → 上翻
    if y + h > screen.bottom() {
        y = anchor.y - h;
    }
    // 翻转后仍越界 → 夹紧
    x = x.clamp(screen.x, (screen.right() - w).max(screen.x));
    y = y.clamp(screen.y, (screen.bottom() - h).max(screen.y));
    Point::new(x, y)
}

/// 子菜单飞出方向：默认右飞；右侧放不下则左飞。
///
/// 返回子菜单左上角。`parent` = 父菜单矩形（已摆放好）。
pub fn place_submenu(parent: Rect, row_y: f32, size: (f32, f32), screen: Rect) -> Point {
    let (w, h) = size;
    // 与父项左/上边对齐，再向右飞一个菜单宽。
    let mut x = parent.right();
    let mut y = parent.y + row_y - MENU_PADDING_Y;
    if x + w > screen.right() {
        x = parent.x - w;
    }
    y = y.clamp(screen.y, (screen.bottom() - h).max(screen.y));
    x = x.clamp(screen.x, (screen.right() - w).max(screen.x));
    Point::new(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_menu::editor_menu;

    fn screen() -> Rect {
        Rect::new(0., 0., 1200., 800.)
    }

    #[test]
    fn menu_size_grows_with_item_count() {
        let a = menu_size(&editor_menu(false, false));
        let b = menu_size(&editor_menu(false, false)[..2]);
        assert!(a.1 > b.1, "项多则高，{} vs {}", a.1, b.1);
    }

    #[test]
    fn menu_width_fits_cjk_labels() {
        // 中文标签不能被按拉丁字符宽估成 160 最小宽就完事。
        let items = vec![MenuItem::action("x", "在资源管理器中显示").with_shortcut("Ctrl+Shift+E")];
        let (w, _) = menu_size(&items);
        assert!(w > MENU_MIN_WIDTH + 40., "CJK 长标签应把菜单撑宽，实际 {w}");
    }

    #[test]
    fn place_menu_default_down_right() {
        let p = place_menu(Point::new(100., 100.), (200., 300.), screen());
        assert_eq!(p, Point::new(100., 100.), "有空间时就在点击点右下");
    }

    #[test]
    fn place_menu_flips_left_at_right_edge() {
        let p = place_menu(Point::new(1150., 100.), (200., 300.), screen());
        assert!(p.x + 200. <= 1200., "右越界必须左翻，实际 x={}", p.x);
        assert!(p.x >= 0.);
    }

    #[test]
    fn place_menu_flips_up_at_bottom_edge() {
        let p = place_menu(Point::new(100., 750.), (200., 300.), screen());
        assert!(p.y + 300. <= 800., "下越界必须上翻，实际 y={}", p.y);
    }

    #[test]
    fn place_menu_clamps_when_larger_than_screen() {
        // 菜单比屏幕还大：翻不动，只能夹紧到左上角。
        let p = place_menu(Point::new(600., 400.), (2000., 1000.), screen());
        assert_eq!(p, Point::new(0., 0.));
    }

    #[test]
    fn submenu_flyout_right_by_default() {
        let parent = Rect::new(100., 100., 200., 300.);
        let p = place_submenu(parent, 26., (180., 120.), screen());
        assert!((p.x - 300.).abs() < 0.01, "子菜单默认贴父菜单右侧飞出");
    }

    #[test]
    fn submenu_flips_left_when_no_room() {
        let parent = Rect::new(1100., 100., 90., 300.);
        let p = place_submenu(parent, 26., (180., 120.), screen());
        assert!(p.x < parent.x, "右侧放不下时左翻，实际 x={}", p.x);
    }

    /// 原 context_menu::ContextMenu::layout 的暴力扫描覆盖（#385 批 1
    /// 迁移后 struct 已删——断言面平移到 place_menu + menu_size 组合，
    /// 覆盖语义不变：任何展开点算出的菜单矩形都必须在屏幕内）。
    #[test]
    fn place_menu_never_leaves_screen() {
        let items = editor_menu(false, false);
        let (w, h) = menu_size(&items);
        for x in (0..1200).step_by(37) {
            for y in (0..800).step_by(41) {
                let p = place_menu(Point::new(x as f32, y as f32), (w, h), screen());
                assert!(p.x >= -0.01, "x 越界 {p:?}（宽 {w}）");
                assert!(p.y >= -0.01, "y 越界 {p:?}（高 {h}）");
                assert!(p.x + w <= 1200.01, "右边越界 {p:?}（宽 {w}）");
                assert!(p.y + h <= 800.01, "下边越界 {p:?}（高 {h}）");
            }
        }
    }
}
