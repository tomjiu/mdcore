//! 外壳主题设置 —— `ThemeSettingsProvider` 实现 + 配置落盘（F2 起）。
//!
//! `ui` 组件（`font_ui` / `text_ui_size` 等）会经 `theme::theme_settings(cx)`
//! 读取全局 provider，未注册时 panic（theme crate 契约）——本类型在启动时
//! 经 `theme::set_theme_settings_provider` 落位。
//!
//! 字体走 fontconfig 系统面：`Noto Sans SC`（本仓目标环境常备，含拉丁与
//! CJK 字形，Regular/Bold 双字重）。不内嵌字体文件——系统缺字时升级为
//! bundled 字体是后续 PR 的事。
//!
//! ## 配置落盘（F2：工作区记忆；F5 最近列表同宿主扩展）
//!
//! - 位置：`$XDG_CONFIG_HOME/mang/settings.json`（缺省 `$HOME/.config/mang`
//!   ——Linux 口径；Windows/macOS 平台目录规范随 F5 落盘件统一）；
//! - 格式：serde_json 对象（键渐进扩展——F2 仅 `workspace`）；
//! - 容错：目录不可建/不可写一律静默跳过（记忆是增强不是依赖——
//!   写失败不阻断打开工作区；读失败返回 None 走无工作区启动）。

use std::path::{Path, PathBuf};

use gpui::{App, Font, Pixels, px};
use theme::{ThemeSettingsProvider, UiDensity};

/// 配置目录（XDG 口径；不可得 → None——调用方静默跳过落盘/恢复）。
pub fn config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("mang"))
}

