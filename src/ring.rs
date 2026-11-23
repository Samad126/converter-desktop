//! The quick-convert ring: a transparent pop-up under the cursor that appears when a Ctrl+drag carries files.
//! Targets float around a hub as rounded "orbit chips"; drop on one to convert, Tab / wheel for more, Esc to cancel.
use crate::{jobs, theme::*};
use converter_core::{formats, progress, Config};
use gpui::{
    div, point, prelude::*, px, rgba, size, AnyWindowHandle, App, Bounds, Context, DragMoveEvent, ExternalPaths, FontWeight, Global, MouseButton,
    Pixels, Point, SharedString, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub const WIN: f32 = 480.0;
const DISC: f32 = 456.0;
const RADIUS: f32 = 150.0;
const CHIP_W: f32 = 96.0;
const CHIP_H: f32 = 42.0;
const HUB: f32 = 132.0;
pub const PAGE_SIZE: usize = 7;

// ------------------------------------------------------------------ pure logic

/// Most-wanted targets first, so page 1 is what people usually reach for.
const POPULAR: &[&str] = &["pdf", "docx", "png-image", "jpg-image", "webp", "media:mp3", "media:mp4", "gif", "txt", "xlsx", "csv", "json", "html",
    "media:wav", "zip", "epub", "svg", "pptx", "media:webm", "markdown"];

pub fn rank(mut v: Vec<(String, String)>) -> Vec<(String, String)> {
    let pos = |id: &str| POPULAR.iter().position(|p| *p == id).unwrap_or(usize::MAX);
    v.sort_by(|a, b| pos(&a.0).cmp(&pos(&b.0)).then_with(|| a.1.cmp(&b.1)));
    v
}

pub fn page_count(n: usize) -> usize { n.div_ceil(PAGE_SIZE).max(1) }

pub fn page_items<T>(items: &[T], page: usize) -> &[T] {
    let start = (page * PAGE_SIZE).min(items.len());
    &items[start..(start + PAGE_SIZE).min(items.len())]
}

/// Centre of slot `k` of `n` on the orbit, starting at the top and going clockwise.
pub fn slot_center(k: usize, n: usize, radius: f32, cx: f32, cy: f32) -> (f32, f32) {
    let a = -std::f32::consts::FRAC_PI_2 + (k as f32) * std::f32::consts::TAU / (n.max(1) as f32);
    (cx + radius * a.cos(), cy + radius * a.sin())
}

/// Top-left of a `win`-sized window centred on `(x, y)`, pushed back inside `screen` = (left, top, width, height)
/// so the ring is never cut off when the cursor is near a corner or edge.
pub fn clamp_origin(x: f32, y: f32, win: f32, screen: (f32, f32, f32, f32), margin: f32) -> (f32, f32) {
    let fit = |c: f32, lo: f32, len: f32| {
        let (min, max) = (lo + margin, lo + len - win - margin);
        if max < min { lo } else { (c - win / 2.0).clamp(min, max) }
    };
    (fit(x, screen.0, screen.2), fit(y, screen.1, screen.3))
}

pub fn hit(centers: &[(f32, f32)], w: f32, h: f32, p: (f32, f32)) -> Option<usize> {
    centers.iter().position(|(cx, cy)| (p.0 - cx).abs() <= w / 2.0 && (p.1 - cy).abs() <= h / 2.0)
}

/// "3 × JPG" for a homogeneous drop, "3 files" for a mixed one.
pub fn badge(exts: &[String]) -> String {
    let n = exts.len();
    let first = exts.first().map(|e| e.trim_start_matches('.').to_uppercase()).unwrap_or_default();
    if exts.iter().all(|e| e == &exts[0]) { if n == 1 { first } else { format!("{n} × {first}") } } else { format!("{n} files") }
}

// ------------------------------------------------------------------ view

#[derive(Clone, PartialEq)]
enum Phase { Waiting, Choosing, Converting { done: usize, total: usize }, Finished { ok: usize, failed: Vec<String> } }

#[derive(Clone)]
struct Chip { id: String, label: String, missing: Option<&'static str> }

pub struct Ring {
    cfg: Arc<Config>,
    exts: Vec<String>,
    chips: Vec<Chip>,
    page: usize,
    hover: Option<usize>,
    phase: Phase,
    window: Option<AnyWindowHandle>,
}

/// The one open ring, if any (the gesture can only run once at a time).
pub struct Current(pub Option<WindowHandle<Ring>>);
impl Global for Current {}

impl Ring {
    fn new(cfg: Arc<Config>) -> Self { Self { cfg, exts: vec![], chips: vec![], page: 0, hover: None, phase: Phase::Waiting, window: None } }

    /// Called as the drag moves over the window: the first call tells us which files are in flight.
    fn learn_paths(&mut self, paths: &[PathBuf]) {
        if self.phase != Phase::Waiting || paths.is_empty() { return; }
        self.exts = paths.iter().map(|p| formats::extension_of(p)).collect();
        self.chips = jobs::common_targets(&self.exts).into_iter().map(|(id, label)| {
            let missing = self.exts.iter().find_map(|e| jobs::missing_engine(&self.cfg, e, &id));
            Chip { id, label, missing }
        }).collect();
        let order = rank(self.chips.iter().map(|c| (c.id.clone(), c.label.clone())).collect());
        self.chips.sort_by_key(|c| order.iter().position(|(id, _)| *id == c.id));
        self.phase = Phase::Choosing;
    }

    fn visible(&self) -> &[Chip] { page_items(&self.chips, self.page) }

    fn centers(&self) -> Vec<(f32, f32)> {
        let n = self.visible().len();
        (0..n).map(|k| slot_center(k, n, RADIUS, WIN / 2.0, WIN / 2.0)).collect()
    }

    pub fn flip(&mut self, delta: i32, cx: &mut Context<Self>) {
        if self.phase != Phase::Choosing { return; }
        let pages = page_count(self.chips.len()) as i32;
        self.page = (self.page as i32 + delta).rem_euclid(pages) as usize;
        self.hover = None;
        cx.notify();
    }

    /// True while nothing has been dropped yet (the caller may then close the ring).
    pub fn is_idle(&self) -> bool { matches!(self.phase, Phase::Waiting | Phase::Choosing) }

    fn close_later(&self, after: Duration, cx: &mut Context<Self>) {
        let Some(handle) = self.window else { return };
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(after).await;
            let _ = cx.update_window(handle, |_, window, _| window.remove_window());
        }).detach();
    }

    fn drop_on(&mut self, chip: Chip, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if let Some(missing) = chip.missing {
            self.phase = Phase::Finished { ok: 0, failed: vec![format!("{} needs {missing}. Open Converter → Engines to install it.", chip.label)] };
            cx.notify();
            self.close_later(Duration::from_secs(6), cx);
            return;
        }
        let total = paths.len();
        self.phase = Phase::Converting { done: 0, total };
        cx.notify();
        let handles: Vec<_> = paths.into_iter().map(|p| (p.clone(), jobs::spawn(p, chip.id.clone(), progress::noop()))).collect();
        cx.spawn(async move |this, cx| {
            let (mut ok, mut failed) = (0usize, vec![]);
            for (i, (path, h)) in handles.into_iter().enumerate() {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                match h.await {
                    Ok(Ok(_)) => ok += 1,
                    Ok(Err(e)) => failed.push(format!("{name}: {}", e.message)),
                    Err(e) => failed.push(format!("{name}: stopped unexpectedly ({e})")),
                }
                this.update(cx, |t, cx| { t.phase = Phase::Converting { done: i + 1, total }; cx.notify(); }).ok();
            }
            let wait = if failed.is_empty() { Duration::from_millis(2200) } else { Duration::from_secs(7) };
            this.update(cx, |t, cx| { t.phase = Phase::Finished { ok, failed }; cx.notify(); t.close_later(wait, cx); }).ok();
        }).detach();
    }
}

