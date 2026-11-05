# Secpar

## Download Preview 10

[Android APK](https://github.com/modnite/Secpar/releases/download/v3.0.3-secpar-preview.10/Secpar-preview10.apk) · [Windows host installer](https://github.com/modnite/Secpar/releases/download/v3.0.3-secpar-preview.10/Secpar-Host-preview10-triangle-setup.exe) · [Release notes](https://github.com/modnite/Secpar/releases/tag/v3.0.3-secpar-preview.10)

Both apps use the rounded mint-and-orange triangle as their app icon. These direct links work without expanding GitHub’s Assets panel. Update in place to preserve pairings; do not uninstall first. The Windows host installs alongside PC Remote. This is a debug-signed Android preview and unsigned Windows installer.


Secpar is a fork of [PC Remote](https://github.com/SadikTD/pc-remote) focused on docked keyboard and mouse use, resolution handling, and a simpler Android interface. See [fork changes](docs/dock-input.md). Secpar Host now installs alongside PC Remote with its own service, ports and pairing identity.

The Android app is named Secpar; the Android package ID and pairing-link format remain compatible. Secpar Host uses a separate Downloads/Secpar transfer folder.

Control your Windows PC from your Android phone, at home or from anywhere. You can:
- see the PC's screen;
- use the phone as a trackpad, keyboard or gamepad;
- control media and slides;
- move files and whole folders both ways, and browse the PC's files from the phone;
- share the clipboard;
- control the PC at the sign-in screen, the lock screen and admin (UAC) prompts, and unlock it with your phone's fingerprint;
- wake it from anywhere (through Home Assistant), sleep it or shut it down.

Both apps update themselves.

## Desktop expansion

The target is one Secpar app for Windows, Linux and macOS that can host or connect, with accounts, friends and approved multiplayer sharing. Android stays client-only. An experimental [native desktop client](desktop/README.md) is now in development; Linux/macOS hosts and the unified sharing interface remain unfinished. The [account/friend API](cloud/accounts/README.md) is implemented for development and is awaiting deployment and host integration. Preview 10 remains Android-to-Windows. See the [architecture and delivery plan](docs/desktop-expansion.md).

## How it works

- **Phone app and PC app:** a native Android app and a small Windows app. On the PC, a Windows service starts at boot and runs the host as SYSTEM in the console session, so it can see and control the sign-in screen, the lock screen and UAC prompts; the tray window runs as you and talks to it over a local pipe. Both apps are written from scratch for this project.
- **Fingerprint unlock:** the phone keeps your Windows PIN behind its fingerprint (Android Keystore) and the PC types it on the lock screen.
- **Updates:** the PC service installs new releases silently after checking their signature; the phone installs them through Android's package installer (silent on Android 12+ after the first one).
- **Wake from anywhere:** a shut-down PC can only be woken from its own network, so the phone asks Home Assistant to send the magic packet. Set the address and a long-lived token in the phone's Settings, and add `wake_on_lan:` to Home Assistant's configuration. The installer turns off Windows Fast Startup and enables wake-on-LAN on the network adapter; "Wake on LAN" must also be on in the PC's BIOS.
- **Screen streaming:** our own engine. The PC captures the screen and encodes it on the GPU (H.264, HEVC or AV1). The video travels over encrypted UDP with forward error correction, and the phone decodes it in hardware with low-latency settings. On Wi-Fi this is typically about 10 ms from PC to phone.
- **Same network:** the phone finds the PC automatically (mDNS) and connects without any taps.
- **From anywhere:** the PC keeps one idle connection to a small Cloudflare Worker, so the phone can reach it from mobile data. The stream goes direct through UDP hole punching when possible, and through a Cloudflare TURN relay otherwise. See [docs/remote-protocol.md](docs/remote-protocol.md).

## Get it

From [Releases](https://github.com/SadikTD/pc-remote/releases/latest):

- **Windows:** `PC-Remote-Host-3.0.1-setup.exe`. Install it, and it shows a QR code. The only extra piece it installs is the virtual gamepad driver (ViGEmBus).
- **Android:** `PC-Remote-3.0.1.apk`.

Open PC Remote on the phone and scan the code, with the app or any camera app. If the phone can't scan, use **Copy pairing link** on the PC, send the link to the phone, and choose **Paste a pairing link** in the app.

## Build

- **Android:** JDK 17+ and the Android SDK.
  ```
  ./gradlew :app:assembleNonRootRelease
  ```
- **Windows host:** Rust, Node.js and the Visual Studio C++ build tools.
  ```
  cd host && npm install && npx tauri build
  ```
  Then sign the installer for the auto-updater (key in `~/.pcremote/updater.key`, public half in `host/src-tauri/updater.pub`):
  ```
  npx tauri signer sign -f ~/.pcremote/updater.key -p "" PC-Remote-Host-<version>-setup.exe
  ```
  Release assets must be named `PC-Remote-Host-<version>-setup.exe`, `…-setup.exe.sig` and `PC-Remote-<version>.apk`.
- **Rendezvous Worker:**
  ```
  cd cloud/rendezvous && npm install && npx wrangler deploy
  ```
  For the TURN relay, also set the `TURN_KEY_ID` and `TURN_KEY_TOKEN` secrets.

Protocols: [docs/protocol.md](docs/protocol.md) (control), [docs/stream-protocol.md](docs/stream-protocol.md) (video, audio and input), [docs/remote-protocol.md](docs/remote-protocol.md) (remote access).

## License

GPL-3.0.

## Secpar branding

Secpar is an independent fork with parody branding. Its SECPAR wordmark uses rearranged letter outlines sourced from the logo on [Parsec’s official website](https://parsec.app/), with a triangular ribbon in complementary mint and orange. These are exact source letterforms, not an identified full font family. The [brand assets](assets/branding/README.md) include SVG/PNG artwork and a six-letter logo font.
