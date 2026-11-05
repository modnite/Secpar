#!/bin/sh
set -eu
src=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
data_home=${XDG_DATA_HOME:-"$HOME/.local/share"}
bin_home="$HOME/.local/bin"
install -d "$bin_home" "$data_home/applications" "$data_home/icons/hicolor/512x512/apps"
install -m 755 "$src/secpar-desktop" "$bin_home/secpar-desktop"
install -m 644 "$src/secpar-icon.png" "$data_home/icons/hicolor/512x512/apps/com.secpar.desktop.png"
# Desktop Exec fields require their own quoting; no shell is invoked by the launcher.
quoted=$(printf '%s' "$bin_home/secpar-desktop" | sed 's/\\/\\\\/g; s/"/\\"/g; s/`/\\`/g; s/\$/\\$/g; s/%/%%/g')
printf '%s\n' '[Desktop Entry]' 'Type=Application' 'Name=Secpar' "Exec=\"$quoted\"" 'Icon=com.secpar.desktop' 'StartupWMClass=Secpar' 'Terminal=false' 'Categories=Network;RemoteAccess;' > "$data_home/applications/com.secpar.desktop.desktop"
printf '%s\n' 'Installed the Secpar development client and launcher icon for this user.'
