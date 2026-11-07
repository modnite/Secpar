#!/bin/sh
set -eu
binary=$(realpath "${1:?Native Linux host test binary required}")
package=$(realpath "${2:?Candidate AppImage required}")
docker run --rm -i --network none --cap-drop ALL --security-opt no-new-privileges \
 --mount "type=bind,src=$binary,dst=/artifacts/host-test,readonly" \
 --mount "type=bind,src=$package,dst=/artifacts/Secpar.AppImage,readonly" \
 secpar-audio-review:local bash -s <<'CONTAINER'
set -eu
review_dir=$(mktemp -d /tmp/secpar-host-audio.XXXXXX)
export XDG_RUNTIME_DIR="$review_dir/runtime"
mkdir -m700 "$XDG_RUNTIME_DIR" "$review_dir/pkg"
(cd "$review_dir/pkg" && /artifacts/Secpar.AppImage --appimage-extract >/dev/null)
export LD_LIBRARY_PATH="$review_dir/pkg/squashfs-root/usr/lib"
export GST_PLUGIN_SYSTEM_PATH_1_0="$LD_LIBRARY_PATH/gstreamer-1.0" GST_PLUGIN_PATH_1_0="$LD_LIBRARY_PATH/gstreamer-1.0"
export GST_PLUGIN_SCANNER="$review_dir/pkg/squashfs-root/usr/bin/gst-plugin-scanner" GST_REGISTRY="$review_dir/registry.bin"
pulseaudio --daemonize=yes --exit-idle-time=-1 -n --load="module-native-protocol-unix" --load="module-null-sink sink_name=secpar_review norewinds=1"
export SECPAR_AUDIO_REVIEW=1
/artifacts/host-test --list | grep -q 'desktop_output_monitor_is_discovered_without_pactl: test'
PATH=/no-external-tools /artifacts/host-test --exact stream::tests::desktop_output_monitor_is_discovered_without_pactl --ignored --nocapture
CONTAINER
