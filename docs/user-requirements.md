# Secpar: persistent user requirements

Latest correction: floating top-left stream menu in WINDOWED AND FULLSCREEN
on desktop and Android. Remove desktop windowed streaming toolbar strip and
Android right-edge handle. Ctrl+C/X/V currently fail on Windows client because
eframe emits Copy/Cut/Paste events; translate those to remote shortcuts.

Latest work, 2026-11-07: top-left translucent fullscreen stream menu required on
BOTH desktop and Android. Remove Android right-edge handle; move its controls into
that menu. Research/adapt relevant Parsec support docs into original Secpar app
and site help. Persist researched writing rules in docs/writing-style.md.
Windows/Linux text clipboard needs implementation both directions. Windows client
cannot finish Linux hosting after KDE consent; optional host-audio failure is
incorrectly fatal in client, actual capture diagnostics were also being hidden.
Linux3.10.1 video regression: STILL CHOPPY with audio OFF; user confirms SOFTWARE
decoding RESTORES smooth playback. Hardware path is implicated; protect software
and accepted audio. Add actual negotiated-rate decode budget fallback rather than
claiming128px hardware sample proves streaming performance. Other scope stays.

Accepted 2026-11-07: user confirms Linux3.10.1 audio plays and Android3.10.1
Home stays aligned on YouTube/PiP return. Preserve this accepted baseline;
do not repeat those acceptance questions. Continue remaining platform work.

Latest feedback 2026-11-07: Android3.10 returns with asymmetric home layout:
left margin enlarged and header under status bar when returning with YouTube PiP;
force-stop/reopen restores it. Grey screen improved but lifecycle/layout issue is
NOT accepted as fixed. Linux3.10 audio STILL SILENT; do not treat virtual-sink
success as physical acceptance. Root owns both defects; Linux outside-LAN work
remains queued. User confirms Retry stays silent and exact error is "no usable audio output
format found".3.10.1 is published with desktop sound service first, ALSA fallback;
physical acceptance and Android alignment feedback pending.

Latest corrections: desktop client must have a free native pointer in a window,
capture only in fullscreen, Alt+F fullscreen toggle. This desktop request does not
override Android's approved free dock pointer. User confirms Linux video is perfect
but no audio output; preserve the video path while fixing audio. Android whole app
can return grey after stream ended and switching to YouTube (sometimes YouTube
PiP); launcher works, force-stop/reopen restores. Present since3.8, predates account
integration; do not misdiagnose as stream/audio reconnect. Keep Claude updated via
the sibling handoff file; it owns macOS hosting, Codex owns shared client fixes.
Controller support remains required: Windows virtual Xbox/per-device isolation and
Android touch pad implemented; physical controller forwarding in Android/desktop,
Linux/macOS virtual pads and full4-player/rumble validation still outstanding.

Read this with AGENTS.md and docs/next-milestone.md when resuming work. Additional
prompts add to the task; they do not cancel prior requirements. Later explicit
corrections take precedence over earlier preferences.

## Approved baseline

- Android 3.8.0 with Windows host 3.7.0: user confirms audio stays live and
  recovers after pauses/video switches, dock pointer leaves the window freely,
  and Android/PC text clipboard works. Latest assessment: "simply perfect so far".
- Preserve this streaming/input/UI baseline. Keep unfinished account/platform
  work separate; do not alter the streaming engine for unrelated features.

## Working relationship

- Deliver completed, usable results; do not stop at acknowledgement or a plan.
- Continue after check-ins and questions. Give meaningful progress updates and
  actual screenshots, not only descriptions or mockups.
- Preserve completed work and diagnostics. Do not rediscover/repeat work without
  a reason. Keep current failures, release state and remaining tasks in the notes.
- Subagents are explicitly permitted again (2026-11-06), but use them sparingly,
  with small independent scopes. Be extremely careful with token usage and pace;
  do not burn through hours of allowance in minutes. No recursive delegation.
- The user authorizes publishing optimized releases/fork changes and installing
  and launching Windows Secpar updates on this PC. Preserve independent PC Remote,
  Sunshine and other remote access. OS administrator prompts still require the
  user's interaction. Do not claim an update installed before reading it back.
- Keep debug/review/test builds local. Publish lean optimized packages, retaining
  Android signing identity and updater compatibility.
- Versions are x.x.x: major milestones / features and bug fixes / minor visual
  or small fixes. Avoid preview labels as the normal long-term naming scheme.

## Product and connectivity

- Name: Secpar. Make it its own product, removing user-visible PC Remote wording,
  old icons, setup links and phone-only assumptions.
- ONE host/client application per desktop OS: Windows, Linux, macOS. Every desktop
  OS must eventually host and connect to every other desktop OS. Background service
  binaries are implementation details, not additional products/downloads.
- Android stays client-only and connects to those same desktop hosts.
- Accounts, friends, guest consent, shared computers and multiplayer remain in
  scope. The user prefers ordinary Secpar account sign-in, avoiding required
  biometrics. Email registration/sign-in were confirmed working.
- Same-account machines should discover and authorize each other on LAN without
  QR/approval prompts, save access through the account, and appear on signed-in
  Android too. Keep per-device credentials and revocation/account isolation.
