# Secpar delivery record

## CURRENT — 3.11.1 PUBLISHED, 2026-11-07

https://github.com/modnite/Secpar/releases/tag/v3.11.1, source/tag669baef.
Original Lucide0.468.0 icons now replace hand-drawn menu symbols on desktop and
Material menu symbols on Android. Desktop PNGs use DPI-aware Lanczos filtering;
Android uses generated vectors preserving SVG geometry. Pin and ISC license in
assets/icons/lucide; notices included in APK, Windows/Linux packages and desktop
README for internal Mac archives. Corner button stays above Android backdrop;
real injected-mouse open/close/disconnect test PASS. Native640x400 screenshot and
Android vector menu screenshot reviewed. Linux optimized AppImage renders via
Wayland. Android R8/non-debuggable code528, unchanged certificate. Streaming/audio
engines unchanged in this release. All4 CI42367431921 jobs PASS. Live website
releases.json points to3.11.1 for all platforms; download digests match staging.
Windows still installed3.11.0 with active streams; do not interrupt for glyph patch.

Assets:
- Windows6,643,672 bytes SHA f5504167952528bd81633768ef02627d6aec35285b04d364e02c9eda58b0fc3d
- Linux22,645,240 bytes SHA0c62972ec17aa5b07693391e8e7d20299572e8bda56f01753615d11e29af4f4a
- Android24,128,404 bytes SHA6c413f314cc0d0e80c3b968c7407ffa62774e1ae7024b4d254cba67763ff6a2e
Windows installer UI payload matches host/src-tauri/unified/secpar-desktop.exe;
CI shares D:/Secpar-CI/target and overwrites that cache, so do not compare later
cached binaries against staged payloads. Use isolated release targets next time.

## IN PROGRESS — client backpressure/audio independence, after3.11.1

User reports1920x1080/60 Direct, RTT3ms,0.8Mbps,21.9 received fps, actual label
software(VA-API too slow). It is fallback software, not hardware currently working.
While typing, stream dropped: no authenticated stream packets reached client.
Host protected log read with user approval, filtered/address-redacted locally in
D:/Secpar-CI/secpar-host-stream-diagnostics.log. No service restart/settings changes.
Historical logs show RTT spikes/bitrate cuts but do not prove current outage cause.

Reproduced independent bug with real pinned TLS/WebSocket/encrypted UDP fixture:
pausing UI consumption fills8-event channel and stops authenticated keepalives.
Test failed on release code, passes on candidate. New bounded/coalesced critical
UI state and discardable telemetry/media avoid transport backpressure; missing
compressed video resumes only at a new IDR. Client retries candidate routes after
2s silence and distinguishes established-stream packet loss from initial failure.

Candidate audio delivery uses nonblocking Options.audio_sink callback + dedicated
desktop worker owning the UNCHANGED Audio engine. Audio bypasses UI event queue;
retry swaps output route, new sessions get independent routes, logout clears it.
Stale fallback UI/audio-worker packets expire at200ms; bounded16-packet input.
12 client tests pass incl UI-paused probes, continuous callback audio and IDR
recovery. Windows32 desktop pass/1 ignored; native Linux36 pass/5 ignored. New real
Pulse generated-audio worker/silence gate in progress. NOT released/installed.
Native VA/EGL rendering not implemented; remains separate priority (linux-rendering.md).


## IN PROGRESS — glyphs and Linux native rendering, 2026-11-07

User confirms Linux hardware performance still below Moonlight. Do NOT call3.11
hardware fallback a performance fix. Current VA path downloads frames, CPU-converts
RGB and uploads an egui texture. Investigating native VA-surface/EGL presentation.
Icon rule persisted in AGENTS/user-requirements: always established library assets,
never improvised text symbols or painter line art. Menus switched to original
Lucide0.468.0 SVG artwork, commitf12b0de177fbc2a6795e99be065887e72b237123,
ISC notice retained. Android and desktop include DPI-aware raster derivatives.
Code compiles on Windows; Android build and actual visual checks in progress.
Latest Mac CI42365335046 ALL FOUR jobs PASS (including both Mac package builds).
Windows3.11 installed/user has an active stream; no restarts for these changes.

## CURRENT — 3.11.0 PUBLISHED, 2026-11-07

Public https://github.com/modnite/Secpar/releases/tag/v3.11.0. Runtime836c4c0;
tag/release branch c8c46ce adds TEST-ONLY scheduling fixes. One Windows unified
installer6,631,410 bytes, one Linux unified AppImage22,608,376 bytes, Android
APK24,112,203 bytes (code527), checksums published. Optimized/stripped packages;
no review fixtures/PDBs. Installer UI payload matches build; host matches except
expected Tauri UNK→NSS bundle marker. Android keeps the same signing certificate.

SHA256:
- Windows46f51281e50aa68fd503f0d9204550bf6d3e4bc0392cf75e7b4c569dc932879b
- Linux1221cc689d8ecd3c4ec7daf4f8298b4c3e7daf77ca271b70f027819469c07bde
- Android ebfd8cc39e5858c50ada06d5b3382048902b9c8a022b430921d500814f082e70

