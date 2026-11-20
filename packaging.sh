#!/usr/bin/env bash
# Packs a release build. usage: packaging.sh <binary> <archive-name> <tar.gz|zip|app>
#   tar.gz  Linux: folder with the binary, scripts/, licenses, a desktop entry and a per-user install.sh
#   zip     Windows: folder with the .exe, scripts/ and licenses
#   (the Windows setup.exe is built from the staged zip folder by windows/installer.nsi in the release workflow)
#   app     macOS: Converter.app (binary in Contents/MacOS, scripts in Contents/Resources), ad-hoc signed, zipped with ditto
set -euo pipefail
bin="$1"; name="$2"; kind="$3"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
rm -rf dist; mkdir -p "dist/$name"

licenses() { cp README.md "$1/"; cp assets/fonts/NOTO-LICENSE.txt "$1/"; cp ../core/assets/fonts/OFL-DancingScript.txt "$1/" 2>/dev/null || true; }
scripts()  { cp -r ../core/scripts "$1/scripts"; find "$1/scripts" -name __pycache__ -prune -exec rm -rf {} +; }

case "$kind" in
  app)
    app="dist/$name/Converter.app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    cp "$bin" "$app/Contents/MacOS/converter-desktop"
    scripts "$app/Contents/Resources"; licenses "$app/Contents/Resources"
    cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Converter</string>
  <key>CFBundleDisplayName</key><string>Converter</string>
  <key>CFBundleIdentifier</key><string>com.converter.desktop</string>
  <key>CFBundleExecutable</key><string>converter-desktop</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
    # Not notarised (that needs a paid Apple developer account), but an ad-hoc signature stops the "damaged" error on Apple Silicon.
    codesign --force --deep -s - "$app"
    (cd "dist/$name" && ditto -c -k --keepParent Converter.app "../$name.zip")
    ;;
  zip)
    out="dist/$name"; cp "$bin" "$out/"; scripts "$out"; licenses "$out"
    (cd dist && (7z a -tzip "$name.zip" "$name" >/dev/null 2>&1 || zip -qr "$name.zip" "$name"))
    ;;
  tar.gz)
    out="dist/$name"; cp "$bin" "$out/"; scripts "$out"; licenses "$out"
    cp linux/converter-desktop.desktop linux/install.sh "$out/"; chmod +x "$out/install.sh" "$out/converter-desktop"
    tar -C dist -czf "dist/$name.tar.gz" "$name"
    ;;
  *) echo "unknown kind $kind" >&2; exit 2;;
esac
ls -la dist
