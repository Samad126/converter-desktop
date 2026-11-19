//! Converter desktop app (gpui). Screens: Convert (drop files, pick a target) and Engines.
mod runtime;
mod theme;

use converter_core::{
    engines::{self, manager},
    formats,
    progress::{Progress, ProgressFn},
    Config, ConvertOptions, ErrorCode,
};
use gpui::{
    div, prelude::*, px, relative, App, Application, Bounds, Context, ExternalPaths, FontWeight, SharedString, TitlebarOptions, Window,
    WindowBounds, WindowOptions,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use theme::*;

#[derive(Clone, PartialEq)]
enum Status { Idle, Running(Option<f32>), Done(PathBuf), Failed(String, Option<String>) }

struct Job { path: PathBuf, ext: String, status: Status }

#[derive(Clone, Copy, PartialEq)]
enum Page { Convert, Engines }

struct EngineRow { name: &'static str, used_for: &'static str, state: EngineState }

#[derive(Clone, PartialEq)]
enum EngineState { Installed(String), Available(String, u64), Missing(&'static str), Installing(f32, &'static str), Failed(String) }

struct Converter { page: Page, jobs: Vec<Job>, engines: Vec<EngineRow>, cfg: Arc<Config> }

impl Converter {
    fn new() -> Self {
        let mut s = Self { page: Page::Convert, jobs: vec![], engines: vec![], cfg: Arc::new(Config::default()) };
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
                    (Some(p), _) => EngineState::Installed(if p.starts_with(&self.cfg.engines_dir) { "installed by Converter".into() } else { format!("found on this computer ({})", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()) }),
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

    /// Targets for a file: documents/images/data from the matrix, or audio/video from the media matrix.
    fn targets_for(ext: &str) -> Vec<(String, String)> {
        let m = formats::matrix();
        let mut v: Vec<(String, String)> = formats::targets_for(ext).iter().filter_map(|id| m.targets.get(id).map(|t| (id.clone(), t.label.clone()))).collect();
        for id in converter_core::media::media_targets_for(ext) { if let Some(t) = m.media.get(id) { v.push((format!("media:{id}"), t.label.clone())); } }
        v
    }

    /// The engine a target needs, if it is not installed yet.
    fn missing_engine(&self, ext: &str, target: &str) -> Option<&'static str> {
        let tool = if target.starts_with("media:") { "ffmpeg" } else { engines::required_tool(formats::resolve_conversion(ext, target)?.engine)? };
        let t = engines::tool(tool)?;
        engines::resolve(t, &self.cfg, None).is_none().then_some(t.name)
    }

    fn convert(&mut self, idx: usize, target: String, cx: &mut Context<Self>) {
        let (path, ext) = { let j = &self.jobs[idx]; (j.path.clone(), j.ext.clone()) };
        if let Some(missing) = self.missing_engine(&ext, &target) {
            self.jobs[idx].status = Status::Failed(format!("This needs {missing}, which is not installed yet."), Some(missing.to_string()));
            cx.notify();
            return;
        }
        self.jobs[idx].status = Status::Running(None);
        cx.notify();
        let cfg = self.cfg.clone();
        let frac = Arc::new(Mutex::new(None::<f32>));
        let f2 = frac.clone();
        let progress: ProgressFn = Arc::new(move |p| if let Progress::Fraction(f) = p { *f2.lock().unwrap() = Some(f); });
        let handle = runtime::get().spawn(async move {
            let opts = ConvertOptions::default();
            match target.strip_prefix("media:") {
                Some(id) => converter_core::media::convert_media(&cfg, &path, id, opts, progress).await,
                None => converter_core::convert_file(&cfg, &path, &target, opts).await.map(|mut v| v.remove(0)),
            }
        });
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
                if let Ok(Err(e)) = &result { if let Some(r) = t.engines.iter_mut().find(|e| e.name == name) { r.state = EngineState::Failed(e.message.clone()); } }
                cx.notify();
            }).ok();
        }).detach();
    }
}

