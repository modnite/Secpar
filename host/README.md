# Secpar host for Windows

The Windows side of Secpar: the background hosting service and the installer. The Secpar desktop app (`desktop/`) is the window and tray; it runs this service as `secpar-desktop.exe --service`.

The service captures the screen with DXGI, encodes with NVENC, AMF, Quick Sync or Media Foundation, and streams to up to three devices at once. It also carries input, controllers, sound, clipboard, files, previews and power actions. It listens on TLS port 48999 and streams on UDP 48998, and keeps its data in `%PROGRAMDATA%\Secpar`. Devices reach it on the local network, directly over the internet, or through the relay.

Owners connect with their own devices. Friends connect with a grant from the account service, and the host enforces what that grant allows (keyboard, mouse, files).

## Build

Rust with MSVC, Node and the Visual Studio C++ tools, then from the repository root:

```
scripts/build-windows-release.ps1 -Features gpu-isolation
```

The installer requires administrator access. It registers the service and its firewall rules, removes folders older versions left behind, and leaves other remote desktop apps alone. Installing restarts hosting, which ends any stream from this computer.
