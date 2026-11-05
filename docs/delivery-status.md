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
  real browser. Sixteen account-service tests now include owner-isolated block lists.

## Active implementation

1. Package and validate desktop sign-in/friend actions on all build targets. Browser approval, friend codes/accept/remove/block/unblock and OS credential storage are implemented in source.
2. Compact desktop layouts reviewed with native-rendered screenshots at 800×600 and 640×400. Host pairing/settings reviewed in the actual frontend with sample state; settings now has zero scrolling at 640×400.
3. Last paired-computer persistence is implemented; the Windows credential-store roundtrip passed using a disposable entry. Native app sign-in and cross-process reconnect still need real-user checks.

## Still required

- Full saved-computer list, real host/client validation and packaged updates.
- Native account integration on host and Android, computer enrollment and presence.
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
- Desktop development 1: https://github.com/modnite/Secpar/releases/tag/v0.1.0-secpar-desktop-dev.1
- Android/host Preview 10: https://github.com/modnite/Secpar/releases/tag/v3.0.3-secpar-preview.10
- Account sign-in: https://accounts.secpar.win/auth/sign-in

This list is a work record, not a claim that the app family is complete.
