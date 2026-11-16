#!/bin/sh
# Build the one Linux Secpar application as an AppImage, with the update
# information the AppImage update tools and Secpar's own updater read.
#
# Usage: build-appimage.sh <Linux package folder> [output folder]
#
# The package folder is an extracted Linux release: secpar, secpar-service,
# libsecpar_app.so, lib\ (Qt and the app's libraries), qt6\ (plugins and QML),
# engine\ and stream\. Everything stays internal to this one file; the engine
# and the stream component are parts of the app, not separate downloads.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
version=$(tr -d '[:space:]' < "$root/version.txt")
package=${1:?Usage: build-appimage.sh <Linux package folder> [output folder]}
out=${2:-$root/dist-desktop}
name=Secpar-Linux-x64.AppImage
# Where secpar.win and the GitHub releases say the update lives. Secpar's own
# updater finds the same asset by name; this line is what AppImageUpdate reads.
update="gh-releases-zsync|modnite|Secpar|latest|$name.zsync"
tools=${SECPAR_BUILD_TOOLS:-"$root/.secpar-build-tools"}

for needed in secpar secpar-service libsecpar_app.so engine/sunshine stream/secpar-stream secpar.svg; do
    [ -e "$package/$needed" ] || { echo "missing $package/$needed" >&2; exit 2; }
done
# The binaries must not carry debug data, and the package's own Qt is what runs.
if command -v readelf >/dev/null 2>&1 && readelf -S "$package/secpar" 2>/dev/null | grep -q '\.debug_'; then
    echo "the package's secpar carries debug sections" >&2
    exit 2
fi

mkdir -p "$out" "$tools"
out=$(CDPATH= cd -- "$out" && pwd)
tools=$(CDPATH= cd -- "$tools" && pwd)
if [ ! -x "$tools/appimagetool.AppImage" ] || [ ! -f "$tools/runtime-x86_64" ]; then
    curl -fLsS https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage \
        -o "$tools/appimagetool.AppImage"
    curl -fLsS https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64 \
        -o "$tools/runtime-x86_64"
fi
printf '%s  %s\n' \
    ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0 "$tools/appimagetool.AppImage" \
    2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d "$tools/runtime-x86_64" \
    | sha256sum -c -
chmod 755 "$tools/appimagetool.AppImage"

work=$(mktemp -d "${TMPDIR:-/tmp}/secpar-appimage-XXXXXX")
trap 'rm -rf "$work"' EXIT
app=$work/Secpar.AppDir
mkdir -p "$app/usr/bin" "$app/usr/lib" "$app/usr/share/applications" \
    "$app/usr/share/icons/hicolor/512x512/apps" "$app/usr/share/icons/hicolor/scalable/apps" \
    "$app/usr/share/licenses/secpar"

# The application, exactly as the Linux package ships it, under usr/lib. The
# window looks for stream/ and engine/ beside itself, so the folder is kept
# whole instead of being split across the tree.
cp -a "$package" "$app/usr/lib/secpar"

# Libraries are also on usr/lib's own path, as in the earlier AppImage: LD_LIBRARY_PATH
# names both, so a library's own dependencies are found either way.
cat > "$app/usr/bin/secpar" <<'SH'
#!/bin/sh
exec "$(dirname -- "$0")/../lib/secpar/start-secpar.sh" "$@"
SH
chmod 755 "$app/usr/bin/secpar"
cp "$root/packaging/linux/appimage-install.sh" "$app/usr/bin/secpar-install"
chmod 755 "$app/usr/bin/secpar-install"

cp "$root/assets/branding/secpar-icon.png" "$app/com.secpar.desktop.png"
cp "$root/assets/branding/secpar-icon.png" "$app/usr/share/icons/hicolor/512x512/apps/com.secpar.desktop.png"
cp "$package/secpar.svg" "$app/usr/share/icons/hicolor/scalable/apps/com.secpar.desktop.svg"

