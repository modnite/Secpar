# Upstream streaming migration

This branch replaces PRS with the upstream streaming implementations. It is not
enabled in released Secpar. The installed host and accepted clients stay intact.

Selected baseline:

| Role | Source | Pin |
| --- | --- | --- |
| Windows/Linux/macOS client | Moonlight Qt | v6.2.0 |
| Android client | Moonlight Android | v12.2 |
| Desktop host | Sunshine | v2026.914.233613 |
| Feature references | Artemis / Apollo | v20.2.6 / v0.4.6 |

The exact commits are in `upstream-lock.json`, and the original repositories are
submodules under `upstream/`. Keep upstream licenses and nested dependency notices
with binaries and publish corresponding source for distributed modifications.
Sunshine declares GPL-3.0-only; preserve the original license terms for each component.

Moonlight Qt's full FFmpeg/SDL decoding, audio, frame pacing and native GPU
presentation are used together. Importing only moonlight-common-c while keeping
Secpar's current RGB renderer would retain the bottleneck we are replacing.
There is no claim yet that this migration matches Moonlight on the user's X250.

The baseline selection uses the user's working Moonlight comparison, current
upstream platform support and the available stable releases. Apollo and Artemis
have useful extensions; their stable releases are older than these baseline pins.
Neither a fork's name nor a release date proves better stream performance. Revisit
individual extensions against actual reproduction cases, without replacing the
whole media pipeline with Secpar-specific scheduling again.

## What runs now

- Full Moonlight Qt release worker compiled on the isolated Linux runner. Its
  media implementation is unchanged. Small reviewed patches set a separate
  Secpar settings identity, internal executable name and Alt+F fullscreen shortcut. A managed-pairing
  environment flag exits the standard CLI pairing flow with success/failure instead
  of leaving a confirmation dialog open; upstream crypto is unchanged. A third
  patch adds the Secpar in-stream menu (`0003`), described below.
- Actual worker `--version` and `stream --help` run with no network, user home,
  audio, display or GPU mounts. A source audit matches511 tracked files, including
  pinned nested dependencies. Stripped worker2.2MiB (runtime libraries not included).
  It creates `.config/Secpar/SecparStream.conf` in
  its temporary home, rather than touching an installed Moonlight profile.
- Optional Android library harness compiles the original native core/JNI and
  original Android audio renderer. This is not yet the complete Android client:
  the upstream MediaCodec renderer, input and connection lifecycle still need
  integration. Real JNI initialization passes on owned emulator5556. ARM64 ELF LOAD
  segments use16KiB alignment. JNI consumer keep rules carry the upstream binding
  rule into future optimized APKs. The existing APK does not depend on this harness.
- `prepare.py` verifies the release commit and every initialized nested gitlink,
  refuses existing output directories, and applies reviewed patches to a fresh
  build checkout. Windows builds use short paths to avoid nested JNI path limits.

## Build the private desktop component

```sh
python3 integrations/gamestream/prepare.py moonlight-qt /tmp/secpar-qt-source
sh integrations/gamestream/build-linux-client.sh /tmp/secpar-qt-source
```

No installer, system service, firewall rule or public release is created. Packaging
must put this private executable and its libraries inside the single Secpar app.
Do not offer another client download or ask users to install Moonlight separately.

## The Secpar stream menu

`patches/moonlight-qt/0003-secpar-in-stream-menu.patch` adds a floating menu over
the video. It is drawn through the renderer's overlay channel, so it appears in
windowed and fullscreen mode and in every renderer Moonlight selects. Upstream
decoding, audio, pacer and crypto code is untouched.

- Button: 40 px, translucent, with the approved triangle. It is anchored to the
  top-left of the picture, 12 px inside it, and stays on screen while streaming.
  On a 4:3 stream in a 16:9 window it sits at the top-left of the picture, not at
  the window corner.
- Drag it anywhere inside the picture. The place is kept as a fraction of the
  picture area, so it survives a window resize and a fullscreen toggle, and it is
  saved for the next stream in `secpar/buttonX` and `secpar/buttonY` under the
  `Secpar` identity. It cannot be dragged outside the picture, or over the
  letterbox bars. Use **Reset position** in the menu to put it back at the
  top-left.
- The panel uses the desktop stream menu numbers: 250 px wide, 34 px rows,
  20 px Lucide 0.468.0 glyphs (pinned commit `f12b0de1`, ISC), near-white text,
  mint checks, and red for Disconnect. It opens to the right of the button, or to
  the left when the button is near the right edge, and stays inside the picture.