fn settings_path_in(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

fn load_from(dir: &Path) -> serde_json::Value {
    std::fs::read_to_string(settings_path_in(dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

fn save_to(dir: &Path, value: &serde_json::Value) {
    let _ = std::fs::create_dir_all(dir);
    if let Ok(text) = serde_json::to_string_pretty(value) {
        let _ = std::fs::write(settings_path_in(dir), text);
    }
}

/// G3：缩进单位读写（`indent_unit` 键："2" / "4" / "tab"——消费方
/// [`crate::editor::buffer::IndentUnit::from_setting`] 解析，未知值
/// 归 4 空格）。
pub fn save_indent_unit(key: &str) {
    if let Some(dir) = config_dir() {
        let mut value = load_from(&dir);
        value["indent_unit"] = serde_json::Value::String(key.to_string());
        save_to(&dir, &value);
    }
}

/// #111 批②：括号自动闭合读写（`auto_close_brackets` 键：bool——
/// 默认开；沿 indent_unit 载体先例）。
pub fn save_auto_close_brackets(on: bool) {
    if let Some(dir) = config_dir() {
        let mut value = load_from(&dir);
        value["auto_close_brackets"] = serde_json::Value::Bool(on);
        save_to(&dir, &value);
    }
}

pub fn load_auto_close_brackets() -> Option<bool> {
    let dir = config_dir()?;
    load_from(&dir).get("auto_close_brackets")?.as_bool()
}

/// #111 P2：软换行开关读写（`soft_wrap` 键：bool——默认开；沿
/// auto_close_brackets 载体先例。软换行实现早已有（display_map
/// wraps + 渲染回填），本批补的是用户可关面——Q-IDE
/// settings/window.rs:302-309 对照；报告 §3 P2 末项）。
pub fn save_soft_wrap(on: bool) {
    if let Some(dir) = config_dir() {
        let mut value = load_from(&dir);
        value["soft_wrap"] = serde_json::Value::Bool(on);
        save_to(&dir, &value);
    }
}

pub fn load_soft_wrap() -> Option<bool> {
    let dir = config_dir()?;
    load_from(&dir).get("soft_wrap")?.as_bool()
}

/// #110 G2：用户主题目录（对齐既有用户配置目录——报告 §5 D2 裁定：
/// 不做 exe 目录回退；`$XDG_CONFIG_HOME/mang/themes/*.json`）。
pub fn themes_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("themes"))
}

/// #110 G2：装载用户主题（磁盘 → ThemeRegistry——报告 §3 G2「引擎
/// 在线但无装载函数」的补口）。返回装载成功数。
///
/// 策略（§5 D1 A 案装载器版）：用户 JSON = 少量 UI 色覆盖派生——
/// 以 registry 内 base 主题（zed_default_dark）为底、用户色覆盖
/// `background/foreground/…` 少数高频槽；**不重造 31 槽全链**（G1
/// 大批的事——本批先通水路：磁盘文件 → registry.list_names 可见）。
/// 坏文件容错：解析失败跳过 + stderr 证据行（不炸启动）。
pub fn load_user_themes(cx: &mut gpui::App) -> usize {
    let Some(dir) = themes_dir() else {
        return 0;
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    let registry = theme::ThemeRegistry::default_global(cx);
    // #110 G1 批A：双基底——深色沿 One Dark 兜底；浅色 = derive_light
    // （theme_switch 派生链——浅色槽消费面 #560 锁证的那套）。
    let base_dark = registry.get("One Dark").unwrap_or_else(|_| {
        registry
            .list()
            .into_iter()
            .next()
            .map(|m| registry.get(&m.name).unwrap())
            .unwrap()
    });
    let base_light = crate::theme_switch::derive_light(&base_dark);
    let mut loaded = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            eprintln!("theme-loader: 读失败跳过 {}", path.display());
            continue;
        };
        match parse_user_theme(&text, &base_dark, Some(&base_light)) {
            Some(theme) => {
                registry.insert_themes([theme]);
                loaded += 1;
            }
            None => {
                eprintln!(
                    "theme-loader: 解析失败跳过 {}（需含 name 与至少一个色槽）",
                    path.display()
                );
            }
        }
    }
    loaded
}

/// 用户主题 JSON → Theme（覆盖派生）。识别槽（报告 §5 D1 A 案
/// 「11 槽 UI 色」起步集）：background / surface / editor /
/// foreground / accent / border / 元素前景五色。
fn parse_user_theme(
    text: &str,
    base_dark: &theme::Theme,
    base_light: Option<&theme::Theme>,
) -> Option<theme::Theme> {
    #[derive(serde::Deserialize)]
    struct UserThemeJson {
        name: String,
        appearance: Option<String>,
        // #110 G1 批A：31 槽全量（6 组——border/surface/element/ghost/text/icon+editor）。
        // 键沿用 G2 六键（向下兼容旧 themes/*.json），新键按 ThemeColors 字段直名。
        background: Option<String>,
        surface: Option<String>,
        editor: Option<String>,
        foreground: Option<String>,
        accent: Option<String>,
        border: Option<String>,
        border_variant: Option<String>,
        border_focused: Option<String>,
        border_selected: Option<String>,
        border_transparent: Option<String>,
        border_disabled: Option<String>,
        element_background: Option<String>,
        element_hover: Option<String>,
        element_active: Option<String>,
        element_selected: Option<String>,
        element_selection_background: Option<String>,
        element_disabled: Option<String>,
        ghost_element_background: Option<String>,
        ghost_element_hover: Option<String>,
        ghost_element_active: Option<String>,
        ghost_element_selected: Option<String>,
        ghost_element_disabled: Option<String>,
        surface_background: Option<String>,
        text_muted: Option<String>,
        text_placeholder: Option<String>,
        text_disabled: Option<String>,
        text_accent: Option<String>,
        icon: Option<String>,
        icon_muted: Option<String>,
        icon_disabled: Option<String>,
        #[allow(dead_code)]
        #[serde(default)]
        author: Option<String>,
    }
    let spec: UserThemeJson = serde_json::from_str(text).ok()?;
    // #110 G1 批A：双主题基底——appearance=light 且浅色基底可用时用
    // 派生浅色，否则深色基底（dark/缺省/浅色不可用三态归一）。
    let use_light = spec
        .appearance
        .as_deref()
        .map(|a| a.eq_ignore_ascii_case("light"))
        .unwrap_or(false)
        && base_light.is_some();
    let base = if use_light {
        // borrow 双态——clone 归一（Theme 不大，装载路径一次性）。
        base_light.unwrap().clone()
    } else {
        base_dark.clone()
    };
    let mut theme = base.clone();
    theme.id = format!("user-{}", spec.name);
    theme.name = spec.name.into();
    if let Some(app) = &spec.appearance {
        theme.appearance = if app.eq_ignore_ascii_case("light") {
            theme::Appearance::Light
        } else {
            theme::Appearance::Dark
        };
    }
    let mut styles = theme.styles.clone();
    // #110 G1 批A：31 槽分组映射。G2 六键保留级联派生（老 JSON 兼容）；
    // 直名槽（border_variant 等）只写自身。appearance=light 时基底已在上
    // 游换成派生浅色（load_user_themes 的 base 选择），覆盖槽盖在其上。
    let slot = |v: &Option<String>| parse_hsla(v.as_deref().unwrap_or(""));
    // 组1：基底三面 + 文本主色（G2 键——级联）。
    if let Some(c) = slot(&spec.background) {
        styles.colors.background = c;
    }
    if let Some(c) = slot(&spec.editor) {
        styles.colors.editor_background = c;
    }
    if let Some(c) = slot(&spec.surface) {
        styles.colors.elevated_surface_background = c;
        styles.colors.surface_background = c;
    }
    if let Some(c) = slot(&spec.foreground) {
        styles.colors.text = c;
        styles.colors.text_muted = c;
    }
    if let Some(c) = slot(&spec.accent) {
        styles.colors.element_active = c;
        styles.colors.text_accent = c;
    }
    if let Some(c) = slot(&spec.border) {
        styles.colors.border = c;
        styles.colors.border_variant = c;
    }
    // 组2：border 族直名槽。
    if let Some(c) = slot(&spec.border_variant) {
        styles.colors.border_variant = c;
    }
    if let Some(c) = slot(&spec.border_focused) {
        styles.colors.border_focused = c;
    }
    if let Some(c) = slot(&spec.border_selected) {
        styles.colors.border_selected = c;
    }
    if let Some(c) = slot(&spec.border_transparent) {
        styles.colors.border_transparent = c;
    }
    if let Some(c) = slot(&spec.border_disabled) {
        styles.colors.border_disabled = c;
    }
    // 组3：element 族直名槽。
    if let Some(c) = slot(&spec.element_background) {
        styles.colors.element_background = c;
    }
    if let Some(c) = slot(&spec.element_hover) {
        styles.colors.element_hover = c;
    }
    if let Some(c) = slot(&spec.element_active) {
        styles.colors.element_active = c;
    }
    if let Some(c) = slot(&spec.element_selected) {
        styles.colors.element_selected = c;
    }
    if let Some(c) = slot(&spec.element_selection_background) {
        styles.colors.element_selection_background = c;
    }
    if let Some(c) = slot(&spec.element_disabled) {
        styles.colors.element_disabled = c;
    }
    // 组4：ghost 族直名槽。
    if let Some(c) = slot(&spec.ghost_element_background) {
        styles.colors.ghost_element_background = c;
    }
    if let Some(c) = slot(&spec.ghost_element_hover) {
        styles.colors.ghost_element_hover = c;
    }
    if let Some(c) = slot(&spec.ghost_element_active) {
        styles.colors.ghost_element_active = c;
    }
    if let Some(c) = slot(&spec.ghost_element_selected) {
        styles.colors.ghost_element_selected = c;
    }
    if let Some(c) = slot(&spec.ghost_element_disabled) {
        styles.colors.ghost_element_disabled = c;
    }
    // 组5：text 族残余直名槽。
    if let Some(c) = slot(&spec.surface_background) {
        styles.colors.surface_background = c;
    }
    if let Some(c) = slot(&spec.text_muted) {
        styles.colors.text_muted = c;
    }
    if let Some(c) = slot(&spec.text_placeholder) {
        styles.colors.text_placeholder = c;
    }
    if let Some(c) = slot(&spec.text_disabled) {
        styles.colors.text_disabled = c;
    }
    if let Some(c) = slot(&spec.text_accent) {
        styles.colors.text_accent = c;
    }
    // 组6：icon 族。
    if let Some(c) = slot(&spec.icon) {
        styles.colors.icon = c;
    }
    if let Some(c) = slot(&spec.icon_muted) {
        styles.colors.icon_muted = c;
    }
    if let Some(c) = slot(&spec.icon_disabled) {
        styles.colors.icon_disabled = c;
    }
    theme.styles = styles;
    Some(theme)
}

/// #110 G4：写用户主题文件（themes_dir/<name>.json——G2 装载格式：
/// name + appearance + 6 高频槽；取色器「保存为主题」收口）。
/// 返回 Some(路径) = 写盘成功（调用方据此提示/装载）。
pub fn save_user_theme_file(
    name: &str,
    slots: &[(String, String)],
    cx: &mut gpui::App,
) -> Option<std::path::PathBuf> {
    let dir = themes_dir()?;
    let _ = std::fs::create_dir_all(&dir);
    let mut value = serde_json::json!({"name": name});
    // hex 原文透传（保真——调用方已校验合法；#110 G4：色值 Hsla 往返
    // 漂移教训（parse_hsla 是装载侧灰度近似语义，不作保存中转））。
    for (key, hex) in slots {
        value[key.as_str()] = serde_json::json!(hex);
    }
    let path = dir.join(format!("{name}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(&value).ok()?)
        .ok()?;
    let _ = cx.refresh_windows();
    Some(path)
}

/// hex（#rrggbb / #rgb）或 hsla 数组 → Hsla（坏值 None）。
/// #110 G4 起 pub(crate)：设置弹窗草稿校验复用（行内报错判定）。
pub(crate) fn parse_hsla(s: &str) -> Option<gpui::Hsla> {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix('#') {
        let (r, g, b) = match hex.len() {
            6 => (
                u8::from_str_radix(&hex[0..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..6], 16).ok()?,
            ),
            3 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()?;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()?;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()?;
                (r * 17, g * 17, b * 17)
            }
            _ => return None,
        };
        return Some(gpui::Hsla {
            h: 0.,
            s: 0.,
            l: (r.max(g).max(b) as f32 + r.min(g).min(b) as f32) / 510.,
            a: 1.,
        });
    }
    None
}

/// #108 批③：大纲折叠态读写（`outline_collapsed` 键：折叠头行索引
/// 数组——面板 refresh 差量刷新后按当前行集装载；沿 indent_unit
/// /auto_close 载体先例）。
pub fn save_outline_collapsed(indexes: Vec<usize>) {
    if let Some(dir) = config_dir() {
        let mut value = load_from(&dir);
        value["outline_collapsed"] =
            serde_json::Value::Array(indexes.into_iter().map(|i| serde_json::json!(i)).collect());
        save_to(&dir, &value);
    }
}

pub fn load_outline_collapsed() -> Vec<usize> {
    config_dir()
        .map(|dir| load_from(&dir))
        .and_then(|v| {
            v.get("outline_collapsed")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|i| i.as_u64().map(|u| u as usize))
                        .collect::<Vec<usize>>()
                })
        })
        .unwrap_or_default()
}

/// #114 批①：会话标签列表读写（`session_tabs` 键：打开文件路径数组 +
/// `session_active` 键：激活索引——沿 F2 先例；open/close/activate 三
/// 时机统一落盘）。
pub fn save_session_tabs(paths: &[std::path::PathBuf], active: usize) {
    if let Some(dir) = config_dir() {
        let mut value = load_from(&dir);
        value["session_tabs"] = serde_json::Value::Array(
            paths
                .iter()
                .map(|p| serde_json::json!(p.to_string_lossy()))
                .collect(),
        );
        value["session_active"] = serde_json::json!(active);
        save_to(&dir, &value);
    }
}

