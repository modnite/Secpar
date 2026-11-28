# Secpar 4: where it stands

Last updated 2026-11-21 (4.2.0-r1). Branch `secpar-4`, merged into `main` for each release.
Read this first, then `docs/user-requirements.md` for what the owner wants.
Update this file after each result the owner confirms.

## What Secpar 4 is

The Secpar 3 apps, updated: one desktop app for Windows and Linux that hosts and
connects, and a client-only Android app. Every device streams with Secpar's own
engines. The third-party engines embedded in 4.0.0 were removed in 4.0.9.

| Role | Path |
| --- | --- |
| Desktop window, tray, client | `desktop/` |
| Windows hosting service and installer | `host/src-tauri` |
| Linux hosting | `crates/secpar-host-linux` |
| Shared host protocol, accounts, friends | `crates/secpar-host-core` |
| Client connection | `crates/secpar-client` |
| Stream and network | `crates/secpar-stream`, `crates/secpar-network` |
| Android client | `app/` |
| Account service and site | `cloud/accounts`, `cloud/site` |

## Confirmed by the owner

- Android, Windows and Linux stream to each other (4.0.8 and later).
- Linux hosting on KDE Plasma (Wayland), with the one-time screen sharing prompt.
- Linux pointer lines up with the device's cursor (4.0.10); Show pointer in
  picture off removes the remaining lag.
- Smaller packages without the embedded engines (4.0.9).
- 4.2.0: one Secpar per Linux machine, usernames for who is connected, list padding,
  and the virtual display without a trailing pointer. The driver runs as LOCAL
  SERVICE and reads its settings from `%ProgramData%\Secpar Virtual Display`;
  Secpar's own data folder is closed to it.

## Open

- Friends' computers did not appear on other accounts: hosts reported their relay
  as `wss://…`, which the presence check refused, so no host ever counted as
  shared. Fixed in the next release; needs the owner and a friend on it.
- Friend permission switches only apply to connections made through the account,
  not to devices paired directly as owners.
- A game at 60 fps reached about 30 fps over a friend's stream on Windows: the
  send grid dropped frames whose timing drifted from it. Fixed in the next
  release; unconfirmed.
- Remote connections from work once needed a restart of Secpar on the hosts.
- Android TV (Chromecast with Google TV, 32-bit ARM) runs the same APK: it lists on the
  TV home screen, every focusable control shows a 2dp accent outline, computer cards
  outline as a whole, and the stream menu moves with the remote's arrows. The APK
  carries armeabi-v7a as well as arm64-v8a; the unused QR scanner was removed (2.0 MB).
- 4.2.0-r2, confirmed by the owner on 2026-11-22: KDE screen choice, the virtual display
  size, and lock screen keyboard and mouse.
- 4.2.0, not yet tried by the owner: 4:4:4 on real devices and the relay over TLS on a
  network that blocks UDP.
- Chromecast 4:4:4 (HEVC 1920×2160, GPU reads the decoder output directly, one pass): the
  GPU needed 12–21 ms of the 16.7 ms frame; half-precision colour maths brought it to
  12–15 ms. The too-slow fallback and the stats now count frames actually drawn.
- Laptop 4:4:4: drops with ping spikes after a few minutes. The X250 is on Wi-Fi with
  power saving on; unconfirmed whether that is the cause.
- Chromecast audio trailed its picture: AudioTrack there goes through the mixer's 2048-frame
  blocks (about 130 ms). Stream audio now plays through AAudio (app/src/main/cpp), which reaches
  the MMAP output: 42–56 ms total on the Chromecast, measured. AudioTrack stays as the fallback;
  the AAudio queue is trimmed above 40 ms. AAudio is used only when it lands on MMAP (the
  phone gets the shared path, whose start threshold never filled: no sound). After a gap an MMAP
  stream reads past the write position; the player now reopens it (owner: sound stays on,
  2026-11-23). The owner still hears the TV's audio behind its picture: the remaining delay is
  after HDMI. Not released.
