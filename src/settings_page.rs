//! The Settings page. Every change is applied and saved immediately.
use crate::theme::*;
use crate::widgets::{segment, segments, switch};
use crate::{autostart, button, page_body, page_header, settings};
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
        self.say("Removing… this can take a minute if programs have to be uninstalled.", true, cx);
        // Uninstalling packages can take a while: keep it off the UI thread.
        let handle = crate::runtime::get().spawn_blocking(move || match what { Confirm::Data => crate::uninstall::remove_all_data(), Confirm::App => crate::uninstall::uninstall_app() });
        if let Ok(mut s) = crate::uninstall::STATUS.lock() { s.clear(); }
        cx.spawn(async move |this, cx| {
            // Show which step is running; uninstalling a big program can take minutes.
            while !handle.is_finished() {
                cx.background_executor().timer(std::time::Duration::from_millis(400)).await;
                let step = crate::uninstall::STATUS.lock().map(|s| s.clone()).unwrap_or_default();
                if !step.is_empty() { this.update(cx, |t, cx| t.say(step, true, cx)).ok(); }
            }
            let result = handle.await.map_err(|e| e.to_string()).and_then(|r| r);
            let quit = this.update(cx, |t, cx| match result {
                Ok(done) => { t.say(done.message, true, cx); done.quit }
                Err(e) => { t.say(format!("Could not finish: {e}"), false, cx); false }
            }).unwrap_or(false);
            if quit { cx.background_executor().timer(std::time::Duration::from_millis(2500)).await; std::process::exit(0); }
        }).detach();
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

/// One setting: name and explanation on the left, its control on the right.
fn row(title: &'static str, desc: impl Into<SharedString>, control: impl IntoElement) -> gpui::Div {
    div().flex().justify_between().items_center().gap_6().p_4().border_b_1().border_color(BORDER)
        .child(div().flex().flex_col().gap_1().flex_1().min_w(px(0.)).child(div().font_weight(FontWeight::SEMIBOLD).child(title)).child(div().text_sm().text_color(MUTED).child(desc.into())))
        .child(div().flex_none().child(control))
}

impl gpui::Render for SettingsPage {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut s = settings::get();
        // Show what is really on disk: the entry may have been removed outside the app.
        if autostart::SUPPORTED { s.autostart = autostart::is_enabled(); }
        let folder_text: SharedString = match &s.output_dir { Some(d) => d.display().to_string().into(), None => "Next to the original file".into() };
        let toggle = |id: &'static str, on: bool, cx: &mut Context<Self>, f: fn(&mut Self, bool, &mut Context<Self>)| {
            switch(id, on).on_click(cx.listener(move |t, _, _, cx| f(t, !on, cx)))
        };
        let list = div().flex().flex_col().rounded(px(10.)).bg(SURFACE).border_1().border_color(BORDER).overflow_hidden().pb(px(-1.))
            .child(row("Save results", folder_text, segments()
                .child(segment("out-same", "Next to the original", s.output_dir.is_none()).on_click(cx.listener(|t, _, _, cx| { settings::update(|s| s.output_dir = None); cx.notify(); let _ = t; })))
                .child(segment("out-pick", "Choose a folder…", s.output_dir.is_some()).on_click(cx.listener(|t, _, _, cx| t.choose_folder(cx))))))
            .child(row("If a file with that name exists", if s.overwrite { "The old file is replaced." } else { "Both are kept: the new one gets \"(1)\" added to its name." }, segments()
                .child(segment("ow-keep", "Keep both", !s.overwrite).on_click(cx.listener(|_, _, _, cx| { settings::update(|s| s.overwrite = false); cx.notify(); })))
                .child(segment("ow-replace", "Replace", s.overwrite).on_click(cx.listener(|_, _, _, cx| { settings::update(|s| s.overwrite = true); cx.notify(); })))))
            .child(row("Quick-convert ring", if cfg!(target_os = "linux") { "Hold Ctrl while dragging files to open a ring of formats under the cursor." } else if cfg!(target_os = "macos") { "Hold Option while dragging files to open a ring of formats. Experimental here: needs the Accessibility permission and has not been tested on real Macs yet." } else { "Hold Ctrl while dragging files to open a ring of formats. Experimental on Windows: not tested on real hardware yet, so it is off by default." },
                toggle("ring", s.ring, cx, Self::set_ring)))
            .child(row("Start with this computer", if autostart::SUPPORTED { "Launches hidden when you sign in, so the app (and the ring, where supported) is always ready." } else { "Not available on this platform." },
                div().when(autostart::SUPPORTED, |d| d.child(toggle("auto", s.autostart, cx, Self::set_autostart)))));
        let danger = div().flex().flex_col().gap_3().p_4().rounded(px(10.)).border_1().border_color(tint(ERR, 0.35))
            .child(div().flex().flex_col().gap_1().child(div().font_weight(FontWeight::SEMIBOLD).child("Remove Converter"))
                .child(div().text_sm().text_color(MUTED).child(if cfg!(windows) { "Remove all data deletes your settings, downloaded engines and temporary files. Uninstall opens the uninstaller, which asks whether to delete them too." } else { "Remove all data deletes your settings, downloaded engines and temporary files. Uninstall does that and removes the app itself. Your converted files are never touched." })))
            .child(match self.confirm {
                None => div().flex().gap_2()
                    .child(button("rm-data", "Remove all data…", false).text_color(ERR).on_click(cx.listener(|t, _, _, cx| { t.confirm = Some(Confirm::Data); cx.notify(); })))
                    .child(button("rm-app", "Uninstall Converter…", false).text_color(ERR).on_click(cx.listener(|t, _, _, cx| { t.confirm = Some(Confirm::App); cx.notify(); }))),
                Some(what) => div().flex().flex_col().gap_2()
                    .child(div().text_sm().text_color(ERR).child(format!("{} {}", if what == Confirm::Data { "This cannot be undone and the app will close." } else { "The app will be removed and closed." }, {
                        let names: Vec<_> = crate::uninstall::system_tools().into_iter().map(|(n, _)| n).collect();
                        if names.is_empty() { "Continue?".to_string() } else { format!("It will also uninstall these programs from this computer, even if you installed them yourself: {}. Continue?", names.join(", ")) }
                    })))
                    .child(div().flex().gap_2()
                        .child(button("rm-yes", "Yes, remove", true).bg(ERR).text_color(ON_ACCENT).on_click(cx.listener(move |t, _, _, cx| t.run_removal(what, cx))))
                        .child(button("rm-no", "Cancel", false).on_click(cx.listener(|t, _, _, cx| { t.confirm = None; cx.notify(); })))),
            });
        page_body("settings-page", div().flex().flex_col().gap_4()
            .child(page_header("Settings", "Changes apply and save immediately."))
            .child(list).child(danger)
            .when_some(self.note.clone(), |d, (msg, ok)| d.child(div().text_sm().text_color(if ok { OK } else { ERR }).child(msg))))
    }
}
