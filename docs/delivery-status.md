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
  (42079362570). Native desktop/host packages are being prepared for Development 3.

- Update checks now recognize GitHub's expected “no stable release” response and
  offer development downloads, rather than reporting a connection failure. Silent
  updates still require a signed stable release; prereleases are not auto-installed.

- Real workerd/D1 enrollment regression test added to account CI and deployment
  checks. Test tools upgraded after an audit; all 22 tests/type checks/runtime
  smoke passed locally and npm audit reports zero vulnerabilities.

## Active implementation

1. Native Windows desktop sign-in **confirmed by the user on 2026-11-05**: the account appears after browser approval in Desktop Development 2. Friend actions with another real account and restart reconnect are still unverified.
2. Cryptographic host enrollment, restricted machine credentials, local Windows host linking/unlinking and heartbeat are implemented. Verification includes 22 API tests, Rust/Jose signature interoperability, a local Cloudflare/D1 runtime smoke, 14 desktop tests and four host credential tests. Not yet validated against an installed host; friend streaming remains disabled pending host permission enforcement.
3. Compact desktop layouts reviewed with native-rendered screenshots at 800×600 and 640×400. Host pairing/settings reviewed in the actual frontend with sample state; settings now has zero scrolling at 640×400.
4. Last paired-computer persistence is implemented; the Windows credential-store roundtrip passed using a disposable entry. Native Windows sign-in is confirmed; cross-process reconnect and Linux/macOS native sign-in still need real-user checks.

## Still required

- Full saved-computer list, real host/client validation and packaged updates.
- Native account integration on host and Android, cryptographically proven computer enrollment and presence.
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
- Desktop development 2: https://github.com/modnite/Secpar/releases/tag/v0.1.0-secpar-desktop-dev.2
- Android/host Preview 10: https://github.com/modnite/Secpar/releases/tag/v3.0.3-secpar-preview.10
- Account sign-in: https://accounts.secpar.win/auth/sign-in

This list is a work record, not a claim that the app family is complete.
