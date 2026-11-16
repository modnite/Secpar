# Secpar 4: rebuild plan

Status: approved by the owner and in progress since 2026-11-13. What is built and
checked so far is recorded in sections 3 and 4.
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

Design, decided and first built 2026-11-13 (`crates/secpar-tunnel`):

- The tunnel is one QUIC connection (the `quinn` library) between two devices.
  QUIC streams carry the engines' TCP connections and QUIC datagrams carry their
  UDP packets, each tagged with the engine port it belongs to.
- Both ends prove a device identity with a certificate. The connecting side
  accepts only the host certificate it expects; the host accepts only
  certificates on its allow-list. No certificate authority is involved.
- The engine is configured with `bind_address = 127.0.0.1`, so it listens only on
  the computer itself. On the connecting side the tunnel listens on another
  loopback address (`127.0.0.2`) with the same port numbers, and the stream
  component connects there as if it were the host. A computer can therefore host
  and connect at the same time without port clashes.
- The video packet size is set to 1024 bytes so a packet plus tunnel overhead
  fits in one network packet.

Checked in the KDE container: with the engine bound to loopback it did not answer
on the container's network address; through the tunnel, pairing and a 16 second
720p stream ran at 60.1 fps received, 59.6 rendered, no loss, with sound. Unit
tests cover delivery, an unlisted device, a wrong host certificate and a port the
host does not offer.

Added the same day, with unit tests: the tunnel can run on a socket the caller
supplies, try several addresses for one host in order and use the first that
answers, send "punch" packets from its own port to a device's addresses, and
close a device's open tunnel the moment it leaves the allow-list.

How this maps onto the 3.x scheme (`docs/remote-protocol.md`), which stays:

| 3.x piece | In Secpar 4 |
| --- | --- |
| Control messages over LAN or the rendezvous Worker | Kept for signalling: exchanging addresses and the allow-list |
| One UDP stream socket per side, addresses found by STUN and UPnP | The tunnel's socket, passed to `serve_on` / `connect_on` |
| Host sends probes to the device's addresses | `Host::punch` |
| Device tries every host address | `connect_on` with the address list, LAN first |
| TURN relay when punching fails | Not built yet: needs a socket wrapper that sends through `secpar-network`'s TURN client |

Still to do for the tunnel: the relay wrapper; wiring to the signalling channel;
behaviour on lossy or low-MTU paths, where QUIC's congestion control and packet
size limits may need tuning for video; measurements at higher bitrates and over
a real internet path.

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

Built 2026-11-13 (`crates/secpar-core`, identity only): the device identity is a
certificate and key in the system credential store, used directly as the tunnel
identity. A store that cannot be read is an error and never produces a new
identity; a new identity is used only after it was saved and read back; the
fingerprint is also noted in a plain file, so a key that has really gone is
reported as missing and only an explicit reset makes the installation a new
device. Unit tests cover each rule, and a round trip through the real Windows
credential store passed. Not done: the Linux credential store on a real desktop,
storage for the host service (which runs without a user session), Android.

Account service, device part (written 2026-11-13/14 in `cloud/accounts`, not
deployed). The 3.x service and its database are kept; Secpar 4 adds one table and
these routes, all for a signed-in account:

| Route | Purpose |
| --- | --- |
| `POST /v1/devices/challenge` | One-use challenge, valid five minutes |
| `POST /v1/devices` | Enrol this installation: its certificate plus a proof signed with the certificate's key over the challenge. The same certificate again updates its row, so there is never a second record for one device. A certificate held by another account is refused. |
| `GET /v1/devices` | The account's devices |
| `DELETE /v1/devices/:id` | Remove a device |
| `GET /v1/devices/:id/access` | For a hosting device: the access list, signed by the service and valid seven days. It names the owner's other devices and the devices of accepted, unblocked friends, each friend with trusted or not, keyboard and mouse. |

The host feeds the fingerprints from the access list to the tunnel's allow-list
and the supervisor, and applies the friend flags itself. Tests cover each rule
above (64 pass, 7 of them new). Not done: deploying it (the migration changes the
live database, so it needs the owner's go-ahead), the client side in
`secpar-core`, friend devices asking a non-trusting host, and how a host that has
no user session signs in to fetch its list.

**App side of the account service (2026-11-14).** `crates/secpar-core/src/account.rs`
signs in through the existing `/v1/app/sign-in` route, keeps the session in the
credential store, enrols the device identity with a proof signed by its own key,
lists and removes devices, and fetches the access list. `verify_access` checks a
signed list against the service's public keys without the network (signature,
issuer, audience, hosting device, expiry), which is what a hosting device uses for
its saved copy. The client refuses plain HTTP except for a test service on
127.0.0.1.

