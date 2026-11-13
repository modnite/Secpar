# Secpar 4: rebuild plan

Status: draft for the owner's approval, 2026-11-13. Nothing here is built yet.
This replaces the 3.x design. Where it conflicts with older documents, this wins.

## 1. What Secpar is

A Parsec-like app for playing with friends on one PC, for anyone who downloads it.
One desktop app hosts and connects. Android connects only and offers everything the
desktop client offers. The name is Parsec reversed, on purpose. The mint/orange
triangle and wordmark are fixed.

Platforms: Windows 10/11, Linux, Android. macOS and iOS are out of scope.
Licence: GPL-3.0 for all app code, permanently.

## 2. Decisions from the owner's answers

| Area | Decision |
| --- | --- |
| Engines | Sunshine as the host core and Moonlight as the client core, embedded as they are so upstream fixes can be pulled. Apollo's virtual display and per-client permission code is ported onto Sunshine as a separate, small patch set. Decided 2026-11-13 after Apollo was found to be far less actively maintained. |
| FFmpeg | Our own minimal build: only the encoders, decoders and filters streaming needs. |
| Interop | Secpar to Secpar only. No stock Sunshine or Moonlight peers. |
| Android | Shared client core with desktop, native Android shell. |
| 3.x installs | Clean break. New install, new sign-in, new pairing. No 3.x data is read. |
| Hosting | Windows and Linux in the first release. |
| Encoders | NVIDIA, AMD, Intel hardware encoders and a software fallback. |
| Quality | Whatever the engines support, offered per client from detected hardware. |
| Trade-off | Low latency by default, with a per-connection option to prefer quality. |
| Players | Three remote clients on one host (four players), each with their own controller. |
| Connection | Keep the 3.x scheme: LAN first, then direct, then relay. |
| Features | Sound, two-way clipboard, two-way file browsing and transfer, monitor choice and switching, virtual display, wake-on-LAN. Microphone later. |
| Input | Controllers with rumble, gyro and touchpad, full keyboard capture. Android on-screen gamepad later; basic touch pointer and keyboard are needed to use the app at all. |
| Accounts | Kept and redesigned: one permanent identity per device, account service is the single source of truth. |
| Sign-in | In the app, never a browser portal. Email and password. Passwordless and two-factor are optional. Cloudflare, accounts.secpar.win. |
| Friends | Trusted friends join without a prompt using saved permissions; others need approval. |
| Unattended | Reachable before login on Windows and Linux. Required. |
| Look | Closer to Parsec, with Steam and Moonlight as further references. Identical custom look on all three platforms. Usable at 640x400. Fresh mock-ups welcome before code. |
| Main layout | Top tabs (Computers, Friends, Settings) with large preview tiles; own and friends' computers in one grid. Chosen 2026-11-13 over a sidebar. At 640x400 tiles go two per row. |
| In stream | Floating Secpar button plus a configurable one-hand key chord. |
| Background | System service plus tray icon. |
| Linux packages | Arch package, deb, rpm, AppImage. |
| Updates | GitHub releases and secpar.win, in-app prompt; Linux packages through the package manager. |
| Testing | The owner tests public releases. No laptop access for the developer. |
| Delivery | Everything in one release. |
| Carried over | Connection scheme, computer previews, website and domains, Android audio recovery behaviour. |

Left to the developer: server trust model, language and toolkit. Both are decided below.

## 3. Architecture

### 3.1 Parts

1. **Host engine**: Sunshine, built as a library or a supervised child process with
   its web UI and its own pairing switched off. Secpar feeds it the list of allowed
   client certificates and its settings.
2. **Client engine**: moonlight-common-c for the protocol. Desktop uses Moonlight-qt's
   decoders and renderers; Android uses Moonlight Android's MediaCodec decoder and
   controller code.
3. **Secpar core**: one library used by the desktop app, the host service and the
   Android app. It owns identity, accounts, friends, the connection scheme, files,
   clipboard and previews. Rust, because the working 3.x connection, relay and file
   code is Rust and can be carried over after review.
4. **Desktop app**: C++ with Qt 6 and Qt Quick. This is what Moonlight-qt uses, so its
   video, audio and input code is reused instead of rewritten, which is where 3.x
   went wrong. One Qt Quick interface gives the identical look on Windows and Linux.
