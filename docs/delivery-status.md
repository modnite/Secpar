# Secpar delivery record

## User preferences

- Deliver completed results and keep working through the agreed app-family scope.
- Give regular check-ins and actual interface screenshots.
- Compact, plain, beautiful interfaces like Parsec; minimal scrolling on small
  desktop displays. Review at 800×600 and 640×400 logical pixels, including scaling.
- Email/password accounts; no required passkeys or biometrics. Minimum password
  length is eight characters at the user's request.
- Android is client-only. Desktop should eventually combine hosting/connecting.
- Preserve existing remote access and side-by-side installation identities.

## Verified

- Android Preview 10: dock keyboard/mouse, capture escape, trackpad fallback,
  resolution changes and triangle branding. Physical dock input was user-confirmed.
- Side-by-side Windows Secpar Host, separate from PC Remote.
- Desktop development client released for Windows, Linux and both Mac architectures:
  encrypted transport, software video, audio and input. Hardware end-to-end validation
  is incomplete; builds are not proof of supported host/client combinations.
- Dedicated Windows/OMV runners and GitHub macOS build/test jobs.
- Account/friend API deployed at accounts.secpar.win, with D1 and persistent signing key.
- Email/password registration, verification email and sign-in **confirmed working by
  the user on 2026-11-05**. Resend sender uses accounts@mail.secpar.win.
- Browser form origin bug fixed with same-origin referrer policy; verified with a
  real browser. Account-service tests include owner-isolated block lists.

- Desktop Development 2 published: native account/friend management and secure
  last-pair storage, plus the compact Windows host installer. All four CI build/package
  targets passed (run 41985220035). Public download sizes/SHA-256 match the local
  verified artifacts; six direct asset links returned 200. Both Windows executables'
  embedded icon frames match the approved triangle. Native HTTPS reached the live
  account service with normal certificate validation.
- Compact host frontend Home/Settings measured zero main-panel overflow at 640×400
  with sample state. The QR enlargement dialog opens and closes with Escape. No
  installed host or active connection was replaced/restarted during the work.

- Host Home and Devices lists now scroll within their own area. Twelve-device sample
  review at 640×400 and 800×600 measured zero main-panel/horizontal overflow;
  keyboard navigation reached device 12 with main scroll remaining zero. Settings
  still has zero overflow at 640×400.
- Host-enrollment backend deployed successfully (run 42079405866); account CI passed
  (42079362570). Development 3 is published (see delivery links below).

- Update checks now recognize GitHub's expected “no stable release” response and
  offer development downloads, rather than reporting a connection failure. Silent
  updates still require a signed stable release; prereleases are not auto-installed.

- Real workerd/D1 enrollment regression test added to account CI and deployment
  checks. Test tools upgraded after an audit; all 22 tests/type checks/runtime
  smoke passed locally and npm audit reports zero vulnerabilities.

- Development 3 published with six verified public assets: Windows/Linux/Mac Intel
  clients built from 15903ea, ARM client cross-built from b4cb939 (same app/library
  source), and Windows host installer from a9fa7f9. All download HEADs returned 200;
  sizes/GitHub SHA-256 digests match local files. Windows CRT/icon checks and Linux/
  Mac archive, architecture, bundle-version/signature-resource checks passed.
- ARM fallback cross-build 42082328506 passed. Native ARM run 42079362595 failed
  before starting because GitHub could not acquire a runner (capacity annotations).
  Native Apple Silicon execution is still unverified. Normal CI now uses native
  Intel protocol tests plus an ARM cross build; dedicated native ARM checks remain
  available manually. All four normal CI targets passed in run 42082490182. Account CI including
  the workerd/D1 smoke passed again in run 42084030014.
- The published host is an unsigned debug installer and was not installed. Native
  host enrollment needs a real-user check with backup access available; do not
  interrupt the user's sole remote connection to perform that check.

- Visual polish pass completed and user-approved on 2026-11-05: desktop uses a shared control height, deliberate
  page insets, flat navigation underline, tonal/bordered sections and compact friend
  rows. Rendered at 640×400 and 800×600; 14 desktop tests passed. Android now has a
  two-column home layout for wide docked windows, denser mode controls, matching
  48dp Material buttons and consistent mint/orange accents. Android debug/release
  builds and 177 unit tests passed; portrait and wide emulator layout review passed after reducing the preview height to keep
  the primary action visible. Preview 11 retains the Preview 10 signing certificate;
  the debug-only sample activity is excluded from the release APK.

