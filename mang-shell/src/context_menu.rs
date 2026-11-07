//! 右键菜单数据层（#63 Lead 打磨项 → #385 批 1 迁 `ui::ContextMenu`）。
//!
//! # 分层（批 1 迁移后）
//!
//! - [`MenuItem`] / [`editor_menu`] / [`file_tree_menu`]：**纯数据**，零
//!   GPUI 依赖——可单测、可在无窗口环境构造。菜单有哪些项、哪些灰显、
//!   快捷键标签显示什么（keymap 单一事实源），全在这一层定死。
//! - 构建/部署层在 `context_menu_view.rs`：把 [`MenuItem`] 集构建为
//!   `ui::ContextMenu` 实体（CustomEntry + handler 回派 shell），部署用
//!   gpui `Anchored`（边缘翻转/夹紧）——旧的手写渲染浮层（146 行）与
//!   自研 `ContextMenu::layout` 几何已在批 1 退役。
//! - 点击/键盘接线在 `shell.rs`（`activate_context_menu_item` 派发体 +
//!   根 `on_key_down` 键盘导航）。
//! - 菜单**几何**（place_menu/estimate_width 族）已迁 `menu_geometry.rs`
//!   ——右键菜单路径不再消费（gpui Anchored 承担），存续消费者是批 4
//!   未迁移的顶部菜单栏。
//!
//! # 灰显语义
//!
//! `enabled: false` 用于「有菜单项但此刻不可用」——灰显而不是消失：
//! 消失会让菜单长短跳变，用户刚瞄准的项会跑位。构建层把灰显项映射为
//! 不可选 CustomEntry（无 hover/无点击/键盘导航跳过——`ui::ContextMenu`
//! 的 `is_selectable` 语义）。

/// 菜单项。
#[derive(Clone, Debug, PartialEq)]
pub enum MenuItem {
    /// 可点击动作。`id` 是派发键（构建层用它匹配 shell 派发体）。
    Action {
        id: &'static str,
        label: String,
        shortcut: Option<String>,
        enabled: bool,
    },
    /// 分隔线。
    Separator,
    /// 子菜单（点开后横向飞出；现役预设未使用——构建层保守渲染为
    /// 不可选提示行，与旧渲染层同口径，不假装支持）。
    Submenu { label: String, items: Vec<MenuItem> },
}

impl MenuItem {
    /// 动作项构造（缺省可用）。
    pub fn action(id: &'static str, label: impl Into<String>) -> Self {
        MenuItem::Action {
            id,
            label: label.into(),
            shortcut: None,
            enabled: true,
        }
    }

    /// 附加快捷键提示。
    pub fn with_shortcut(mut self, key: &str) -> Self {
        if let MenuItem::Action { shortcut, .. } = &mut self {
            *shortcut = Some(key.to_string());
        }
        self
    }

    /// 从 keymap 表取该动作当前键位标签（#217 单一事实源：菜单标签
    /// 与实际键位同源，用户换绑后跟随；无绑定 → 不标）。
    pub fn with_shortcut_for(mut self, action_id: &str) -> Self {
        if let MenuItem::Action { shortcut, .. } = &mut self {
            *shortcut = crate::keymap::store().label(action_id);
        }
        self
    }

    /// 置灰/启用。
    pub fn with_enabled(mut self, on: bool) -> Self {
        if let MenuItem::Action { enabled, .. } = &mut self {
            *enabled = on;
        }
        self
    }

    /// 占位高度（行）：动作/子菜单 1 行，分隔线按「半行+上下留白」占位。
    /// （消费者 = `menu_geometry::menu_size`——menu_bar 下拉估算用。）
    pub fn height_units(&self) -> f32 {
        match self {
            MenuItem::Separator => crate::menu_geometry::SEPARATOR_UNITS,
            _ => 1.0,
        }
    }

    pub fn is_action(&self) -> bool {
        matches!(self, MenuItem::Action { .. })
    }
}

// ── 预置菜单 ───────────────────────────────────────────────────

