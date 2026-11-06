# Linux decoding and Android audio timing: 3.8.0

Linux video decoding and RGB conversion run off the UI thread. A three-frame
compressed queue discards dependent frames after overflow, waits for an IDR and
requests recovery at most every 250 ms. Only the latest decoded image is retained.
VA-API is offered only after a real synthetic H.264 frame is decoded correctly
through vah264dec/vaapih264dec on the current machine. An available plugin alone
is insufficient. Runtime failure falls back to software at a fresh keyframe.
Windows/macOS hardware backends are not implemented by this change.

Real decode, queue overflow and native-pipeline failure/recovery tests pass on
Linux. The runner has no hardware decoder; it validates the pipeline with its
bundled software plugin and explicitly induced native error, not a pretend GPU.
The actual X250 VA-API driver and playback still need user validation.

Android keeps host capture timestamps beside queued PCM sample counts. Decoder
output PTS no longer determines capture age: some Opus implementations synthesize
a continuous sample clock, which cannot represent pauses or missing packets.
Zero-size/configuration output is ignored. A bounded ten-second low-delay probe
window replaces the lifetime-minimum clock estimate, allowing clock drift and
route changes to recover without adopting one slow echo as the new anchor.

Audio diagnostics distinguish incoming age, decoded capture age, decoder PTS
offset and sample-mapping misses. They contain timing counters, never audio.
No physical S23 success is claimed until tested. Existing masking fades remain.

Windows capture correction is in 3.7.0. Release 3.8.0 updates Linux and Android;
Windows remains on its separately verified 3.7.0 installer. Debug/review packages
stay local. Native account-owned discovery work remains on main and is excluded
from this release until Android account integration and Linux remote routes work.
