//! Small reusable pieces. gpui 0.2.2 ships no text input, so this is a minimal single-line one. Text arrives through
//! the platform input-method channel (`EntityInputHandler`), which is what makes accented letters, dead keys, compose
//! sequences and IMEs work (ə ş ğ ı ç ö ü é …). The cursor is always at the end; there is no selection yet.
use crate::theme::*;
use gpui::{
    div, prelude::*, px, relative, App, Bounds, Context, Element, ElementId, ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable,
    GlobalElementId, InspectorElementId, KeyDownEvent, LayoutId, Pixels, SharedString, Style, UTF16Selection, Window,
};
use std::ops::Range;

// ------------------------------------------------------------------ pure helpers (unit-tested)

/// UTF-16 offset → byte offset in `s` (clamped to the end).
pub fn utf16_to_byte(s: &str, off16: usize) -> usize {
    let (mut b, mut u) = (0, 0);
    for c in s.chars() { if u >= off16 { break; } u += c.len_utf16(); b += c.len_utf8(); }
    b
}

pub fn byte_to_utf16(s: &str, off: usize) -> usize {
    let (mut b, mut u) = (0, 0);
    for c in s.chars() { if b >= off { break; } b += c.len_utf8(); u += c.len_utf16(); }
    u
}

/// Replaces `range` (bytes) of `value` with `text`. Returns the byte range the new text occupies.
pub fn splice(value: &mut String, range: Range<usize>, text: &str) -> Range<usize> {
    let end = range.end.min(value.len());
    let start = range.start.min(end);
    value.replace_range(start..end, text);
    start..start + text.len()
}

/// Backspace / Ctrl+Backspace (word) / Ctrl+V on `value`; everything else is left to the input-method channel.
pub fn apply_key(value: &mut String, key: &str, ctrl: bool, clipboard: impl FnOnce() -> Option<String>) -> bool {
    match (key, ctrl) {
        ("backspace", true) => {
            let t = value.trim_end().len();
            let cut = value[..t].rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0);
            value.truncate(cut);
            true
        }
        ("backspace", false) => value.pop().is_some(),
        ("v", true) => match clipboard() {
            Some(t) => { value.push_str(t.lines().next().unwrap_or("").trim_end()); true }
            None => false,
        },
        _ => false,
    }
}

// ------------------------------------------------------------------ the widget

pub struct TextField { pub value: String, placeholder: SharedString, secret: bool, focus: FocusHandle, marked: Option<Range<usize>> }

impl TextField {
    pub fn new(placeholder: impl Into<SharedString>, secret: bool, initial: &str, cx: &mut Context<Self>) -> Self {
        Self { value: initial.to_string(), placeholder: placeholder.into(), secret, focus: cx.focus_handle(), marked: None }
    }
}

impl Focusable for TextField { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus.clone() } }

impl EntityInputHandler for TextField {
    fn text_for_range(&mut self, range_utf16: Range<usize>, actual: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let r = utf16_to_byte(&self.value, range_utf16.start)..utf16_to_byte(&self.value, range_utf16.end);
        actual.replace(byte_to_utf16(&self.value, r.start)..byte_to_utf16(&self.value, r.end));
        Some(self.value[r].to_string())
    }
    fn selected_text_range(&mut self, _ignore: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let n = byte_to_utf16(&self.value, self.value.len());
        Some(UTF16Selection { range: n..n, reversed: false })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|m| byte_to_utf16(&self.value, m.start)..byte_to_utf16(&self.value, m.end))
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) { self.marked = None; }
    fn replace_text_in_range(&mut self, range_utf16: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        let range = range_utf16.map(|r| utf16_to_byte(&self.value, r.start)..utf16_to_byte(&self.value, r.end))
            .or_else(|| self.marked.clone()).unwrap_or(self.value.len()..self.value.len());
        // Control characters (Enter, Tab) are never text in a single-line field.
        let clean: String = text.chars().filter(|c| !c.is_control()).collect();
        splice(&mut self.value, range, &clean);
        self.marked = None;
        cx.notify();
    }
    fn replace_and_mark_text_in_range(&mut self, range_utf16: Option<Range<usize>>, text: &str, _sel: Option<Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        let range = range_utf16.map(|r| utf16_to_byte(&self.value, r.start)..utf16_to_byte(&self.value, r.end))
            .or_else(|| self.marked.clone()).unwrap_or(self.value.len()..self.value.len());
        let placed = splice(&mut self.value, range, text);
        self.marked = (!text.is_empty()).then_some(placed);
        cx.notify();
    }
    fn bounds_for_range(&mut self, _range: Range<usize>, element_bounds: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        Some(element_bounds) // good enough to place the IME candidate window next to the field
    }
    fn character_index_for_point(&mut self, _p: gpui::Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> { None }
}