- Ready but not committed or released (owner said not yet, 2026-11-24): sign in with username or
  email (account service change needs deploying to OMV first; apps say "Username or email");
  dark look kept when Windows is in light mode (Bryan saw white fields and buttons); the desktop
  loading screen named the link-paired computer (ULTISUS) when joining a friend's (bryan-bed-pc);
  friend connections now logged by computer id; KDE virtual keyboard typed only one character
  (egui drops input-method commits after the first; plain commits now count as typing; test build
  on the laptop, untried). "Busy" from a friend's PC means its owner is streaming to it.
- Relay for strict networks, 2026-11-24: relay.secpar.win (Worker secpar-rendezvous) had no TURN
  key, so no host could ever allocate a relay (answer 503). It now has the same key as the OMV
  rendezvous (selfhost-tunnel.yml, relay-turn-secrets.mjs); a host-signed request gets
  credentials. Friend streams also had no client-side relay; desktop and Android now take TURN
  from the account (/v1/devices/relay) when joining. Owner streaming no longer blocks or closes
  friends: the PC plus three streaming devices in total. These app changes are not released.
- Mute audio in the stream menu on Android and desktop: this device only, plays silence so
  unmuting stays in sync. Not released.
- Phone (SM-S911B) audio crackled once on the AudioTrack path (18 underruns at stream start, none
  after); gone in the owner's next test on 2026-11-23 without a change for it. Watch for a return.
- Windows window garbled until moved: after an update (the service starts it hidden; reproduced by
  restarting the service) and, as the owner saw on 2026-11-23, when restored after hours minimized
  (not reproduced). r4 nudged only on the first open. Next release: the window moves a pixel and
  back every time it comes back from minimized or hidden, waiting until Windows reports a real
  position (it reports -32000 during the restore). Checked with a temporary log and the service
  restart test. The owner saw it again after under an hour minimized; moving the window by a pixel by hand cured it, so the nudge now also repeats at 0.5 s and 2 s and starts on focus gain (not in fullscreen). A 10-minute minimize restored clean; unconfirmed by the owner.
- Android stats now match the desktop (decoded and drawn fps, 4:4:4) and both refresh
  every half second, with "Ping (RTT)" as the label. Not released yet.

- 4.2.0-r8 (2026-11-27): held keys repeat on the host (Windows does not repeat injected keys;
  Linux hosts drop the extra downs because the compositor repeats). Windows builds carry FFmpeg
  (`-Features 'ffmpeg,gpu-isolation'`, `SECPAR_FFMPEG_DIR`), so hosts offer Quick Sync and AMF and
  no longer Media Foundation. Unconfirmed by the owner.
- Open from the owner's T480 (Windows 11 client) report, 2026-11-26: no keyboard reaches the host
  and Win opens the local Start menu; choppy picture and sound with macroblock pulses to NIGG10
  (NVENC host, so client or network side); about one second of delay to Bryan. Bryan's bed PC is
  unreachable on the LAN since r7; its ufw allows only KDE Connect and 22 (needs 48999/tcp).
- 4.2.0-r9 (2026-11-27), untried: the Windows client's hook now takes keys injected by other
  software (only keys a Secpar host marks with dwExtraInfo 0x5EC9A400 pass), and the window path
  forwards keys the hook let through; hook trips log a `keys` line. Linux hosts follow the client's
  REPORTs with the Windows host's controller (now `secpar-host-core::bitrate`). The owner clarified:
  the one-second delay was to Bryan's Windows desktop, Bryan's ufw is off, and bryan-bed-pc has not
  connected at all since r7 (control connects, no stream packets, no `turn:` lines). Next: the T480's
  stream stats (decoder, decoded/drawn fps, ping, loss, bitrate) to NIGG10.
- r9 on the T480 (owner, 2026-11-27): letters type, Shift and combinations did not, and Fn functions
  went to the host. So the hook was tripping, not meeting injected keys. 4.2.0-r10: the hook's focus
  check accepts any window of this process; the window path sends modifiers and copy/paste too;
  injected keys stay local again. The service read `-rN` versions as unparsable and refused every
  revision, so the app opened the interactive installer; fixed (hosts on r9 or older still get it
  once). Hardware decode falls back only after two slow seconds (T480 4:4:4 1080p60 fell to software
  at 6-14 fps). Android's input is the standard for all clients (owner).
