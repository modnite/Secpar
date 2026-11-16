#!/bin/sh
# Secpar's one-time system setup on Linux, run as root through pkexec from Settings.
# The same steps Sunshine's own packages take when they are installed:
#  - a root-owned copy of the stream engine in /usr/local/lib/secpar/engine, allowed
#    to capture the screen directly (cap_sys_admin) and to run at high priority
#    (cap_sys_nice). The permissions go on a copy only root can change, never on the
#    one in the user's folder;
#  - Sunshine's udev rules, so the signed-in user may create virtual keyboards, mice
#    and controllers;
#  - the uinput and uhid modules loaded at boot.
# Usage: system-setup.sh <engine folder> | --remove
set -eu
DEST=/usr/local/lib/secpar/engine
RULES=/etc/udev/rules.d/60-secpar.rules
MODULES=/etc/modules-load.d/60-secpar.conf

if [ "$(id -u)" != 0 ]; then echo "Run through pkexec" >&2; exit 2; fi

if [ "${1:-}" = --remove ]; then
    rm -rf "$DEST" "$RULES" "$MODULES"
    rmdir /usr/local/lib/secpar 2>/dev/null || true
    udevadm control --reload-rules 2>/dev/null || true
    exit 0
fi

SRC=${1:?engine folder}
[ -f "$SRC/sunshine" ] || { echo "No engine in $SRC" >&2; exit 3; }
command -v setcap >/dev/null || { echo "setcap is missing (install libcap)" >&2; exit 4; }

# A fresh copy, made root's before it gets its permissions.
STAGE=$(mktemp -d /usr/local/lib/secpar-engine.XXXXXX)
trap 'rm -rf "$STAGE"' EXIT
cp -R "$SRC/." "$STAGE/"
rm -f "$STAGE"/*.log
chown -R root:root "$STAGE"
chmod -R u=rwX,go=rX "$STAGE"
setcap cap_sys_admin,cap_sys_nice+p "$STAGE/sunshine"
mkdir -p /usr/local/lib/secpar
rm -rf "$DEST"
mv "$STAGE" "$DEST"
trap - EXIT

cat > "$RULES" <<'EOF'
# Secpar: lets the signed-in user's stream engine create virtual input devices.
# The same rules as Sunshine's 60-sunshine.rules.
KERNEL=="uinput", SUBSYSTEM=="misc", OPTIONS+="static_node=uinput", GROUP="input", MODE="0660", TAG+="uaccess"
KERNEL=="uhid", GROUP="input", MODE="0660", TAG+="uaccess"
SUBSYSTEM=="hidraw", KERNEL=="hidraw*", IMPORT{parent}="HID_*"
SUBSYSTEM=="hidraw", KERNEL=="hidraw*", ENV{HID_PHYS}=="libvirtualhid/uhid/*", GROUP="input", MODE="0660", TAG+="uaccess"
SUBSYSTEM=="input", KERNEL=="event*", ATTRS{phys}=="libvirtualhid/uhid/*", GROUP="input", MODE="0660", TAG+="uaccess"
SUBSYSTEM=="hidraw", KERNEL=="hidraw*", ENV{HID_NAME}=="Sunshine (libvirtualhid)*", GROUP="input", MODE="0660", TAG+="uaccess"
SUBSYSTEMS=="input", ATTRS{name}=="Sunshine (libvirtualhid)*", GROUP="input", MODE="0660", TAG+="uaccess"
EOF
chmod 644 "$RULES"
printf 'uinput\nuhid\n' > "$MODULES"
chmod 644 "$MODULES"
modprobe uinput 2>/dev/null || true
modprobe uhid 2>/dev/null || true
udevadm control --reload-rules 2>/dev/null || true
udevadm trigger --subsystem-match=misc --attr-match=dev 2>/dev/null || true
echo ready
