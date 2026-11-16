//! Tab actions adapt the identity / SaveIntent model in Zed pane.rs at
//! 1663af412c625417759561c0d15ab5a1668aafa1 (3133-3257); no workspace crate fork.
//! The existing ContextMenu host is retained so deferred focus can be invalidated.
use super::*;

impl ShellApp {
    pub(crate) fn context_menu_generation(&self) -> u64 {
        self.context_menu_generation
    }

    pub(super) fn tab_index_by_id(&self, id: gpui::EntityId) -> Option<usize> {
        self.tabs
            .iter()
            .position(|tab| tab.editor.entity_id() == id)
    }

    pub(crate) fn activate_tab_id(
        &mut self,
        id: gpui::EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.confirm.is_some() || self.settings_open {
            return;
        }
        if let Some(index) = self.tab_index_by_id(id) {
            self.activate_tab(index, window, cx);
        }
    }

    pub(crate) fn request_close_tab_id(
        &mut self,
        id: gpui::EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.tab_index_by_id(id) {
            self.request_close_tab(index, window, cx);
        }
    }

    pub(crate) fn open_tab_context_menu(
        &mut self,
        id: gpui::EntityId,
        pos: gpui::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.confirm.is_some() || self.settings_open {
            return;
        }
        let Some(index) = self.tab_index_by_id(id) else {
            return;
        };
        let snapshot = self.tabs.iter().map(|tab| tab.editor.entity_id()).collect();
        let items = context_menu::tab_menu(
            index,
            self.tabs.len(),
            self.tabs
                .iter()
                .enumerate()
                .any(|(i, _)| !self.tab_is_modified(i, cx).unwrap_or(true)),
            self.tabs[index].path.is_absolute() && !self.tabs[index].path.starts_with("git:"),
            self.workspace.as_ref().is_some_and(|tree| {
                self.tabs[index]
                    .path
                    .strip_prefix(tree.read(cx).root())
                    .is_ok()
            }),
            !self.closed_tabs.is_empty(),
        );
        self.spawn_context_menu(items, pos, MenuOrigin::Tab, None, window, cx);
        self.context_tab_target = Some((id, snapshot));
    }

    pub(super) fn activate_tab_menu_item(
        &mut self,
        action: &'static str,
        target: Option<(gpui::EntityId, Vec<gpui::EntityId>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.confirm.is_some() || self.settings_open {
            return;
        }
        if action == "tab-reopen" {
            self.reopen_closed_tab(window, cx);
            return;
        }
        let Some((id, snapshot)) = target else {
            return;
        };
        let Some(index) = self.tab_index_by_id(id) else {
            return;
        };
        if action == "tab-copy-relative" {
            if let Some(tree) = &self.workspace {
                if let Ok(relative) = self.tabs[index].path.strip_prefix(tree.read(cx).root()) {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                        relative.display().to_string(),
                    ));
                }
            }
            return;
        }
        if action == "tab-copy-path" {
            let path = &self.tabs[index].path;
            if path.is_absolute() {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(path.display().to_string()));
            }
            cx.notify();
            return;
        }
        let Some(pivot) = snapshot.iter().position(|&item| item == id) else {
            return;
        };
        self.close_queue = snapshot
            .into_iter()
            .enumerate()
            .filter_map(|(i, item)| {
                let live = self.tab_index_by_id(item)?;
                let selected = match action {
                    "tab-close" => item == id,
                    "tab-close-others" => item != id,
                    "tab-close-left" => i < pivot,
                    "tab-close-right" => i > pivot,
                    "tab-close-clean" => !self.tab_is_modified(live, cx).unwrap_or(true),
                    "tab-close-all" => true,
                    _ => false,
                };
                selected.then_some((item, action == "tab-close-clean"))
            })
            .collect();
        self.drain_close_queue(window, cx);
    }

    pub(super) fn drain_close_queue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        while self.confirm.is_none() && !self.settings_open {
            let Some((id, clean_only)) = self.close_queue.pop_front() else {
                break;
            };
            let Some(index) = self.tab_index_by_id(id) else {
                continue;
            };
            if clean_only && self.tab_is_modified(index, cx).unwrap_or(true) {
                continue;
            }
            self.request_close_tab_internal(index, window, cx);
        }
    }

    pub(crate) fn reopen_closed_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirm.is_some() || self.settings_open {
            return;
        }
        let Some(path) = self.closed_tabs.last().cloned() else {
            return;
        };
        // Failure remains retryable, and must not fabricate a successful reopen.
        if self.open_document_tab(path, cx).is_some() {
            self.closed_tabs.pop();
            // Multi-format views may hide their editor: focus the mounted display surface.
            self.focus_active(window, cx);
        }
    }
}
