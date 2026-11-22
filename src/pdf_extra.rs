//! Two PDF tools that are not a fixed list of fields: filling a form (the fields come from the PDF itself)
//! and comparing two PDFs (the result is a report to read, not a file).
use crate::theme::*;
use crate::widgets::TextField;
use crate::{button, runtime, settings};
use converter_core::pdf::forms::{self, FieldSummary, FieldValue};
use converter_core::{convert::output_path, formats, ErrorCode};
use gpui::{div, prelude::*, Context, Entity, ExternalPaths, FontWeight, PathPromptOptions, SharedString, Window};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn is_pdf(p: &Path) -> bool { p.is_file() && formats::extension_of(p) == ".pdf" }

fn file_name(p: &Path) -> String { p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() }

// ------------------------------------------------------------------------------------------------ form filling

enum Ui { Text(Entity<TextField>), Check(bool), Choice { options: Vec<String>, value: Option<String> } }

struct UiField { name: String, kind: &'static str, initial: Value, ui: Ui }

pub enum FormStatus { Idle, Working, Done(PathBuf), Failed(String) }

pub struct FormPanel { file: Option<PathBuf>, fields: Vec<UiField>, flatten: bool, status: FormStatus }

impl FormPanel {
    pub fn new() -> Self { Self { file: None, fields: vec![], flatten: false, status: FormStatus::Idle } }

    fn load(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.fields.clear();
        self.status = FormStatus::Idle;
        match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| forms::list_fields(&b).map_err(|e| e.message)) {
            Ok(list) => {
                self.fields = list.into_iter().filter_map(|f| to_ui(f, cx)).collect();
                if self.fields.is_empty() { self.status = FormStatus::Failed("This PDF has no fillable fields.".into()); }
                self.file = Some(path);
            }
            Err(m) => { self.file = None; self.status = FormStatus::Failed(m); }
        }
        cx.notify();
    }

    fn add(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        match paths.iter().find(|p| is_pdf(p)) { Some(p) => self.load(p.clone(), cx), None => { self.status = FormStatus::Failed("Please choose a PDF file.".into()); cx.notify(); } }
    }

    fn browse(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: None });
        cx.spawn(async move |this, cx| { if let Ok(Ok(Some(p))) = rx.await { this.update(cx, |t, cx| t.add(&p, cx)).ok(); } }).detach();
    }

    /// Only values the user changed are sent, so untouched fields keep their original appearance.
    fn changes(&self, cx: &gpui::App) -> Vec<(String, FieldValue)> {
        self.fields.iter().filter_map(|f| match &f.ui {
            Ui::Text(e) => { let v = e.read(cx).value.clone(); (Value::String(v.clone()) != f.initial).then(|| (f.name.clone(), FieldValue::Text(v))) }
            Ui::Check(b) => (Value::Bool(*b) != f.initial).then(|| (f.name.clone(), FieldValue::Bool(*b))),
            Ui::Choice { value: Some(v), .. } => (f.initial.as_str() != Some(v.as_str())).then(|| (f.name.clone(), FieldValue::Text(v.clone()))),
            Ui::Choice { value: None, .. } => None,
        }).collect()
    }

    fn run(&mut self, cx: &mut Context<Self>) {
        let Some(file) = self.file.clone() else { return };
        let (values, flatten) = (self.changes(cx), self.flatten);
        if values.is_empty() && !flatten { self.status = FormStatus::Failed("Change at least one field first (or turn on Flatten).".into()); cx.notify(); return; }
        self.status = FormStatus::Working;
        cx.notify();
        let (cfg, out_dir) = (settings::config(), settings::get().output_dir);
        let handle = runtime::get().spawn(async move {
            let bytes = tokio::fs::read(&file).await?;
            let filled = tokio::task::spawn_blocking(move || forms::fill(&bytes, &values, flatten)).await.map_err(|e| converter_core::Error::new(ErrorCode::EInternal, e.to_string()))??;
            let dir = out_dir.unwrap_or_else(|| file.parent().map(Path::to_path_buf).unwrap_or_default());
            tokio::fs::create_dir_all(&dir).await?;
            let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "form".into());
            let dest = output_path(&cfg, &dir, &format!("{stem}-filled"), ".pdf");
            tokio::fs::write(&dest, filled).await?;
            Ok::<_, converter_core::Error>(dest)
        });
        cx.spawn(async move |this, cx| {
            let r = handle.await;
            this.update(cx, |t, cx| { t.status = match r { Ok(Ok(p)) => FormStatus::Done(p), Ok(Err(e)) => FormStatus::Failed(e.message), Err(e) => FormStatus::Failed(e.to_string()) }; cx.notify(); }).ok();
        }).detach();
    }
}

