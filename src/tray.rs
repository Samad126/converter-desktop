//! System tray (Linux: StatusNotifierItem over D-Bus). Left-click opens the window; the menu toggles the
//! Ctrl+drag ring and quits. The app keeps running in the tray when the main window is closed.
use crate::AppEvent;
use std::sync::mpsc::Sender;

/// 32×32 ARGB icon drawn in code: a teal ring with a dot, echoing the quick-convert ring. Anti-aliased by coverage.
pub fn icon_argb(size: i32) -> Vec<u8> {
    let c = (size as f32 - 1.0) / 2.0;
    let (outer, inner, dot) = (size as f32 * 0.46, size as f32 * 0.30, size as f32 * 0.11);
    let cover = |d: f32, lo: f32, hi: f32| ((d - lo + 0.5).clamp(0.0, 1.0)) * ((hi - d + 0.5).clamp(0.0, 1.0));
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let d = (((x as f32 - c).powi(2)) + ((y as f32 - c).powi(2))).sqrt();
            let a = (cover(d, inner, outer) + cover(d, -1.0, dot)).min(1.0);
            // ARGB32, network byte order. Teal 0x2dd4bf.
            out.extend_from_slice(&[(a * 255.0) as u8, 0x2d, 0xd4, 0xbf]);
        }
    }
    out
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use ksni::{menu::*, Icon, MenuItem, Tray, TrayMethods};

    struct ConverterTray { tx: Sender<AppEvent>, ring: bool }

    static HANDLE: std::sync::OnceLock<ksni::Handle<ConverterTray>> = std::sync::OnceLock::new();

    /// Keeps the menu checkmark in step when the setting is changed elsewhere (the Settings page).
    pub fn set_ring(on: bool) {
        if let Some(h) = HANDLE.get().cloned() { crate::runtime::get().spawn(async move { h.update(|t| t.ring = on).await; }); }
    }

    impl Tray for ConverterTray {
        fn id(&self) -> String { "converter-desktop".into() }
        fn title(&self) -> String { "Converter".into() }
        fn icon_pixmap(&self) -> Vec<Icon> { vec![Icon { width: 32, height: 32, data: icon_argb(32) }] }
        fn activate(&mut self, _x: i32, _y: i32) { let _ = self.tx.send(AppEvent::OpenMain); }
        fn menu(&self) -> Vec<MenuItem<Self>> {
            vec![
                StandardItem { label: "Open Converter".into(), activate: Box::new(|t: &mut Self| { let _ = t.tx.send(AppEvent::OpenMain); }), ..Default::default() }.into(),
                MenuItem::Separator,
                CheckmarkItem {
                    label: "Quick-convert ring (Ctrl+drag)".into(), checked: self.ring,
                    activate: Box::new(|t: &mut Self| { t.ring = !t.ring; let _ = t.tx.send(AppEvent::SetRing(t.ring)); }),
                    ..Default::default()
                }.into(),
                MenuItem::Separator,
                StandardItem { label: "Quit".into(), icon_name: "application-exit".into(), activate: Box::new(|t: &mut Self| { let _ = t.tx.send(AppEvent::Quit); }), ..Default::default() }.into(),
            ]
        }
    }

    /// Registers the tray icon. `Err` means there is no tray host (or no D-Bus): the caller then quits when the last window closes.
    pub fn start(tx: Sender<AppEvent>, ring: bool) -> Result<(), String> {
        let tray = ConverterTray { tx, ring };
        let spawned = crate::runtime::get().block_on(async { tokio::time::timeout(std::time::Duration::from_secs(4), tray.spawn()).await });
        match spawned {
            Ok(Ok(handle)) => { let _ = HANDLE.set(handle); Ok(()) } // the static keeps the icon alive for the whole process
            Ok(Err(e)) => Err(format!("{e}")),
            Err(_) => Err("timed out talking to the tray host".into()),
        }
    }
}

#[cfg(target_os = "linux")]
pub use imp::{set_ring, start};

#[cfg(not(target_os = "linux"))]
pub use other::{pump, set_ring, start};

