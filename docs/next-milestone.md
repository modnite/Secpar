# Active Secpar work — 2026-11-06

Keep working, solo. User wants completed releases, check-ins and actual screenshots.
Never lose the broader Windows/Linux/macOS host+client scope; Android client only.

## Latest delivery

- 3.6.0 is published at https://github.com/modnite/Secpar/releases/tag/v3.6.0.
  Source tag is commit 23dd11f on fix/shared-capture in adjacent worktree
  ../secpar-stream-fix. Main contains uncommitted account work; don't build its
  current Windows tree as a release or overwrite it with the release worktree.
- Windows installer and Android APK staged in D:/Secpar-CI/publish-3.6.0 alongside
  the Linux AppImage and checksums. Lean optimized binaries; APK same certificate,
  package com.pcremote.app, version 3.6.0 code 522, 206 unit tests passed. APK is
  non-debuggable; all five black launcher icon densities match source. Padding
  and transparent desktop icon unchanged. Native desktop capture tests 21 passed
  (one vault integration check ignored); one-hand modifier regression passed.
- Windows 3.5.0 did NOT allow a second real stream. Reproduced exactly: first HEVC
  stream opens, second DXGI capture fails. Windows permits one duplication per
  output/process. d473b33 fixes it with one producer per monitor, latest frames,
  bounded immutable GPU texture leases, independent encoders/transports. Strict
  hardware test now passes three streams, fourth refused, replacement after a
  disconnect, capture stops after last viewer. Do not claim physical multiplayer
  validation or desktop gamepad forwarding is finished.
- Windows 3.6.0 installation launched with RunAs /S, exec session 28826. Still
  waiting at OS UAC as last checked; installed UI version 3.5.0, service running.
  Poll/read back installed host/UI 3.6.0 before claiming installed. User explicitly
  authorized updating/running Windows releases; do not ask permission again.
  Only Secpar may restart; preserve independent PC Remote/Sunshine.
- 3.6 UI: default 540×400, version by logo, Client settings tab for resolution,
  FPS, bitrate, audio and capture shortcut; Ctrl+Alt+G default plus alternatives.
  Legacy Ctrl+F12 remains an unadvertised fallback. Android defaults Ctrl+Alt+G,
  explicit saved choices retained, Moonlight shortcut still accepted. Hosting
  quality choices now show actual resolution/FPS and are enforced on Windows.
  Software H.264 decoder is honestly labeled; hardware decoding not implemented.
  Screenshots outputs/ui-review/client-settings-540.png, hosting-settings-540.png.
- Release installer extracted to D:/Secpar-CI/release-verify-3.6.0; UI exact match
  to worktree unified exe. Host differs from target only expected three-byte
  Tauri UNK→NSS marker. No .debug sections, debug/review/PDB payloads. PE CodeView
  directory may exist even in optimized stripped MSVC output; don't mistake that
  external reference metadata for a debug build.
- Linux release built on OMV in /tmp/secpar-core.JBPqA6/repo with release-worktree
  sources. This remote checkout currently has RELEASE sources, not main's WIP.
  Public AppImage again passed isolated Arch/Weston native Wayland/EGL render test.
  Do not overwrite remote source while a build is running. ghrunner has no Docker
  access; use existing root-owned isolated test image, don't grant Docker rights.

## Current priority: client hardware decode and polish

- User reports Linux video/cursor stalls while video plays, same symptom Android
  previously had. Current desktop decoder is OpenH264 software on the render
  thread, converting to RGB there. It can block cursor/UI processing. Implement
  hardware decoding and move decoding off UI thread with bounded latest output.
- User laptop is ThinkPad X250 / CachyOS KDE Plasma Wayland. Lenovo PSREF identifies
  Intel HD Graphics 5500; CPU SKU varies. Probe actual driver/decoder capability,
  not just plugin presence. Target hardware H.264 via VA-API on Broadwell; don't
  advertise unsupported HEVC/AV1. Existing AppImage already optionally bundles
  libgstva.so and excludes system libva/GPU/Wayland runtimes. GStreamer libraries
  are existing Linux host dependencies; add desktop-target deps if needed.
- Primary docs: https://gstreamer.freedesktop.org/documentation/va/vah264dec.html,
  vapostproc.html and valib/index.html. A real decode probe/first output must prove
  acceleration; don't label it active because a plugin exists. Handle fallback
  cleanly and expose usable choices. Windows/Mac decoding also remains software.
- User explicitly wants: compact professional aligned UI with little dead space,
  separate host/client settings, real resolution/FPS labels, version beside logo,
  comfortable one-hand configurable shortcut without function keys. They still
  disliked 3.5 UI. Don't claim the entire UI overhaul is complete.
