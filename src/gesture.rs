//! The quick-convert gesture: hold Ctrl, press the left button and start dragging → open the ring at the cursor.
//! While it is open: Tab / scroll wheel flip the page, Esc cancels, releasing the button ends the gesture.
//!
//! `Machine` is the platform-independent decision logic (unit-tested). `listen` feeds it from the OS:
//! XRecord on Linux/X11. (Wayland forbids global input capture; macOS needs the Accessibility permission —
//! both fall back to the file manager's right-click menu, see README.)

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Raw {
    ButtonPress { button: u8, x: i32, y: i32, ctrl: bool },
    ButtonRelease { button: u8 },
    Motion { x: i32, y: i32, ctrl: bool },
    Key { key: Key },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Key { Tab, Escape, Other }

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    /// Open the ring centred on this screen position.
    Open { x: i32, y: i32 },
    /// Flip the ring's page (+1 forward, -1 back).
    Page(i32),
    Cancel,
    /// The button went up (the drop, if any, is happening now).
    Released,
}

/// Movement (in pixels) before a Ctrl+press counts as a drag, so a plain click never opens the ring.
pub const DRAG_THRESHOLD: i32 = 8;

#[derive(Default)]
pub struct Machine { armed: Option<(i32, i32)>, open: bool }

impl Machine {
    pub fn feed(&mut self, e: Raw) -> Option<Gesture> {
        match e {
            Raw::ButtonPress { button: 1, x, y, ctrl } => { self.armed = ctrl.then_some((x, y)); None }
            Raw::ButtonPress { button: b @ (4 | 5), .. } if self.open => Some(Gesture::Page(if b == 5 { 1 } else { -1 })),
            Raw::Motion { x, y, ctrl } => {
                let (sx, sy) = self.armed?;
                if self.open { return None; }
                if !ctrl { self.armed = None; return None; }
                ((x - sx).abs() > DRAG_THRESHOLD || (y - sy).abs() > DRAG_THRESHOLD).then(|| { self.open = true; Gesture::Open { x, y } })
            }
            Raw::ButtonRelease { button: 1 } => {
                let was_open = std::mem::take(&mut self.open);
                self.armed = None;
                was_open.then_some(Gesture::Released)
            }
            Raw::Key { key } if self.open => match key {
                Key::Tab => Some(Gesture::Page(1)),
                Key::Escape => { self.open = false; self.armed = None; Some(Gesture::Cancel) }
                Key::Other => None,
            },
            _ => None,
        }
    }
}

#[cfg(target_os = "linux")]
mod x11 {
    use super::*;
    use x11rb::connection::Connection;
    use x11rb::protocol::record::{self, ConnectionExt as _};
    use x11rb::protocol::xproto::ConnectionExt as _;
    use x11rb::wrapper::ConnectionExt as _;
    use x11rb::rust_connection::RustConnection;

    const KEY_PRESS: u8 = 2;
    const BUTTON_PRESS: u8 = 4;
    const BUTTON_RELEASE: u8 = 5;
    const MOTION_NOTIFY: u8 = 6;
    const CONTROL_MASK: u16 = 4;
    const XK_TAB: u32 = 0xff09;
    const XK_ESCAPE: u32 = 0xff1b;

    pub fn parse(ev: &[u8; 32], tab: u8, esc: u8) -> Option<Raw> {
        let ty = ev[0] & 0x7f;
        let detail = ev[1];
        let i16_at = |o: usize| i16::from_le_bytes([ev[o], ev[o + 1]]) as i32;
        let ctrl = u16::from_le_bytes([ev[28], ev[29]]) & CONTROL_MASK != 0;
        let (x, y) = (i16_at(20), i16_at(22));
        match ty {
            BUTTON_PRESS => Some(Raw::ButtonPress { button: detail, x, y, ctrl }),
            BUTTON_RELEASE => Some(Raw::ButtonRelease { button: detail }),
            MOTION_NOTIFY => Some(Raw::Motion { x, y, ctrl }),
            KEY_PRESS => Some(Raw::Key { key: if detail == tab { Key::Tab } else if detail == esc { Key::Escape } else { Key::Other } }),
            _ => None,
        }
    }

