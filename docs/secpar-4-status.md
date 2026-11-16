# Secpar 4: where it stands

Last updated 2026-11-15. Branch `secpar-4`. Read this first, then
`docs/secpar-4-plan.md` for the design and `docs/user-requirements.md` for what
the owner wants. Update this file after each result the owner confirms.

## What Secpar 4 is

Secpar 4 is the approved new app, not an update of the 3.x UI. The desktop app is
the Qt interface in `desktop-qt/` with `crates/secpar-app`, the background
`secpar-service`, the Secpar tunnel and Sunshine/Moonlight. The Android app is
the new client in `android-secpar/` with `crates/secpar-android`; its interface
follows the approved three-tab design. Accounts, pairing, relay and the existing
network services are reused; the app keeps one permanent device identity per
installation and one Moonlight profile per computer.

The 3.x app (`desktop/`, `host/`, `app/`) is the feature reference and the
source of the old repeated-pairing model. Its UI builds are withdrawn: they did
not show the approved interface and the Linux 3.x client created many repeated
x250 paired-device entries. The 3.x `gamestream`/engine work in
`crates/secpar-client`, `crates/secpar-host-core` and `host/src-tauri` is a
reference/alternate path, not the shipping stack.

### Current product stack

| Role | Path |
| --- | --- |
| Desktop UI and connecting role | `desktop-qt/`, `crates/secpar-app` |
| Background hosting role | `crates/secpar-service` |
| Engine data path | `crates/secpar-tunnel`, `crates/secpar-network` |
| Host engine | `crates/secpar-gamestream-host` + Sunshine |
| Connecting engine | pinned Moonlight Qt component (`SecparStream`) |
| Android client | `android-secpar/`, `crates/secpar-android` |

Old x250 entries live in the old 3.x host/client pairing model. The new app does
not create them: the app keeps one device identity and one stream profile per
computer, so the host sees one client per installation. Stale account records
are removed from Settings â†’ Devices; old 3.x paired-device records are separate
and are not part of the new app.

The 3.x `gamestream.info`/`gamestream.pair` client work is retained for the
older path and as a reference. It is not what the Qt or new Android app uses.

## Current test builds (2026-11-15)