fn to_ui(f: FieldSummary, cx: &mut Context<FormPanel>) -> Option<UiField> {
    let value = f.value.clone().unwrap_or(Value::Null);
    let first = |v: &Value| v.as_array().and_then(|a| a.first()).cloned().unwrap_or(Value::Null);
    let ui = match f.kind {
        "text" => { let init = value.as_str().unwrap_or("").to_string(); Ui::Text(cx.new(|cx| TextField::new("", false, &init, cx))) }
        "checkbox" => Ui::Check(value.as_bool().unwrap_or(false)),
        "radio" => Ui::Choice { options: f.options.clone().unwrap_or_default(), value: value.as_str().map(String::from) },
        "dropdown" | "optionList" => { let v = first(&value); return Some(UiField { name: f.name, kind: f.kind, initial: v.clone(), ui: Ui::Choice { options: f.options.unwrap_or_default(), value: v.as_str().map(String::from) } }); }
        _ => return None,
    };
    Some(UiField { name: f.name, kind: f.kind, initial: value, ui })
}

impl gpui::Render for FormPanel {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let drop = div().id("form-drop").flex().items_center().justify_between().gap_3().p_4().rounded_lg().border_2().border_color(BORDER).bg(SURFACE)
            .drag_over::<ExternalPaths>(|s, _, _, _| s.border_color(ACCENT).bg(tint(ACCENT, 0.10)))
            .on_drop(cx.listener(|t, p: &ExternalPaths, _, cx| t.add(p.paths(), cx)))
            .child(div().flex().flex_col().child(div().font_weight(FontWeight::SEMIBOLD).child(self.file.as_deref().map(file_name).unwrap_or_else(|| "Drop a PDF form here".into())))
                .child(div().text_sm().text_color(MUTED).child(if self.file.is_some() { format!("{} fillable fields", self.fields.len()) } else { "PDF with form fields".into() })))
            .child(button("form-browse", "Browse…", false).on_click(cx.listener(|t, _, _, cx| t.browse(cx))));

        let rows = self.fields.iter().enumerate().map(|(i, f)| {
            let control = match &f.ui {
                Ui::Text(e) => e.clone().into_any_element(),
                Ui::Check(on) => div().flex().gap_1()
                    .child(button(SharedString::from(format!("chk{i}-y")), "Checked", *on).on_click(cx.listener(move |t, _, _, cx| { if let Ui::Check(b) = &mut t.fields[i].ui { *b = true; cx.notify(); } })))
                    .child(button(SharedString::from(format!("chk{i}-n")), "Unchecked", !*on).on_click(cx.listener(move |t, _, _, cx| { if let Ui::Check(b) = &mut t.fields[i].ui { *b = false; cx.notify(); } }))).into_any_element(),
                Ui::Choice { options, value } => div().flex().flex_wrap().gap_1().children(options.iter().enumerate().map(|(oi, o)| {
                    let (o2, on) = (o.clone(), value.as_deref() == Some(o.as_str()));
                    button(SharedString::from(format!("ch{i}-{oi}")), o.clone(), on).on_click(cx.listener(move |t, _, _, cx| { if let Ui::Choice { value, .. } = &mut t.fields[i].ui { *value = Some(o2.clone()); cx.notify(); } }))
                })).into_any_element(),
            };
            div().flex().flex_col().gap_1().child(div().text_sm().text_color(MUTED).child(format!("{} · {}", f.name, f.kind))).child(control)
        }).collect::<Vec<_>>();

        let status = match &self.status {
            FormStatus::Idle => div(), FormStatus::Working => div().text_sm().text_color(MUTED).child("Filling…"),
            FormStatus::Failed(m) => div().text_sm().text_color(ERR).child(m.clone()),
            FormStatus::Done(p) => { let p = p.clone(); div().flex().flex_col().gap_1().child(div().text_sm().text_color(OK).child(format!("Saved {}", file_name(&p))))
                .child(div().flex().child(button("form-reveal", "Show in folder", false).on_click(move |_, _, _| crate::jobs::reveal(&p)))) }
        };
        div().id("form-panel").flex().flex_col().gap_3().p_5().flex_1().min_w(gpui::px(0.)).h_full().overflow_y_scroll()
            .child(div().text_size(gpui::px(22.)).font_weight(FontWeight::SEMIBOLD).child("Fill form"))
            .child(div().text_sm().text_color(MUTED).child("Fill in a PDF form. Flatten turns the filled values into ordinary page content."))
            .child(drop).children(rows)
            .when(!self.fields.is_empty(), |d| d.child(div().flex().gap_2()
                .child(button("form-flat", if self.flatten { "Flatten: on" } else { "Flatten: off" }, self.flatten).on_click(cx.listener(|t, _, _, cx| { t.flatten = !t.flatten; cx.notify(); })))
                .child(button("form-run", "Save filled copy", true).on_click(cx.listener(|t, _, _, cx| t.run(cx))))))
            .child(status)
    }
}

