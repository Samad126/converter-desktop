// Release builds on Windows are GUI apps: without this a console window opens behind the app.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

//! Converter desktop app (gpui). Screens: Convert (drop files, pick a target) and Engines.
mod editor_model;
mod elevation;
mod gesture;
mod jobs;
mod page_editor;
mod pdf_extra;
mod pdf_page;
mod pdf_tools;
mod ring;
mod autostart;
mod runtime;
mod settings;
mod settings_page;
mod sysinstall;
mod theme;
mod uninstall;
mod tray;
mod widgets;

use converter_core::{
    engines::{self, manager},
    formats,
    progress::{Progress, ProgressFn},
    Config, ErrorCode,
};
use gpui::{
    div, prelude::*, px, relative, App, Application, Bounds, Context, ExternalPaths, FontWeight, SharedString, TitlebarOptions, Window,
    WindowBounds, WindowOptions,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use theme::*;

/// Everything that can ask the running app to do something: the global gesture and the tray menu.
pub enum AppEvent { Gesture(gesture::Gesture), OpenMain, SetRing(bool), Quit }

struct MainWindow(Option<gpui::AnyWindowHandle>);
impl gpui::Global for MainWindow {}

/// gpui's X11 backend ends the app when the last window closes. A 1×1 window that is never shown keeps it alive
/// while the tray is the user's way back in.
struct KeepAlive;
impl Render for KeepAlive { fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { div() } }

fn open_keepalive(cx: &mut App) {
    let bounds = Bounds { origin: gpui::point(px(-200.), px(-200.)), size: gpui::size(px(1.), px(1.)) };
    let _ = cx.open_window(WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)), titlebar: None, focus: false, show: false, is_movable: false, is_resizable: false,
        kind: gpui::WindowKind::PopUp, window_background: gpui::WindowBackgroundAppearance::Transparent, ..Default::default()
    }, |_, cx| cx.new(|_| KeepAlive));
}

/// Shows the main window: raises it if it is already open, otherwise creates it.
fn open_main(cx: &mut App, files: Vec<PathBuf>) {
    if let Some(h) = cx.try_global::<MainWindow>().and_then(|m| m.0) {
        // gpui can keep listing a window the user already closed; only trust it if raising it actually works.
        if cx.windows().contains(&h) && h.update(cx, |_, window, _| window.activate_window()).is_ok() { return; }
    }
    let bounds = Bounds::centered(None, gpui::size(px(980.), px(660.)), cx);
    let opened = cx.open_window(WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions { title: Some("Converter".into()), ..Default::default() }),
        ..Default::default()
    }, move |_, cx| cx.new(|cx| { let mut c = Converter::new(cx); c.add_files(&files); c }));
    if let Ok(h) = opened { cx.set_global(MainWindow(Some(h.into()))); cx.activate(true); }
}

#[derive(Clone, PartialEq)]
enum Status { Idle, Running(Option<f32>), Done(PathBuf), Failed(String, Option<String>) }

struct Job { path: PathBuf, ext: String, status: Status }

#[derive(Clone, Copy, PartialEq)]
enum Page { Convert, Pdf, Engines, Settings }

struct EngineRow { name: &'static str, used_for: &'static str, state: EngineState }

#[derive(Clone, PartialEq)]
/// `Installed(label, removable)`: removable = we know how to uninstall it (its own folder, or a winget/brew/apt package).
/// `Failed(message, was_removal)`.
enum EngineState { Installed(String, bool), Available(String, u64), Missing(&'static str), Installing(f32, &'static str), Failed(String, bool) }

struct Converter { page: Page, jobs: Vec<Job>, engines: Vec<EngineRow>, cfg: Arc<Config>, pdf: gpui::Entity<pdf_page::PdfPage>, settings: gpui::Entity<settings_page::SettingsPage> }

