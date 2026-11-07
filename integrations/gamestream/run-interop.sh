#!/bin/sh
# A test fixture only. Never mount an installed app, user home, display or devices.
set -eu
sunshine=$(realpath "${1:?Pinned Sunshine AppImage required}")
worker=$(realpath "${2:?Built private Moonlight worker required}")
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
python3 - "$sunshine" <<'PY'
import hashlib,sys
from pathlib import Path
expected='6f80297a853bcf8cda114ff131b5c0a8c9a4d96a2a9c3dee1f04c39c54206846'
assert hashlib.sha256(Path(sys.argv[1]).read_bytes()).hexdigest()==expected, 'Sunshine test asset differs from the reviewed release'
PY
docker build -f "$here/interop.Dockerfile" -t secpar-gamestream-interop:local "$here"
docker run --rm --network none --cap-drop ALL --security-opt no-new-privileges \
    --read-only --tmpfs /tmp:rw,exec,nosuid,nodev --user 1000:1000 \
    --mount "type=bind,src=$sunshine,dst=/artifacts/Sunshine.AppImage,readonly" \
    --mount "type=bind,src=$worker,dst=/artifacts/secpar-stream,readonly" \
    --mount "type=bind,src=$here/interop-smoke.sh,dst=/artifacts/review.sh,readonly" \
    secpar-gamestream-interop:local sh /artifacts/review.sh
