# Active Secpar work — 2026-11-06

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

## CURRENT: supersedes historical statuses below

- VERIFIED NEWEST: Windows3.9 installation completed with exit0; installed host/UI
  versions3.9 and SHA256 match the published installer payloads exactly. Secpar
  service Running. Desktop launched; sanitized host_account_state confirms linked,
  owner present, online, no note/error. User has now physically confirmed Android
  3.9 Account sign-in shows the PC and connects WITHOUT QR. End-to-end account flow
  is USER-ACCEPTED, not just a scripted/local test. Prior UAC-pending text below is
  historical. Do not repeat installation or ask the same account test again.
  Next active implementation: Linux outside-LAN hosting (relay/STUN/TURN). Keep
  this isolated from approved Windows/Android engine code. Mac work stays Claude's.

- NEWEST:3.9.0 is PUBLIC at https://github.com/modnite/Secpar/releases/tag/v3.9.0,
  source99b8762, release/3.9.0 branch/tag pushed. Website42308784762 passed.
  Android/Windows/Linux release packages +checksums match GitHub asset digests.
  Stage D:/Secpar-CI/publish-3.9.0; Windows6582865bytes, APK24112771, Linux23497208.
  APK v3.9/code524, unchanged signing cert, R8/non-debuggable, no review/test media.
- WINDOWS INSTALL PENDING OS UAC approval, exec session41501. User asked ONCE via
  async question after consent.exe observed. Published silent installer launched
  with RunAs only after TWO checks showed0 connected Secpar devices. Do not launch
  another installer. Read session/result and installed PE versions before claiming
  installed. Result D:/Secpar-CI/publish-3.9.0/install-exit-code.txt. Original PC Remote
  and Sunshine services were STOPPED already; preserve them. Prior installed3.7.
- Final gates:252 Android units;5 real audio cases incl180s uninterrupted playback
  passed in193.978s; final3.9 debug actual TLS/native owner pairing+reconnect+Ping+
  logout/manual preservation passed in9.106s, native host proves account provenance.
  Bridge ACCOUNT HTTP IS SCRIPTED, actual host/TLS is real. Optimized production APK
  cold-launch, Account screen, LIVE HTTPS device-approval startup and cancellation
  passed (no credentials approved). Input4/PiP2/video1/layout1 passed earlier.
- Windows15 agent+1 real DPAPI test; client10 incl actual LAN-first trap-relay case;
  desktop25 pass/1 keychain ignored. Linux21 core+12 host+10 client+28 desktop pass
  (1 keychain ignored); optimized AppImage passed actual Arch native Wayland/EGL.
  Native owned desktop pairing now LAN-first; manual QR routing retained.
- Source prs/stream2/Windows engine/desktop audio is unchanged from user-approved
  Android3.8/Windows3.7 baseline. No new physical3.9 acceptance yet. Linux remote
  HOST relay/STUN/TURN still missing, Linux physical KDE/X250 validation unavailable,
  Mac hosting remains Claude scope. Do not describe app family as complete.
- Public SHA256: APK e00727878c8402911aa7c5d7dacd6ff7365092529681a529e75ed7ace2ce8b56;
  Linux f46512062238571f8c3a364dba370ccfcb97f25376442d684e552edd1f09b064;
  installer6735523660683724810dca2668dbdf09e357b620b363db216bf1ef3d3258c890.
  Installer payload hosteb3bb789c1c00395188163611e153945531affbb5f65dde7b78d135b5a09cf69;
  UI58149593c7ce2ddce4f8c5655124a7452fb50289c0ab84e8f4bbafb036f58de0.
  Tauri packaging changes exactly TAURI_BUNDLE_TYPE_VAR_UNK -> NSS (3 bytes), not
  arbitrary payload drift. Host/UI PE versions3.9, no PDB/debug sections.
- Only test emulator5556 used;5554 and physical phone untouched. Physical S23 adb
  is visible again.5556 remains RUNNING: automatic tool review rejected a combined
  notes-save/emulator-shutdown command with only 'blocked by policy'. No shutdown
  or notes from that rejected command occurred; notes saved via file tool instead.
  Avoid repeating blocked shutdown. Debug app com.pcremote.app.debug; test .debug.test.
- NEXT: finish UAC-dependent Windows install/readback, launch desktop and confirm
  automatic account enrollment via sanitized host_account_state, then user Android
  3.9 Account sign-in/owned connection check. No passwords/approval codes in chat.
  Continue Linux anywhere hosting and pending friends/gamepad scope afterwards.
  Claude macOS checkout remains separate. Preserve approved baseline; new prompts
  accumulate. Earlier pre-publication statuses below are historical now.