5. **Android app**: Kotlin shell around the same Secpar core and client engine. The
   interface follows the same design and the same screens as desktop.
6. **Host service**: a system service on both systems (Windows service; systemd
   system unit on Linux) that runs the host engine and Secpar core from boot.
7. **Account service**: rebuilt on Cloudflare at accounts.secpar.win.
8. **Relay**: as in 3.x.

### 3.2 Carrying the engines over our connection

Sunshine and Moonlight expect to reach each other on fixed TCP and UDP ports. The
3.x connection scheme gives us one authenticated, encrypted path between two devices
(LAN, direct or relayed). Secpar core will present that path to both engines as a
local tunnel, so the engines run unmodified and never see the internet directly.
This also enforces "Secpar to Secpar only": the engine ports are never open on the
network.

### 3.3 Identity and access (replacing 3.x pairing)

- Each installation creates one key pair at first run and keeps it for good. The
  account service knows the device by that key. Reinstalling or signing in again
  does not create a second device unless the key is really gone, and then the old
  record can be deleted from the account.
- No per-host credential lists. A host asks the account service which device keys
  may connect: the owner's devices, and trusted friends' devices with their
  permissions. The host caches a signed copy so LAN play works when the service is
  unreachable.
- Removing a device or a friend in the account removes access everywhere.
- A device never appears in its own computer list; it is matched by its key.
- Secrets live in the system credential store, with a failure to read treated as an
  error shown to the user, never as "no credential".

### 3.4 Servers and what they see

The owner runs the account service and relay. Video, sound, input, clipboard and
files are encrypted between the two devices, so the relay carries data it cannot
read. The servers see account email, device names, friend links, and which device
connects to which and when. Self-hosting is not a goal for the first release, but
server addresses will be a build setting, not scattered constants.

## 4. Risks found while checking upstream

These were checked on 2026-11-13 and change what "engine as-is" can deliver.

1. **Three clients.** Current Sunshine lets several clients join, and they share one
   session: same screen, shared control. That fits four players on one host. Each
   client having its own controller slot must be confirmed in a test build before
   the design depends on it. Independent desktops per client are not possible on
   consumer GPUs and are not planned.
2. **Virtual display.** Stock Sunshine has none. The Apollo fork (GPL-3.0, tracks
   Sunshine) adds one on Windows through the SudoVDA driver and adds per-client
   permissions; Linux support there is only planned. Options: base the host on
   Apollo instead of Sunshine, or add a virtual display ourselves. Either way the
   Windows driver must be signed and its licence confirmed. Linux needs a separate
   approach (KDE and wlroots can create virtual outputs; GNOME differs).
3. **Linux login screen.** Sunshine captures the login screen only through KMS
   capture with elevated permission, and it normally runs as a user service, so
   nothing is running before login. A system service with KMS capture is feasible
   but depends on the display manager, and Sunshine's documentation says KMS capture
   is to be phased out in favour of the desktop portal, which cannot see the login
   screen. This requirement carries the highest risk of the whole plan.
4. **Qt on a small window.** A Parsec-style layout at 640x400 needs a collapsing
   sidebar. This will be settled in mock-ups first.
5. **One release, tested only after publishing.** Every feature will first be
   exercised by the owner on real hardware after it is public. Expect several fix
   releases after 4.0.0. Version 4.0.0 will be marked as a preview on the website
   until the owner confirms each area.

6. **Patches on Sunshine.** Read from GitHub on 2026-11-13: Apollo's last release is
   v0.4.6 (2025-07) and its last commit is from 2026-05, while Sunshine released in
   2026-09 and is active daily. Sunshine is therefore the base, pinned at
   v2026.914.233613. The virtual display and per-client permissions come from Apollo
   (GPL-3.0) and must be carried as our own patches, kept few and separate so Sunshine
   updates still apply. The SudoVDA driver states MIT or CC0 for its author's changes
   in its README and has no licence file; shipping it needs a signed driver package.

### Findings from reading the pinned Sunshine source (2026-11-13)

These come from the code, not from a running build, so they are not yet confirmed
on hardware.

- **Controllers per client (risk 1).** Each client session has its own controller
  table, and host gamepads are handed out from one shared pool of 16
  (`src/input.cpp`, `alloc_gamepad`). A second client joins the running session
  through the resume path (`src/nvhttp.cpp`). So three clients with separate
  controllers is how the engine is written; no patch is expected.
