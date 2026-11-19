//! "Edit, sign & redact": the PDF page is rendered with poppler and shown as an image; clicks and drags on it
//! become annotations (in PDF points). Saving runs redact (Python/PyMuPDF), then edit, then sign (pure Rust).
use crate::editor_model::*;
use crate::theme::*;
use crate::widgets::TextField;
use crate::{button, runtime, settings};
use converter_core::pdf::{overlay, pages, python_ops};
use converter_core::{convert::output_path, engines, formats, process, ErrorCode};
use gpui::{
    canvas, div, img, point, prelude::*, px, Bounds, Context, Entity, ExternalPaths, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PathBuilder, PathPromptOptions, Pixels, Rgba, SharedString, Window,
};
use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Width of the page on screen, in px. The PDF is rendered at about twice this so it stays sharp.
const SHOW_W: f32 = 560.0;

enum Status { Idle, Working, Done(PathBuf), Failed(String) }

pub struct PageEditor {
    file: Option<PathBuf>,
    sizes: Vec<(f64, f64)>,
    page: usize,
    images: HashMap<usize, PathBuf>,
    rendering: Option<usize>,
    workdir: PathBuf,
    items: Vec<Item>,
    tool: Tool,
    color: &'static str,
    text: Entity<TextField>,
    drag: Option<((f64, f64), (f64, f64))>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    status: Status,
}

fn color_rgba(name: &str) -> Rgba {
    let hex = match name { "red" => 0xbf1414, "blue" => 0x1740b3, "green" => 0x147f2e, "yellow" => 0xd9b30d, "orange" => 0xd97300, _ => 0x0d0d0d };
    Rgba { r: ((hex >> 16) & 255) as f32 / 255.0, g: ((hex >> 8) & 255) as f32 / 255.0, b: (hex & 255) as f32 / 255.0, a: 1.0 }
}

