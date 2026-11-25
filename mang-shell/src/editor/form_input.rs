//! Bounded form backend. No Buffer, Core, history, dirty or document read-only state.
//! Editor is only the GPUI host/command adapter; every field owns text, selection,
//! composition and focus. Platform callbacks and keyboard editing share this model.
use super::*;
use std::ops::Deref;

#[derive(Clone, Debug, Default)]
pub(super) struct FormInput {
    text: String,
    // UTF16 offsets; anchor/head preserve reverse selections.
    anchor: usize,
    head: usize,
    pub(super) marked: Option<Range<usize>>,
}
impl Deref for FormInput {
    type Target = str;
    fn deref(&self) -> &str {
        &self.text
    }
}
impl PartialEq<&str> for FormInput {
    fn eq(&self, rhs: &&str) -> bool {
        self.text == *rhs
    }
}
impl std::fmt::Display for FormInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.text.fmt(f)
    }
}
impl FormInput {
    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_owned();
        self.head = self.len_utf16();
        self.anchor = self.head;
        self.marked = None;
    }
    pub fn clear(&mut self) {
        self.set_text("");
    }
    pub fn len_utf16(&self) -> usize {
        self.text.encode_utf16().count()
    }
    // Snap an endpoint inside a surrogate to the scalar boundary, never split UTF8.
    fn boundary(&self, offset: usize, ceil: bool) -> usize {
        let mut u = 0;
        for ch in self.text.chars() {
            let end = u + ch.len_utf16();
            if offset < end {
                return if ceil && offset > u { end } else { u };
            }
            u = end;
        }
        u
    }
    fn byte(&self, offset: usize) -> usize {
        let mut u = 0;
        for (b, ch) in self.text.char_indices() {
            if u >= offset {
                return b;
            }
            u += ch.len_utf16();
        }
        self.text.len()
    }
    pub fn normalized(&self, range: Range<usize>) -> Range<usize> {
        let a = range.start.min(range.end);
        let b = range.start.max(range.end);
        if a == b {
            let p = self.boundary(a, false);
            p..p
        } else {
            self.boundary(a, false)..self.boundary(b, true)
        }
    }
    pub fn selection(&self) -> UTF16Selection {
        UTF16Selection {
            range: self.anchor.min(self.head)..self.anchor.max(self.head),
            reversed: self.head < self.anchor,
        }
    }
    pub fn select(&mut self, range: Range<usize>) {
        let reversed = range.start > range.end;
        let r = self.normalized(range);
        (self.anchor, self.head) = if reversed {
            (r.end, r.start)
        } else {
            (r.start, r.end)
        };
    }
    pub fn selected_text(&self) -> String {
        self.text_range(self.selection().range).0
    }
    pub fn text_range(&self, range: Range<usize>) -> (String, Range<usize>) {
        let r = self.normalized(range);
        (
            self.text[self.byte(r.start)..self.byte(r.end)].to_owned(),
            r,
        )
    }
    pub fn replace(&mut self, range: Option<Range<usize>>, text: &str) -> Range<usize> {
        let r = self.normalized(
            range
                .or_else(|| self.marked.clone())
                .unwrap_or_else(|| self.selection().range),
        );
        // UI fields are single-line. Programmatic replacement seeds may retain newlines.
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        self.text
            .replace_range(self.byte(r.start)..self.byte(r.end), &text);
        let end = r.start + text.encode_utf16().count();
        self.anchor = end;
        self.head = end;
        self.marked = None;
        r.start..end
    }
    pub fn mark(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
    ) {
        let r = self.replace(range, text);
        if let Some(s) = selected {
            self.select(r.start + s.start.min(r.len())..r.start + s.end.min(r.len()));
        }
        self.marked = Some(r);
    }
    #[allow(dead_code)]
    pub fn cursor_col(&self) -> usize {
        self.text[..self.byte(self.head)].chars().count()
    }
    pub fn is_text_key(ks: &gpui::Keystroke) -> bool {
        !ks.modifiers.control
            && !ks.modifiers.platform
            && !ks.modifiers.alt
            && !matches!(
                ks.key.as_str(),
                "enter"
                    | "escape"
                    | "tab"
                    | "backspace"
                    | "delete"
                    | "left"
                    | "right"
                    | "up"
                    | "down"
                    | "home"
                    | "end"
            )
    }
    pub fn key(&mut self, ks: &gpui::Keystroke) -> bool {
        let ctrl = ks.modifiers.control || ks.modifiers.platform;
        let shift = ks.modifiers.shift;
        if ctrl && ks.key.eq_ignore_ascii_case("a") {
            self.select(0..self.len_utf16());
            return false;
        }
        if ctrl || ks.modifiers.alt {
            return false;
        }
        let selection = self.selection().range;
        match ks.key.as_str() {
            "backspace" | "delete" => {
                let r = if !selection.is_empty() {
                    selection
                } else if ks.key == "backspace" {
                    self.boundary(self.head.saturating_sub(1), false)..self.head
                } else {
                    self.head..self.boundary(self.head + 1, true)
                };
                let changed = !r.is_empty();
                self.replace(Some(r), "");
                changed
            }
            "left" | "right" | "home" | "end" => {
                let head = match ks.key.as_str() {
                    "home" => 0,
                    "end" => self.len_utf16(),
                    "left" if !shift && !selection.is_empty() => selection.start,
                    "right" if !shift && !selection.is_empty() => selection.end,
                    "left" => self.boundary(self.head.saturating_sub(1), false),
                    _ => self.boundary(self.head + 1, true),
                };
                self.head = head;
                if !shift {
                    self.anchor = head;
                }
                self.marked = None;
                false
            }
            // Compatibility for pure FindState unit callers only. Production strips
            // key_char; native text always arrives through EntityInputHandler.
            _ => {
                if let Some(text) = &ks.key_char {
                    self.replace(None, text);
                    true
                } else {
                    false
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FormField {
    Find,
    Replacement,
    Palette,
    Goto,
}

impl Editor {
    pub(super) fn form(&self, field: FormField) -> Option<&FormInput> {
        match field {
            FormField::Find | FormField::Replacement => self
                .find
                .as_ref()
                .map(|f| f.input(field == FormField::Replacement)),
            FormField::Palette => self.palette.as_ref().map(|p| &p.query),
            FormField::Goto => self.goto.as_ref().map(|g| &g.input),
        }
    }
    pub(super) fn form_mut(&mut self, field: FormField) -> Option<&mut FormInput> {
        match field {
            FormField::Find | FormField::Replacement => self
                .find
                .as_mut()
                .map(|f| f.input_mut(field == FormField::Replacement)),
            FormField::Palette => self.palette.as_mut().map(|p| &mut p.query),
            FormField::Goto => self.goto.as_mut().map(|g| &mut g.input),
        }
    }
    fn form_focus(&self, f: FormField) -> &FocusHandle {
        match f {
            FormField::Find => &self.find_focus,
            FormField::Replacement => &self.replacement_focus,
            FormField::Palette => &self.palette_focus,
            FormField::Goto => &self.goto_focus,
        }
    }
    pub(super) fn active_form(&self, window: &Window) -> Option<FormField> {
        [
            FormField::Palette,
            FormField::Goto,
            FormField::Replacement,
            FormField::Find,
        ]
        .into_iter()
        .find(|f| self.form(*f).is_some() && self.form_focus(*f).is_focused(window))
    }
    /// Finish preedit as retained text, without carrying its replacement range
    /// into another field, saved seed, or the document. Also cancel older focus requests.
    pub(super) fn finish_input_transition(&mut self) {
        self.form_focus_epoch = self.form_focus_epoch.wrapping_add(1);
        self.marked = None;
        for field in [
            FormField::Find,
            FormField::Replacement,
            FormField::Palette,
            FormField::Goto,
        ] {
            if let Some(input) = self.form_mut(field) {
                input.marked = None;
            }
        }
        self.find_saved.input_mut(false).marked = None;
        self.find_saved.input_mut(true).marked = None;
    }
    pub(super) fn focus_document_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_input_transition();
        window.focus(&self.focus, cx);
    }
    pub(super) fn focus_form(&mut self, f: FormField, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_input_transition();
        if let Some(find) = self.find.as_mut() {
            find.set_focus_replacement(f == FormField::Replacement);
        }
        window.focus(self.form_focus(f), cx);
        cx.notify();
    }
    pub(super) fn defer_form_focus(&mut self, f: FormField, cx: &mut Context<Self>) {
        self.finish_input_transition();
        let epoch = self.form_focus_epoch;
        let entity = cx.weak_entity();
        // Public menu API has no Window argument. Do not focus during render or
        // steal focus from a later command: recheck field existence at effect end.
        let windows = cx.windows();
        cx.defer(move |cx| {
            for w in windows {
                let _ = w.update(cx, |_, window, cx| {
                    let _ = entity.update(cx, |ed, cx| {
                        if ed.form_focus_epoch == epoch
                            && ed.form(f).is_some()
                            && (ed.focus.is_focused(window) || ed.active_form(window).is_some())
                        {
                            ed.focus_form(f, window, cx);
                        }
                    });
                });
            }
        });
    }
    pub(super) fn defer_document_focus(&mut self, cx: &mut Context<Self>) {
        self.finish_input_transition();
        let epoch = self.form_focus_epoch;
        let entity = cx.weak_entity();
        let windows = cx.windows();
        cx.defer(move |cx| {
            for w in windows {
                let _ = w.update(cx, |_, window, cx| {
                    let _ = entity.update(cx, |ed, cx| {
                        if ed.form_focus_epoch == epoch
                            && ed.palette.is_none()
                            && ed.goto.is_none()
                            && ed.palette_focus.is_focused(window)
                        {
                            window.focus(&ed.focus, cx);
                        }
                    });
                });
            }
        });
    }
    pub(super) fn form_changed(&mut self, f: FormField, cx: &mut Context<Self>) {
        match f {
            FormField::Find => {
                if let Some(find) = self.find.as_mut() {
                    find.input_changed(&self.buffer);
                }
            }
            FormField::Palette => {
                if let Some(pal) = self.palette.as_mut() {
                    Self::palette_refill(pal, &self.palette_recent);
                    pal.selected = 0;
                }
            }
            FormField::Goto => {
                if let Some(goto) = self.goto.as_mut() {
                    goto.hint = goto_hint(&goto.input, self.buffer.line_count());
                }
            }
            FormField::Replacement => {}
        }
        cx.notify();
    }
    pub(super) fn form_clipboard_key(
        &mut self,
        ks: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(f) = self.active_form(window) else {
            return false;
        };
        if !(ks.modifiers.control || ks.modifiers.platform) || ks.modifiers.alt {
            return false;
        }
        match ks.key.to_lowercase().as_str() {
            "c" | "x" => {
                let text = self.form(f).unwrap().selected_text();
                if !text.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    if ks.key.eq_ignore_ascii_case("x") {
                        self.form_mut(f).unwrap().replace(None, "");
                        self.form_changed(f, cx);
                    }
                }
            }
            "v" => {
                if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    self.form_mut(f).unwrap().replace(None, &text);
                    self.form_changed(f, cx);
                }
            }
            _ => return false,
        }
        cx.stop_propagation();
        true
    }
    pub(super) fn form_range_bounds(
        &self,
        f: FormField,
        range: Range<usize>,
        window: &mut Window,
    ) -> Option<Bounds<Pixels>> {
        let bounds = self
            .form_bounds
            .borrow()
            .iter()
            .find(|(field, _)| *field == f)?
            .1;
        let input = self.form(f)?;
        let r = input.normalized(range);
        let style = editor_hit_text_style(px(14.));
        let start = shape_text_width(window, &input.text_range(0..r.start).0, &style);
        let end = shape_text_width(window, &input.text_range(0..r.end).0, &style);
        Some(Bounds::new(
            point(bounds.origin.x + start, bounds.origin.y + px(8.)),
            size((end - start).max(px(1.)), px(20.)),
        ))
    }
    pub(super) fn form_index_for_point(
        &self,
        f: FormField,
        p: gpui::Point<Pixels>,
        window: &mut Window,
    ) -> Option<usize> {
        let bounds = self
            .form_bounds
            .borrow()
            .iter()
            .find(|(field, _)| *field == f)?
            .1;
        let input = self.form(f)?;
        let style = editor_hit_text_style(px(14.));
        if !bounds.contains(&p) {
            return None;
        }
        let x = p.x - bounds.origin.x;
        let mut offset = 0;
        let mut prev = px(0.);
        for ch in input.chars() {
            let next = offset + ch.len_utf16();
            let width = shape_text_width(window, &input.text_range(0..next).0, &style);
            if x < (prev + width) / 2. {
                return Some(offset);
            }
            prev = width;
            offset = next;
        }
        Some(offset)
    }
    pub(super) fn render_form_input(
        &self,
        f: FormField,
        id: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let input = self.form(f).cloned().unwrap_or_default();
        let focus = self.form_focus(f).clone();
        let focused = focus.is_focused(window);
        let colors = cx.theme().colors().clone();
        let style = find_text_style(&colors);
        let sel = input.selection().range;
        let a = shape_text_width(window, &input.text_range(0..sel.start).0, &style);
        let b = shape_text_width(window, &input.text_range(0..sel.end).0, &style);
        let caret = shape_text_width(window, &input.text_range(0..input.head).0, &style);
        let entity = cx.entity();
        let paint_entity = entity.clone();
        div()
            .id(id)
            .track_focus(&focus)
            .relative()
            .flex_1()
            .min_w_0()
            .h(px(36.))
            .overflow_hidden()
            .on_key_down(cx.listener(move |ed, event: &KeyDownEvent, window, cx| {
                if f == FormField::Find || f == FormField::Replacement {
                    ed.dispatch_find_key(event, window, cx);
                } else {
                    ed.dispatch_key(event, window, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |ed, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    ed.focus_form(f, window, cx);
                    if let Some(offset) = ed.form_index_for_point(f, event.position, window) {
                        let input = ed.form_mut(f).unwrap();
                        let anchor = if event.modifiers.shift {
                            input.anchor
                        } else {
                            offset
                        };
                        input.select(anchor..offset);
                    }
                    cx.notify();
                }),
            )
            .on_drag(EditorDrag, |_, _, _, cx| cx.new(|_| DragGhost))
            .on_drag_move::<EditorDrag>(cx.listener(
                move |ed, event: &DragMoveEvent<EditorDrag>, window, cx| {
                    cx.stop_propagation();
                    if let Some(offset) = ed.form_index_for_point(f, event.event.position, window) {
                        let input = ed.form_mut(f).unwrap();
                        input.select(input.anchor..offset);
                        cx.notify();
                    }
                },
            ))
            .when(focused && !sel.is_empty(), |el| {
                el.child(
                    div()
                        .absolute()
                        .top(px(8.))
                        .left(a)
                        .w(b - a)
                        .h(px(20.))
                        .bg(colors.element_active),
                )
            })
            .child(
                div()
                    .absolute()
                    .top(px(8.))
                    .text_size(px(14.))
                    .line_height(px(20.))
                    .font_family(EDITOR_FONT)
                    .text_color(colors.text)
                    .child(input.text),
            )
            .when(focused, |el| {
                el.child(
                    div()
                        .absolute()
                        .top(px(8.))
                        .left(caret)
                        .w(px(1.))
                        .h(px(20.))
                        .bg(colors.text),
                )
            })
            .child(
                canvas(
                    move |bounds, _, cx| {
                        entity.update(cx, |ed, _| {
                            let mut all = ed.form_bounds.borrow_mut();
                            all.retain(|(field, _)| *field != f);
                            all.push((f, bounds));
                        });
                        bounds
                    },
                    move |bounds, _, window, cx| {
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, paint_entity.clone()),
                            cx,
                        );
                    },
                )
                .absolute()
                .size_full(),
            )
    }
}
