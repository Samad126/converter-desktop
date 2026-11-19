//! Small reusable pieces. gpui 0.2.2 ships no text input, so this is a minimal single-line one:
//! typing, backspace, Ctrl+V, Ctrl+Backspace (delete word). No cursor movement or selection yet.
use crate::theme::*;
use gpui::{div, prelude::*, px, Context, FocusHandle, Focusable, KeyDownEvent, SharedString, Window};

/// Applies one key press to `value`. Returns true when the text changed.
pub fn apply_key(value: &mut String, key: &str, key_char: Option<&str>, ctrl: bool, clipboard: impl FnOnce() -> Option<String>) -> bool {
    match (key, ctrl) {
        ("backspace", true) => {
            let trimmed = value.trim_end().len();
            value.truncate(value.trim_end().rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0).min(trimmed));
            true
        }
        ("backspace", false) => value.pop().is_some(),
        ("v", true) => match clipboard() {
            Some(t) => { value.push_str(t.lines().next().unwrap_or("").trim_end()); true }
            None => false,
        },
        (_, true) => false,
        (_, false) => match key_char {
            Some(c) if !c.is_empty() && !c.chars().any(char::is_control) => { value.push_str(c); true }
            _ => false,
        },
    }
}

pub struct TextField { pub value: String, placeholder: SharedString, secret: bool, focus: FocusHandle }

impl TextField {
    pub fn new(placeholder: impl Into<SharedString>, secret: bool, initial: &str, cx: &mut Context<Self>) -> Self {
        Self { value: initial.to_string(), placeholder: placeholder.into(), secret, focus: cx.focus_handle() }
    }
}

impl Focusable for TextField { fn focus_handle(&self, _: &gpui::App) -> FocusHandle { self.focus.clone() } }

impl gpui::Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let shown: String = if self.secret { "•".repeat(self.value.chars().count()) } else { self.value.clone() };
        let handle = self.focus.clone();
        div().id("text-field").track_focus(&self.focus).h(px(34.)).w_full().px_3().flex().items_center().rounded_md().text_sm()
            .bg(SURFACE).border_1().border_color(if focused { ACCENT } else { BORDER }).cursor_text()
            .on_click(move |_, window, _| window.focus(&handle))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                let ks = &ev.keystroke;
                let ctrl = ks.modifiers.control || ks.modifiers.platform;
                let clip = || cx.read_from_clipboard().and_then(|c| c.text());
                if apply_key(&mut this.value, &ks.key, ks.key_char.as_deref(), ctrl, clip) { cx.notify(); }
            }))
            .child(if self.value.is_empty() && !focused { div().text_color(MUTED).child(self.placeholder.clone()) }
                   else { div().text_color(TEXT).child(format!("{shown}{}", if focused { "▏" } else { "" })) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(v: &mut String, k: &str, c: Option<&str>, ctrl: bool) -> bool { apply_key(v, k, c, ctrl, || Some("pasted\nsecond line".into())) }
    #[test] fn typing_and_backspace() {
        let mut v = String::new();
        assert!(key(&mut v, "a", Some("a"), false)); assert!(key(&mut v, "1", Some("1"), false));
        assert_eq!(v, "a1");
        assert!(key(&mut v, "backspace", None, false)); assert_eq!(v, "a");
        assert!(key(&mut v, "backspace", None, false)); assert!(!key(&mut v, "backspace", None, false), "nothing left to delete");
    }
    #[test] fn unicode_is_one_character() {
        let mut v = "ə".to_string();
        key(&mut v, "backspace", None, false);
        assert_eq!(v, "");
        key(&mut v, "e", Some("ə"), false); assert_eq!(v, "ə");
    }
    #[test] fn control_keys_never_insert_text() {
        let mut v = String::new();
        assert!(!key(&mut v, "enter", Some("\n"), false));
        assert!(!key(&mut v, "tab", Some("\t"), false));
        assert!(!key(&mut v, "a", Some("a"), true), "ctrl+a is not an 'a'");
        assert!(!key(&mut v, "escape", None, false));
        assert_eq!(v, "");
    }
    #[test] fn paste_takes_the_first_line_only() {
        let mut v = "x ".to_string();
        assert!(key(&mut v, "v", None, true)); assert_eq!(v, "x pasted");
    }
    #[test] fn ctrl_backspace_deletes_a_word() {
        let mut v = "1,3-5 7".to_string();
        key(&mut v, "backspace", None, true); assert_eq!(v, "1,3-5 ");
        key(&mut v, "backspace", None, true); assert_eq!(v, "");
    }
}
