//! Palette (dark, original). Everything the UI colours comes from here.
use gpui::Rgba;

const fn hex(h: u32) -> Rgba {
    Rgba { r: ((h >> 16) & 255) as f32 / 255.0, g: ((h >> 8) & 255) as f32 / 255.0, b: (h & 255) as f32 / 255.0, a: 1.0 }
}

pub const BG: Rgba = hex(0x111318);
pub const SURFACE: Rgba = hex(0x1a1d24);
pub const SURFACE_HI: Rgba = hex(0x252932);
pub const BORDER: Rgba = hex(0x2f3441);
pub const TEXT: Rgba = hex(0xe8eaf0);
pub const MUTED: Rgba = hex(0x9097a8);
pub const ACCENT: Rgba = hex(0x2dd4bf);
pub const ACCENT_HI: Rgba = hex(0x5eead4);
pub const ON_ACCENT: Rgba = hex(0x06201d);
pub const OK: Rgba = hex(0x4ade80);
pub const ERR: Rgba = hex(0xf87171);

/// The UI font, bundled with the app so text looks (and bolds) the same on every platform.
pub const FONT: &str = "Noto Sans";
pub const FONT_REGULAR: &[u8] = include_bytes!("../assets/fonts/NotoSans-Regular.ttf");
pub const FONT_BOLD: &[u8] = include_bytes!("../assets/fonts/NotoSans-Bold.ttf");