/// Opens the ring centred on a screen position.
/// Windows 11 draws a thin border, rounded corners and a shadow around every top-level window, which shows up as a
/// grey box behind the ring. Ask DWM to drop all three.
#[cfg(windows)]
fn strip_frame(window: &Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    #[link(name = "dwmapi")]
    extern "system" { fn DwmSetWindowAttribute(hwnd: isize, attr: u32, value: *const core::ffi::c_void, size: u32) -> i32; }
    let Ok(h) = HasWindowHandle::window_handle(window) else { return };
    let RawWindowHandle::Win32(w) = h.as_raw() else { return };
    let hwnd = w.hwnd.get();
    let set = |attr: u32, v: u32| unsafe { DwmSetWindowAttribute(hwnd, attr, &v as *const u32 as *const _, 4) };
    set(33, 1);           // DWMWA_WINDOW_CORNER_PREFERENCE = DWMWCP_DONOTROUND
    set(34, 0xFFFF_FFFE); // DWMWA_BORDER_COLOR = DWMWA_COLOR_NONE
    set(2, 1);            // DWMWA_NCRENDERING_POLICY = DWMNCRP_DISABLED (no shadow)
}
#[cfg(not(windows))]
fn strip_frame(_: &Window) {}

pub fn open(x: i32, y: i32, cfg: Arc<Config>, cx: &mut App) {
    close(cx);
    // Keep the whole ring on the monitor the cursor is on (corners and edges would otherwise clip it).
    let (fx, fy) = (x as f32, y as f32);
    let screen = cx.displays().into_iter().map(|d| d.bounds()).find(|b| {
        let (l, t) = (f32::from(b.origin.x), f32::from(b.origin.y));
        fx >= l && fy >= t && fx < l + f32::from(b.size.width) && fy < t + f32::from(b.size.height)
    });
    let (ox, oy) = match screen {
        Some(b) => clamp_origin(fx, fy, WIN, (f32::from(b.origin.x), f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height)), 8.0),
        None => (fx - WIN / 2.0, fy - WIN / 2.0),
    };
    let bounds = Bounds { origin: point(px(ox), px(oy)), size: size(px(WIN), px(WIN)) };
    let opened = cx.open_window(WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        kind: WindowKind::PopUp,
        focus: false,
        is_movable: false,
        is_resizable: false,
        // On Windows gpui's `Transparent` turns on a tinted accent gradient that shows as a grey box around the ring;
        // the swap chain already carries per-pixel alpha, so leave the accent effect off there.
        window_background: if cfg!(windows) { WindowBackgroundAppearance::Opaque } else { WindowBackgroundAppearance::Transparent },
        ..Default::default()
    }, |window, cx| { strip_frame(window); let handle = window.window_handle(); cx.new(|_| { let mut r = Ring::new(cfg); r.window = Some(handle); r }) });
    if let Ok(handle) = opened { cx.set_global(Current(Some(handle))); }
}