Website release deployment42352132073 PASS; live releases.json advertises3.11.0
for all three platforms with matching digests. Help live at secpar.win/help/;
desktop and390px mobile layouts visually checked. Actual menu screenshots at
outputs/ui-review/stream-menu-640x400.png and android-stream-menu-3.11.png.

Final Linux package passes native Wayland rendering, three real generated-audio
output/silence/ALSA-fallback gates, monitor discovery with external tools excluded,
Wayland clipboard exchange, and real delayed native decode→software recovery.
Android release cold-launch passes on owned5556. Latest13 Android instrumentation
checks PASS. User S23 and X250 acceptance of NEW changes remains pending.

CI42351875638: Windows/Linux PASS; Mac tests/builds PASS but packaging FAIL (below).
Earlier run exposed test-order assumptions: TCP clipboard/control may arrive after
UDP video; account fixtures must initialize their own TLS provider. Corrected in
c8c46ce; no production changes/rebuild needed. Windows core26/client10 also pass.

Windows3.11.0 installed after user approved administrator prompt. Installed UI
and host hashes exactly match verified installer payloads. Secpar service RUNNING,
IPC responds, all4 paired devices preserved,1 device now connected. Desktop app
is running. Do not restart/replace it for further verification.
Mac CI tests and optimized x64/arm64 compilation passed; packaging failed because
portable workflow omitted SECPAR_RELEASE_TAG after build.rs became version-driven.
Workflow now reads Cargo version into GITHUB_ENV for BOTH compile and packaging.
No Mac package published; fixed workflow still needs a CI run.
Next acceptance: Automatic decoder on X250 (Software confirmed smooth workaround),
Windows↔Linux text clipboard/Ctrl+V, Windows→KDE host, local corner menu interaction.
Do not repeat accepted3.10.1 audio/alignment checks. Android engines untouched.


## IN PROGRESS — 3.11.0 candidates, 2026-11-07

User wants ONE top-left translucent stream menu in WINDOWED AND FULLSCREEN on
all clients. Desktop streaming toolbar and Android right-edge handle removed;
Android More keys contains shortcut/media submenus; Disconnect is the final red row.
Flat icon rows and submenus follow the reviewed Parsec reference. Optional stats
float separately: negotiated resolution/FPS, actual decoder, route and measured
RTT/receive bitrate/FPS. Display/fullscreen, clipboard and Audio live in the menu.
Ctrl+C/X/V were intercepted by egui as Copy/Cut/Paste; new input handling forwards
remote chords. Text clipboard now uses a separate secpar-clipboard worker on both
desktop clients and Linux host; OS operations never run on video/UI thread. First
clipboard is seeded, not exported; remote writes suppress echoes;1MB limit. Existing
Linux profiles migrate clipboard support once, later explicit opt-out preserved.
Windows/Linux real control+UDP integration proves bidirectional clipboard and
optional host audio errors do not kill video. Persistent connection errors with
Copy error replace silent list return. Linux host capture errors retain encoder/
plugin details; async audio errors no longer spin forever. Native monitor discovery
replaces pactl dependency and never falls back to microphone; isolated native
monitor gate passes with PATH excluding all external tools.

IMPORTANT new Linux3.10.1 regression feedback: ~2fps video remains choppy with
audio OFF. User confirms SOFTWARE decoding RESTORES smooth playback. Hardware
sample probe proved only compatibility. Added sustained actual-frame-budget native
decode fallback using negotiated FPS; real delayed GStreamer native pipeline test
recovers at fresh software keyframe. Physical Automatic fallback still unverified.
Do not undo accepted desktop-service audio or Android alignment baseline.

Gates: latest flat-list Android menu passes13 instrumentation checks, including
injected dock mouse clicks, PiP, return alignment and video surface pacing;
252 JVM checks passed before the UI-only revision. Windows32 pass/1 keychain
ignored. Native Linux desktop36 pass/4 environment checks ignored by default;
host17 pass/1 native-monitor check explicitly passes separately. Transport10 pass.
Actual isolated Sway/Wayland clipboard Unicode/multiline/echo check passes.
Desktop640x400 and Android1024x700 screenshots reviewed; no clipped main-menu
controls. All packages versioned3.11.0, Android code527. Production packages and
final bundled-runtime gates are next. No public3.11 release/install yet.
Original Secpar3.9 Windows service remains running; preserve connected sessions.

Research completed: Parsec overlay/stats, immersive mode, hotkeys, connectivity,
feature matrix; original Secpar help in cloud/site/public/help, linked from menus
and site. docs/writing-style.md + AGENTS persist anti-fluff writing requirements.
No copied articles/screenshots or fake metrics. Claude remains macOS owner.
Linux remote relay connector/media, friends/controllers remain later scope.

## UNRELEASED — shared relay control, after accepted3.10.1

