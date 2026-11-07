#!/bin/sh
# Generated audio only. No user sound devices, microphone, home or desktop access.
set -eu
binary=$(realpath "${1:?Native desktop test binary required}")
package=$(realpath "${2:?AppImage runtime required}")
docker run --rm -i --network none --cap-drop ALL --security-opt no-new-privileges \
 --mount "type=bind,src=$binary,dst=/artifacts/test,readonly" \
 --mount "type=bind,src=$package,dst=/artifacts/package,readonly" secpar-audio-review:local bash -s <<'CONTAINER'
set -eu
review_dir=$(mktemp -d /tmp/secpar-worker-audio.XXXXXX)
export XDG_RUNTIME_DIR="$review_dir/runtime" XDG_CONFIG_HOME="$review_dir/config" XDG_CACHE_HOME="$review_dir/cache"
mkdir -m700 "$XDG_RUNTIME_DIR" "$review_dir/pkg"
(cd "$review_dir/pkg" && /artifacts/package --appimage-extract >/dev/null)
export LD_LIBRARY_PATH="$review_dir/pkg/squashfs-root/usr/lib" SECPAR_AUDIO_REVIEW=1
export GST_PLUGIN_SYSTEM_PATH_1_0="$LD_LIBRARY_PATH/gstreamer-1.0" GST_PLUGIN_PATH_1_0="$LD_LIBRARY_PATH/gstreamer-1.0"
export GST_PLUGIN_SCANNER="$review_dir/pkg/squashfs-root/usr/bin/gst-plugin-scanner" GST_REGISTRY="$review_dir/registry.bin"
export ALSA_CONFIG_PATH="$review_dir/broken-alsa.conf"
printf 'pcm.!default { type secpar_nonexistent_pcm }\n' > "$ALSA_CONFIG_PATH"
pulseaudio -n --daemonize=yes --exit-idle-time=-1 --load="module-native-protocol-unix" --load="module-null-sink sink_name=secpar_review norewinds=1"
/artifacts/test --list | grep -q 'audio_worker::tests::audio_plays_and_recovers_without_any_ui_polling: test'
timeout 6s parec --device=secpar_review.monitor --raw --format=float32le --rate=48000 --channels=2 --latency-msec=10 >"$review_dir/output.raw" &
capture=$!
/artifacts/test --exact audio_worker::tests::audio_plays_and_recovers_without_any_ui_polling --ignored --nocapture
wait "$capture" || test "$?" -eq 124
python3 - "$review_dir/output.raw" <<'PY'
import sys,struct,math
samples=[x[0] for x in struct.iter_unpack('<f',open(sys.argv[1],'rb').read())]
assert all(math.isfinite(s) for s in samples)
blocks=[samples[i:i+9600] for i in range(0,len(samples),9600)]
active=[max(map(abs,b),default=0)>0.01 for b in blocks]
indices=[i for i,on in enumerate(active) if on]
assert len(indices)>=15, 'Not enough real generated audio reached the monitor'
gaps=[(a,b) for a,b in zip(indices,indices[1:]) if b-a>=4]
assert gaps,'The generated silence interval was not observed'
assert any(sum(active[b:])>=6 for a,b in gaps),'Audio did not resume after silence'
assert max(map(abs,samples),default=0)<0.5,'Unexpected clipping'
print('PASS: audio worker rendered generated Opus before/after silence with no UI polling and broken ALSA default.')
PY
CONTAINER
