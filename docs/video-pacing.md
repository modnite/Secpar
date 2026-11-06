# Android video pacing

Preview 14 addresses two independent backlog problems in the Android video path.
Compressed input has a short age/count budget. If discarded, dependent pictures
are held until a new keyframe; IDR requests are throttled and retried. A fresh
keyframe can safely replace older queued encoded pictures. Codec feed requests
are coalesced rather than posting one handler task for every arriving picture.

Decoded output keeps at most two codec-buffer leases. Surplus decoded pictures
are released without rendering, preserving decoder reference frames. Choreographer
presents at most one picture per display tick, with minimum spacing for streams
slower than the display. Timestamped surface release prevents catch-up bursts.
Held output leases are released during teardown. The video surface receives a
fixed-source frame-rate hint, and codec operating rate matches the requested fps
rather than requesting 32767fps. Audio code and host routing are unchanged.

Diagnostics under PcrVideo every five seconds report presented fps, decoded output
drops, input recovery count and average arrival-to-presentation-submission time.
Output drops can be normal when stream fps exceeds physical display refresh; they
are distinct from packet loss. These are not end-to-end capture latency metrics.

Validation: eight queue regression tests cover overflow dependencies, stale input,
keyframe recovery and retry limits, burst output, 60fps/120Hz spacing and teardown.
A debug-only SurfaceView test uses a synthetic 320×180 H.264 fixture at 120fps,
then PixelCopy verifies a blue picture was actually presented. The fixture and
review activity are excluded from the release APK. Emulator results do not prove
1080p/120 HEVC performance on Qualcomm hardware; S23 real-stream comparison remains
required. No phone app or installed PC service was replaced during testing.

The display-paced approach is informed by [Moonlight's renderer](https://github.com/moonlight-stream/moonlight-android/blob/master/app/src/main/java/com/limelight/binding/video/MediaCodecDecoderRenderer.java)
and the [Android MediaCodec surface-output contract](https://developer.android.com/reference/android/media/MediaCodec).
The Secpar queue implementation is independent and uses the existing PRS protocol.

The test asset is generated locally, not captured from a user's screen:

```
ffmpeg -f lavfi -i color=c=0x2070b0:s=320x180:r=120 -frames:v 240 -c:v libx264 -preset ultrafast -tune zerolatency -pix_fmt yuv420p -g 15 -x264-params aud=1:repeat-headers=1:scenecut=0 -f h264 video-pacing.h264
```


## Preview 15: packet integrity and upstream measurements

The user reported Preview 14 unchanged or worse. Phone logs showed low presented
fps during the bad interval while local decode/submission stayed around 12ms,
with no growing output drops or input recovery count. Pacing alone did not solve
the reported symptom; upstream delivery and frame assembly need measurement.

A separate full-duplex defect was reproduced: Crypt had independent encrypt/decrypt
Cipher instances but shared their mutable nonce scratch buffer. The receive thread
and sender could overwrite that array concurrently, invalidating authenticated
packets. Separate per-direction buffers preserve the same wire format and avoid
serializing video reception behind input sends. The sender's existing lock and
single receive thread still serialize calls within each direction.

The JVM regression failed 29 of 40,000 packet checks before the fix and zero after.
Android's actual crypto provider also passed 12,000 simultaneous video/input packet
checks on the isolated emulator. These tests demonstrate the defect/fix, not a
measured real-world loss percentage or proof that all video stutters are resolved.

PcrReceive now logs receive fps/bitrate, packet loss, cumulative frame-assembly
losses, invalid packet count, maximum per-packet processing wall time and socket
backlog/drop counts every five seconds. The measurements distinguish missing input
frames from decoder backlog without recording screen content, keys or credentials.


## Host packetization bottleneck (Desktop Development 6)

The user reports no improvement with Android Preview 15. The corrected symptom is
that video and cursor problems go away when video playback is paused; the cursor
itself does not disappear. Physical S23 logs show direct LAN UDP to
192.168.100.130, 4–11 received video frames/sec during playback, zero reported
packet loss and no invalid, assembly-dropped or socket-dropped packets. This is
not evidence for another decoder-queue adjustment.

The host packetizer constructed a ReedSolomon instance for every block of every
frame. That creates/inverts a coding matrix, synchronously on the video thread
that also polls cursor updates. A synthetic test in the same Windows host debug
build profile measured:

| Encoded frame bytes | Before (ms/frame) | Matrix cache + optimized shared crate (ms/frame) |
| --- | ---: | ---: |
| 16,384 | 1.961 | 0.216 |
| 65,536 | 71.466 | 1.631 |
| 196,608 | 237.353 | 5.391 |

Each measurement averages 60 frames of the same size, including initial cache
construction. No capture, installed service, actual screen content, network or
encoder was involved. These are packetization timings, not end-to-end latency or
a promise of physical stream performance. The fix uses a bounded 128-entry shared
LRU of coding matrices and optimizes secpar-stream in debug preview packages.
Uncached-versus-cached wire-byte comparisons and concurrent loss recovery tests
pass; Android and the on-wire protocol are unchanged. The PC needs the updated
single Secpar installer to receive this fix. Physical comparison remains pending.