impl Converter {
    fn new(cx: &mut Context<Self>) -> Self {
        let cfg = Arc::new(Config::default());
        let pdf = cx.new(pdf_page::PdfPage::new);
        // "Install qpdf" in a PDF tool jumps to the Engines page and starts the download.
        cx.subscribe(&pdf, |this, _, ev: &pdf_page::PdfEvent, cx| { let pdf_page::PdfEvent::Install(name) = ev; this.page = Page::Engines; this.install(name, cx); }).detach();
        let settings = cx.new(|_| settings_page::SettingsPage::new());
        let mut s = Self { page: Page::Convert, jobs: vec![], engines: vec![], cfg, pdf, settings };
        s.refresh_engines(true);
        s
    }

    /// Re-reads what is installed. Rows with a download in flight keep their progress state
    /// unless `keep_installing` is false (used when that download has just finished).
    fn refresh_engines(&mut self, keep_installing: bool) {
        let installing: Vec<_> = self.engines.iter().filter(|_| keep_installing).filter(|e| matches!(e.state, EngineState::Installing(..))).map(|e| e.name).collect();
        self.engines = manager::catalog(&self.cfg).into_iter().map(|c| EngineRow {
            name: c.name, used_for: c.used_for,
            state: if installing.contains(&c.name) { EngineState::Installing(0.0, "Starting") } else {
                match (c.installed_path, c.downloadable) {
                    (Some(p), _) => {
                        let own = p.starts_with(&self.cfg.engines_dir);
                        let label = if own { "installed by Converter".to_string() } else { format!("found on this computer ({})", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()) };
                        EngineState::Installed(label, own || sysinstall::uninstall_plan(c.hint).is_some())
                    }
                    (None, Some(v)) => EngineState::Available(v, c.size.unwrap_or(0)),
                    (None, None) => EngineState::Missing(c.hint),
                } },
        }).collect();
    }

    fn add_files(&mut self, paths: &[PathBuf]) {
        for p in paths.iter().filter(|p| p.is_file()) {
            self.jobs.push(Job { ext: formats::extension_of(p), path: p.clone(), status: Status::Idle });
        }
    }

    fn missing_engine(&self, ext: &str, target: &str) -> Option<&'static str> { jobs::missing_engine(&self.cfg, ext, target) }

    fn convert(&mut self, idx: usize, target: String, cx: &mut Context<Self>) {
        let (path, ext) = { let j = &self.jobs[idx]; (j.path.clone(), j.ext.clone()) };
        if let Some(missing) = self.missing_engine(&ext, &target) {
            self.jobs[idx].status = Status::Failed(format!("This needs {missing}, which is not installed yet."), Some(missing.to_string()));
            cx.notify();
            return;
        }
        self.jobs[idx].status = Status::Running(None);
        cx.notify();
        let frac = Arc::new(Mutex::new(None::<f32>));
        let f2 = frac.clone();
        let progress: ProgressFn = Arc::new(move |p| if let Progress::Fraction(f) = p { *f2.lock().unwrap() = Some(f); });
        let handle = jobs::spawn(path, target, progress);
        // Poll the shared fraction while the job runs, then deliver the result.
        cx.spawn(async move |this, cx| {
            while !handle.is_finished() {
                cx.background_executor().timer(std::time::Duration::from_millis(150)).await;
                let f = *frac.lock().unwrap();
                this.update(cx, |t, cx| { if let Some(Status::Running(cur)) = t.jobs.get_mut(idx).map(|j| &mut j.status) { *cur = f; cx.notify(); } }).ok();
            }
            let result = handle.await;
            this.update(cx, |t, cx| {
                t.jobs[idx].status = match result {
                    Ok(Ok(p)) => Status::Done(p),
                    Ok(Err(e)) => Status::Failed(e.message.clone(), (e.code == ErrorCode::EEngineMissing).then(|| e.engine.clone()).flatten()),
                    Err(e) => Status::Failed(format!("The conversion stopped unexpectedly: {e}"), None),
                };
                cx.notify();
            }).ok();
        }).detach();
    }