- Android icon background BLACK is the latest correction, overriding white. Done
  in 3.6. Preserve padding and approved mint/orange triangle geometry.

## Account-owned access (main WIP, not in public 3.6)

- Live backend API committed 58400d4 and deployed successfully (42206186616).
  29 tests plus actual workerd/D1 encrypted handoff pass. /v1/computers lists owned
  enrollment/presence/fingerprint. Owner-only two-minute pairing requests are
  bound to requesting native session and current host credential generation.
  Host polls, returns one-use link encrypted at rest, requesting session consumes
  once; expiry, relink/unlink, sign-out, ciphertext swapping and races covered.
- Main working tree implements shared machine proof/account/heartbeat/polling in
  crates/secpar-host-core/src/{account,account_link,pairing_codes}. Windows DPAPI
  adapter preserves host-account.bin format. Linux private 0600 machine file,
  enrollment commands, heartbeat and polling. No user bearer enters Windows SYSTEM.
- Separate owner pairing codes preserve manual QR. Atomic PairingStore.pair_device
  binds code origin to device creation under one lock; owner-tagged tokens are
  denied/revoked when host account changes/unlinks; manual access retained.
- Desktop main WIP auto-enrolls on authenticated sign-in/startup, lists owned PCs,
  requests bootstrap on first connect and remembers each client token in an OS
  vault namespace per account+cloud computer. Explicit host unlink suppresses
  automatic relink via stored preference. Logout clears owned UI/access and stops
  owned streams; immutable session_account prevents a late pair response from
  being saved into global/manual storage after logout. This last change compiles.
- Verified: 21 shared tests; 22 desktop tests (one vault ignored); 15 Windows TLS
  agent tests (one live relay ignored), including concurrent owner pairings and
  manual session surviving unlink. Linux earlier snapshot passed 11 host and 20
  desktop tests; latest Linux account-origin test and desktop account tests need
  refreshed Linux run after release checkout is no longer in use.
- Finish Android account sign-in/list/bootstrap, Linux control relay + STUN/TURN
  media path, cache updates/revocation and installed end-to-end testing. Android
  does not yet have account UI. Main WIP needs commit, then merge/cherry-pick 3.6
  desktop settings changes without losing owned-account changes in lib.rs.
- Existing control pairing currently tries relay FIRST on Android and desktop;
  reconnect is LAN-first. User wants LAN-first. Fix route order without allowing
  mismatched certificates. Android currently aborts on first pin mismatch; a
  different LAN with overlapping private IPs needs trying other verified routes,
  never sending credentials to the mismatched peer. Relevant tests in agent tests.
- Relay default remains original author's Worker. Own relay not deployed. Preserve
  old paired routes/migrate both sides safely; changing one host route alone can
  strand existing remote clients. Shared relay transport is portable in principle.

## Website, environment and remaining family work

- secpar.win is LIVE, accounts on accounts.secpar.win. Site code in cloud/site.
  User rejected fluffy text. edb7f2d removes eyebrow slogans, redundant role prose,
  open-source essay and shortens headline/setup/download text. 56f003f tightens
  mobile headline size. Latest font change deployed, verify mobile two-line span
  after reload. Public downloads refresh on each published release.
- Browser had stale runtime `browser.user unavailable`; interpreter reset fixed
  it. Current CUA bindings secparSitePreview tab 10, siteBrowser browser 2. Docs
  read. Viewport reset, tab marked deliverable. Don't reselect browsers to fix
  stale tabs. Website screenshots outputs/ui-review/secpar-site-short-copy.jpg
  and -mobile.jpg (mobile shot before final font-size change; refresh it).
- C: filled mid-build. Relocated ONLY own build directories to D:, with junctions:
  host/src-tauri/target → D:/Secpar-CI/archived-original-host-target;
  app/build → D:/Secpar-CI/android-app-build. C: recovered ~10 GB. Zero-byte new
  account.rs and tracked core Cargo.lock were recovered before continuing.
  Keep builds/cache on D:, CARGO_INCREMENTAL=0, TEMP/TMP=D:/Secpar-CI/secpar-build-temp.
- Kotlin incremental cache became stale after relocation. A full nonincremental
  compile fixed the unresolved imports. Quote '-Pkotlin.incremental=false' in
  PowerShell; unquoted dot property was split. Do not delete caches blindly.
- Windows CI run 42206187371 failed at setup when C: was full; Linux/macOS passed.
  New CI runs should use recovered space. Don't rerun unchanged builds needlessly.
- Next after urgent clients/accounts: macOS native hosting, guest permission
  enforcement/revocation, desktop physical controller forwarding and Linux virtual
  gamepads, actual four-player testing, all nine desktop directions + Android.
  No new agents; previous agent quota exhausted. Debug/review builds stay local.
