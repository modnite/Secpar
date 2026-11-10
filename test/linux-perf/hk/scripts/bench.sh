#!/bin/bash
# Runs the whole measurement matrix inside a running session container and prints one line per result.
# Usage (host): docker exec CONTAINER /opt/hk/bench.sh [full|gl]    env: HK_BENCH_SECS (default 20)
# full: every row. gl: only the DMA-BUF (glupload) rows, for a compositor started with INTEL_DEBUG=noccs.
. /tmp/hk-env
MODE=${1:-full}
SECS=${HK_BENCH_SECS:-20}
T=/usr/local/bin/secpar-host-tests
export SECPAR_BENCH_SECS=$SECS

cgroup_cpu() { awk '/^usage_usec/{print $2/1e6}' /sys/fs/cgroup/cpu.stat; }
run() { # label, env assignments..., test path; prints the interesting lines
  local label=$1 test=$2; shift 2
  local out; out=$(env "$@" timeout 120 $T --ignored --exact "$test" --nocapture 2>&1); local rc=$?
  echo "$out" | grep -aE '^(BENCH|RESIZE|CYCLE|CYCLES|TEARDOWN|AUDIO|INPUT)' | sed "s/^/[$label] /"
  if [ $rc -ne 0 ]; then
    echo "[$label] exit $rc: $(echo "$out" | grep -av CRITICAL | grep -aE "panicked|capture:|assertion|SIG|Aborted|free\(\)|corrupted" | tr '\n' ' ' | cut -c1-400)"
  fi
  return $rc
}

echo "== environment"
dbus-send --session --print-reply --dest=org.kde.KWin /KWin org.kde.KWin.supportInformation 2>/dev/null \
  | grep -aE "Name: KWin::|Geometry:|Refresh Rate|OpenGL renderer|Compositing Type" | sed 's/^ *//'
echo "client: ${HK_CLIENT:-egl}"
LIBVA_DRIVER_NAME=iHD vainfo 2>/dev/null | grep -aE "Driver version|H264(Main|High|ConstrainedBaseline).*Enc|HEVCMain.*Enc"
gst-inspect-1.0 vah264enc >/dev/null 2>&1 && echo "gst vah264enc: present"
gst-inspect-1.0 vah265enc >/dev/null 2>&1 && echo "gst vah265enc: present"
a=$(cgroup_cpu); sleep 10; b=$(cgroup_cpu)
echo "baseline (compositor + client + pipewire, nothing capturing): container cpu $(echo "$a $b" | awk '{printf "%.0f", ($2-$1)/10*100}')% of one core"

echo "== throughput, ${SECS} s each"
for pair in vah264enc:h264 vah265enc:hevc x264enc:h264; do
  enc=${pair%%:*}; codec=${pair##*:}
  for cap in dmabuf copy; do
    [ $MODE = gl ] && [ $cap = copy ] && continue
    run "$enc $cap" capture::tests::portal_stream_throughput SECPAR_ENCODER=$enc SECPAR_CODEC=$codec SECPAR_CAPTURE=$cap
  done
done

echo "== live resize"
for pair in vah264enc:h264 vah265enc:hevc x264enc:h264; do
  enc=${pair%%:*}; codec=${pair##*:}
  [ $MODE = gl ] && cap=dmabuf || cap=copy
  run "$enc $cap" capture::tests::portal_resize_live SECPAR_ENCODER=$enc SECPAR_CODEC=$codec SECPAR_CAPTURE=$cap
done

echo "== reconnect cycles (parked teardown)"
[ $MODE = gl ] && SECPAR_CAPTURE=dmabuf || SECPAR_CAPTURE=copy
for enc in vah264enc x264enc; do
  run "$enc $SECPAR_CAPTURE, new portal per cycle" capture::tests::portal_reconnect_cycles SECPAR_ENCODER=$enc SECPAR_CAPTURE=$SECPAR_CAPTURE
  run "$enc $SECPAR_CAPTURE, one shared portal" capture::tests::portal_reconnect_cycles SECPAR_ENCODER=$enc SECPAR_CAPTURE=$SECPAR_CAPTURE SECPAR_SHARED_PORTAL=1
done
echo "== unparked teardown (iHD abort probe)"
for cap in copy dmabuf; do
  [ $MODE = gl ] && [ $cap = copy ] && continue
  for enc in vah264enc vah265enc; do
    run "$enc $cap" capture::tests::portal_va_teardown_unparked SECPAR_ENCODER=$enc SECPAR_CAPTURE=$cap SECPAR_CODEC=$([ $enc = vah265enc ] && echo hevc || echo h264)
    echo "[$enc $cap] exit status $?"
  done
done

[ $MODE = gl ] && exit 0
echo "== audio"
run "pipewire-pulse monitor" stream::tests::audio_monitor_produces_opus_frames

echo "== input"
pkill -x weston-simple-e 2>/dev/null; pkill -x eglgears_waylan 2>/dev/null
sleep 1
stdbuf -oL wev >"$HK_LOGS/wev.log" 2>&1 &
wev_pid=$!
sleep 3
run "wev probe" capture::tests::portal_input_reaches_probe SECPAR_INPUT_LOG="$HK_LOGS/wev.log" SECPAR_CAPTURE=copy
kill $wev_pid 2>/dev/null
echo "wev log lines: $(wc -l <"$HK_LOGS/wev.log")"
