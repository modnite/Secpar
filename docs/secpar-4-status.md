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