    fn install(&mut self, name: &'static str, cx: &mut Context<Self>) {
        // No portable download for this tool here: run the system package manager's command instead of asking the user to type it.
        let hint = manager::catalog(&self.cfg).into_iter().find(|c| c.name == name).filter(|c| c.downloadable.is_none()).map(|c| c.hint);
        if let Some(plan) = hint.and_then(sysinstall::plan) { return self.install_with_system(name, plan, cx); }
        if let Some(r) = self.engines.iter_mut().find(|e| e.name == name) { r.state = EngineState::Installing(0.0, "Starting"); }
        cx.notify();
        let state = Arc::new(Mutex::new((0.0f32, "Downloading")));
        let s2 = state.clone();
        let progress: ProgressFn = Arc::new(move |p| { let mut g = s2.lock().unwrap(); match p {
            Progress::Fraction(f) => *g = (f, "Downloading"), Progress::Stage(s) => *g = (1.0, s), _ => {} } });
        let cfg = self.cfg.clone();
        let handle = runtime::get().spawn(async move { manager::install(&cfg, name, progress).await });
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(150)).await;
                let (f, s) = *state.lock().unwrap();
                if handle.is_finished() { break; }
                this.update(cx, |t, cx| { if let Some(r) = t.engines.iter_mut().find(|e| e.name == name) { r.state = EngineState::Installing(f, s); } cx.notify(); }).ok();
            }
            let result = handle.await;
            this.update(cx, |t, cx| {
                t.refresh_engines(false);
                if let Ok(Err(e)) = &result { if let Some(r) = t.engines.iter_mut().find(|e| e.name == name) { r.state = EngineState::Failed(e.message.clone(), false); } }
                cx.notify();
            }).ok();
        }).detach();
    }
}

impl Converter {
    /// Runs `winget` / `brew` / `apt` for a tool; the row shows an indeterminate bar (fraction < 0) until it exits.
    fn install_with_system(&mut self, name: &'static str, plan: sysinstall::Plan, cx: &mut Context<Self>) {
        let stage: &'static str = match sysinstall::manager_name(&plan) { "winget" => "Installing with winget", "brew" => "Installing with Homebrew", _ => "Installing with apt" };
        if let Some(r) = self.engines.iter_mut().find(|e| e.name == name) { r.state = EngineState::Installing(-1.0, stage); }
        cx.notify();
        let handle = runtime::get().spawn_blocking(move || sysinstall::run(&plan));
        cx.spawn(async move |this, cx| {
            let result = handle.await;
            this.update(cx, |t, cx| {
                let failure = match result {
                    Ok(Ok(_)) => None,
                    Ok(Err(m)) => Some(m),
                    Err(e) => Some(e.to_string()),
                };
                t.refresh_engines(false);
                // Installed but not found yet (a fresh PATH entry): say so rather than showing "Not installed" with no explanation.
                let still_missing = t.engines.iter().find(|e| e.name == name).map(|e| !matches!(e.state, EngineState::Installed(..))).unwrap_or(false);
                let msg = failure.or_else(|| still_missing.then(|| "Installed, but Converter cannot find it yet. Restart Converter and it should appear.".to_string()));
                if let (Some(m), Some(r)) = (msg, t.engines.iter_mut().find(|e| e.name == name)) { r.state = EngineState::Failed(m, false); }
                cx.notify();
            }).ok();
        }).detach();
    }

