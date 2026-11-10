# Linux host measurement tools

Development tools. Not part of the product.

## Client and Windows-host frame timing

Set `SECPAR_FRAME_TIMING=1` before starting a local review build. It is off by default.
Every 300 samples it writes interval or duration histograms to stderr. No video,
clipboard text, account identity or connection credentials are logged.

Stages: host capture, selected encoder input, colour conversion, encoding and
encoded output; client complete-frame arrival, decoder submission, decode time,
decoded output, texture upload time and fresh-frame draw interval. Arrival uses one
process clock even when Tokio changes threads. Host counters are per capture/encoder
thread. Measure one client stream at a time. Histograms use 0.5 ms bins; reported
percentiles are bin lower bounds. The final bin includes intervals of 255.5 ms or
longer. `max_us` preserves the longest hitch.

The draw measurement is when the GL callback consumes a new texture. It is not a
swap-completion or display-scanout timestamp. A virtual display cannot establish
physical Wayland refresh pacing. Compare host and client interval distributions;
do not subtract timestamps between unsynchronised computers.

`sh test/linux-perf/frame-pacing.sh /absolute/secpar-desktop-review output.log`
generates 720p60 H.264, opens the real software decoder/GL path in Xvfb for 16 seconds
and checks that measurements exist. It uses a temporary profile and never connects
to a host. The binary needs `--features ui-review`. This fixture checks the timing
path; it does not prove the user's laptop or NVIDIA hardware performs correctly.

## Linux-host measurements

They run the real Linux host against a headless KDE Plasma session in a container on a machine with an Intel GPU.
`hk/` builds the session image (KWin on a virtual output, PipeWire, the KDE portal, an auto approver for the consent
dialog, a click helper). `sync.sh` copies the sources to the build machine.

- `hostrun` runs the host backend and writes a one time pairing link to a file (mode 0600).
- `perfclient` is a headless client. It prints frames per second, bitrate, frame age, and with `--decode` the time from a
  pointer jump to a decoded frame that shows it. `--script "0=pointer:sweep,4=size:1280x720,9=end"` drives it.
- `capprobe` measures what the compositor delivers before any encoder.
- `encprobe` runs the host's own capture and encode chain without a network and reports the same latency, plus
  live size changes with `RESIZE=1280x720,854x480,0x0`.
- `scenario.sh` and `sum.py` run one scenario against one build and summarise it.

Do not take screenshots of the KWin workspace with ScreenShot2 while a portal consent dialog is open. KWin 6.8.2 crashes.

## Numbers (1366x768, 60 fps, vah264enc, Intel iHD)

| | 3.16.4 | 3.16.5 | this branch | this branch, SECPAR_CURSOR=hidden |
|---|---|---|---|---|
| still screen, frames/s, host CPU | 30.7, 25% | 30.7, 21% | 30.5, 25% | 30.6, 24% |
| pointer sweeping, frames/s, host CPU | 30.7, 27% | 54.0, 44% | 53.9, 51% | 30.6, 28% |
| pointer jump to decoded frame, median | no pointer in video | 223 ms | 76 ms | no pointer in video |

The encoder delay measured per element (GStreamer latency tracer): vah264enc 112 ms average before, 2.2 ms after.
