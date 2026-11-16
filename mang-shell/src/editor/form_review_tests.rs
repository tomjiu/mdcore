use super::*;
use gpui::Entity;
struct ReviewHost {
    ed: Entity<Editor>,
}
impl Render for ReviewHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.ed.clone())
    }
}
fn setup(cx: &mut gpui::TestAppContext) -> (Entity<Editor>, gpui::WindowHandle<ReviewHost>) {
    let ed = cx.update(|cx| {
        cx.new(|cx| Editor::new(Some("review.md".into()), "one 😀\ntwo\n".into(), cx))
    });
    cx.update(|app| {
        theme::init(theme::LoadThemes::JustBase, app);
        theme::set_theme_settings_provider(Box::new(crate::settings::ShellThemeSettings), app);
    });
    let host = ed.clone();
    let w = cx.add_window(move |window, cx| {
        let focus = host.read(cx).focus_handle().clone();
        window.focus(&focus, cx);
        ReviewHost { ed: host }
    });
    (ed, w)
}
#[gpui::test]
fn form_review_document_composition_does_not_survive_field_transition(
    cx: &mut gpui::TestAppContext,
) {
    let (ed, w) = setup(cx);
    cx.update_window(w.into(), |_, window, cx| {
        ed.update(cx, |ed, cx| {
            ed.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
            ed.open_find(window, cx);
            assert!(
                ed.marked.is_none(),
                "document composition must finish on field entry"
            );
            ed.close_find(window, cx);
            ed.replace_text_in_range(None, "X", window, cx);
            assert_eq!(ed.buffer.to_plain_text(), "niXone 😀\ntwo\n");
        })
    })
    .unwrap();
}
#[gpui::test]
fn form_review_replacement_composition_cannot_reappear_on_reopen(cx: &mut gpui::TestAppContext) {
    let (ed, w) = setup(cx);
    cx.update_window(w.into(), |_, window, cx| {
        ed.update(cx, |ed, cx| {
            ed.open_replace(window, cx);
            ed.focus_form(FormField::Replacement, window, cx);
            ed.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
            ed.close_find(window, cx);
            ed.open_replace(window, cx);
            ed.focus_form(FormField::Replacement, window, cx);
            assert!(
                ed.marked_text_range(window, cx).is_none(),
                "saved field seed cannot retain old composition"
            );
            ed.replace_text_in_range(None, "X", window, cx);
            assert_eq!(ed.find.as_ref().unwrap().replacement(), "niX");
        })
    })
    .unwrap();
}
#[gpui::test]
fn form_review_deferred_focus_preserves_later_field_intent(cx: &mut gpui::TestAppContext) {
    let (ed, w) = setup(cx);
    cx.update_window(w.into(), |_, window, cx| {
        ed.update(cx, |ed, cx| {
            ed.toggle_command_palette(cx);
            ed.goto = Some(GotoState::default());
            ed.focus_form(FormField::Goto, window, cx);
        })
    })
    .unwrap();
    cx.update_window(w.into(), |_, window, cx| {
        assert!(
            ed.read(cx).goto_focus.is_focused(window),
            "deferred palette focus must not overwrite later goto intent"
        );
    })
    .unwrap();
}