Checked end to end on Windows against the real service code running locally
(`cloud/accounts/test/local-server.mjs`, in-memory database, accounts created
through the real sign-up routes): wrong password refused, sign-in, enrol twice
stays one device, another account cannot take the device, Android cannot host,
list fetched and verified, list refused for another device, removed device gone
from the next list, closed port reported as unreachable. Not checked: the live
service (not deployed), Linux and Android, and TLS to the real address.

**Hosting before login, and who may connect (2026-11-14).** Decision: on a
hosting computer the part that runs before login holds the device key and never
holds the password or the session. Two consequences:

- Enrolling is split in two. The signed-in window asks the service for a
  challenge (`Service::challenge`), the part with the key signs it
  (`account::enrolment`), and the window hands the proof in (`Service::submit`).
- The hosting part fetches its own list with only its key: `GET /v1/device/time`
  for the service's time, then `POST /v1/device/access` with the certificate and a
  token of type `secpar-device-request+jwt` signed by it. A device that does not
  host, was removed, or signs with another key gets nothing.

`crates/secpar-core/src/hosting.rs` (`Access`) turns the list into the answer the
tunnel needs. Allowed: the owner's other devices, trusted friends' devices, and
friends who are asked about each time only while the owner's acceptance lasts.
The last list is saved with the service's keys and checked again at start, so it
works offline until it expires (7 days). An unreachable service keeps the saved
list; a service that no longer knows the device as a host clears it.

Checked on Windows: 17 unit tests, 66 service tests, and the end-to-end run
against the local service (fetch with the device key, offline use, restart from
the saved file, removed laptop dropped, removed host cleared). Not done: where
the hosting part keeps the device key on each system, the local channel between
the window and the hosting part, the request a friend sends to be accepted, and
handing `Access::allowed` to the tunnel and the engine's pairing in a running
hosting service.

**Hosting put together (2026-11-14).** `crates/secpar-hosting` joins the three
parts: the access list decides who the tunnel lets in, and the engine behind the
tunnel only pairs with a device that came in through it.

