#!/bin/sh
# Build the Linux Secpar 4 release in one run: the window, the app library, the
# background part, the bundled Sunshine engine and Moonlight stream component,
# laid out as the published package, then one AppImage with its .zsync.
#
# Run from the repository root inside the release image:
#
#   scripts/build-secpar4-linux.sh [engine build dir] [stream component] \
#       [output folder] [extra libraries folder]
#
# The defaults are the build box's paths. Build it in the oldest distribution
# that has to run it (secpar-linux-deb, Debian 13, glibc 2.41): everything the
# package carries is then built against that glibc.
#
# Nothing is published that would not run: a missing part, debug sections, a
# missing update line, a package that does not match the AppImage, or an
# AppImage that will not start with the system's own Qt taken away all stop the
# build.
set -eu

engine_build=${1:-/opt/secpar4/src/sunshine/build}
stream=${2:-/opt/secpar4/qt-stream-res/out/secpar-stream}
out=${3:-$PWD/dist-desktop}
extra_libs=${4:-}

repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo"
fail() { echo "$1" >&2; exit 2; }

version=$(tr -d '[:space:]' < version.txt)
case "$version" in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *) fail "version.txt must hold X.Y.Z, found '$version'" ;;
esac
echo "== Secpar $version"

# --- what the build needs -----------------------------------------------------
command -v cmake >/dev/null 2>&1 || fail "cmake is missing"
command -v cargo >/dev/null 2>&1 || fail "cargo is missing"
command -v readelf >/dev/null 2>&1 || fail "readelf is missing"
command -v strip >/dev/null 2>&1 || fail "strip is missing"
if ! command -v ninja >/dev/null 2>&1; then
    if command -v pacman >/dev/null 2>&1; then
        # Only ever inside a throwaway container.
        pacman -Sy --noconfirm --needed --quiet ninja >/dev/null 2>&1 \
            || pacman -S --noconfirm --needed ninja >/dev/null 2>&1 \
            || fail "ninja is missing and could not be installed"
    else
        fail "ninja is missing; install it in the build image"
    fi
fi
qmake=$(command -v qmake6 || echo /usr/lib/qt6/bin/qmake6)
[ -x "$qmake" ] || fail "qmake6 is missing"
qt_version=$("$qmake" -query QT_VERSION 2>/dev/null || true)
qt_plugins=$("$qmake" -query QT_INSTALL_PLUGINS 2>/dev/null || true)
qt_qml=$("$qmake" -query QT_INSTALL_QML 2>/dev/null || true)
echo "   Qt ${qt_version:-unknown} from $("$qmake" -query QT_INSTALL_PREFIX 2>/dev/null || echo '?')"
[ -n "$qt_plugins" ] && [ -d "$qt_plugins" ] || fail "Qt's plugin folder is not known"
[ -n "$qt_qml" ] && [ -d "$qt_qml" ] || fail "Qt's QML folder is not known"

for f in "$engine_build/sunshine" "$engine_build/assets" "$stream"; do
    [ -e "$f" ] || fail "missing $f"
done

# --- the Rust parts ----------------------------------------------------------
echo "== the app library and the background part"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$repo/build-linux/cargo}
cargo build --release --locked --lib --manifest-path crates/secpar-app/Cargo.toml
cargo build --release --locked --manifest-path crates/secpar-service/Cargo.toml
cargo_lib=$CARGO_TARGET_DIR/release/libsecpar_app.so
cargo_service=$CARGO_TARGET_DIR/release/secpar-service
[ -f "$cargo_lib" ] || fail "the app library was not built"
[ -f "$cargo_service" ] || fail "the background part was not built"

# --- the window --------------------------------------------------------------
echo "== the Qt window"
cmake -B build-linux/qt -G Ninja -S desktop-qt \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_PREFIX_PATH=/usr \
    -DCMAKE_BUILD_RPATH='$ORIGIN;$ORIGIN/lib' \
    -DSECPAR_APP_DIR="$CARGO_TARGET_DIR/release"
ninja -C build-linux/qt
window=build-linux/qt/secpar
[ -f "$window" ] || fail "the window was not built"

