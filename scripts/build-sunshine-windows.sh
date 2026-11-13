#!/bin/sh
# Build the pinned Sunshine host on Windows. Run from an MSYS2 UCRT64 shell.
# Usage: scripts/build-sunshine-windows.sh <sunshine source dir> [minimal FFmpeg dir]
# The source comes from scripts/fetch-upstream.sh with SECPAR_UPSTREAM_DIR set to a
# short path (the submodules exceed the Windows path limit under a deep directory).
#
# Packages (pacman -S --needed), all with the mingw-w64-ucrt-x86_64- prefix except git:
#   git toolchain cmake ninja boost cppwinrt curl-winssl miniupnpc onevpl openssl opus
#   MinHook nsis libtiff python
# Native Node.js must be installed for the web interface build.
set -eu
src=${1:?give the Sunshine source directory}
[ "${MSYSTEM:-}" = UCRT64 ] || { echo "Run this in the MSYS2 UCRT64 environment." >&2; exit 1; }
# CMake reads the processor type from this variable. Without it the build asks for
# a wrongly named FFmpeg archive and links ARM libraries.
export PROCESSOR_ARCHITECTURE=${PROCESSOR_ARCHITECTURE:-AMD64}
command -v node >/dev/null 2>&1 || export PATH="$PATH:/c/Program Files/nodejs"
cd "$src"
# Sunshine's tray icon (its only use of Qt) and its WiX installer are not used:
# Secpar has its own tray and installer.
# Optional second argument: Secpar's minimal FFmpeg from build-ffmpeg-windows.sh.
# Without it Sunshine downloads its own prebuilt FFmpeg.
ffmpeg=${2:+-DFFMPEG_PREPARED_BINARIES=$(cygpath -m "$2")}
cmake -B build -G Ninja -S . \
  -DCMAKE_BUILD_TYPE=Release -DBUILD_DOCS=OFF -DBUILD_TESTS=OFF \
  -DSUNSHINE_ENABLE_TRAY=OFF "-DCMAKE_IGNORE_PATH=C:/Program Files/dotnet" $ffmpeg
ninja -C build
