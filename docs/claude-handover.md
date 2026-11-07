# Secpar — full-project Claude handover

Updated2026-11-07. Read this first, then AGENTS.md, docs/user-requirements.md and
only the CURRENT/top entries of docs/delivery-status.md. Older delivery entries
are history, not current acceptance. This replaces the earlier Mac-only handoff.

## Objective and working agreement

Finish Secpar as one application per Windows/Linux/macOS, each hosting and
connecting. Android is client-only. Retain account-owned computers, automatic
LAN-first authorization, VPN-free remote access, friends/guests, clipboard and
controllers. Host plus THREE remote devices is the multiplayer target. Never
publish separate host/client products or make users install Moonlight separately.

The user explicitly ordered replacing the custom PC Remote/PRS media engine with
mature upstream code. Use the whole upstream streaming path: client decoding,
audio, frame pacing and GPU presentation, plus upstream host capture/encoding.
Do NOT resume the parked custom VA/EGL renderer or spend another round tuning PRS.
Moonlight/Sunshine is the selected baseline; Artemis/Apollo remain useful feature
references. Selection is not a claim of measured performance parity on user devices.

Keep working to concrete checkpoints with useful check-ins and actual screenshots
when interfaces change. Do not repeat completed investigations/tests without a new
change or unresolved concern. Conserve tokens and agents. No further agent spawning
without an explicit need/authorization; none were used for the latest migration.

Do not interrupt the installed Windows host or other remote connections for tests.
Do not touch the original PC Remote/Sunshine installations. User permits installing
Secpar Windows releases, but preserve active sessions and respect OS UAC prompts.
Debug/test builds stay LOCAL. Publish optimized, stripped, non-debuggable releases.
Versions x.x.x: major milestones / features and bug fixes / small visual fixes.

## Repository and worktrees

Repository https://github.com/modnite/Secpar. User modnite. Draft migration PR:
https://github.com/modnite/Secpar/pull/1 (NOT ready to merge or ship).

Workspace parent:
C:/Users/c/Documents/Codex/2026-11-04/https-github-com-sadiktd-pc-remote/work/

- pc-remote: stable/release worktree, branch fix/desktop-audio-and-resume; pushes main.
  Main last recorded8ec2b61 before this handover documentation. Check git log/status.
- secpar-upstream-engine: ACTIVE implementation, branch
  feat/upstream-streaming-migration, checkpoint a9b14d5. Continue here.
- secpar-claude-macos: Claude's existing worktree, branch claude/macos-host,
  last observed df7a62c, clean. Do not reset/rebase/delete it. Reassess its bespoke
  native-host work against the new upstream migration before merging it.
- secpar-native-renderer: PARKED prototype, branch feat/linux-native-renderer,
 19d7d59. Its modules are test-only and never enabled. Do not continue that approach.

Main and the migration branch can have newer documentation commits. Inspect both;
do not overwrite unrelated changes or infer newest code from an old handoff SHA.

## Released and installed

Public Android3.12.1 (code529), desktop Windows/Linux3.12.0.
https://github.com/modnite/Secpar/releases/tag/v3.12.1
https://github.com/modnite/Secpar/releases/tag/v3.12.0
https://secpar.win/releases.json lists verified downloads per platform.

Android3.12.1 only fixes the stream-menu version footer's explicit font/line-height/
letter-spacing. No media/PiP changes. R8 release, existing signing certificate,
real emulator cold launch passed. APK SHA256:
845ed484b2ca622fa373431d8748ce553e6e02e627c7c85b94c86863328981d8.
Do not change the signing key. The certificate's DN says Android Debug; existing
APK updates depend on this certificate, despite the build being non-debuggable.

Installed Windows remains3.11.0 under C:/Program Files/Secpar Host. The Secpar
service was running and four pairings preserved. No service restart/update during
migration work. Do not assume latest public installer is already installed.

Website live at secpar.win; accounts at accounts.secpar.win. Cloudflare and Resend
are configured through GitHub secrets. Password/email accounts work; user declined
biometric/passkey dependence. User confirmed account-owned Windows appears and
connects on Android without QR. Never read or print API keys/passwords/tickets.

## Acceptance and current faults

- User Galaxy S23, Android17, USB keyboard/mouse dock. CachyOS laptop is a ThinkPad
  X250 running KDE Plasma Wayland; do not invent its exact CPU SKU.
- Android audio recovery/video, free windowed dock pointer and clipboard were
  accepted at earlier baselines. Protect these against regressions.
- Linux3.12.0 now preserves audio and connection while covered/minimized20s: USER
  CONFIRMED. It fixes UI/event-queue backpressure, not sustained playback smoothness.
