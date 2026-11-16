#!/bin/sh
# Build Secpar's minimal FFmpeg for the Windows host. Run in MSYS2 UCRT64.
# Usage: scripts/build-ffmpeg-windows.sh <dir with the pinned sources> [output dir]
# The source dir is what scripts/fetch-upstream.sh fills for:
#   ffmpeg amf x264 sunshine-build-deps
# Output (default <source dir>/ffmpeg-min) is passed to the Sunshine build as
# FFMPEG_PREPARED_BINARIES.
#
# Contents: libavcodec, libavutil, libswscale and Sunshine's cbs library, with only
# the encoders the Windows host can use through FFmpeg: AMD (AMF), Intel (Quick
# Sync), Media Foundation and x264 as the software fallback. NVIDIA is not here
# because Sunshine drives NVENC directly on Windows. No decoders, no network, no
# programs.
# Needs MSYS2 packages: make, nasm, diffutils, and mingw-w64-ucrt-x86_64-libvpl.
set -eu
src=$(cd "${1:?give the directory holding the pinned sources}" && pwd)
out=${2:-$src/ffmpeg-min}
[ "${MSYSTEM:-}" = UCRT64 ] || { echo "Run this in the MSYS2 UCRT64 environment." >&2; exit 1; }
jobs=${SECPAR_JOBS:-$(nproc)}
mkdir -p "$out/include/AMF" "$out/lib" "$src/x264-build" "$src/ffmpeg-build"
out=$(cd "$out" && pwd)

# Sunshine's two cbs patches, applied once.
for p in "$src"/sunshine-build-deps/patches/FFmpeg/FFmpeg/cbs/*.patch; do
  if git -C "$src/ffmpeg" apply --check "$p" 2>/dev/null; then
    git -C "$src/ffmpeg" apply "$p"
  else
    git -C "$src/ffmpeg" apply -R --check "$p"   # already applied, or fail here
  fi
done

cp -R "$src/amf/amf/public/include/." "$out/include/AMF/"

cd "$src/x264-build"
"$src/x264/configure" --prefix="$out" --enable-static --disable-cli \
  --disable-opencl --bit-depth=8 --disable-avs --disable-swscale --disable-lavf \
  --disable-ffms --disable-gpac --disable-lsmash
make -j"$jobs"
make install

cd "$src/ffmpeg-build"
PKG_CONFIG_PATH="$out/lib/pkgconfig:${PKG_CONFIG_PATH:-}" "$src/ffmpeg/configure" \
  --prefix="$out" --disable-all --disable-autodetect --disable-iconv \
  --enable-gpl --enable-static --disable-shared \
  --enable-avcodec --enable-avutil --enable-swscale --enable-bsfs \
  --enable-d3d11va --enable-amf --enable-libvpl --enable-mediafoundation --enable-libx264 \
  --enable-encoder=h264_amf,hevc_amf,av1_amf,h264_qsv,hevc_qsv,av1_qsv,h264_mf,hevc_mf,av1_mf,libx264 \
  --disable-debug --disable-doc --disable-programs \
  --extra-cflags="-I$out/include" --extra-ldflags="-L$out/lib" --pkg-config-flags=--static
make -j"$jobs"
make install

# cbs: the same files Sunshine's own recipe compiles into libcbs.a.
mkdir -p cbs-obj
objs=
for f in libavcodec/cbs.c libavcodec/cbs_h2645.c libavcodec/cbs_av1.c libavcodec/cbs_vp8.c \
         libavcodec/cbs_vp9.c libavcodec/cbs_mpeg2.c libavcodec/cbs_jpeg.c libavcodec/cbs_sei.c \
         libavcodec/h264_levels.c libavcodec/h2645_parse.c libavcodec/vp8data.c libavutil/intmath.c; do
  o=cbs-obj/$(basename "$f" .c).o
  gcc -O2 -c -I. -I"$src/ffmpeg" -I"$src/ffmpeg/$(dirname "$f")" -DHAVE_AV_CONFIG_H \
    -Wno-incompatible-pointer-types "$src/ffmpeg/$f" -o "$o"
  objs="$objs $o"
done
rm -f "$out/lib/libcbs.a"
ar rcs "$out/lib/libcbs.a" $objs

# Private headers Sunshine includes for cbs.
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
