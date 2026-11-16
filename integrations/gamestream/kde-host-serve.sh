#!/bin/bash
# CONTAINER ONLY. Runs the Secpar-built hosting engine inside the headless KDE
# session of the secpar-hk image for a limited time, so a connecting part on
# another computer can pair and stream over the real network.
# Publish these container ports when starting it: TCP 47984, 47989, 48010 and
# UDP 47998-48000. Environment: SERVE_PIN (four digits, required),
# SERVE_SECS (default 180). Mount expected: /work/src/sunshine (built, relocatable).
[[ "${SERVE_PIN:-}" =~ ^[0-9]{4}$ ]] || { echo "SERVE_PIN must be four digits"; exit 1; }
pacman -Syu --noconfirm --needed miniupnpc libevdev opus numactl avahi libcap libmfx libva libdrm \
  >/tmp/pacman.log 2>&1 || { tail -5 /tmp/pacman.log; exit 1; }
cp /usr/bin/kwin_wayland /opt/nocap/kwin_wayland
host=/work/src/sunshine/build/sunshine
missing=$(ldd "$host" | grep -c "not found"); [ "$missing" = 0 ] || { echo "$missing libraries missing"; exit 1; }
mkdir -p /tmp/h/usr/local/assets /tmp/h/state
cp -r /work/src/sunshine/build/assets/. /tmp/h/usr/local/assets/
cp -r /work/src/sunshine/src_assets/linux/assets/. /tmp/h/usr/local/assets/ 2>/dev/null
source /opt/hk/session.sh || exit 1
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
printf '%s\n%s\n' "$SERVE_PIN" "$SERVE_PIN" | "$host" $s/sunshine.conf -0 >/tmp/host-console.log 2>&1 &
host_pid=$!
for i in $(seq 300); do curl -s -f http://127.0.0.1:47989/serverinfo >/dev/null 2>&1 && break; sleep 0.2; done
echo "READY"
sleep "${SERVE_SECS:-180}"
grep -a -E "Found .*encoder|Screencasting|Creating encoder|Color coding|CLIENT CONNECTED|CLIENT DISCONNECTED|New streaming session|Error|Fatal|Warning" $s/host.log \
  | sed -E 's/^\[[^]]*\]: //' | grep -v -E "CUDA|CAP_SYS|[Vv]ulkan|av1|nvenc|Gamepad|gamepad|Daemon not running" | awk '!s[$0]++' | tail -20
kill -INT $host_pid 2>/dev/null