- **KDE capture (risk 3, partly).** Sunshine now has a KDE backend
  (`src/platform/linux/kwingrab.cpp`) that asks KWin for a screen stream directly,
  without the portal's consent dialog. KMS capture is still present for the login
  screen. The KWin protocol it uses can also create virtual outputs, which the
  backend does not use yet; that is the likely route to a virtual display on KDE.
- **FFmpeg.** Sunshine links static FFmpeg libraries from a directory given by
  `FFMPEG_PREPARED_BINARIES`, by default a prebuilt encoder-only set from LizardByte.
  Our own minimal build can be supplied through that setting without changing
  Sunshine. Encoders it can use: NVENC, AMF, Quick Sync, VA-API, Vulkan, Media
  Foundation, x264, x265 and SVT-AV1, for H.264, HEVC and AV1.

### Windows host build (2026-11-13)

The pinned Sunshine builds unmodified on this PC with MSYS2 UCRT64 (GCC 16.2) using
`scripts/build-sunshine-windows.sh`. Result: `sunshine.exe`, about 69 MB before
stripping, with Sunshine's prebuilt encoder-only FFmpeg. Its tray icon and WiX
installer are switched off through its own build options.

Startup check, same day: run from the build directory with its own config
(`port = 46989`, Steam audio driver install off, display changes off) next to the
installed Sunshine on its default ports. It needs `C:\msys64\ucrt64\bin` on PATH
and `src_assets/windows/assets/shaders` copied into `build/assets`. It found the
H.264 and HEVC NVENC encoders on the RTX 2060 (no AV1 on that card), listened on
46984, 46989, 46990 and 47010, answered its local status request, and the installed
Sunshine kept answering on 47989 throughout. No client has connected to it, so
streaming, three clients and controllers are still unconfirmed.

### Minimal FFmpeg for the Windows host (2026-11-13)

`scripts/build-ffmpeg-windows.sh` builds our own FFmpeg from the sources Sunshine
pins (FFmpeg, AMF headers, x264, with Sunshine's two cbs patches). It contains
libavcodec, libavutil, libswscale and the cbs library, and only these encoders:
AMD (AMF), Intel (Quick Sync) and Media Foundation for H.264, HEVC and AV1, and
x264 as the software fallback. NVIDIA is not in it because Sunshine drives NVENC
directly on Windows. Software HEVC and AV1 (x265, SVT-AV1) are left out: they are
too slow to be a useful fallback and were most of the size.

Result: about 9 MB of libraries against 55 MB for Sunshine's prebuilt set, and
`sunshine.exe` drops from about 69 MB to about 31 MB. Sunshine itself is unchanged;
the libraries are passed in through `FFMPEG_PREPARED_BINARIES`.

Checked on this PC: the host starts with it, finds NVENC H.264 and HEVC, and when
forced to the software encoder finds x264 through our FFmpeg. Not checked: AMD and
Intel encoders (no such hardware here) and Media Foundation, and no stream has
been encoded for a client yet.

### Linux host build (2026-11-13)

The pinned Sunshine builds unmodified on Arch Linux with
`scripts/build-sunshine-linux.sh`, run on the OMV build box in a throwaway
container from the existing build image. Result: `sunshine`, about 42 MB, with
Sunshine's prebuilt FFmpeg. Compiled in: KMS, KWin, portal, wlroots and X11
capture, and VA-API. Left out for now: the tray icon and CUDA, so NVIDIA capture
and direct NVENC on Linux are not in this build.

One workaround: the Boost 1.89 that Sunshine downloads has an IDE file-grouping
line that current CMake rejects; the script deletes that line from the downloaded
copy. The program has not been run. The build box has no desktop, so capture, the
login screen and streaming on Linux are all still unconfirmed. A minimal FFmpeg
for Linux is not built yet.

Run in a headless KDE Wayland session (throwaway container on the build box, Intel
UHD 730, same day): the host starts, captures the virtual screen through the
desktop portal by default, and with `capture = kwin` captures through KWin
directly. It finds VA-API H.264 and HEVC; that GPU has no AV1 encoder. Limits of
this check: KWin's permission checks are switched off in that session, so the
desktop-file permission KWin capture needs on a real desktop is untested; the
portal dialog was pressed by a script; KMS capture and virtual gamepads are not
available in a container; nothing connected to it.

The owner's laptop, read over SSH: CachyOS, KWin 6.8.2, SDDM with automatic login
into Plasma, Intel HD 5500 (Broadwell) whose driver offers H.264 encoding only.
So on that laptop the pre-login case is really the locked session after automatic
login, and HEVC hosting from it is not possible in hardware.

Run on the owner's laptop in the real Plasma session (30 seconds, from a folder in
the home directory, no administrator rights): KWin capture works with KWin's
permission checks on. Sunshine gets the permission by writing a temporary desktop
file into `~/.local/share/applications` and waiting three seconds; Secpar should
install its own desktop file with that permission instead. It found the laptop
screen (1366x768), VA-API H.264 as the only hardware encoder, and reported no
problem creating virtual controllers. Not checked there: capture while the
session is locked, the portal route, and any stream to a client. The laptop lacks
OpenSSL 4 and the UPnP library version this build links, so Linux packages must
bundle or pin those.

