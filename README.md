# converter-desktop

Native desktop app (Windows / Linux / macOS) for the converter, written in Rust with
[gpui](https://crates.io/crates/gpui) (no webview, no Node). All conversions come from
[converter-core](https://github.com/Samad126/converter-core).

## Status
Early. Working: Convert page (drop files, pick a target, progress, result saved next to the original),
Engines page (live status; one-click install of pandoc, qpdf, 7zip, yt-dlp, ffmpeg with checksum
verification). Planned: PDF tools screens, settings, system tray, and the Ctrl+drag quick-convert ring.

## Build
Clone this repo **next to** `converter-core` (the dependency is a sibling path for now):

```
git clone https://github.com/Samad126/converter-core
git clone https://github.com/Samad126/converter-desktop
cd converter-desktop && cargo run
```

Linux needs the X11/XCB/xkbcommon development packages (`libxcb1-dev libxkbcommon-x11-dev libxkbcommon-dev
libfontconfig-dev libfreetype-dev libwayland-dev libvulkan-dev` on Debian/Ubuntu).

`vendor/xattr` is a one-line patch of `xattr 0.2.3` (pulled in by gpui) so it compiles against current `libc`
on Linux (`ENOATTR` -> `ENODATA`); drop it once gpui's dependency is updated.
`CONVERTER_ENGINES_DIR` points the app at a different engines folder (handy for testing).