Added secpar_host_core::agent::relay::serve: Worker envelopes to real authenticated
agent sessions,16 pending/authenticated control peers, per-peer bounded queues,
16MiB global inbound/outbound queued-byte budgets,5s socket write timeout,
heartbeat/disconnect handling and cancellation cleanup. Session close shares the
reply queue so it cannot overtake an auth error. Revocation remains per device.
Shared session registry now unregisters via RAII even when its future is aborted.
Five new in-memory transport tests exercise actual core auth/session lifecycle,
revocation isolation, stalled-writer cancellation, socket EOF, framing/budget and
pending-peer limits. Core26 tests PASS on Windows AND native Linux. No real Worker
connection/TLS/NAT tested here; no platform activation and no release/install.
NEXT: Linux relay registration/reconnect connector, owner/manual relay bootstrap
links and media STUN/TURN routing, preserving existing device auth and3.10.1 engines.
Claude may reuse the shared mux for macOS; coordinate shared-file integration.

## ACCEPTED BASELINE — 3.10.1, 2026-11-07

User physically confirms BOTH fixes: Linux stream audio plays; Android Home stays
aligned after returning from YouTube with PiP. These are USER-ACCEPTED, not pending.
Preserve3.10.1 Android engines/window restoration and Linux desktop-service output.
No repeat acceptance questions. Next: Linux remote HOST relay/STUN/TURN, followed
by physical controller forwarding and remaining friends/guest/multiplayer scope.
Windows3.9 host remains installed; do not restart it while connected. Claude owns
macOS native host in its separate checkout. New prompts accumulate.

## CURRENT — 3.10.1 PUBLISHED, 2026-11-07

- Website release refresh42343540816 passed. User asked for3.10.1 Linux audio
  and Android return-layout acceptance; await replies without repeating questions.
- Public https://github.com/modnite/Secpar/releases/tag/v3.10.1, source3505eee;
  tag/branch release/3.10.1 pushed. Android APK + one Linux host/client AppImage,
  stripped optimized builds only. Public GitHub asset digests matched staging.
- Android code526, unchanged signing identity, R8/non-debuggable; final APK embeds
  source3505eee metadata.12 instrumentation +252 JVM tests pass; release cold-launch
  checked on owned5556. Physical S23/YouTube asymmetric-return acceptance pending.
- Linux now uses desktop PulseAudio/PipeWire service FIRST, ALSA fallback. User's
  exact3.10 error was "no usable audio output format found" and Retry stayed silent.
  Linux32 unit checks pass/4 environment cases ignored by default; all3 output
  gates explicitly pass with final AppImage libraries. Monitor proves generated
  audio before/after600ms silence with broken ALSA, plus ALSA compatibility path.
  Unavailable desktop service -> synthetic ALSA null fallback also verified.
  Final optimized AppImage launches/renders via native Wayland/EGL. Physical CachyOS
  audio remains pending. Windows30 unit checks pass/1 keychain ignored.
- No new Windows installer this release. Existing Windows3.9 host left running;
  Android/Linux clients remain compatible. Android audio/video, desktop video,
  network and Windows host streaming engines unchanged. Latest remote-host auth
  groundwork remains in source but Linux relay/anywhere hosting is not enabled.
- Stage D:/Secpar-CI/publish-3.10.1. Exact SHA256:
  Secpar.apk: 8a051ce99e6d45badbadb99f90fda0ae3dee757894fc2055ecb4ebc3c3829060
  Secpar-Linux-x64.AppImage: f1869ca708439ab93a15cb41a6075171275d4d727e7f239be56e5e95b583579c

## CURRENT — 3.10.1 candidates, 2026-11-07

- User rejects3.10 Linux audio: exact error "no usable audio output format found";
  Retry remains silent. Default CPAL Linux backend is ALSA, not KDE's selected
  desktop service. New Linux output uses GStreamer pulsesink via PulseAudio or
  PipeWire's Pulse-compatible service FIRST; ALSA compatibility is fallback.
  Existing video, network, Android audio and Windows host engines unchanged.
- Real isolated tests: broken ALSA default + live desktop service renders generated
  Opus before/after600ms silence (96000 frames,0 decode errors); unavailable Pulse
  service falls back to synthetic ALSA null PCM. Separate real ALSA/Pulse monitor
  gate still passes. New script verifies requested tests exist (version changes
  change executable hash; NEVER accept an exact filter running0 tests).
- Linux32 unit checks pass/4 environment gates ignored by default; all3 relevant
  output gates explicitly passed separately. No physical laptop acceptance yet.
- Android user reports Home shifted right and header under status bar on return
  with YouTube PiP; force-stop fixes. Refresh actual insets on resume/focus/config/
  PiP exit and restore system bars only outside immersive stream. Explicit stream
  window ownership prevents normal-home repair from showing bars during streaming.
- Baseline3.10 fails lingering-immersive-bars resume check; candidate passes.
  Synthetic stale inset injection recovers on BOTH baseline and candidate, so it
  is not proof of exact Samsung side-offset root cause.12 layout/nav/PiP/input/video
  instrumentation cases +252 JVM tests pass; portrait checking/online render saved
  outputs/ui-review/home-insets-{checking,connected}-1080x2340.png. Samsung acceptance
  remains pending. Lean Android3.10.1/code526 built/signed/cold-launch verified on
  owned5556 only; phone untouched. Keep debug local. Release not published yet.

