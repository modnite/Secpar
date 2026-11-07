#!/bin/sh
# Private software EGL only. No user display, render node, input or audio mounts.
set -eu
binary=$(realpath "${1:?Native unit-test binary required}")
package=$(realpath "${2:?AppImage runtime required}")
docker run --rm -i --network none --cap-drop ALL --security-opt no-new-privileges \
 --mount "type=bind,src=$binary,dst=/artifacts/test,readonly" \
 --mount "type=bind,src=$package,dst=/artifacts/package,readonly" secpar-wayland-review:local bash -s <<'CONTAINER'
set -eu
review_dir=$(mktemp -d /tmp/secpar-native-video.XXXXXX)
(cd "$review_dir" && /artifacts/package --appimage-extract >/dev/null)
export LD_LIBRARY_PATH="$review_dir/squashfs-root/usr/lib"
export GST_PLUGIN_SYSTEM_PATH_1_0="$LD_LIBRARY_PATH/gstreamer-1.0" GST_PLUGIN_PATH_1_0="$LD_LIBRARY_PATH/gstreamer-1.0"
export GST_PLUGIN_SCANNER="$review_dir/squashfs-root/usr/bin/gst-plugin-scanner" GST_REGISTRY="$review_dir/registry.bin"
export SECPAR_GPU_REVIEW=1 LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe MESA_LOADER_DRIVER_OVERRIDE=llvmpipe EGL_PLATFORM=surfaceless
/artifacts/test --list | grep -q 'video::yuv_shader::egl_test::gpu_nv12_colors_orientation_and_padding_on_gl_and_gles: test'
/artifacts/test --exact video::yuv_shader::egl_test::gpu_nv12_colors_orientation_and_padding_on_gl_and_gles --ignored --nocapture
/artifacts/test video::dmabuf::tests --nocapture
/artifacts/test video::dma_import::tests --nocapture
/artifacts/test video::native_decode::tests --nocapture
CONTAINER