    fn keycode_for(conn: &RustConnection, keysym: u32) -> Option<u8> {
        let setup = conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let map = conn.get_keyboard_mapping(min, max - min + 1).ok()?.reply().ok()?;
        let per = map.keysyms_per_keycode as usize;
        map.keysyms.chunks(per).position(|c| c.contains(&keysym)).map(|i| min + i as u8)
    }

    pub fn listen(mut on_gesture: impl FnMut(Gesture) + Send + 'static) -> Result<(), String> {
        // Two connections: one carries the recorded stream, the other configures the context.
        let (ctl, _) = RustConnection::connect(None).map_err(|e| format!("cannot reach the X server: {e}"))?;
        let (data, _) = RustConnection::connect(None).map_err(|e| format!("cannot reach the X server: {e}"))?;
        ctl.record_query_version(1, 13).map_err(|e| e.to_string())?.reply().map_err(|_| "the X server has no RECORD extension".to_string())?;
        let (tab, esc) = (keycode_for(&ctl, XK_TAB).unwrap_or(23), keycode_for(&ctl, XK_ESCAPE).unwrap_or(9));
        let ctx = ctl.generate_id().map_err(|e| e.to_string())?;
        let range = record::Range { device_events: record::Range8 { first: KEY_PRESS, last: MOTION_NOTIFY }, ..Default::default() };
        ctl.record_create_context(ctx, 0, &[record::CS::ALL_CLIENTS.into()], &[range]).map_err(|e| e.to_string())?;
        ctl.sync().map_err(|e| e.to_string())?;

        let mut machine = Machine::default();
        let mut pending: Vec<u8> = vec![];
        for reply in data.record_enable_context(ctx).map_err(|e| e.to_string())? {
            let reply = reply.map_err(|e| e.to_string())?;
            if reply.category != 0 { continue; } // 0 = data from the server (device events)
            pending.extend_from_slice(&reply.data);
            while pending.len() >= 32 {
                let ev: [u8; 32] = pending[..32].try_into().unwrap();
                pending.drain(..32);
                if let Some(g) = parse(&ev, tab, esc).and_then(|r| machine.feed(r)) { on_gesture(g); }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn ev(ty: u8, detail: u8, x: i16, y: i16, state: u16) -> [u8; 32] {
            let mut b = [0u8; 32];
            b[0] = ty; b[1] = detail; b[20..22].copy_from_slice(&x.to_le_bytes()); b[22..24].copy_from_slice(&y.to_le_bytes()); b[28..30].copy_from_slice(&state.to_le_bytes());
            b
        }
        #[test] fn parses_core_events() {
            assert_eq!(parse(&ev(4, 1, 100, -5, 4), 23, 9), Some(Raw::ButtonPress { button: 1, x: 100, y: -5, ctrl: true }));
            assert_eq!(parse(&ev(6, 0, 7, 8, 0), 23, 9), Some(Raw::Motion { x: 7, y: 8, ctrl: false }));
            assert_eq!(parse(&ev(5, 1, 0, 0, 0), 23, 9), Some(Raw::ButtonRelease { button: 1 }));
            assert_eq!(parse(&ev(2, 23, 0, 0, 0), 23, 9), Some(Raw::Key { key: Key::Tab }));
            assert_eq!(parse(&ev(2, 9, 0, 0, 0), 23, 9), Some(Raw::Key { key: Key::Escape }));
            assert_eq!(parse(&ev(2, 40, 0, 0, 0), 23, 9), Some(Raw::Key { key: Key::Other }));
            assert_eq!(parse(&ev(3, 40, 0, 0, 0), 23, 9), None, "key release is ignored");
        }
    }
}

/// Windows and macOS: `rdev` (low-level mouse/keyboard hooks / an event tap). The modifier is Ctrl on Windows and Option
/// on macOS. macOS needs the Accessibility permission; without it the listener cannot start and says so.
#[cfg(any(windows, target_os = "macos"))]
mod rdev_backend {
    use super::*;
    use rdev::{Button, EventType, Key};

