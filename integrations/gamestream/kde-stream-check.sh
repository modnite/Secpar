#!/bin/bash
# CONTAINER ONLY. Pairs the Secpar-built engine with the Secpar-built stream
# component inside the headless KDE session of the secpar-hk image and streams
# for a few seconds: KWin capture, hardware encoding where the container has a
# render device, and sound from the session's test tone.
# Mounts expected: /work/src/sunshine (built, relocatable) and /work/qt-source.
# Environment: CHECK_CODEC (H.264 | HEVC), CHECK_DECODER (auto | software),
# CHECK_RES (default 1280x720), CHECK_FPS (default 60), CHECK_SECS (default 16).
pacman -Syu --noconfirm --needed miniupnpc libevdev opus numactl avahi libcap libmfx libva libdrm \
  qt6-base qt6-declarative qt6-svg qt6-wayland sdl2-compat sdl2_ttf ffmpeg libplacebo libvdpau \
  >/tmp/pacman.log 2>&1 || { tail -5 /tmp/pacman.log; exit 1; }
cp /usr/bin/kwin_wayland /opt/nocap/kwin_wayland
host=/work/src/sunshine/build/sunshine
client=/work/qt-source/app/secpar-stream
for b in "$host" "$client"; do
  missing=$(ldd "$b" | grep -c "not found"); [ "$missing" = 0 ] || { echo "$b: $missing libraries missing"; exit 1; }
done
mkdir -p /tmp/h/usr/local/assets /tmp/h/state
cp -r /work/src/sunshine/build/assets/. /tmp/h/usr/local/assets/
cp -r /work/src/sunshine/src_assets/linux/assets/. /tmp/h/usr/local/assets/ 2>/dev/null
source /opt/hk/session.sh || exit 1
pactl load-module module-null-sink sink_name=client_audio >/dev/null
s=/tmp/h/state
cat > $s/sunshine.conf <<CONFIG
sunshine_name = Secpar check
capture = kwin
origin_web_ui_allowed = pc
file_state = $s/clients.json
credentials_file = $s/management.json
file_apps = $s/apps.json
cert = $s/cert.pem
pkey = $s/key.pem
log_path = $s/host.log
min_log_level = info
CONFIG
echo '{"env":{},"apps":[{"name":"Desktop","image-path":""}]}' > $s/apps.json
cd /tmp/h
# -0 hands the PIN to the engine on its input. Fixed PIN for this container only.
printf '1234\n1234\n' | "$host" $s/sunshine.conf -0 >/tmp/host-console.log 2>&1 &
host_pid=$!
for i in $(seq 300); do curl -s -f http://127.0.0.1:47989/serverinfo >/tmp/serverinfo.xml 2>/dev/null && break; sleep 0.2; done
grep -q '<root' /tmp/serverinfo.xml || { echo "engine did not answer"; tail -20 /tmp/host-console.log; exit 1; }
grep -a -E "Found .*encoder|Screencasting" $s/host.log | sed -E 's/^\[[^]]*\]: //' | sort -u
# The stream component must play into its own sink, not the one being captured.
export SECPAR_MANAGED_PAIRING=1 PULSE_SINK=client_audio SDL_AUDIODRIVER=pulseaudio SDL_AUDIO_DRIVER=pulseaudio
if ! timeout 30 "$client" pair 127.0.0.1:47989 --pin 1234 >/tmp/pair.log 2>&1; then
  echo "pairing failed"; tail -15 /tmp/pair.log | sed 's/?.*/?[query omitted]/'; tail -15 /tmp/host-console.log; exit 1
