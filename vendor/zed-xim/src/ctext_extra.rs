//! Patch: decoder for the ISO 8859 right-half designations of X11 COMPOUND_TEXT (`ESC - <final>`), which the
//! `xim_ctext` crate does not cover. IBus uses them for e.g. ş (`ESC - B 0xBA`) and ğ ı (`ESC - C 0xBB / 0xB9`).
use crate::ctext_tables::SETS;
use alloc::string::String;

const ESC: u8 = 0x1b;

/// Decodes COMPOUND_TEXT limited to: ASCII in the left half, one ISO 8859 set at a time in the right half
/// (`ESC - F` selects it; Latin-1 until told otherwise), and `ESC ( B` back to ASCII. Anything else → `None`.
pub fn decode(bytes: &[u8]) -> Option<String> {
    let mut out = String::new();
    let mut right: &[char; 96] = &SETS.iter().find(|(f, _)| *f == b'A')?.1;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == ESC {
            match (bytes.get(i + 1), bytes.get(i + 2)) {
                (Some(b'-'), Some(&f)) => { right = &SETS.iter().find(|(s, _)| *s == f)?.1; i += 3; }
                (Some(b'('), Some(b'B')) => i += 3,
                _ => return None,
            }
            continue;
        }
        match b {
            0x00..=0x7f => out.push(b as char),
            0xa0..=0xff => { let c = right[(b - 0xa0) as usize]; if c == '\u{fffd}' { return None; } out.push(c); }
            _ => return None, // C1 controls: not text
        }
        i += 1;
    }
    Some(out)
}
