#!/bin/sh
# CONTAINER ONLY: private X servers, generated fixture, no host devices or network.
set -eu
export HOME=/tmp/review XDG_RUNTIME_DIR=/tmp/runtime
mkdir -p "$HOME" "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
cd "$HOME"
/artifacts/Sunshine.AppImage --appimage-extract >/dev/null
host="$HOME/squashfs-root/AppRun"
Xvfb :94 -screen 0 640x360x24 -nolisten tcp >/tmp/host-display.log 2>&1 &
host_display=$!
Xvfb :95 -screen 0 800x600x24 -nolisten tcp >/tmp/client-display.log 2>&1 &
client_display=$!
trap 'kill "$host_display" "$client_display" ${host_pid:-} ${picture_pid:-} ${audio_pid:-} ${monitor_pid:-} 2>/dev/null || true' EXIT
export LIBGL_ALWAYS_SOFTWARE=1 QT_QUICK_BACKEND=software
export DISPLAY=:94
sleep 1
xclock -digital -update 1 -geometry 480x180+40+60 >/tmp/picture.log 2>&1 &
picture_pid=$!
pulseaudio --start --exit-idle-time=-1 --log-target=file:/tmp/pulse.log
pactl load-module module-null-sink sink_name=host_audio >/dev/null
pactl load-module module-null-sink sink_name=client_audio >/dev/null
pactl set-default-sink host_audio
python3 - <<'PY'
import math,struct,wave
with wave.open('/tmp/generated-tone.wav','wb') as output:
    output.setparams((2,2,48000,48000,'NONE','not compressed'))
    output.writeframes(b''.join(struct.pack('<hh',int(5000*math.sin(2*math.pi*500*i/48000)),int(5000*math.sin(2*math.pi*500*i/48000))) for i in range(48000)))
PY
(while true; do paplay --device=host_audio /tmp/generated-tone.wav; done) >/tmp/tone.log 2>&1 &
audio_pid=$!
cat > /tmp/apps.json <<'JSON'
{"env":{},"apps":[{"name":"Desktop","image-path":""}]}
JSON
cat > /tmp/sunshine.conf <<'CONFIG'
sunshine_name = Secpar generated fixture
port = 48989
capture = x11
encoder = software
sw_preset = ultrafast
sw_tune = zerolatency
file_state = /tmp/sunshine-state.json
credentials_file = /tmp/sunshine-credentials.json
file_apps = /tmp/apps.json
cert = /tmp/fixture-cert.pem
pkey = /tmp/fixture-key.pem
log_path = /tmp/host.log
origin_web_ui_allowed = pc
audio_sink = host_audio
CONFIG
# -0 delegates PIN handling to upstream's stdin interface. Only the isolated
# fixture uses this fixed PIN; production ownership authorization is separate.
printf '1234\n1234\n' | "$host" /tmp/sunshine.conf -0 >/tmp/host-console.log 2>&1 &
host_pid=$!
for attempt in $(seq 1 100); do
    if curl --silent --fail http://127.0.0.1:48989/serverinfo >/tmp/serverinfo.xml; then break; fi
    sleep 0.1
done
if ! grep -q '<root' /tmp/serverinfo.xml; then
    tail -n 25 /tmp/host-console.log
    exit 1
fi
export DISPLAY=:95 PULSE_SINK=client_audio
export SECPAR_MANAGED_PAIRING=1
set +e
XDG_CONFIG_HOME=/tmp/rejected-config XDG_CACHE_HOME=/tmp/rejected-cache timeout 20s /artifacts/secpar-stream pair 127.0.0.1:48989 --pin 0000 >/tmp/rejected-pair.log 2>&1
rejected=$?
set -e
if [ "$rejected" != 1 ]; then
    printf 'Wrong PIN must fail promptly; exit status %s\n' "$rejected"
    exit 1
fi
printf 'Upstream rejected an incorrect pairing PIN.\n'
if ! timeout 20s /artifacts/secpar-stream pair 127.0.0.1:48989 --pin 1234 >/tmp/pair.log 2>&1; then
    tail -n 20 /tmp/pair.log | sed 's/?.*/?[fixture-query-omitted]/'
    tail -n 20 /tmp/host-console.log
    exit 1
fi
printf 'Upstream authenticated pairing completed.\n'
parec --device=client_audio.monitor --format=s16le --rate=48000 --channels=2 /tmp/client-audio.pcm >/tmp/monitor.log 2>&1 &
monitor_pid=$!
# Stream only the private generated X display. The host never sees a user desktop.
set +e
timeout -s INT 12s /artifacts/secpar-stream stream 127.0.0.1:48989 Desktop \
    --resolution 640x360 --fps 30 --bitrate 3000 --video-codec H.264 \
    --video-decoder software --display-mode windowed --absolute-mouse \
    --capture-system-keys fullscreen --keep-awake --frame-pacing \
    --no-mute-on-focus-loss >/tmp/stream.log 2>&1
status=$?
set -e
if [ "$status" != 0 ] && [ "$status" != 124 ]; then
    tail -n 25 /tmp/stream.log
    exit 1
fi
kill "$monitor_pid"
wait "$monitor_pid" 2>/dev/null || true
python3 - <<'PY'
import array,math,re
from pathlib import Path
text=Path('/tmp/stream.log').read_text(errors='replace')
for label in ['Incoming frame rate from network','Decoding frame rate','Rendering frame rate']:
    values=[float(v) for v in re.findall(re.escape(label)+r': ([0-9.]+) FPS',text)]
    assert values and max(values)>0,(label,text[-3000:])
    print(f'{label}: {values[-1]:.2f} FPS (generated software fixture only)')
samples=array.array('h',Path('/tmp/client-audio.pcm').read_bytes())[::2]
start=next((i for i,v in enumerate(samples) if abs(v)>200),None)
assert start is not None,'No generated audio reached the client output monitor'
chunk=samples[start+2000:start+50000]
assert len(chunk)==48000,'Insufficient decoded client audio'
rms=math.sqrt(sum(v*v for v in chunk)/len(chunk))
crossings=sum((a<0)!=(b<0) for a,b in zip(chunk,chunk[1:]))
assert rms>1000 and 900<=crossings<=1100,(rms,crossings)
print('Generated500Hz audio reached the separate client sink through the upstream stream.')
PY
printf 'Private upstream host/client streaming smoke completed.\n'