- Rows: Window/Fullscreen, Quality, Players and stats, Reset position, then
  Disconnect. The Players page toggles the stats overlay and lists the attached
  controllers with their player numbers.
- There is no Files row. This component has no file transfer, so the row would
  be a button that does nothing.

### Changing the size while a stream runs

The Quality page lists the size the stream is running at first, then 1280x720,
1920x1080, 2560x1440 and 3840x2160, with the current one checked. Picking
another sends this to the local background part, on the same channel the Secpar
window uses:

    {"do":"resolution","width":1280,"height":720}

The background part carries it over the tunnel to the host engine
(`docs/live-resolution.md`). A status request on the same channel first tells the
page whether a size change is possible at all:

- **Channels.** Windows named pipe `secpar-service`, Linux
  `$XDG_RUNTIME_DIR/secpar-service.sock`. `SECPAR_SERVICE_CHANNEL` overrides the
  name for a development build.
- **While waiting** the page says "Changing to 1280 × 720…" and the rows stay
  read-only until the picture reports the new size. The check mark and the
  numbers come from the decoded frame, so they show what the stream really is,
  not what was asked for.
- **When it cannot work** the rows are read-only and the page says why: no
  Secpar background part on this computer, not connected through the app, the
  app refused the change, or only the owner may change it. A component run
  outside Secpar therefore keeps the read-only page.
- The size change keeps the stream running: the host rebuilds its encoder and
  does not restart capture. Bitrate, frame rate, codec and the hardware/software
  decoder are shown read-only, because they cannot change inside a session.

### Chords

The three chords work whether or not the menu is open, and none of them reaches
the host:

| Setting key | Environment override | Default | Action |
| --- | --- | --- | --- |
| `secpar/chordMenu` | `SECPAR_CHORD_MENU` | Ctrl+Alt+S | Open or close the menu |
| `secpar/chordFullscreen` | `SECPAR_CHORD_FULLSCREEN` | Ctrl+Alt+W | Window or fullscreen |
| `secpar/chordDisconnect` | `SECPAR_CHORD_DISCONNECT` | Ctrl+Alt+Q | End the stream |

The keys live in the component's own `SecparStream.conf` under the `Secpar`
identity, so a launcher can set them there or in the environment before starting
the component. Each value is a `Ctrl`/`Alt`/`Shift` combination plus one key. A
value with a function key, with the Windows key, or without a modifier is
rejected and the default is used, and the component logs that it did.

While the menu is open it owns the pointer and the keyboard, and nothing it
consumes reaches the host. Closing it restores capture only if capture was
active when it opened. Escape closes it, the arrow keys and Enter move and
select, and Backspace goes back one page. The button and its drag need the same
released pointer a click always needed, so use the chord first when the stream
has the mouse.

### Where each renderer puts it

The overlay channel used to place every overlay at the bottom-left of the frame,
which is where Moonlight's performance overlay belongs. The Secpar image carries
its own place in window pixels and each renderer applies it in its own
coordinates:

| Renderer | Placement |
| --- | --- |
| D3D11VA, DXVA2 | x and y as given, with y flipped because this path counts from the bottom |
| SDL, libplacebo, VDPAU, DRM | x and y as given |
| EGL (OpenGL) | x as given, y flipped into GL's bottom-left origin |
| VAAPI | converted into the source surface's pixels, so on that path the button scales with the picture |

Moonlight's own debug and status overlays keep their previous positions.

What was run on 2026-11-15: the D3D11VA path on Windows, windowed and fullscreen,
with the menu anchored to the picture, dragged to the bottom-right corner, kept
there across a resize and a new stream, reset from the menu, and a real size
change from 1024x768 to 1280x720 driven from the Quality page. The other renderer
paths above were written but not run here.

## Build the Android core harness

On Windows, with the Android SDK, NDK28.2 and Java21 available:

```powershell
python integrations/gamestream/prepare.py moonlight-android D:/Secpar-CI/engine-build/android-source
./gradlew.bat :gamestream-core:assembleDebug :gamestream-core:assembleDebugAndroidTest '-PsecparUpstreamSource=D:/Secpar-CI/engine-build/android-source' '-PsecparEngineBuildRoot=D:/Secpar-CI/engine-build/android-harness'
```

These debug artifacts stay local. Existing app/release builds do not include the
harness unless `secparUpstreamSource` is explicitly supplied.

## Actual host/client interoperability gate