- 4.2.0-r11: the Windows client draws D3D11-decoded frames from the GPU (video/dxgl.rs splits NV12
  into R8/RG8 textures; gpu.rs samples them through WGL_NV_DX_interop2; copy-back stays as fallback).
  Seen working on NIGG10 (NVIDIA) streaming the T480 at 1080p 4:4:4: stats read "drawn from the
  GPU". Not yet tried on the T480's Intel graphics. Found: NIGG10 streaming the T480 at work drops
  after about 2 s of direct video; the client allocates a relay and sends stream.cand, but the
  T480's host never answers with its relayed address, so the stream ends after 15 s. r11's Windows
  host logs each relay step; read the T480's host log next. The Windows host has no TURN-over-TLS
  fallback (the client and Linux host only try UDP to the relay too on the host side).
- 4.2.0-r12/r13: Settings has one General tab (Account, About, Logs). Save logs copies connect.log and
  the hosting log (Windows: service command `host_log`, newest 360 KB; Linux: Secpar lines from the
  user journal) into "Secpar logs <time> UTC" in Downloads. Owner used it on NIGG10. Next: the same
  from the T480 after a stream from NIGG10 dies.
- T480 host log (owner, 2026-11-27): every relay attempt "turn: allocation failed" from 141.101.90.1:3478;
  the office blocks UDP to the relay. 4.2.0-r14: Windows and Linux hosts fall back to the same relay
  over TLS (turns: entry, port 443); the TLS link moved to secpar-network (feature "tls"), frames reach
  the stream socket through loopback. Checked against Cloudflare over TLS; not yet tried at the office.
- r14 on the T480 at the office: "trying TLS" then "turn: allocation failed" within a second, no
  request error, so no answer reached the request through the loopback hand-off (works on NIGG10).
  4.2.0-r15: the relay's answers go straight to the waiting request (client's way); only relayed
  stream packets use loopback. Logs "TLS connected to", and "N frames came back over TLS" on failure.
- r15 on the T480: "TLS connected", then 0 frames. A PowerShell probe on the T480 at the office got
  answers from the relay over UDP 3478, TCP 3478 and TLS 443/5349: the office blocks nothing. Real
  cause: the device's first PROBE arrived directly, then its direct path died; the host ends a stream
  after 5 s of silence once the peer is known, and the device's relay candidate arrives about 3 s in,
  so the host's relay setup ran into the shutdown (and the stopped stream dropped the TLS link).
  4.2.0-r16: `Link::searching()` (a candidate inside the punch window, or a relay setup running) keeps
  the Windows and Linux receive loops open. Not yet tried at the office.
- r16 on the T480, even on the home LAN: same pattern, and NIGG10's own host allocates its relay
  every time. Since r8, Intel hosts encode with FFmpeg Quick Sync (never confirmed working). QSV
  keeps each input surface until the next frame is sent, but the frame pool had one surface, so the
  next frame's buffer failed, and `encode(..) else { break }` ended the stream silently (which also
  stopped the receive loop: no relay answers, 0 TLS frames). 4.2.0-r17: QSV pool of 4; encode
  errors are logged and the encoder reopened (stream ends after 4 in a row, logged); the chosen
  encoder is logged at each open. Not yet tried on the T480.
