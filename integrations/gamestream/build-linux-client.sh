#!/bin/sh
# Build a private component of the one Secpar application, not a second product.
set -eu
source_dir=$(realpath "${1:?Prepared source directory required}")
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
test -f "$source_dir/SECPAR-BUILD-ORIGIN.json"
python3 - "$source_dir" "$here/upstream-lock.json" <<'PY'
import json,sys
from pathlib import Path
origin=json.loads((Path(sys.argv[1])/'SECPAR-BUILD-ORIGIN.json').read_text())
pin=json.loads(Path(sys.argv[2]).read_text())['components']['moonlight-qt']['commit']
assert origin['component']=='moonlight-qt' and origin['upstream']['commit']==pin
assert origin['patches'], 'Prepare the Secpar identity/fullscreen patch first'
PY
docker build -f "$here/linux-client.Dockerfile" -t secpar-moonlight-build:local "$here"
docker run --rm --network none --cap-drop ALL --security-opt no-new-privileges \
    --mount "type=bind,src=$source_dir,dst=/source" secpar-moonlight-build:local sh -c \
    'qmake6 moonlight-qt.pro CONFIG+=release && make qmake_all && make -j4 release && strip --strip-unneeded app/secpar-stream'
printf 'Private streaming component: %s/app/secpar-stream\n' "$source_dir"