    /// Uninstalls an engine: Converter's own downloaded copy, or the winget/brew/apt package of a system-wide one.
    fn remove_engine(&mut self, name: &'static str, cx: &mut Context<Self>) {
        let own = manager::catalog(&self.cfg).into_iter().find(|c| c.name == name).and_then(|c| c.installed_path).is_some_and(|p| p.starts_with(&self.cfg.engines_dir));
        if let Some(r) = self.engines.iter_mut().find(|e| e.name == name) { r.state = EngineState::Installing(-1.0, "Removing"); }
        cx.notify();
        let cfg = self.cfg.clone();
        let hint = converter_core::engines::tool(name).map(|t| t.hint());
        let handle = runtime::get().spawn(async move {
            if own { return manager::remove(&cfg, name).await.map_err(|e| e.message); }
            let Some(plan) = hint.and_then(sysinstall::uninstall_plan) else { return Err("There is no automatic uninstall for this tool.".to_string()) };
            tokio::task::spawn_blocking(move || sysinstall::run(&plan)).await.map_err(|e| e.to_string())??;
            // Some uninstallers (Tesseract's, for one) hand off to a helper and exit before the files are gone: wait for them.
            if let Some(tool) = converter_core::engines::tool(name) {
                for _ in 0..24 {
                    if converter_core::engines::resolve(tool, &cfg, None).is_none() { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
            Ok(())
        });
        cx.spawn(async move |this, cx| {
            let result = handle.await.map_err(|e| e.to_string()).and_then(|r| r);
            this.update(cx, |t, cx| {
                t.refresh_engines(false);
                // The package manager may report "nothing to do" for a program it did not install: say so instead of looking like nothing happened.
                let still_there = t.engines.iter().find(|e| e.name == name).is_some_and(|e| matches!(e.state, EngineState::Installed(..)));
                let msg = match result { Err(m) => Some(m), Ok(()) if still_there => Some("It is still installed. Remove it from Settings > Apps (Windows) or with your package manager.".to_string()), Ok(()) => None };
                if let (Some(m), Some(r)) = (msg, t.engines.iter_mut().find(|e| e.name == name)) { r.state = EngineState::Failed(m, true); }
                cx.notify();
            }).ok();
        }).detach();
    }
}

pub fn button(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, primary: bool) -> gpui::Stateful<gpui::Div> {
    div().id(id).flex().items_center().h(px(30.)).px_3().rounded(px(6.)).text_sm().cursor_pointer().child(label.into())
        .bg(if primary { ACCENT } else { SURFACE_HI }).text_color(if primary { ON_ACCENT } else { TEXT })
        .when(primary, |d| d.font_weight(FontWeight::SEMIBOLD))
        .hover(move |s| s.bg(if primary { ACCENT_HI } else { BORDER }))
        .active(|s| s.opacity(0.8))
}

/// A format the user can pick: tinted by its file family.
fn format_chip(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, ext: &str) -> gpui::Stateful<gpui::Div> {
    let c = family_color(ext);
    div().id(id).flex().items_center().h(px(28.)).px_3().rounded(px(6.)).text_sm().cursor_pointer().child(label.into())
        .bg(tint(c, 0.12)).text_color(c).border_1().border_color(tint(c, 0.25))
        .hover(move |s| s.bg(tint(c, 0.24)).border_color(tint(c, 0.6)))
        .active(|s| s.opacity(0.8))
}

fn progress_bar(frac: Option<f32>) -> impl IntoElement {
    div().h(px(4.)).w_full().rounded_full().bg(BORDER).child(div().h_full().rounded_full().bg(ACCENT).w(relative(frac.unwrap_or(0.35).clamp(0.02, 1.0))))
}

/// Page header used by every top-level page: title plus one muted sentence.
pub fn page_header(title: impl Into<SharedString>, sub: impl Into<SharedString>) -> impl IntoElement {
    div().flex().flex_col().gap_1().pb_2()
        .child(div().text_size(px(22.)).font_weight(FontWeight::SEMIBOLD).child(title.into()))
        .child(div().text_sm().text_color(MUTED).child(sub.into()))
}

/// Scrollable page body: content stays a readable width and is centred in wide windows.
pub fn page_body(id: &'static str, content: impl IntoElement) -> gpui::Stateful<gpui::Div> {
    div().id(id).size_full().flex().items_start().overflow_y_scroll().px_8().py_7()
        .child(div().flex().flex_col().gap_4().flex_none().w_full().max_w(px(760.)).mx_auto().child(content))
}

impl Render for Converter {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let nav = |page: Page, label: &'static str, cx: &mut Context<Self>| {
            let on = self.page == page;
            div().id(label).flex().items_center().gap_2().h(px(34.)).px_3().rounded(px(6.)).cursor_pointer().text_sm().text_color(if on { TEXT } else { MUTED }).child(div().w(px(3.)).h(px(16.)).rounded_full().bg(if on { ACCENT } else { tint(ACCENT, 0.0) })).child(label)
                .when(on, |d| d.bg(SURFACE_HI).font_weight(FontWeight::SEMIBOLD))
                .hover(|s| s.bg(SURFACE_HI).text_color(TEXT))
                .on_click(cx.listener(move |t, _, _, cx| { t.page = page; if page == Page::Engines { t.refresh_engines(true); } cx.notify(); }))
        };
        let mark = div().flex().items_center().justify_center().size(px(26.)).rounded(px(7.)).bg(ACCENT).text_color(ON_ACCENT).font_weight(FontWeight::BOLD).child("→");
        div().flex().size_full().bg(BG).text_color(TEXT).font_family(FONT)
            .child(div().flex().flex_col().gap_1().w(px(200.)).py_4().px_3().bg(SURFACE).border_r_1().border_color(BORDER)
                .child(div().flex().items_center().gap_2().px_3().pb_4().child(mark).child(div().text_size(px(16.)).font_weight(FontWeight::BOLD).child("Converter")))
                .child(nav(Page::Convert, "Convert", cx)).child(nav(Page::Pdf, "PDF tools", cx)).child(nav(Page::Engines, "Engines", cx)).child(div().flex_1()).child(nav(Page::Settings, "Settings", cx)))
            .child(div().flex_1().h_full().overflow_hidden().child(match self.page { Page::Convert => self.convert_page(cx).into_any_element(), Page::Pdf => self.pdf.clone().into_any_element(), Page::Settings => self.settings.clone().into_any_element(), Page::Engines => self.engines_page(cx).into_any_element() }))
    }
}

/// The big target: highlights while files are dragged over it.
pub fn drop_zone(id: &'static str, title: &'static str, hint: impl Into<SharedString>, cx: &mut App) -> gpui::Stateful<gpui::Div> {
    let _ = cx;
    div().id(id).flex().flex_col().items_center().justify_center().gap_1().py_8().rounded(px(12.)).border_2().border_color(BORDER).bg(SURFACE)
        .drag_over::<ExternalPaths>(|s, _, _, _| s.border_color(ACCENT).bg(tint(ACCENT, 0.10)))
        .child(div().flex().items_center().gap_2().pb_2()
            .child(widgets::ext_tile("docx")).child(div().text_color(MUTED).child("→")).child(widgets::ext_tile("pdf")))
        .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(title))
        .child(div().text_sm().text_color(MUTED).child(hint.into()))
}