- Desktop Development 4 and Android Preview 11 are published. Desktop CI run
  42087534176 passed all four build/package targets. All seven public downloads
  returned 200; asset sizes/GitHub hashes match local files. Windows icon/static
  CRT, Linux/Mac archive/architecture, Mac bundle version 4 and signature-resource
  checks passed. Android 177 unit tests, release/debug builds and vital lint passed;
  the release certificate matches Preview 10 and the debug fixture is excluded.
- Android portrait, paired wide landscape and unpaired wide landscape samples were
  visually checked. Open screen, Scan code and normal mode controls remain visible.
  A separate Secpar-UI-Review emulator was created on D: (port 5556) after another
  app took over the shared emulator. The isolated emulator is stopped after review.
  No connected physical phone or installed PC host was updated/restarted.
- This UI release requires no PC host update. Android account/friend integration
  and broader physical DeX/window-layout validation remain pending; Preview 13 QR pairing was subsequently user-confirmed on the S23.

- Tray exit discoverability fix: the misleading "Hide from tray" action is now
  "Quit Secpar", separated from Open Secpar. Explicit app.exit(0) bypasses the
  close-window exit guard; the separate Windows hosting service keeps running.
  Host Preview 12 installer built successfully and published with matching
  asset hashes. No installed process/service was stopped to test this change.

- Android scanner startup fix (Preview 12): recovered the Galaxy S23 crash log
  and reproduced the identical exception in the signed Preview 11 APK on the
  isolated emulator. R8 removed the manifest-discovered ML Kit registrars' public
  constructors. Added a targeted constructor keep rule and a ZXing fallback for
  native detector initialization failures. The minified release now opens and
  analyzes camera frames, survives two reopen cycles, handles permission denial,
  and recovers after permission is granted. No native-init fallback or crash was
  logged during those checks; all 177 unit tests passed. Physical Galaxy S23
  scanning was not yet checked in Preview 12; Preview 13 pairing was subsequently user-confirmed. No physical phone or installed host updated.
  A release instrumentation attempt could not run because AndroidX Test calls a
  tracing method optimized out of the app; release verification used the actual
  UI/camera instead. No instrumentation-only keep rules are shipped.
  Preview 12 is published; both Android assets and both Host Preview 12 assets
  returned HTTP 200 with matching local/GitHub sizes and SHA-256 digests. The
  Android certificate matches Preview 11. The isolated emulator is stopped.

- Desktop unification milestone: Windows now has a Hosting tab
  controlling the same installed Secpar engine for Android and desktop clients.
  Pairing QR/link, paired-device removal, display/quality, autostart, file/clipboard
  toggles and direct-access settings are in the desktop UI. Account linking moves
  under Hosting → Settings. A combined installer embeds the desktop binary; the
  existing service/server modes, data directory and TLS identity are retained.
  Windows close-to-tray, Open/Quit and single-instance reopening are implemented.
  The native file/folder pickers and network diagnostics still need migration.
- Android Preview 13 scanner changes: use native potential-code bounds for cropped
  fallback decoding (including camera rotation), remove frame-count-driven zoom
  jumps, and request a focused full-resolution still when preview detection cannot
  decode text. Cameras unable to bind three streams retain preview scanning.
  On the isolated emulator, a real CameraX camera-image fixture was decoded and
  its exact 437-byte link returned to the caller (camera handoff test passed).
  The test intercepts the result and never connects to the fixture host. Earlier
  attempts failed because the emulator image was clipped at the right edge;
  correcting the fixture framing made the camera handoff pass. Native still
  capture callbacks were exercised; the user subsequently confirmed Preview 13
  built-in scanning and pairing on the physical S23.
  179 Android unit tests and 15 desktop tests passed (credential-store test ignored).
  Read-only get_state/get_config calls against the installed host succeeded; no
  OTP was generated, configuration changed, or service restarted by these checks.

- Desktop Development 5 now publishes ONLY Secpar-Windows-x64-setup.exe plus its
  checksum. The separate standalone Windows download and client-only Linux/Mac
  downloads were removed from this release after the user's clarification.
  There must be ONE application/package per desktop OS with both roles; Android
  stays client-only and connects to those same desktop hosts. Background service
  binaries are internal implementation details, not separate user-facing apps.
  All four platform CI jobs passed in run 42105758113, including the ARM cross-build. Those role-limited Linux/Mac artifacts stay internal
  until their hosting implementations are available. The Windows installer was
  unpacked and checked, never installed; no active service was restarted. Static
  CRT/x64 PE checks and seven host command tests passed.