pub fn close(cx: &mut App) {
    if let Some(Current(Some(h))) = cx.try_global::<Current>().map(|c| Current(c.0)) {
        let _ = h.update(cx, |_, window, _| window.remove_window());
    }
    cx.set_global(Current(None));
}

fn pos_in_window(p: Point<Pixels>) -> (f32, f32) { (f32::from(p.x), f32::from(p.y)) }

impl Render for Ring {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div().id("ring").relative().size(px(WIN)).font_family(FONT)
            .on_drag_move(cx.listener(|this, ev: &DragMoveEvent<ExternalPaths>, _, cx| {
                this.learn_paths(ev.drag(cx).paths());
                let hover = hit(&this.centers(), CHIP_W, CHIP_H, pos_in_window(ev.event.position));
                if hover != this.hover || this.phase == Phase::Choosing { this.hover = hover; }
                cx.notify();
            }));
        if self.phase == Phase::Waiting { return root; } // fully transparent until we know files are being dragged

        let hub_text: SharedString = match (&self.phase, self.hover.and_then(|i| self.visible().get(i))) {
            (Phase::Choosing, Some(c)) => format!("Release to make {}", c.label).into(),
            (Phase::Choosing, None) => "Drop on a format".into(),
            (Phase::Converting { done, total }, _) => format!("Converting {done}/{total}…").into(),
            (Phase::Finished { ok, failed }, _) if failed.is_empty() => format!("Done · {ok} saved").into(),
            (Phase::Finished { ok, .. }, _) => format!("{ok} saved, some failed").into(),
            _ => "".into(),
        };
        let hub = div().absolute().left(px((WIN - HUB) / 2.0)).top(px((WIN - HUB) / 2.0)).size(px(HUB)).rounded_full().bg(SURFACE_HI).border_2().border_color(BORDER)
            .flex().flex_col().items_center().justify_center().gap_1().px_2().text_center()
            .child(div().text_lg().font_weight(FontWeight::BOLD).text_color(TEXT).child(badge(&self.exts)))
            .child(div().text_xs().text_color(if self.hover.is_some() { ACCENT_HI } else { MUTED }).child(hub_text));

