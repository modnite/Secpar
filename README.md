# Secpar

A fork of [PC Remote](https://github.com/SadikTD/pc-remote) for docked Android use and a growing desktop app family. Secpar focuses on physical keyboard/mouse input, resolution handling and compact, plain interfaces. Android stays client-only. Desktop hosting and connecting on Windows, Linux and macOS is the target; the current platform limits are listed below.

## Desktop Development 5

[Windows desktop app](https://github.com/modnite/Secpar/releases/download/v0.1.0-secpar-desktop-dev.5/Secpar-Desktop-dev5-windows-x64.exe) · [Combined Windows installer](https://github.com/modnite/Secpar/releases/download/v0.1.0-secpar-desktop-dev.5/Secpar-dev5-windows-x64-setup.exe) · [Linux client](https://github.com/modnite/Secpar/releases/download/v0.1.0-secpar-desktop-dev.5/Secpar-Desktop-dev5-linux-x64.tar.gz)

Windows hosts and connects through one desktop interface. To pair Android, open **Hosting → Pair a device** and scan its code in Android. Android and desktop clients use the same Windows hosting service. Hosting includes paired devices, display/quality, autostart, clipboard/files and direct-access settings. Account linking is under Hosting → Settings → Account link.

The standalone Windows app manages an already-installed Secpar service without restarting it. The combined installer includes the app and service, preserves existing Secpar pairings and replaces the older Secpar Host listing. **Installing restarts the service; do not install through your only remote connection.** PC Remote remains separate.

Mac Development 5 builds are being checked. Current client downloads: [Mac Apple Silicon (Development 4)](https://github.com/modnite/Secpar/releases/download/v0.1.0-secpar-desktop-dev.4/Secpar-Desktop-dev4-macos-arm64.tar.gz) · [Mac Intel (Development 4)](https://github.com/modnite/Secpar/releases/download/v0.1.0-secpar-desktop-dev.4/Secpar-Desktop-dev4-macos-x64.tar.gz).

Linux/macOS hosting, friend streaming and multiplayer remain unfinished. Native file/folder picker controls and network diagnostics still need migration from the older host frontend. See the [release notes](https://github.com/modnite/Secpar/releases/tag/v0.1.0-secpar-desktop-dev.5) and [implementation/validation limits](desktop/README.md).

Apple Silicon is cross-compiled on an Intel Mac because GitHub's native ARM pool
could not supply a runner. Its ARM64 executable, bundle metadata and ad-hoc signature
were checked; native execution and hardware streaming are still unverified.

![Unified Windows Hosting at 640×400, using sample state](docs/screenshots/desktop-hosting-640x400.png)

Screenshots show rendered interfaces. This image is explicitly a layout sample, not a live user session. [More screenshots and review details](docs/ui-review.md).

## Android Preview 13

[Android APK](https://github.com/modnite/Secpar/releases/download/v3.0.3-secpar-preview.13/Secpar-preview13.apk) · [Release notes](https://github.com/modnite/Secpar/releases/tag/v3.0.3-secpar-preview.13)

Android supports docked keyboard/mouse, cursor capture/release, trackpad fallback, remote pairing links and revised resolution handling. It stays client-only. Preview 12 fixed release scanner initialization; Preview 13 adds cropped decoding and focused still capture for dense QR codes, and directs setup to the desktop Hosting page. The exact host QR link was read through the isolated emulator camera and returned to the app in an instrumented test. Physical Galaxy S23 validation and account/friend integration remain pending. Install over existing previews to preserve pairings. See [Android fork changes](docs/dock-input.md).

These are prereleases, installed manually. Android is debug-signed; Windows builds are unsigned; Mac development bundles are ad hoc signed and not notarized. GitHub's latest-stable endpoint excludes prereleases, so use these direct download links or the repository's Releases page.

## Current capabilities and limits

- **Windows host:** native capture and GPU encoding, audio, input, file/clipboard transfer, optional power and lock-screen controls, and direct/relay routing. The existing engine still has one active stream.
- **Android:** client-only. Physical input and trackpad fallback are retained. Account/friend integration is not implemented yet.
- **Desktop clients:** experimental Windows, Linux and Mac clients, local/remote owner pairing, native rendering, software decode and account/friend management. Real hardware and network validation remains incomplete. Only the last paired computer is currently remembered.
- **Accounts:** email/password registration, verification and browser sign-in are live at [accounts.secpar.win](https://accounts.secpar.win/auth/sign-in) and were confirmed by the user. No passkey or biometric requirement. Native Windows sign-in was also user-confirmed; friend actions and Linux/Mac native sign-in still need real-user validation.
- **Friends and sharing:** explicit friendship is separate from permission to use a PC. The API supports guest requests, permission subsets and short-lived admission grants, and cryptographically proven Windows host enrollment is implemented. Host guest admission and permission enforcement remain unfinished. No friend Connect button or multiplayer claim is presented before that works.
- **Desktop unification:** Windows has one hosting/client UI and combined installer. Linux/macOS hosting is unfinished. All nine desktop host/client combinations plus Android-to-each-host remain the target.

The Secpar background hosting service retains separate ports, protected storage and installer identity from PC Remote. Existing remote profiles can continue using the original rendezvous relay. The Secpar account service does not carry the video stream. Our own relay is reserved for a later migration; switching one side prematurely can strand existing pairings.

The project descends from Moonlight/Sunshine work, but its current PRS streaming engine directly uses Windows capture, encoding, service and input APIs. Portable protocol/client libraries exist; Linux/macOS hosting still requires platform-specific implementation. See the [architecture](docs/desktop-expansion.md) and [delivery record](docs/delivery-status.md).

## Build

- **Android:** Android SDK and a compatible JDK; `./gradlew :app:assembleNonRootRelease`.
- **Windows host:** Rust/MSVC, Node and Visual Studio C++ build tools; run `npm ci` and `npm run tauri build` in host. [Host instructions](host/README.md).
- **Desktop clients:** Rust, C/C++ tools and CMake; `cargo run --manifest-path desktop/Cargo.toml`. Linux additionally needs audio, window-system and DBus development libraries. [Desktop instructions](desktop/README.md).
- **Account service:** Cloudflare Workers + D1 and a verified email sender. [Setup and security boundaries](cloud/accounts/README.md).
- **Rendezvous service:** [Protocol and configuration](docs/remote-protocol.md). Production endpoint migration and TURN provisioning remain separate work.

Windows and Linux CI use dedicated self-hosted runners; Mac builds use GitHub runners. Test success is not a claim that every hardware/platform combination is supported. Avoid testing a new host/client pairing through your only active remote session.

Protocol references: [control](docs/protocol.md), [stream](docs/stream-protocol.md), [remote access](docs/remote-protocol.md).

## License and attribution

GPL-3.0. Original project: [SadikTD/pc-remote](https://github.com/SadikTD/pc-remote). Secpar retains upstream attribution and documents fork changes separately. Secpar is an independent project.
