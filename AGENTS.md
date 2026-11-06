# Secpar working agreement

Read `docs/delivery-status.md` when continuing this project. Update it after a
verified milestone so unfinished work survives chat/context changes.

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
use the triangle on transparency; Android icons use white with safe padding.
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
The user explicitly permits root to install and launch new Windows releases on
this PC. Preserve PC Remote/Sunshine and other independent connections; subagents
must not modify running installations. Conserve agent usage; no more spawning
without a new explicit need/authorization.
