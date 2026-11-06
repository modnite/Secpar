# Secpar desktop application

The desktop application hosts and connects through one interface on Windows. Hosting → Pair a device pairs Android and desktop clients with the same Windows service. The combined Windows installer includes that service and the desktop UI, preserving existing Secpar owner pairings and TLS identity. Android stays client-only. Linux/macOS builds currently connect only; their hosting backends remain unfinished. PC Remote stays separate.

Implemented: pairing links for pinned direct TLS and signed relay bootstrap, OS credential-store persistence for the last paired computer, encrypted UDP video, bounded frame assembly/FEC, duplicate rejection, direct candidate probing, STUN discovery, TURN allocation/channel/permission refresh, Opus audio playback, bounded worker decoding with verified Linux VA-API and OpenH264 fallback to a GPU-rendered window, host-native/720p/1080p/1440p/4K resolution choice, native cursor images, display switching, fullscreen, relative mouse/keyboard, configurable Ctrl+Alt+G cursor capture/release and held-input cleanup on focus loss/disconnect.

The compact window uses the accepted Secpar triangle and wordmark. Computers and Friends are separate tabs. Friends supports browser-based Secpar sign-in, copy/paste friend codes, incoming acceptance, cancellation/removal, blocking and unblocking. Friendship itself grants no PC access; host admission and sharing are still unfinished.

## Run

Requires a Rust toolchain, C/C++ build tools and CMake. Linux additionally needs ALSA and window-system development libraries (for example `libasound2-dev`, `libxkbcommon-dev` and `libwayland-dev` and `libdbus-1-dev` on Debian/Ubuntu).

```sh
cargo run --manifest-path desktop/Cargo.toml
```

Generate a fresh pairing link on Secpar Host, paste it in the client and choose Pair and connect. Pairing may interrupt an existing stream because the current host has one active streaming session. Do not use your only remote connection for the first live test.

The client captures mouse and keyboard when the first decoded video frame arrives while its window has focus. Ctrl+F12 releases/reacquires them. Fullscreen capture hides the toolbar until release. Focus loss releases capture and lifts held remote keys/buttons. Disconnect returns to the connection screen. The last paired computer and account session are saved in Windows Credential Manager, macOS Keychain or the desktop Linux Secret Service. There is no plaintext fallback. If secure storage is unavailable, the session can work in memory and a warning is shown. Only the last paired computer is currently retained. Forget on this device removes the local pairing credential; it does not revoke the phone/desktop pairing at the host. Account sign-out revokes its own server session and forgets its saved credential. Signing out does not erase independent owner pairings.

## Validation and remaining work

The portable client has real loopback TLS/WebSocket/UDP tests for pairing/reconnect, certificate mismatch, encrypted input, audio envelopes and video recovery with packet loss and duplicates. Decoder tests exercise actual H.264 and Opus bitstreams. Input tests cover release cleanup and fractional mouse movement. The same suite runs on the Windows desktop runner, OMV Linux runner and GitHub macOS runner.

These are development checks, not a claim that all hardware/platform combinations are supported. Native rendered screenshots have now been reviewed at 800×600 and 640×400 logical pixels, with no scrolling on the Computers or signed-out Friends pages. A signed-in friend-list fixture also fits at 640×400; it is explicitly marked Layout sample and is not proof of a live friend session. See [visual review](../docs/ui-review.md). Real host-to-client sessions and physical mouse-grab behavior require validation on each target OS. TURN allocation against the production relay has not been tested with this desktop client.

Remaining client work: Windows/macOS hardware decoding, audio/video timing and jitter handling, IME and full physical-key coverage, a full saved-computer list, native signed packaging and live network/performance testing. Linux/macOS hosting, the unified host UI, concurrent guest sessions and host/Android account integration are separate unfinished milestones. Native account approval, restart persistence and friend actions need real-user validation on each target OS. Linux VA-API must pass an actual decoded-frame probe before it is offered; failure during a stream requests a fresh keyframe and switches to software. The X250 driver still requires physical validation.

## Development packages

Windows provides a standalone x64 executable. Linux provides an x64 archive with a user-level launcher installer and the triangle icon; extract it and run `install-user.sh` if you want a desktop entry. The Linux binary is built on the OMV runner, so older distributions may need a source build. macOS provides separate Apple Silicon and Intel app bundles for macOS 13 or later, with the triangle Dock icon. Mac builds are ad hoc signed, not notarized; Windows builds are unsigned. These are client development packages, not unified host/client installers.

## Account sign-in

Choose Friends → Sign in. Secpar opens accounts.secpar.win in your default browser and shows an eight-character approval code. Sign in using your existing email/password account, confirm the matching code, then return to the desktop app. No password is collected or stored by the desktop UI. Approval polling honors the server interval and expires after ten minutes. Cancel stops local polling. Account API errors and sign-in expiry are shown without raw HTTP bodies or credentials.

The native worker pins its account origin, permits HTTPS only, refuses redirects, bounds response bodies and times out requests. Per-user secure storage and network operations run off the UI thread. Windows credential write/read/delete was verified with a disposable test entry; equivalent Linux/macOS runtime checks are still required. Linux needs an unlocked Secret Service in the desktop session (for example GNOME Keyring or KWallet).

## Reproduce layout screenshots

Build the separate `secpar-desktop-review` binary with `--features ui-review`. Set `EFRAME_SCREENSHOT_TO` to an absolute PNG path and `SECPAR_REVIEW_SIZE` to `640x400` or `800x600`. Set `SECPAR_REVIEW_PAGE` to `computers`, `friends` or `friends-sample`. The sample page uses fictional names and says Layout sample. Screenshot mode disables account networking and credential-store access and exits after the native renderer exports the frame. Review code is excluded from ordinary builds.


## Linking the local Windows host

Under Hosting → Settings → Account link, choose Link my account after signing in
under Friends. The Secpar hosting service must be running in the same Windows session.
The host proves possession of its existing TLS key using an account-bound one-use
challenge. The account session stays in the desktop worker. Only a separate,
machine-scoped credential is sent to the host service and protected with Windows
DPAPI in the existing restricted Secpar data directory.

Unlink host revokes that credential and guest leases. It does not change existing
owner pairings. Signing out of the desktop account is separate from unlinking the
host. Friend access remains off until host permission enforcement is complete.
Linux/macOS hosting is still in development. This update must not be installed over
the only working remote connection as part of a development test.

The native screenshot fixture `SECPAR_REVIEW_PAGE=host-sample` shows fictional
linked state without accessing the account service or local host.


## Single Windows application package

Build the desktop Windows binary with `-C target-feature=+crt-static`, copy it to
`host/src-tauri/unified/secpar-desktop.exe`, then run `scripts/build-windows-release.ps1` from the repository root. The one Secpar
installer contains the desktop UI and background service. Normal shortcuts and
sign-in autostart launch the desktop UI. `--service` and `--server` retain the
existing engine; Explorer file offers go through that same service.

Upgrading the host package restarts the service. Do not install through the only
active remote connection. Closing the desktop window hides it in the Windows
tray; Quit Secpar exits the UI while hosting continues in the background.

The desktop executable is an internal component of the single Secpar installation;
it is not published as a separate client application.
Hosting settings currently expose display/quality, autostart, clipboard/files and
UPnP; native file/folder picker controls and network diagnostics still need migration
from the legacy frontend. The legacy frontend remains a fallback if the desktop
binary is absent. It is not a separate host for Android.
