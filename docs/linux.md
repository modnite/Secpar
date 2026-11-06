# Secpar on Linux

Secpar is one application for hosting and connecting. The first native Linux host supports owner-paired connections on a local network. KDE Plasma on Wayland is the initial target. Android, Windows and another Linux Secpar client use the same host; no separate phone host is required.

Download Secpar-Linux-x64.AppImage from the Linux release, mark it executable and run it. To install a launcher for your user, run:

```
chmod +x Secpar-Linux-x64.AppImage
./Secpar-Linux-x64.AppImage --install
```

The installer uses your user data directory and does not need administrator access. It installs the transparent triangle icon. Updates are checked in Secpar; the verified download is installed when you click Install update, then Secpar restarts. The installer waits for the old process before replacing the application.

In Hosting, pair the connecting device. On the first stream, KDE asks you to choose a monitor and allow keyboard/pointer control. PipeWire and xdg-desktop-portal-kde must be running. Bundled GStreamer libraries provide hardware encoder candidates and an H264 software fallback; GPU drivers come from your distribution. System audio requires pipewire-pulse and pactl (libpulse on CachyOS). No microphone fallback is used. Your firewall must allow TCP 48999 and the negotiated UDP media port on your trusted LAN.

This is initial Linux hosting: real KDE Wayland streaming is awaiting laptop validation. Linux hosting does not yet provide remote relay/account enrollment, guest access, files, clipboard, unattended login-screen capture or autostart. These unavailable host settings are hidden. Accounts/friends in the connecting client remain available. macOS hosting is still unfinished; internal Mac client builds are not the finished combined product.

Build an optimized package with `scripts/build-linux-release.sh vX.Y.Z`. It pins and verifies appimagetool 1.9.1 and the 20251108 runtime, strips debug symbols, checks the embedded release tag, bundles media libraries/plugins and includes license notices. Build prerequisites include Rust, Python, pkg-config, binutils, GStreamer development libraries and the app/core/audio/PulseAudio/PipeWire/video/parser/x264 plugins. CI also runs ten Linux host tests (synthetic video only) and the shared protocol/client tests.
