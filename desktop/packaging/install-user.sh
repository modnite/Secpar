#!/bin/sh
set -eu
src=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
data_home=${XDG_DATA_HOME:-"$HOME/.local/share"}
bin_home=${SECPAR_BIN_HOME:-"$HOME/.local/bin"}
install -d "$bin_home" "$data_home/applications" "$data_home/icons/hicolor/512x512/apps" "$data_home/icons/hicolor/scalable/apps"
install -m 755 "$src/secpar-desktop" "$bin_home/secpar-desktop"
install -m 644 "$src/secpar-icon.png" "$data_home/icons/hicolor/512x512/apps/com.secpar.desktop.png"
if [ -f "$src/secpar-icon.svg" ]; then
    install -m 644 "$src/secpar-icon.svg" "$data_home/icons/hicolor/scalable/apps/com.secpar.desktop.svg"
fi
# Desktop Exec fields require their own quoting; no shell is invoked by the launcher.
quoted=$(printf '%s' "$bin_home/secpar-desktop" | sed 's/\\/\\\\/g; s/"/\\"/g; s/`/\\`/g; s/\$/\\$/g; s/%/%%/g')
printf '%s\n' '[Desktop Entry]' 'Type=Application' 'Name=Secpar' "Exec=\"$quoted\"" 'Icon=com.secpar.desktop' 'StartupWMClass=secpar-desktop' 'Terminal=false' 'Categories=Network;RemoteAccess;' > "$data_home/applications/com.secpar.desktop.desktop"
# Refresh only this user's launchers/icons. Never restart Plasma, GNOME or a service.
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$data_home/applications" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache --force --ignore-theme-index "$data_home/icons/hicolor" >/dev/null 2>&1 || true
fi
if command -v kbuildsycoca6 >/dev/null 2>&1; then
    kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
elif command -v kbuildsycoca5 >/dev/null 2>&1; then
    kbuildsycoca5 --noincremental >/dev/null 2>&1 || true
fi
printf '%s\n' 'Installed Secpar and its launcher icon for this user.'