impl PageEditor {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let workdir = std::env::temp_dir().join(format!("converter-editor-{}", std::process::id()));
        Self { file: None, sizes: vec![], page: 0, images: HashMap::new(), rendering: None, workdir, items: vec![], tool: Tool::Text, color: "red",
               text: cx.new(|cx| TextField::new("Text to place", false, "", cx)), drag: None, bounds: Rc::new(Cell::new(Bounds::default())), status: Status::Idle }
    }

    fn scale(&self) -> f32 { self.sizes.get(self.page).map(|s| SHOW_W / s.0 as f32).unwrap_or(1.0) }

    fn load(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| pages::page_sizes(&b).map_err(|e| e.message)) {
            Ok(sizes) if !sizes.is_empty() => {
                // A fresh folder per file so a previous document's page images can never show up.
                let _ = std::fs::remove_dir_all(&self.workdir);
                let _ = std::fs::create_dir_all(&self.workdir);
                self.file = Some(path); self.sizes = sizes; self.page = 0; self.images.clear(); self.items.clear(); self.drag = None; self.status = Status::Idle;
                self.ensure_page(cx);
            }
            Ok(_) => self.status = Status::Failed("That PDF has no pages.".into()),
            Err(m) => self.status = Status::Failed(m),
        }
        cx.notify();
    }

    fn add(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        match paths.iter().find(|p| p.is_file() && formats::extension_of(p) == ".pdf") { Some(p) => self.load(p.clone(), cx), None => { self.status = Status::Failed("Please choose a PDF file.".into()); cx.notify(); } }
    }

    fn browse(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: None });
        cx.spawn(async move |this, cx| { if let Ok(Ok(Some(p))) = rx.await { this.update(cx, |t, cx| t.add(&p, cx)).ok(); } }).detach();
    }

    /// Renders the current page with pdftoppm if it is not on screen yet.
    fn ensure_page(&mut self, cx: &mut Context<Self>) {
        let (Some(file), Some(&(w, _))) = (self.file.clone(), self.sizes.get(self.page)) else { return };
        if self.images.contains_key(&self.page) || self.rendering == Some(self.page) { return; }
        let cfg = settings::config();
        let Some(bin) = engines::resolve(engines::tool("poppler").unwrap(), &cfg, None) else {
            self.status = Status::Failed("Showing pages needs poppler, which is not installed. Install it from the Engines page.".into()); return;
        };
        let page = self.page;
        self.rendering = Some(page);
        let dpi = ((2.0 * SHOW_W as f64) / (w / 72.0)).clamp(50.0, 300.0);
        let dir = self.workdir.clone();
        let prefix = dir.join(format!("p{page}"));
        let args: Vec<String> = ["-png".to_string(), "-r".into(), format!("{dpi:.0}"), "-f".into(), (page + 1).to_string(), "-l".into(), (page + 1).to_string(), "-singlefile".into(),
            file.to_string_lossy().into_owned(), prefix.to_string_lossy().into_owned()].to_vec();
        let handle = runtime::get().spawn(async move { process::run(&bin, &args, &dir, std::time::Duration::from_secs(60)).await });
        cx.spawn(async move |this, cx| {
            let r = handle.await;
            this.update(cx, |t, cx| {
                t.rendering = None;
                match r {
                    Ok(Ok(o)) if o.ok() => { t.images.insert(page, prefix.with_extension("png")); }
                    Ok(Ok(o)) => t.status = Status::Failed(format!("Could not draw the page: {}", o.stderr.trim())),
                    Ok(Err(e)) => t.status = Status::Failed(e.message),
                    Err(e) => t.status = Status::Failed(e.to_string()),
                }
                cx.notify();
            }).ok();
        }).detach();
    }

    fn go(&mut self, delta: i32, cx: &mut Context<Self>) {
        let n = self.sizes.len() as i32;
        if n == 0 { return; }
        self.page = (self.page as i32 + delta).clamp(0, n - 1) as usize;
        self.drag = None;
        self.ensure_page(cx);
        cx.notify();
    }

    fn pt(&self, p: gpui::Point<Pixels>) -> (f64, f64) {
        let b = self.bounds.get();
        screen_to_pt((f32::from(p.x), f32::from(p.y)), (f32::from(b.origin.x), f32::from(b.origin.y)), self.scale(), self.sizes.get(self.page).copied().unwrap_or((612.0, 792.0)))
    }

    fn begin(&mut self, p: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        if self.file.is_none() { return; }
        let at = self.pt(p);
        if std::env::var_os("CONVERTER_DEBUG_EDITOR").is_some() { eprintln!("editor: click {:?} bounds {:?} scale {} -> pt {:?}", p, self.bounds.get(), self.scale(), at); }
        self.drag = Some((at, at));
        self.status = Status::Idle;
        cx.notify();
    }

    fn drag_to(&mut self, p: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        if let Some((start, _)) = self.drag { self.drag = Some((start, self.pt(p))); cx.notify(); }
    }

    fn finish(&mut self, p: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        let Some((start, _)) = self.drag.take() else { return };
        let end = self.pt(p);
        let text = self.text.read(cx).value.clone();
        let size = self.sizes.get(self.page).copied().unwrap_or((612.0, 792.0));
        match make_item(self.page, self.tool, self.color, start, end, &text, size) {
            Some(it) => self.items.push(it),
            None if self.tool.is_placed() => self.status = Status::Failed("Type the text first, then click where it should go.".into()),
            None => {}
        }
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(file) = self.file.clone() else { return };
        if self.items.is_empty() { self.status = Status::Failed("Nothing to save yet: add some text, shapes, a signature or a redaction.".into()); cx.notify(); return; }
        let req = to_requests(&self.items);
        self.status = Status::Working;
        cx.notify();
        let (cfg, out_dir) = (settings::config(), settings::get().output_dir);
        let handle = runtime::get().spawn(async move {
            let mut data = tokio::fs::read(&file).await?;
            if !req.redact.is_empty() { data = python_ops::redact(&cfg, &data, &req.redact).await?; }
            let (edit, sign) = (req.edit, req.sign);
            data = tokio::task::spawn_blocking(move || -> converter_core::Result<Vec<u8>> {
                let mut d = data;
                let edits: Vec<overlay::EditElement> = serde_json::from_value(edit).map_err(|e| converter_core::Error::new(ErrorCode::EInternal, e.to_string()))?;
                let signs: Vec<overlay::SignElement> = serde_json::from_value(sign).map_err(|e| converter_core::Error::new(ErrorCode::EInternal, e.to_string()))?;
                if !edits.is_empty() { d = overlay::edit(&d, &edits, &[])?; }
                if !signs.is_empty() { d = overlay::sign(&d, &signs, &[])?; }
                Ok(d)
            }).await.map_err(|e| converter_core::Error::new(ErrorCode::EInternal, e.to_string()))??;
            let dir = out_dir.unwrap_or_else(|| file.parent().map(Path::to_path_buf).unwrap_or_default());
            tokio::fs::create_dir_all(&dir).await?;
            let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "document".into());
            let dest = output_path(&cfg, &dir, &format!("{stem}-edited"), ".pdf");
            tokio::fs::write(&dest, data).await?;
            Ok::<_, converter_core::Error>(dest)
        });
        cx.spawn(async move |this, cx| {
            let r = handle.await;
            this.update(cx, |t, cx| { t.status = match r { Ok(Ok(p)) => Status::Done(p), Ok(Err(e)) => Status::Failed(e.message), Err(e) => Status::Failed(e.to_string()) }; cx.notify(); }).ok();
        }).detach();
    }
}

