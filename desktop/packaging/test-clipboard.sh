#!/bin/sh
# A private compositor and generated text only; no personal display or clipboard.
set -eu
binary=$(realpath "${1:?Native clipboard test binary required}")
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
docker build -t secpar-clipboard-review:local -f "$root/desktop/packaging/clipboard-smoke.Dockerfile" "$root/desktop/packaging"
docker run --rm -i --network none --cap-drop ALL --security-opt no-new-privileges \
 --mount "type=bind,src=$binary,dst=/artifacts/clipboard-test,readonly" secpar-clipboard-review:local bash -s <<'CONTAINER'
set -eu
review_dir=$(mktemp -d /tmp/secpar-clipboard.XXXXXX)
export XDG_RUNTIME_DIR="$review_dir/runtime" XDG_CONFIG_HOME="$review_dir/config" XDG_CACHE_HOME="$review_dir/cache"
mkdir -m700 "$XDG_RUNTIME_DIR"
printf 'output * bg #202a30 solid_color\n' >"$review_dir/sway.conf"
export WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1
sway --unsupported-gpu -c "$review_dir/sway.conf" >"$review_dir/sway.log" 2>&1 &
for n in $(seq 1 60); do
 for socket in "$XDG_RUNTIME_DIR"/wayland-*; do
  if [ -S "$socket" ]; then export WAYLAND_DISPLAY=$(basename "$socket"); break 2; fi
 done
 sleep 0.1
done
if [ -z "${WAYLAND_DISPLAY:-}" ]; then cat "$review_dir/sway.log"; exit 1; fi
export SECPAR_CLIPBOARD_REVIEW=1
/artifacts/clipboard-test --exact actual_wayland_copy_paste_and_remote_echo_suppression --ignored --nocapture
CONTAINER
