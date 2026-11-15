# Secpar 4: where it stands

Last updated 2026-11-15. Branch `secpar-4`. Read this first, then
`docs/secpar-4-plan.md` for the design and `docs/user-requirements.md` for what
the owner wants. Update this file after each result the owner confirms.

## What Secpar 4 is

A rebuild of Secpar for Windows 10/11, Linux and Android (macOS and iOS are out
of scope). It embeds Sunshine as the hosting engine and Moonlight as the stream
component, both unmodified at the versions in `upstream/pins.json`, with a small
Secpar layer around them: a tunnel, one identity per installation, and access
decided by the account. To the person it is one app; never present the engines
as separate products.

## Order of work the owner set

1. Hosting on the Windows PC. Working.
2. The Android app. Working on the home network and over mobile data.
3. Connecting from outside the home network. Working by direct connection.

Linux, desktop window polish, friends and extras come after these are solid.
The owner uses the Android app from work to reach the home PC; that use comes
first in every trade-off.

## Confirmed by the owner on real devices

- Phone to PC video on the home Wi-Fi: sign-in, device list, tunnel, automatic
  pairing, stream.
- Phone to PC over mobile data, with the home line behind the provider's shared
  address (no port forward possible) and no relay key configured.
- Android to Windows over the home LAN and over cellular data (owner, 2026-11-15).
- Sound and touch in the phone app (owner, 2026-11-15). Controllers are not tried.

## Checked locally, not by the owner

- Desktop window to the real Sunshine engine on this PC over loopback: the window
  started the real Moonlight component, it paired through the tunnel, and it
  received and decoded 1920x1080x60 HEVC with D3D11VA and started the audio
  stream. The window showed Streaming. Both ends were on one PC, so picture and
  sound were not judged on a second device.
- Relay check on this PC: the tunnel socket is seen from the internet (STUN
  works), but the account service answers `relay_not_configured` with 503. Direct
  connections are unaffected.
- Desktop Settings now reads and saves the real stream choices on the existing
  connection path: resolution, frame rate, bitrate, decoder, display mode, audio
  and mute on focus loss. Settings → Hosting shows the real host state, allowed
  device count and access-list expiry. The window was run signed in against a
  local background part; the settings API round-trip and validation are tested.
  Captures: `settings-connecting-640x400.png` and `settings-hosting-640x400.png`.

## Built but not confirmed

- Controllers in the Android app.
- The owner's work network. If it blocks UDP, a relay key is needed (see below).
- Hosting on Linux through `crates/secpar-hosting`; only container runs so far.
- The relay path. It is written on both sides and unused, because the relay
  service has no TURN key.

## Parts

| Path | What it is |
| --- | --- |
| `crates/secpar-tunnel` | QUIC tunnel with pinned certificates on both ends. Streams carry TCP, datagrams carry UDP. `relay.rs` puts address lookup (STUN) and the relay (TURN) on the tunnel's own socket. |
| `crates/secpar-network` | Secpar 3's STUN, TURN and UPnP code. The tunnel uses its TURN client. |
| `crates/secpar-core` | Device identity, secret stores, the account service client, the access list. |
| `crates/secpar-gamestream-host` | Starts and supervises the hosting engine, bound to this computer only. |
| `crates/secpar-hosting` | Ties the access list to the tunnel and the engine. `keep_current` runs three loops: keep the list fresh, answer devices that ask to meet (`meet`), send packets towards them (`punch`). `outside.rs` finds the internet address and asks the router for a port. |
| `crates/secpar-service` | The background part on a desktop. Holds the device key, hosts, opens tunnels for the window. |
| `crates/secpar-app` | The window's side as a library (`secpar_app.dll`). |
| `desktop-qt` | The Qt window. |
| `crates/secpar-android` | The Android app's core, called over JNI. |
| `android-secpar` | Java files laid over the pinned Moonlight Android source by `scripts/build-android.py`. |
| `cloud/accounts` | Account service (Cloudflare Worker and D1). |
| `cloud/rendezvous` | Secpar 3's relay service. Secpar 4 asks it for relay access through a service binding. |

## How a device reaches a computer from outside

1. The hosting computer reports its home addresses and tunnel port with
   `POST /v1/device/access`.
2. The device tries those addresses for 1.5 seconds.
3. If nothing answers, the device asks a public server where its tunnel socket
   is seen from the internet, takes a relay address if the service offers one,
   and calls `POST /v1/devices/:id/knock` with those addresses.
4. The hosting computer waits on `POST /v1/device/knocks` (held open up to 20
   seconds), checks the device against its access list, answers with its own
   addresses, and sends small packets towards the device for 12 seconds.
5. The device connects to the first address that answers as the expected host.

In both address lists the first entry is the direct address (`0.0.0.0:9` when
unknown) and the second, if present, the relay address.

## Account service routes added for Secpar 4

`/v1/devices/challenge`, `POST /v1/devices`, `GET /v1/devices`,
`DELETE /v1/devices/:id`, `GET /v1/devices/:id/access`,
`POST /v1/devices/:id/knock`, `POST /v1/devices/relay` (signed-in person);
`GET /v1/device/time`, `POST /v1/device/access`, `POST /v1/device/knocks`,
`POST /v1/device/relay` (signed with the device key). Migrations `0009` to
`0011`. The live service is deployed from `main`, which carries only the
`cloud/` and workflow changes; `secpar-4` itself is not pushed.

## Known gaps, most useful first

1. Hosting only runs while the owner is signed in to Windows. Verified
   2026-11-15: `Secpar 4 hosting (test)` has a logon trigger and runs
   interactively as the owner, so it starts after sign-in and stops at sign-out.
   The running `Secpar` service is the old installed Secpar 3, not this. A service
   alone cannot capture the signed-out desktop; the practical choices are
   automatic sign-in to a locked session (if capture works there) or accepting
   that the PC stays logged in. Administrator rights and the owner's go-ahead are
   needed either way.
2. No relay key on the relay service, so networks that block direct connection
   cannot connect. `relay_not_configured` is the answer today. Setting
   `TURN_KEY_ID` and `TURN_KEY_TOKEN` on the relay service is the owner's step;
   no code change is needed after that.
3. A computer shows as online for up to 20 minutes after it stops.
4. The Android app is a debug build named "Secpar 4 (test)". No release build,
   no updater path from Secpar 3 yet.
5. The hosting computer polls the account service continuously for knocks. Fine
   for a few computers; replace with the relay service's WebSocket before many
   people use it.
6. Relay credentials last six hours; a relayed stream longer than that has not
   been thought through.
7. Desktop window: the real Moonlight component now streams from the real
   engine over loopback. A check between two physical devices, previews, the
   in-stream menu and packaging remain.
8. Friends and guest access through the new access list: service side exists,
   no screens.
9. Installers, Linux packaging, updater compatibility with the old versions.

## Working rules that came from the owner

- Work lean. Moonlight and Sunshine are known to work; do not build test rigs
  or proof rounds for each step. One quick check, then the owner's devices are
  the test.
- Reuse what Secpar 3 already runs (relay service, account service, keys).
  Secpar 4 is an update, not a separate system.
- Never say something works without the owner's result or a real check. Say
  plainly what was not run.
- Usage is tight. No long write-ups, no speculative work.