`interop-smoke.sh` runs a real Sunshine host and the built Secpar-named Moonlight
worker in one isolated container. The host sees only a private generated X display;
the client uses a second display. A generated500Hz tone plays into a host-only Pulse
sink, while a separate sink receives the streamed client audio. Its monitor must
contain that tone (RMS and zero-crossing checks), not just silence or an open device.

The test rejects an incorrect PIN from a separate client profile, then authenticates
the correct client, checks received/decoded/rendered frames and recovered audio.
PASS: generated640x360/30fps software fixture, including normal stream teardown.
This is interoperability evidence, not X250 hardware throughput or1080p acceptance.
The rejected profile is separate: retrying the same rejected identity immediately
can encounter Sunshine's pending-session409. Do not work around this in production
by silently rotating device identities; the ownership broker needs bounded pending
pairing cleanup/retry handling.

The test uses the official Sunshine2026.914.233613 x86-64 AppImage, not a rebuilt
Secpar host. The asset SHA256 is verified before running it. No host installation,
user display, audio device, render node, input device, network or home is mounted.
All test credentials, paired profiles and generated audio stay in container tmpfs.

After building the client dependency image and preparing the worker:

```sh
sh integrations/gamestream/run-interop.sh /path/to/Sunshine.AppImage /path/to/secpar-stream
```

The fixture asset is available at:
https://github.com/LizardByte/Sunshine/releases/download/v2026.914.233613/Sunshine_2026.914.233613_x86_64.AppImage

## Required integration before switching the product

1. Supervise Sunshine in a Secpar-owned state directory and separate port range.
   Preserve installed Sunshine/Apollo/PC Remote instances, services and pairings.
   Keep its management endpoint local and authenticated.
2. Authorize standard GameStream pairing through the existing authenticated,
   per-device Secpar ownership channel. Existing PRS credentials are not GameStream
   certificates. Do not silently discard them, open pairing to LAN strangers or
   require the user to repeat QR setup. Map revocation to the new client certificate.
3. Launch the private Moonlight worker from the Secpar computer list. Pass
   `--keep-awake`, `--absolute-mouse`, `--capture-system-keys fullscreen`, appropriate
   resolution/FPS/decoder options and frame pacing. Keep windowed mouse travel free,
   fullscreen capture and Alt+F. Do not use `--quit-after` to kill other viewers.
4. Integrate Android's upstream renderer/audio/input and dedicated stream activity.
   Only that activity may declare PiP support; the launcher/settings must not.
   Keep the device awake for the whole connected session, releasing the sleep
   inhibition when it ends. Verify this on Windows, KDE Wayland, macOS and Android.
5. Keep accounts, ownership, friends, clipboard, the approved menu/brand assets,
   automatic updates, three remote viewers and the single app per OS. Android is
   client-only. Preserve LAN-first discovery and VPN-free remote access; standard
   GameStream uses multiple TCP/UDP flows, so the current PRS relay is not a
   drop-in transport. Reuse mature network components around the upstream engine.
6. Test actual1080p60 and720p60 video, text colours, independent cursor movement,
   pause/resume audio, minimizing, reconnection, controllers and concurrent viewers.
   Compilation, JNI initialization and software-generated fixtures do not establish
   sustained hardware throughput or parity with Moonlight.

Sunshine's current source has KWin capture and portal capture implementations;
physical KDE permissions/capture/input still require a real test. Moonlight's
4:4:4 option is capability dependent, not a promised solution for the X250's GPU.
Text fringing also needs comparison with actual source scaling and subpixel fonts.

## Latest performance evidence

The user confirms3.12.0 retains audio and the connection when the Linux stream is
covered/minimized20s. This fixes the reproduced UI-backpressure outage, not stream
smoothness.1080p is poor with either decoder;720p is usable but still hitches.
Slowdowns correlate with received bitrate suddenly falling below1Mbps. Track
encoder target separately from received bitrate, loss and complete received frames.
The custom host normally floors its requested target at2Mbps (unless the user cap
is lower); a received rate below1Mbps does not establish what the encoder requested.
No new bitrate tuning is being added to PRS as a substitute for this migration.

Windows720p text shows coloured fringes. The current software converter assumes
BT.601 while Windows capture produces BT.709; source downscaling/ClearType/chroma
sampling may also contribute. Preserve this case for the new renderer comparison.
No assumption that switching decoders, disabling audio, or selecting720p resolves
all performance problems. Glyphs are approved. Android footer metrics have been
corrected separately without changing streaming code.