        let mut ring = root.child(div().absolute().left(px((WIN - DISC) / 2.0)).top(px((WIN - DISC) / 2.0)).size(px(DISC)).rounded_full()
            .bg(rgba(0x111318ea)).border_1().border_color(BORDER)).child(hub);

        if self.phase == Phase::Choosing {
            for (k, ((cx_, cy_), chip)) in self.centers().into_iter().zip(self.visible().iter().cloned()).enumerate() {
                let hot = self.hover == Some(k);
                let dim = chip.missing.is_some();
                let c2 = chip.clone();
                ring = ring.child(div().id(SharedString::from(format!("chip-{}", chip.id))).absolute().left(px(cx_ - CHIP_W / 2.0)).top(px(cy_ - CHIP_H / 2.0 - if hot { 3.0 } else { 0.0 }))
                    .w(px(CHIP_W)).h(px(CHIP_H)).rounded_full().flex().items_center().justify_center().text_sm().font_weight(FontWeight::SEMIBOLD)
                    .border_1().border_color(if hot { ACCENT_HI } else { BORDER })
                    .bg(if hot { ACCENT } else { SURFACE }).text_color(if hot { ON_ACCENT } else { TEXT }).when(dim, |d| d.opacity(0.5))
                    .child(chip.label.clone())
                    .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| this.drop_on(c2.clone(), paths.paths().to_vec(), cx))));
            }
            let pages = page_count(self.chips.len());
            if pages > 1 {
                ring = ring.child(div().absolute().left(px(0.)).top(px(WIN / 2.0 + HUB / 2.0 + 14.0)).w(px(WIN)).flex().justify_center().text_xs().text_color(MUTED)
                    .child(format!("Tab or scroll for more formats · {}/{pages}", self.page + 1)));
            }
            ring = ring.child(div().absolute().left(px(0.)).top(px(WIN - 52.0)).w(px(WIN)).flex().justify_center().text_xs().text_color(MUTED).child("Esc to cancel"));
        }
        if let Phase::Finished { failed, .. } = &self.phase {
            if !failed.is_empty() {
                ring = ring.child(div().absolute().left(px(60.)).top(px(WIN / 2.0 + HUB / 2.0 + 12.0)).w(px(WIN - 120.0)).text_xs().text_color(ERR).text_center().child(failed[0].clone()));
            }
        }
        let _ = MouseButton::Left;
        ring
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn t(id: &str, label: &str) -> (String, String) { (id.into(), label.into()) }

    #[test] fn ring_stays_on_screen_in_corners_and_edges() {
        let s = (0.0, 0.0, 1920.0, 1080.0);
        assert_eq!(clamp_origin(960.0, 540.0, 480.0, s, 8.0), (720.0, 300.0), "centred when there is room");
        assert_eq!(clamp_origin(5.0, 5.0, 480.0, s, 8.0), (8.0, 8.0), "top-left corner");
        assert_eq!(clamp_origin(1915.0, 1075.0, 480.0, s, 8.0), (1432.0, 592.0), "bottom-right corner");
        assert_eq!(clamp_origin(960.0, 1070.0, 480.0, s, 8.0).1, 592.0, "bottom edge only moves y");
        let second = (1920.0, 0.0, 1280.0, 720.0);
        assert_eq!(clamp_origin(1925.0, 400.0, 480.0, second, 8.0).0, 1928.0, "second monitor uses its own origin");
        assert_eq!(clamp_origin(100.0, 100.0, 480.0, (0.0, 0.0, 300.0, 300.0), 8.0), (0.0, 0.0), "screen smaller than the ring");
    }

    #[test] fn popular_targets_come_first_then_alphabetical() {
        let r = rank(vec![t("zip", "ZIP"), t("odt", "ODT"), t("pdf", "PDF"), t("bmp", "BMP"), t("docx", "DOCX")]);
        assert_eq!(r.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), ["pdf", "docx", "zip", "bmp", "odt"]);
    }
    #[test] fn paging() {
        let items: Vec<usize> = (0..16).collect();
        assert_eq!(page_count(16), 3);
        assert_eq!(page_count(0), 1);
        assert_eq!(page_items(&items, 0).len(), 7);
        assert_eq!(page_items(&items, 2), &[14, 15]);
        assert!(page_items(&items, 9).is_empty());
    }
    #[test] fn slots_start_at_top_and_go_clockwise() {
        let (x, y) = slot_center(0, 4, 100.0, 240.0, 240.0);
        assert!((x - 240.0).abs() < 1e-3 && (y - 140.0).abs() < 1e-3, "first slot is straight up");
        let (x, y) = slot_center(1, 4, 100.0, 240.0, 240.0);
        assert!((x - 340.0).abs() < 1e-3 && (y - 240.0).abs() < 1e-3, "then to the right");
    }
    #[test] fn slots_never_overlap_with_seven_chips() {
        let c: Vec<_> = (0..PAGE_SIZE).map(|k| slot_center(k, PAGE_SIZE, RADIUS, 0.0, 0.0)).collect();
        for i in 0..c.len() { for j in i + 1..c.len() {
            let overlap_x = (c[i].0 - c[j].0).abs() < CHIP_W; let overlap_y = (c[i].1 - c[j].1).abs() < CHIP_H;
            assert!(!(overlap_x && overlap_y), "chips {i} and {j} overlap");
        } }
    }
    #[test] fn chips_and_hub_fit_inside_the_window_and_dont_touch_the_hub() {
        for k in 0..PAGE_SIZE {
            let (x, y) = slot_center(k, PAGE_SIZE, RADIUS, WIN / 2.0, WIN / 2.0);
            assert!(x - CHIP_W / 2.0 >= 0.0 && x + CHIP_W / 2.0 <= WIN && y - CHIP_H / 2.0 >= 0.0 && y + CHIP_H / 2.0 <= WIN);
            let dist = ((x - WIN / 2.0).powi(2) + (y - WIN / 2.0).powi(2)).sqrt();
            assert!(dist - CHIP_W / 2.0 > HUB / 2.0 - 12.0, "chip {k} crowds the hub");
        }
    }
    #[test] fn hit_testing() {
        let centers = [(100.0, 100.0), (300.0, 100.0)];
        assert_eq!(hit(&centers, 96.0, 42.0, (110.0, 90.0)), Some(0));
        assert_eq!(hit(&centers, 96.0, 42.0, (300.0, 125.0)), None);
        assert_eq!(hit(&centers, 96.0, 42.0, (340.0, 100.0)), Some(1));
    }
    #[test] fn badge_text() {
        assert_eq!(badge(&[".jpg".into()]), "JPG");
        assert_eq!(badge(&[".jpg".into(), ".jpg".into(), ".jpg".into()]), "3 × JPG");
        assert_eq!(badge(&[".jpg".into(), ".png".into()]), "2 files");
    }
}
