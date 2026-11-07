#!/bin/sh
# Container-only owned-process review. Does not install or control a service.
set -eu
export HOME=/tmp/review XDG_RUNTIME_DIR=/tmp/runtime DISPLAY=:94
export LIBGL_ALWAYS_SOFTWARE=1 QT_QUICK_BACKEND=software SECPAR_SUPERVISOR_REVIEW=1
mkdir -p "$HOME" "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
cd "$HOME"
/artifacts/Sunshine.AppImage --appimage-extract >/dev/null
Xvfb :94 -screen 0 640x360x24 -nolisten tcp >/tmp/display.log 2>&1 &
display_pid=$!
trap 'kill "$display_pid" 2>/dev/null || true' EXIT
pulseaudio --start --exit-idle-time=-1 --log-target=file:/tmp/pulse.log
pactl load-module module-null-sink sink_name=host_audio >/dev/null
sleep 1
/artifacts/supervisor-review "$HOME/squashfs-root/AppRun" "$HOME/state"