impl Converter {
    fn browse(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions { files: true, directories: false, multiple: true, prompt: None });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await { this.update(cx, |t, cx| { t.add_files(&paths); cx.notify(); }).ok(); }
        }).detach();
    }

    fn convert_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let hint = match settings::get().output_dir { Some(d) => format!("Results are saved in {}.", d.display()), None => "Results are saved next to the original.".to_string() };
        let drop_zone = drop_zone("drop", "Drop files to convert", hint, cx)
            .on_drop(cx.listener(|t, paths: &ExternalPaths, _, cx| { t.add_files(paths.paths()); cx.notify(); }))
            .child(div().pt_3().child(button("browse", "Browse files", true).on_click(cx.listener(|t, _, _, cx| t.browse(cx)))));
        let n = self.jobs.len();
        let rows = self.jobs.iter().enumerate().map(|(i, j)| {
            let name = j.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let body = match &j.status {
                Status::Idle => {
                    let targets = jobs::targets_for(&j.ext);
                    if targets.is_empty() { div().text_sm().text_color(MUTED).child(format!("{} files can't be converted.", if j.ext.is_empty() { "These".into() } else { j.ext.clone() })).into_any_element() }
                    else { div().flex().flex_col().gap_2().child(div().text_sm().text_color(MUTED).child("Convert to"))
                        .child(div().flex().flex_wrap().gap_2().children(targets.into_iter().map(|(id, label)| {
                        let missing = self.missing_engine(&j.ext, &id).is_some();
                        let tid = id.clone();
                        format_chip(SharedString::from(format!("t{i}-{id}")), label, &id).when(missing, |b| b.opacity(0.45))
                            .on_click(cx.listener(move |t, _, _, cx| t.convert(i, tid.clone(), cx)))
                    }))).into_any_element() }
                }
                Status::Running(f) => div().flex().flex_col().gap_2().child(div().text_sm().text_color(MUTED).child(match f { Some(f) => format!("Converting… {:.0}%", f * 100.0), None => "Converting…".into() })).child(progress_bar(*f)).into_any_element(),
                Status::Done(p) => {
                    let (p1, saved) = (p.clone(), p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                    div().flex().flex_wrap().items_center().gap_3()
                        .child(div().size(px(8.)).rounded_full().bg(OK))
                        .child(widgets::ext_tile(&j.ext)).child(div().text_color(MUTED).child("→")).child(widgets::ext_tile(&formats::extension_of(p)))
                        .child(div().text_sm().text_color(OK).child(format!("Saved {saved}")))
                        .child(div().flex_1())
                        .child(button(SharedString::from(format!("show{i}")), "Show in folder", false).on_click(move |_, _, _| jobs::reveal(&p1)))
                        .child(button(SharedString::from(format!("again{i}")), "Convert again", false).on_click(cx.listener(move |t, _, _, cx| { t.jobs[i].status = Status::Idle; cx.notify(); }))).into_any_element()
                }
                Status::Failed(msg, engine) => {
                    let engine = engine.clone();
                    div().flex().flex_col().gap_2().pl_3().border_l_2().border_color(ERR).child(div().text_sm().text_color(ERR).child(msg.clone()))
                        .child(div().flex().gap_2()
                            .when_some(engine.and_then(|e| engines::tool(&e).map(|t| t.name)), |d, name| d.child(button(SharedString::from(format!("inst{i}")), format!("Install {}", jobs::engine_label(name)), true)
                                .on_click(cx.listener(move |t, _, _, cx| { t.page = Page::Engines; t.install(name, cx); t.jobs[i].status = Status::Idle; cx.notify(); }))))
                            .child(button(SharedString::from(format!("retry{i}")), "Try again", false).on_click(cx.listener(move |t, _, _, cx| { t.jobs[i].status = Status::Idle; cx.notify(); })))).into_any_element()
                }
            };
            div().flex().flex_col().gap_3().p_4().when(i + 1 < n, |d| d.border_b_1().border_color(BORDER))
                .child(div().flex().items_center().gap_3().child(widgets::ext_tile(&j.ext)).child(div().flex_1().min_w(px(0.)).overflow_hidden().font_weight(FontWeight::SEMIBOLD).child(name))
                    .child(button(SharedString::from(format!("rm{i}")), "Remove", false).bg(tint(SURFACE_HI, 0.0)).text_color(MUTED).on_click(cx.listener(move |t, _, _, cx| { t.jobs.remove(i); cx.notify(); }))))
                .child(body)
        }).collect::<Vec<_>>();
        page_body("convert-page", div().flex().flex_col().gap_4()
            .child(page_header("Convert", "Drop in files, then pick the format you want for each."))
            .child(drop_zone)
            .when(n > 0, |d| d.child(div().flex().flex_col().flex_none().rounded(px(10.)).bg(SURFACE).border_1().border_color(BORDER).overflow_hidden().children(rows))))
    }

    fn engines_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let n = self.engines.len();
        let rows = self.engines.iter().enumerate().map(|(i, e)| {
            let name = e.name;
            let (dot, right) = match &e.state {
                EngineState::Installed(p, removable) => (OK, div().flex().items_center().gap_3().child(div().text_sm().text_color(OK).child(format!("Installed · {p}")))
                    .when(*removable, |d| d.child(button(SharedString::from(format!("rm-{name}")), "Uninstall", false).on_click(cx.listener(move |t, _, _, cx| t.remove_engine(name, cx))))).into_any_element()),
                EngineState::Available(v, size) => (ACCENT, div().flex().items_center().gap_3().child(div().text_sm().text_color(MUTED).child(format!("v{v} · {:.0} MB", *size as f64 / 1_048_576.0)))
                    .child(button(SharedString::from(format!("get-{name}")), "Install", true).on_click(cx.listener(move |t, _, _, cx| t.install(name, cx)))).into_any_element()),
                EngineState::Missing(hint) if sysinstall::plan(hint).is_some() => (MUTED, div().flex().items_center().gap_3().child(div().text_sm().text_color(MUTED).child("Not installed"))
                    .child(button(SharedString::from(format!("get-{name}")), "Install", true).on_click(cx.listener(move |t, _, _, cx| t.install(name, cx)))).into_any_element()),
                EngineState::Missing(hint) => (MUTED, div().flex().items_center().gap_3().child(div().text_sm().text_color(MUTED).child(format!("Not installed · {hint}")))
                    .when_some(sysinstall::download_page(name), |d, url| d.child(button(SharedString::from(format!("dl-{name}")), "Download page", false).on_click(cx.listener(move |_, _, _, cx| cx.open_url(url))))).into_any_element()),
                EngineState::Installing(f, stage) if *f < 0.0 => (ACCENT, div().flex().flex_col().gap_1().w(px(220.)).child(div().text_sm().text_color(MUTED).child(format!("{stage}…"))).child(progress_bar(None)).into_any_element()),
                EngineState::Installing(f, stage) => (ACCENT, div().flex().flex_col().gap_1().w(px(220.)).child(div().text_sm().text_color(MUTED).child(format!("{stage}… {:.0}%", f * 100.0))).child(progress_bar(Some(*f))).into_any_element()),
                EngineState::Failed(m, was_removal) => { let was_removal = *was_removal; (ERR, div().flex().items_center().gap_3().child(div().text_sm().text_color(ERR).max_w(px(320.)).child(m.clone()))
                    .child(button(SharedString::from(format!("retry-{name}")), "Try again", false).on_click(cx.listener(move |t, _, _, cx| if was_removal { t.remove_engine(name, cx) } else { t.install(name, cx) })))
                    .when(!was_removal, |d| d.when_some(sysinstall::download_page(name), |d, url| d.child(button(SharedString::from(format!("dl-{name}")), "Download page", false).on_click(cx.listener(move |_, _, _, cx| cx.open_url(url)))))).into_any_element()) }
            };
            div().flex().justify_between().items_center().gap_4().p_4().when(i + 1 < n, |d| d.border_b_1().border_color(BORDER))
                // Left side may shrink and wrap; the status on the right keeps its size up to a cap and wraps beyond it,
                // so a long status or a narrow window never pushes text off the edge.
                .child(div().flex().flex_1().min_w_0().items_center().gap_3().child(div().flex_none().size(px(8.)).rounded_full().bg(dot))
                    .child(div().flex().flex_col().flex_1().min_w_0().child(div().font_weight(FontWeight::SEMIBOLD).child(name)).child(div().text_sm().text_color(MUTED).child(e.used_for))))
                .child(div().flex().flex_none().max_w(px(380.)).justify_end().child(right))
        }).collect::<Vec<_>>();
        page_body("engines-page", div().flex().flex_col().gap_4()
            .child(page_header("Engines", "Most conversions are built in. Helpers add higher fidelity or rarer formats; install them here. Some installs (winget, apt) may ask for administrator permission."))
            .child(div().flex().flex_col().flex_none().rounded(px(10.)).bg(SURFACE).border_1().border_color(BORDER).overflow_hidden().children(rows)))
    }
}