- r17 on the T480: "FFmpeg: Unknown error occurred (-1313558101)" at connect: QSV refuses a pool of 4.
  4.2.0-r18: pool of 1 again, and the one hardware frame is taken once and sent for every picture
  (Sunshine's way), so QSV's held reference no longer blocks the next frame. Loopback test passed on
  NIGG10 with ffmpeg-nvenc (same bridge code). Owner rejects Media Foundation as an encoder, even as
  a fallback. Owner confirmed r18 streams from the T480 (NIGG10 client, LAN).
- r18 T480 stream: occasional black flicker; host log shows no encode failures, but the client
  reports 12-29% loss on the LAN (rtt 2-16 ms). Cause of the flicker not found yet.
- 4.2.0-r19: stream menu on desktop and Android: Mute audio at the top (desktop: under the window
  toggle), Share clipboard only in Settings, Stream stats opens a page with Show stats bar and the
  audio details (the old Audio page). Desktop: closing the window during a stream disconnects it.
- T480 is on Wi-Fi; NIGG10 and Android both receive about 30 fps from it, with 12-29% loss. NIGG10
  shows a black flicker, Android does not (so the NIGG10 client's GPU draw path is the suspect).
- 4.2.0-r20: Windows host and client hold Wi-Fi media streaming mode during a stream (no background
  scans; `secpar_network::wlan`), Android holds a low-latency Wi-Fi lock on the stream screen. Linux
  needs root to change Wi-Fi power save, so nothing there. Not yet measured on the T480.
- Next, owner asked (2026-11-27), from the Pulsar comparison: (1) NACK retransmission and recovery
  from an earlier frame instead of a full keyframe; (2) delay-based bitrate cuts before loss;
  (3) pick the faster of direct and relay, NAT-PMP port mapping; (4) Direct3D drawing on Windows.
- NACK (item 1), 4.2.0-r21; unit-tested on both clients, not yet measured on a lossy link. Design: today `Assembler` (secpar-client/receiver.rs, Android
  prs/FrameAssembler.kt) drops a frame FEC can't rebuild after 60 ms or when a newer frame
  completes, sets need_key, and then discards every frame until a keyframe. New `Type::Nack = 10`
  (client to host): frame_id u32, then u16 frag indices. Hosts keep the sealed packets of the last
  ~300 ms of frames and resend the asked ones unchanged (no replay check in Crypt::open; the
  assembler ignores duplicates). Clients NACK the missing frags of short FEC blocks once a newer
  frame's packet shows the gap (re-NACK once after ~RTT), hold later complete frames behind the
  incomplete one, and give up after a repair window (then need_key as before). Old hosts drop the
  unknown type, so old and new mix safely.
- 4.2.0-r22 (item 2): clients measure frame queueing delay (capture to first packet, minus the
  smallest gap of the last 5-10 s; `QueueDelay` in secpar-client/receiver.rs and Android
  prs/QueueDelay.kt) and send it in REPORT bytes 12-13. `Bitrate::on_report_queue`: two reports
  at or above 25 ms count as congestion (cut x0.85, as for queueing RTT) and a report above the line
  holds off raises. Hosts log it in the bitrate line. Stream menu labels elide and its scroll bar
  sits beside the rows (the Displays page overflowed in a 375x400 window on the X250 host).
- 4.2.0-r23 (item 3, first half): NAT-PMP (`secpar_network::natpmp`, RFC 6886) when no UPnP
  router answers: Windows host maps its agent and stream ports through it (renewed hourly, 2 h
  lifetime), per-stream hosts (Linux, `reach::gather`) map the stream port (4 h, removed at the
  end). Gateway from GetBestRoute / /proc/net/route. NIGG10's router (192.168.100.1) does not speak
  NAT-PMP, so only the fake-router test covers the mapping. Still to do: time direct vs relay and
  keep the faster path (item 3b), Direct3D drawing (item 4), NIGG10 black flicker.
- T480 log, r23, Android client (Hevc 4:4:4 packed, encoder 1280x1440 QSV): resent 37 packets in
  10 s, media streaming mode on, no bitrate cuts; still 20-40 fps received and drawn, so the T480
  produces too few frames. 4.2.0-r24 logs every 10 s per stream: captured/sent/repeats per second
  and convert/encode avg and max ms (`Pace` in host session.rs). Next: read that line from the T480.
- 4.2.0-r24 (item 3b): `PATH_TEST` PROBE flag (header.rs) and `pathprobe` cap on both hosts (guest
  filter passes it with view): hosts answer a test probe to its sender without moving the stream.
  The desktop client, against such hosts, starts its relay 8 s into a direct stream without
  announcing it, sends a test probe on the other path every second, and switches when five
  samples each show the other path 30 % and 20 ms faster (`PathTimes`); after a switch it ignores
  the old path for 2 s and tests again after 10 s. Android has no path tests yet. Not tried live.
- r24 Pace lines: T480 (UHD 620, QSV Hevc 4:4:4 packed 1280x1440): encode 30 ms avg, so ~30 fps;
  at notch 1 (960x1080) 18 ms and ~48 fps. Convert <0.5 ms. NIGG10 (NVENC): encode ~6 ms, but
  DXGI delivers only 0-35 new frames/s (repeats fill to 60) with 25-37 ms convert/encode spikes:
  NIGG10 itself is loaded (OpenCode desktop since 2026-11-14, ~30 processes, ~56% CPU; old
  esbuild watchers), outside Secpar. 4.2.0-r25: QSV reports whether it opened in low-power
  (VDENC) mode; a host skips a client codec the GPU can only run in its slow mode when a later
  one opens fast (HEVC on pre-Ice-Lake Intel), and logs "(no low-power mode on this GPU)".
- r25 confirmed by the owner (2026-11-28): the T480 picks H.264 in low-power mode, ~10 ms per
  720p 4:4:4 frame, 60 fps to one or two viewers. With three viewers each had its own encoder:
  15-16 ms per frame and 40-50 fps. 4.2.0-r26 (Windows hosts): viewers of the same display share
  one capture and one encoder when they decode its codec (and 4:4:4 if packed). The encoder runs
  at the lowest bitrate and the largest step-down any viewer needs; a joining viewer gets frames
  from the next key frame; each viewer's packets are paced out on its own thread. A viewer that
  switches display leaves for that display's pipeline; size requests wait until a viewer has the
  pipeline alone. Linux hosts still encode per viewer.
- Open: Android and Chromecast audio drops out for 1-2 s at times (desktop clients don't). Android
  discards audio older than 160 ms and restarts its decoder with a back-off of up to 1 s; the
  desktop client only caps its buffer at 200 ms. Needs the Stream stats audio counters after a
  dropout to tell which.
- Known failing test, not caused by recent work: secpar-network
  `link::tests::relayed_peers_get_wrapped_replies_and_the_rate_gate` fails on NIGG10 at r16 too
  (the reply after the permit never reaches the fake server within 100 ms).

## Account service: self-hosted since 2026-11-21

On 2026-11-21 the Cloudflare account ran out of the free plan's 100,000 Worker requests a day
(error 1027), and sign-in stopped for everyone. The account service now runs on the OMV build box:

- `/opt/secpar-accounts` on its system disk (never the data disk): the service under `wrangler dev`
  in the `secpar-accounts` Docker container, data in `state/accounts` (SQLite, from the D1 export),
  grant signing key in `accounts/.dev.vars` (new, root only), hourly job from root's crontab.
- `accounts.secpar.win` reaches it through the `secpar-home` Cloudflare Tunnel (`secpar-tunnel`
  container); tunnel traffic is not counted against Worker requests. `/__scheduled` is blocked there.
- The relay stays on its Worker (`relay.secpar.win`): hosts' relay keys live in its storage and
  cannot be exported, and its traffic is light.
- The deploy workflow refuses to deploy the account Worker, which would take the name back with
  stale data. `selfhost-tunnel.yml` and `accounts-export.yml` are the one-off jobs used.
- Email works again since 2026-11-22: a sending-only Resend key for mail.secpar.win, in
  `accounts/.dev.vars`. Since 2026-11-22 the rendezvous also holds a Cloudflare TURN key
  (`secpar-self-hosted`) in `rendezvous/.dev.vars`, so friends get relay credentials again.
  `selfhost-tunnel.yml` with `turn-key.mjs` replaces it (API token needs Cloudflare Realtime: Edit).
- Apps poll less (desktop alerts 60 s, owned list 120 s; Android alerts 60 s) and keep the last
  known computers when the service is unreachable, connecting with their saved keys.
- Next release, not yet deployed: hosts stop asking for owner pairings every 3 s (about 28,800
  requests a day each); the service holds `GET /v1/host/owner-pairings?wait=25` until a device asks.
  Apps whose live channel is connected poll only every 10 minutes. Old hosts and an old service
  still work with either side.
