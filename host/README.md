# Secpar Host for Windows

Windows tray app and background service for the Secpar Android and development desktop clients. It uses the existing native PRS capture/encode/input engine; Sunshine is not required for streaming.

Secpar installs alongside PC Remote with its own identity: service `Secpar`, executable `secpar-host.exe`, TLS port 48999, stream UDP port 48998 and protected data under `%PROGRAMDATA%\Secpar`. Existing PC Remote pairings and services are not migrated or replaced.

The host supports signed, expiring pairing links/QR codes, physical keyboard/mouse, trackpad input, display/resolution negotiation, audio, clipboard/files and direct/relay routing. Owner pairings have broader privileges than friend admission grants. Account login and permission-enforced friend hosting are not integrated yet; the current streaming engine still has one active stream.

## Compact interface

The window opens at 640×480, can resize to 480×400 and adapts settings into two columns at widths of at least 560. Pairing actions sit beside the QR code; click the code to enlarge it, and press Escape to close. The enlarged code follows normal QR refreshes. Home and Settings were reviewed at 640×400 with zero main-panel scrolling and sample state; see [screenshots and validation limits](../docs/ui-review.md).

For a frontend-only review, run `npm run dev` and open `/layout-review.html`. That page supplies sample state with the pairing QR hidden and does not connect to the installed service. It is not the production Vite entry point.

## Build

Requires Windows, Rust/MSVC, Node and the Visual Studio C++ build tools.

```sh
npm ci
npm run build
npm run tauri build
```

For a development installer use `npm run tauri build -- --debug`. Building does not install or restart the running service. Installers are under `src-tauri/target/{debug,release}/bundle/nsis`.

The installer requires administrator access, registers/updates the Secpar service and its firewall rules, and preserves the independent PC Remote installation. Updating an installed Secpar service can interrupt its current stream: do not install a development build over your only remote connection. No installer should run automatically as a build or visual-test step.

The existing installer no longer installs/uninstalls Sunshine or changes shared display adapters and power settings. Keep existing virtual-controller drivers intact. Legacy upstream size/performance figures do not describe current Secpar builds.