// ------------------------------------------------------------------------------------------------ compare

pub enum CmpStatus { Idle, Working, Done(Report), Failed(String) }

pub struct Report { a_pages: u64, b_pages: u64, differing: Vec<PageDiff>, extra_a: usize, extra_b: usize }
pub struct PageDiff { page: u64, lines: Vec<(char, String)> }

/// `compare` JSON (from the Python engine) → what the page shows. Equal runs are skipped; `-` removed, `+` added.
pub fn parse_report(v: &Value) -> Report {
    let n = |k: &str| v[k].as_u64().unwrap_or(0);
    let mut differing = vec![];
    for p in v["pages"].as_array().into_iter().flatten().filter(|p| p["equal"] == Value::Bool(false)) {
        let mut lines = vec![];
        for d in p["diff"].as_array().into_iter().flatten() {
            let (op, a, b) = (d["op"].as_str().unwrap_or(""), &d["a"], &d["b"]);
            let strs = |x: &Value| x.as_array().into_iter().flatten().filter_map(|s| s.as_str().map(String::from)).collect::<Vec<_>>();
            if op == "equal" { continue; }
            if op != "insert" { lines.extend(strs(a).into_iter().map(|s| ('-', s))); }
            if op != "delete" { lines.extend(strs(b).into_iter().map(|s| ('+', s))); }
        }
        differing.push(PageDiff { page: p["page"].as_u64().unwrap_or(0), lines });
    }
    Report { a_pages: n("pageCountA"), b_pages: n("pageCountB"), differing,
        extra_a: v["extraPagesInA"].as_array().map_or(0, Vec::len), extra_b: v["extraPagesInB"].as_array().map_or(0, Vec::len) }
}

pub struct ComparePanel { a: Option<PathBuf>, b: Option<PathBuf>, status: CmpStatus }

impl ComparePanel {
    pub fn new() -> Self { Self { a: None, b: None, status: CmpStatus::Idle } }

    fn add(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        for p in paths.iter().filter(|p| is_pdf(p)) { if self.a.is_none() { self.a = Some(p.clone()); } else { self.b = Some(p.clone()); } }
        self.status = CmpStatus::Idle;
        cx.notify();
    }

    fn browse(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: None });
        cx.spawn(async move |this, cx| { if let Ok(Ok(Some(p))) = rx.await { this.update(cx, |t, cx| t.add(&p, cx)).ok(); } }).detach();
    }

    fn run(&mut self, cx: &mut Context<Self>) {
        let (Some(a), Some(b)) = (self.a.clone(), self.b.clone()) else { return };
        self.status = CmpStatus::Working;
        cx.notify();
        let cfg = settings::config();
        let handle = runtime::get().spawn(async move {
            let (ba, bb) = (tokio::fs::read(&a).await?, tokio::fs::read(&b).await?);
            converter_core::pdf::python_ops::compare(&cfg, &ba, &bb).await
        });
        cx.spawn(async move |this, cx| {
            let r = handle.await;
            this.update(cx, |t, cx| { t.status = match r {
                Ok(Ok(v)) => CmpStatus::Done(parse_report(&v)),
                Ok(Err(e)) => CmpStatus::Failed(e.message),
                Err(e) => CmpStatus::Failed(e.to_string()) }; cx.notify(); }).ok();
        }).detach();
    }
}