- The engine now listens on this computer only (`bind_address = 127.0.0.1` in
  the supervisor's config).
- The tunnel has its own small channel to the host (`Client::request`,
  `Host::on_request`). The host knows which device asks from the tunnel's
  certificate check, not from anything the device says.
- Pairing: the connecting side starts its stream component's pairing against the
  tunnel's local end and asks the host over that channel to complete it. The
  host takes the account and the device name from the signed list and records
  the grant for that device. No PIN or prompt is shown to anyone.
- `Hosting::apply` closes the tunnels of devices that may no longer connect and
  takes the engine pairing away from devices the list no longer names. A friend
  whose acceptance ran out only loses the tunnel. A device that returns to the
  list must make a new stream identity (the host answers `revoked`).
- A lost or expired list lets nobody in but revokes nothing.

Checked: 5 tests with real tunnels and a stand-in engine (Windows), and
`integrations/gamestream/kde-hosting-check.sh` in a throwaway container on the
build box with the real engine and stream component: not reachable on the
network address, unlisted device refused, listed device paired through the
tunnel, grant recorded for that device, removal closed the tunnel and removed
the engine's certificate, old stream identity refused afterwards, new one
paired. That run pairs only; the engine there had no usable display for its
X11 capture setting, so no stream was started. Not done: a stream through this
path, the hosting service as an installed background service, where it keeps
the device key, the channel to the window, the friend's request to be accepted,
and keyboard/mouse limits for guests.

**Stream through the hosting path (2026-11-14).** `kde-hosting-check.sh` with
`HOSTING_CHECK_STREAM=1`, in the test container on the build box: after pairing
over the tunnel, the stream component streamed the KDE session for 15 seconds
through the tunnel. KWin capture, VA-API encoding and decoding, 720p60 H.264.
Two runs with figures: 60.0 and 60.5 fps from the network, 59.7 and 58.6 fps
rendered, 0% dropped by the network. A first run connected but its log had no
figures; not explained. With `HOSTING_CHECK_REMOVE=1` the device was removed
from the list 8 seconds into the stream and its tunnel closed.

Open from this run: after the tunnel closes, the stream component keeps its
window until it is stopped, and the engine only ends the session by its own
timeout. The app must stop the component when the tunnel reports closed, and the
hosting side should end the engine session of a device whose access ended. Sound
was not measured here, and both ends ran on one computer.

**The background part of the app (2026-11-14).** `crates/secpar-service`.
Decision: on a desktop computer one background part holds the device key and is
the only part that speaks as the device. It accepts tunnels when the computer
hosts and opens the tunnel when the person connects to another computer. The
window holds the session and does everything with the account service; it never
has the device key, and the background part never has the password or session.
This is still one app: the background part is an internal piece of it.

The window talks to it over a channel that stays on the computer: a named pipe
on Windows (system, administrators and the logged-in person only, no remote
clients), a socket file on Linux (owner only). One JSON object per line:
`status`, `enrolment`, `refresh`, `accept`, `dismiss`, `connect`, `disconnect`,
`pair`. The engine's base port on every hosting computer is 46989, so it runs
beside an installed engine of the same family; the connecting side offers the
same ports on 127.0.0.2.

Checked on Windows (`tests/two_computers.rs`, real pipe, real tunnels, the
account service code running locally, stand-in engine): window signs in and
enrols a hosting desktop and a connecting laptop through their background
parts; the laptop is refused before the desktop has its list and let in after;
pairing is granted under the account for that laptop; another account's
computer is refused; removing the laptop ends its tunnel and grant; removing
the desktop leaves it with no list. Compiles on Linux; not run there.

Not done: running it as an installed service (Windows service before login,
Linux user service), the program itself with the real credential store and real
engine, starting the stream component from the window, the friend's request to
be accepted, and guest keyboard/mouse limits.

**The window signs in (2026-11-14).** `crates/secpar-app` is the window's
non-drawing side, built as a library the Qt window loads (`secpar_app.dll`): one
JSON request in, one answer out (`configure`, `state`, `sign_in`, `sign_out`,
`devices`, `remove_device`). It keeps the session in the system credential
store and asks the background part for anything that needs the device key.

- Signing in enrols this computer in the same step. Signing in again keeps one
  device.
- Signing out takes this computer off the account, so it stops hosting for it.
- A session the service no longer accepts is forgotten and the sign-in page
  returns.

The Qt window has a sign-in page (`SignInPage.qml`, `FlatField.qml`) and a
`Backend` object that makes each call on a worker thread. Computers lists the
account's other hosting computers; Settings shows the account and its devices.
Connect is still off: a computer does not yet report where it can be reached.

Checked on Windows: `crates/secpar-app/tests/window.rs` against the local
account service and a real background part, and the built window run against
the same, with captures in `docs/screenshots/secpar4/` (sign-in at 800x600 and
640x400, Computers, Settings: Devices and Account). Not checked: typing into the
sign-in page by hand, the real credential store from the window, Linux.

**Finding a computer and Connect (2026-11-14).** A hosting computer tells the
account service where it can be reached each time it fetches its access list:
its tunnel port and the address it has on its own network (migration
`0010_device_reach.sql`). The report is inside the token signed with the device
key. The device list gives it to the same account's signed-in devices only, and
marks a hosting computer online for 20 minutes after its last report. The
servers therefore also see each hosting computer's local network address.

A hosting computer also fetches a new list when a device it does not know tries
to connect (at most once in ten seconds), so a computer the owner has just
signed in on is let in on its next try instead of waiting for the 15 minute
refresh. The window tries three times.

Connect in the window opens the tunnel and the tile shows Connected with
Disconnect. No stream starts yet: starting the stream component and pairing it
from the window is the next step.

Checked on Windows: 67 service tests; `crates/secpar-app/tests/connect.rs`
(online and offline shown, a just-enrolled laptop connects, disconnect, a
computer that is off, an unknown computer); the built window against a stand-in
hosting computer (`crates/secpar-service/examples/test_host.rs`), with captures
of Online, Connected and "did not answer". Known gaps: a computer that was
switched off still shows Online for up to 20 minutes; only one local address is
reported and none for the internet side, so this works on one network only; the
three tries take about 30 seconds before the window says the computer did not
answer.

**The window starts the stream (2026-11-14).** `crates/secpar-app/src/stream.rs`
runs the private Moonlight component once the tunnel is open, so Connect now
produces a picture instead of only a tunnel. The large preview tile the plan
asks for is still the monitor icon; previews remain a later step.

- The component is installed beside the window (`SecparStream.exe` on Windows,
  `secpar-stream` elsewhere). It runs in a profile per hosting computer under
  Secpar's data folder, with `portable.dat`, so it never reads or writes an
  installed Moonlight profile.
- The first Connect to a computer pairs without a PIN on screen: the component
  writes its client id and certificate, the window sends them and a one-use PIN
  to the background part through the tunnel, and the host records the grant for
  that device. The profile is marked as paired. If the host no longer knows it,
  the stream ends at once and the next Connect pairs again with a new identity.
- The stream runs windowed with the packet size, absolute mouse, keep-awake,
  frame pacing and system-key capture settings this plan asks for. A closed
  tunnel stops the component; closing the stream window disconnects the tunnel;
  closing the Secpar window stops both before the process exits.
- While the stream is live the computer tile reads Streaming with Disconnect.
  Connecting to another computer ends the first stream and tunnel first.

Checked on Windows against the local account service, real background parts,
real tunnels and a stand-in hosting engine, with a test-only stand-in for the
stream component (`--features fake-stream`): sign-in, enrolment, Connect,
pairing over the tunnel, the component starting in its own profile, state
reporting `streaming`, and Disconnect ending both. Captures are
`docs/screenshots/secpar4/computers-streaming-800x600.png` and
`computers-streaming-640x400.png`. A single tile now keeps one column width
instead of stretching across the row. Not checked: the real Moonlight window
with a real engine started from this window, sound, input, Linux, and packaging
the component and its libraries into the one installer.

### 3.4 Servers and what they see

The owner runs the account service and relay. Video, sound, input, clipboard and
files are encrypted between the two devices, so the relay carries data it cannot
read. The servers see account email, device names, friend links, and which device
connects to which and when, and each hosting computer's tunnel port and local
network address. Self-hosting is not a goal for the first release, but
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
- `integrations/gamestream/patches/sunshine`: three patches, 549 lines together.
  One binds pairing approval to a client certificate; one reads the management
  password from a pipe instead of the command line; one changes the size of a
  running stream on request (`docs/live-resolution.md`). Both build scripts now
  apply them. These count toward the "few and separate" patch set in risk 6.
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

### First stream between the Secpar-built parts (2026-11-13)

`integrations/gamestream/kde-stream-check.sh`, run in a throwaway container from
the headless KDE image on the build box (Intel UHD 730). Both parts were built
from the pinned sources with Secpar's patches: the Linux engine and the Moonlight
stream component. Result of a 16 second stream at 1280x720, 60 fps, H.264:

- PIN pairing completed.
- Capture through KWin, encoding and decoding through VA-API on the Intel GPU.
- 60.0 fps received, 59.5 fps rendered, no frames lost.
- The session's test tones arrived at the receiving side's sound output.

What this does not show: both ends ran on one machine over loopback, so there is
no real network, no second GPU and no real display. Pairing used the engine's
fixed-PIN input, not the supervisor's certificate-bound approval. Controllers,
keyboard and mouse were not exercised. One run of the script ended early without
output and passed when repeated; the cause was not captured.

Sound note for the product: while a stream runs the engine makes its own virtual
output the system default and captures from it, unless `audio_sink` is set, in
which case it captures that named output instead. Secpar should leave
`audio_sink` unset unless the user picks a device.

### First stream over the real network (2026-11-13)

Hosting engine in the KDE container on the build box (wired, Intel UHD 730,
`integrations/gamestream/kde-host-serve.sh`, ports published for 100 seconds).
Connecting part on the owner's laptop (ThinkPad X250, HD 5500, Wi-Fi, real Plasma
Wayland session), run from the test folder. 20 seconds at 1280x720, 60 fps, H.264:

- Pairing by PIN completed across the LAN.
- The laptop decoded in hardware (VA-API) and used Moonlight's VA-API renderer on
  Wayland. This is the case 3.x handled with a software decoder.
- 59.8 fps received and decoded, 58.0 fps rendered.
- No frames lost to the network, 0.83% dropped for jitter.
- Network latency 13 ms average, decoding 0.65 ms, host processing 2.7 ms average.

A second run of two minutes was watched by the owner on the laptop, who judged it
"smooth and clear": 59.2 fps received, 59.0 rendered, no network loss, 0.11%
jitter drops, 6 ms latency.

Not covered: the content was a simple animation, not a game or desktop work; no
sound (the tone was off), no input, no controllers; 1080p untested; fixed-PIN
pairing. A new test container has a new identity at the same address, and the
connecting part then refuses to pair until its saved entry is cleared; Secpar
must handle "same address, different computer" itself.

Blocked for now: streaming from this Windows PC's engine to another device.
Windows Firewall blocks inbound connections to the test build and has no rule
for it. The owner must allow it; Secpar's installer will add its own rule.
The laptop's firewall (ufw) is on as well, so the laptop cannot host a LAN test
without a rule either.

### Device-bound pairing through the supervisor (2026-11-13)

The existing `pairing_review` check was run in the KDE container against today's
patched engine and connecting part. All of it passed:

- The engine refused an approval with the wrong certificate, the wrong client id,
  or missing identity, and enrolled nothing.
- Approval naming the exact device completed pairing, and the engine recorded
  that device's certificate.
- A second device needed its own approval and could not reuse the first.
- A wrong PIN enrolled nothing.
- Revoking one device removed only that device, could not be undone by
  re-approval, and still held after the engine restarted.

This is the mechanism Secpar's account service will drive: the service tells a
host which device certificates may connect, and the supervisor approves exactly
those. The check used the review example's software and X11 settings, so the
engine reported no display there; pairing does not depend on capture.

Two things to handle in the Secpar host: Sunshine always announces itself on the
LAN by mDNS under the computer's name (no setting turns it off), and its data
folder is always `config` beside the executable.

Pinned sources are in `upstream/pins.json`; `scripts/fetch-upstream.sh` fetches them
and `--check` confirms each pinned commit exists upstream.

### Desktop app window, first version (2026-11-13)

`desktop-qt/` is the Qt Quick app, built on Windows with MSVC 2022 and Qt 6.12.0
(`scripts/build-desktop-qt.cmd`). Qt 6.12.0 is the version Moonlight's own Windows
builds use. It was fetched from download.qt.io with checksums verified, because
the usual download tool does not yet understand Qt's folder layout from 6.11 on.

What exists: the window with top tabs (Computers, Friends, Settings), the approved
wordmark with the version beside it, computer tiles two per row at 640 wide,
a friends list and a settings page with sections on the left. Icons are the pinned
Lucide set. Two rows of tiles fit without scrolling at 640x400 and 800x600.

It shows no real data yet. Without `--sample` every page is an empty state; with
it the pages show fictional entries marked "Layout sample". Nothing is connected
to the tunnel, the engines or an account. Captures from the running program are
in `docs/screenshots/secpar4/`. Not built or run on Linux yet.

### Windows as the connecting side (2026-11-13)

The pinned, patched Moonlight stream component builds on Windows with MSVC 2022
and Qt 6.12.0 (`scripts/build-stream-windows.cmd`), using Moonlight's own prebuilt
Windows libraries (release v19, checksum in the script). Result: `SecparStream.exe`,
2.8 MB before its libraries.

Stream from the KDE test container on the build box to this PC over the wired LAN,
20 seconds at 1280x720, 60 fps, H.264: paired by PIN; decoded in hardware
(D3D11VA on the RTX 2060); 59.1 fps received and rendered; no network loss, 0.03%
jitter drops; 1 ms latency. Nobody was asked to judge the picture. No sound, input
or controllers.

Measured size of the Qt parts the app window needs on Windows (`windeployqt`,
release, no translations): 47.2 MB in 149 files, 18.4 MB zipped. The largest are
Qt6Core 10.0, Qt6Gui 9.2, Qt6Quick 6.4 and Qt6Qml 5.1 MB.

Open question raised by the owner the same day: Qt or an SDL-based window. In the
pinned Moonlight source the streaming code is about 24,700 lines in 63 files, 55
of which use SDL and 14 include a Qt header; the pairing and host communication
code (about 4,600 lines) uses Qt throughout. The owner then said to stick to the
plan, so the window stays on Qt and Moonlight's code stays unmodified. The release
also stays as planned: everything in one release, no reduced first build.

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
- Decided 2026-11-13: the 3.x account database is kept. Existing emails and
  passwords continue to work. Devices and pairings still start fresh, as in the
  "3.x installs" decision above.

## Sources

- https://docs.lizardbyte.dev/projects/sunshine/v0.23.0/about/advanced_usage.html
- https://redlib.hbubli.cc/r/LizardByte/comments/1jf2lqu/multiple_clients_connecting_to_windows_sunshine
- https://github.com/orgs/LizardByte/discussions/262
- https://github.com/ClassicOldSong/Apollo
- https://github.com/ClassicOldSong/Apollo/issues/874
- https://github.com/LizardByte/Sunshine/issues/1320
- https://docs.lizardbyte.dev/projects/sunshine/latest/md_docs_2troubleshooting.html
