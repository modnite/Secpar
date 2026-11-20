# Secpar 4: where it stands

Last updated 2026-11-20. Branch `secpar-4`, merged into `main` for each release.
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
