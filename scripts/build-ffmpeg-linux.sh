#!/bin/sh
# Build Secpar's minimal FFmpeg for the Linux host.
# Usage: scripts/build-ffmpeg-linux.sh <dir with the pinned sources> [output dir]
# The source dir holds ffmpeg, x264 and sunshine-build-deps, as for Windows.
# Output (default <source dir>/ffmpeg-min-linux) is passed to the Sunshine build as
# FFMPEG_PREPARED_BINARIES.
#
# Contents: libavcodec, libavutil, libswscale and Sunshine's cbs library, with only
# the encoders a Linux host streams with: VA-API (Intel and AMD), NVENC (NVIDIA)
# and x264 as the software fallback. The software HEVC and AV1 encoders (x265,
# SVT-AV1) are left out: they are too slow for a live stream and made up most of
# the engine's size. No decoders, no network, no programs.
# Needs: a C toolchain, nasm, pkg-config, libva and nv-codec-headers.
set -eu
src=$(cd "${1:?give the directory holding the pinned sources}" && pwd)
out=${2:-$src/ffmpeg-min-linux}
jobs=${SECPAR_JOBS:-$(nproc)}
mkdir -p "$out/lib" "$src/x264-build-linux" "$src/ffmpeg-build-linux"
out=$(cd "$out" && pwd)

for p in "$src"/sunshine-build-deps/patches/FFmpeg/FFmpeg/cbs/*.patch; do
  if git -C "$src/ffmpeg" apply --check "$p" 2>/dev/null; then
    git -C "$src/ffmpeg" apply "$p"
  else
    git -C "$src/ffmpeg" apply -R --check "$p"
  fi
done

cd "$src/x264-build-linux"
"$src/x264/configure" --prefix="$out" --enable-static --enable-pic --disable-cli \
  --disable-opencl --bit-depth=8 --disable-avs --disable-swscale --disable-lavf \
  --disable-ffms --disable-gpac --disable-lsmash
make -j"$jobs"
make install

cd "$src/ffmpeg-build-linux"
PKG_CONFIG_PATH="$out/lib/pkgconfig:${PKG_CONFIG_PATH:-}" "$src/ffmpeg/configure" \
  --prefix="$out" --disable-all --disable-autodetect --disable-iconv \
  --enable-gpl --enable-static --disable-shared --enable-pic \
  --enable-avcodec --enable-avutil --enable-swscale --enable-bsfs \
  --enable-vaapi --enable-libdrm --enable-ffnvcodec --enable-nvenc --enable-libx264 \
  --enable-encoder=h264_vaapi,hevc_vaapi,av1_vaapi,h264_nvenc,hevc_nvenc,av1_nvenc,libx264 \
  --disable-debug --disable-doc --disable-programs \
  --extra-cflags="-I$out/include -Os" --extra-ldflags="-L$out/lib" --pkg-config-flags=--static
make -j"$jobs"
make install

mkdir -p cbs-obj
objs=
for f in libavcodec/cbs.c libavcodec/cbs_h2645.c libavcodec/cbs_av1.c libavcodec/cbs_vp8.c \
         libavcodec/cbs_vp9.c libavcodec/cbs_mpeg2.c libavcodec/cbs_jpeg.c libavcodec/cbs_sei.c \
         libavcodec/h264_levels.c libavcodec/h2645_parse.c libavcodec/vp8data.c libavutil/intmath.c; do
  o=cbs-obj/$(basename "$f" .c).o
  cc -Os -fPIC -c -I. -I"$src/ffmpeg" -I"$src/ffmpeg/$(dirname "$f")" -DHAVE_AV_CONFIG_H \
    -Wno-incompatible-pointer-types "$src/ffmpeg/$f" -o "$o"
  objs="$objs $o"
done
rm -f "$out/lib/libcbs.a"
ar rcs "$out/lib/libcbs.a" $objs

cp config.h "$out/include/"
mkdir -p "$out/include/libavcodec/hevc" "$out/include/libavcodec/x86" "$out/include/libavutil/x86"
for h in av1 cbs_av1 cbs_bsf cbs cbs_h2645 cbs_h264 cbs_h265 cbs_jpeg cbs_mpeg2 cbs_sei cbs_vp8 \
         cbs_vp9 codec_desc codec_id codec_par defs get_bits h264_levels h2645_parse h264 mathops \
         packet sei version_major vlc; do
  cp "$src/ffmpeg/libavcodec/$h.h" "$out/include/libavcodec/"
done
cp "$src/ffmpeg/libavcodec/hevc/hevc.h" "$out/include/libavcodec/hevc/"
cp "$src/ffmpeg/libavcodec/x86/mathops.h" "$out/include/libavcodec/x86/"
for h in attributes attributes_internal intmath; do cp "$src/ffmpeg/libavutil/$h.h" "$out/include/libavutil/"; done
cp "$src/ffmpeg/libavutil/x86/asm.h" "$src/ffmpeg/libavutil/x86/intmath.h" "$out/include/libavutil/x86/"

ls -l "$out/lib"/*.a