/// Windows and macOS: `tray-icon`. The icon must be created on the main thread (we are on it, inside gpui's launch
/// callback) and kept alive there; menu clicks arrive on a global channel that a helper thread forwards to the app.
#[cfg(any(windows, target_os = "macos"))]
mod other {
    use super::*;
    use std::cell::RefCell;
    use std::sync::Mutex;
    use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

    struct Held { _icon: TrayIcon, ring: CheckMenuItem }
    thread_local! { static HELD: RefCell<Option<Held>> = const { RefCell::new(None) }; }
    static PENDING_RING: Mutex<Option<bool>> = Mutex::new(None);

    /// ARGB (our icon) → RGBA (what tray-icon wants).
    fn rgba_icon() -> Result<Icon, String> {
        let argb = icon_argb(32);
        let rgba: Vec<u8> = argb.chunks(4).flat_map(|p| [p[1], p[2], p[3], p[0]]).collect();
        Icon::from_rgba(rgba, 32, 32).map_err(|e| e.to_string())
    }

    pub fn start(tx: Sender<AppEvent>, ring: bool) -> Result<(), String> {
        let (open, ring_item, quit) = (MenuItem::new("Open Converter", true, None), CheckMenuItem::new("Quick-convert ring (Ctrl/Option+drag)", true, ring, None), MenuItem::new("Quit", true, None));
        let menu = Menu::new();
        menu.append_items(&[&open, &PredefinedMenuItem::separator(), &ring_item, &PredefinedMenuItem::separator(), &quit]).map_err(|e| e.to_string())?;
        let icon = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("Converter").with_icon(rgba_icon()?).build().map_err(|e| e.to_string())?;
        let (open_id, ring_id, quit_id) = (open.id().clone(), ring_item.id().clone(), quit.id().clone());
        HELD.with(|h| *h.borrow_mut() = Some(Held { _icon: icon, ring: ring_item.clone() }));
        let tx_menu = tx.clone();
        std::thread::Builder::new().name("tray-menu".into()).spawn(move || {
            for ev in MenuEvent::receiver() {
                let msg = if ev.id == open_id { AppEvent::OpenMain } else if ev.id == quit_id { AppEvent::Quit }
                    else if ev.id == ring_id { AppEvent::SetRing(ring_item.is_checked()) } else { continue };
                if tx_menu.send(msg).is_err() { break; }
            }
        }).map_err(|e| e.to_string())?;
        std::thread::Builder::new().name("tray-click".into()).spawn(move || {
            for ev in TrayIconEvent::receiver() {
                if let TrayIconEvent::Click { button: tray_icon::MouseButton::Left, button_state: tray_icon::MouseButtonState::Up, .. } = ev {
                    if tx.send(AppEvent::OpenMain).is_err() { break; }
                }
            }
        }).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Called by the Settings page (any thread): remembered, applied on the main thread by `pump`.
    pub fn set_ring(on: bool) { *PENDING_RING.lock().unwrap() = Some(on); }

    /// Call regularly from the main thread: applies queued changes to menu items (they are not thread-safe).
    pub fn pump() {
        if let Some(on) = PENDING_RING.lock().unwrap().take() { HELD.with(|h| if let Some(h) = h.borrow().as_ref() { h.ring.set_checked(on); }); }
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod other {
    use super::*;
    pub fn start(_tx: Sender<AppEvent>, _ring: bool) -> Result<(), String> { Err("the tray is not available on this platform".into()) }
    pub fn set_ring(_on: bool) {}
    pub fn pump() {}
}

#[cfg(target_os = "linux")]
pub fn pump() {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn icon_has_the_right_size_and_shape() {
        let d = icon_argb(32);
        assert_eq!(d.len(), 32 * 32 * 4);
        let px = |x: usize, y: usize| &d[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4];
        assert_eq!(px(0, 0)[0], 0, "corners are transparent");
        assert_eq!(px(16, 16)[0], 255, "centre dot is opaque");
        assert!(px(16, 3)[0] > 0 && px(16, 3)[0] < 255 || px(16, 4)[0] == 255, "the ring is drawn near the edge");
        assert_eq!(&px(16, 16)[1..], &[0x2d, 0xd4, 0xbf]);
        assert_eq!(px(16, 10)[0], 0, "the gap between the dot and the ring is transparent");
    }
}
