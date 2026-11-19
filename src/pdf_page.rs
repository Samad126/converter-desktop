//! The "PDF tools" page: pick a tool, add files, fill its fields, run.
use crate::pdf_tools::{accepts_file, build_flags, Accepts, FieldKind, ToolSpec, TOOLS};
use crate::theme::*;
use crate::widgets::TextField;
use crate::{jobs, runtime};
use converter_core::{formats, ErrorCode};
use gpui::{div, prelude::*, px, Context, Entity, EventEmitter, ExternalPaths, FontWeight, PathPromptOptions, SharedString, Window};
use std::collections::HashMap;
use std::path::PathBuf;

pub enum PdfEvent { Install(&'static str) }

/// The list on the left has the form-driven tools, then two special ones with their own panels.
#[derive(Clone, Copy, PartialEq)]
enum Special { Form, Compare, Editor }

enum Field { Text(Entity<TextField>), Choice(String) }

enum Status { Idle, Running, Done(Vec<PathBuf>), Failed(String, Option<String>) }

pub struct PdfPage { special: Option<Special>, form: Entity<crate::pdf_extra::FormPanel>, compare: Entity<crate::pdf_extra::ComparePanel>, editor: Entity<crate::page_editor::PageEditor>, selected: usize, files: Vec<PathBuf>, fields: Vec<Field>, status: Status }

impl EventEmitter<PdfEvent> for PdfPage {}

fn button(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, primary: bool) -> gpui::Stateful<gpui::Div> {
    crate::button(id, label, primary)
}

impl PdfPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let form = cx.new(|_| crate::pdf_extra::FormPanel::new());
        let compare = cx.new(|_| crate::pdf_extra::ComparePanel::new());
        let editor = cx.new(crate::page_editor::PageEditor::new);
        let mut s = Self { special: None, form, compare, editor, selected: 0, files: vec![], fields: vec![], status: Status::Idle };
        s.select(0, cx);
        s
    }

    fn tool(&self) -> &'static ToolSpec { &TOOLS[self.selected] }

    fn select(&mut self, idx: usize, cx: &mut Context<Self>) {
        self.special = None;
        self.selected = idx;
        let tool = self.tool();
        self.files.retain(|f| accepts_file(tool, &formats::extension_of(f)));
        self.status = Status::Idle;
        self.fields = tool.fields.iter().map(|f| match &f.kind {
            FieldKind::Text { secret } => Field::Text(cx.new(|cx| TextField::new(f.hint, *secret, f.default, cx))),
            FieldKind::Choice(_) => Field::Choice(f.default.to_string()),
        }).collect();
        cx.notify();
    }

    fn add_files(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        let tool = self.tool();
        let good: Vec<PathBuf> = paths.iter().filter(|p| p.is_file() && accepts_file(tool, &formats::extension_of(p))).cloned().collect();
        if good.is_empty() { self.status = Status::Failed(format!("{} only takes {} files.", tool.name, if tool.accepts == Accepts::Pdf { "PDF" } else { "PNG or JPG" }), None); }
        else {
            self.status = Status::Idle;
            if tool.multi { for p in good { if !self.files.contains(&p) { self.files.push(p); } } } else { self.files = vec![good[0].clone()]; }
        }
        cx.notify();
    }

    fn browse(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: self.tool().multi, prompt: None });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await { this.update(cx, |t, cx| t.add_files(&paths, cx)).ok(); }
        }).detach();
    }

    fn run(&mut self, cx: &mut Context<Self>) {
        let tool = self.tool();
        let values: HashMap<&'static str, String> = tool.fields.iter().zip(&self.fields).map(|(spec, st)| (spec.key, match st {
            Field::Text(e) => e.read(cx).value.clone(), Field::Choice(v) => v.clone() })).collect();
        let flags = match build_flags(tool, &values) { Ok(f) => f, Err(m) => { self.status = Status::Failed(m, None); cx.notify(); return; } };
        self.status = Status::Running;
        cx.notify();
        let (cfg, files, op, out) = (crate::settings::config(), self.files.clone(), tool.op, crate::settings::get().output_dir);
        let handle = runtime::get().spawn(async move {
            let f = move |k: &str| flags.get(k).cloned();
            converter_core::pdf_cli::run(&cfg, op, &files, &f, out).await
        });
        cx.spawn(async move |this, cx| {
            let r = handle.await;
            this.update(cx, |t, cx| {
                t.status = match r {
                    Ok(Ok(v)) => Status::Done(v),
                    Ok(Err(e)) => Status::Failed(e.message.clone(), (e.code == ErrorCode::EEngineMissing).then(|| e.engine.clone()).flatten()),
                    Err(e) => Status::Failed(format!("The operation stopped unexpectedly: {e}"), None),
                };
                cx.notify();
            }).ok();
        }).detach();
    }
}

