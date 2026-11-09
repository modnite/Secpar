#!/bin/bash
# Runs one measurement inside the headless KDE session container (see Dockerfile and README.md).
#   scenario.sh <bin dir> <label> "<perfclient args>" [ENV=value ...]
# Starts the real host (hostrun) with the given environment, runs perfclient against it, samples the CPU use of
# the host and of kwin each second (lines "C"), and stops the host. Output goes to /out/<label>.txt as well.
BIN=$1; LABEL=$2; CLIENT_ARGS=$3; shift 3
. /tmp/hk-env
export RUST_BACKTRACE=full
OUT=/out/$LABEL.txt
: >"$OUT"
pkill -x hostrun 2>/dev/null; sleep 1
rm -f /tmp/link
env "$@" "$BIN/hostrun" /tmp/link >/out/$LABEL.host.log 2>&1 &
HOST=$!
for i in $(seq 50); do [ -f /tmp/link ] && break; sleep 0.2; done
KWIN=$(pgrep -x kwin_wayland | head -1)
ticks() { awk '{print $14+$15}' /proc/$1/stat 2>/dev/null; }
(
  while ! grep -q "^I stream" "$OUT" 2>/dev/null; do sleep 0.2; done
  h0=$(ticks $HOST); k0=$(ticks $KWIN); n=0
  while kill -0 $HOST 2>/dev/null && ! grep -q "^E " "$OUT"; do
    sleep 1; n=$((n+1)); h1=$(ticks $HOST); k1=$(ticks $KWIN)
    echo "C t=$n host=$((h1-h0))% kwin=$((k1-k0))%" >>"$OUT"; h0=$h1; k0=$k1
  done
) &
SAMPLER=$!
timeout ${HK_TIMEOUT:-90} "$BIN/perfclient" /tmp/link $CLIENT_ARGS >>"$OUT" 2>&1
RC=$?
sleep 1
if kill -0 $HOST 2>/dev/null; then echo "host: still running after client exit $RC" >>"$OUT"; else wait $HOST; echo "HOST DIED exit=$?" >>"$OUT"; fi
kill $SAMPLER 2>/dev/null
kill $HOST 2>/dev/null; sleep 1; kill -9 $HOST 2>/dev/null
grep -aE "panicked|SIG|Aborted|free\(\)|corrupted|backtrace|stack backtrace" -A12 /out/$LABEL.host.log | head -60 >>"$OUT"
cat "$OUT"
