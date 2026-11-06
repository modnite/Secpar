#!/bin/sh
# Runs only an isolated compositor and the public package; no host display/input access.
set -eu
package=$(realpath "${1:?Usage: test-wayland-startup.sh Secpar-Linux-x64.AppImage}")
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
test_image=${SECPAR_WAYLAND_TEST_IMAGE:-secpar-wayland-review:local}
if ! docker image inspect "$test_image" >/dev/null 2>&1; then
    docker build -t "$test_image" -f "$root/desktop/packaging/wayland-smoke.Dockerfile" "$root/desktop/packaging"
fi
docker run --rm -i --mount "type=bind,src=$package,dst=/artifacts/Secpar.AppImage,readonly" "$test_image" bash -s <<'CONTAINER'
set -eu
export XDG_RUNTIME_DIR=$(mktemp -d /tmp/wayland.XXXXXX)
weston --backend=headless --renderer=pixman --socket=wayland-secpar --idle-time=0 >/tmp/compositor.log 2>&1 &
compositor=$!
trap 'kill "$compositor" 2>/dev/null || true' EXIT
for n in $(seq 1 50); do
    [ -S "$XDG_RUNTIME_DIR/wayland-secpar" ] && break
    sleep 0.1
done
kill -0 "$compositor" || { cat /tmp/compositor.log; exit 1; }
export WAYLAND_DISPLAY=wayland-secpar APPIMAGE_EXTRACT_AND_RUN=1 LIBGL_ALWAYS_SOFTWARE=1 WAYLAND_DEBUG=client
unset DISPLAY
set +e
timeout 8 /artifacts/Secpar.AppImage >/tmp/application.log 2>&1
result=$?
set -e
if [ "$result" -ne 124 ] || ! grep -q 'set_title("Secpar")' /tmp/application.log || ! grep -Eq 'wl_surface.*attach\(' /tmp/application.log; then
    tail -50 /tmp/application.log
    exit 1
fi
echo 'PASS: public AppImage created and rendered its window through native Wayland/EGL.'
CONTAINER
