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
  and physical Galaxy S23/DeX/QR validation remain pending.

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
  scanning still needs a user check. No physical phone or installed host updated.
  A release instrumentation attempt could not run because AndroidX Test calls a
  tracing method optimized out of the app; release verification used the actual
  UI/camera instead. No instrumentation-only keep rules are shipped.
  Preview 12 is published; both Android assets and both Host Preview 12 assets
  returned HTTP 200 with matching local/GitHub sizes and SHA-256 digests. The
  Android certificate matches Preview 11. The isolated emulator is stopped.

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
- Android built-in QR scanner still needs physical validation on the Galaxy S23; external Camera pairing was confirmed.
- Physical high-DPI reviews on Windows, Linux and macOS; current compact layout checks use controlled logical viewport sizes.
- Guest admission enforced by host (separate from owner tokens), permissions and live revocation.
- Unified desktop host/client interface without enabling hosting implicitly.
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
- Desktop development 4: https://github.com/modnite/Secpar/releases/tag/v0.1.0-secpar-desktop-dev.4
- Android Preview 12: https://github.com/modnite/Secpar/releases/tag/v3.0.3-secpar-preview.12
- Host Preview 12 (tray Quit): https://github.com/modnite/Secpar/releases/tag/v3.0.3-secpar-host-preview.12
- Previous desktop development 3: https://github.com/modnite/Secpar/releases/tag/v0.1.0-secpar-desktop-dev.3
- Previous desktop development 2: https://github.com/modnite/Secpar/releases/tag/v0.1.0-secpar-desktop-dev.2
- Android/host Preview 10: https://github.com/modnite/Secpar/releases/tag/v3.0.3-secpar-preview.10
- Account sign-in: https://accounts.secpar.win/auth/sign-in

This list is a work record, not a claim that the app family is complete.