cat > "$app/com.secpar.desktop" <<ENTRY
[Desktop Entry]
Type=Application
Name=Secpar
Comment=Play on your own computers
Exec=secpar
Icon=com.secpar.desktop
Terminal=false
Categories=Network;RemoteAccess;
Keywords=remote;desktop;stream;game;
X-AppImage-Version=$version
ENTRY
cp "$app/com.secpar.desktop" "$app/usr/share/applications/com.secpar.desktop.desktop"

cp "$root/LICENSE.txt" "$app/usr/share/licenses/secpar/Secpar-GPL-3.0.txt"
cp "$root/assets/icons/lucide/LICENSE" "$app/usr/share/licenses/secpar/Lucide-ISC.txt"
cp "$root/packaging/linux/THIRD-PARTY.md" "$app/usr/share/licenses/secpar/THIRD-PARTY.md"

# One font family travels with the app: a system without any fonts (a bare
# container, an install without a desktop) then still shows text. AppRun adds
# the system's own fonts to the same fontconfig file when they exist.
font_dir=$app/usr/share/fonts/truetype/secpar
font_src=/usr/share/fonts/truetype/dejavu
if [ -f "$font_src/DejaVuSans.ttf" ]; then
    mkdir -p "$font_dir"
    cp "$font_src/DejaVuSans.ttf" "$font_src/DejaVuSans-Bold.ttf" "$font_dir/"
    if [ -f /usr/share/doc/fonts-dejavu-core/copyright ]; then
        cp /usr/share/doc/fonts-dejavu-core/copyright "$app/usr/share/licenses/secpar/DejaVu-fonts-copyright.txt"
    fi
else
    echo "note: no DejaVu font in the build image; the package will rely on system fonts" >&2
fi

cat > "$app/AppRun" <<'RUN'
#!/bin/sh
set -eu
APPDIR=${APPDIR:-$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)}
export APPDIR
version="@SECPAR_VERSION@"

# --install installs the AppImage file itself, so it runs from the mounted one.
if [ "${1:-}" = --install ]; then
    shift
    exec "$APPDIR/usr/bin/secpar-install" "$@"
fi

# The parts must not run from the compressed mount: started from there the engine
# stops right after its monitor list. One writable copy is made per version and
# every start, including the background part, runs from it.
if [ -z "${SECPAR_FROM_CACHE:-}" ]; then
    cache_root="${XDG_CACHE_HOME:-${HOME:-/tmp}/.cache}/secpar"
    cache_dir="$cache_root/app-$version"
    if [ ! -x "$cache_dir/AppRun" ]; then
        if mkdir -p "$cache_root" 2>/dev/null; then
            staging="$cache_root/.app-$version.$$"
            rm -rf "$staging"
            if cp -a "$APPDIR/." "$staging" 2>/dev/null && [ -x "$staging/AppRun" ]; then
                rm -rf "$cache_dir" 2>/dev/null || true
                mv -f "$staging" "$cache_dir" 2>/dev/null || rm -rf "$staging"
            else
                rm -rf "$staging" 2>/dev/null || true
            fi
        fi
    fi
    if [ -x "$cache_dir/AppRun" ]; then
        # Copies of other versions are of no further use.
        for old in "$cache_root"/app-*; do
            [ "$old" = "$cache_dir" ] && continue
            [ -d "$old" ] && rm -rf "$old" 2>/dev/null || true
        done
        APPDIR="$cache_dir" SECPAR_FROM_CACHE=1 exec "$cache_dir/AppRun" "$@"
    fi
    echo "Secpar: no writable cache folder, running from the AppImage itself" >&2