fn tool_hint(t: Tool) -> &'static str {
    match t {
        Tool::Text => "Type the text below, then click on the page where it should go.",
        Tool::Signature => "Type the name below, then click to place it (or drag a box to size it).",
        Tool::Rect | Tool::Ellipse | Tool::Line => "Drag on the page to draw.",
        Tool::Redact => "Drag over what to remove. The text under it is deleted from the file, not just covered.",
    }
}

impl gpui::Render for PageEditor {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.scale();
        let size = self.sizes.get(self.page).copied().unwrap_or((612.0, 792.0));
        let (show_w, show_h) = (SHOW_W, (size.1 as f32 * s).max(1.0));

        let drop = div().id("ed-drop").flex().flex_wrap().items_center().justify_between().gap_3().p_3().rounded_lg().border_2().border_color(BORDER).bg(SURFACE)
            .drag_over::<ExternalPaths>(|st, _, _, _| st.border_color(ACCENT).bg(SURFACE_HI))
            .on_drop(cx.listener(|t, p: &ExternalPaths, _, cx| t.add(p.paths(), cx)))
            .child(div().font_weight(FontWeight::SEMIBOLD).child(self.file.as_deref().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Drop a PDF here".into())))
            .child(button("ed-browse", "Browse…", false).on_click(cx.listener(|t, _, _, cx| t.browse(cx))));
        if self.file.is_none() {
            return div().id("editor-panel").flex().flex_col().gap_3().p_5().flex_1().min_w(px(0.)).h_full().overflow_y_scroll()
                .child(div().text_xl().font_weight(FontWeight::BOLD).child("Edit, sign & redact"))
                .child(div().text_sm().text_color(MUTED).child("Open a PDF, then write on it, draw, add a signature, or black out text for good."))
                .child(drop).child(match &self.status { Status::Failed(m) => div().text_sm().text_color(ERR).child(m.clone()), _ => div() });
        }

        let tools = div().flex().flex_wrap().gap_1().children(Tool::ALL.iter().map(|(t, name)| {
            let t = *t;
            button(SharedString::from(format!("tool-{name}")), *name, self.tool == t).on_click(cx.listener(move |this, _, _, cx| { this.tool = t; cx.notify(); }))
        }));
        // The colour row and text field keep their space for every tool (hidden, not removed) so the page does not jump
        // under the cursor when the tool changes.
        let uses_color = !matches!(self.tool, Tool::Redact);
        let colors = div().flex().flex_wrap().gap_1().when(!uses_color, |d| d.invisible()).children(COLORS.iter().map(|c| {
            let c = *c;
            button(SharedString::from(format!("col-{c}")), c, self.color == c).on_click(cx.listener(move |this, _, _, cx| { this.color = c; cx.notify(); }))
        }));
        let nav = div().flex().items_center().gap_2()
            .child(button("ed-prev", "Prev", false).when(self.page == 0, |b| b.opacity(0.45)).on_click(cx.listener(|t, _, _, cx| t.go(-1, cx))))
            .child(div().text_sm().text_color(MUTED).child(format!("Page {} of {}", self.page + 1, self.sizes.len())))
            .child(button("ed-next", "Next", false).when(self.page + 1 >= self.sizes.len(), |b| b.opacity(0.45)).on_click(cx.listener(|t, _, _, cx| t.go(1, cx))));

        // Everything drawn on this page, plus the shape being dragged right now.
        let mut shown: Vec<Item> = self.items.iter().filter(|i| i.page == self.page).cloned().collect();
        if let (Some((a, b)), false) = (self.drag, self.tool.is_placed()) {
            if let Some(it) = make_item(self.page, self.tool, self.color, a, b, "", size) { shown.push(it); }
            else if self.tool != Tool::Line { let (x, y, w, h) = norm_rect(a.0, a.1, b.0, b.1); shown.push(Item { page: self.page, tool: self.tool, x0: x, y0: y, x1: x + w.max(1.0), y1: y + h.max(1.0), text: String::new(), color: self.color }); }
        }
        let lines: Vec<((f32, f32), (f32, f32), Rgba)> = shown.iter().filter(|i| i.tool == Tool::Line).map(|i| ((i.x0 as f32 * s, i.y0 as f32 * s), (i.x1 as f32 * s, i.y1 as f32 * s), color_rgba(i.color))).collect();
        let boxes = shown.iter().filter(|i| i.tool != Tool::Line).map(|i| {
            let (x, y, w, h) = norm_rect(i.x0, i.y0, i.x1, i.y1);
            let base = div().absolute().left(px(x as f32 * s)).top(px(y as f32 * s));
            match i.tool {
                Tool::Text => base.text_size(px(14.0 * s)).text_color(color_rgba(i.color)).child(i.text.clone()).into_any_element(),
                Tool::Signature => base.w(px(w as f32 * s)).h(px(h as f32 * s)).flex().items_center().justify_center().border_1().border_color(color_rgba(i.color))
                    .text_size(px((h as f32 * 0.55 * s).min(40.0))).italic().text_color(color_rgba(i.color)).child(i.text.clone()).into_any_element(),
                Tool::Redact => base.w(px(w as f32 * s)).h(px(h as f32 * s)).bg(Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.82 }).border_1().border_color(Rgba { r: 0.9, g: 0.2, b: 0.2, a: 1.0 }).into_any_element(),
                Tool::Ellipse => base.w(px(w as f32 * s)).h(px(h as f32 * s)).rounded_full().border_2().border_color(color_rgba(i.color)).into_any_element(),
                _ => base.w(px(w as f32 * s)).h(px(h as f32 * s)).border_2().border_color(color_rgba(i.color)).into_any_element(),
            }
        }).collect::<Vec<_>>();

        let cell = self.bounds.clone();
        let page_view = div().id("page-canvas").relative().w(px(show_w)).h(px(show_h)).bg(Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }).border_1().border_color(BORDER).cursor_crosshair()
            .child(match self.images.get(&self.page) { Some(p) => img(p.clone()).w(px(show_w)).h(px(show_h)).into_any_element(),
                None => div().p_4().text_sm().text_color(Rgba { r: 0.3, g: 0.3, b: 0.3, a: 1.0 }).child("Drawing the page…").into_any_element() })
            .child(canvas(move |b, _, _| cell.set(b), |_, _, _, _| {}).absolute().top(px(0.)).left(px(0.)).size_full())
            .children(boxes)
            .child(canvas(|_, _, _| (), move |b, _, window, _| {
                for (a, c, col) in lines {
                    let mut pb = PathBuilder::stroke(px(2.0));
                    pb.move_to(point(b.origin.x + px(a.0), b.origin.y + px(a.1)));
                    pb.line_to(point(b.origin.x + px(c.0), b.origin.y + px(c.1)));
                    if let Ok(path) = pb.build() { window.paint_path(path, col); }
                }
            }).absolute().top(px(0.)).left(px(0.)).size_full())
            .on_mouse_down(MouseButton::Left, cx.listener(|t, e: &MouseDownEvent, _, cx| t.begin(e.position, cx)))
            .on_mouse_move(cx.listener(|t, e: &MouseMoveEvent, _, cx| t.drag_to(e.position, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|t, e: &MouseUpEvent, _, cx| t.finish(e.position, cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|t, e: &MouseUpEvent, _, cx| t.finish(e.position, cx)));

        let status = match &self.status {
            Status::Idle => div(), Status::Working => div().text_sm().text_color(MUTED).child("Saving…"),
            Status::Failed(m) => div().text_sm().text_color(ERR).child(m.clone()),
            Status::Done(p) => { let p = p.clone(); div().flex().flex_col().gap_1().child(div().text_sm().text_color(OK).child(format!("Saved {}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())))
                .child(div().flex().child(button("ed-reveal", "Show in folder", false).on_click(move |_, _, _| crate::jobs::reveal(&p)))) }
        };
        let n = self.items.len();
        div().id("editor-panel").flex().flex_col().gap_3().p_5().flex_1().min_w(px(0.)).h_full().overflow_y_scroll()
            .child(div().text_xl().font_weight(FontWeight::BOLD).child("Edit, sign & redact"))
            .child(drop).child(tools).child(div().text_sm().text_color(MUTED).child(tool_hint(self.tool)))
            .child(div().when(!self.tool.is_placed(), |d| d.invisible()).child(self.text.clone())).child(colors).child(nav).child(page_view)
            .child(div().flex().gap_2().items_center()
                .child(button("ed-undo", "Undo", false).when(n == 0, |b| b.opacity(0.45)).on_click(cx.listener(|t, _, _, cx| { t.items.pop(); cx.notify(); })))
                .child(button("ed-clear", "Clear all", false).when(n == 0, |b| b.opacity(0.45)).on_click(cx.listener(|t, _, _, cx| { t.items.clear(); cx.notify(); })))
                .child(button("ed-save", "Save edited copy", true).on_click(cx.listener(|t, _, _, cx| t.save(cx))))
                .child(div().text_sm().text_color(MUTED).child(format!("{n} change{}", if n == 1 { "" } else { "s" }))))
            .child(status)
    }
}