# --- the package folder ------------------------------------------------------
pkg=$repo/build-linux/pkg/secpar4-qt
echo "== the package folder: $pkg"
rm -rf "$pkg"
mkdir -p "$pkg/stream" "$pkg/lib" "$pkg/qt6/plugins" "$pkg/qt6/qml" "$pkg/engine/lib" "$pkg/engine/usr/local"

install -m 755 "$window" "$pkg/secpar"
install -m 755 "$cargo_lib" "$pkg/libsecpar_app.so"
install -m 755 "$cargo_service" "$pkg/secpar-service"
install -m 755 "$stream" "$pkg/stream/secpar-stream"
install -m 755 "$engine_build/sunshine" "$pkg/engine/sunshine"

cp -a "$engine_build/assets" "$pkg/engine/assets"
if [ -d "$engine_build/usr/local/assets" ]; then
    cp -a "$engine_build/usr/local/assets" "$pkg/engine/usr/local/assets"
else
    # The engine looks for its assets beside itself and under usr/local/assets;
    # without that tree it takes the same folder twice.
    cp -a "$engine_build/assets" "$pkg/engine/usr/local/assets"
fi

# The engine's own two or three libraries: OpenSSL and the UPnP client, with
# whichever sonames this image built them as.
ldd "$pkg/engine/sunshine" 2>/dev/null \
    | sed -n 's/^[[:space:]]*\([^ ][^ ]*\) => \(\/[^ ]*\).*/\1 \2/p' \
    | while read -r name path; do
        case "$name" in
            libssl.so*|libcrypto.so*|libminiupnpc.so*)
                cp -L "$path" "$pkg/engine/lib/$name" ;;
        esac
    done

install -m 644 assets/branding/secpar-icon.svg "$pkg/secpar.svg"
install -m 644 LICENSE.txt "$pkg/LICENSE.txt"
install -m 755 packaging/linux/start-secpar.sh "$pkg/start-secpar.sh"
install -m 755 packaging/linux/start-hosting.sh "$pkg/start-hosting.sh"
cat > "$pkg/README.txt" <<TEXT
Secpar $version for Linux (x86_64). One app with both roles.

  ./start-hosting.sh   the background part: identity, engine, access list
  ./start-secpar.sh    the window: sign in, connect, hosting settings

The window talks to the background part on
\$XDG_RUNTIME_DIR/secpar-service.sock. Service state and the device identity
live in ~/.local/share/Secpar4, the identity itself in the system keychain.
The engine runs from engine/sunshine and writes under the service state.
Tunnel port is UDP 46900.

Qt, the stream component and the engine are bundled. So are the ordinary
runtime libraries the window needs (fonts, image formats, X11). Graphics
drivers, the Wayland server, the audio stack and the GPU drivers come from the
system.
TEXT

# Qt's plugins: the platform plugins (including the offscreen one the start
# check uses), image formats, the TLS backends and the Wayland client helpers
# that belong to Qt. The KDE decoration plugins and the eglfs/kms backends stay
# with the desktop: they are built for the machine's driver stack, not for a
# window on Wayland or X11.
for d in platforms imageformats iconengines tls wayland-graphics-integration-client \
         wayland-shell-integration xcbglintegrations; do
    if [ -d "$qt_plugins/$d" ]; then
        cp -a "$qt_plugins/$d" "$pkg/qt6/plugins/"
    else
        echo "note: Qt has no plugin folder $d in this image" >&2
    fi
done
[ -f "$pkg/qt6/plugins/platforms/libqoffscreen.so" ] || fail "the offscreen platform plugin is missing"
[ -f "$pkg/qt6/plugins/platforms/libqxcb.so" ] || fail "the X11 platform plugin is missing"

# The QML modules the interface imports, with what they pull in: QtQuick and its
# Controls, Layouts and Dialogs, Qt.labs.platform, QtQml, QtCore and Qt.
for m in Qt QtCore QtQml QtQuick; do
    [ -d "$qt_qml/$m" ] || fail "QML module $m is missing from the image"
    cp -a "$qt_qml/$m" "$pkg/qt6/qml/"
done
for m in QtQuick/Controls QtQuick/Layouts QtQuick/Dialogs QtQuick/Window Qt/labs/platform; do
    [ -d "$pkg/qt6/qml/$m" ] || fail "QML module $m is missing from the package"
