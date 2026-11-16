#!/bin/bash
# CONTAINER ONLY. The hosting side with the real engine and stream component:
# access list -> tunnel -> pairing over the tunnel -> removal. See
# crates/secpar-hosting/examples/hosting_check.rs.
# Environment: HOSTING_CHECK_STREAM=1 also streams the KDE session for a few
# seconds through the tunnel; HOSTING_CHECK_REMOVE=1 removes the device from the
# list halfway through that stream.
# Mounts expected: /work/src/sunshine (built, relocatable), /work/qt-source,
# /work/repo/crates, and /work writable for the cargo build.
pacman -Syu --noconfirm --needed miniupnpc libevdev opus numactl avahi libcap libmfx libva libdrm \
  qt6-base qt6-declarative qt6-svg qt6-wayland sdl2-compat sdl2_ttf ffmpeg libplacebo libvdpau \
  >/tmp/pacman.log 2>&1 || { tail -5 /tmp/pacman.log; exit 1; }
cp /usr/bin/kwin_wayland /opt/nocap/kwin_wayland
cd /work/repo/crates/secpar-hosting
CARGO_TARGET_DIR=/work/target cargo build --example hosting_check >/tmp/cargo.log 2>&1 || { grep -E '^error|-->' -A6 /tmp/cargo.log | head -40; exit 1; }
[ "$HOSTING_CHECK_STREAM" = 1 ] || export HK_TONE=0
source /opt/hk/session.sh || exit 1
pactl load-module module-null-sink sink_name=host_audio >/dev/null
pactl load-module module-null-sink sink_name=client_audio >/dev/null
# Both ends share one sound server here; the stream component plays into its own sink.
export SECPAR_SUPERVISOR_REVIEW=1 PULSE_SINK=client_audio SDL_AUDIODRIVER=pulseaudio SDL_AUDIO_DRIVER=pulseaudio
s=/tmp/hosting-state
rm -rf $s
timeout 300 /work/target/debug/examples/hosting_check /work/src/sunshine/build/sunshine $s /work/qt-source/app/secpar-stream 2>&1 | cut -c1-200 | tail -30
echo "hosting check exit=${PIPESTATUS[0]}"
grep -a -E "Fatal" $s/host/host.log 2>/dev/null | sed -E 's/^\[[^]]*\]: //' | awk '!s[$0]++' | head -5
[ "$HOSTING_CHECK_STREAM" = 1 ] || exit 0
grep -a -E "Found .*encoder|Screencasting|Creating encoder|CLIENT CONNECTED|CLIENT DISCONNECTED" $s/host/host.log | sed -E 's/^\[[^]]*\]: //' | grep -v -E "CUDA|vulkan|Vulkan|av1" | awk '!s[$0]++' | tail -8
grep -a -i -E "Using .* (decoder|renderer)|FFmpeg-based|VAAPI|Connection terminated|error" $s/stream.log | cut -c1-160 | awk '!s[$0]++' | head -10
python3 - <<'PY'
import re
from pathlib import Path
text = Path('/tmp/hosting-state/stream.log').read_text(errors='replace')
for label in ['Incoming frame rate from network', 'Decoding frame rate', 'Rendering frame rate']:
    v = [float(x) for x in re.findall(re.escape(label) + r': ([0-9.]+) FPS', text)]
    print(f'{label}: {v[-1]:.1f} FPS' if v else f'{label}: no figure in the log')
for label in ['Frames dropped by your network connection', 'Average network latency', 'Average decoding time']:
    m = re.findall(re.escape(label) + r': ([^\r\n]+)', text)
    if m: print(f'{label}: {m[-1]}')
PY