- Android account integration IMPLEMENTED on main, not yet published: browser device
  sign-in, owned-computer list, automatic bootstrap on Connect, encrypted per-account
  access overlay beside manual PcStore, explicit owned navigation routes (a stale
  manual pairing cannot mask new account access), LAN-first pinned TLS for owned
  pairing/reconnect, cancellation/expiry/logout isolation and cached-session cleanup.
  In-memory owner gate hides access immediately; a second revision follows durable
  removal. Main-thread controller uses separate AccountApi per login generation.
  AccountScreen cancels pairing/navigation on Back, pause and disposal.
- Validation:252 Android JVM tests pass. Earlier latest production-code APK passed
  eight emulator layout/input/PiP/video gates; actual Android -> isolated Windows
  native-host owner-pair TLS/reconnect/logout passed, including old manual duplicate
  and host-side account provenance check. Account service in that bridge is SCRIPTED,
  not a live account sign-in. Final added connection-attempt generation/expiry timer
  have JVM gates; rebuild final APK before release. Native fixture is ignored by
  default and requires explicit SECPAR_ANDROID_OWNER_FIXTURE; deletes its OTP file.
- Synthetic account screenshots reviewed at600x420 and360x640, all three entries fit:
  outputs/ui-review/account-computers-{600x420,360x640}.png. Capture fixture now waits
  for actual Compose draw then presentation; old two-frame-only phone capture was
  stale and replaced. Debug package is com.pcremote.app.debug; instrumentation uses
  com.pcremote.app.debug.test.5556 is our emulator;5554 and physical S23 untouched.
  Physical S23 is visible via adb again, but do not install/debug builds onto it.
- Next release must include matching desktop/host account foundation. Installed
  Windows3.7 cannot yet serve automatic owner bootstrap; do not ship Android alone
  as finished account access. Desktop initial pairing still relay-first; add a
  separate LAN-first owned route without altering the approved manual stream path.
  Host engine and desktop audio source match v3.7 exactly. Main release validation,
  native builds and live user account sign-in remain before claiming release ready.

- User requested a parallel Claude handoff. Prepared sibling worktree
  ../secpar-claude-macos on claude/macos-host at f2cb44e and sibling file
  ../secpar-claude-handoff.md. User launches Claude separately; not running yet.
  Scope: new macOS native host adapter with narrow macOS-only desktop/package/CI
  integration. Codex retains Android account integration and other active work.
  Do not edit Claude-owned files or merge unreviewed work; no release/install
  permission delegated to Claude. Review handoff commits/tests before integration.

- Android account SDK/storage foundation added on MAIN ONLY;19 focused JVM tests
  pass (11 API +8 vault). Fixed accounts.secpar.win origin/system TLS/no redirects,
  bounded responses, validated browser approval and owner-bootstrap links; encrypted
  native session and owned grants separately from manual PcStore. Grants bound
  internally to owner+session hash; stale writes/sign-out rejected; expiry and
  corrupt ciphertext fail closed. No production adapter/controller/UI integration
  yet, no account release or on-device account claim. Existing approved3.8 untouched.
  Next: one application-scoped vault using KeystoreBox + excluded preferences,
  controller with independent AccountApi PER generation (SDK poll installs bearer,
  logout clears its own instance). Guard late login before saveSession; compare
  bootstrap fp to expected cloud-computer fp; never log PairUri/PairedPc secrets.
  Expose owned grants to Home/mode sessions without copying into manual PcStore;
  sign-out must close cached account connections and clear access. Then test actual
  signed-in Android -> enrolled host before publishing matching native foundation.

- Read docs/user-requirements.md and AGENTS.md. New prompts accumulate; no scope
  abandonment. User permits a few narrow subagents, with extreme token care.
- Windows 3.7.0 published AND installed, exit0, host/UI3.7.0, service running.
  User confirms Android video smooth again.135 host tests + controlled latency
  A/B + three actual concurrent hardware streams passed. Linux3.7 published too.
- Android 3.8 is USER-CONFIRMED: audio stays live and recovers; dock mouse leaves
  the window freely; Android/PC text clipboard works. User calls it "simply perfect
  so far". Approved baseline with Windows3.7; preserve it. Linux validation still
  pending availability; physical phone debugging remains unavailable.
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
- S23 validation complete: audio, free dock pointer and text clipboard all work;
  overall 3.8 approved. No unresolved Android streaming defect reported. Broader
  owner accounts, Linux remote/native-host validation and macOS hosting remain
  required. Current next task: Android account sign-in/owned-computer access on
  main, separate from the approved release; no streaming engine edits.
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