pub fn load_session_tabs() -> (Vec<std::path::PathBuf>, usize) {
    config_dir()
        .map(|dir| load_from(&dir))
        .map(|v| {
            let paths = v
                .get("session_tabs")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|i| i.as_str().map(std::path::PathBuf::from))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let active = v
                .get("session_active")
                .and_then(|i| i.as_u64())
                .map(|u| u as usize)
                .unwrap_or(0);
            (paths, active)
        })
        .unwrap_or_default()
}

/// #114 P1④（#230 同型带入——本分支基面未合）：窗口矩形写（
/// `window_bounds` 键 {x,y,w,h}）。
pub fn save_window_bounds(x: f32, y: f32, w: f32, h: f32) {
    if let Some(dir) = config_dir() {
        let mut value = load_from(&dir);
        value["window_bounds"] = serde_json::json!({ "x": x, "y": y, "w": w, "h": h });
        save_to(&dir, &value);
    }
}

/// 读回 + 校验钳制（报告锚点 layout.rs:379-394 语义）：坏数据（负值/
/// 小于最小 760×480）回 None（调用方走默认 96,64,1280,800）。
pub fn load_window_bounds() -> Option<(f32, f32, f32, f32)> {
    load_window_bounds_from(&config_dir()?)
}

/// 注入变体（测试隔离——目录直达）。
pub fn load_window_bounds_from(dir: &Path) -> Option<(f32, f32, f32, f32)> {
    let v = load_from(dir);
    let o = v.get("window_bounds")?;
    let (x, y, w, h) = (
        o.get("x")?.as_f64()?,
        o.get("y")?.as_f64()?,
        o.get("w")?.as_f64()?,
        o.get("h")?.as_f64()?,
    );
    // 校验：全正 + 不小于最小窗（760×480——WindowOptions window_min_size
    // 同口径）；越界（屏不存在时 WM 自钳）不预判——只拒明显坏值。
    if x < 0. || y < 0. || w < 760. || h < 480. || w > 16384. || h > 16384. {
        return None;
    }
    Some((x as f32, y as f32, w as f32, h as f32))
}

pub fn load_indent_unit() -> Option<String> {
    let dir = config_dir()?;
    load_from(&dir)
        .get("indent_unit")?
        .as_str()
        .map(|s| s.to_string())
}

/// G4：面板宽度读写（`panel_widths` 键：`{"left": px, "right": px}`
/// 对象——拖拽热区收尾时落盘；消费方钳制到面板拖拽域）。
pub fn save_panel_widths(left: f32, right: f32) {
    if let Some(dir) = config_dir() {
        let mut value = load_from(&dir);
        value["panel_widths"] = serde_json::json!({ "left": left, "right": right });
        save_to(&dir, &value);
    }
}

pub fn load_panel_widths() -> Option<(f32, f32)> {
    let dir = config_dir()?;
    let value = load_from(&dir);
    let widths = value.get("panel_widths")?;
    let left = widths.get("left")?.as_f64()? as f32;
    let right = widths.get("right")?.as_f64()? as f32;
    Some((left, right))
}

/// #120（mdc-dock 批②）：dock 布局节点读写。
///
/// 结构（serde 形状发布冻结——[`mdc_dock::DockAreaState`]）:
/// `{"dock": {"left": <DockAreaState>, "right_open": <bool>}}`
///
/// 版本裁决在 [`mdc_dock::Dock::restore`]（不符整份丢弃回默认，
/// Q-IDE layout.rs:518-521 口径）；本层只管存取与坏值容忍：
/// 缺键/坏 JSON/类型不符 → `None` → 调用方走默认（开/无激活/缺省宽）。
/// #106 报告 §6 顺手缺陷（开合态不持久化）收口。
pub fn save_dock_layout(left: &mdc_dock::DockAreaState, right_open: bool) {
    if let Some(dir) = config_dir() {
        save_dock_layout_in(&dir, left, right_open);
    }
}

/// #120：dock 布局记忆读取（无/坏/缺 = None——调用方走默认）。
pub fn load_dock_layout() -> Option<(mdc_dock::DockAreaState, bool)> {
    let dir = config_dir()?;
    load_dock_layout_in(&dir)
}

fn save_dock_layout_in(dir: &Path, left: &mdc_dock::DockAreaState, right_open: bool) {
    let mut value = load_from(dir);
    value["dock"] = serde_json::json!({
        // left 整体经 serde 序列化（version 在其内——恢复时校验）。
        "left": left,
        "right_open": right_open,
    });
    save_to(dir, &value);
}

fn load_dock_layout_in(dir: &Path) -> Option<(mdc_dock::DockAreaState, bool)> {
    let value = load_from(dir);
    let node = value.get("dock")?;
    let left: mdc_dock::DockAreaState = serde_json::from_value(node.get("left")?.clone()).ok()?;
    let right_open = node.get("right_open")?.as_bool()?;
    Some((left, right_open))
}

/// #121（批③b）：右 dock 布局落盘（`dock.right` additive 键——旧文件
/// 缺键读取走 None 默认，`left`/`right_open` 形状不动：serde 形状冻结
/// 纪律只加不改）。
pub fn save_right_dock_layout(right: &mdc_dock::DockAreaState) {
    if let Some(dir) = config_dir() {
        save_right_dock_layout_in(&dir, right);
    }
}

/// #121（批③b）：右 dock 布局记忆读取（无/坏/缺 = None——调用方默认）。
pub fn load_right_dock_layout() -> Option<mdc_dock::DockAreaState> {
    let dir = config_dir()?;
    load_right_dock_layout_in(&dir)
}

/// #112 P1（MRU 持久化）：命令面板最近使用落盘（`palette.recent` 键，
/// 值 = 命令稳定名数组——[`crate::editor::commands::Cmd::key_name`] 口径，
/// 最新在前）。write-through：容量 4 的微 JSON，无需防抖。
pub fn save_palette_recent(names: &[String]) {
    if let Some(dir) = config_dir() {
        save_palette_recent_in(&dir, names);
    }
}

/// #112 P1：恢复最近使用（构造点消费——旧档未知名/缺键返回空，容忍前向）。
pub fn load_palette_recent() -> Vec<String> {
    config_dir()
        .map(|dir| load_palette_recent_in(&dir))
        .unwrap_or_default()
}

/// 测试形态：直接写指定目录（e2e 注入用——不动进程 env，避免并行套件
/// 下的 XDG 污染窗；生产路径走 [`save_palette_recent`]）。
pub fn save_palette_recent_for_test(dir: &Path, names: &[String]) {
    std::fs::create_dir_all(dir).unwrap();
    save_palette_recent_in(dir, names);
}

/// 测试形态：直接读指定目录（同上）。
pub fn load_palette_recent_from_test(dir: &Path) -> Vec<String> {
    load_palette_recent_in(dir)
}

fn save_palette_recent_in(dir: &Path, names: &[String]) {
    let mut value = load_from(dir);
    if value.get("palette").is_none() {
        value["palette"] = serde_json::json!({});
    }
    value["palette"]["recent"] = serde_json::json!(names);
    save_to(dir, &value);
}

fn load_palette_recent_in(dir: &Path) -> Vec<String> {
    value_recent_names(load_from(dir).get("palette"))
}