- Linux1080p is poor with EITHER decoder;720p is usable but also hitches. Hitches
  correlate with received bitrate suddenly below1Mbps. Keep host target bitrate,
  received rate, loss and received FPS separate. Earlier stats were software after
  VA-API fallback,1080p60 Direct, RTT3ms,0.8Mbps,21.9 received FPS. No parity claim.
- Windows text at720p has colour fringing. Old software conversion assumes BT.601
  while Windows host produces BT.709; scaling/ClearType/chroma may also contribute.
  Preserve this reproduction for upstream comparison; no rushed PRS converter fix.
- Windows→Linux hosting previously granted KDE capture consent then returned to
  list. Not physically confirmed fixed. Cross Windows/Linux clipboard also needs
  acceptance. macOS full-role integration, guest enforcement and controllers remain.
- Keep CLIENT devices awake for the entire connected session on ALL OSes, release
  inhibition on disconnect. Upstream Qt provides keep-awake and release calls;
  apply that option and physically test it. Not yet delivered by the migration.
- Android only the ACTIVE STREAM may use PiP. Current MainActivity declares PiP,
  so desktop controls can force the whole interface into it. Use a dedicated stream
  activity; main/list/settings must not advertise PiP. Upstream Game already uses
  that separation. This restriction has not been shipped.
- Glyphs approved. Always use pinned established libraries, currently Lucide0.468.0
  with originals/licenses under assets/icons/lucide. Never improvise glyph strokes
  or Unicode symbols. Keep the approved mint/orange triangle, desktop transparency,
  Android BLACK safe-padded background and sourced wordmark.
- Compact polished grids, small displays, clear labels, free mouse in windows,
  capture in fullscreen, Alt+F. Plain copy; read docs/writing-style.md. No gradients,
  verbose tips, AI phrasing, placeholder features or fake performance claims.

## Migration implementation and verified evidence

Read integrations/gamestream/README.md and upstream-lock.json in the ACTIVE branch.
Pins: Moonlight Qt6.2.0, Moonlight Android12.2, Sunshine2026.914.233613; reference
Artemis20.2.6/Apollo0.4.6. Source submodules and original licenses are retained.

Private Moonlight Qt worker fully compiles on Linux. Small patches only private
Secpar executable/settings identity, Alt+F and managed CLI pairing exit; media and
crypto implementations are unchanged.511 tracked source files audited against the
pinned/patched checkout. Stripped worker2.2MiB before runtime libraries. It uses
.config/Secpar/SecparStream.conf, not installed Moonlight settings.

REAL isolated Sunshine→Moonlight interop passes:
- wrong PIN rejected from a separate client identity; correct client authenticates;
- actual received, decoded and rendered video from a private X11 clock display;
- generated500Hz tone travels from host-only Pulse sink to separate client sink,
  verified by RMS/zero crossings on its monitor; no real audio captured;
- normal stream teardown. This is640x360/30 SOFTWARE evidence, not X250/GPU/1080p.
Host test artifact is the OFFICIAL pinned Sunshine AppImage, not a rebuilt Secpar
host. SHA6f80297a853bcf8cda114ff131b5c0a8c9a4d96a2a9c3dee1f04c39c54206846.
Same rejected identity can hit pending-session409 immediately; do not silently
rotate device identities to bypass it. Add bounded cancellation/retry handling.

Android optional gamestream-core library builds original JNI/native core and audio
renderer for ARM64/x86-64. Actual JNI initialization passes on owned emulator5556;
ARM64 ELF uses16KiB alignment. JNI keep rules copied from upstream. Full upstream
MediaCodec/input/NvConnection/activity integration is still missing. Existing APK
DOES NOT depend on this harness or select the new engine.

NEW a9b14d5: crates/secpar-gamestream-host owns a separate Sunshine child, private
state/config/management secret, exclusive lock, exact certificate-pinned HTTPS,
readiness and stop/reap. Windows DACL / Unix0700/0600, bounded replies, redirected
file guards. Uses explicit TLS crypto provider, not ambient global state.
Five Windows and six native Linux tests PASS, including rejecting a different TLS
certificate before sending credentials. REAL isolated supervisor review PASS:
authenticated readiness, duplicate-state refusal, refusal to approve an unbound
backend, stopping one host preserves another, restart retains TLS identity.
It is NOT wired into app hosting or agent authorization yet.

Host::approve requires secpar_identity_binding=1, exact client ID + certificate
SHA256, and forwards those preconditions to the approval POST. Original Sunshine
lacks this binding metadata and is intentionally rejected. Do not remove this gate
or approve by device name/address/global stdin PIN. That API patch is next.

## Next implementation, in order

1. Add/review a small Sunshine MANAGEMENT API patch exposing pending client ID and
   certificate DER SHA256 and checking expected values atomically on PIN approval.
   Keep its media/cryptographic code intact; build/test the patched source before
   allowing the controller's pairing capability. Existing web UI compatibility can
   remain, but Secpar must require the explicit binding capability.
