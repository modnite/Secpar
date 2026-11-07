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
  Secpar settings identity, internal executable name and Alt+F fullscreen shortcut.
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

## Build the Android core harness

On Windows, with the Android SDK, NDK28.2 and Java21 available:

```powershell
python integrations/gamestream/prepare.py moonlight-android D:/Secpar-CI/engine-build/android-source
./gradlew.bat :gamestream-core:assembleDebug :gamestream-core:assembleDebugAndroidTest '-PsecparUpstreamSource=D:/Secpar-CI/engine-build/android-source' '-PsecparEngineBuildRoot=D:/Secpar-CI/engine-build/android-harness'
```

These debug artifacts stay local. Existing app/release builds do not include the
harness unless `secparUpstreamSource` is explicitly supplied.

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