## CURRENT — 3.10.0 published; physical acceptance pending

- FOLLOW-UP after3.10 (not released): shared secpar-network::relay_auth now matches
  Worker/Windows HMAC vectors. Linux ticket uses ticket:<pc>:<device> with existing
  master and device auth retained. Windows subset5 and actual Linux13 host tests
  pass; network9 pass/1 live-STUN ignored. No relay enabled or engine changed yet.
  Linux backend wiring, registration, bounded transport and NAT/media remain next.
- Public release: https://github.com/modnite/Secpar/releases/tag/v3.10.0,
  tag/source929ec17, branch release/3.10.0. Website refresh42319008952 passed.
- Desktop: windowed free native pointer, capture only in fullscreen, Alt+F toggle;
  local controls and popup layers excluded from remote input. Video engine unchanged.
- Linux audio: AppImage uses host ALSA/Pulse/PipeWire libraries; all native PCM
  formats supported, visible output failure and Retry audio without reconnect.
  Isolated Arch test reproduces old package's missing ALSA plugin; candidate sends
  actual decoded Opus through native output to a synthetic Pulse sink/monitor.
  96000 input frames,0 dropped,85306 audible; monitor confirms nonzero audio.
  Virtual fixture uses norewinds=1 to avoid null sink's artificial2-second blocks;
  production output buffering was NOT retuned. Physical CachyOS audio not accepted yet.
- Android: reproduced double navigation pop from retained outgoing stream callback,
  leaving empty NavHost. Guard exact entry identity and provide Home fallback.
  252 JVM tests and10 navigation/PiP/input/video instrumentation tests pass; optimized
  APK cold-launch/background-return smoke passes. Exact S23/YouTube trigger and
  long-duration recovery still require user acceptance. Streaming engines unchanged.
- Windows29 desktop checks pass/1 keychain ignored; final extra real egui video-layer
  versus foreground-popup hit-test passed separately. Linux32 desktop checks pass/
  2 ignored, plus real native audio output and Wayland launch checks.
- Optimized packages verified against public asset digests. APK code525, signing
  unchanged, non-debuggable/R8, no review fixtures. Windows stripped/no debug sections.
  Stage D:/Secpar-CI/publish-3.10.0. SHA256:
  APK17b9375c12871360380e115e023a178fadd7b53f931748c342a388246b4d525b;
  Linux3f6c2938bc8954b6f34921b8588a2024756770b8df55632e66cb30df35fbb82d;
  Windows1631e72bc810d09f312c69ec280542b0a0f5269135fe3b2fad3d023798dd8e81.
- Windows STILL INSTALLED3.9.0, account flow physically accepted. One active session
  observed; do NOT install/restart3.10 while connected. Android/Linux3.10 work with
  host3.9. User was asked for Linux audio/free-pointer/Alt+F and Android YouTube/PiP
  return acceptance; last async tool output truncated, avoid immediately repeating.
- Screenshot: outputs/ui-review/client-settings-640x400.png, actual native Linux
  renderer in isolated synthetic environment, visually reviewed and shown to user.
- Claude owns separate macOS checkout; shared fixes now committed, coordinate before
  integration. Linux remote HOST relay/STUN/TURN remains unimplemented and next.
- Controller support: Windows virtual Xbox/per-device isolation and Android touch
  gamepad exist. Physical USB/Bluetooth forwarding on both clients, Linux/macOS
  virtual pads and real4-player/rumble tests remain unfinished. Three stream slots
  do not establish complete controller multiplayer.

## Verified: Windows installed and account flow accepted

Windows3.9 installer exit0; installed host/UI versions and hashes match staged
release payloads, Secpar service running. Desktop launched and automatically
linked the host: account state linked/owner-present/online with no error. User
physically confirms Android3.9 sign-in shows the Windows PC and connects without
QR. Account milestone is accepted end to end. Next is Linux outside-LAN hosting;
preserve working Windows/Android engines and Claude's separate macOS scope.

## Latest: 3.9.0 published, Windows approval pending

Source99b8762 and branch release/3.9.0 published with optimized Android/Windows/Linux
packages and verified checksums. Website refresh42308784762 passed. Account-owned
desktop enrollment/access and Android integration included.252 Android units,
five final audio gates, actual final Android/native-host owner TLS/reconnect/logout,
optimized APK live approval-start/cancel and native Linux Wayland launch pass.
Bridge account HTTP is scripted; live account-holder approval remains pending.
Linux remote hosting and Mac hosting remain incomplete. Working engine source is
unchanged from the approved Android3.8/Windows3.7 baseline.

