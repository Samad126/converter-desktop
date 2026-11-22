//! Palette (dark, warm neutral). Everything the UI colours comes from here.
use gpui::Rgba;

const fn hex(h: u32) -> Rgba {
    Rgba { r: ((h >> 16) & 255) as f32 / 255.0, g: ((h >> 8) & 255) as f32 / 255.0, b: (h & 255) as f32 / 255.0, a: 1.0 }
}
/// `h` with an alpha, for tinted fills.
pub const fn tint(h: Rgba, a: f32) -> Rgba { Rgba { r: h.r, g: h.g, b: h.b, a } }

pub const BG: Rgba = hex(0x141517);
pub const SURFACE: Rgba = hex(0x1b1d20);
pub const SURFACE_HI: Rgba = hex(0x25282c);
pub const BORDER: Rgba = hex(0x32363c);
pub const TEXT: Rgba = hex(0xecebe6);
pub const MUTED: Rgba = hex(0x8b9098);
pub const ACCENT: Rgba = hex(0x8aa4ff);
pub const ACCENT_HI: Rgba = hex(0xa9bcff);
pub const ON_ACCENT: Rgba = hex(0x10142a);
pub const OK: Rgba = hex(0x6fd49a);
pub const ERR: Rgba = hex(0xff8a80);

/// File families, so a format is recognisable by colour before it is read.
const DOC: Rgba = hex(0x7fb2ff);
const IMAGE: Rgba = hex(0xc79bff);
const MEDIA: Rgba = hex(0xff9f7a);
const DATA: Rgba = hex(0x6fd4c2);
const ARCHIVE: Rgba = hex(0xe0c06a);

/// Colour of a file type: documents blue, images violet, audio/video orange, data teal, archives yellow.
pub fn family_color(ext: &str) -> Rgba {
    let e = ext.trim_start_matches('.').to_ascii_lowercase();
    match e.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "svg" | "ico" | "heic" | "heif" | "avif" | "psd" | "raw" | "exr" | "hdr" | "dds" | "pnm" => IMAGE,
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "opus" | "wma" | "mp4" | "mkv" | "mov" | "avi" | "webm" | "flv" | "wmv" | "m4v" | "gifv" => MEDIA,
        "csv" | "tsv" | "json" | "xml" | "ini" | "yaml" | "yml" | "toml" | "parquet" | "orc" | "feather" | "ndjson" | "xlsx" | "xls" | "ods" => DATA,
        "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "tgz" | "cbz" | "cbr" | "lz4" | "lzma" => ARCHIVE,
        _ => DOC,
    }
}

/// The UI font, bundled with the app so text looks (and bolds) the same on every platform.
pub const FONT: &str = "Noto Sans";
pub const FONT_REGULAR: &[u8] = include_bytes!("../assets/fonts/NotoSans-Regular.ttf");
pub const FONT_BOLD: &[u8] = include_bytes!("../assets/fonts/NotoSans-Bold.ttf");