fn button(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, primary: bool) -> gpui::Stateful<gpui::Div> {
    div().id(id).px_3().py_1().rounded_md().text_sm().cursor_pointer().child(label.into())
        .bg(if primary { ACCENT } else { SURFACE_HI }).text_color(if primary { ON_ACCENT } else { TEXT })
        .hover(move |s| s.bg(if primary { ACCENT_HI } else { BORDER }))
}

fn progress_bar(frac: Option<f32>) -> impl IntoElement {
    div().h(px(6.)).w_full().rounded_full().bg(BORDER).child(div().h_full().rounded_full().bg(ACCENT).w(relative(frac.unwrap_or(0.35).clamp(0.02, 1.0))))
}

impl Render for Converter {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let nav = |page: Page, label: &'static str, cx: &mut Context<Self>| {
            let on = self.page == page;
            div().id(label).px_3().py_2().rounded_md().cursor_pointer().text_sm().child(label)
                .when(on, |d| d.bg(SURFACE_HI).font_weight(FontWeight::SEMIBOLD))
                .hover(|s| s.bg(SURFACE_HI))
                .on_click(cx.listener(move |t, _, _, cx| { t.page = page; if page == Page::Engines { t.refresh_engines(true); } cx.notify(); }))
        };
        div().flex().size_full().bg(BG).text_color(TEXT)
            .child(div().flex().flex_col().gap_1().w(px(170.)).p_3().bg(SURFACE).border_r_1().border_color(BORDER)
                .child(div().px_3().py_2().text_lg().font_weight(FontWeight::BOLD).child("Converter"))
                .child(nav(Page::Convert, "Convert", cx)).child(nav(Page::Engines, "Engines", cx)))
            .child(div().flex_1().h_full().overflow_hidden().child(match self.page { Page::Convert => self.convert_page(cx).into_any_element(), Page::Engines => self.engines_page(cx).into_any_element() }))
    }
}