- Android Preview 13 is public; both assets return HTTP 200 with matching digests.
  Release certificate matches Preview 12; debug layout activity is excluded. The
  camera handoff and original/rotated host-QR instrumented tests both passed on the
  isolated emulator. On 2026-11-05, the user confirmed that Preview 13 reads
  a fresh QR code and completes pairing on the Galaxy S23. The built-in scanner
  path is now physically verified; no external Camera app is required for this check.

- Android video pacing work (Preview 14): the user reports periodic video slowdown
  and catch-up while audio stays synchronized, with Moonlight on the laptop smooth.
  Read-only S23 logs show c2.qti.hevc.decoder.low_latency at 1920×1080/120fps.
  Identified that every decoded picture was immediately sent to the surface and
  overflow discarded compressed frames without gating dependent P frames until
  a new keyframe. Added a two-picture vsync-paced output queue, a time-bounded input
  queue with keyframe recovery, display frame-rate hints and a realistic codec
  operating-rate hint. Audio and the host engine are unchanged. Eight new queue
  tests pass (187 total), and a real MediaCodec/SurfaceView instrumented test feeds
  120fps synthetic input and verifies the decoded blue picture via PixelCopy.
  This validates the path, not real S23 smoothness; physical comparison is pending.
- Test topology clarified: one Windows PC; CachyOS laptop is the Linux test target.
  OMV is not a streaming target. Linux hosting is the next desktop priority, and
  macOS also needs both roles in its single application. Do not publish separate
  client-only desktop products to meet this request.

- Android Preview 14 published from dd091f6. Both public assets return HTTP 200
  with matching local/GitHub SHA-256 digests and sizes. Certificate matches Preview
  13; both debug review activities and the synthetic video fixture are excluded
  from the release APK. Final builds/vital lint and all 187 unit tests passed; the
  real decoder/SurfaceView/PixelCopy test passed again after diagnostics were added.
  No physical phone installation or installed PC service change was performed.

- Preview 14 physical result: user reports unchanged or worse video slowdowns.
  S23 logs show 8–59 presented fps, stable 11–14ms local decode/submission time,
  almost no output drops and only one startup input resync. This does not support
  decoder queue buildup as the main reported failure. Direct UDP was confirmed.
- Preview 15 work: found that Receiver shares a Crypt instance between its receive
  thread and synchronized sender, but both cipher directions reused ONE nonce
  scratch array. A full-duplex regression reproduced 29 authentication failures
  in 40,000 packet checks before the fix; independent send/receive buffers give
  zero failures. All 188 unit tests pass. A device-provider instrumented test on
  the isolated emulator also passed all 12,000 video/input packet checks. This is
  a proven transport defect; its contribution to the S23 slowdown still needs a
  real-stream comparison. Added receive diagnostics for frame rate, throughput,
  loss, frame-assembly drops, invalid packets, processing wall-time peak and socket
  backlog/drop counts. Audio code and the PC host engine remain unchanged.
- Android Preview 15 published from e489fc6: APK 24,008,327 bytes, SHA-256
  95f92d9cc1c8b92b25c008e2e1170724f2b75f3c34a9f3c58b216b4b621652bf,
  matching GitHub asset digest. The isolated Secpar emulator was stopped after
  verification; the unrelated emulator and physical phone were left untouched.
  Physical S23 result: unchanged or worse. Both video and cursor problems go away
  when playback is PAUSED; the cursor itself does NOT disappear. User corrected
  that misinterpretation explicitly. P15 logs confirm direct LAN UDP to
  192.168.100.130, 4–11 received fps under playback, zero assembly/invalid/socket
  drops, zero measured loss. Capture/encoding/packetization/send path now under
  investigation. Do not ship another decoder-pacing guess.
- Desktop Development 6 / Android Preview 16 implementation: automatic checks of
  product-specific preview releases, explicit download/install UI, SHA-256/size
  verification and Android package/version/certificate checks. Unified Windows
  packages disable the legacy service's unattended timer. No live installation was
  modified. Android has 190 passing unit tests and an emulator activity smoke;
  desktop has 19 passing tests, including corrupted-download rejection. Native
  install/relaunch and physical S23 performance remain user checks.
