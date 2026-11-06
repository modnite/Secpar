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


## Preview 18: real-time recovery and transition smoothing

The user reports Preview 17 hiccups cause permanent audio lag while video recovers;
Moonlight on the same desktop does not have the fault. S23 logs show hardware video
still active, audio underruns rising to 15, a 60–70 ms PCM queue, up to eight pending
audio packets, six overflow packets and six observed audio packet gaps. Partial
writes and decoder input waits remain zero in that session. Preserving delayed
PCM alone was insufficient: capture timestamps were ignored and no recovery reset
existed. Preview 17 is not considered a successful audio fix.

Preview 18 carries the host capture timestamp through MediaCodec and converts it
with the existing PRS clock synchronization. Before the clock handshake completes,
it avoids using an invalid host/local offset. Audio older than 40 ms is discarded;
projected capture-to-output queue delay above 80 ms flushes stale output. Actual
AudioTrack underruns pause/flush/reprime rather than accumulating shifted playback.
An incomplete nonblocking write has a deadline and retains accurate accepted-frame
accounting. The Opus decoder continues consuming stale input so its decode state
can recover without an unsafe asynchronous codec flush. Old/duplicate packet IDs
are ignored. The priming buffer itself is checked for age, including silence gaps.

Five-millisecond fades soften restart edges. A short crossfade from the previous
stereo sample softens a detected sequence gap. These are transition smoothing, not
full Opus packet-loss concealment, and cannot recreate missing music. Video and the
PC host are unchanged.

Five pure timing-policy cases cover starvation, late input, excessive output lag,
underrun and an unsynchronized clock. Two PCM smoothing cases verify silence/previous
sample boundaries, channel balance and untouched steady audio. Real Android tests
cover normal complete sample acceptance and an actual 180 ms starvation followed
by ten stale packets and resumed live input. Both pass: recovery occurs, delayed
packets are rejected, and recent capture age is restored. Physical S23 success is
still pending. The Android audio implementation is MediaCodec/AudioTrack; video
logs independently confirm the Qualcomm low-latency HEVC hardware decoder.
