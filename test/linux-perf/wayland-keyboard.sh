#!/bin/bash
# Run only inside the private secpar-hk session. Never point this at a user's desktop.
set -euo pipefail
binary=${1:?Supply the review executable}
export HOME=/tmp/review-home XDG_CONFIG_HOME=/tmp/review-config SECPAR_HOST_PORT=53989
mkdir -p "$HOME" "$XDG_CONFIG_HOME"
export SECPAR_REVIEW_START_FULLSCREEN=1
export SECPAR_REVIEW_PAGE=stream SECPAR_REVIEW_HOOK=1 SECPAR_REVIEW_KEYS=1
"$binary" >/tmp/review-keys.log 2>&1 &
app=$!
trap 'kill "$app" 2>/dev/null || true; tail -n 22 /tmp/hk-logs/kwin.log' EXIT
sleep 5
hk-click 683 384
sleep 1
hk-click chord 125
hk-click chord 125 18
hk-click chord 56 15
hk-click chord 29 56 17
sleep 1
hk-click chord 29 56 17
sleep 1
# The local stream menu must immediately return shortcuts to this desktop.
meta_before=$(grep -c 'review_key vk=91 down=true' /tmp/review-keys.log)
hk-click 24 24
sleep 1
grep -q 'review_shortcuts active=false' /tmp/review-keys.log
hk-click chord 125
sleep 1
test "$(grep -c 'review_key vk=91 down=true' /tmp/review-keys.log)" -eq "$meta_before"
cat /tmp/review-keys.log
grep -q 'review_shortcuts active=true' /tmp/review-keys.log
grep -q 'review_key vk=91 down=true' /tmp/review-keys.log
grep -q 'review_key vk=91 down=false' /tmp/review-keys.log
grep -q 'review_key vk=69 down=true' /tmp/review-keys.log
grep -q 'review_key vk=9 down=true' /tmp/review-keys.log
test "$(grep -c 'review_escape Fullscreen' /tmp/review-keys.log)" -eq 2
! grep -q 'review_key vk=87 down=true' /tmp/review-keys.log
echo 'PASS Wayland shortcut inhibition, Meta, Meta+E, Alt+Tab, two fullscreen escape chords and local-menu release'
