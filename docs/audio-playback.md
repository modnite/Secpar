# Android audio playback

The user confirms Windows Development 6 fixed video and cursor performance. Rare
slight audio crackle/slowdown remains on the S23 and is absent in Moonlight. Phone
logs confirm the fast AudioTrack path; no useful underrun counters were previously
logged by Secpar. Do not assume a general phone, dock or network problem.

Preview 17 corrects independently verifiable audio defects:

- AudioTrack.WRITE_NON_BLOCKING return values were ignored. Short writes lost their
  tail, while the entire requested buffer was counted as accepted. That could make
  the queue estimate permanently wrong and trigger further sample drops.
- Decoder input was silently skipped when dequeueInputBuffer briefly returned no
  available slot. Output is now drained before bounded input retries.
- Playback started on an empty track. It now primes 30 ms and uses the matching
  Android 12+ start threshold while retaining the platform allocation minimum.
- Delayed decoder output is drained every 5 ms on the same serialized audio thread,
  even when no subsequent packet arrives. Input timestamps advance by 10 ms.
- PCM writes preserve partial tails and count only accepted frames. Backpressure
  waits on the audio thread instead of discarding ordinary decoded PCM. Encoded
  work is capped at 12 pending packets to bound latency after severe stalls.
- AudioTrack's playback head is extended as an unsigned wrapping frame counter.

PcrAudio logs hardware underruns, queued milliseconds, short/zero writes, decoder
input waits, pending packets and overflow drops. PcrReceive adds observed audio
sequence gaps and late/duplicate packets; a later reordered arrival may explain an
observed gap, so the gap counter is not a definitive permanent-loss count.

The PC audio encoder, video timing and protocol remain unchanged. No audio speed
or pitch adjustment is added. A normal startup reserve adds at most approximately
30 ms of audio buffering, in exchange for room to absorb small scheduling bursts.

Verification includes four pure regression tests for partial/zero writes, stop,
errors and head wrap; an encrypted receiver sequence-diagnostic test; and an actual
Android MediaCodec Opus → AudioTrack test with a synthetic 10 ms stereo fixture.
The device test feeds 300 packets in 20 ms batches and confirms all 144,000 expected
sample frames are accepted. This does not prove rare physical S23 crackles are
resolved. Synthetic audio belongs only to androidTest assets, never the release APK.
