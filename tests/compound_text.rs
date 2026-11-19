//! Tests for the `ctext_extra` patch we carry in `vendor/zed-xim` (see Cargo.toml). They live here because that crate
//! is `no_std` and has dev-dependencies, so it cannot run its own tests outside gpui's workspace.
use xim::ctext_extra::decode;

#[test] fn azerbaijani_and_turkish_letters() {
    assert_eq!(decode(b"\x1b-B\xba").as_deref(), Some("ş"));
    assert_eq!(decode(b"\x1b-C\xbb").as_deref(), Some("ğ"));
    assert_eq!(decode(b"\x1b-C\xb9").as_deref(), Some("ı"));
    assert_eq!(decode(b"\x1b-M\xfd").as_deref(), Some("ı"), "ISO 8859-9 (Turkish)");
}
#[test] fn cyrillic_and_mixed_text() {
    assert_eq!(decode(b"\x1b-L\xd0\xd1").as_deref(), Some("аб"));
    assert_eq!(decode(b"ab \x1b-B\xba x").as_deref(), Some("ab ş x"));
}
#[test] fn bare_high_bytes_are_latin1_and_unknown_escapes_are_refused() {
    assert_eq!(decode(&[0xe9]).as_deref(), Some("é"));
    assert_eq!(decode(b"\x1b$)A").as_deref(), None, "multibyte sets are not supported here");
    assert_eq!(decode(b"\x1b-Z\xa1").as_deref(), None, "unknown set");
    assert_eq!(decode(&[0x85]).as_deref(), None, "C1 control");
}