- Confirmed host packetization bottleneck: fresh ReedSolomon matrices were built
  synchronously per block/frame on the same video/cursor-polling thread. The exact
  host debug profile reproduced 71.466 ms/64 KiB and 237.353 ms/192 KiB. A bounded
  matrix cache plus optimized secpar-stream lowers those to 1.631 and 5.391 ms.
  Nine shared-stream tests pass, including byte compatibility and concurrent loss
  recovery. Fourteen agent tests pass; seven session tests passed and one tight
  timeout test failed during concurrent builds, then passed alone. No capture or
  real service restart was used. Details: docs/video-pacing.md and docs/updates.md.
- Windows packaging first hit low C: disk space. No cache was deleted: automatic
  review rejected that cleanup. Builds continue with a copied compiler cache and
  temporary output on D:/Secpar-CI/secpar-host-updates-target. Originals retained.
- Published Windows Development 6 from f30d1dd: one installer, 9,825,691 bytes,
  SHA-256 60316dd65ee435292bcbdab3bc596d8c7e096e94a68fb3285f50332081da0528.
  Packaged desktop bytes match the built x64 UI; host bytes match after Tauri's
  expected UNK→NSS installer marker patch. Executable code sections match.
- Published Android Preview 16 from ddbf61d: APK 24,008,155 bytes, SHA-256
  96d53836ecba5c9b4c7eb26e9d416b4a8f4b97aba0ded141b71e03720cb3c7aa.
  Version code 516, same signing certificate as prior previews, no test video
  fixture included. All 190 tests pass; release/vital lint pass. Final update UI
  smoke passes on phone and 800×600 isolated emulator layouts, with screenshots
  under outputs/ui-review. Four public asset links return 200; GitHub digests
  match local checksums. All four CI jobs pass in run 42135559751, including native Intel macOS and ARM cross-build.
  Physical S23 host-fix test requested asynchronously. Do not install/restart the
  live Windows service or physical phone to force that result.
- User confirms Desktop Development 6: video and cursor are perfect on the S23.
  Rare slight audio crackle/slowdown remains and is absent in Moonlight. This is
  treated as a Secpar audio defect, not a general device/network limitation.
- Android Preview 17 work corrects ignored/partial AudioTrack writes, false frame
  accounting and dropped MediaCodec input when busy. It primes 30 ms, preserves
  waveform samples under backpressure, drains delayed decoder output, and adds
  underrun/queue/sequence-gap diagnostics. No video behavior or PC audio code changes.
  The real Android Opus/AudioTrack fixture test preserves all 144,000 sample frames.
- Desktop Development 7 work places Check for updates / Download update on one row.
  Download remains visible and greyed out without an available update, while busy,
  or once downloaded. Both states reviewed at 640×400 with native screenshots.
- Published Android Preview 17 and Desktop Development 7 from 22f9fff. APK:
  24,024,539 bytes, SHA-256
  49938d047f2760672a5134b46d18bbdbf4fd510528d7b40390faef4749cac485.
  Windows single installer: 9,829,835 bytes, SHA-256
  00be8f68b5baff14e8a61619a5b2d501fe85fdf11ee5f31801eb606201a7f9e4.
  Both GitHub digests match local files; direct APK/installer links return 200.
  APK retains the signing certificate; synthetic audio/video test assets excluded.
  Packaged desktop matches the built UI, and the host matches with the expected
  Tauri installer marker. All 195 unit tests and vital release lint pass. Final
  Android Opus/AudioTrack smoke again preserves all 144,000 sample frames. Desktop
  tests: 19 passed. Windows/Linux CI pass in run 42142493406; Mac jobs pending.
  User asked to test Preview 17 via the new in-app updater. The physical phone and
  installed Windows service have not been modified by tools.
- Linux port checkpoint is on branch linux-host, commit 7135571, in the adjacent
  worktree work/secpar-linux-host. Shared host protocol/control code passes eight
  portable wire-model tests and 14 Windows TLS/pairing/relay tests. Native Linux
  capture/input and Linux runtime checks are still pending. Merge main's subsequent
  Android hotfixes before integrating this branch. No Linux/Mac client-only package
  was published. Main’s later video, updater and audio fixes must be merged before
  integration; work was set aside for Android streaming/audio fixes.

## Active implementation

