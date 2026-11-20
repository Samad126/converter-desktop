//! The Settings page. Every change is applied and saved immediately.
use crate::theme::*;
use crate::{autostart, button, settings};
use gpui::{div, prelude::*, px, Context, FontWeight, PathPromptOptions, SharedString, Window};
use std::sync::atomic::Ordering;

#[derive(Clone, Copy, PartialEq)]
enum Confirm { Data, App }

pub struct SettingsPage { note: Option<(String, bool)>, confirm: Option<Confirm> }

impl SettingsPage {
    pub fn new() -> Self { Self { note: None, confirm: None } }

    fn say(&mut self, msg: impl Into<String>, ok: bool, cx: &mut Context<Self>) { self.note = Some((msg.into(), ok)); cx.notify(); }

    pub fn set_ring(&mut self, on: bool, cx: &mut Context<Self>) {
        settings::update(|s| s.ring = on);
        settings::RING_ENABLED.store(on, Ordering::Relaxed);
        crate::tray::set_ring(on);
        cx.notify();
    }

    fn set_autostart(&mut self, on: bool, cx: &mut Context<Self>) {
        match autostart::set(on) {
            Ok(()) => { settings::update(|s| s.autostart = on); self.note = None; cx.notify(); }
            Err(e) => self.say(e, false, cx),
        }
    }

    /// Runs the removal, shows its message, then closes the app (it would otherwise re-create its settings file).
    fn run_removal(&mut self, what: Confirm, cx: &mut Context<Self>) {
        self.confirm = None;
        let result = match what { Confirm::Data => crate::uninstall::remove_all_data(), Confirm::App => crate::uninstall::uninstall_app() };
        match result {
            Ok(done) => {
                self.say(done.message, true, cx);
                if done.quit { cx.spawn(async move |_, cx| { cx.background_executor().timer(std::time::Duration::from_millis(2500)).await; std::process::exit(0); }).detach(); }
            }
            Err(e) => self.say(format!("Could not finish: {e}"), false, cx),
        }
    }

    fn choose_folder(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Save results here".into()) });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(dir) = paths.into_iter().next() { settings::update(|s| s.output_dir = Some(dir)); this.update(cx, |_, cx| cx.notify()).ok(); }
            }
        }).detach();
    }
}

fn section(title: &'static str, body: impl IntoElement) -> impl IntoElement {
    div().flex().flex_col().gap_2().p_4().rounded_lg().bg(SURFACE).border_1().border_color(BORDER)
        .child(div().font_weight(FontWeight::SEMIBOLD).child(title)).child(body)
}

impl gpui::Render for SettingsPage {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut s = settings::get();
        // Show what is really on disk: the entry may have been removed outside the app.
        if autostart::SUPPORTED { s.autostart = autostart::is_enabled(); }
        let folder_text: SharedString = match &s.output_dir { Some(d) => d.display().to_string().into(), None => "Next to the original file".into() };
        let toggle = |id: &'static str, on: bool, cx: &mut Context<Self>, f: fn(&mut Self, bool, &mut Context<Self>)| {
            div().flex().gap_1()
                .child(button(SharedString::from(format!("{id}-on")), "On", on).on_click(cx.listener(move |t, _, _, cx| f(t, true, cx))))
                .child(button(SharedString::from(format!("{id}-off")), "Off", !on).on_click(cx.listener(move |t, _, _, cx| f(t, false, cx))))
        };

        div().id("settings-page").flex().flex_col().gap_3().p_5().size_full().overflow_y_scroll()
            .child(div().text_xl().font_weight(FontWeight::BOLD).child("Settings"))
            .child(section("Where results are saved", div().flex().flex_col().gap_2()
                .child(div().text_sm().text_color(MUTED).child(folder_text))
                .child(div().flex().gap_1()
                    .child(button("out-same", "Next to the original", s.output_dir.is_none()).on_click(cx.listener(|t, _, _, cx| { settings::update(|s| s.output_dir = None); cx.notify(); let _ = t; })))
                    .child(button("out-pick", "Choose a folder…", s.output_dir.is_some()).on_click(cx.listener(|t, _, _, cx| t.choose_folder(cx)))))))
            .child(section("If a file with that name already exists", div().flex().flex_col().gap_2()
                .child(div().text_sm().text_color(MUTED).child(if s.overwrite { "The old file is replaced." } else { "Both are kept: the new one gets \"(1)\" added to its name." }))
                .child(div().flex().gap_1()
                    .child(button("ow-keep", "Keep both", !s.overwrite).on_click(cx.listener(|_, _, _, cx| { settings::update(|s| s.overwrite = false); cx.notify(); })))
                    .child(button("ow-replace", "Replace", s.overwrite).on_click(cx.listener(|_, _, _, cx| { settings::update(|s| s.overwrite = true); cx.notify(); }))))))
            .child(section("Quick-convert ring", div().flex().flex_col().gap_2()
                .child(div().text_sm().text_color(MUTED).child(if cfg!(target_os = "linux") { "Hold Ctrl while dragging files to open a ring of formats under the cursor." } else if cfg!(target_os = "macos") { "Hold Option while dragging files to open a ring of formats. Experimental here: needs the Accessibility permission and has not been tested on real Macs yet." } else { "Hold Ctrl while dragging files to open a ring of formats. Experimental on Windows: not tested on real hardware yet, so it is off by default." }))
                .child(toggle("ring", s.ring, cx, Self::set_ring))))
            .child(section("Start with this computer", div().flex().flex_col().gap_2()
                .child(div().text_sm().text_color(MUTED).child(if autostart::SUPPORTED { "Launches hidden when you sign in, so the app (and the ring, where supported) is always ready." } else { "Not available on this platform." }))
                .when(autostart::SUPPORTED, |d| d.child(toggle("auto", s.autostart, cx, Self::set_autostart)))))
            .child(section("Remove Converter", div().flex().flex_col().gap_2()
                .child(div().text_sm().text_color(MUTED).child(if cfg!(windows) { "Remove all data deletes your settings, downloaded engines and temporary files. Uninstall opens the uninstaller, which asks whether to delete them too." } else { "Remove all data deletes your settings, downloaded engines and temporary files. Uninstall does that and removes the app itself. Your converted files are never touched." }))
                .child(match self.confirm {
                    None => div().flex().gap_1()
                        .child(button("rm-data", "Remove all data…", false).on_click(cx.listener(|t, _, _, cx| { t.confirm = Some(Confirm::Data); cx.notify(); })))
                        .child(button("rm-app", "Uninstall Converter…", false).on_click(cx.listener(|t, _, _, cx| { t.confirm = Some(Confirm::App); cx.notify(); }))),
                    Some(what) => div().flex().flex_col().gap_2()
                        .child(div().text_sm().text_color(ERR).child(if what == Confirm::Data { "This cannot be undone and the app will close. Continue?" } else { "The app will be removed and closed. Continue?" }))
                        .child(div().flex().gap_1()
                            .child(button("rm-yes", "Yes, remove", true).on_click(cx.listener(move |t, _, _, cx| t.run_removal(what, cx))))
                            .child(button("rm-no", "Cancel", false).on_click(cx.listener(|t, _, _, cx| { t.confirm = None; cx.notify(); })))),
                })))
            .when_some(self.note.clone(), |d, (msg, ok)| d.child(div().text_sm().text_color(if ok { OK } else { ERR }).child(msg)))
            .child(div().h(px(8.)))
    }
}
