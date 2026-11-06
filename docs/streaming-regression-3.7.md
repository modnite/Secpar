# Shared capture regression: 3.7.0

The user confirmed 3.6.0 allows two clients, but video and cursor regress on
both Android and Linux even with one viewer. Audio recovery remains unresolved.

The shared producer continuously called DXGI AcquireNextFrame with an 8 ms
timeout on the protected D3D device also used by hardware encoders. Conversion
and encoder submission can stall behind that wait. Restore zero-wait acquisition
and yield for 1 ms only when no event is available. Preserve the single duplication,
bounded immutable texture leases and independent encoders.

Controlled same-binary loopback A/B, with 10% simulated packet drops:
- 8 ms wait: capture-to-receive p50 31.7 ms, p95 56.7 ms; latency check failed.
- Zero wait: p50 3.8 ms, p95 7.8 ms; passed.
- Earlier failing run: conversion 25-29 ms, encode 62-70 ms, p95 166.9 ms.
- Fixed run: conversion 0.03-0.06 ms, encode 1.7-2.6 ms, p95 5.5 ms.
These are local measurements, not a physical S23/X250 success claim. A still or
partially changing desktop is not a 60 fps animation benchmark.

Strict concurrent hardware check still passes HEVC plus two H.264 streams,
fourth-device refusal, disconnect/replacement, and last-viewer capture shutdown.
Actual decoded smoothness on the S23/X250 still requires validation.

Desktop branding now uses Lanczos downsampling to physical display pixels before
GPU upload, refreshed after DPI changes, preserving the approved PNG geometry.
The full-size texture previously used only bilinear minification at 112x27.

Audio: S23 3.6.0 logs show queuedMs=0 and staleFrames increasing by 100/second
indefinitely after failure. A live source-only timestamp diagnostic had no audio
playing and was inconclusive. No speculative audio fix is included in 3.7.0.