Windows3.9 installer is awaiting actual OS UAC approval (session41501); user asked
asynchronously after consent.exe confirmed. Do not claim installed yet. Before
launch0 active Secpar devices checked twice; original PC Remote/Sunshine stopped
state preserved. Phone and5554 untouched.5556 remains running after automatic
review blocked a combined notes-save/shutdown command ('blocked by policy').
Use docs/next-milestone.md NEWEST entry for exact artifact checksums and next steps.

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
- Preview 17 physical result: occasional hiccups produce permanent music lag;
  video stays synchronized and Moonlight does not hiccup on the same desktop.
  Read-only S23 logs show 60–70 ms queued PCM, up to eight pending packets, 15
  underruns, six overflow packets and six observed audio gaps. No short writes or
  decoder input waits in that session. Do not claim Preview 17 fixed audio.
- Preview 18 work uses host capture timestamps and PRS clock conversion, rejects
  audio older than 40 ms, flushes projected latency over 80 ms, and resets/reprimes
  on real underruns. Five-ms restart fades and gap crossfades soften transitions;
  full Opus PLC is not implemented. Android tests cover a real 180 ms starvation,
  ten delayed packets and return to live audio. Both device tests pass; physical
  S23 comparison is pending. No video or PC engine changes.
- Published Preview 18 from f0d6dde, version code 518. APK 24,024,539 bytes,
  SHA-256 834a38bd267bf4e55f7308c5c073e9a0918b1795d77f1a2a8dc55ac561f7e37d,
  matching the GitHub digest; both release assets return HTTP 200. Same signing
  certificate and no synthetic test assets in release. Release/vital lint and all
  202 unit tests pass. Both Android audio tests pass, including actual starvation,
  ten stale packets and live recovery. Published P17 regression notice pointing to
  P18. Physical audio comparison requested asynchronously. The Secpar emulator was
  stopped after tests; physical phone and installed host left untouched.
- Physical S23 confirmation: Preview 18 audio is "In sync and clean" after the
  hiccup comparison. Windows Development 6 video/cursor and Android Preview 18
  audio are now user-verified. Preserve these working paths while continuing
  Linux/macOS hosting. This does not validate all devices or network conditions.
- CachyOS laptop is confirmed KDE Plasma on Wayland. Linux branch has merged
  main through a7233eb, merge checkpoint 93ed978, and all eight portable core tests
  pass. Branch is pushed; native capture/input and Linux runtime validation remain.
  Work temporarily switches to the requested icon polish.
- Icon update: transparent triangle for desktop PNG/ICO/window/tray/installer;
  Android adaptive background white, foreground scale 0.56 instead of 0.72;
  legacy Android icons use a white tile with matching visual padding. Approved
  triangle paths and colours are unchanged. Reproducible SVG renderer is under
  assets/branding/generate-icons.py. PNG/ICO transparency and foreground safe
  bounds verified, plus circle/rounded-square comparison generated for review.
- Linux portable core was also built/tested natively on the OMV runner as
  ghrunner: eight tests pass. Isolated checkout /tmp/secpar-core.JBPqA6/repo.
  No capture/input or system package installation was attempted on the headless
  server. This verifies protocol portability, not a working Linux host yet.
- Published icon releases from bd3377e: Android Preview 19 (24,080,295 bytes,
  SHA-256 797cf47b5f2c21d87408ad52b594c6e8bde9eae231d8d07c60265931a7dc812a),
  Windows Development 8 single installer (9,807,317 bytes, SHA-256
  ed2c9218a89ab15024d8e176aa717228f2dfad61b6e1614295384a69257fde0a).
  GitHub digests match; public APK/installer links return 200. Same APK signer,
  release/vital lint pass. All ten Android PNG density assets verified in the
  release APK. All seven transparent ICO frames verified in packaged desktop and
  host executables. Android native launcher review shows the new white padded icon
  next to the older black release icon in the isolated emulator. Audio/video code
  unchanged from user-confirmed Preview 18. Phone and installed host untouched.
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
- Desktop development 8: https://github.com/modnite/Secpar/releases/tag/v0.1.0-secpar-desktop-dev.8
- Previous desktop development 7: https://github.com/modnite/Secpar/releases/tag/v0.1.0-secpar-desktop-dev.7
- Previous desktop development 6: https://github.com/modnite/Secpar/releases/tag/v0.1.0-secpar-desktop-dev.6
- Previous desktop development 5: https://github.com/modnite/Secpar/releases/tag/v0.1.0-secpar-desktop-dev.5
- Android Preview 19: https://github.com/modnite/Secpar/releases/tag/v3.0.3-secpar-preview.19
- Previous Android Preview 18: https://github.com/modnite/Secpar/releases/tag/v3.0.3-secpar-preview.18
- Previous Android Preview 17: https://github.com/modnite/Secpar/releases/tag/v3.0.3-secpar-preview.17
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

- 2026-11-06: 3.1.0 Windows release built from optimized release profiles for both UI and host (6,501,246-byte installer). Isolated native tray check passed hidden→Open and minimized→Open. Desktop tests: 20 passed, one OS-vault integration check ignored. Installer payloads matched release binaries (expected NSIS marker patch only), transparent content-addressed ICO included, no debug/review binaries or PDBs. Shortcut icon repair covers upgrades and the finish-page desktop shortcut. Linux launcher identity/cache and Mac versioned-icon packaging corrected; native Mac runtime remains unverified. Installation/publication pending. User release/version rules are in docs/versioning.md.

