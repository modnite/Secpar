#!/bin/sh
# Build the pinned Sunshine host on Arch Linux.
# Usage: scripts/build-sunshine-linux.sh <sunshine source dir>
# The source comes from scripts/fetch-upstream.sh sunshine (with submodules).
# SECPAR_INSTALL_DEPS=1 installs the Arch packages first; use it in a throwaway
# container, not on a machine you care about.
#
# Sunshine's tray icon (its only use of Qt) is off: Secpar has its own tray.
# CUDA is off for now, so NVIDIA capture and direct NVENC are not in this build.
set -eu
src=${1:?give the Sunshine source directory}
if [ "${SECPAR_INSTALL_DEPS:-}" = 1 ]; then
  pacman -Syu --noconfirm --needed \
    appstream-glib avahi base-devel cmake curl git libcap libdrm libevdev libmfx \
    libpulse libva libx11 libxcb libxfixes libxrandr libxtst miniupnpc ninja nodejs \
    npm numactl openssl opus pipewire python-jinja shaderc systemd vulkan-icd-loader wayland
fi
here=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$src"
# Secpar's patches on Sunshine (integrations/gamestream/patches/sunshine): pairing
# bound to a client certificate, the management password read from a pipe, and the
# live resolution endpoint. A patch that neither applies nor is already applied
# stops the build: an engine without them looks like it works until a feature asks.
patches=$here/integrations/gamestream/patches/sunshine
for p in "$patches"/*.patch; do
  if git apply --check "$p" 2>/dev/null; then
    git apply "$p"
  elif git apply -R --check "$p" 2>/dev/null; then
    echo "$(basename "$p") is already applied" >&2
  else
    echo "cannot apply $(basename "$p"): the source is not the pinned Sunshine tree" >&2
    exit 2
  fi
done
# SECPAR_RELOCATABLE=1 makes the host look for its assets in ./usr/local/assets
# under the directory it is started from, so it can run from any folder.
relocatable=${SECPAR_RELOCATABLE:+-DSUNSHINE_BUILD_APPIMAGE=ON}
# SECPAR_FFMPEG=<dir from build-ffmpeg-linux.sh> links Secpar's minimal FFmpeg,
# and SECPAR_SMALL=1 builds for size; Secpar ships that combination.
small=""
if [ -n "${SECPAR_FFMPEG:-}" ]; then small="-DFFMPEG_PREPARED_BINARIES=$SECPAR_FFMPEG"; fi
if [ "${SECPAR_SMALL:-}" = 1 ]; then
  export CFLAGS="-Os -ffunction-sections -fdata-sections" CXXFLAGS="-O2 -ffunction-sections -fdata-sections" LDFLAGS="-s -Wl,--gc-sections"
fi
configure() {
  cmake -B build -G Ninja -S . \
    -DCMAKE_BUILD_TYPE=Release -DBUILD_DOCS=OFF -DBUILD_TESTS=OFF \
    -DSUNSHINE_ENABLE_TRAY=OFF -DSUNSHINE_ENABLE_CUDA=OFF \
    -DSUNSHINE_ENABLE_DRM=ON -DSUNSHINE_ENABLE_KWIN=ON \
    -DSUNSHINE_ENABLE_PORTAL=ON -DSUNSHINE_ENABLE_WAYLAND=ON $relocatable $small
}
# Sunshine downloads Boost 1.89, whose Boost.Locale has an IDE file-grouping line
# that current CMake rejects on Linux. The line has no effect on the build, so it
# is removed from the downloaded copy and the configure step is run again.
locale=build/_deps/boost-src/libs/locale/CMakeLists.txt
configure || {
  [ -f "$locale" ] || exit 1
  sed -i '/^source_group(TREE .*include\/boost\/locale" PREFIX "headers"/d' "$locale"
  configure
}
ninja -C build
