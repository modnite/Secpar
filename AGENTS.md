# Secpar working agreement

Read `docs/delivery-status.md` when continuing this project. Update it after a
verified milestone so unfinished work survives chat/context changes.
Read `docs/user-requirements.md` for the cumulative user preferences and latest
corrections, including input scope, audio release gates and careful agent usage.

The user wants completed, usable results, regular progress check-ins, and actual
screenshots of the running interfaces. Continue authorized implementation after
answering status questions; an acknowledgement or a plan is not a deliverable.
Never claim a platform or feature works based only on compilation or unit tests.

Keep interfaces compact, simple, flat and clean, inspired by Parsec. Design for
small desktop displays and scaling: normal actions should fit without scrolling
at 800×600 and remain usable at 640×400 logical pixels. Use readable type and
standard controls rather than shrinking text. Avoid oversized cards, decorative
gradients, promotional prose, repeated tips, and placeholder feature buttons.

Android stays client-only. Desktop targets are Windows/Linux/macOS hosting and
connecting, with accounts, friends, explicit guest consent and multiplayer.
Protect the user's existing remote connection: don't replace, stop or restart
installed PC Remote/Secpar services to validate a development build. Keep the
original project's installations and identities separate.

Use the approved mint/orange triangle and Secpar wordmark. Desktop app icons must
use the triangle on transparency; Android icons use BLACK with safe padding
(latest user correction on 2026-11-06 overrides the earlier white background).
Keep the approved geometry and colours when resizing. Never include QR
pairing credentials, account tokens, private keys or personal data in screenshots.

Flat and clean must still look deliberately designed: align control heights,
baselines, shared column edges and section spacing. Use restrained surface contrast,
borders and typography rather than leaving the default toolkit appearance. Apply
layout attention to Android phone and docked/wide windows as well as desktop.

The desktop/Android visual direction in docs/ui-review.md was approved by the user
on 2026-11-05. Preserve it while completing the remaining app-family features.

The desktop product must be ONE application that can host and connect on Windows,
Linux and macOS. Android stays client-only and must connect to the same desktop
hosting engines; there must not be a separate Android-specific host product.
Background services are implementation details inside the desktop installation.

Publish ONE application/package per desktop OS, with both host and client roles.
Do not offer a separate desktop client download alongside a "combined" installer,
or promote client-only Linux/macOS builds as the requested finished applications.
Internal UI/service binaries and CI artifacts are implementation details; the user
must see one Secpar app per OS. Android has one client-only APK.

The user has one Windows PC and a CachyOS laptop for testing. The OMV box is a
build runner/server, not a streaming target. Prioritize Linux hosting next so
Windows↔Linux and Linux↔Linux can be tested; macOS must also eventually support
both roles in its one application. The laptop uses KDE Plasma on Wayland.

Distribution: keep debug/test builds local; publish only optimized release-profile
packages with no debug binaries or test fixtures. Use x.x.x versions: first number
for major milestones, second for features and bug fixes, third for minor fixes and
visual polish. Preserve updater compatibility from the old preview versions.
Ask before installing or launching a new Windows release on this PC. Preserve PC Remote/Sunshine and other independent connections; subagents
must not modify running installations. Conserve agent usage; no more spawning
without a new explicit need/authorization.

Same-account ownership flow requested 2026-11-06: signing into Windows/Linux
must automatically enroll the local host, find/authorize the other owned computers
on LAN without app-level QR/approval prompts, and synchronize all owned/paired
computer access through the account. Android needs account sign-in and the same
computer list/access. Prefer LAN; retain authenticated remote access anywhere.
Keep each client credential separate, preserve revocation/account isolation, and
never treat a shared LAN as proof of account ownership. OS screen-sharing consent
still uses the system portal. This feature remains required after the 3.4 fixes.

Additional 2026-11-06 requirements: public website on secpar.win, taking typography,
layout/spacing inspiration from parsec.app with Secpar's approved logo/colours.
Deploy in the existing Secpar Cloudflare account/repository. Accounts remain on
accounts.secpar.win. No fake platform/features/performance claims or tracking fluff.
Desktop Hosting settings and overall layout need another professional polish pass:
reduce wasted space, improve hierarchy/rows/alignment, test small resolutions.
Allow THREE simultaneous remote streaming devices per host (host + three clients
for four players). Replace phone-centric connection errors with device wording;
report the connection limit when all three slots are occupied. Keep per-client
controllers and sessions independent. Linux client launch/sign-in/streaming to
Windows were confirmed working by the user after the 3.4 launch fix; native Linux
hosting and multiplayer still need real device validation.

Latest user corrections, 2026-11-06: 3.5.0 still refuses a second stream with the
misleading locked-screen/capture error, in either phone/laptop connection order.
Reproduced with real hardware streams: Windows permits only one DXGI duplication
per output/process. Share capture among viewers; slot-count tests alone did not
validate this. Fix and verify actual concurrent video before claiming it works.
UI changes shown in review captures were not yet shipped in 3.5.0.
Required UI: narrower default window, version beside logo, explicit host/client
settings, resolution and FPS labels instead of Smooth/Sharp/Battery saver, and a
configurable one-hand capture-release shortcut without function keys. Ctrl+F12 is
unacceptable. Keep device focus/release safety. Remove website prose/fluff and
model its writing as well as layout on Parsec. Linux client video/cursor performance
is poor during playback; current desktop software OpenH264 decoder is a clear
limitation. Add real hardware decoding with capability-based client choices and
verify on the user's ThinkPad X250/CachyOS KDE Wayland. Detect actual hardware and
driver support; do not pretend a decoder is accelerated because a plugin exists.

Latest validation: Windows 3.6.0 is installed (UI and service). User confirms two
physical clients can stream concurrently and the UI is better. Performance is a
regression on BOTH Android and Linux even with only ONE stream connected. Audio
recovery still fails; do not call it fixed. Prioritize shared host regression and
real audio recovery, then Linux decoder acceleration. Linux wordmark/logo look
jagged: fix high-quality physical-pixel downsampling while retaining approved art.
Additional prompts accumulate; never drop previous scope due to steering.

Read docs/writing-style.md before app/site/help copy changes. User explicitly
requires plain, original writing without canned AI phrasing or promotional fluff.

For Windows Python text writes, specify UTF-8 and LF newlines (or write UTF-8
bytes). Default newline conversion creates broad CRLF diffs; default encoding
can corrupt punctuation. Keep patches focused.

Glyph requirement, 2026-11-07: ALWAYS use an established, named icon/glyph library
as the source for UI symbols. Prefer original library assets with a pinned version
and license; never improvise icons from text characters or ad hoc painter strokes.
Use a consistent family/weight/size across menus. Secpar triangle is approved brand
art, not a generic UI glyph. User rejects current hand-drawn desktop menu symbols.


Current engineering baseline,2026-11-10: the desktop product is one integrated
Rust application. Desktop UI/client code is in desktop/ and hosting code is in
host/src-tauri and crates/secpar-host-linux. The earlier separate upstream-worker
prototype is not the production integration path. Use the latest released source
and docs/delivery-status.md when resuming; older migration notes are historical.
Keep desktop layout aligned with the Android client: three main tabs, narrow
vertical cards and full-width settings groups. Android is a portrait list and a
landscape grid. Linux Wayland system shortcuts belong to the focused stream and
return to the local desktop for local menus or focus loss. Preserve the existing
escape chord and host-side guest permission enforcement.

Keep personal handovers, operational notes and machine-specific continuation
instructions local and untracked. Publish code, licenses and product documentation.
