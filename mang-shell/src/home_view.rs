//! Local Chinese home actions using the existing vendored ui::Button.
//! ProjectEmptyState has English/Clone assumptions, so only its layout pattern
//! is adapted here; no workspace/editor/ui_input dependency is introduced.
use gpui::{App, ClickEvent, FocusHandle, RenderOnce, Window};
use ui::prelude::*;

#[derive(Clone, Copy)]
pub(super) enum HomeAction {
    OpenFile,
    OpenFolder,
    NewMarkdown,
}

#[derive(IntoElement)]
pub(super) struct HomeView {
    focus: [FocusHandle; 3],
    on_action: std::rc::Rc<dyn Fn(HomeAction, &ClickEvent, &mut Window, &mut App)>,
}
impl HomeView {
    pub(super) fn new(
        focus: &[FocusHandle; 3],
        on_action: impl Fn(HomeAction, &ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            focus: focus.clone(),
            on_action: std::rc::Rc::new(on_action),
        }
    }
}
impl RenderOnce for HomeView {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        v_flex()
            .id("home-actions")
            .tab_group()
            .w(gpui::px(280.))
            .max_w_full()
            .gap_2()
            .children(
                [
                    ("home-open-file", "打开文件", HomeAction::OpenFile),
                    ("home-open-folder", "打开文件夹", HomeAction::OpenFolder),
                    (
                        "home-new-markdown",
                        "新建 Markdown",
                        HomeAction::NewMarkdown,
                    ),
                ]
                .into_iter()
                .enumerate()
                .map(|(index, (id, label, action))| {
                    let on_action = self.on_action.clone();
                    div().debug_selector(move || id.to_string()).w_full().child(
                        ui::Button::new(id, label)
                            .full_width()
                            .tab_index(index as isize)
                            .track_focus(&self.focus[index])
                            .on_click(move |event, window, cx| {
                                on_action(action, event, window, cx)
                            }),
                    )
                }),
            )
    }
}