- Try LAN first, then authenticated anywhere connectivity. Minimize VPN reliance;
  support initial remote pairing. Do not require ownership of the upstream relay.
- Support three connected client devices per host (four players including host).
  Say devices, not phones; report a full connection limit accurately.
- All applications need updaters. The user wants Check for updates and Download
  update beside each other, with Download disabled unless an update exists.
- Wake-on-LAN should have a broadly usable setup without requiring Home Assistant.
- secpar.win is the website; accounts.secpar.win handles accounts. Use the existing
  Cloudflare/Resend configuration; never put tokens, passwords or links in chat.

## Input, picture and audio

- Moonlight/Artemis-like resolution and physical keyboard/mouse behaviour. USB
  keyboard/mouse on a phone dock must work without the on-screen keyboard toggle
  or controls obstructing the desktop. Touch fallback should act as a trackpad.
- No unwanted quick-tips popup. Android PiP is required and was confirmed working.
- Latest clarification: the request for a freely moving mouse is specifically
  ANDROID ON THE DOCK. Its pointer must leave the app/window naturally, without
  needing capture/release shortcuts, and interact with Android/DeX and overlays.
  Do not assume this clarification requires changing Linux's pointer mode.
- The Android dock pointer feels like 30 fps while video plays at 60. Fix cursor
  responsiveness; investigate local/native pointer rendering and forwarding rather
  than treating a smooth video result as proof of smooth input.
- Cross-device copy/paste is required. Android 3.8.0 streaming clipboard/refocus is confirmed working; preserve it.
- Audio must recover after silence, pause/resume and switching videos without
  disconnecting. It can also fail after minutes of uninterrupted playback.
  These failures are resolved in the user-confirmed 3.8.0 baseline. Retain them as
  regression cases; do not rely on pure timing tests alone.
- The user requires no audio/video streaming regressions. Gate releases on steady
  playback, silence/resume, track changes, loss/hiccups, timing and the working
  video/cursor path. Never trade smooth video for broken audio or the reverse.
- Keep latency bounded, regain synchronization after stalls, and soften crackles
  with appropriate transition handling. Do not conceal growing lag with buffering.
- Expose real client hardware-acceleration choices supported by the actual system.
  A plugin name alone is not proof of hardware decoding; verify actual output and
  provide an honest fallback. Preserve Android's working hardware video decoder.

## UI, branding and testing devices

- Clean, compact, professionally aligned Android/desktop UI inspired by Parsec.
  No sheen, radial gradients, decorative fluff, verbose descriptions or AI prose.
  Flat must still be polished: align rows, grids, heights, baselines and spacing.
- Separate host/client settings, version beside logo, actual preset resolutions
  and frame rates. Keep the desktop window as narrow as practical; test smaller
  displays without tiny text, excess dead space, awkward wrapping or scrolling.
- Android desktop/DeX checking preview must remain compact: never expand to the
  full window, clip computer controls, or change the grid when it becomes online.
- Fit Picture in picture on one line without clipping or awkward wrapping.
- Website writing as well as layout should learn from Parsec's concise style.
- Preserve the approved mint/orange rounded perspective triangle, equal side
  widths, direction and Parsec-like proportions. Preserve the sourced wordmark.
  Desktop icons: triangle only, transparent. Android icons: BLACK background with
  safe padding (latest correction overrides the earlier white request).
- Linux logo/wordmark looked jagged; filter properly at physical display scale.
- Known devices: Galaxy S23 on Android 17, USB keyboard/mouse dock; ThinkPad X250,
  CachyOS KDE Plasma Wayland; one Windows PC. Probe the laptop's actual CPU/driver;
  do not invent its exact SKU. OMV is a build runner, not a streaming test target.
- The user is currently away from home and cannot test Linux. Continue independent
  checks without repeatedly requesting laptop tests. Physical S23 debugging is
  currently disconnected from this home PC; do not claim live log access.

Glyph requirement, 2026-11-07: ALWAYS use an established, named icon/glyph library
as the source for UI symbols. Prefer original library assets with a pinned version
and license; never improvise icons from text characters or ad hoc painter strokes.
Use a consistent family/weight/size across menus. Secpar triangle is approved brand
art, not a generic UI glyph. User rejects current hand-drawn desktop menu symbols.
Linux hardware playback remains below Moonlight after3.11.0. Software is the known
smooth workaround; fallback is NOT proof that hardware rendering is fixed.

2026-11-07 diagnostic: Linux1920x1080/60 Direct, RTT3ms,0.8Mbps,21.9 received FPS;
actual decoder software(VA-API too slow). Stream later drops with no authenticated
packets error while typing elsewhere. Preserve icon work and pursue both renderer
performance and UI-independent stream/audio transport. Do not call fallback fixed.

2026-11-07 acceptance/correction: Linux3.12.0 audio continues and connection stays
up when covered/minimized20s. Glyphs approved.1080p video remains poor regardless
of hardware/software decoder, while720p is usable. Do not assume decoder choice
alone explains1080p slowdown.720p stream has coloured fringes around Windows text;
inspect chroma/subpixel scaling and actual matrix/range before claiming a fix.
Android menu version footer needs explicit compact text metrics; inherited body
letter spacing/line height at10sp is inappropriate. Preserve accepted audio paths.