- 2026-11-06: Windows 3.1.0 published, then installed successfully on this PC (installer exit 0). Installed UI/host both report 3.1.0; UI launched and Secpar service is running. Both public Desktop and all-users Start-menu shortcuts explicitly reference the installed transparent, content-addressed Secpar ICO. Independent PC Remote/Sunshine not modified.
- Android 3.2.0 / code 520: floating capture bubble removed; capture and PiP actions are full-width, single-line controls inside the panel. PiP keeps the video SurfaceView, releases remote input and hides local controls; hiding/dismissing the stream ends control. Native emulator entry/return/input tests passed (three earlier checks; two final PiP lifecycle checks). All 205 unit tests passed, including encrypted wake-link storage, request validation and migration of existing HA preferences. Optional HTTPS wake POST supports router/NAS/helper endpoints without requiring HA; local wake also sends directed subnet broadcasts. Release R8/vital lint passed; same signing certificate and package identity retained; APK is non-debuggable and review activities absent. Physical S23 PiP/remote wake still need user testing. Publication pending final UI review.
- Linux native hosting work is preserved on linux-host: owner pairing over pinned TLS, portal-consented monitor capture and keyboard/mouse through KDE Wayland, GStreamer hardware candidates with software fallback, system-output audio, authenticated PRS transport in the same desktop executable. Ten native host tests and 20 desktop tests passed on OMV; optimized combined executable built. Synthetic capture test uses videotestsrc, never the server's screen or input. Real KDE consent/streaming is unverified. Remote relay/account hosting, guest sessions, distribution/update integration and macOS hosting remain unfinished. Do not publish client-only archives as completed desktop apps.

- Android 3.2.0 published at https://github.com/modnite/Secpar/releases/tag/v3.2.0 with a 24,080,455-byte Secpar.apk; GitHub SHA-256 matches 2da22cb19b40999623c2033ef84b94b4ef26cbf81e9684c7a93401315ab2623d. Final full-width single-line controls screenshot checked at outputs/ui-review/android-controls-3.2.0.png. Final native PiP entry/return and dismissal tests both passed. S23 validation requested; no physical phone was modified. Windows shared-host extraction regression run: 132 passed, six intentionally ignored. Platform CI run 42174275218 in progress.

- Linux 3.3.0 packaging implemented: one AppImage with optimized host/client, bundled capture/encoder/audio plugins and dependency closure, transparent icon and license notices. Pinned/verified appimagetool 1.9.1 and runtime 20251108. Extracted package audit passed matching ELF/icon/version, no debug/review payloads, no missing dependencies and bundled plugin loading. Size 23,120,376 bytes; SHA-256 1032dd06530e80c6f35c599dd372e056b70748a292e818e83ac16b912d5a9b24. Installer fixture passed process-exit waiting, atomic replacement, automatic launch and quoted launcher/icon creation with spaces/percent in the user directory. Linux updater now launches the verified AppImage installer and closes the old app so replacement can proceed. Native KDE/Wayland screen sharing remains unverified. Linux public release pending final UI review/source commit.

- Linux 3.3.0 test release published: https://github.com/modnite/Secpar/releases/tag/v3.3.0. Final readable Hosting layout at 640×400 checked in outputs/ui-review/linux-hosting-640.png. Public AppImage is 23,120,376 bytes, SHA-256 588cd4bb346409d0ee08bfe8f713355a6aaad09fd7bedff7962da9fe11942be6, matching GitHub. One optimized host/client product, no separate client download. Marked prerelease because native KDE Wayland streaming still needs laptop validation; LAN owner-hosting limitations are explicit. Final Windows desktop checks also passed (20; one vault test skipped). Redundant platform CI runs cancelled; concurrency now avoids duplicates. Latest CI run 42176134216 is in progress for 18bb9ea. User's installed Windows Secpar service remains running. Closed only our tray-review process and emulator-5556; physical S23 and unrelated emulator-5554 untouched.

