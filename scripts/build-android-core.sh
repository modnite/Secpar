#!/bin/sh
# Build Secpar's Android library (crates/secpar-android) for 64-bit ARM phones.
# Usage: scripts/build-android-core.sh <output folder for libsecpar_android.so>
# Needs Rust with the aarch64-linux-android target and the Android NDK:
#   ANDROID_NDK_HOME=<ndk folder>, for example <sdk>/ndk/28.2.13676358
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
out=${1:?output folder}
ndk=${ANDROID_NDK_HOME:?ANDROID_NDK_HOME}
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) host=windows-x86_64; ext=.cmd ;;
  Darwin) host=darwin-x86_64; ext= ;;
  *) host=linux-x86_64; ext= ;;
esac
bin="$ndk/toolchains/llvm/prebuilt/$host/bin"
# API 26 is the oldest Android version Secpar supports.
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$bin/aarch64-linux-android26-clang$ext"
export CC_aarch64_linux_android="$bin/aarch64-linux-android26-clang$ext"
export AR_aarch64_linux_android="$bin/llvm-ar"
cd "$here/crates/secpar-android"
# Built for size: the library ships inside the APK on every phone.
export CARGO_PROFILE_RELEASE_OPT_LEVEL=z CARGO_PROFILE_RELEASE_LTO=fat CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 CARGO_PROFILE_RELEASE_STRIP=symbols
cargo build --release --target aarch64-linux-android
target=${CARGO_TARGET_DIR:-target}
mkdir -p "$out"
cp "$target/aarch64-linux-android/release/libsecpar_android.so" "$out/"
echo "built $out/libsecpar_android.so"