done

# --- the libraries, from ldd -------------------------------------------------
# Everything the programs, plugins and QML modules link against is bundled,
# except what belongs to the machine: the C runtime and compiler runtime, the
# graphics driver stack (GL and its GLVND dispatch libraries, GLES, Vulkan,
# DRM, GBM, VA-API) and the audio stack (ALSA, Pulse, PipeWire, GStreamer).
# GL must come from the machine: a bundled GLX/EGL loader cannot pair with the
# vendor driver's own libGLX_*.so/libEGL_*.so, and context creation then fails.
# X11, xcb, the fonts and the image libraries are ordinary runtime libraries and
# a bare system has none of them, so they are carried here.
echo "== the libraries the package needs"
excluded='^(ld-linux|libc\.so|libm\.so|libpthread|libdl\.so|librt\.so|libresolv|libutil\.so|libstdc\+\+|libgcc_s|libGL|libEGL|libGLX|libGLES|libOpenGL|libGLdispatch|libvulkan|libdrm|libgbm|libva|libasound|libpulse|libpipewire|libgst|libwayland-client|libwayland-cursor|libwayland-egl|libwayland-server|libKWaylandClient|libQt6EglFS|libQt6Kms)'
ldd_of() {
    for f in "$@"; do
        ldd "$f" 2>/dev/null | sed -n 's/^[[:space:]]*\([^ ][^ ]*\) => \(\/[^ ]*\).*/\1 \2/p'
    done
}
# Three rounds take in the copies' own dependencies as well.
round=1
while [ "$round" -le 5 ]; do
    before=$(ls "$pkg/lib" | wc -l)
    # shellcheck disable=SC2046
    ldd_of "$pkg/secpar" "$pkg/secpar-service" "$pkg/libsecpar_app.so" "$pkg/stream/secpar-stream" \
           "$pkg/engine/sunshine" \
           $(find "$pkg/qt6" "$pkg/lib" -name '*.so*' -type f) \
        | sort -u \
        | while read -r name path; do
            [ -f "$pkg/lib/$name" ] && continue
            if echo "$name" | grep -Eq "$excluded"; then
                continue
            fi
            case "$name" in
                libQt6*|libSDL2*|libX*|libxcb*) ;;
                *)
                    # Only libraries the image resolves; a "not found" one is
                    # reported below, never guessed.
                    [ -f "$path" ] || continue ;;
            esac
            cp -L "$path" "$pkg/lib/$name"
        done
    after=$(ls "$pkg/lib" | wc -l)
    [ "$before" = "$after" ] && break
    round=$((round + 1))
done
# Anything still missing is reported by the start check at the end.
libs=$(ls "$pkg/lib" | wc -l)
[ "$libs" -ge 20 ] || fail "only $libs libraries were resolved; the package would not run"
echo "   $libs libraries"

# The GLVND dispatch libraries are carried apart, in gl/, and AppRun adds them to
# the search path only on a machine that has no GL at all: the offscreen software
# backend still has to load the library that the window links. On a desktop the
# machine's own GLX/EGL is used, so the vendor driver's pairing is untouched.
mkdir -p "$pkg/gl"
for name in libGLX.so.0 libOpenGL.so.0 libEGL.so.1 libGLdispatch.so.0; do
    path=$(ldconfig -p 2>/dev/null | sed -n "s/.*=> \\(.*\\/$name\\).*/\\1/p" | head -1)
    [ -z "$path" ] && [ -e "/usr/lib/x86_64-linux-gnu/$name" ] && path=/usr/lib/x86_64-linux-gnu/$name
    [ -n "$path" ] && cp -L "$path" "$pkg/gl/$name"
