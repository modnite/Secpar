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
# Sunshine's tray icon (its only use of Qt) and its WiX installer are not used:
# Secpar has its own tray and installer.
# Optional second argument: Secpar's minimal FFmpeg from build-ffmpeg-windows.sh.
# Without it Sunshine downloads its own prebuilt FFmpeg.
ffmpeg=${2:+-DFFMPEG_PREPARED_BINARIES=$(cygpath -m "$2")}
# Sunshine's web-UI rule runs npm through `cmd /C`, and cmd cannot start a path
# that contains spaces ("C:/Program Files/nodejs/npm.cmd"), which fails the whole
# build on a normal Windows Node installation. Point it at a space-free shim
# that calls the same npm through node, so the rule itself runs unchanged.
npm_opt=""
npm_bin=$(command -v npm 2>/dev/null || true)
if [ -n "$npm_bin" ]; then
  npm_dir=$(dirname "$(cygpath -m "$npm_bin")")
  node_exe="$npm_dir/node.exe"
  npm_cli="$npm_dir/node_modules/npm/bin/npm-cli.js"
  shim=$(cygpath -m "$src")/build/npm.cmd
  case "$shim" in
    *" "*) echo "build-sunshine-windows: the source path contains a space; Sunshine's web-UI rule needs a space-free build folder" >&2; exit 2 ;;
  esac
  if [ -f "$node_exe" ] && [ -f "$npm_cli" ]; then
    mkdir -p "$(dirname "$shim")"
    printf '@echo off\r\n"%s" "%s" %%*\r\n' "$(cygpath -w "$node_exe")" "$(cygpath -w "$npm_cli")" > "$shim"
    # Sunshine looks for node.exe in the same folder as npm.
    cp -f "$node_exe" "$(dirname "$(cygpath -u "$shim")")/node.exe"
    npm_opt="-DNPM=$shim"
  fi
fi
# SECPAR_SMALL=1 builds the engine for size into build-small: optimised for
# size (C at -Os; C++ at -O2, since GCC 16 at -Os calls a std::string
# constructor MSYS2's static libstdc++ does not export), unused sections
# dropped and symbols stripped. Secpar ships that one.
if [ "${SECPAR_SMALL:-}" = 1 ]; then
  cmake -B build-small -G Ninja -S . \
    -DCMAKE_BUILD_TYPE=Release -DBUILD_DOCS=OFF -DBUILD_TESTS=OFF \
    -DSUNSHINE_ENABLE_TRAY=OFF "-DCMAKE_IGNORE_PATH=C:/Program Files/dotnet" $ffmpeg $npm_opt \
    "-DCMAKE_C_FLAGS_RELEASE=-Os -DNDEBUG -ffunction-sections -fdata-sections" \
    "-DCMAKE_CXX_FLAGS_RELEASE=-O2 -DNDEBUG -ffunction-sections -fdata-sections" \
    "-DCMAKE_EXE_LINKER_FLAGS=-s -Wl,--gc-sections"
  ninja -C build-small
  exit 0
fi
cmake -B build -G Ninja -S . \
  -DCMAKE_BUILD_TYPE=Release -DBUILD_DOCS=OFF -DBUILD_TESTS=OFF \
  -DSUNSHINE_ENABLE_TRAY=OFF "-DCMAKE_IGNORE_PATH=C:/Program Files/dotnet" $ffmpeg $npm_opt
ninja -C build