/// 编辑器右键菜单。
///
/// `has_selection` 决定剪切/复制/删除是否可用（无选区时灰显，不消失）；
/// `read_only` 决定所有改写类项是否可用。
pub fn editor_menu(has_selection: bool, read_only: bool) -> Vec<MenuItem> {
    let writable = !read_only;
    vec![
        MenuItem::action("ed-cut", "剪切")
            .with_shortcut_for("cut")
            .with_enabled(has_selection && writable),
        MenuItem::action("ed-copy", "复制")
            .with_shortcut_for("copy")
            .with_enabled(has_selection),
        MenuItem::action("ed-paste", "粘贴")
            .with_shortcut_for("paste")
            .with_enabled(writable),
        MenuItem::Separator,
        MenuItem::action("ed-select-all", "全选").with_shortcut_for("select_all"),
        MenuItem::action("ed-find", "查找").with_shortcut_for("open_find"),
        MenuItem::action("ed-replace", "替换")
            .with_shortcut_for("open_replace")
            .with_enabled(writable),
        MenuItem::Separator,
        MenuItem::action("ed-goto", "跳转到行").with_shortcut_for("goto_line"),
        MenuItem::action("ed-fold", "折叠/展开").with_shortcut_for("toggle_fold"),
    ]
}

/// 文件树右键菜单。`is_dir` 决定「新建文件」是建在目录里还是同级。
pub fn file_tree_menu(is_dir: bool) -> Vec<MenuItem> {
    vec![
        MenuItem::action("ft-new-file", "新建文件"),
        MenuItem::action("ft-new-folder", "新建文件夹").with_enabled(is_dir),
        MenuItem::Separator,
        MenuItem::action("ft-open", "打开").with_enabled(!is_dir),
        MenuItem::action("ft-rename", "重命名").with_shortcut("F2"),
        MenuItem::action("ft-delete", "删除").with_shortcut("Delete"),
        MenuItem::Separator,
        MenuItem::action("ft-reveal", "在资源管理器中显示"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_menu_disables_cut_without_selection() {
        let items = editor_menu(false, false);
        let cut = items.iter().find_map(|i| match i {
            MenuItem::Action { id, enabled, .. } if *id == "ed-cut" => Some(*enabled),
            _ => None,
        });
        assert_eq!(cut, Some(false), "无选区时剪切应灰显");
    }

    #[test]
    fn editor_menu_disables_edits_when_read_only() {
        let items = editor_menu(true, true);
        for id in ["ed-cut", "ed-paste", "ed-replace"] {
            let enabled = items.iter().find_map(|i| match i {
                MenuItem::Action {
                    id: got, enabled, ..
                } if *got == id => Some(*enabled),
                _ => None,
            });
            assert_eq!(enabled, Some(false), "只读态 {id} 应灰显");
        }
    }

    #[test]
    fn readonly_still_allows_copy_and_find() {
        let items = editor_menu(true, true);
        for id in ["ed-copy", "ed-select-all", "ed-find", "ed-goto"] {
            let enabled = items.iter().find_map(|i| match i {
                MenuItem::Action {
                    id: got, enabled, ..
                } if *got == id => Some(*enabled),
                _ => None,
            });
            assert_eq!(enabled, Some(true), "只读态 {id} 仍应可用");
        }
    }

    #[test]
    fn file_tree_menu_new_folder_only_for_dirs() {
        for (is_dir, want) in [(true, true), (false, false)] {
            let items = file_tree_menu(is_dir);
            let enabled = items.iter().find_map(|i| match i {
                MenuItem::Action { id, enabled, .. } if *id == "ft-new-folder" => Some(*enabled),
                _ => None,
            });
            assert_eq!(enabled, Some(want), "is_dir={is_dir} 时新建文件夹可用性");
        }
    }

    #[test]
    fn menu_ids_are_unique() {
        for items in [editor_menu(true, false), file_tree_menu(true)] {
            let mut ids: Vec<&'static str> = Vec::new();
            for i in &items {
                if let MenuItem::Action { id, .. } = i {
                    assert!(!ids.contains(id), "菜单项 id 重复：{id}");
                    ids.push(id);
                }
            }
        }
    }

    /// #385 批 1 迁移断言：快捷键标签继续走 keymap 单一事实源（#217），
    /// 用户换绑后菜单标签跟随。
    #[test]
    fn shortcut_labels_come_from_keymap_store() {
        let items = editor_menu(true, false);
        let cut = items.iter().find_map(|i| match i {
            MenuItem::Action { id, shortcut, .. } if *id == "ed-cut" => shortcut.clone(),
            _ => None,
        });
        assert_eq!(
            cut.as_deref(),
            crate::keymap::store().label("cut").as_deref(),
            "剪切快捷键标签应与 keymap store 同源"
        );
    }
}