/// Starts the tray and the global gesture listener, and applies their events on the UI thread.
fn start_background(cx: &mut App) -> bool {
    let (tx, rx) = std::sync::mpsc::channel::<AppEvent>();
    let ring_on = settings::get().ring;
    settings::RING_ENABLED.store(ring_on, std::sync::atomic::Ordering::Relaxed);
    let tray_ok = match tray::start(tx.clone(), ring_on) { Ok(()) => true, Err(e) => { eprintln!("no tray icon ({e}); closing the window will quit"); false } };
    let tx2 = tx.clone();
    // Global hooks are only installed when the ring is wanted (security software dislikes idle keyboard hooks).
    if ring_on || cfg!(target_os = "linux") {
        if let Err(e) = gesture::listen(move |g| { let _ = tx2.send(AppEvent::Gesture(g)); }) { eprintln!("quick-convert ring disabled: {e}"); }
    }
    let cfg = Arc::new(Config::default());
    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(std::time::Duration::from_millis(20)).await;
        tray::pump(); // menu items may only be touched from the main thread (Windows/macOS)
        while let Ok(ev) = rx.try_recv() {
            let cfg = cfg.clone();
            let _ = cx.update(|cx| match ev {
                AppEvent::OpenMain => open_main(cx, vec![]),
                AppEvent::Quit => cx.quit(),
                AppEvent::SetRing(on) => { settings::RING_ENABLED.store(on, std::sync::atomic::Ordering::Relaxed); settings::update(|s| s.ring = on); if !on { ring::close(cx); } }
                AppEvent::Gesture(_) if !settings::RING_ENABLED.load(std::sync::atomic::Ordering::Relaxed) => {}
                AppEvent::Gesture(g) => match g {
                    gesture::Gesture::Open { x, y } => { let (x, y) = gesture::to_logical(x, y); ring::open(x, y, cfg, cx) }
                    gesture::Gesture::Page(d) => { if let Some(h) = cx.try_global::<ring::Current>().and_then(|c| c.0) { let _ = h.update(cx, |r, _, cx| r.flip(d, cx)); } }
                    gesture::Gesture::Cancel => ring::close(cx),
                    gesture::Gesture::Released => {
                        // The drop (if any) is being delivered right now; give it a moment before closing an unused ring.
                        if let Some(h) = cx.try_global::<ring::Current>().and_then(|c| c.0) {
                            cx.spawn(async move |cx| {
                                cx.background_executor().timer(std::time::Duration::from_millis(700)).await;
                                let _ = cx.update(|cx| { if h.update(cx, |r, _, _| r.is_idle()).unwrap_or(false) { ring::close(cx); } });
                            }).detach();
                        }
                    }
                },
            });
        }
    }).detach();
    tray_ok
}