Streaming architecture correction,2026-11-07: user reports720p also hitches with
BOTH decoders; never mark it smooth60fps or imply hardware choice solves this.
User explicitly requests using mature Artemis/Apollo/Moonlight/Sunshine code
DIRECTLY instead of continuing the custom PRS streaming engine. Prioritize an
upstream engine migration while retaining Secpar accounts, ownership, UI, automatic
updates and one-app-per-OS packaging (Android client-only). Park the bespoke
VA/EGL renderer work; do not spend more building a parallel custom media stack.
Preserve the installed working connection while developing/testing replacements.

2026-11-07: choose the better upstream streaming implementation based on code,
platform support and validation; no requirement to prefer Artemis/Apollo by name.
Selected migration baseline Moonlight Qt6.2.0, Moonlight Android12.2 and Sunshine
2026.914.233613; keep Artemis/Apollo as feature references. Selection is not a
claim of measured parity on X250/S23. Keep client devices awake throughout active
remote sessions on ALL OSes and release sleep inhibition on disconnect. Android
PiP belongs only to the active stream; launcher, computer list and settings must
not enter PiP, even via desktop-window controls. Current MainActivity declares
PiP support; use a separate stream activity in migration rather than merely
switching auto-enter off on the launcher. This restriction is not yet shipped.

2026-11-07: Linux hitches correlate with sudden received bitrate below1Mbps.
Keep this as a migration acceptance case; correlation is not proven causation.

Workflow correction, 2026-11-10: test confirmations and additional prompts do not
end the ongoing task. Record the result and continue the remaining authorized work.
Do not stop with an acknowledgement-only reply while implementation remains.
Pause dependent work only for required approval/input or a real blocker, and
continue independent work meanwhile. Preserve accepted fixes during follow-up work.

Hard release gate, 2026-11-10: the next public release must fix frame pacing on
Android and Linux. No UI-only or audio-only release while it remains unresolved.
Preserve the physically accepted Android audio fix and validate actual presentation,
not just received FPS. Include the shared host encoder/capture path in the diagnosis.


2026-11-10 host test and encoder direction: user reports pacing seems okay after
installing the private Windows3.19.0 host; this is encouraging physical feedback,
not full Android/Linux release-gate acceptance. The S23 pacing APK is authorized
but not yet installed because ADB is disconnected; preserve accepted audio.
User explicitly authorizes a stripped custom FFmpeg dependency for the Sunshine
encoding path, overriding the earlier no-FFmpeg restriction. Aim below10MB total,
measure both delivered package and installed payload, and report actual sizes.
Follow pinned Sunshine implementation and per-vendor routing, including native
NVENC where upstream uses it. Expose a persisted host encoder selection and real
supported encoder controls; unavailable backends must not masquerade as working.
Do not silently fall back to Media Foundation for explicit encoder selections.
Desktop Settings must show Client and Host settings expanded, with one vertical
layout. Files belongs on computer cards, matching Android, not in Settings.

2026-11-10 clarification: the user accepts deferring the full Android streaming
engine migration. Keep its accepted audio, input and rendering fixes. Continue
the host encoder integration, desktop UI and Linux client work. Do not describe
MediaCodec or Moonlight-inspired timing as running the Moonlight/Artemis engine.

2026-11-10 release workflow correction: use the normal optimized release artifacts
for validation and eventual publication. Avoid a separate private test-build cycle.
This does not waive the Android/Linux gameplay pacing gate or authorize rushing a
release. Keep development/debug fixtures local; retain the release as a draft
until the required validation passes.

2026-11-10 latest correction: publish optimized public releases after build checks
and let the user manually update clients and perform physical stream testing.
This supersedes the earlier instruction to hold publication for Android/Linux
pacing acceptance. Report unverified fixes honestly; do not call the remaining
hitches/audio dips resolved from compilation or unit tests. Conserve usage.

2026-11-10 standing workflow: for changes without UI work, build and publish the
optimized release, then let the user manually update and test. Avoid additional
local self-test cycles for those changes. For UI changes, perform local layout
and visual checks before publication. Preserve the existing CI configuration.

2026-11-10 scope correction: encoder AND decoder improvements are required across
Windows, Linux and macOS; Android remains client-only and needs decoder work.
Windows-only improvements do not satisfy this requirement. Testing a Linux client
against a changed Windows host is not a Linux engine improvement. Prioritize the
Linux media path next, retain macOS hosting/client work as active scope, and report
unverified improvements or regressions plainly. The latest Android audio dips are
reported worse, so accepted historical audio fixes are not proof of current health.

Latest feedback: frame pacing much improved; Linux720p smoother but above720p
severely hitches on X250. Do not dismiss that as a hardware limit without evidence.
Android audio clipping/hitching remains a distracting post3.19 regression.
Prioritize its recovery path, preserve video improvements, conserve the reported
9% remaining weekly usage, and publish non-UI fixes for user testing.
