#!/bin/bash
# CONTAINER ONLY. The hosting side with the real engine and stream component:
# access list -> tunnel -> pairing over the tunnel -> removal. See
# crates/secpar-hosting/examples/hosting_check.rs. No stream is started.
# Mounts expected: /work/src/sunshine (built, relocatable), /work/qt-source,
# /work/repo/crates, and /work writable for the cargo build.
pacman -Syu --noconfirm --needed miniupnpc libevdev opus numactl avahi libcap libmfx libva libdrm \
  qt6-base qt6-declarative qt6-svg qt6-wayland sdl2-compat sdl2_ttf ffmpeg libplacebo libvdpau \
  >/tmp/pacman.log 2>&1 || { tail -5 /tmp/pacman.log; exit 1; }
cp /usr/bin/kwin_wayland /opt/nocap/kwin_wayland
cd /work/repo/crates/secpar-hosting
CARGO_TARGET_DIR=/work/target cargo build --example hosting_check >/tmp/cargo.log 2>&1 || { grep -E '^error|-->' -A6 /tmp/cargo.log | head -40; exit 1; }
export HK_TONE=0
source /opt/hk/session.sh || exit 1
pactl load-module module-null-sink sink_name=host_audio >/dev/null
export SECPAR_SUPERVISOR_REVIEW=1
rm -rf /tmp/hosting-state
timeout 300 /work/target/debug/examples/hosting_check /work/src/sunshine/build/sunshine /tmp/hosting-state /work/qt-source/app/secpar-stream 2>&1 | cut -c1-200 | tail -30
echo "hosting check exit=${PIPESTATUS[0]}"
grep -a -E "Fatal" /tmp/hosting-state/host/host.log 2>/dev/null | sed -E 's/^\[[^]]*\]: //' | awk '!s[$0]++' | head -5