impl gpui::Render for PdfPage {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tool = self.tool();
        let list = div().id("pdf-tools").flex().flex_col().gap_1().w(px(210.)).h_full().p_3().border_r_1().border_color(BORDER).overflow_y_scroll()
            .children(TOOLS.iter().enumerate().map(|(i, t)| {
                let on = self.special.is_none() && i == self.selected;
                div().id(SharedString::from(format!("tool-{}", t.op))).px_3().py_2().rounded_md().cursor_pointer().text_sm().child(t.name)
                    .when(on, |d| d.bg(SURFACE_HI).font_weight(FontWeight::SEMIBOLD)).hover(|s| s.bg(SURFACE_HI))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(i, cx)))
            }))
            .child(div().h(px(1.)).my_2().bg(BORDER))
            .children([(Special::Editor, "Edit, sign & redact"), (Special::Form, "Fill form"), (Special::Compare, "Compare")].into_iter().map(|(sp, name)| {
                div().id(SharedString::from(format!("special-{name}"))).px_3().py_2().rounded_md().cursor_pointer().text_sm().child(name)
                    .when(self.special == Some(sp), |d| d.bg(SURFACE_HI).font_weight(FontWeight::SEMIBOLD)).hover(|s| s.bg(SURFACE_HI))
                    .on_click(cx.listener(move |this, _, _, cx| { this.special = Some(sp); cx.notify(); }))
            }));

        let drop_zone = div().id("pdf-drop").flex().items_center().justify_between().gap_3().p_4().rounded_lg().border_2().border_color(BORDER).bg(SURFACE)
            .drag_over::<ExternalPaths>(|s, _, _, _| s.border_color(ACCENT).bg(SURFACE_HI))
            .on_drop(cx.listener(|this, p: &ExternalPaths, _, cx| this.add_files(p.paths(), cx)))
            .child(div().flex().flex_col().child(div().font_weight(FontWeight::SEMIBOLD).child(if tool.multi { "Drop files here" } else { "Drop a file here" }))
                .child(div().text_sm().text_color(MUTED).child(if tool.accepts == Accepts::Pdf { "PDF" } else { "PNG or JPG images" })))
            .child(button("pdf-browse", "Browse…", false).on_click(cx.listener(|this, _, _, cx| this.browse(cx))));

        let n = self.files.len();
        let file_rows = self.files.iter().enumerate().map(|(i, f)| {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            div().flex().items_center().justify_between().gap_2().px_3().py_1().rounded_md().bg(SURFACE).border_1().border_color(BORDER)
                .child(div().text_sm().child(format!("{}. {name}", i + 1)))
                .child(div().flex().gap_1()
                    .when(tool.multi && i > 0, |d| d.child(button(SharedString::from(format!("up{i}")), "Up", false).on_click(cx.listener(move |t, _, _, cx| { t.files.swap(i, i - 1); cx.notify(); }))))
                    .when(tool.multi && i + 1 < n, |d| d.child(button(SharedString::from(format!("dn{i}")), "Down", false).on_click(cx.listener(move |t, _, _, cx| { t.files.swap(i, i + 1); cx.notify(); }))))
                    .child(button(SharedString::from(format!("rmf{i}")), "Remove", false).on_click(cx.listener(move |t, _, _, cx| { t.files.remove(i); cx.notify(); }))))
        }).collect::<Vec<_>>();

        let fields = tool.fields.iter().zip(self.fields.iter()).enumerate().map(|(fi, (spec, st))| {
            let input = match (st, &spec.kind) {
                (Field::Text(e), _) => e.clone().into_any_element(),
                (Field::Choice(cur), FieldKind::Choice(opts)) => div().flex().gap_1().children(opts.iter().map(|(v, label)| {
                    let (v, on) = (v.to_string(), cur == v);
                    button(SharedString::from(format!("c{fi}-{v}")), *label, on).on_click(cx.listener(move |t, _, _, cx| { if let Some(Field::Choice(c)) = t.fields.get_mut(fi) { *c = v.clone(); cx.notify(); } }))
                })).into_any_element(),
                _ => div().into_any_element(),
            };
            div().flex().flex_col().gap_1().child(div().text_sm().text_color(MUTED).child(spec.label)).child(input)
        }).collect::<Vec<_>>();

        let ready = n >= tool.min_files && !matches!(self.status, Status::Running);
        let status = match &self.status {
            Status::Idle => div(),
            Status::Running => div().text_sm().text_color(MUTED).child("Working…"),
            Status::Done(v) => div().flex().flex_col().gap_1()
                .child(div().text_sm().text_color(OK).child(format!("Saved {}", v.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect::<Vec<_>>().join(", "))))
                .when_some(v.first().cloned(), |d, first| d.child(div().flex().child(button("pdf-reveal", "Show in folder", false).on_click(move |_, _, _| jobs::reveal(&first))))),
            Status::Failed(msg, engine) => {
                let name = engine.as_deref().and_then(converter_core::engines::tool).map(|t| t.name);
                div().flex().flex_col().gap_1().child(div().text_sm().text_color(ERR).child(msg.clone()))
                    .when_some(name, |d, name| d.child(div().flex().child(button("pdf-install", format!("Install {name}"), true).on_click(cx.listener(move |_, _, _, cx| cx.emit(PdfEvent::Install(name)))))))
            }
        };

        let panel = div().id("pdf-panel").flex().flex_col().gap_3().p_5().flex_1().min_w(px(0.)).h_full().overflow_y_scroll()
            .child(div().text_xl().font_weight(FontWeight::BOLD).child(tool.name))
            .child(div().text_sm().text_color(MUTED).child(tool.desc))
            .child(drop_zone)
            .when(!file_rows.is_empty(), |d| d.child(div().flex().flex_col().gap_1().children(file_rows)))
            .children(fields)
            .child(div().flex().child(button("pdf-run", if tool.min_files > 1 && n < tool.min_files { format!("Add at least {} files", tool.min_files) } else { "Run".to_string() }, true)
                .when(!ready, |b| b.opacity(0.45)).on_click(cx.listener(move |this, _, _, cx| { if ready { this.run(cx); } }))))
            .child(status);
        let right = match self.special {
            Some(Special::Form) => self.form.clone().into_any_element(),
            Some(Special::Compare) => self.compare.clone().into_any_element(),
            Some(Special::Editor) => self.editor.clone().into_any_element(),
            None => panel.into_any_element(),
        };
        div().flex().size_full().child(list).child(right)
    }
}
