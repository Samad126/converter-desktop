#!/usr/bin/env bash
# Packs a release build: the binary plus the Python scripts it looks for next to itself (see core's `scripts_dir`).
# usage: packaging.sh <binary> <archive-name-without-extension> <zip|tar.gz>
set -euo pipefail
bin="$1"; name="$2"; kind="$3"
out="dist/$name"; rm -rf dist; mkdir -p "$out"
cp "$bin" "$out/"
cp -r ../core/scripts "$out/scripts"
find "$out/scripts" -name __pycache__ -prune -exec rm -rf {} +
cp README.md "$out/"
cp assets/fonts/NOTO-LICENSE.txt "$out/"
cp ../core/assets/fonts/OFL-DancingScript.txt "$out/" 2>/dev/null || true
if [ "$kind" = zip ]; then (cd dist && 7z a -tzip "$name.zip" "$name" >/dev/null 2>&1 || zip -qr "$name.zip" "$name"); else tar -C dist -czf "dist/$name.tar.gz" "$name"; fi
ls -la dist
