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
cd "$src"
# SECPAR_RELOCATABLE=1 makes the host look for its assets in ./usr/local/assets
# under the directory it is started from, so it can run from any folder.
relocatable=${SECPAR_RELOCATABLE:+-DSUNSHINE_BUILD_APPIMAGE=ON}
configure() {
  cmake -B build -G Ninja -S . \
    -DCMAKE_BUILD_TYPE=Release -DBUILD_DOCS=OFF -DBUILD_TESTS=OFF \
    -DSUNSHINE_ENABLE_TRAY=OFF -DSUNSHINE_ENABLE_CUDA=OFF \
    -DSUNSHINE_ENABLE_DRM=ON -DSUNSHINE_ENABLE_KWIN=ON \
    -DSUNSHINE_ENABLE_PORTAL=ON -DSUNSHINE_ENABLE_WAYLAND=ON $relocatable
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