done
[ -e "$pkg/gl/libGLX.so.0" ] || fail "no GLVND dispatch library in the image for the fallback"
echo "   $(ls "$pkg/gl" | wc -l) GLVND libraries in gl/ (fallback only)"
if [ -n "$extra_libs" ]; then
    for f in "$extra_libs"/*.so*; do
        [ -e "$f" ] || continue
        [ -f "$pkg/lib/$(basename "$f")" ] || cp -L "$f" "$pkg/lib/"
    done
fi

# --- release, not a debug build ---------------------------------------------
echo "== checks"
for f in secpar secpar-service libsecpar_app.so stream/secpar-stream engine/sunshine; do
    strip --strip-all "$pkg/$f"
    if readelf -S "$pkg/$f" 2>/dev/null | grep -q '\.debug_'; then
        fail "$f carries debug sections"
    fi
done

# --- the machine's graphics and Wayland stack never travels ------------------
# Mesa, the GLVND loaders, the KMS/GBM helpers, Vulkan, VA-API and the KDE
# Wayland client belong to the desktop: a vendor driver only works with the
# loader and the EGL/GLX modules built with it. A copy from the build image made
# the packaged engine fail with "Couldn't open EGL display: EGL_BAD_PARAMETER"
# on the owner's laptop. Whatever copied them, they are removed here, and the
# build fails if any of them is still in the package.
echo "== keeping the machine's graphics stack out of the package"
find "$pkg" -type f | while read -r f; do
    case "$f" in
        "$pkg"/gl/*) continue ;;
    esac
    base=$(basename "$f")
    if echo "$base" | grep -Eq '^(libEGL|libGLX|libGLES|libOpenGL|libGLdispatch|libgbm|libdrm|libvulkan|libva|libwayland-client|libwayland-cursor|libwayland-egl|libwayland-server|libKWaylandClient|libQt6EglFS|libQt6Kms)'; then
        rm -f "$f"
    fi
done
rm -rf "$pkg/lib/gbm" "$pkg/lib/dri" "$pkg/qt6/plugins/egldeviceintegrations" "$pkg/qt6/plugins/generic"
# gl/ is the exception and stays: AppRun uses it only where the machine has no
# libGLX.so.0 at all, so that a bare system can still load the window.
left=$(find "$pkg" -path "$pkg/gl" -prune -o \( -name 'libEGL*' -o -name 'libGLX*' -o -name 'libGLES*' -o -name 'libOpenGL*' \
    -o -name 'libGLdispatch*' -o -name 'libgbm*' -o -name 'libdrm*' -o -name 'libvulkan*' \
    -o -name 'libva*' -o -name 'libwayland-client*' -o -name 'libwayland-cursor*' \
    -o -name 'libwayland-egl*' -o -name 'libwayland-server*' \
    -o -name 'libKWaylandClient*' -o -name 'libQt6EglFS*' -o -name 'libQt6Kms*' \
    -o -name 'gbm' -o -name 'dri' \) -print | head -5)
[ -z "$left" ] || fail "the package carries the machine's graphics stack: $left"
echo "   no Mesa, GLVND, DRM, GBM, Vulkan, VA-API, Wayland client or KDE Wayland library in the package"

# --- nothing may be half bundled ---------------------------------------------
# A bundled library that needs a system library of another version breaks on the
# machine that has it (Debian's ngtcp2 core with CachyOS's crypto module did).
# Each program must resolve every dependency either inside the bundle or from
# the short list that stays with the machine, and no dependency may have an
# undefined symbol when the bundle is the only place to look.
echo "== resolving every dependency"
checks=$repo/build-linux/checks
rm -rf "$checks"
mkdir -p "$checks"
allowed='^(ld-linux|libc\.so|libm\.so|libpthread|libdl\.so|librt\.so|libresolv|libutil\.so|libstdc\+\+|libgcc_s|libGL\.so|libGLX|libOpenGL|libEGL|libGLdispatch|libGLES|libvulkan|libdrm|libgbm|libva|libasound|libpulse|libpipewire|libgst|libwayland-client|libwayland-cursor|libwayland-egl|libwayland-server|libplacebo|libavcodec|libavutil|libavformat|libswscale|libswresample|libSDL2|libvdpau|libKWaylandClient|libQt6EglFS|libQt6Kms)'
: > "$checks/outside"
for f in secpar secpar-service libsecpar_app.so stream/secpar-stream engine/sunshine; do
    log=$checks/$(echo "$f" | tr '/' '_').ldd
    LD_LIBRARY_PATH="$pkg/lib:$pkg/engine/lib:$pkg/gl" LD_BIND_NOW=1 ldd -r "$pkg/$f" > "$log" 2>&1 || true
    # The stream component's rendering and decoding libraries (libplacebo,
    # FFmpeg, SDL2) belong to the machine, like GL, and the component is built
    # against the Arch stack it runs on; its symbols are checked there, not
    # against this image's versions. Everything else must be complete here.
    if [ "$f" != "stream/secpar-stream" ] && grep -q 'undefined symbol' "$log"; then
        fail "$f has an undefined symbol: $(grep -m1 'undefined symbol' "$log")"
    fi
    if grep -qE 'not found' "$log"; then
        fail "$f has an unresolved library: $(grep -m1 'not found' "$log")"
    fi
    sed -n 's/^[[:space:]]*\([^ ][^ ]*\) => \(\/[^ ]*\).*/\1 \2/p' "$log" \
        | while read -r name path; do
            case "$path" in
                "$pkg"/*) continue ;;
            esac
            echo "$name" | grep -Eq "$allowed" || echo "$f: $name comes from $path"
        done >> "$checks/outside"
done
if [ -s "$checks/outside" ]; then
    head -5 "$checks/outside" >&2
    fail "the package does not carry a complete set of libraries"
fi
echo "   every dependency is in the package or on the machine's side of the line"

# --- the AppImage ------------------------------------------------------------
echo "== the AppImage"
mkdir -p "$out"
out=$(CDPATH= cd -- "$out" && pwd)
sh packaging/linux/build-appimage.sh "$pkg" "$out"
app=$out/Secpar-Linux-x64.AppImage
zsync_file=$app.zsync
[ -f "$app" ] || fail "no AppImage was built"
[ -f "$zsync_file" ] || fail "no .zsync was built"
readelf -p .upd_info "$app" 2>/dev/null \
    | grep -q 'gh-releases-zsync|modnite|Secpar|latest|Secpar-Linux-x64.AppImage.zsync' \
    || fail "the AppImage does not carry its update information"

# --- the AppImage holds what was built --------------------------------------
work=$(mktemp -d "${TMPDIR:-/tmp}/secpar-check-XXXXXX")
trap 'rm -rf "$work"' EXIT INT TERM
mkdir -p "$work/extract"
(
    cd "$work/extract"
    APPIMAGE_EXTRACT_AND_RUN=1 "$app" --appimage-extract >/dev/null 2>&1
)
for f in secpar secpar-service libsecpar_app.so stream/secpar-stream engine/sunshine; do
    inside=$work/extract/squashfs-root/usr/lib/secpar/$f
    [ -f "$inside" ] || fail "the AppImage has no $f"
    [ "$(sha256sum "$pkg/$f" | cut -d' ' -f1)" = "$(sha256sum "$inside" | cut -d' ' -f1)" ] \
        || fail "the AppImage's $f is not the one just built"
done

# --- it has to start without the image's own Qt -----------------------------
if [ -d "$qt_qml" ] && [ "${SECPAR_SKIP_START_CHECK:-0}" != "1" ]; then
    echo "== starting it with the image's Qt taken away"
    mv "$qt_qml" "$qt_qml.system-off"
    restore() {
        [ -d "$qt_qml.system-off" ] && mv "$qt_qml.system-off" "$qt_qml"
        [ -d "$qt_plugins.system-off" ] && mv "$qt_plugins.system-off" "$qt_plugins"
    }
    trap 'restore; rm -rf "$work"' EXIT INT TERM
    [ -d "$qt_plugins" ] && mv "$qt_plugins" "$qt_plugins.system-off"
    if ! APPIMAGE_EXTRACT_AND_RUN=1 QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software \
            QT_PLUGIN_PATH=/nonexistent QML2_IMPORT_PATH=/nonexistent \
            "$app" --sample --size 640x400 --screenshot "$work/started.png" --screenshot-delay 2500; then
        fail "the AppImage did not start with the system's Qt taken away"
    fi
    [ -s "$work/started.png" ] || fail "the AppImage started but drew nothing"
    cp "$work/started.png" "$out/started-640x400.png"
    restore
    trap 'rm -rf "$work"' EXIT INT TERM
fi

echo "== done"
echo "package: $(find "$pkg" -type f | wc -l) files, $(du -sh "$pkg" | cut -f1)"
ls -l "$app" "$zsync_file"
sha256sum "$app" "$zsync_file"