1. Native Windows desktop sign-in **confirmed by the user on 2026-11-05**: the account appears after browser approval in Desktop Development 2. Friend actions with another real account and restart reconnect are still unverified.
2. Cryptographic host enrollment, restricted machine credentials, local Windows host linking/unlinking and heartbeat are implemented. Verification includes 22 API tests, Rust/Jose signature interoperability, a local Cloudflare/D1 runtime smoke, 14 desktop tests and four host credential tests. Not yet validated against an installed host; friend streaming remains disabled pending host permission enforcement.
3. Compact desktop layouts reviewed with native-rendered screenshots at 800×600 and 640×400. Host pairing/settings reviewed in the actual frontend with sample state; settings now has zero scrolling at 640×400.
4. Last paired-computer persistence is implemented; the Windows credential-store roundtrip passed using a disposable entry. Native Windows sign-in is confirmed; cross-process reconnect and Linux/macOS native sign-in still need real-user checks.

Guest-grant signature verification and an atomic replay guard are implemented as
an isolated host module, with a public Jose interoperability vector. Four tests
pass, including simultaneous replay, wrong host/expiry, token-supplied keys,
algorithm confusion, altered signatures and forbidden permission claims. They do not
yet admit guests. The remaining security boundary is recorded in
`docs/guest-access.md`; video and both TCP/UDP input must enforce the same policy.

## Still required

- Full saved-computer list, real host/client validation and packaged updates.
- Android account/friend integration. Windows host enrollment/presence are implemented but still need an installed-host user check.
- Broader Android/DeX layout and lifecycle checks; built-in Galaxy S23 QR pairing is user-confirmed in Preview 13.
- Physical high-DPI reviews on Windows, Linux and macOS; current compact layout checks use controlled logical viewport sizes.
- Guest admission enforced by host (separate from owner tokens), permissions and live revocation.
- Complete desktop unification on all platforms; Windows UI/service package is built and published; native installation remains unverified.
- Linux capture/input/encoding host; macOS capture/input/encoding host.
- Hardware decoding, audio/video timing, IME/full keyboard handling and routing benchmarks.
- Concurrent streams, encode fan-out, independent keys/permissions, controller seats.
- Test all nine desktop host/client combinations plus Android-to-each-host, LAN and remote.
- Signed distribution and stable releases only after the corresponding validation.

Own relay at relay.secpar.win is reserved, not deployed or migrated into current apps.
Current remote profiles may still use the original relay; changing one side alone can
strand existing connections. LiveKit was discussed, not selected as a replacement.

## Delivery links

- Repository: https://github.com/modnite/Secpar
- Desktop development 7: https://github.com/modnite/Secpar/releases/tag/v0.1.0-secpar-desktop-dev.7
- Previous desktop development 6: https://github.com/modnite/Secpar/releases/tag/v0.1.0-secpar-desktop-dev.6
- Previous desktop development 5: https://github.com/modnite/Secpar/releases/tag/v0.1.0-secpar-desktop-dev.5
- Android Preview 17: https://github.com/modnite/Secpar/releases/tag/v3.0.3-secpar-preview.17
- Previous Android Preview 16: https://github.com/modnite/Secpar/releases/tag/v3.0.3-secpar-preview.16
- Previous Android Preview 15: https://github.com/modnite/Secpar/releases/tag/v3.0.3-secpar-preview.15
- Previous Android Preview 14: https://github.com/modnite/Secpar/releases/tag/v3.0.3-secpar-preview.14
- Previous Android Preview 13: https://github.com/modnite/Secpar/releases/tag/v3.0.3-secpar-preview.13
- Previous desktop development 4: https://github.com/modnite/Secpar/releases/tag/v0.1.0-secpar-desktop-dev.4
- Android Preview 12: https://github.com/modnite/Secpar/releases/tag/v3.0.3-secpar-preview.12
- Host Preview 12 (tray Quit): https://github.com/modnite/Secpar/releases/tag/v3.0.3-secpar-host-preview.12
- Previous desktop development 3: https://github.com/modnite/Secpar/releases/tag/v0.1.0-secpar-desktop-dev.3
- Previous desktop development 2: https://github.com/modnite/Secpar/releases/tag/v0.1.0-secpar-desktop-dev.2
- Android/host Preview 10: https://github.com/modnite/Secpar/releases/tag/v3.0.3-secpar-preview.10
- Account sign-in: https://accounts.secpar.win/auth/sign-in

This list is a work record, not a claim that the app family is complete.