    pub fn translate(held: &mut bool, pos: &mut (f64, f64), ev: &EventType) -> Option<Raw> {
        let is_mod = |k: &Key| if cfg!(target_os = "macos") { matches!(k, Key::Alt | Key::AltGr) } else { matches!(k, Key::ControlLeft | Key::ControlRight) };
        match ev {
            EventType::KeyPress(k) if is_mod(k) => { *held = true; None }
            EventType::KeyRelease(k) if is_mod(k) => { *held = false; None }
            EventType::KeyPress(Key::Tab) => Some(Raw::Key { key: super::Key::Tab }),
            EventType::KeyPress(Key::Escape) => Some(Raw::Key { key: super::Key::Escape }),
            EventType::KeyPress(_) => Some(Raw::Key { key: super::Key::Other }),
            EventType::MouseMove { x, y } => { *pos = (*x, *y); Some(Raw::Motion { x: *x as i32, y: *y as i32, ctrl: *held }) }
            EventType::ButtonPress(Button::Left) => Some(Raw::ButtonPress { button: 1, x: pos.0 as i32, y: pos.1 as i32, ctrl: *held }),
            EventType::ButtonRelease(Button::Left) => Some(Raw::ButtonRelease { button: 1 }),
            // Wheel up = previous page, down = next (the same buttons X11 reports).
            EventType::Wheel { delta_y, .. } if *delta_y != 0 => Some(Raw::ButtonPress { button: if *delta_y > 0 { 4 } else { 5 }, x: pos.0 as i32, y: pos.1 as i32, ctrl: *held }),
            _ => None,
        }
    }

    pub fn listen(mut on_gesture: impl FnMut(Gesture) + Send + 'static) -> Result<(), String> {
        std::thread::Builder::new().name("gesture".into()).spawn(move || {
            let (mut machine, mut held, mut pos) = (Machine::default(), false, (0.0, 0.0));
            let r = rdev::listen(move |event| {
                if let Some(g) = translate(&mut held, &mut pos, &event.event_type).and_then(|raw| machine.feed(raw)) { on_gesture(g); }
            });
            if let Err(e) = r {
                eprintln!("gesture listener stopped: {e:?}{}", if cfg!(target_os = "macos") { " (grant Converter the Accessibility permission in System Settings > Privacy & Security)" } else { "" });
            }
        }).map(|_| ()).map_err(|e| e.to_string())
    }
}

/// Global pointer positions are physical pixels on Windows; gpui window bounds are logical (DIPs).
/// Uses the system DPI (exact on the primary monitor; monitors with a different scale may be slightly off).
pub fn to_logical(x: i32, y: i32) -> (i32, i32) {
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        extern "system" { fn GetDpiForSystem() -> u32; }
        let k = 96.0 / (unsafe { GetDpiForSystem() }.max(96) as f32);
        return ((x as f32 * k).round() as i32, (y as f32 * k).round() as i32);
    }
    #[cfg(not(windows))]
    (x, y)
}

