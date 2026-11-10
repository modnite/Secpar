#!/bin/sh
# Local review build only. Synthetic frames, private profile, no remote host/account.
set -eu
binary=${1:?Usage: frame-pacing.sh /absolute/secpar-desktop-review output.log}
output=${2:?Supply an output log path}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
mkdir -p "$tmp/config/secpar" "$tmp/run"
chmod 700 "$tmp/run"
printf '%s' '{"decoder":"Software"}' >"$tmp/config/secpar/client.json"
ffmpeg -nostdin -hide_banner -loglevel error -f lavfi -i testsrc2=size=1280x720:rate=60 -t 2 -c:v libx264 -preset ultrafast -tune zerolatency -g 60 -f h264 "$tmp/source.h264"
ffprobe -v error -show_packets -show_entries packet=pos,size,flags -of json "$tmp/source.h264" >"$tmp/packets.json"
python3 - "$tmp" <<'PY'
from pathlib import Path
import json,struct,sys
root=Path(sys.argv[1]);data=(root/'source.h264').read_bytes()
with (root/'frames').open('wb') as out:
 for pkt in json.loads((root/'packets.json').read_text())['packets']:
  at,n=int(pkt['pos']),int(pkt['size'])
  out.write(struct.pack('>IB',n,int('K' in pkt['flags']))+data[at:at+n])
PY
# timeout is intentional: the stream review loops until its window closes.
set +e
HOME="$tmp" XDG_CONFIG_HOME="$tmp/config" XDG_RUNTIME_DIR="$tmp/run" SECPAR_HOST_PORT=53989 \
SECPAR_FRAME_TIMING=1 SECPAR_REVIEW_PAGE=stream SECPAR_REVIEW_STREAM="$tmp/frames" \
xvfb-run -a -s '-screen 0 1366x768x24' timeout 16 "$binary" >"$output" 2>&1
rc=$?
set -e
[ "$rc" -eq 124 ] || { cat "$output"; echo "Review exited before measurement finished ($rc)" >&2; exit 1; }
# A successful startup alone is not a pacing measurement.
for stage in client.submit.interval client.decode.duration client.draw.interval; do
 grep -q "stage=$stage " "$output" || { cat "$output"; echo "Missing $stage" >&2; exit 1; }
done
grep '^frame_timing ' "$output"
