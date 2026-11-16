# Secpar 4: the shipping plan

Written 2026-11-15 after the owner's review of the Qt-based 4.0 builds. This
replaces the Qt window and the bundled Moonlight and Sunshine applications as
the way forward. Secpar 4 ships as the next update of Secpar 3 on secpar.win.

## What the owner asked for, all of it

- Sound that stays right through a whole session.
- A streaming engine better than 3.x: the Moonlight protocol's pacing, FEC and
  audio handling, with Sunshine's capture and encoder handling on the host.
- No duplicate paired entries, and the existing x250 duplicates cleaned up.
- A UI better than Secpar 3, not barer.
- Live previews of every computer you can reach, on desktop and Android: your
  own computers and friends' hosting computers you are paired with.
- Secpar 3's file sizes and connect speed.
- No Qt. Nothing from Moonlight's or Sunshine's own interfaces.

## Size budget (Secpar 3.20.6 in brackets)

| Package | Budget | 3.20.6 | Qt 4.0.1 |
| --- | --- | --- | --- |
| Windows installer | 10 MB | 4.2 MB | 40 MB |
| Linux AppImage | 10 MB | 8.7 MB | 125 MB |
| Android APK | 10 MB | 8.7 MB | 14 to 19 MB |

A build over budget is not published. The owner raised the budget from 6 MB
to 10 MB on 2026-11-15.

## The engine without the applications

The engines are used as libraries inside Secpar 3's own apps.

Connecting side (desktop and Android): `moonlight-common-c` only. It is the
Moonlight protocol core in C, about 650 KB of source: RTSP setup, the control
channel, video depacketising and FEC, and audio with its own FEC. It is compiled
into the desktop app through the `cc` crate and into the Android app through the
NDK. Its frames go to decoders Secpar already has or the system provides:

- Windows: Media Foundation's hardware H.264 and HEVC decoders (part of Windows,
  nothing shipped), with OpenH264 as the fallback that 3.x already carries.
- Linux: VA-API through the system's `libva`, loaded at run time, falling back
  to OpenH264. Nothing from the graphics stack is bundled.
- Android: `MediaCodec`, as the 3.x app already does.
- Audio: Opus through the `opus` crate on desktop (already in 3.x) and the NDK
  decoder path on Android, played with cpal and AAudio. Lost packets are
  concealed by the decoder instead of the stream stalling.

Hosting side: Sunshine built as a headless engine. No web interface (11 MB of
assets today), no tray, no Steam or app-list artwork, and FFmpeg configured with
only the hardware encoders Secpar uses (NVENC, QSV, AMF, VA-API) and the
software H.264 fallback. Today's 30 MB executable is mostly FFmpeg and Boost
linked in with everything enabled; the trimmed build is measured against the
budget before anything else depends on it. If a trimmed Sunshine cannot fit, the
host keeps Secpar 3's own capture and encoders and answers the same protocol
from its own code instead, using Sunshine's sources as the reference.

The engine runs behind Secpar 3's existing connection, as in
`docs/gamestream-engine.md`. The control connection, accounts, pairing, relay
and friends are the ones 3.x already has, so connecting is as quick as 3.x.

## Duplicates

- One device identity per installation, kept in the system's secret store, is
  what the client pairs with. Reinstalling or updating never makes a new one.
- The host merges records that share a device key, and on first start of 4.x it
  folds the repeated x250 records into one, keeping the newest.
- Settings, Devices lists each device once with "Remove" beside it.

## UI plan

Starting point: Secpar 3's approved desktop direction (`docs/ui-review.md`, 2026-11-05)
with its shared control heights and keylines, and the Secpar 4 goals that were
right: one app with both roles, the account's computers listed without pairing
codes, and previews.

Desktop (egui, the renderer 3.x already uses, one window on Windows and Linux):

1. Header: triangle, wordmark, version, then three tabs: Computers, Friends,
   Settings. Nothing else.
2. Computers: a grid of tiles, three across at 800 wide and two at 640. Each tile
   is a live preview (a still refreshed every few seconds while the page is in
   view) with the name, owner for friends' computers, state and route (This
   network, Direct, Relay) under it. Your computers first, then friends'
   computers you are paired with. Clicking the preview connects; there is no
   separate button. An offline tile is dimmed and shows when it was last seen.
3. Friends: one row per friend: name, @handle, Connect (Ask each time or Always),
   Keyboard and Mouse as separate switches, Remove. Requests on top with
   Accept and Decline.
4. Settings: left section list: This computer (hosting on or off, who may join,
   players up to three), Streaming (resolution, frame rate, bitrate, decoder,
   all as labelled values), Controls (the one-hand release shortcut), Devices
   (each once, with Remove), Account, About and updates.
5. In-stream: the Secpar button opens a small panel: picture size and rate,
   players, send file, clipboard, disconnect. Release shortcut shown there.
6. Fits 640x400 without scrolling for normal actions; Lucide icons only.

Android (the 3.x app's native views, not a second app):

1. Same three tabs. Computers is one column of preview cards in portrait and a
   grid in landscape, own computers and paired friends' computers alike.
2. Tapping a card connects; long-press opens its details (route, last seen,
   remove).
3. Friends and Settings match the desktop's rows and wording.
4. The in-stream Secpar button and panel match the desktop's.

### Measured 2026-11-15: Sunshine does not fit

The Windows Sunshine build is 31.3 MB; without debug symbols 22.3 MB; compressed
with xz -9, 7.1 MB. That alone is over the whole 6 MB Windows budget before the
app itself (4.2 MB in 3.20.6). Of the symbols that could be attributed, OpenSSL is
1.8 MB, Boost 1.6 MB, x264 1.0 MB, FFmpeg 1.0 MB, curl 0.5 MB; 12.9 MB is in
tables and code not attributed to a library.

With the budget at 10 MB, 4.2 MB of app plus 7.1 MB of engine is 11.3 MB, so the
headless build has to lose about 1.5 MB compressed: no curl (the engine never
fetches anything), no web interface code or assets, OpenSSL built with only TLS
1.3 server and the ciphers the protocol uses, Boost.Log replaced by the plain
log sink, and debug symbols stripped. The host therefore ships a trimmed
Sunshine. If the trimmed build still misses the budget, the fallback is Secpar
3's host answering the Moonlight protocol from its own code, with Sunshine's
sources as the reference.

## Order of work

1. Sunshine measured against the budget (above); build the trimmed headless
   engine and measure it again.
2. Compile `moonlight-common-c` into the desktop app, decode and play one stream
   from this PC's engine on the laptop.
3. The same in the Android app.
4. Previews in the host's existing connection for own and friends' computers;
   tiles and cards on both clients.
5. Duplicate merge on the host; Devices list.
6. The UI plan above, desktop and Android.
7. Release builds checked against the size budget; the owner installs from them;
   published to secpar.win after the owner's go-ahead.
