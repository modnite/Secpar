#!/bin/sh
# Container-only owned-process review of device-bound pairing. No service, user
# home, real display or device is involved.
set -eu
export HOME=/tmp/review XDG_RUNTIME_DIR=/tmp/runtime DISPLAY=:94
export LIBGL_ALWAYS_SOFTWARE=1 QT_QUICK_BACKEND=software SECPAR_SUPERVISOR_REVIEW=1
mkdir -p "$HOME" "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
cd "$HOME"
Xvfb :94 -screen 0 640x360x24 -nolisten tcp >/tmp/display.log 2>&1 &
display_pid=$!
trap 'kill "$display_pid" 2>/dev/null || true' EXIT
pulseaudio --start --exit-idle-time=-1 --log-target=file:/tmp/pulse.log
pactl load-module module-null-sink sink_name=host_audio >/dev/null
sleep 1
/artifacts/pairing-review /artifacts/sunshine "$HOME/state" /artifacts/secpar-stream