fn value_recent_names(palette: Option<&serde_json::Value>) -> Vec<String> {
    palette
        .and_then(|p| p.get("recent"))
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn save_right_dock_layout_in(dir: &Path, right: &mdc_dock::DockAreaState) {
    let mut value = load_from(dir);
    // dock 节点存在则 additive 合入；不存在则建骨架（right_open 用
    // 中性 true——随后 save_dock_layout 会写真实值）。
    if value.get("dock").is_none() {
        value["dock"] = serde_json::json!({});
    }
    value["dock"]["right"] = serde_json::to_value(right).unwrap_or(serde_json::Value::Null);
    save_to(dir, &value);
}

fn load_right_dock_layout_in(dir: &Path) -> Option<mdc_dock::DockAreaState> {
    let value = load_from(dir);
    let node = value.get("dock")?;
    let right: mdc_dock::DockAreaState = serde_json::from_value(node.get("right")?.clone()).ok()?;
    Some(right)
}

/// 记住工作区（`workspace` 键——路径以 lossy 写入，非 UTF-8 路径的
/// 记忆为近似值[边界登记]）。
pub fn remember_workspace(root: &Path) {
    if let Some(dir) = config_dir() {
        remember_workspace_in(&dir, root);
    }
}

/// 恢复上次工作区（无记忆/读失败 → None）。
pub fn remembered_workspace() -> Option<PathBuf> {
    let dir = config_dir()?;
    remembered_workspace_in(&dir)
}

/// F6：主题模式读写（`theme` 键："system"/"light"/"dark"——未知值由
/// [`crate::theme_switch::parse_mode`] 归一到 system）。
pub fn save_theme_mode(mode: &str) {
    if let Some(dir) = config_dir() {
        let mut value = load_from(&dir);
        value["theme"] = serde_json::Value::String(mode.to_string());
        save_to(&dir, &value);
    }
}

pub fn load_theme_mode() -> Option<String> {
    let dir = config_dir()?;
    load_from(&dir)
        .get("theme")?
        .as_str()
        .map(|s| s.to_string())
}

/// F6：编辑器字号读写（`font_size` 键——pt 数；8-32 域外由消费方
/// clamp）。
pub fn save_font_size(size: f32) {
    if let Some(dir) = config_dir() {
        let mut value = load_from(&dir);
        value["font_size"] = serde_json::Value::from(size);
        save_to(&dir, &value);
    }
}

pub fn load_font_size() -> Option<f32> {
    let dir = config_dir()?;
    load_from(&dir).get("font_size")?.as_f64().map(|v| v as f32)
}

/// #217：键位覆盖读取（`keymap` 键：对象数组——`{"bind": "ctrl+alt+p",
/// "action": "toggle_command_palette"}` 换绑 / `{"unbind": "ctrl+shift+p"}`
/// 解绑，按序应用）。`keymap` 键存在但非数组 → stderr 警告后忽略
/// （#218 review：不再静默——配置类型错误应可发现）；结构不符的
/// **单条**跳过（stderr 警告——与 `keymap::KeymapStore::apply_user`
/// 的逐条容错同口径：一条坏配置不废掉全部自定义）；键串/动作名的
/// 合法性由 keymap 层检测并警告。解析体见 [`parse_keymap_entries`]
/// （纯函数——单测面）。
pub fn load_keymap_overrides() -> Vec<crate::keymap::UserOverride> {
    let Some(dir) = config_dir() else {
        return Vec::new();
    };
    parse_keymap_entries(&load_from(&dir))
}

/// `keymap` 键解析（纯函数——仅依赖 serde_json + std，`load_from`
/// 产物或任意等价 JSON 树）：
///
/// - 键缺失 → 空（静默——未配置即默认键位）；
/// - 键存在但非数组 → stderr 警告（类型中文名）+ 空；
/// - 数组 → 逐条解析（单条结构错误跳过 + 警告）。
fn parse_keymap_entries(value: &serde_json::Value) -> Vec<crate::keymap::UserOverride> {
    let Some(entries) = value.get("keymap") else {
        return Vec::new();
    };
    let Some(arr) = entries.as_array() else {
        eprintln!(
            "keymap: 配置应为数组（当前为{}），已忽略",
            json_type_name_cn(entries)
        );
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, entry) in arr.iter().enumerate() {
        let Some(obj) = entry.as_object() else {
            eprintln!("keymap: 第 {i} 条不是对象，已跳过");
            continue;
        };
        if let (Some(bind), Some(action)) = (
            obj.get("bind").and_then(|v| v.as_str()),
            obj.get("action").and_then(|v| v.as_str()),
        ) {
            out.push(crate::keymap::UserOverride::Bind {
                bind: bind.to_string(),
                action: action.to_string(),
            });
        } else if let Some(bind) = obj.get("unbind").and_then(|v| v.as_str()) {
            out.push(crate::keymap::UserOverride::Unbind {
                bind: bind.to_string(),
            });
        } else {
            eprintln!("keymap: 第 {i} 条缺 bind/action 或 unbind 键，已跳过");
        }
    }
    out
}

/// JSON 值类型中文名（keymap 非数组警告的「当前为{类型}」填充——
/// 与逐条警告「不是对象」同风格的中文可读面）。
fn json_type_name_cn(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "布尔",
        serde_json::Value::Number(_) => "数字",
        serde_json::Value::String(_) => "字符串",
        serde_json::Value::Array(_) => "数组",
        serde_json::Value::Object(_) => "对象",
    }
}

/// #132-3：折叠状态读写（`folds` 键：`{"<文件路径>": [起始行, ...]}`）。
///
/// 语义：**按文件记住「折叠头行的行号集合」**（0 基）。存头行而非区间，
/// 是因为区间尾行在编辑后会漂移；头行在同一块没被编辑时相对稳定，
/// 恢复时由调用方按当前文本重算区间尾（算不出就丢弃该条——宁缺毋错）。
///
/// 容量：每文件最多 [`FOLD_PERSIST_CAP`] 条、全局最多
/// [`FOLD_FILE_CAP`] 个文件（超限丢最旧写入的那个键，避免配置文件
/// 无限增长）。
pub const FOLD_PERSIST_CAP: usize = 200;
/// 全局最多记住多少个文件的折叠状态。
pub const FOLD_FILE_CAP: usize = 64;

pub fn save_folds(key: &str, heads: &[usize]) {
    if let Some(dir) = config_dir() {
        save_folds_in(&dir, key, heads);
    }
}

pub fn load_folds(key: &str) -> Vec<usize> {
    let Some(dir) = config_dir() else {
        return Vec::new();
    };
    load_folds_in(&dir, key)
}

fn save_folds_in(dir: &Path, key: &str, heads: &[usize]) {
    // 空键 = 未命名文档：不持久化（无稳定身份）。
    if key.is_empty() {
        return;
    }
    let mut value = load_from(dir);
    if value.get("folds").is_none_or(|v| !v.is_object()) {
        value["folds"] = serde_json::json!({});
    }
    let Some(map) = value["folds"].as_object_mut() else {
        return;
    };
    if heads.is_empty() {
        // 全展开 = 清掉该文件的记录（不积空数组垃圾）。
        map.remove(key);
    } else {
        let mut sorted: Vec<usize> = heads.iter().copied().take(FOLD_PERSIST_CAP).collect();
        sorted.sort_unstable();
        sorted.dedup();
        map.insert(
            key.to_string(),
            serde_json::Value::Array(
                sorted
                    .into_iter()
                    .map(|h| serde_json::Value::from(h as u64))
                    .collect(),
            ),
        );
        // 容量控制：超限时丢掉最旧写入的**别的**文件（Map 保插入序）。
        if map.len() > FOLD_FILE_CAP
            && let Some(oldest) = map.keys().find(|k| k.as_str() != key).cloned()
        {
            map.remove(&oldest);
        }
    }
    save_to(dir, &value);
}

fn load_folds_in(dir: &Path, key: &str) -> Vec<usize> {
    if key.is_empty() {
        return Vec::new();
    }
    let value = load_from(dir);
    let Some(arr) = value
        .get("folds")
        .and_then(|f| f.get(key))
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };
    let mut out: Vec<usize> = arr
        .iter()
        .filter_map(|v| v.as_u64())
        .map(|v| v as usize)
        .take(FOLD_PERSIST_CAP)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn remember_workspace_in(dir: &Path, root: &Path) {
    let mut value = load_from(dir);
    value["workspace"] = serde_json::Value::String(root.to_string_lossy().into_owned());
    save_to(dir, &value);
}

