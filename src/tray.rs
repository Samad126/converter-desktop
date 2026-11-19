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
            Ok(Ok(handle)) => { std::mem::forget(handle); Ok(()) } // dropping the handle would remove the icon; it lives as long as the process
            Ok(Err(e)) => Err(format!("{e}")),
            Err(_) => Err("timed out talking to the tray host".into()),
        }
    }
}

#[cfg(target_os = "linux")]
pub use imp::start;

#[cfg(not(target_os = "linux"))]
pub fn start(_tx: Sender<AppEvent>, _ring: bool) -> Result<(), String> { Err("the tray is not implemented on this platform yet".into()) }

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