- 2026-11-06 user confirmed Android 3.2 PiP entry/return works. New reproducible issue: desktop video pause/resume permanently silences Android audio until reconnect. S23 PcrAudio log showed continuous packet decode but monotonically increasing staleFrames with queuedMs=0. Host regression reproduced exactly: a 25 ms capture leaves a 5 ms tail, then resumed PCM incorrectly inherits old timestamps indefinitely. Regression failed before fix and passed after shared capture-time AudioFrames reanchors at discontinuities, drops only incomplete old PCM and preserves contiguous batches. Windows WASAPI also respects in-batch discontinuities/timestamp-error fallback; Linux uses shared framing. Host full tests: 133 passed, six skipped; shared core 11 passed; Linux host ten passed. Building 3.4.0 optimized Windows/Linux fixes.
- User's CachyOS AppImage failed before window creation with NoGlutinConfigs/NotSupported(provided display handle is not supported). Reproduced identical failure using the public AppImage in an isolated current Arch/Weston Wayland environment. Loader trace: bundled Wayland 1.23 hides system Wayland, causing Mesa EGL's wl_fixes_interface to be unresolved. Excluding bundled libwayland-* fixes startup. Actual rebuilt AppImage passed native Wayland window title plus rendered wl_surface buffer checks (test-wayland-startup.sh), not just an X11/review-binary test. Build runner has no Docker access for ghrunner; tests were run by root in isolated containers. No Docker privileges granted to the runner; no server screen/input was captured.
- New required feature: same-account Windows/Linux automatic host enrollment/discovery and LAN authorization without app prompts, account-synchronized computer list/access usable from Android after account sign-in, with LAN-first then anywhere connectivity. Existing account computer records currently only hold name/platform; desktop remembers one local computer; Android lacks account UI; Linux host lacks account enrollment and relay. These gaps must be implemented, not represented as already available. Finish/install 3.4 audio/startup fixes first, then continue this ownership flow. Never upload reusable pairing secrets in plaintext or accept LAN presence as identity.

- 3.4.0 published for Windows/Linux. Windows updated successfully (installer exit 0, UI and host both 3.4.0; service running). User confirmed Linux app launches, signs in and connects to Windows. This validates Linux as a client on CachyOS KDE Wayland; it does not yet validate native Linux hosting.
- Public website deployed and verified at https://secpar.win/ (workflow 42184485853 passed). www redirects to root; accounts remain on accounts.secpar.win. Rubik sourced under OFL, approved Secpar wordmark, current per-platform verified downloads, accurate Linux/macOS status, no trackers. Browser checks passed at desktop and 375px phone size with no horizontal overflow and full download labels. Screenshots outputs/ui-review/secpar-site-live.jpg, secpar-site-desktop.jpg and secpar-site-mobile.jpg. Website uses existing Secpar Cloudflare account/repo and refreshes downloads on releases.
- 3.5.0 three-device streaming limit implemented for Windows/Linux via shared race-safe RAII reservations, held until streaming workers finish. Three slots accepted; fourth refused with connection_limit and device wording; failed starts/disconnect release capacity. Three capacity tests, 14 shared core and ten Linux host tests passed, along with Windows 133-regression run and a separate three-device controller-isolation test. Native Windows controller allocation remains per authenticated device; physical multiplayer and desktop hardware-controller forwarding are not yet validated/complete. Android 205 tests and minified release build passed, certificate/package retained, version 3.5.0 code 521. Linux production AppImage again passed native Arch Wayland window/render checks. Optimized Windows installer payloads verified. Publishing/installing 3.5 next; account-discovery and professional desktop UI work still required.

- 3.5.0 is published for Windows/Linux/Android. GitHub asset digests match staged
  release checksums. Site download refresh 42204063388 passed. Windows installation
  is waiting at the OS administrator prompt; installed app still reports 3.4.0.
  Do not report the update installed until version/service readback confirms it.
- Hosting settings polish: aligned two-column display/quality controls, grouped
  device access and connection/startup switches, Account action moved out of the
  settings scroll area. Native-rendered 640×400 review caught bottom clipping;
  section spacing was corrected so all six settings fit. Review screenshot
  outputs/ui-review/hosting-settings-640.png; larger-window review also checked.
  Desktop regression suite: 20 passed, one OS-keychain integration test skipped.
  This UI change is not yet in a published binary.
- Account-owned access API implemented with private owned-computer presence,
  enrolled-host polling, bounded two-minute bootstrap requests, requesting-session
  isolation, encrypted-at-rest one-use results and enrollment/revocation checks.
  New cases exercise isolation, invalid links, racing claims, expiry, cancellation,
  relinking, ciphertext swapping and sign-out. Local workerd/D1 runtime verifies
  the actual AES-GCM/database roundtrip. Native applications still need integration;
  no claim that automatic pairing is available in current releases.

- Windows 3.5.0 installation completed (exit 0); installed host/UI both 3.5.0,
  Secpar service running. Installed host hash matches the verified installer
  payload. User then confirmed a second device fails with capture/locked-screen
  error in either connection order. New strict hardware regression reproduced it:
  first HEVC stream starts, second capture fails. DXGI permits one duplication per
  output/process, irrespective of the new three-slot capacity counter.
- Shared Windows capture now uses one producer per monitor, latest-frame delivery,
  bounded immutable GPU texture leases and independent client encoders/transports.
  Actual hardware regression passes three concurrent streams (HEVC + H.264 + H.264),
  fourth rejected at the connection limit, disconnect/replacement succeeds, and
  capture closes after the last viewer. Received 89/285/263 video packets plus 46
  replacement packets in the checked run. This fix is not yet in a public binary.
