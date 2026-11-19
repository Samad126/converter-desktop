#!/usr/bin/env bash
# Per-user install: copies this folder to ~/.local/share/converter-desktop and adds an application-menu entry.
# Touches nothing outside your home folder; `./install.sh --uninstall` removes everything it added.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
share="${XDG_DATA_HOME:-$HOME/.local/share}"
dest="$share/converter-desktop"
entry="$share/applications/converter-desktop.desktop"
if [ "${1:-}" = "--uninstall" ]; then rm -rf "$dest" "$entry"; echo "Removed $dest and $entry"; exit 0; fi
mkdir -p "$dest" "$share/applications"
cp -r "$here"/. "$dest"/
rm -f "$dest/install.sh"
sed "s|@EXEC@|$dest/converter-desktop|" "$here/converter-desktop.desktop" > "$entry"
echo "Installed to $dest. Start it from your application menu (Utility > Converter) or run: $dest/converter-desktop"