fi
package="$APPDIR/usr/lib/secpar"
# The bundled Qt and the app's own libraries first; then whatever the system
# already provides (drivers, Wayland, the audio stack).
export LD_LIBRARY_PATH="$APPDIR/usr/lib:$package/lib:$package/engine/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
# GL belongs to the desktop: its loader pairs with the vendor driver's own
# libGLX_*.so and libEGL_*.so, and a bundled one breaks context creation. The
# copies in gl/ are used only where the machine has no GLVND at all, so that the
# window still loads its libraries on a bare system.
if [ -d "$package/gl" ]; then
    system_gl=$(ldconfig -p 2>/dev/null | sed -n 's/.*=> \(.*libGLX\.so\.0\).*/\1/p' | head -1)
    if [ -z "$system_gl" ]; then
        for dir in /usr/lib/x86_64-linux-gnu /usr/lib64 /usr/lib /lib/x86_64-linux-gnu; do
            if [ -e "$dir/libGLX.so.0" ]; then
                system_gl=$dir/libGLX.so.0
                break
            fi
        done
    fi
    if [ -z "$system_gl" ]; then
        export LD_LIBRARY_PATH="$LD_LIBRARY_PATH:$package/gl"
    fi
fi
export QT_PLUGIN_PATH="$package/qt6/plugins${QT_PLUGIN_PATH:+:$QT_PLUGIN_PATH}"
export QML2_IMPORT_PATH="$package/qt6/qml${QML2_IMPORT_PATH:+:$QML2_IMPORT_PATH}"
export QT_QPA_PLATFORM="${QT_QPA_PLATFORM:-wayland;xcb}"
# The window is a small 2D interface; video is decoded by the stream component,
# not here. A Qt that travels with the app cannot reliably create a GL context
# against the machine's vendor drivers, so the window draws with Qt's software
# renderer by default: it always starts, and the picture in a stream is not
# affected. SECPAR_RENDERER=gl (or setting QT_QUICK_BACKEND yourself) tries
# hardware rendering instead.
if [ -z "${QT_QUICK_BACKEND:-}" ] && [ "${SECPAR_RENDERER:-}" != "gl" ]; then
    export QT_QUICK_BACKEND=software
fi
# Fonts: a machine with its own fontconfig configuration keeps it, and finds its
# own fonts. Only a system without any (a bare container, an install without a
# desktop) is given a small configuration of our own, which points at the font
# family that travels with the app.
if [ ! -e /etc/fonts/fonts.conf ] && [ -z "${FONTCONFIG_FILE:-}" ]; then
    fontconf=$(mktemp -d "${TMPDIR:-/tmp}/secpar-fonts-XXXXXX")
    cat > "$fontconf/fonts.conf" <<FONTS
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <dir>$APPDIR/usr/share/fonts</dir>
  <dir>/usr/share/fonts</dir>
  <dir>/usr/local/share/fonts</dir>
  <dir prefix="xdg">fonts</dir>
  <cachedir>$fontconf/cache</cachedir>
</fontconfig>
FONTS
    export FONTCONFIG_FILE="$fontconf/fonts.conf"
    export FONTCONFIG_PATH="$fontconf"
fi
if [ "${1:-}" = --host ]; then
    exec "$package/start-hosting.sh"
fi
exec "$package/secpar" "$@"
RUN
sed "s/@SECPAR_VERSION@/$version/" "$app/AppRun" > "$app/AppRun.tmp"
mv "$app/AppRun.tmp" "$app/AppRun"
chmod 755 "$app/AppRun"

# appimagetool writes the .zsync beside its working directory, so it runs there.
# shellcheck disable=SC2086
(cd "$out" && env ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$tools/appimagetool.AppImage" \
    --runtime-file "$tools/runtime-x86_64" \
    --no-appstream \
    --updateinformation "$update" \
    "$app" "$out/$name")

if [ ! -f "$out/$name.zsync" ]; then
    echo "appimagetool did not write $name.zsync" >&2
    exit 1
fi
if command -v readelf >/dev/null 2>&1; then
    readelf -p .upd_info "$out/$name" | sed -n 's/^ *\[ *[0-9a-f]*\] *//p' | grep -qx "$update" \
        || { echo "the AppImage does not carry the update information" >&2; exit 1; }
fi
ls -l "$out/$name" "$out/$name.zsync"
sha256sum "$out/$name" "$out/$name.zsync"