`D:\Secpar-CI\s4\publish-secpar4-qt\` holds one package per operating system;
`TESTING.txt` and `SHA256SUMS.txt` sit beside them.

| Package | Size | SHA-256 |
| --- | --- | --- |
| `windows\Secpar-Windows-x64-setup.exe` | 40,419,485 | `666154afe9eaae4af561acd1836d62d4211dff410ddb707a7764ac58cff1b52a` |
| `linux\Secpar-Linux-x64.AppImage` | 125,200,888 | `1b19811193386125a30447c7ce2e92aaabf59ca328b0ec7e6d2ccc741a64d2e4` |
| `linux\Secpar-Linux-x64.AppImage.zsync` | 214,191 | `afc29b4a322bb36a531e7472c98de6615a5d4dd0f8464c04346e9ca06d03f0d8` |
| `android\Secpar-4.0.0.apk` | 13,990,235 | `7b7761c1b4bde2bfb8d2bcb95c35671e47de5f1a0268e391978d2fe16638f83e` |
| (previous) `windows\Secpar-4.0.0-Qt-Windows.zip` | 79,810,958 | `2a554d985d71c4566b3a15a204fc8a6c627a0c4e723fe696eafc4c97b9477d90` |
| (previous) `linux\secpar4-qt-4.0.0-linux-x86_64.tar.gz` | 55,735,167 | `f3fa5fa8be86ead57bba13736bf7b61906a53341e031501c8c09a9c8f7594346` |

- The Windows package contains the Qt window, the background part, the Sunshine
  engine and the Moonlight component, the last with the Secpar in-stream button
  and menu (`SecparStream.exe`, SHA-256 `c9b3d25e05a32ec3ebdba300fc04cb421277e9d752f44bcacae6477b6c1eaa8e`).
- The Linux package was rebuilt from this worktree with Friends, device removal
  and deduplication, the tray, clipboard text sharing and file sending. All four
  binaries carry `$ORIGIN` run paths, and the Qt Quick dialogs module is bundled
  and loads with the system Qt paths masked.
- The Android APK is a release build signed with Secpar's new release key (kept
  outside the repository; see the local handover note). It installs and opens on
  the review emulator; sign-in and streaming from it are still untried.

### Installer, AppImage and in-place updates (built 2026-11-15)

One application per desktop system, each able to update itself; the build steps
are in `docs/packaging.md` and the version comes from `version.txt`.

- Windows: `Secpar-Windows-x64-setup.exe`, NSIS, per-user into
  `%LOCALAPPDATA%\Programs\Secpar` with no elevation. It writes a Start menu
  entry, an Apps & Features entry, a silent uninstaller and an HKCU Run entry
  `secpar.exe --start-host`, which starts `secpar-service.exe` with the bundled
  engine and `%LOCALAPPDATA%\Secpar4` as state, at install time and at each
  sign-in. It installs over an existing copy in place and never writes to
  `%LOCALAPPDATA%\Secpar4` or `%LOCALAPPDATA%\Secpar`; the uninstaller removes
  only program files. Not code-signed.
- Linux: `Secpar-Linux-x64.AppImage` with the package under `usr/lib/secpar`,
  `AppRun` setting `LD_LIBRARY_PATH`, `QT_PLUGIN_PATH` and `QML2_IMPORT_PATH`,
  desktop entry, icon, licences, and the update information
  `gh-releases-zsync|modnite|Secpar|latest|Secpar-Linux-x64.AppImage.zsync`.
  `--install` copies it to `~/.local/share/secpar` with a menu entry and a login
  entry for the background part; `--host` runs the background part.
- Settings â†’ About has one Updates row: version, `Check for updates`, the state
  and one confirmation before installing, because installing ends a stream.
  Captures: `settings-update-800x600.png`, `settings-update-640x400.png`,
  `settings-update-confirm-640x400.png`, and `appimage-settings-about.png` taken
  from inside the built AppImage.

Checked: the installer's file list (`7z l`), and the installer run twice plus its
uninstaller inside Wine on the build box â€” first install with all parts and
registry entries, a second install over it that replaced a changed file in place
and kept `%LOCALAPPDATA%\Secpar4` and one Apps & Features entry, then uninstall
that removed the program files, the shortcut and the registry entries and kept
the data folder. Wine has no PowerShell, so the installer's own step of stopping
the running parts was done by hand there; that step has not run on real Windows.
The AppImage's contents were extracted and listed, and the AppImage itself was
started headless with the build image's own Qt plugins and QML modules moved out
of reach, drawing Settings â†’ Connecting at 640x400 (the value dropdowns with
their chevrons and "Share clipboard text") from its bundled Qt alone:
`appimage-connecting-640x400.png`. Rebuilt after the dropdown and friend
permission work (2026-11-15); `readelf -p .upd_info` still carries the
`gh-releases-zsync` line and the file is executable.

The Linux release is reproducible from the repository since 2026-11-15, and is
now built against Debian 13 so that it runs on a plain system:
`scripts/build-secpar4-linux.sh` builds the window, the app library, the
background part and the package folder, calls `packaging/linux/build-appimage.sh`
and checks the result, in one run inside a `debian:trixie` image
(`secpar-linux-deb`, Qt 6.8.2; `secpar-hk` is no longer used for releases). The
Sunshine engine is built in the same image, so nothing in the package asks for a
newer glibc than Debian 13's 2.41: the window and the stream component ask for
2.34, the background part and the app library for 2.39, the engine for 2.38.
The package carries Qt and the ordinary runtime libraries a bare system lacks
(X11/xcb, xkbcommon, freetype, fontconfig, harfbuzz, libpng16, ICU, glib,
pcre2, libb2, double-conversion, md4c, libproxy, brotli, liburing,
libgssapi_krb5, OpenSSL) and one font family. GL is deliberately not one of
them: hardware rendering needs the desktop's own GLX/EGL so the loader can pair
with the vendor driver, and a bundled GLVND loader made context creation fail on
the laptop (`QRhiGles2: Failed to create temporary context`). A copy of the
GLVND dispatch libraries sits in `gl/` and AppRun adds it to the search path only
where the machine has no GL at all, which is what lets a bare container start in
software. Vulkan, DRM, GBM, VA-API, the audio stack and the Wayland server stay
with the machine. AppImage 125,200,888 bytes,
`1b19811193386125a30447c7ce2e92aaabf59ca328b0ec7e6d2ccc741a64d2e4`, with its
`.zsync` beside it.

The acceptance test passes: in a bare `debian:trixie` with nothing installed, the
AppImage starts under `QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software` and
writes a real window at 640x400 and at 1280x720, exit code 0 for both
(`appimage-debian-trixie-640x400.png`). Software rendering is required there
because the container has no GL; hardware rendering comes from the desktop.

The engine in the package is built **without** the live-resolution patch, because
that patch regressed engine start-up on Windows (access violation in one build,
`NvEnc: encoding format is not supported by the gpu` on `av1_nvenc` in another,
then `Bundled host stopped before it became ready`); the evidence is in
`docs/live-resolution.md`. The stream component is built on Debian too, so its
Qt 6.8, FFmpeg, SDL2 and libplacebo match the rest of the package. Because the
packaging now carries the whole dependency chain of every program, the AppImage
grew to 125,200,888 bytes; the gate that enforces it (`LD_BIND_NOW=1 ldd -r` on
each program, and no dependency outside the package except the machine's list)
is what caught the ngtcp2 mismatch: Debian's `libngtcp2` had been bundled while
the crypto module came from the machine, so the background part died with
`undefined symbol: ngtcp2_conn_get_remote_transport_params2` on CachyOS. In the
Arch build image the packaged engine now loads, reads its assets and reaches
display enumeration without a symbol error.

The graphics stack was the next half-chain: the package carried the build
machine's Mesa EGL/GLX modules, `gbm/` and the KDE Wayland client, and the
engine on the laptop then failed at `Couldn't open EGL display:
EGL_BAD_PARAMETER` and `Unable to find display or encoder during startup`. The
package now carries none of it — Mesa, GLVND, `libgbm`, `libdrm`, `libvulkan`,
`libva`, the Wayland client libraries (`libwayland-client`,
`libwayland-cursor`, `libwayland-egl`, `libwayland-server`), the `gbm/` and
`dri/` folders, `libKWaylandClient` and the Qt eglfs/kms and KDE decoration
plugins — and the packaging fails if any of those names is present. The bundled
`libwayland-client` was the last one found: with Mesa out but it still inside,
EGL failed the same way; removing it opened EGL and let the engine build an
encoder on the laptop. Qt's own Wayland modules stay, using the machine's
`libwayland` at run time. `gl/` keeps the four GLVND dispatch libraries for machines with no GL at
all; the window itself draws in software.

Started from the compressed mount, the engine stopped right after its KMS
monitor list, while the same binary from an extracted folder went on to open EGL
and create an encoder. AppRun therefore extracts the payload once per version
into `~/.cache/secpar/app-4.0.0` and runs everything, the background part
included, from that writable copy; later starts reuse it and other versions are
deleted. `--install` still runs from the mounted file.

Because GL context creation against vendor drivers kept failing with a bundled
Qt (the laptop showed `QRhiGles2: Failed to create temporary context`), AppRun
now sets `QT_QUICK_BACKEND=software` itself: the window always draws, which
costs only the window's own drawing since the stream component does the video.
`SECPAR_RENDERER=gl`, or an explicit `QT_QUICK_BACKEND`, asks for hardware
rendering. The Computers list also stays empty with "Starting Secpar on this
computer" until the background part answers, so a computer no longer lists
itself while the service starts.

A launched window now starts the packaged background part itself when nothing
answers on `$XDG_RUNTIME_DIR/secpar-service.sock`, with
`--state $HOME/.local/share/Secpar4` and the package's own engine;
`packaging/linux/start-hosting.sh` was changed to that same state folder so one
computer keeps one identity whichever starts it. In a bare container the service
exits with `device identity: StoreUnavailable` (no credential store), so its
listening socket exists only on a machine with KWallet or gnome-keyring. The
window also takes its interface font from `~/.config/kdeglobals`, and the
bundled DejaVu Sans is used only where the machine has no font configuration of
its own. The window uses `QApplication` and links Qt Widgets, because Qt 6.8's
Qt.labs.platform needs that for the tray icon and its menu; on Qt 6.12 a
`QGuiApplication` was enough. The Windows package predates that change and needs
a rebuild before it is published again.

Still to check on the laptop: hardware GL with the owner's Mesa driver, the KDE
font, and the background part staying up under KWallet.

Checked without the network: `cargo test --test updates` against a feed served
on 127.0.0.1 with the real `Secpar-Windows-x64-setup.exe` and the real
`Secpar-Linux-x64.AppImage` as the asset â€” discovery, version comparison, the
download, the SHA-256 check, refusal of a wrong digest and of a wrong size, and
(on Linux) the atomic replacement of the running AppImage with its executable
bit kept. On Windows the replacement was proved by the Wine run above; the step
that starts the installer from the window was not executed on this PC, because
that installs Secpar.

Not checked: the real GitHub feed and a real install or update on Windows and
Linux, code signing, and AppImageUpdate's own delta path.

### Clipboard text sharing (built 2026-11-15)

The tunnel's app channel (reserved stream port 2, u32 length framing, 1 MiB each
way) carries `{"app":{"clipboard":{"get"|"set":â€¦}}}` to the hosting computer.
`crates/secpar-hosting` answers only for `Peer::Own` or a guest with `keyboard`,
and `crates/secpar-service/src/clipboard.rs` follows the OS clipboard in both
roles about twice a second, only while a computer is connected, never sharing text
copied before the connection. The window's Settings â†’ Connecting row turns it off.
Proven by tests over a real loopback tunnel with stand-in clipboards; the real OS
clipboard on two machines is still untested.

### File sending (built 2026-11-15)

`{"app":{"file":{"name","size","offset","data"}}}` on the same app channel, 256 KiB
of file per frame, 2 GiB cap, answered only for `Peer::Own`. `crates/secpar-service/src/files.rs`
writes into the downloads folder under `Secpar`, keeps a `.part` name until the
file is complete, and numbers a repeated name instead of overwriting. The window
shows "Send a fileâ€¦" on the connected computer's card and "Received <name> â€”
Downloads/Secpar" for a file that arrived. Tests: a 700 KB file arrives
byte-identical over a real loopback tunnel, a second file of the same name becomes
`report (2).bin`, traversal and reserved names and oversized pieces are refused,
and a friend is refused. Not tested between two real computers or on Linux.

### Screen previews (built 2026-11-15)

The still of a computer's screen that its tile shows in the list. On the app
channel the request is `{"app":{"preview":{"max_width":480}}}` and the answer is
`{"ok":true,"jpeg":"<base64>","width":480,"height":270,"at":1699999999}`, or
`{"ok":false,"err":"not_available"}` when that computer has nothing to show.
Only the owner's own devices are given a still, like files; a friend's device is
answered `not_allowed`.

The hosting side captures the primary display with GDI, which needs no
duplication session of its own and so does not take the output from the engine
while a stream runs, scales it, writes a JPEG at quality 65 and keeps it for
five seconds; the same bytes are answered again within that window. A second
capture at a lower quality keeps a picture under 320 KB, and an answer over
512 KB of base64 is refused rather than sent, so it stays well inside the
channel's 1 MiB frame. Widths are kept between 16 and 1024. Linux has no capture
path in the background part yet and answers `not_available` there.

The window asks its own background part
`{"do":"preview","host":"<certificate fingerprint>","addresses":[...],"max_width":480}`
and the answer comes back unchanged; `crates/secpar-app` forwards it. A still is
for any of the owner's computers the account lists as on, whether or not this
window is connected to it. If that computer already has the window's tunnel, the
request goes over it and no second tunnel is opened. Otherwise the background
part peeks: it opens a tunnel to that computer for this one question and drops
it. The peek carries no engine ports and keeps no local socket, so a stream on
another computer keeps its own tunnel and addresses. One peek runs at a time and
a computer is not peeked twice within the five seconds its host keeps a still, so
a page of tiles queues instead of opening a tunnel per tile at once. A computer
the account lists as offline carries no address, and a request without one is
answered `not_available` without any tunnel being opened. The line limit between
window and background part went from 16 KB to 1 MB so a still fits in one line.

The tile draws the still where the monitor icon was and keeps the icon when there
is none; computer, tile and window sizes are unchanged. The window asks every
online computer at most every 15 seconds, one computer after the other, and only
while the Computers page is on screen. A landscape sample still
(`desktop-qt/sample/preview.jpg`, drawn for the sample, shipped only for
`--sample`) appears in one sample tile.

Proved on this PC: `crates/secpar-hosting` tests with a stand-in screen show that
the owner's device gets the still, a friend's request takes no picture, widths
are clamped to 1024 and 16, and three requests at one width capture once.
`crates/secpar-service/tests/preview.rs` runs two real hosting computers with the
real capture, a real background part and real loopback tunnels: a still comes
from the connected computer over its tunnel, a still of 480 pixels comes from a
computer that is not connected at all, that peek leaves the open tunnel working
(afterwards the connected computer still answers and the background part still
reports it as connected), a friend is refused, an oversized width comes back at
1024, the same computer is not asked twice within the window, and an offline
computer is never asked. The built window was then run against two stand-in
hosting computers on the local test account with nothing connected: both tiles
drew a still of this PC's screen, each from its own peek. Capture:
`computers-preview-640x400.png`, which uses the sample still so no real screen is
published.

Not done: Linux (no capture path in the background part, so it answers
`not_available`), Android, and two real computers. A computer that is on but
behind another network is peeked at the addresses the account has for it, the
same direct path Connect uses; there is no knock or relay step for a peek.

### Live resolution (built 2026-11-15)

Changing the size of a running stream. Apollo, the pinned reference, only takes a
stream size from the client's RTSP `ANNOUNCE`, so a size change there means a new
session with the display configured for it; it has no mid-stream path. Our
mechanism is described in `docs/live-resolution.md`:
`integrations/gamestream/patches/sunshine/0003-secpar-live-resolution.patch` adds
`POST /api/secpar/resolution` to the engine's authenticated management port
(loopback only). The capture thread builds its encoder again at the requested
size, keeps capturing the same display and lets the new sequence header and
keyframe reconfigure the client. The tunnel's app channel carries
`{"app":{"resolution":{"width","height"}}}`, answered only for the owner's own
devices, and `secpar-service` takes a `{"do":"resolution"}` line from the window.
Proved on this PC with the patched engine on its own ports and state and the
Secpar Moonlight component: a 1280x720 HEVC stream of the real display changed to
1024x768 and back inside one session, the engine log naming both changes and the
encoder restart, and the client's picture going 4:3 (pillarboxed, 98% of the left
edge black) and back while the client kept running. The Linux engine is built
from the same patches on the build box and its endpoint answers there (see
`docs/live-resolution.md`), but no frame has been encoded on Linux yet. Not done:
a real Linux session on the laptop, the Android client's reaction, and the
in-stream menu control, which still shows the resolution as read-only.

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
  and mute on focus loss. Settings â†’ Hosting shows the real host state, allowed
  device count and access-list expiry. The window was run signed in against a
  local background part; the settings API round-trip and validation are tested.
  Captures: `settings-connecting-640x400.png` and `settings-hosting-640x400.png`.
- Settings choices and friend permissions (2026-11-15): every Settings row whose
  value is a list opens a dropdown instead of changing on click. The menu is the
  app's own flat popup (dark surface, 1 px border, 7 px radius, 32 px rows, a mint
  check on the current value), anchored to the value and turned upwards when the
  page has no room below. Space or a click opens it, arrows move, Enter or Space
  picks, Escape closes. Proved by driving the built window with real input: a
  click on the Resolution value opened the list and a click on 2560 Ã— 1440 set
  the row to it; Enter opened the same list on the focused row, Down and Enter
  picked the same value; Space opened it, and Escape closed it and left the value
  alone.
  Friends rows now expand into "Allow to connect" (Ask each time / Always) and
  "Control" (View only / Keyboard and mouse), saved through
  `GET`/`PUT /v1/friends/<id>/settings`, and the Add friend row sends a request by
  account name. `FriendChange` refuses values the service would refuse, and the
  page shows the service's own answer. Run against `cloud/accounts/test/local-server.mjs`:
  `tests/friend_permissions.rs` drives the window's JSON and checks the owner's
  list and the friend's separate defaults; the built window signed in to that local
  service changed a friend from Always to Ask each time and showed the saved value.
  Captures: `settings-dropdown-640x400.png`, `settings-dropdown-800x600.png`,
  `friends-permissions-640x400.png`, `friends-permissions-800x600.png`,
  `friends-640x400.png`. Not run: a permission change against the live account
  service, two real people, Linux.
- Clipboard text sharing (2026-11-15): the tunnel has a second app channel
  (stream port 2, a four byte length in front of each payload, 1 MB each way) and
  `{"app": {"clipboard": {"get": true}}}` / `{"app": {"clipboard": {"set": "â€¦"}}}`
  are answered only for the owner's own devices or a friend whose keyboard
  permission is on; text over the crate's 1 MB limit is refused. The background
  part keeps the clipboard in step about twice a second, and only while a
  computer is connected: nothing is read or written when none is, and text that
  was already in the clipboard is never shared. Settings â†’ Connecting has
  "Share clipboard text" (on by default). Proved with the real hosting side and
  the background part's clipboard loops over a real loopback tunnel, with
  stand-in clipboards: text travels both ways, a guest without the keyboard
  permission is refused, oversized text is refused, and an oversized tunnel frame
  is dropped unread. Captures:
  `settings-connecting-640x400.png`, `settings-connecting-800x600.png`. Not run:
  two real computers, the real OS clipboard in a live session, Linux.
- Sending a file (2026-11-15): one way, from the window to the computer it is
  connected to. On the same app channel,
  `{"app": {"file": {"name": "report.bin", "size": 700000, "offset": 0, "data":
  "<base64>"}}}`, each frame at most 512 KB of base64 (256 KiB of the file), the
  host answering each one `{"ok": true, "received": N}` and naming the saved file
  on the last. Only the owner's own devices are given files: a friend's device
  gets `not_allowed`, keyboard permission or not. Names with a path separator, a
  control character, a Windows-reserved character, a trailing dot or no name at
  all are refused, as is anything over 2 GiB, an empty file, a piece out of order
  and a piece past the announced size. Files go into the person's real downloads
  folder under `Secpar`, written under a temporary name until the last piece has
  arrived; a name already there becomes "name (2).ext". The window has
  "Send a fileâ€¦" on the connected computer's tile with one line beside it
  ("Sending report.pdf 45%", then "Sent report.pdf"), and the background part
  reports the last file it saved for the one-line "Received name â€” folder" note
  along the bottom edge. Proved with a test over a real loopback tunnel and the
  real writer: a 700 KB file takes three frames, arrives byte-identical under its
  own name and as "report (2).bin" the second time, a friend's device and seven
  bad names are refused, and the writer's own tests show a partial file never has
  the final name. Also run by hand: the built window against a stand-in hosting
  computer, a 700 KB file sent through the real channel and tunnel, arriving
  byte-identical in `Downloads\Secpar`. Captures: `computers-sending-640x400.png`,
  `computers-sent-640x400.png`, `computers-send-800x600.png`. Not run: two real
  computers, the "Received" note seen on screen, Linux.
- Android app, new interface (2026-11-15): `win.secpar.app.SecparActivity` is now
  the approved three tabs (Computers, Friends, Settings) with the wordmark,
  version, one column of computer cards in portrait and two in a docked window.
  Friends is an honest empty state. Rendered on the review emulator at 411x914
  and 1024x700 dp; the signed-in pages used sample device names, not account
  data. Debug APK builds as `app-nonRoot-debug.apk` with `versionName 4.0.0`;
  `testNonRootDebugUnitTest` passes 5 layout checks. Not run: account sign-in,
  Connect and a stream from this interface on the phone.
- Android Friends and permissions (2026-11-15): the phone app's Friends tab is now
  the account's own list, through five new requests on the existing `Core` JSON
  (`friends`, `add_friend`, `accept_friend`, `remove_friend`, `friend_settings`).
  Incoming requests take Accept or Decline, a sent request can be cancelled, and
  an accepted friend opens "Allow to connect" (Ask each time / Always) and
  "Control" (View only / Keyboard and mouse), saved through the account service
  and re-read afterwards. One column in portrait, two in a docked window, honest
  empty and error states. The release APK is signed with Secpar's key and now
  carries the version code from `version.txt` (4.0.0 -> 40000) instead of
  Moonlight's 315, so a later release can install over it. Rendered on the review
  emulator only: the real account list, saving a permission against
  `accounts.secpar.win` and any phone install are untested.

## Built but not confirmed

- Controllers in the Android app.- The owner's work network. If it blocks UDP, a relay key is needed (see below).
- Hosting on Linux through `crates/secpar-hosting`; only container runs so far.
- The relay path. It is written on both sides and unused, because the relay
  service has no TURN key.

## What the old app has and this one still lacks

The owner's complaint of 2026-11-15 was right: the new app is usable but is not
yet the finished product the old one was. These are the differences, and what is
being done about each.

| Old app | New app now | Being fixed |
| --- | --- | --- |
| `Secpar-Windows-x64-setup.exe`, installs and updates in place | done: NSIS per-user installer, same asset name, in-app update with SHA-256 check and in-place reinstall | code signing still missing |
| `Secpar-Linux-x64.AppImage`, zsync update | done: AppImage with zsync update information, self-replace, same asset name | not yet run on the laptop; AppImageUpdate's delta path untested |
| Friend permissions (connect: ask/always, control: view/full, keyboard, mouse) | done: inline permission editor with dropdowns, add friend, cancel request | a running host acting on a changed permission is untested |
| Dropdown lists in settings | done: flat dropdowns with keyboard support | none |
| In-stream menu with quality, players, files, disconnect | in-stream menu without files; resolution is read-only | live resolution change is the next engine task; the 3.x hosts do it today (Windows: `engine/session.rs` resize; Linux: `crates/secpar-host-linux/src/capture.rs` caps renegotiation) |
| Preview of the remote screen, monitor switching, wake-on-LAN, virtual display | a still in every online computer's tile (2026-11-15); monitor switching, wake-on-LAN and a virtual display are not built | monitor switching, then wake-on-LAN |

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
8. Friends and guest access through the new access list: the Friends screen now
   lists friends and requests and edits what each accepted friend may do; a guest
   joining a hosted stream, and a permission change being felt by a host that is
   already running, are still untried on real machines.
9. Installers and Linux packaging are built (see above); what is left is the
   owner's install and update on real Windows and Linux, and code signing.

## Working rules that came from the owner

- Work lean. Moonlight and Sunshine are known to work; do not build test rigs
  or proof rounds for each step. One quick check, then the owner's devices are
  the test.
- Reuse what Secpar 3 already runs (relay service, account service, keys).
  Secpar 4 is an update, not a separate system.
- Never say something works without the owner's result or a real check. Say
  plainly what was not run.
- Usage is tight. No long write-ups, no speculative work.

Laptop run, 2026-11-15 evening (CachyOS, KDE Wayland, 1366x768):

- The 125 MB AppImage starts with its own defaults (software window); the window process stays up.
- The window starts the packaged background part itself, with the right paths
  (`--state ~/.local/share/Secpar4 --channel /run/user/1000/secpar-service.sock
  --engine <mount>/usr/lib/secpar/engine/sunshine --tunnel-port 46900`).
- The engine now loads on CachyOS: the `libngtcp2_crypto_ossl` symbol error is
  gone after the bundle scan included the engine's own dependency closure. It
  reads the Wayland/KMS monitor list (eDP-1, Built-in Screen, 1366x768).
- It then stops after the monitor list: no encoder line, no "ready", so the
  service exits and `/run/user/1000/secpar-service.sock` never appears, and the
  window cannot add this computer. The KMS line "Mismatch on expected Resolution
  compared to actual resolution: 0x0 vs 1366x768" is the last thing said.
- Next step: run the packaged service from an extracted copy (so its stdout and
  the engine logs are writable), capture the stop, and decide between the KWin
  capture path and the portal for KDE Wayland at 1366x768. This is the last
  blocker to Linux hosting from inside the package.
## Handover for Claude

`docs/handover-claude.md` records where every part of this work lives (repository,
release folder, keys, build caches, staging folders, build machines, images, test
devices), the verified build commands for all three platforms, the proven and
unproven list, the open defects with their evidence, and the owner's rules. Start
there when continuing, together with `docs/secpar-4-status.md`.