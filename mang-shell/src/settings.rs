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
    let base = registry.get("One Dark").unwrap_or_else(|_| {
        registry
            .list()
            .into_iter()
            .next()
            .map(|m| registry.get(&m.name).unwrap())
            .unwrap()
    });
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
        match parse_user_theme(&text, &base) {
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
fn parse_user_theme(text: &str, base: &theme::Theme) -> Option<theme::Theme> {
    #[derive(serde::Deserialize)]
    struct UserThemeJson {
        name: String,
        appearance: Option<String>,
        background: Option<String>,
        surface: Option<String>,
        editor: Option<String>,
        foreground: Option<String>,
        accent: Option<String>,
        border: Option<String>,
        #[allow(dead_code)]
        #[serde(default)]
        author: Option<String>,
    }
    let spec: UserThemeJson = serde_json::from_str(text).ok()?;
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
    if let Some(c) = parse_hsla(spec.background.as_deref().unwrap_or("")) {
        styles.colors.background = c;
    }
    if let Some(c) = parse_hsla(spec.editor.as_deref().unwrap_or("")) {
        styles.colors.editor_background = c;
    }
    if let Some(c) = parse_hsla(spec.surface.as_deref().unwrap_or("")) {
        styles.colors.elevated_surface_background = c;
    }
    if let Some(c) = parse_hsla(spec.foreground.as_deref().unwrap_or("")) {
        styles.colors.text = c;
        styles.colors.text_muted = c;
    }
    if let Some(c) = parse_hsla(spec.border.as_deref().unwrap_or("")) {
        styles.colors.border = c;
        styles.colors.border_variant = c;
    }
    if let Some(c) = parse_hsla(spec.accent.as_deref().unwrap_or("")) {
        styles.colors.element_active = c;
        styles.colors.text_accent = c;
    }
    theme.styles = styles;
    Some(theme)
}

/// hex（#rrggbb / #rgb）或 hsla 数组 → Hsla（坏值 None）。
fn parse_hsla(s: &str) -> Option<gpui::Hsla> {
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
