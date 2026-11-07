#!/bin/sh
# Only generated audio inside an isolated container. No host sound, home or devices mounted.
set -eu
old=$(realpath "${1:?Usage: test-audio-output.sh previous.AppImage candidate.AppImage desktop-unit-test-binary}")
candidate=$(realpath "${2:?Candidate AppImage required}")
unit=$(realpath "${3:?Native Linux unit-test executable required}")
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
docker build -t secpar-audio-review:local -f "$root/desktop/packaging/audio-smoke.Dockerfile" "$root/desktop/packaging"
docker run --rm -i --network none --cap-drop ALL --security-opt no-new-privileges \
  --mount "type=bind,src=$old,dst=/artifacts/old.AppImage,readonly" \
  --mount "type=bind,src=$candidate,dst=/artifacts/candidate.AppImage,readonly" \
  --mount "type=bind,src=$unit,dst=/artifacts/desktop-test,readonly" \
  secpar-audio-review:local bash -s <<'CONTAINER'
set -eu
export HOME=$(mktemp -d /tmp/secpar-audio.XXXXXX)
export XDG_RUNTIME_DIR="$HOME/runtime"
mkdir -m 700 "$XDG_RUNTIME_DIR"
mkdir "$HOME/old" "$HOME/candidate"
(cd "$HOME/old" && /artifacts/old.AppImage --appimage-extract >/dev/null)
(cd "$HOME/candidate" && /artifacts/candidate.AppImage --appimage-extract >/dev/null)
export SECPAR_AUDIO_REVIEW=1 ALSA_CONFIG_PATH="$HOME/alsa.conf"
unset ALSA_PLUGIN_DIR
printf 'pcm.!default { type pulse }\nctl.!default { type pulse }\n' > "$ALSA_CONFIG_PATH"
pulseaudio -n --daemonize=yes --exit-idle-time=-1 --log-target="file:$HOME/pulse.log" \
  --load="module-native-protocol-unix auth-anonymous=1" \
  --load="module-null-sink sink_name=secpar_review norewinds=1"
# norewinds uses a 50ms virtual sink instead of its artificial two-second default:
# https://github.com/pulseaudio/pulseaudio/blob/master/src/modules/module-null-sink.c
# Keep product device buffering unchanged; this setting belongs only to the fixture.
for n in $(seq 1 30); do pactl info >/dev/null 2>&1 && break; sleep 0.1; done
pactl set-default-sink secpar_review
case_name=audio::device_tests::renders_opus_through_system_audio_output
if LD_LIBRARY_PATH="$HOME/old/squashfs-root/usr/lib" /artifacts/desktop-test \
    --exact "$case_name" --ignored --nocapture >"$HOME/old.log" 2>&1; then
  echo 'Previous package unexpectedly passed; do not claim a packaging reproduction.'; exit 1
fi
if ! grep -q 'libasound_module_pcm_pulse.so' "$HOME/old.log"; then
  cat "$HOME/old.log"; echo 'Previous package failed for a different reason.'; exit 1
fi
echo 'REPRODUCED: previous bundled ALSA cannot locate the system Pulse PCM plugin.'
timeout 6s parec --device=secpar_review.monitor --raw --format=float32le --rate=48000 --channels=2 --latency-msec=10 >"$HOME/output.raw" &
capture=$!
(sleep 0.8; pactl --format=json list sink-inputs >"$HOME/sink-inputs.json") &
set +e
LD_LIBRARY_PATH="$HOME/candidate/squashfs-root/usr/lib" /artifacts/desktop-test \
  --exact "$case_name" --ignored --nocapture
result=$?
set -e
wait "$capture" || test "$?" -eq 124
python3 - <<'PY'
import os,math,struct,json
from pathlib import Path
root=Path(os.environ['HOME'])
libraries=root/'candidate/squashfs-root/usr/lib'
assert not any(p.name.startswith(('libasound.so','libpulse','libpipewire-','libspa-')) for p in libraries.iterdir())
data=(root/'output.raw').read_bytes()
assert len(data)>=48000*2*4, 'Not enough captured virtual-output frames'
values=[v[0] for v in struct.iter_unpack('<f',data)]
print('Generated monitor frames',len(values)//2,'peak',max(abs(v) for v in values),'nonzero',sum(abs(v)>0.01 for v in values))
for item in json.loads((root/'sink-inputs.json').read_text()):
    print({k:item.get(k) for k in ['sample_specification','sink','mute','volume','buffer_latency_usec','sink_latency_usec']})
assert all(math.isfinite(v) for v in values)
assert sum(abs(v)>0.01 for v in values)>48000, 'Opus did not reach the virtual system output'
assert max(abs(v) for v in values)<0.5, 'Unexpected output clipping'
print('PASS: real Opus decoding/PCM conversion/ALSA/Pulse output reached the isolated monitor.')
PY
test "$result" -eq 0
CONTAINER
