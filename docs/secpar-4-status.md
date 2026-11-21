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
- 4.2.0, not yet tried by the owner: 4:4:4 on real devices, the relay over TLS on
  a network that blocks UDP, KDE screen choice, the virtual display size, Linux
  lock screen input through uinput (owner sessions only) and Windows lock screen
  input after the per-session pointer change.

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
- Not yet on the box: email (needs a new Resend key) and TURN credentials for friends (needs a new
  Cloudflare TURN key).
- Apps poll less (desktop alerts 60 s, owned list 120 s; Android alerts 60 s) and keep the last
  known computers when the service is unreachable, connecting with their saved keys.
