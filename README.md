# converter-desktop

Native desktop app (Windows / Linux / macOS) for the converter, written in Rust with
[gpui](https://crates.io/crates/gpui) (no webview, no Node). All conversions come from
[converter-core](https://github.com/Samad126/converter-core).

## Status
Early. Working: the Ctrl+drag quick-convert ring (below), tray, PDF tools page, Convert page (drop files, pick a target, progress, result saved next to the original),
Engines page (live status; one-click install of pandoc, qpdf, 7zip, yt-dlp, ffmpeg with checksum
verification). Planned: PDF tools screens, settings, autostart, Windows/macOS builds.

## Platforms
CI builds and unit-tests every push on Linux, Windows and macOS (gpui, the core library and the app all compile
and pass on each). **Only Linux/X11 has been run by hand**; the rest is compile-and-test verified, not click-tested.

| | Linux (X11) | Windows | macOS |
|---|---|---|---|
| Convert / PDF tools / Engines / Settings pages | tested | builds | builds |
| Quick-convert ring (Ctrl/Option+drag) | tested | implemented, off by default, **never run on real hardware** | implemented, off by default, **never run on real hardware** (needs Accessibility permission) |
| Tray icon, run-in-background | tested (Cinnamon) | implemented (`tray-icon`), compile-checked only | implemented (`tray-icon`), compile-checked only |
| Start with the computer | tested | registry `Run` key (CI-tested against a throwaway key) | LaunchAgent plist (file logic unit-tested) |
| Engine downloads | pandoc, qpdf, 7zip, yt-dlp, ffmpeg, Calibre, **Python tools** | pandoc, qpdf, yt-dlp, ffmpeg, LibreOffice, Calibre, **Python tools** | pandoc, qpdf, 7zip, yt-dlp, LibreOffice, Calibre, **Python tools** |

**One install, no prerequisites.** The conversion core is compiled into the app. What used to need separate installs
(Python and its packages for PDF -> Word/PowerPoint/Excel, Parquet, fonts, compare, redact) is one in-app button:
*Python tools* downloads a private portable Python (~70 MB) and pip-installs the packages into the app's own folder
(~750 MB on disk), checked on all three platforms in CI. LibreOffice (Windows/macOS) and Calibre are in-app downloads
too. Still from your package manager on Linux: LibreOffice, Tesseract (OCR), heif, assimp.

Without a tray, closing the window quits the app. Release packages (binary + the Python scripts it looks for next
to itself) are built by the *Release build* workflow: run it manually for downloadable artifacts, or push a `v*` tag
to attach them to a GitHub pre-release. macOS builds are unsigned (right-click → Open the first time).

## Quick-convert ring
Hold **Ctrl**, drag files out of a file manager, and a ring of format chips opens under the cursor. Drop on a
chip to convert (result saved next to the original); **Tab** or the **scroll wheel** shows more formats, **Esc**
cancels. Files that need an engine you haven't installed show dimmed, with the install hint in the result.

Linux: X11 only (global input is read through XRecord). On Wayland the ring is unavailable (the app says so on
stderr); use the drop zone or `converter-desktop <files>`. Windows/macOS are not implemented yet.
`converter-desktop --debug-gesture` prints the gestures the listener sees.

## PDF tools
15 tools as forms (drop or browse for files, fill the fields, Run): merge, split, extract / remove / reorder pages,
rotate, crop, compress, add / remove password, repair, watermark, page numbers, images to PDF, OCR. Missing engines
(qpdf for passwords, Python packages for OCR) show an **Install** button in the result. Form
filling and compare are separate panels: **Fill form** lists the PDF's own fields as inputs and saves a filled
copy (optionally flattened); **Compare** shows which text lines were removed/added between two PDFs (needs Python +
PyMuPDF). **Edit, sign & redact** shows the page (rendered with poppler) and turns clicks/drags into annotations: text,
rectangles, ellipses, lines, typed signatures (Dancing Script) and redactions that delete the text under them.
Save runs redact, then edit, then sign. Limits: pages are drawn unrotated, freehand and image stamps are not in the UI yet.

## Settings
Output folder (or next to the original), keep-both vs replace when a result exists, the ring on/off, and start
with the computer (Linux: an XDG autostart entry that launches `--background`). Saved to `settings.json` in the
config folder (`CONVERTER_CONFIG_DIR` overrides it) and applied immediately.

## System tray
The app lives in the tray (Linux: StatusNotifierItem over D-Bus, so it needs a tray host: KDE, Cinnamon, XFCE,
GNOME with the AppIndicator extension). Left-click opens the window; the menu has *Open Converter*, a
*Quick-convert ring* toggle and *Quit*. Closing the window keeps the app (and the ring) running. Without a tray
host the app says so and quits when its window closes. `converter-desktop --background` starts hidden in the tray.

## Build
Clone this repo **next to** `converter-core` (the dependency is a sibling path for now):

```
git clone https://github.com/Samad126/converter-core
git clone https://github.com/Samad126/converter-desktop
cd converter-desktop && cargo run
```

Linux needs the X11/XCB/xkbcommon development packages (`libxcb1-dev libxkbcommon-x11-dev libxkbcommon-dev
libfontconfig-dev libfreetype-dev libwayland-dev libvulkan-dev` on Debian/Ubuntu).

Two small vendored patches of gpui's dependencies (drop them once gpui updates):
- `vendor/xattr`: `ENOATTR` -> `ENODATA` so it compiles against current `libc` on Linux.
- `vendor/zed-xim`: the X input-method client panicked on input it could not decode and did not understand the
  ISO 8859 designations IBus uses for ş ğ ı (and Cyrillic, Greek, ...). `src/ctext_extra.rs` decodes them
  (tables generated by `tools/gen-ctext.py`; tests in `tests/compound_text.rs`).
`CONVERTER_ENGINES_DIR` points the app at a different engines folder (handy for testing).