impl Converter {
    fn convert_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let drop_zone = div().id("drop").flex().flex_col().items_center().justify_center().gap_1().h(px(130.)).rounded_lg().border_2().border_color(BORDER)
            .bg(SURFACE).text_color(MUTED)
            .drag_over::<ExternalPaths>(|s, _, _, _| s.border_color(ACCENT).bg(SURFACE_HI))
            .on_drop(cx.listener(|t, paths: &ExternalPaths, _, cx| { t.add_files(paths.paths()); cx.notify(); }))
            .child(div().text_color(TEXT).font_weight(FontWeight::SEMIBOLD).child("Drop files here"))
            .child(div().text_sm().child("documents, images, audio, video, PDFs, archives, data files…"));
        let rows = self.jobs.iter().enumerate().map(|(i, j)| {
            let name = j.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let body = match &j.status {
                Status::Idle => {
                    let targets = Self::targets_for(&j.ext);
                    if targets.is_empty() { div().text_sm().text_color(MUTED).child(format!("{} files can't be converted.", if j.ext.is_empty() { "These".into() } else { j.ext.clone() })).into_any_element() }
                    else { div().flex().flex_wrap().gap_1().children(targets.into_iter().map(|(id, label)| {
                        let missing = self.missing_engine(&j.ext, &id).is_some();
                        let tid = id.clone();
                        button(SharedString::from(format!("t{i}-{id}")), label, false).when(missing, |b| b.opacity(0.45))
                            .on_click(cx.listener(move |t, _, _, cx| t.convert(i, tid.clone(), cx)))
                    })).into_any_element() }
                }
                Status::Running(f) => div().flex().flex_col().gap_1().child(div().text_sm().text_color(MUTED).child(match f { Some(f) => format!("Converting… {:.0}%", f * 100.0), None => "Converting…".into() })).child(progress_bar(*f)).into_any_element(),
                Status::Done(p) => div().flex().gap_2().items_center().child(div().text_sm().text_color(OK).child(format!("Saved {}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())))
                    .child(button(SharedString::from(format!("again{i}")), "Convert again", false).on_click(cx.listener(move |t, _, _, cx| { t.jobs[i].status = Status::Idle; cx.notify(); }))).into_any_element(),
                Status::Failed(msg, engine) => {
                    let engine = engine.clone();
                    div().flex().flex_col().gap_1().child(div().text_sm().text_color(ERR).child(msg.clone()))
                        .child(div().flex().gap_2()
                            .when_some(engine.and_then(|e| engines::tool(&e).map(|t| t.name)), |d, name| d.child(button(SharedString::from(format!("inst{i}")), format!("Install {name}"), true)
                                .on_click(cx.listener(move |t, _, _, cx| { t.page = Page::Engines; t.install(name, cx); t.jobs[i].status = Status::Idle; cx.notify(); }))))
                            .child(button(SharedString::from(format!("retry{i}")), "Try again", false).on_click(cx.listener(move |t, _, _, cx| { t.jobs[i].status = Status::Idle; cx.notify(); })))).into_any_element()
                }
            };
            div().flex().flex_col().gap_2().p_3().rounded_lg().bg(SURFACE).border_1().border_color(BORDER)
                .child(div().flex().justify_between().child(div().font_weight(FontWeight::SEMIBOLD).child(name))
                    .child(button(SharedString::from(format!("rm{i}")), "Remove", false).on_click(cx.listener(move |t, _, _, cx| { t.jobs.remove(i); cx.notify(); }))))
                .child(body)
        }).collect::<Vec<_>>();
        div().id("convert-page").flex().flex_col().gap_3().p_5().size_full().overflow_y_scroll().child(drop_zone)
            .child(div().text_sm().text_color(MUTED).child("Pick a format for each file. The result is saved next to the original."))
            .children(rows)
    }

    fn engines_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.engines.iter().map(|e| {
            let name = e.name;
            let right = match &e.state {
                EngineState::Installed(p) => div().text_sm().text_color(OK).child(format!("Installed · {p}")).into_any_element(),
                EngineState::Available(v, size) => div().flex().items_center().gap_2().child(div().text_sm().text_color(MUTED).child(format!("v{v} · {:.0} MB", *size as f64 / 1_048_576.0)))
                    .child(button(SharedString::from(format!("get-{name}")), "Install", true).on_click(cx.listener(move |t, _, _, cx| t.install(name, cx)))).into_any_element(),
                EngineState::Missing(hint) => div().text_sm().text_color(MUTED).child(format!("Not installed · {hint}")).into_any_element(),
                EngineState::Installing(f, stage) => div().flex().flex_col().gap_1().w(px(220.)).child(div().text_sm().text_color(MUTED).child(format!("{stage}… {:.0}%", f * 100.0))).child(progress_bar(Some(*f))).into_any_element(),
                EngineState::Failed(m) => div().text_sm().text_color(ERR).child(m.clone()).into_any_element(),
            };
            div().flex().justify_between().items_center().gap_4().p_3().rounded_lg().bg(SURFACE).border_1().border_color(BORDER)
                .child(div().flex().flex_col().child(div().font_weight(FontWeight::SEMIBOLD).child(name)).child(div().text_sm().text_color(MUTED).child(e.used_for)))
                .child(right)
        }).collect::<Vec<_>>();
        div().id("engines-page").flex().flex_col().gap_3().p_5().size_full().overflow_y_scroll()
            .child(div().text_xl().font_weight(FontWeight::BOLD).child("Engines"))
            .child(div().text_sm().text_color(MUTED).child("Some conversions use helper programs. Install them here; nothing needs admin rights."))
            .children(rows)
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, gpui::size(px(980.), px(660.)), cx);
        cx.open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("Converter".into()), ..Default::default() }),
            ..Default::default()
        }, |_, cx| cx.new(|_| { let mut c = Converter::new(); c.add_files(&std::env::args_os().skip(1).map(PathBuf::from).collect::<Vec<_>>()); c })).expect("could not open the window");
        cx.activate(true);
    });
}
