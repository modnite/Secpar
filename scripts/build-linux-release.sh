#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
export SECPAR_RELEASE_TAG=${1:?Usage: build-linux-release.sh vX.Y.Z}
tools=${SECPAR_BUILD_TOOLS:-"$root/.secpar-build-tools"}
mkdir -p "$tools"
curl -fLsS https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage -o "$tools/appimagetool.AppImage"
curl -fLsS https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64 -o "$tools/runtime-x86_64"
printf '%s  %s\n' ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0 "$tools/appimagetool.AppImage" 2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d "$tools/runtime-x86_64" | sha256sum -c -
chmod 755 "$tools/appimagetool.AppImage"
export SECPAR_APPIMAGETOOL="$tools/appimagetool.AppImage" SECPAR_APPIMAGE_RUNTIME="$tools/runtime-x86_64"
export CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_PROFILE_RELEASE_STRIP=symbols
cargo build --release --locked --manifest-path desktop/Cargo.toml
python3 desktop/packaging/appimage.py
