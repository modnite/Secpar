# Secpar: persistent user requirements

Read this with AGENTS.md and docs/next-milestone.md when resuming work. Additional
prompts add to the task; they do not cancel prior requirements. Later explicit
corrections take precedence over earlier preferences.

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
- Cross-device copy/paste is required. Android streaming currently lacks the
  clipboard integration used by other mode screens; handle focus changes too.
- Audio must recover after silence, pause/resume and switching videos without
  disconnecting. It can also fail after minutes of uninterrupted playback.
  This is still a confirmed fault; previous partial fixes are not
  sufficient. Do not claim success based on a pure timing test alone.
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
