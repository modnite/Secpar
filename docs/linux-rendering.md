# Linux hardware rendering investigation — 2026-11-07

User reports3.11.0 hardware playback is still worse than Moonlight. Software
was confirmed smooth; received FPS and the current decoder label are requested.
Do not describe automatic fallback as a hardware performance fix.

Current path: H.264 appsrc → h264parse → VA decoder → videoconvert → CPU RGB
appsink → per-row RGB copy → egui ColorImage RGBA conversion → texture upload.
Each input also waits up to100ms for output before the next input is submitted.
There is no native GPU surface presentation in this path.

Moonlight source reviewed:
https://github.com/moonlight-stream/moonlight-qt/blob/master/app/streaming/video/ffmpeg-renderers/vaapi.cpp
Moonlight can export VA surfaces and import them through EGL instead of converting
every decoded frame to a CPU RGB image. That is the path to investigate here.

GStreamer VA1.26 library on the Linux runner exports gst_va_memory_get_surface,
gst_va_memory_peek_display and gst_va_display_get_va_dpy. libva exposes
vaExportSurfaceHandle with DRM PRIME2 descriptors. A native renderer would need:
- VA-memory caps, validated surface export and owned DMA-BUF file descriptors.
- EGL image import with the actual format, plane pitch/offset and DRM modifiers.
- GPU YUV conversion using the stream's color matrix/range, correct orientation.
- Retain the sample until GPU use finishes, then release surfaces and descriptors.
- Bounded input/output, real decode/presentation timings and graceful software
  recovery if export/import/context sharing is unavailable.

Do not just add glupload and label it zero-copy. A negotiation fallback can still
copy through system memory. No accelerated renderer is implemented or verified yet.
The OMV runner has no GPU: it can validate compilation, descriptor ownership,
software-EGL shader output and fallback, but cannot establish X250 performance.
Physical VA/EGL playback must be checked before claiming parity with Moonlight.

Primary references:
https://gstreamer.freedesktop.org/documentation/va/vah264dec.html
https://gstreamer.freedesktop.org/documentation/additional/design/dmabuf.html
https://gstreamer.freedesktop.org/documentation/valib/index.html
