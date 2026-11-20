#!/usr/bin/env bash
# Per-user install: copies this folder to ~/.local/share/converter-desktop and adds an application-menu entry.
# Touches nothing outside your home folder.
#   ./install.sh --uninstall   removes the app, its menu entry and the start-at-login entry (keeps your settings and downloaded engines)
#   ./install.sh --purge       the same, plus settings, downloaded engines and temp files
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
share="${XDG_DATA_HOME:-$HOME/.local/share}"
dest="$share/converter-desktop"
entry="$share/applications/converter-desktop.desktop"
config="${XDG_CONFIG_HOME:-$HOME/.config}"; cache="${XDG_CACHE_HOME:-$HOME/.cache}"
autostart="$config/autostart/converter-desktop.desktop"
case "${1:-}" in
  --uninstall|--purge)
    pkill -f "^$dest/converter-desktop( |\$)" 2>/dev/null || true
    rm -rf "${dest:?}" "${entry:?}" "${autostart:?}"
    echo "Removed $dest, its menu entry and the start-at-login entry"
    if [ "$1" = "--purge" ]; then
      # Exactly these three folders, each named "converter"; nothing else is touched.
      rm -rf "${config:?}/converter" "${share:?}/converter" "${cache:?}/converter"
      echo "Removed settings, downloaded engines and temporary files"
    else
      echo "Kept your settings and downloaded engines. Run with --purge to delete them too."
    fi
    exit 0;;
esac
if [ "$here" = "$dest" ]; then echo "This is the installed copy. Run install.sh from the extracted download, or use --uninstall / --purge." >&2; exit 2; fi
mkdir -p "$dest" "$share/applications"
cp -r "$here"/. "$dest"/
# Keep install.sh inside the install as the uninstaller, so removal still works after the extracted folder is deleted.
cat > "$dest/uninstall.sh" <<'EOS'
#!/usr/bin/env bash
# Usage: uninstall.sh [--purge]   (--purge also deletes settings and downloaded engines)
exec "$(cd "$(dirname "$0")" && pwd)/install.sh" "${1:---uninstall}"
EOS
chmod +x "$dest/uninstall.sh"
sed "s|@EXEC@|$dest/converter-desktop|" "$here/converter-desktop.desktop" > "$entry"
echo "Installed to $dest. Start it from your application menu (Utility > Converter) or run: $dest/converter-desktop"
