#!/bin/bash
# Starts the headless Plasma session pieces inside the container and leaves them running.
# Sourced by entrypoint.sh. Environment: HK_WIDTH, HK_HEIGHT, HK_GPU (1 = render node, 0 = llvmpipe).
set -u
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp/hk-run}
mkdir -p "$XDG_RUNTIME_DIR" && chmod 700 "$XDG_RUNTIME_DIR"
export HOME=${HOME:-/home/hk}
export XDG_CURRENT_DESKTOP=KDE XDG_SESSION_TYPE=wayland KDE_FULL_SESSION=true
export QT_QPA_PLATFORM=wayland WAYLAND_DISPLAY=wayland-0
export KWIN_WAYLAND_NO_PERMISSION_CHECKS=1   # lets hk-click and shot.py use KWin fake input and ScreenShot2
export XDG_DATA_DIRS=/usr/local/share:/usr/share XDG_CONFIG_DIRS=/etc/xdg
LOGS=${HK_LOGS:-/tmp/hk-logs}; mkdir -p "$LOGS"
W=${HK_WIDTH:-1920}; H=${HK_HEIGHT:-1080}

if [ "${HK_GPU:-1}" = 1 ]; then
  export LIBVA_DRIVER_NAME=iHD LIBVA_DRIVERS_PATH=/usr/lib/dri
  export KWIN_DRM_DEVICES=/dev/dri/renderD128
  export GST_VA_ALL_DRIVERS=1
else
  export LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe
fi

# 1. private session bus
export DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"
dbus-daemon --session --fork --address="$DBUS_SESSION_BUS_ADDRESS" --nopidfile
echo "bus $DBUS_SESSION_BUS_ADDRESS" >>"$LOGS/session.log"

# 2. pipewire, wireplumber, pipewire-pulse
pipewire >"$LOGS/pipewire.log" 2>&1 &
for i in $(seq 50); do [ -S "$XDG_RUNTIME_DIR/pipewire-0" ] && break; sleep 0.1; done
wireplumber >"$LOGS/wireplumber.log" 2>&1 &
pipewire-pulse >"$LOGS/pipewire-pulse.log" 2>&1 &
for i in $(seq 50); do pactl info >/dev/null 2>&1 && break; sleep 0.1; done
# an output to monitor: a null sink that is the default
pactl load-module module-null-sink sink_name=secpar_review sink_properties=device.description=secpar_review >/dev/null
pactl set-default-sink secpar_review

# 3. kwin_wayland on a virtual output

KWIN=/opt/nocap/kwin_wayland   # plain copy: the packaged binary has file capabilities that a default container refuses to exec
env -u QT_QPA_PLATFORM -u WAYLAND_DISPLAY "$KWIN" --virtual --width "$W" --height "$H" --scale 1 --socket wayland-0 \
  --no-lockscreen --no-global-shortcuts >"$LOGS/kwin.log" 2>&1 &
for i in $(seq 100); do [ -S "$XDG_RUNTIME_DIR/wayland-0" ] && break; sleep 0.1; done
[ -S "$XDG_RUNTIME_DIR/wayland-0" ] || { echo "kwin did not start"; tail -n 30 "$LOGS/kwin.log"; exit 1; }
for i in $(seq 100); do dbus-send --session --print-reply --dest=org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus.NameHasOwner string:org.kde.KWin 2>/dev/null | grep -q true && break; sleep 0.1; done

# 4. portals
/usr/lib/xdg-desktop-portal-kde >"$LOGS/xdp-kde.log" 2>&1 &
sleep 1
/usr/lib/xdg-desktop-portal -r >"$LOGS/xdp.log" 2>&1 &
for i in $(seq 100); do dbus-send --session --print-reply --dest=org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus.NameHasOwner string:org.freedesktop.portal.Desktop 2>/dev/null | grep -q true && break; sleep 0.1; done
echo "session up ${W}x${H} gpu=${HK_GPU:-1}" >>"$LOGS/session.log"
# 5. unattended consent: press Allow whenever the portal dialog shows
python3 /opt/hk/approve.py >"$LOGS/approve.log" 2>&1 &

# 6. something to capture. HK_CLIENT: egl (weston-simple-egl, fullscreen 60 fps), gears (eglgears_wayland), none
case "${HK_CLIENT:-egl}" in
  egl) weston-simple-egl -f >"$LOGS/client.log" 2>&1 & ;;
  gears) eglgears_wayland >"$LOGS/client.log" 2>&1 & ;;
esac
# a 440 Hz tone into the null sink, so its monitor has audio to capture
if [ "${HK_TONE:-1}" = 1 ]; then
  gst-launch-1.0 -q audiotestsrc wave=sine freq=440 volume=0.3 is-live=true ! audioconvert ! pulsesink device=secpar_review >"$LOGS/tone.log" 2>&1 &
fi
sleep 2

# Environment for later `docker exec` shells: source /tmp/hk-env
{ for v in XDG_RUNTIME_DIR HOME XDG_CURRENT_DESKTOP XDG_SESSION_TYPE KDE_FULL_SESSION QT_QPA_PLATFORM WAYLAND_DISPLAY \
  DBUS_SESSION_BUS_ADDRESS LIBVA_DRIVER_NAME LIBVA_DRIVERS_PATH LIBGL_ALWAYS_SOFTWARE GALLIUM_DRIVER HK_LOGS; do
  [ -n "${!v:-}" ] && echo "export $v=${!v}"; done; echo "export HK_LOGS=$LOGS"; } >/tmp/hk-env
