# Active Secpar work — 2026-11-06

## CURRENT: supersedes historical statuses below

- Read docs/user-requirements.md and AGENTS.md. New prompts accumulate; no scope
  abandonment. User permits a few narrow subagents, with extreme token care.
- Windows 3.7.0 published AND installed, exit0, host/UI3.7.0, service running.
  User confirms Android video smooth again.135 host tests + controlled latency
  A/B + three actual concurrent hardware streams passed. Linux3.7 published too.
- Physical Android audio still fails after silence, pause/video switches AND
  sometimes after minutes without pause. No physical fix claimed. User is away
  and cannot test Linux. Phone at office, no longer reachable through adb.
- Public3.8 is RELEASED for Android/Linux at https://github.com/modnite/Secpar/releases/tag/v3.8.0.
  Adjacent ../secpar-stream-fix isolates release
  from incomplete main account-owned foundation; never ship main wholesale.
- Prepared Linux3.8: decoding/RGB conversion off UI, bounded queue/IDR recovery,
  real decoded-frame VA-API capability probe + runtime software fallback.26 Linux
  desktop tests passed (one vault skipped), actual native Wayland AppImage check
  passed. Optimized AppImage staged D:/Secpar-CI/publish-3.8.0; X250 GPU unverified.
- Android3.8: PcmTimeline keeps host capture timestamps independent of
  decoder PTS; bounded recent-low-RTT clock window replaces lifetime minimum.
  Extra diagnostics contain counters only. Final optimized signed APK published,
  24079999bytes, SHA256e989b09d869825fec2d79625a1d6d403de80d0f5ba9c574536674d3906599542.
  Same package/certificate, non-debuggable, no review/test payloads; five black
  launcher densities verified exact. Minified APK cold-launched on ONLY5556.
- Exact strict audio failure diagnosed: lazy codec initialization overflowed
  pending>12 queue (2–4 dropped encoded packets); ALL submitted samples decoded
  and written exactly, zero timeline misses/stale/incomplete. prepare() now
  initializes audio on executor before first PROBE, max2s; catch errors to retain
  video startup. Queue sizes unchanged. Original144000-sample-frame gate kept.
- Permanent audio death path found: exception handlers setdead forever. New
  AUDIO-ONLY reset/retry (50ms–1s backoff), fresh PCM timing/output/fades, preserves
  release as permanent stop. No video engine changes. Agent audio_gate implemented
  these owned files and strict silence, codec.stop fault +180s continuous tests.
- Combined Gradle unit/debug/androidTest build PASSED:217 units, zero failures.
  Agent audio_gate returned5556 after all5 real audio gates passed (194.111s):
  exact144000-frame conservation,250/1200/2500ms silence, actual MediaCodec.stop
  fault recovered in sameplayer,180s uninterrupted flow. Soak overflow0, failures0,
  timelineMisses0, incompleteWrites0; accepted progress/current age everysecond.
  Virtual no-audio sink reported859 underrun resets: NOT audible continuity proof.
  Do not tune S23 buffers from this artificial sink or claim physical success.
- Root input test passed after waiting for Compose layout before injecting taps.
  Video pacing passed. PiP dismissal exposed a missing final callback; root now
  ends once before stop/disposal and on final destroy, preserving PiP entry/return.
  New debug APK with endHiddenStream method confirmed installed. All four input,
  two PiP and one video-pacing checks pass after ONLY5556 reboot (its System UI
  hung on live display resize). UI fixture now render-only: brittle native
  accessibility-provider queries removed, no automated geometry claim. Clean
  checking/connected screenshots manually reviewed at1024x700 and600x420, all
  actions fit. outputs/ui-review/android-docked-{checking,connected}.png and
  android-600x420-{checking,connected}.png. Our emulator5556 now SHUT DOWN.
- Root Android pointer change is specifically dock/DeX, not Linux: free local OS
  pointer (no capture request/shortcut), absolute mapping only on video hit target
  in FreeMouse modifier, no Activity-wide mouse interception, overlays stay local,
  focus loss releases held input and focus return automatically forwards again.
  Touch trackpad fallback retained. Capture UI/Android shortcut setting removed.
- ClipboardSync was missing from stream screen; root added it. Agent
  clipboard_focus improved focused reads/resume comparison, first-focus baseline,
  no echo/UTF8 limits, subscribe-after-listener and cleanup; unit tests passed.
- HomeScreen compact DeX layout: stable checking/online/offline summary row at
  >=560dp,180–240dp thumbnail beside identity/action, max760dp content,3-column
  secondary modes. Phone max360dp thumbnail. User supplied giant checking preview
  screenshots; must render/check synthetic wide/narrow fixtures before release.
  New DesktopLayoutInstrumentedTest writes home-checking.png/home-connected.png
  in debug app files. New freeMouse routing/native-control tests await running.
- Main Android changes committed c51556c plus review-fixture df2c0d5, cherry-picked
  to release worktree ec7a1fd/1990786. Public tag v3.8.0 is ecfc96c. All105 Android
  production source files verified identical between main/release trees. Linux
  AppImage23386616bytes, SHA256aa935ab2421b0582f333cc995b9773184201a59ed9351be46cbc566f5f428f01.
  GitHub server digests match both artifacts and checksum file. Site refresh
  42262590998 passed; workflow explicitly checks out main to preserve site copy.
  Windows current3.7 fixes video; no unnecessary second installer this session.
- Pending async S23 validation: Android3.8 update via Settings, check recovery after
  pause/switch + minutes uninterrupted. Do not re-ask Linux testing while user has
  no access. Audio fixes are physically UNCONFIRMED until that answer. Broader owner
  accounts, Linux remote/native-host validation and macOS hosting remain required.
- Punctuation encoding corrected in desktop/lib/build/README, StreamScreen and
  this file after spotting Windows Python cp1252 default. ALWAYS read_text with
  encoding='utf-8', write_bytes UTF8. Apply same punctuation correction to release
  worktree and rebuild its Linux artifact if changed; source/artifact must match.

## Historical context below (superseded where noted above)

Keep working, solo. User wants completed releases, check-ins and actual screenshots.
Never lose the broader Windows/Linux/macOS host+client scope; Android client only.

## Latest delivery

- 3.6.0 is published at https://github.com/modnite/Secpar/releases/tag/v3.6.0.
  Source tag is commit 23dd11f on fix/shared-capture in adjacent worktree
  ../secpar-stream-fix. Main contains committed but unreleased account work; don't build its
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
- Windows 3.6.0 installation completed with exit 0; installed UI and host both
  read back as 3.6.0. User confirms simultaneous physical streams now connect.
  PERFORMANCE FAILED on Android and Linux even with one viewer; audio recovery
  still fails. UI is better. Linux branding also aliases badly when downscaled. User explicitly
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