- UNRELEASED account integration is in the main working tree: shared machine
  enrollment/proof/heartbeat/pair-request polling; Windows DPAPI adapter preserves
  existing file format; Linux private machine credential storage and enrollment;
  independent owner pairing codes beside the manual QR; atomic code/context/device
  creation; account-tagged device credentials and account unlink revocation;
  desktop automatic enrollment, owned-computer list and account-namespaced saved
  access. Native GUI sample fits 640×400. Shared tests 21 passed, desktop 22 passed
  (one vault test ignored), Windows agent 15 passed (one live relay test ignored),
  including concurrent actual TLS pairing and manual-access preservation on unlink.
  Linux earlier baseline passed 11 host and 20 desktop tests before the latest
  account test additions; refresh those checks. Android accounts and Linux remote
  relay/media integration remain unfinished. Do not ship this tree as completed 4.0.
- Latest user steering: source requirements are recorded in AGENTS.md. Narrow the
  app, version beside logo, separate host/client settings, presets labeled by real
  resolution/FPS, configurable one-hand shortcut replacing Ctrl+F12, less website
  prose and real client hardware decode options. X250/Linux playback/cursor is poor;
  current desktop OpenH264 decoding is CPU-only. Restore Android icon background
  to BLACK, keeping the existing padding/geometry. These additions remain required.

- 2026-11-06: Windows 3.7.0 published and installed (host/UI readback3.7.0, exit0, service running). User confirms Android video smooth again after zero-wait shared DXGI capture. Controlled same-binary latency A/B and three concurrent hardware streams passed.
- Android/Linux3.8.0 published: https://github.com/modnite/Secpar/releases/tag/v3.8.0 (ecfc96c). Android APK24079999bytes SHA256e989b09d869825fec2d79625a1d6d403de80d0f5ba9c574536674d3906599542; same signing identity/package, non-debuggable/minified, review/test payloads absent, black icons retained. Linux23386616bytes SHA256aa935ab2421b0582f333cc995b9773184201a59ed9351be46cbc566f5f428f01. GitHub digests verified; site refresh42262590998 passed.
- Android fixes: capture-time PCM timeline + bounded recent clock probes; initialize audio before firstPROBE; bounded audio-only restart after codec/output failure (permanent dead-state bug fixed); free native dock pointer without capture/release shortcuts; absolute input only on video hit target; focus/input safety, clickable Android overlays; streaming text clipboard/refocus; compact stable checking/online home layout and600x420dp freeform default.
- Validation:217 unit tests; all five real audio gates pass including exact144000 frames, repeated silence/switches, deliberately stopped decoder recovering in sameplayer and180s continuous acceptance/sync. Virtual no-audio sink has artificial underruns, so no physical audible-continuity claim. Four input, two PiP and one video-pacing check pass; PiP final task cleanup made one-shot. Wide/small synthetic layouts visually reviewed; debug/review builds kept local. Release APK cold-launch succeeded on5556, then ONLY our emulator shut down. User's5554 and physical phone untouched.
- Linux3.8 adds off-UI bounded decoding, real VA-API decoded-frame probe and software fallback;26 desktop tests plus native Arch Wayland AppImage rendering pass. X250 driver/performance and actual KDE hosting remain physically unverified; user cannot test Linux currently.
- User requirements saved in docs/user-requirements.md, linked from AGENTS.md: new prompts accumulate; audio/video regression gates; Android-dock free pointer/cursor responsiveness; clipboard; compact loading state; responsible narrow subagents. Two fork-none subagents used, no fanout. Main account-owned foundation remains unreleased; Android accounts, Linux anywhere integration and macOS hosting still required. Awaiting S23 audio validation on3.8; do not claim project family complete.

- User physically confirms Android3.8 audio stays live and recovers after pauses/video switches; free dock pointer and Android/PC text clipboard both work. Latest assessment: "This version is simply perfect so far". Record Android3.8 + Windows3.7 as approved baseline. Keep unfinished account/platform work separate and preserve regression gates. Linux testing remains pending; do not assume physical X250 validation.

- Preserved the user's approved Android3.8.0 +Windows3.7.0 baseline. Continued
  Android account work separately on main: fixed-origin typed API and encrypted
  owner/session-bound credential store,19 focused tests passed (zero failures or
  errors). Tests cover request/link validation, credential redaction, expiry,
  sign-out/manual-pair preservation, cross-account isolation, late response writes,
  corrupt storage and encrypted-grant relocation. No existing streaming/input/audio
  sources changed. Not integrated into the UI or released; controller/home/session
  integration and an actual account-to-host test remain required.

- Android account UI/controller/store integration implemented locally.252 JVM tests
  pass, eight layout/input/PiP/video instrumented cases pass, and an actual pinned
  TLS owner pairing/reconnect/sign-out between Android5556 and an isolated native
  Windows test host passes. Backend responses in that bridge are scripted; no live
  user account/auth claim. Two storage/lifecycle bugs from one bounded read-only
  agent review fixed, plus explicit account credential routing beside manual pairs.
  No audio/video engine changes, no installed-app updates or published artifacts.
  Compact600x420 and360x640 screenshots saved with synthetic names. Additional
  cancellation-generation/expiry fixes unit-tested after the instrumented gate;
  final production APK still needs rebuilding and matching native account release.
