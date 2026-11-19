# converter-desktop

Native desktop app (Windows / Linux / macOS) for the converter, written in Rust with
[gpui](https://crates.io/crates/gpui) (no webview, no Node). All conversions come from
[converter-core](https://github.com/Samad126/converter-core).

## Status
Early. Working: the Ctrl+drag quick-convert ring (below), Convert page (drop files, pick a target, progress, result saved next to the original),
Engines page (live status; one-click install of pandoc, qpdf, 7zip, yt-dlp, ffmpeg with checksum
verification). Planned: PDF tools screens, settings, system tray, and the Ctrl+drag quick-convert ring.

## Quick-convert ring
Hold **Ctrl**, drag files out of a file manager, and a ring of format chips opens under the cursor. Drop on a
chip to convert (result saved next to the original); **Tab** or the **scroll wheel** shows more formats, **Esc**
cancels. Files that need an engine you haven't installed show dimmed, with the install hint in the result.

Linux: X11 only (global input is read through XRecord). On Wayland the ring is unavailable (the app says so on
stderr); use the drop zone or `converter-desktop <files>`. Windows/macOS are not implemented yet.
`converter-desktop --debug-gesture` prints the gestures the listener sees.

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
