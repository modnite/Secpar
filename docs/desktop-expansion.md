# Desktop expansion

Secpar is being expanded toward one desktop application that can both host and connect. Android remains client-only. Existing Android-to-Windows pairing and the PRS wire format must remain compatible.

## Target support

Every cell is a target, not an assertion that it works today:

| Host → client | Windows | Linux | macOS | Android |
| --- | --- | --- | --- | --- |
| Windows | Planned | Planned | Planned | Existing |
| Linux | Planned | Planned | Planned | Planned |
| macOS | Planned | Planned | Planned | Planned |

The current applications are a Windows host and an Android client. Although the project has Moonlight/Sunshine history, its current streaming engine is custom PRS, not a bundled portable Moonlight/Sunshine engine. The Windows implementation directly uses DXGI, Media Foundation, WASAPI, Windows services, named pipes and Windows input APIs. Changing build targets alone cannot produce working Linux/macOS hosts.

## Shared implementation

`crates/secpar-stream` now owns the existing PRS header, packet encryption, video framing and forward error correction. The Windows host consumes this library through its existing `prs` module. The Kotlin Android receiver remains independent and uses the same protocol. Golden tests compare both encryption directions and every video/FEC shard against the existing Android fixture in `docs/prs-vectors.json`.

The library has no Tauri, Windows service, capture, decoder or input dependency. CI tests it on Windows, Linux and macOS. Passing these tests proves protocol portability only, not a functioning desktop streaming app.

Next, separate the control/identity protocol and connection establishment from the Windows service. Desktop clients need connection/pairing storage, direct UDP/TURN transport, packet reassembly, hardware video decoding/presentation, audio playback, input capture, cursor release and display/resolution negotiation. A webview alone must not become the video or mouse transport.

The desktop UI should present Computers, Friends and This computer. Hosting is an explicit setting; opening the client does not automatically enable inbound control. Do not add active-looking UI for unavailable features.

## Native platform work

| Platform | Host capture/encode | Client decode/presentation | Host input |
| --- | --- | --- | --- |
| Windows | Reuse existing DXGI/Media Foundation engine | Native hardware decode and render | Existing Windows input, adapted for guest permissions |
| Linux | PipeWire/desktop portal on Wayland; separate X11 path; hardware encoder selection | Hardware decode where available, with supported fallback | Desktop RemoteDesktop portal or compositor integration on Wayland; separate X11 path |
| macOS | ScreenCaptureKit and VideoToolbox | VideoToolbox/native presentation | Accessibility-authorized input integration |

Linux compositor support and macOS recording/accessibility grants must be handled explicitly. Do not assume the Windows sign-in screen, UAC or SYSTEM service behavior can be reproduced on other platforms.

Official platform references: [Apple ScreenCaptureKit](https://developer.apple.com/documentation/screencapturekit), [XDG RemoteDesktop portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html).

## Accounts, friends and sharing

Personal device pairing and friend access are different trust relationships. Keep existing paired-device access working, but never turn a friend relationship into an owner token.

An account service will manage authenticated users, device ownership, friend requests, blocks and online availability. Connection invitations must be short-lived, single-use and bound to the intended host, authenticated guest and requested permissions. The host remains responsible for accepting a session and enforcing the grant. Revocation must also terminate an already-open session.

Start with host approval for each guest connection. Permissions should separately cover viewing/audio, gamepad input, keyboard/mouse, clipboard and files. Windows unlock, power controls, unrestricted file browsing and unattended access are owner functions unless explicitly implemented as separately approved permissions. A friend request alone grants none of them.

Multiplayer needs multiple simultaneous authenticated sessions. The current engine has a single active stream, so a friend list alone cannot deliver multiplayer. Reuse capture/encoding where practical, then fan out with independent session encryption, congestion state and guest identity. Allocate independent virtual gamepad seats and release only the departing guest's inputs. Shared keyboard/mouse behavior needs explicit arbitration; multiple guests must not leave keys stuck or reset another guest's controller.

Accounts require a Secpar-controlled backend. The original project's public rendezvous worker can continue to serve existing compatible device connections, but cannot be extended by this fork. Configure our own endpoint before deploying account/friend services. Signing in should not be mandatory for local personal-device pairing.

## Delivery order and validation

1. Extract and verify portable streaming/control components while retaining existing Android-to-Windows behavior.
2. Deliver Windows-to-Windows pairing and streaming in the unified desktop app, including audio, resolution changes, physical input, fullscreen and reliable mouse release.
3. Port the client to Linux and macOS, testing against the Windows host.
4. Port hosting to Linux and macOS and exercise all nine desktop host/client combinations.
5. Deliver accounts, friend consent and guest permissions, then concurrent multiplayer with controller seats. The service design can proceed alongside desktop work.

Do not label a platform supported until a real host/client session has passed end-to-end tests on that OS. For each combination test LAN and remote routing, resize/multi-monitor behavior, keyboard/mouse capture and escape, audio, reconnect, disconnect cleanup and permissions. Multiplayer additionally needs simultaneous physical controllers, guest revocation and packet loss tests. Native platform testing and signed distribution remain required before public stable releases.

This document and the portable stream extraction are the first foundation milestone. They do not add desktop client, Linux/macOS host or account functionality to Preview 10.