fi
echo "paired"
# The engine moves the default output to its own capture sink while streaming.
# The image's tone is pinned to one sink, so play a second tone that follows the default.
env -u PULSE_SINK gst-launch-1.0 -q audiotestsrc wave=sine freq=500 volume=0.3 is-live=true ! audioconvert ! pulsesink >/dev/null 2>&1 &
tone=$!
parec --device=client_audio.monitor --format=s16le --rate=48000 --channels=2 /tmp/client-audio.pcm >/dev/null 2>&1 &
rec=$!
# Halfway through, show where sound is going.
# Both ends share one sound server here, so the stream component's output has to
# be moved off the sink the engine is capturing. On two computers this cannot happen.
( sleep 7; c=$(pactl list short clients | awk '$3=="secpar-stream"{print $1}' | head -1)
  i=$(pactl list short sink-inputs | awk -v c="$c" '$3==c{print $1}' | head -1)
  [ -n "$i" ] && pactl move-sink-input "$i" client_audio ) &
( sleep 9; echo "-- audio routing during the stream"; echo "default sink: $(pactl get-default-sink)"
  pactl list short sinks | awk '{print "sink", $1, $2, $NF}'
  pactl list short sink-inputs | awk '{print "playing into sink", $2, "client", $3}'
  pactl list short source-outputs | awk '{print "recording from source", $2, "client", $3}'
  pactl list short clients | awk '{print "client", $1, $3}' ) &
timeout -s INT ${CHECK_SECS:-16} "$client" stream 127.0.0.1:47989 Desktop \
  --resolution ${CHECK_RES:-1280x720} --fps ${CHECK_FPS:-60} --bitrate 10000 --video-codec ${CHECK_CODEC:-H.264} \
  --video-decoder ${CHECK_DECODER:-auto} --display-mode windowed --absolute-mouse \
  --keep-awake --frame-pacing --no-mute-on-focus-loss >/tmp/stream.log 2>&1
echo "stream exit=$?"
kill $rec $tone 2>/dev/null; wait $rec 2>/dev/null
grep -a -E "Creating encoder|Color coding|Client connected|CLIENT CONNECTED|New streaming session|Error|Fatal" $s/host.log | sed -E 's/^\[[^]]*\]: //' | grep -v -E "CUDA|CAP_SYS|vulkan|Vulkan|av1" | awk '!s[$0]++' | tail -8
grep -a -i -E "decoder|renderer|Video stream is|FFmpeg-based|Using .* (decoder|renderer)|hwaccel|VAAPI|Dropping|error" /tmp/stream.log | cut -c1-160 | awk '!s[$0]++' | head -14
grep -a -i -E "audio|sink|pulse|opus" $s/host.log | sed -E 's/^\[[^]]*\]: //' | awk '!s[$0]++' | head -8
grep -a -i -E "audio|opus" /tmp/stream.log | cut -c1-150 | awk '!s[$0]++' | head -6
pactl list short sink-inputs | head -5
python3 - <<'PY'
import array, math, re
from pathlib import Path
text = Path('/tmp/stream.log').read_text(errors='replace')
for label in ['Incoming frame rate from network', 'Decoding frame rate', 'Rendering frame rate']:
    v = [float(x) for x in re.findall(re.escape(label) + r': ([0-9.]+) FPS', text)]
    print(f'{label}: {v[-1]:.1f} FPS' if v else f'{label}: no figure in the log')
for label in ['Frames dropped by your network connection', 'Average network latency', 'Average decoding time']:
    m = re.findall(re.escape(label) + r': ([^\r\n]+)', text)
    if m: print(f'{label}: {m[-1]}')
p = Path('/tmp/client-audio.pcm')
samples = array.array('h', p.read_bytes()[: (p.stat().st_size // 4) * 4])[::2] if p.exists() else []
start = next((i for i, v in enumerate(samples) if abs(v) > 200), None)
if start is None:
    print('sound: none reached the client output')
else:
    chunk = samples[start + 2000:start + 50000]
    rms = math.sqrt(sum(v * v for v in chunk) / max(len(chunk), 1))
    crossings = sum((a < 0) != (b < 0) for a, b in zip(chunk, chunk[1:]))
    print(f'sound: reached the client output, level {rms:.0f}, about {crossings / 2:.0f} Hz')
PY
kill -INT $host_pid 2>/dev/null
