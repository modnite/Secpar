#!/bin/sh
set -eu
export PATH="/usr/bin:/bin:$PATH"
export PATH="$(cygpath -u "$VCToolsInstallDir")/bin/Hostx64/x64:$PATH"
root=$(cygpath -u "${SECPAR_ENCODER_DEPS_ROOT:?}")
prefix="$root/ffmpeg-win-full"
mkdir -p "$prefix/include/AMF" "$root/ffmpeg-win-full-build"
cp -R "$root/amf-sunshine/amf/public/include/." "$prefix/include/AMF/"
cd "$root/nv-sdk131"
make PREFIX="$prefix" install
export PKG_CONFIG_PATH="$prefix/lib/pkgconfig"
cd "$root/ffmpeg-win-full-build"
"$root/ffmpeg-sunshine/configure" \
  --toolchain=msvc --arch=x86_64 --target-os=win64 \
  --prefix="$prefix" --disable-all --disable-autodetect \
  --enable-avcodec --enable-avutil --enable-static --disable-shared \
  --enable-d3d11va --enable-amf --enable-ffnvcodec --enable-nvenc --enable-libvpl \
  --enable-encoder=h264_amf,hevc_amf,av1_amf,h264_nvenc,hevc_nvenc,av1_nvenc,h264_qsv,hevc_qsv,av1_qsv \
  --disable-debug --disable-doc --disable-programs --disable-network --enable-small \
  --extra-cflags="-I$prefix/include -MT -Gy -Gw" \
  --extra-ldflags="-LIBPATH:$(cygpath -w "$prefix/lib") -OPT:REF" \
  --extra-libs="advapi32.lib ole32.lib" --pkg-config-flags=--static
export NVENC_SDK_1100_INCLUDE="$root/nv-sdk11/include"
export NVENC_SDK_1200_INCLUDE="$root/nv-sdk12/include"
export NVENC_SDK_1300_INCLUDE="$root/nv-sdk13/include"
export NVENC_SDK_1310_INCLUDE="$root/nv-sdk131/include"
make -j"${SECPAR_ENCODER_JOBS:-2}"
make install
