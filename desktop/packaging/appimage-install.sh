#!/bin/sh
set -eu
[ -n "${APPIMAGE:-}" ] || { echo 'Run the Secpar AppImage to install it.' >&2; exit 1; }
if [ "${1:-}" = --replace ]; then
    pid=${2:-}
    case "$pid" in ''|*[!0-9]*) exit 2;; esac
    count=0
    while kill -0 "$pid" 2>/dev/null; do
        count=$((count+1))
        [ "$count" -lt 200 ] || { echo 'Close Secpar before updating.' >&2; exit 1; }
        sleep 0.1
    done
fi
data_home=${XDG_DATA_HOME:-"$HOME/.local/share"}
app_home="$data_home/secpar"
install -d -m 755 "$app_home" "$data_home/applications" "$data_home/icons/hicolor/512x512/apps"
# Atomic replacement also works while the previous executable is mapped.
install -m 755 "$APPIMAGE" "$app_home/.Secpar.AppImage-$$"
mv -f "$app_home/.Secpar.AppImage-$$" "$app_home/Secpar.AppImage"
install -m 644 "$APPDIR/com.secpar.desktop.png" "$data_home/icons/hicolor/512x512/apps/com.secpar.desktop.png"
quoted=$(printf '%s' "$app_home/Secpar.AppImage" | sed 's/\\/\\\\/g; s/"/\\"/g; s/`/\\`/g; s/\$/\\$/g; s/%/%%/g')
printf '%s\n' '[Desktop Entry]' 'Type=Application' 'Name=Secpar' "Exec=\"$quoted\"" 'Icon=com.secpar.desktop' 'StartupWMClass=secpar-desktop' 'Terminal=false' 'Categories=Network;RemoteAccess;' > "$data_home/applications/com.secpar.desktop.desktop"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$data_home/applications" >/dev/null 2>&1 || true
command -v kbuildsycoca6 >/dev/null 2>&1 && kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
exec "$app_home/Secpar.AppImage"