fn remembered_workspace_in(dir: &Path) -> Option<PathBuf> {
    load_from(dir).get("workspace")?.as_str().map(PathBuf::from)
}

/// #132 任务 2：查找历史读写（`find_history` 键：字符串数组，新→旧）。
///
/// 容错口径与其余键一致：目录/文件不可得 → 静默跳过（历史是增强不是依赖）。
pub fn save_find_history(entries: &[String]) {
    if let Some(dir) = config_dir() {
        save_find_history_in(&dir, entries);
    }
}

pub fn load_find_history() -> Vec<String> {
    config_dir()
        .map(|dir| load_find_history_in(&dir))
        .unwrap_or_default()
}

/// #132 任务 2：正则开关读写（`find_regex` 键：布尔）。
pub fn save_find_regex(on: bool) {
    if let Some(dir) = config_dir() {
        save_find_regex_in(&dir, on);
    }
}

pub fn load_find_regex() -> bool {
    config_dir()
        .map(|dir| load_find_regex_in(&dir))
        .unwrap_or(false)
}

// ---- 目录显式传入的底层实现（单测用——并行测试不互相污染全局态） ----

fn save_find_history_in(dir: &Path, entries: &[String]) {
    let mut value = load_from(dir);
    value["find_history"] = serde_json::Value::Array(
        entries
            .iter()
            .map(|s| serde_json::Value::String(s.clone()))
            .collect(),
    );
    save_to(dir, &value);
}

fn load_find_history_in(dir: &Path) -> Vec<String> {
    load_from(dir)
        .get("find_history")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn save_find_regex_in(dir: &Path, on: bool) {
    let mut value = load_from(dir);
    value["find_regex"] = serde_json::Value::Bool(on);
    save_to(dir, &value);
}

fn load_find_regex_in(dir: &Path) -> bool {
    load_from(dir)
        .get("find_regex")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// UI 字体（界面元素）。
pub fn ui_font() -> &'static Font {
    static FONT: std::sync::OnceLock<Font> = std::sync::OnceLock::new();
    FONT.get_or_init(|| Font {
        family: "Noto Sans SC".into(),
        ..Font::default()
    })
}

/// 缓冲区字体（编辑区/等宽面）。
pub fn buffer_font() -> &'static Font {
    static FONT: std::sync::OnceLock<Font> = std::sync::OnceLock::new();
    FONT.get_or_init(|| Font {
        family: "DejaVu Sans Mono".into(),
        ..Font::default()
    })
}

pub struct ShellThemeSettings;

impl ThemeSettingsProvider for ShellThemeSettings {
    fn ui_font<'a>(&'a self, _cx: &'a App) -> &'a Font {
        ui_font()
    }

    fn buffer_font<'a>(&'a self, _cx: &'a App) -> &'a Font {
        buffer_font()
    }

    fn ui_font_size(&self, _cx: &App) -> Pixels {
        px(15.)
    }

    fn buffer_font_size(&self, _cx: &App) -> Pixels {
        px(14.)
    }