impl gpui::Render for ComparePanel {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let slot = |label: &'static str, f: &Option<PathBuf>| div().flex().flex_col().child(div().text_sm().text_color(MUTED).child(label))
            .child(div().font_weight(FontWeight::SEMIBOLD).child(f.as_deref().map(file_name).unwrap_or_else(|| "—".into())));
        let drop = div().id("cmp-drop").flex().flex_wrap().items_center().justify_between().gap_3().p_4().rounded_lg().border_2().border_color(BORDER).bg(SURFACE)
            .drag_over::<ExternalPaths>(|s, _, _, _| s.border_color(ACCENT).bg(tint(ACCENT, 0.10)))
            .on_drop(cx.listener(|t, p: &ExternalPaths, _, cx| t.add(p.paths(), cx)))
            .child(div().flex().gap_6().child(slot("Original", &self.a)).child(slot("Changed", &self.b)))
            .child(div().flex().gap_1()
                .child(button("cmp-browse", "Browse…", false).on_click(cx.listener(|t, _, _, cx| t.browse(cx))))
                .child(button("cmp-clear", "Clear", false).on_click(cx.listener(|t, _, _, cx| { t.a = None; t.b = None; t.status = CmpStatus::Idle; cx.notify(); }))));
        let ready = self.a.is_some() && self.b.is_some() && !matches!(self.status, CmpStatus::Working);
        let result = match &self.status {
            CmpStatus::Idle => div(), CmpStatus::Working => div().text_sm().text_color(MUTED).child("Comparing…"),
            CmpStatus::Failed(m) => div().text_sm().text_color(ERR).child(m.clone()),
            CmpStatus::Done(r) => {
                let head = if r.differing.is_empty() && r.extra_a + r.extra_b == 0 { "No differences in the text.".to_string() }
                    else { format!("{} of {} pages differ{}", r.differing.len(), r.a_pages.min(r.b_pages), if r.extra_a + r.extra_b > 0 { format!("; page counts differ ({} vs {})", r.a_pages, r.b_pages) } else { String::new() }) };
                div().flex().flex_col().gap_2().child(div().font_weight(FontWeight::SEMIBOLD).text_color(if r.differing.is_empty() { OK } else { TEXT }).child(head))
                    .children(r.differing.iter().map(|p| div().flex().flex_col().gap_1().p_3().rounded_lg().bg(SURFACE).border_1().border_color(BORDER)
                        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(format!("Page {}", p.page)))
                        .children(p.lines.iter().map(|(c, s)| div().text_sm().text_color(if *c == '-' { ERR } else { OK }).child(format!("{c} {s}"))))))
            }
        };
        div().id("cmp-panel").flex().flex_col().gap_3().p_5().flex_1().min_w(gpui::px(0.)).h_full().overflow_y_scroll()
            .child(div().text_size(gpui::px(22.)).font_weight(FontWeight::SEMIBOLD).child("Compare"))
            .child(div().text_sm().text_color(MUTED).child("Shows which lines of text were removed or added between two PDFs. Drop both files together."))
            .child(drop)
            .child(div().flex().child(button("cmp-run", "Compare", true).when(!ready, |b| b.opacity(0.45)).on_click(cx.listener(move |t, _, _, cx| { if ready { t.run(cx); } }))))
            .child(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn report_keeps_only_changed_lines_and_marks_direction() {
        let v: Value = serde_json::from_str(r#"{"pageCountA":3,"pageCountB":4,"extraPagesInA":[],"extraPagesInB":[4],
            "pages":[{"page":1,"equal":true},
                     {"page":2,"equal":false,"diff":[{"op":"equal","a":["same"],"b":["same"]},{"op":"replace","a":["old"],"b":["new","new2"]},
                                                      {"op":"delete","a":["gone"],"b":[]},{"op":"insert","a":[],"b":["added"]}]}]}"#).unwrap();
        let r = parse_report(&v);
        assert_eq!((r.a_pages, r.b_pages, r.extra_b), (3, 4, 1));
        assert_eq!(r.differing.len(), 1);
        assert_eq!(r.differing[0].page, 2);
        assert_eq!(r.differing[0].lines, vec![('-', "old".into()), ('+', "new".into()), ('+', "new2".into()), ('-', "gone".into()), ('+', "added".into())]);
    }
    #[test] fn identical_documents_report_nothing() {
        let v: Value = serde_json::from_str(r#"{"pageCountA":1,"pageCountB":1,"extraPagesInA":[],"extraPagesInB":[],"pages":[{"page":1,"equal":true}]}"#).unwrap();
        let r = parse_report(&v);
        assert!(r.differing.is_empty() && r.extra_a == 0 && r.extra_b == 0);
    }
    #[test] fn garbage_report_does_not_panic() {
        let r = parse_report(&Value::Null);
        assert_eq!((r.a_pages, r.differing.len()), (0, 0));
    }
}