/// Invisible element that registers the field's input handler for the frame (what makes the platform send it text).
struct InputCatcher { field: Entity<TextField>, focus: FocusHandle }

impl IntoElement for InputCatcher { type Element = Self; fn into_element(self) -> Self { self } }

impl Element for InputCatcher {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> { None }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> { None }
    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut (), _: &mut Window, _: &mut App) {}
    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), _: &mut (), window: &mut Window, cx: &mut App) {
        window.handle_input(&self.focus, ElementInputHandler::new(bounds, self.field.clone()), cx);
    }
}

impl gpui::Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let shown: String = if self.secret { "•".repeat(self.value.chars().count()) } else { self.value.clone() };
        let handle = self.focus.clone();
        let catcher = InputCatcher { field: cx.entity(), focus: self.focus.clone() };
        div().id("text-field").track_focus(&self.focus).relative().h(px(34.)).w_full().px_3().flex().items_center().rounded_md().text_sm()
            .bg(SURFACE).border_1().border_color(if focused { ACCENT } else { BORDER }).cursor_text()
            .on_click(move |_, window, _| window.focus(&handle))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                let ks = &ev.keystroke;
                let ctrl = ks.modifiers.control || ks.modifiers.platform;
                let clip = || cx.read_from_clipboard().and_then(|c| c.text());
                if apply_key(&mut this.value, &ks.key, ctrl, clip) { this.marked = None; cx.notify(); }
            }))
            .child(div().absolute().size_full().child(catcher))
            .child(if self.value.is_empty() && !focused { div().text_color(MUTED).child(self.placeholder.clone()) }
                   else { div().text_color(TEXT).child(format!("{shown}{}", if focused { "▏" } else { "" })) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(v: &mut String, k: &str, ctrl: bool) -> bool { apply_key(v, k, ctrl, || Some("pasted\nsecond line".into())) }

    #[test] fn backspace_removes_one_character_not_one_byte() {
        let mut v = "aə".to_string();
        assert!(key(&mut v, "backspace", false)); assert_eq!(v, "a");
        assert!(key(&mut v, "backspace", false)); assert!(!key(&mut v, "backspace", false), "nothing left to delete");
    }
    #[test] fn plain_keys_are_left_to_the_input_method_channel() {
        let mut v = String::new();
        for k in ["a", "enter", "tab", "escape", "ə"] { assert!(!key(&mut v, k, false), "{k}"); }
        assert!(!key(&mut v, "a", true), "ctrl+a is not an 'a'");
        assert_eq!(v, "");
    }
    #[test] fn paste_takes_the_first_line_only() {
        let mut v = "x ".to_string();
        assert!(key(&mut v, "v", true)); assert_eq!(v, "x pasted");
    }
    #[test] fn ctrl_backspace_deletes_a_word() {
        let mut v = "1,3-5 7".to_string();
        key(&mut v, "backspace", true); assert_eq!(v, "1,3-5 ");
        key(&mut v, "backspace", true); assert_eq!(v, "");
    }
    #[test] fn utf16_and_byte_offsets_agree_for_azerbaijani_turkish_and_emoji() {
        let s = "əşğı😀a"; // ə ş ğ ı are 2 bytes / 1 unit; the emoji is 4 bytes / 2 units
        assert_eq!(utf16_to_byte(s, 0), 0);
        assert_eq!(utf16_to_byte(s, 4), 8);              // after ə ş ğ ı
        assert_eq!(utf16_to_byte(s, 6), 12);             // after the emoji (2 UTF-16 units)
        assert_eq!(byte_to_utf16(s, 8), 4);
        assert_eq!(byte_to_utf16(s, 12), 6);
        assert_eq!(utf16_to_byte(s, 99), s.len(), "clamped");
        for off in [0, 2, 4, 6, 7] { assert_eq!(byte_to_utf16(s, utf16_to_byte(s, off)), off); }
    }
    #[test] fn splice_inserts_replaces_and_reports_the_new_range() {
        let mut v = "ab".to_string();
        assert_eq!(splice(&mut v, 2..2, "ə"), 2..4); assert_eq!(v, "abə");
        assert_eq!(splice(&mut v, 2..4, "ş"), 2..4); assert_eq!(v, "abş", "an IME replaces its marked text");
        assert_eq!(splice(&mut v, 0..99, ""), 0..0); assert_eq!(v, "", "out-of-range ends are clamped");
    }
}