    fn ui_density(&self, _cx: &App) -> UiDensity {
        UiDensity::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mang-settings-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// #110 G4：save_user_theme_file 写读往返（写盘 → load_user_themes
    /// 装载 → registry 取回验名与槽色；hex 往返 6 槽）。
    /// #110 G1 批A：31 槽全量写读往返（save_user_theme_file 全槽透传 +
    /// parse_user_theme 全槽覆盖装载——每槽 hex 保真入 ThemeColors）。
    #[gpui::test]
    fn user_theme_31_slot_roundtrip(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            theme::init(theme::LoadThemes::JustBase, cx);
        });
        let tmp = std::env::temp_dir().join(format!("mang-g1a-31-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let themes = tmp.join("mang").join("themes");
        std::fs::create_dir_all(&themes).unwrap();
        let prev_xdg = std::env::var_os("XDG_CONFIG_HOME").unwrap_or_default();
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &tmp) };
        // 31 槽（shell 位表键序同源——手写镜像，避免跨模块依赖）。
        let keys: [&str; 31] = [
            "background",
            "surface",
            "editor",
            "foreground",
            "accent",
            "border",
            "border_variant",
            "border_focused",
            "border_selected",
            "border_transparent",
            "border_disabled",
            "element_background",
            "element_hover",
            "element_active",
            "element_selected",
            "element_selection_background",
            "element_disabled",
            "ghost_element_background",
            "ghost_element_hover",
            "ghost_element_active",
            "ghost_element_selected",
            "ghost_element_disabled",
            "surface_background",
            "text_muted",
            "text_placeholder",
            "text_disabled",
            "text_accent",
            "icon",
            "icon_muted",
            "icon_disabled",
            "text",
        ];
        // 全槽给可辨识 hex（每槽 r 通道递增——装载后逐槽断言取得到）。
        let slots: Vec<(String, String)> = keys
            .iter()
            .enumerate()
            .map(|(i, k)| {
                let r = 0x20 + i as u8; // 0x20..0x3E 互异
                (k.to_string(), format!("#{:02x}1010", r))
            })
            .collect();
        cx.update(|cx| {
            let path = super::save_user_theme_file("g1a-31", &slots, cx);
            assert!(path.is_some(), "写盘成功");
            let loaded = super::load_user_themes(cx);
            assert_eq!(loaded, 1, "装载 1 个");
            let registry = theme::ThemeRegistry::default_global(cx);
            let t = registry.get("g1a-31").expect("registry 可取");
            let c = &t.styles.colors;
            // 高频六键（级联派生面）+ 直名槽抽样断言（31 全断言太脆——
            // 键→字段映射用编译期同构保证，这里抽各族代表）。
            let base = registry.get("One Dark").unwrap();
            assert_ne!(c.background, base.styles.colors.background, "background 已覆盖");
            assert_ne!(
                c.border_focused,
                base.styles.colors.border_focused,
                "border_focused 已覆盖"
            );
            assert_ne!(
                c.element_selected,
                base.styles.colors.element_selected,
                "element_selected 已覆盖"
            );
            assert_ne!(
                c.ghost_element_disabled,
                base.styles.colors.ghost_element_disabled,
                "ghost_element_disabled 已覆盖"
            );
            assert_ne!(
                c.text_placeholder,
                base.styles.colors.text_placeholder,
                "text_placeholder 已覆盖"
            );
            assert_ne!(c.icon, base.styles.colors.icon, "icon 已覆盖");
            // 直名槽的值断言（G2 灰度近似语义：hex → l = (max+min)/510——
            // 断言 l 分量精确复原，色相丢弃是装载侧既定语义非缺陷）。
            let want_l = |hex: &str| {
                let h = hex.trim_start_matches('#');
                let r = u8::from_str_radix(&h[0..2], 16).unwrap();
                let g = u8::from_str_radix(&h[2..4], 16).unwrap();
                let b = u8::from_str_radix(&h[4..6], 16).unwrap();
                (r.max(g).max(b) as f32 + r.min(g).min(b) as f32) / 510.
            };
            // icon（index 27）：#3B1010（0x20+27——键序 r 通道递增）。
            let want = want_l("#3B1010");
            let got = c.icon.l;
            assert!(
                (got - want).abs() < 0.004,
                "icon l 分量：{got} vs {want}（G2 灰度语义）"
            );
            // ghost_element_disabled（index 21）：#351010。
            let want = want_l("#351010");
            let got = c.ghost_element_disabled.l;
            assert!(
                (got - want).abs() < 0.004,
                "ghost disabled l 分量：{got} vs {want}（G2 灰度语义）"
            );
        });
        unsafe { std::env::set_var("XDG_CONFIG_HOME", prev_xdg) };
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// #110 G1 批A：双主题基底（appearance:light → derive_light 浅色基底
    /// 派生——覆盖槽盖在浅色上；dark/缺省 → 深色基底）。
    #[gpui::test]
    fn user_theme_dual_base_light_dark(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            theme::init(theme::LoadThemes::JustBase, cx);
        });
        let tmp = std::env::temp_dir().join(format!("mang-g1a-dual-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let themes = tmp.join("mang").join("themes");
        std::fs::create_dir_all(&themes).unwrap();
        let prev_xdg = std::env::var_os("XDG_CONFIG_HOME").unwrap_or_default();
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &tmp) };
        // 两文件：light 版 + dark 版（同槽不同色）。
        let light_json = r##"{
            "name": "g1a-light",
            "appearance": "light",
            "background": "#f0f0f0"
        }"##;
        let dark_json = r##"{
            "name": "g1a-dark",
            "appearance": "dark",
            "background": "#101010"
        }"##;
        std::fs::write(themes.join("g1a-light.json"), light_json).unwrap();
        std::fs::write(themes.join("g1a-dark.json"), dark_json).unwrap();
        cx.update(|cx| {
            let loaded = super::load_user_themes(cx);
            assert_eq!(loaded, 2, "装载 2 个");
            let registry = theme::ThemeRegistry::default_global(cx);
            let light = registry.get("g1a-light").expect("light 可取");
            let dark = registry.get("g1a-dark").expect("dark 可取");
            // appearance 落位。
            assert_eq!(light.appearance, theme::Appearance::Light);
            assert_eq!(dark.appearance, theme::Appearance::Dark);
            // 双基底对照核心：light 的未覆盖槽 = 浅色派生值（亮），dark 的
            // 未覆盖槽 = One Dark 值（暗）。用 element_hover（两文件均未覆盖）。
            let base = registry.get("One Dark").unwrap();
            let light_hover = light.styles.colors.element_hover;
            let dark_hover = dark.styles.colors.element_hover;
            let base_hover = base.styles.colors.element_hover;
            assert!(
                light_hover != base_hover,
                "light 未覆盖槽走浅色基底（不是深色原值）"
            );
            assert_eq!(
                dark_hover, base_hover,
                "dark 未覆盖槽走深色基底（One Dark 原值）"
            );
            // 覆盖槽双主题各自生效（background 两值互异）。
            assert_ne!(
                light.styles.colors.background,
                dark.styles.colors.background
            );
        });
        unsafe { std::env::set_var("XDG_CONFIG_HOME", prev_xdg) };
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[gpui::test]
    fn user_theme_save_roundtrip(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            theme::init(theme::LoadThemes::JustBase, cx);
        });
        // 指定用户主题目录（temp——config_dir 同源语义由 themes_dir
        // 读 XDG；测试用 env 重定向）。
        let tmp = std::env::temp_dir().join(format!("mang-g4-roundtrip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let themes = tmp.join("mang").join("themes");
        std::fs::create_dir_all(&themes).unwrap();
        let prev_xdg = std::env::var_os("XDG_CONFIG_HOME").unwrap_or_default();
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &tmp) };
        let slots: Vec<(String, String)> = [
            ("background", "#1a1a1a"),
            ("surface", "#1f1f1f"),
            ("editor", "#242424"),
            ("foreground", "#e6e6e6"),
            ("accent", "#808080"),
            ("border", "#4d4d4d"),
        ]
        .into_iter()
        .map(|(k, hex)| (k.to_string(), hex.to_string()))
        .collect();
        cx.update(|cx| {
            let path = super::save_user_theme_file("g4-test", &slots, cx);
            let path = path.expect("写盘成功");
            assert!(path.ends_with("g4-test.json"), "文件名：{:?}", path);
            let text = std::fs::read_to_string(&path).unwrap();
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(v["name"], "g4-test");
            assert_eq!(v["background"], "#1a1a1a", "hex 往返 background");
            // 装载链：load_user_themes 入 registry。
            let loaded = super::load_user_themes(cx);
            assert_eq!(loaded, 1, "装载 1 个");
            let registry = theme::ThemeRegistry::default_global(cx);
            let t = registry.get("g4-test").expect("registry 可取");
            assert_eq!(t.name.as_ref(), "g4-test");
        });
        // XDG 还原（G2 测试同型）。
        unsafe { std::env::set_var("XDG_CONFIG_HOME", prev_xdg) };
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// #110 G4：坏色拒（parse_hsla 坏值 None——行内报错同源判定）。
    #[test]
    fn parse_hsla_rejects_bad_hex() {
        assert!(super::parse_hsla("#1a2").is_some(), "#1a2 是 #rgb 短格式");
        assert!(super::parse_hsla("#1a2b3c").is_some());
        assert!(super::parse_hsla("#zzzzzz").is_none(), "坏 hex 拒");
        assert!(super::parse_hsla("1a2b3c").is_none(), "缺 # 拒（一期口径）");
        assert!(super::parse_hsla("").is_none(), "空拒");
    }

    /// #111 批②：auto_close_brackets 写读往返（load_from/save_to 私有
    /// 面经 mod tests 子模块可见性直达——零全局 config_dir 污染）。
    #[test]
    fn auto_close_brackets_roundtrip() {
        let dir = temp_config("auto-close");
        std::fs::create_dir_all(&dir).unwrap();
        let mut value = load_from(&dir);
        value["auto_close_brackets"] = serde_json::Value::Bool(false);
        save_to(&dir, &value);
        let back = load_from(&dir);
        assert_eq!(
            back.get("auto_close_brackets").and_then(|b| b.as_bool()),
            Some(false),
            "写读往返保持"
        );
        // 未知/缺失键 → 消费方 unwrap_or(true) 缺省开（构造面语义）。
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #111 P2：soft_wrap 写读往返（载体面——沿 auto_close_brackets
    /// roundtrip 同型；缺失键 → 消费方 unwrap_or(true) 缺省开）。
    #[test]
    fn soft_wrap_roundtrip() {
        let dir = temp_config("soft-wrap");
        std::fs::create_dir_all(&dir).unwrap();
        let mut value = load_from(&dir);
        value["soft_wrap"] = serde_json::Value::Bool(false);
        save_to(&dir, &value);
        let back = load_from(&dir);
        assert_eq!(
            back.get("soft_wrap").and_then(|b| b.as_bool()),
            Some(false),
            "写读往返保持"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #110 G2：用户主题装载（磁盘 → registry——XDG 临时区 + 坏文件
    /// 容错 + registry 可见性三面）。
    #[gpui::test]
    fn user_theme_loads_into_registry(cx: &mut gpui::TestAppContext) {
        let xdg = std::env::temp_dir().join(format!("mg2-themes-{}", std::process::id()));
        let themes = xdg.join("mang").join("themes");
        let _ = std::fs::create_dir_all(&themes);
        let prev = std::env::var_os("XDG_CONFIG_HOME");
        unsafe {
            std::env::set_var(
                "XDG_CONFIG_HOME",
                xdg.join("mang")
                    .parent()
                    .map(|_| xdg.clone())
                    .unwrap_or(xdg.clone()),
            )
        };
        // 注：XDG 指到 xdg 根（config_dir = XDG/mang；themes = XDG/mang/themes）。
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &xdg) };
        // 写一个用户主题 + 一个坏文件。
        std::fs::write(
            themes.join("my-dark.json"),
            format!(
                "{{\"name\": \"My Dark\", \"appearance\": \"dark\", \"background\": \"#{}\"}}",
                "1a2b3c"
            ),
        )
        .unwrap();
        std::fs::write(themes.join("bad.json"), "{not json").unwrap();
        let _ = std::fs::write(themes.join("ignored.txt"), "非 json 跳过");

        cx.update(|cx| {
            theme::init(theme::LoadThemes::JustBase, cx);
            let loaded = super::load_user_themes(cx);
            assert_eq!(loaded, 1, "好文件装载 1（坏/非 json 跳过）");
            let names = theme::ThemeRegistry::default_global(cx).list_names();
            assert!(
                names.iter().any(|n| n.as_ref() == "My Dark"),
                "registry 可见用户主题：{names:?}"
            );
        });

        // XDG 还原。
        match prev {
            Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },
            None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
        }
        let _ = std::fs::remove_dir_all(&xdg);
    }

    /// #108 批③：outline_collapsed 写读往返（load_from/save_to 私有面
    /// 直达——零全局 config_dir 污染）。
    #[test]
    fn outline_collapsed_roundtrip() {
        let dir = temp_config("outline-fold");
        std::fs::create_dir_all(&dir).unwrap();
        let mut value = load_from(&dir);
        value["outline_collapsed"] =
            serde_json::Value::Array(vec![serde_json::json!(0), serde_json::json!(3)]);
        save_to(&dir, &value);
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(settings_path_in(&dir)).unwrap())
                .unwrap();
        assert_eq!(
            v.get("outline_collapsed"),
            Some(&serde_json::Value::Array(vec![
                serde_json::json!(0),
                serde_json::json!(3)
            ])),
            "折叠索引数组写读往返保持"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #114 批①：session_tabs/session_active 写读往返（隔离 temp dir）。
    #[test]
    fn session_tabs_roundtrip() {
        // 容器布局：XDG=container → config_dir=container/mang →
        // settings.json=container/mang/settings.json。
        let container = std::env::temp_dir().join(format!("m114-session-{}", std::process::id()));
        let dir = container.join("mang");
        std::fs::create_dir_all(&dir).unwrap();
        let mut value = load_from(&dir);
        value["session_tabs"] = serde_json::Value::Array(vec![
            serde_json::json!("/tmp/a.md"),
            serde_json::json!("/tmp/b.pdf"),
            serde_json::json!("/tmp/c.docx"),
        ]);
        value["session_active"] = serde_json::json!(1);
        save_to(&dir, &value);
        let prev = std::env::var_os("XDG_CONFIG_HOME");
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &container) };
        let (paths, active) = super::load_session_tabs();
        assert_eq!(paths.len(), 3, "三路径读回");
        assert_eq!(paths[1], std::path::PathBuf::from("/tmp/b.pdf"));
        assert_eq!(active, 1, "激活索引读回");
        match prev {
            Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },
            None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
        }
        let _ = std::fs::remove_dir_all(&container);
    }

    /// #114 批②：window_bounds 写读往返 + 坏数据校验回退（隔离
    /// temp dir——load_from/save_to 私有面直达）。
    #[test]
    fn window_bounds_roundtrip_and_validation() {
        let dir = temp_config("win-bounds");
        std::fs::create_dir_all(&dir).unwrap();
        let mut value = load_from(&dir);
        value["window_bounds"] =
            serde_json::json!({ "x": 120.0, "y": 80.0, "w": 1600.0, "h": 900.0 });
        save_to(&dir, &value);
        let back = super::load_window_bounds_from(&dir);
        assert_eq!(back, Some((120., 80., 1600., 900.)), "合法矩形读回");
        // 坏数据：负 origin / 小于最小窗 → None（调用方走默认）。
        let mut bad = load_from(&dir);
        bad["window_bounds"] = serde_json::json!({ "x": -5., "y": 0., "w": 1600., "h": 900. });
        save_to(&dir, &bad);
        assert_eq!(super::load_window_bounds_from(&dir), None, "负 origin 拒绝");
        let mut small = load_from(&dir);
        small["window_bounds"] = serde_json::json!({ "x": 0., "y": 0., "w": 500., "h": 300. });
        save_to(&dir, &small);
        assert_eq!(
            super::load_window_bounds_from(&dir),
            None,
            "小于最小窗 760×480 拒绝"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 工作区记忆往返（写入 → 读回 → 覆盖写 → 读回）。
    #[test]
    fn workspace_memory_roundtrip() {
        let dir = temp_config("roundtrip");
        let root = Path::new("/tmp/some-workspace");
        remember_workspace_in(&dir, root);
        assert_eq!(remembered_workspace_in(&dir).as_deref(), Some(root));

        let root2 = Path::new("/tmp/another-one");
        remember_workspace_in(&dir, root2);
        assert_eq!(
            remembered_workspace_in(&dir).as_deref(),
            Some(root2),
            "覆盖写生效"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 无记忆（目录空/文件不存在）→ None。
    #[test]
    fn missing_settings_returns_none() {
        let dir = temp_config("missing");
        assert_eq!(remembered_workspace_in(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #112 P1：面板最近使用往返（写 → 读回 → 覆盖写 → 读回）。
    #[test]
    fn palette_recent_roundtrip() {
        let dir = temp_config("palette-recent");
        save_palette_recent_in(&dir, &["Save".to_string(), "OpenFind".to_string()]);
        assert_eq!(
            load_palette_recent_in(&dir),
            vec!["Save".to_string(), "OpenFind".to_string()],
            "首写读回"
        );
        save_palette_recent_in(&dir, &["Undo".to_string()]);
        assert_eq!(
            load_palette_recent_in(&dir),
            vec!["Undo".to_string()],
            "覆盖写生效（最新在前）"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #112 P1：缺键/空档返回空；非字符串项过滤；既有键（theme）不被冲掉。
    #[test]
    fn palette_recent_missing_and_mixed_entries() {
        let dir = temp_config("palette-mixed");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(settings_path_in(&dir), r#"{ "theme": "dark" }"#).unwrap();
        assert!(load_palette_recent_in(&dir).is_empty(), "缺键=空");
        std::fs::write(
            settings_path_in(&dir),
            r#"{ "theme": "dark", "palette": { "recent": ["Save", 42, null, "Undo"] } }"#,
        )
        .unwrap();
        assert_eq!(
            load_palette_recent_in(&dir),
            vec!["Save".to_string(), "Undo".to_string()],
            "非字符串项过滤（旧档/脏档容忍）"
        );
        let value = load_from(&dir);
        assert_eq!(value["theme"].as_str(), Some("dark"), "既有键保留");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 既有其它键不被工作区写入冲掉（渐进扩展契约——F5/F6 共用此文件）。
    #[test]
    fn other_keys_preserved() {
        let dir = temp_config("preserve");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            settings_path_in(&dir),
            r#"{ "theme": "dark", "workspace": "/old" }"#,
        )
        .unwrap();
        remember_workspace_in(&dir, Path::new("/new"));
        let value = load_from(&dir);
        assert_eq!(value["theme"].as_str(), Some("dark"), "既有键保留");
        assert_eq!(value["workspace"].as_str(), Some("/new"), "新值生效");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F6：主题/字号键往返 + 与既有键共存。
    #[test]
    fn theme_and_font_size_roundtrip() {
        let dir = temp_config("f6");
        remember_workspace_in(&dir, Path::new("/ws"));
        let mut value = load_from(&dir);
        value["theme"] = serde_json::Value::String("light".into());
        value["font_size"] = serde_json::Value::from(18.0f32);
        save_to(&dir, &value);

        let value = load_from(&dir);
        assert_eq!(value["theme"].as_str(), Some("light"));
        assert_eq!(value["font_size"].as_f64(), Some(18.0));
        assert_eq!(value["workspace"].as_str(), Some("/ws"), "既有键保留");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// G3：缩进单位键往返 + 与既有键共存。
    ///
    /// 注：`save_indent_unit`/`load_indent_unit` 走全局 `config_dir()`，
    /// 测试并行时互相污染（各用例写同一文件）。此处测底层
    /// `save_to`/`load_from` 的键往返语义（与生产同路径，无全局态）。
    #[test]
    fn indent_unit_roundtrip() {
        let dir = temp_config("g3");
        remember_workspace_in(&dir, Path::new("/ws"));
        for key in ["2", "4", "tab"] {
            let mut value = load_from(&dir);
            value["indent_unit"] = serde_json::Value::String(key.to_string());
            save_to(&dir, &value);
            let back = load_from(&dir);
            assert_eq!(back["indent_unit"].as_str(), Some(key), "键值往返 {key}");
        }
        let value = load_from(&dir);
        assert_eq!(value["workspace"].as_str(), Some("/ws"), "既有键保留");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// G4：面板宽度键往返 + 与既有键共存（含覆盖写）。
    ///
    /// 注：与 `indent_unit_roundtrip` 同理——`save_panel_widths`/
    /// `load_panel_widths` 走全局 `config_dir()`，并行测试互相污染；
    /// 此处测底层 `save_to`/`load_from` 的 `panel_widths` 键往返语义
    ///（与生产同路径，无全局态）。
    #[test]
    fn panel_widths_roundtrip() {
        let dir = temp_config("g4");
        remember_workspace_in(&dir, Path::new("/ws"));
        for (left, right) in [(300., 340.), (500., 600.)] {
            let mut value = load_from(&dir);
            value["panel_widths"] = serde_json::json!({ "left": left, "right": right });
            save_to(&dir, &value);
            let back = load_from(&dir);
            assert_eq!(
                back["panel_widths"]["left"].as_f64(),
                Some(left),
                "左宽往返"
            );
            assert_eq!(
                back["panel_widths"]["right"].as_f64(),
                Some(right),
                "右宽往返（第二轮即覆盖写生效）"
            );
        }
        let value = load_from(&dir);
        assert_eq!(value["workspace"].as_str(), Some("/ws"), "既有键保留");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #132-3：折叠状态键往返（头行号数组 + 与既有键共存 + 空列表即删除）。
    ///
    /// 注：`save_folds`/`load_folds` 走全局 `config_dir()`，并行测试互相
    /// 污染；此处测底层 `save_folds_in`/`load_folds_in`（与生产同路径，
    /// 无全局态）。
    #[test]
    fn folds_roundtrip_per_file() {
        let dir = temp_config("fold");
        remember_workspace_in(&dir, Path::new("/ws"));
        save_folds_in(&dir, "/ws/a.md", &[7, 0, 3, 3]);
        save_folds_in(&dir, "/ws/b.md", &[1]);
        assert_eq!(load_folds_in(&dir, "/ws/a.md"), vec![0, 3, 7], "排序去重");
        assert_eq!(load_folds_in(&dir, "/ws/b.md"), vec![1], "按文件隔离");
        assert_eq!(
            load_folds_in(&dir, "/ws/c.md"),
            Vec::<usize>::new(),
            "未知文件空"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #132：查找历史键往返 + 截断语义（超 20 条由消费方截——本层如实存）。
    #[test]
    fn find_history_roundtrip_and_coexist() {
        let dir = temp_config("find-history");
        remember_workspace_in(&dir, Path::new("/ws"));
        let entries: Vec<String> = vec!["\\bfn\\s+\\w+".into(), "foo".into(), "中文".into()];
        save_find_history_in(&dir, &entries);
        assert_eq!(
            load_find_history_in(&dir),
            entries,
            "历史往返（含正则/中文）"
        );

        // 覆盖写：新历史替换旧历史
        save_find_history_in(&dir, &["bar".to_string()]);
        assert_eq!(load_find_history_in(&dir), vec!["bar".to_string()]);

        // 既有键保留
        let value = load_from(&dir);
        assert_eq!(value["workspace"].as_str(), Some("/ws"), "既有键保留");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #132：正则开关键往返 + 缺省 false + 与既有键共存。
    #[test]
    fn find_regex_roundtrip() {
        let dir = temp_config("find-regex");
        assert!(!load_find_regex_in(&dir), "缺省关闭");
        save_find_regex_in(&dir, true);
        assert!(load_find_regex_in(&dir));
        save_find_regex_in(&dir, false);
        assert!(!load_find_regex_in(&dir), "覆盖写生效");

        remember_workspace_in(&dir, Path::new("/ws"));
        save_find_regex_in(&dir, true);
        let value = load_from(&dir);
        assert_eq!(value["workspace"].as_str(), Some("/ws"), "既有键保留");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 空头行集合 = 删除该文件记录（全展开后不留垃圾）。
    #[test]
    fn folds_empty_clears_file_entry() {
        let dir = temp_config("fold-clear");
        save_folds_in(&dir, "/ws/a.md", &[2, 5]);
        assert_eq!(load_folds_in(&dir, "/ws/a.md"), vec![2, 5]);
        save_folds_in(&dir, "/ws/a.md", &[]);
        assert_eq!(load_folds_in(&dir, "/ws/a.md"), Vec::<usize>::new());
        let value = load_from(&dir);
        assert!(
            value.get("folds").and_then(|f| f.get("/ws/a.md")).is_none(),
            "空列表应删除键：{value}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 未命名文档（空键）不落盘——无稳定身份。
    #[test]
    fn folds_empty_key_is_not_persisted() {
        let dir = temp_config("fold-nokey");
        save_folds_in(&dir, "", &[1, 2]);
        assert_eq!(load_folds_in(&dir, ""), Vec::<usize>::new());
        assert!(
            load_from(&dir).get("folds").is_none(),
            "空键不产生 folds 段"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #217/#218：keymap 键缺失 → 空（静默——未配置即默认键位；
    /// 空对象 / 整树 null / 只有别的键 同口径）。
    #[test]
    fn keymap_entries_missing_key_is_silent_empty() {
        assert!(parse_keymap_entries(&serde_json::json!({})).is_empty());
        assert!(parse_keymap_entries(&serde_json::Value::Null).is_empty());
        assert!(
            parse_keymap_entries(&serde_json::json!({ "theme": "dark", "font_size": 15 }))
                .is_empty()
        );
    }

    /// #218 review：keymap 键存在但非数组 → 警告 + 空（五种非数组类型
    /// 全覆盖——对象/字符串/数字/布尔/null；不再静默吞掉）。
    #[test]
    fn keymap_entries_non_array_types_warn_and_empty() {
        for value in [
            serde_json::json!({ "keymap": { "bind": "ctrl+alt+p" } }),
            serde_json::json!({ "keymap": "ctrl+alt+p" }),
            serde_json::json!({ "keymap": 42 }),
            serde_json::json!({ "keymap": true }),
            serde_json::json!({ "keymap": null }),
        ] {
            assert!(
                parse_keymap_entries(&value).is_empty(),
                "非数组 keymap 应解析为空：{value}"
            );
        }
    }

    /// #217：正常条目解析（bind/unbind 两形态）+ 坏条目混排逐条容错
    /// （非对象 / 缺键 / 值类型错——好条目照常产出，坏条目跳过）。
    #[test]
    fn keymap_entries_parses_good_and_skips_bad_mixed() {
        let value = serde_json::json!({
            "keymap": [
                { "bind": "ctrl+alt+p", "action": "toggle_command_palette" },
                "not an object",
                { "unbind": "ctrl+shift+p" },
                { "missing": "fields" },
                { "bind": 3, "action": "save" },
                { "unbind": 7 }
            ]
        });
        assert_eq!(
            parse_keymap_entries(&value),
            vec![
                crate::keymap::UserOverride::Bind {
                    bind: "ctrl+alt+p".into(),
                    action: "toggle_command_palette".into()
                },
                crate::keymap::UserOverride::Unbind {
                    bind: "ctrl+shift+p".into()
                },
            ],
            "好条目按序产出，坏条目（非对象/缺键/值类型错）跳过"
        );
    }
}