fn main() {
    // `--debug-gesture`: print what the global Ctrl+drag listener sees (for troubleshooting).
    if std::env::args().nth(1).as_deref() == Some("--debug-gesture") {
        match gesture::listen(|g| println!("{g:?}")) { Ok(()) => loop { std::thread::sleep(std::time::Duration::from_secs(3600)) }, Err(e) => { eprintln!("{e}"); std::process::exit(1) } }
    }
    // Never run elevated: an administrator process cannot receive drags from a normal Explorer (empty ring, refused drops).
    // Started by hand with arguments we leave it alone; a plain launch hands over to a normal copy started through Explorer.
    if std::env::args().len() == 1 && elevation::running_elevated() && elevation::relaunch_unelevated() { return; }
    // `--background`: start hidden in the tray (for autostart). Ignored when there is no tray, so the app is never invisible.
    let background = std::env::args().skip(1).any(|a| a == "--background");
    let files: Vec<PathBuf> = std::env::args_os().skip(1).filter(|a| a != "--background").map(PathBuf::from).collect();
    Application::new().run(move |cx: &mut App| {
        if let Err(e) = cx.text_system().add_fonts(vec![std::borrow::Cow::Borrowed(theme::FONT_REGULAR), std::borrow::Cow::Borrowed(theme::FONT_BOLD)]) { eprintln!("could not load the bundled font: {e}"); }
        let tray_ok = start_background(cx);
        if tray_ok { open_keepalive(cx); }
        // With a tray the app lives on after the last window closes; without one it would be invisible, so quit.
        cx.on_window_closed(move |cx| { if !tray_ok && cx.windows().is_empty() { cx.quit(); } }).detach();
        if !(background && tray_ok) { open_main(cx, files); }
    });
}