2. Wire broker operations behind the existing authenticated Secpar agent Session.
   Existing core authenticates pair/hello before Handlers::session(device,out).
   Scope grants to that device and session; cancellation/timeouts cannot enroll a
   different client. Add enrollment/revocation journal and kill only revoked client
   sessions. Never pass management credentials to clients or expose a generic API.
3. Supervise process lifetime through crashes (Windows job/platform service control),
   then connect host settings and packaging. Normal child-drop alone is not crash
   containment. Keep separate state/ports; never commandeer installed Sunshine.
4. Launch the private mature client from Secpar computer list and integrate Android
   mature renderer/audio/input and dedicated PiP activity. Preserve owned-account
   flow without another QR/manual PIN round. Keep awake, free window mouse/fullscreen
   capture, clipboard, shortcuts, three viewers and controller isolation.
5. Preserve VPN-free remote access. GameStream has multiple TCP/UDP flows; existing
   PRS single-socket relay is not a drop-in. Reuse mature networking around upstream
   engines instead of writing another media protocol. LAN first, authenticated
   routing, honest direct/relay diagnostics.
6. One optimized app per OS, complete source/license notices, updater compatibility,
   physical S23/X250 tests and all nine desktop host/client directions. Do not
   publish disabled prototypes as a completed migration or claim hardware throughput
   from compilation/JNI/software fixtures. Mac host work remains coordinated above.

## Build and test locations

Windows Java C:/Users/c/.jdks/jbr-21.0.11, Android SDK
C:/Users/c/AppData/Local/Android/Sdk, NDK28.2.13676358. Rust C:/Users/c/.cargo/bin.
Keep builds/temp on D:/Secpar-CI. Cargo test target D:/Secpar-CI/local-target;
CARGO_INCREMENTAL=0. Quote '-Pkotlin.incremental=false' in PowerShell. Write UTF8
bytes/LF; never default Windows text encoding. Shared CI target D:/Secpar-CI/target
can overwrite local release caches: compare staged installer payloads, not later
cached binaries. Use source-pinned short D: checkouts for deep upstream JNI paths.

Owned emulator5556 ONLY for mutable tests. NEVER touch5554. Physical S23 diagnostics
read-only unless user specifically authorizes an install/input action. Never publish
debug APKs or review binaries; never automate UAC approval/security dialogs.

OMV ssh cdsrv, build-only (NOT user's streaming target). Cargo/Rustup/cache:
/opt/secpar-runner/_work/_tool/{cargo,rustup,target}; run compiler as ghrunner.
Docker tests run as root orchestration but user1000 inside isolated container, no
network/caps, read-only artifacts, private tmpfs and no user home/display/devices.
No /dev/dri,/dev/snd,/dev/input, or real desktop mounts. Do not grant runner Docker
privileges, restart OMV services, or use it for physical streaming acceptance.

Private runner sources:
- /tmp/secpar-upstream-client/source: compiled full patched Qt worker.
- /tmp/secpar-upstream-client/Sunshine.AppImage: SHA-verified official fixture.
- /tmp/secpar-upstream-controller/secpar-gamestream-host: native controller source.
- Native controller example /opt/secpar-runner/_work/_tool/target/debug/examples/supervisor_review.
- Docker images secpar-moonlight-build:local / secpar-gamestream-interop:local.
- Older /tmp/secpar-core.JBPqA6/repo: released3.12 desktop/native gates.

Reproduce controller tests: cargo test --manifest-path crates/secpar-gamestream-host/Cargo.toml.
Linux actual supervisor uses integrations/gamestream/supervisor-smoke.sh inside
secpar-gamestream-interop:local, mounting ONLY the example, pinned AppImage and script.
Interop runner: integrations/gamestream/run-interop.sh SUNSHINE_APPIMAGE QT_WORKER.
Preparation/build commands are in integrations/gamestream/README.md.

GitHub CLI C:/Program Files/GitHub CLI/gh.exe is authenticated. Commit identity:
modnite /27398515+modnite@users.noreply.github.com (pass per command, not global).
Cloudflare/Resend secrets are already configured. Do not copy or print their values.

## Usage-triggered handover

User requests handing the FULL project to Claude around10% remaining in the current
five-hour allowance. Read mcp__codex_app__get_usage_limits; primary.usedPercent is
used, not remaining. Latest check71% used /29% remaining, NOT yet at threshold.
Recheck at meaningful milestones. Around90% used: finish a safe checkpoint, push
all authorized code, update this file with exact state/tests/running jobs, supply a
short Claude startup prompt and link. No need to wait until the limit is exhausted.
Do not confuse weekly usage, goal token budget or context length with this window.