/// Starts the global listener on its own thread. Returns an error string when the platform cannot do it
/// (Wayland, missing RECORD extension, unsupported OS) so the UI can say so instead of silently doing nothing.
pub fn listen(on_gesture: impl FnMut(Gesture) + Send + 'static) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_none() {
            return Err("Ctrl+drag needs X11; on Wayland use the file manager's right-click menu or the drop zone.".into());
        }
        std::thread::Builder::new().name("gesture".into()).spawn(move || {
            if let Err(e) = x11::listen(on_gesture) { eprintln!("gesture listener stopped: {e}"); }
        }).map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(any(windows, target_os = "macos"))]
    { rdev_backend::listen(on_gesture) }
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    { let _ = on_gesture; Err("Ctrl+drag is not available on this platform.".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn press(ctrl: bool) -> Raw { Raw::ButtonPress { button: 1, x: 100, y: 100, ctrl } }
    fn mv(x: i32, y: i32, ctrl: bool) -> Raw { Raw::Motion { x, y, ctrl } }

    #[test] fn ctrl_press_then_drag_opens_once_at_cursor() {
        let mut m = Machine::default();
        assert_eq!(m.feed(press(true)), None);
        assert_eq!(m.feed(mv(104, 100, true)), None, "below the threshold");
        assert_eq!(m.feed(mv(120, 103, true)), Some(Gesture::Open { x: 120, y: 103 }));
        assert_eq!(m.feed(mv(300, 300, true)), None, "already open: no second Open");
    }
    #[test] fn plain_drag_without_ctrl_never_opens() {
        let mut m = Machine::default();
        m.feed(press(false));
        assert_eq!(m.feed(mv(500, 500, false)), None);
        assert_eq!(m.feed(mv(500, 500, true)), None, "ctrl pressed later does not retroactively arm");
    }
    #[test] fn releasing_ctrl_before_the_threshold_disarms() {
        let mut m = Machine::default();
        m.feed(press(true));
        assert_eq!(m.feed(mv(103, 100, false)), None);
        assert_eq!(m.feed(mv(300, 300, true)), None);
    }
    #[test] fn release_ends_an_open_gesture_only() {
        let mut m = Machine::default();
        assert_eq!(m.feed(Raw::ButtonRelease { button: 1 }), None);
        m.feed(press(true)); m.feed(mv(150, 150, true));
        assert_eq!(m.feed(Raw::ButtonRelease { button: 1 }), Some(Gesture::Released));
        assert_eq!(m.feed(mv(400, 400, true)), None, "back to idle");
    }
    #[test] fn tab_wheel_and_escape_only_count_while_open() {
        let mut m = Machine::default();
        assert_eq!(m.feed(Raw::Key { key: Key::Tab }), None);
        assert_eq!(m.feed(Raw::ButtonPress { button: 5, x: 0, y: 0, ctrl: false }), None);
        m.feed(press(true)); m.feed(mv(200, 200, true));
        assert_eq!(m.feed(Raw::Key { key: Key::Tab }), Some(Gesture::Page(1)));
        assert_eq!(m.feed(Raw::ButtonPress { button: 5, x: 0, y: 0, ctrl: true }), Some(Gesture::Page(1)));
        assert_eq!(m.feed(Raw::ButtonPress { button: 4, x: 0, y: 0, ctrl: true }), Some(Gesture::Page(-1)));
        assert_eq!(m.feed(Raw::Key { key: Key::Other }), None);
        assert_eq!(m.feed(Raw::Key { key: Key::Escape }), Some(Gesture::Cancel));
        assert_eq!(m.feed(Raw::ButtonRelease { button: 1 }), None, "cancelled gesture does not also report a release");
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test] fn rdev_events_become_the_same_gestures_as_on_x11() {
        use rdev::{Button, EventType, Key as RKey};
        let modk = if cfg!(target_os = "macos") { RKey::Alt } else { RKey::ControlLeft };
        let mut m = Machine::default();
        let (mut held, mut pos) = (false, (0.0, 0.0));
        let mut feed = |e: EventType| rdev_backend::translate(&mut held, &mut pos, &e).and_then(|r| m.feed(r));
        assert_eq!(feed(EventType::MouseMove { x: 100.0, y: 100.0 }), None);
        assert_eq!(feed(EventType::KeyPress(modk)), None);
        assert_eq!(feed(EventType::ButtonPress(Button::Left)), None);
        assert_eq!(feed(EventType::MouseMove { x: 130.0, y: 105.0 }), Some(Gesture::Open { x: 130, y: 105 }));
        assert_eq!(feed(EventType::KeyPress(RKey::Tab)), Some(Gesture::Page(1)));
        assert_eq!(feed(EventType::Wheel { delta_x: 0, delta_y: -1 }), Some(Gesture::Page(1)));
        assert_eq!(feed(EventType::ButtonRelease(Button::Left)), Some(Gesture::Released));
    }
}
