# Linux host measurement tools

Development tools. Not part of the product.

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