### Groundwork already in the repository (found 2026-11-13)

An earlier 3.x migration attempt left work against the same pins that Secpar 4
builds on instead of rewriting. See `integrations/gamestream/README.md`.

- `crates/secpar-gamestream-host`: runs a private Sunshine as a supervised child
  with its own state folder and port range, ties the process to its parent, talks
  to its local management port pinned to its certificate, and keeps a durable
  record of which client certificate belongs to which Secpar device, with
  revocation.
- `integrations/gamestream/patches/sunshine`: two patches, 221 lines together.
  One binds pairing approval to a client certificate; one reads the management
  password from a pipe instead of the command line. Both build scripts now apply
  them. These count toward the "few and separate" patch set in risk 6.
- `integrations/gamestream/patches/moonlight-qt`: two patches giving the client a
  separate Secpar settings identity and a pairing mode that exits instead of
  showing a dialog.
- A container test that paired and streamed generated video and sound between
  Sunshine and the patched Moonlight client (software, 640x360), and an Android
  harness that compiles Moonlight's native core.

Checked today on Windows: with both patches the engine builds, and the supervisor
starts it on separate ports, reads its version, refuses a second start on the
same state, stops it, and restarts it with the same certificate. The start
allowance was raised from 15 to 60 seconds because the engine took about 30
seconds to open its management port on this PC. Pairing through the supervisor
has not been run on Windows yet.

Two things to handle in the Secpar host: Sunshine always announces itself on the
LAN by mDNS under the computer's name (no setting turns it off), and its data
folder is always `config` beside the executable.

Pinned sources are in `upstream/pins.json`; `scripts/fetch-upstream.sh` fetches them
and `--check` confirms each pinned commit exists upstream.

## 5. Order of work

1. Mock-ups of the main screens (computers, friends, settings, in-stream menu) at
   normal size and 640x400, for the owner to choose from.
2. New repository layout, pinned upstream sources, minimal FFmpeg build scripts,
   licence records.
3. Test builds answering risks 1 to 3. Results reported before more is built on them.
4. Secpar core: identity, account client, connection scheme, tunnel for the engines.
5. Account service rebuild: sign-up, sign-in, optional passwordless and two-factor,
   devices, friends, trust and permissions.
6. Host service on Windows and Linux.
7. Desktop app: interface, streaming, in-stream button and chord, tray.
8. Files, clipboard, previews, monitor switching, wake-on-LAN, virtual display.
9. Android app.
10. Packaging (installer, Arch, deb, rpm, AppImage, APK), updater, website.
11. Release 4.0.0.

## 6. Questions still open

- If the Linux login screen cannot be made reliable on a given display manager, is
  "reachable after automatic login to a locked session" an acceptable fallback there?
- Is the old 3.x account database kept (same emails and passwords) or do accounts
  also start fresh?

## Sources

- https://docs.lizardbyte.dev/projects/sunshine/v0.23.0/about/advanced_usage.html
- https://redlib.hbubli.cc/r/LizardByte/comments/1jf2lqu/multiple_clients_connecting_to_windows_sunshine
- https://github.com/orgs/LizardByte/discussions/262
- https://github.com/ClassicOldSong/Apollo
- https://github.com/ClassicOldSong/Apollo/issues/874
- https://github.com/LizardByte/Sunshine/issues/1320
- https://docs.lizardbyte.dev/projects/sunshine/latest/md_docs_2troubleshooting.html
