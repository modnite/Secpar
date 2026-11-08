# Secpar on Linux

Secpar is one application for hosting and connecting. The first native Linux host supports owner-paired connections on a local network. KDE Plasma on Wayland is the initial target. Android, Windows and another Linux Secpar client use the same host; no separate phone host is required.

Download Secpar-Linux-x64.AppImage from the Linux release, mark it executable and run it. To install a launcher for your user, run:

```
chmod +x Secpar-Linux-x64.AppImage
./Secpar-Linux-x64.AppImage --install
```

The installer uses your user data directory and does not need administrator access. It installs the transparent triangle icon. Updates are checked in Secpar; the verified download is installed when you click Install update, then Secpar restarts. The installer waits for the old process before replacing the application.

In Hosting, pair the connecting device. On the first stream, KDE asks you to choose a monitor and allow keyboard/pointer control. PipeWire and xdg-desktop-portal-kde must be running. Bundled GStreamer libraries provide hardware encoder candidates and an H264 software fallback; GPU drivers come from your distribution. System audio requires pipewire-pulse and pactl (libpulse on CachyOS). No microphone fallback is used. Your firewall must allow TCP 48999 and the negotiated UDP media port on your trusted LAN.

This is initial Linux hosting: real KDE Wayland streaming is awaiting laptop validation. Linux hosting does not yet provide guest access, unattended login-screen capture, autostart, router ports, display switching or power actions. Those host settings are hidden.

## Phone features on a Linux host

The Android app uses the same messages as for a Windows host (`docs/protocol.md`). The host tells the phone which of these it can do in `welcome.caps`.

| Feature | Cap | How it works on Linux |
|---|---|---|
| Preview | `thumb` | A 320 px wide JPEG of the monitor on the phone's computer list, every 5 s. |
| Media | `media` | Now playing and play, pause, next and previous through MPRIS on the session bus. Volume and mute through `wpctl` (PipeWire), or `pactl` when `wpctl` is missing. |
| Gamepad | `pad` | One virtual Xbox 360 controller per pad id 0 to 3 through `/dev/uinput`, with rumble sent back to the phone. |
| Files | `files`, `fsbrowse` | Send files to this computer, and browse and download from it. |

### Preview

Wayland gives no screenshot without your consent, so the preview reuses the screen sharing permission that your first stream saved. Until you have streamed once, the host sends no preview and asks nothing. If the saved permission was revoked in the KDE settings, the next preview attempt can show the KDE dialog once. The host does not retry after a failure until the phone subscribes again.

While the phone shows the computer list, the host keeps one screen sharing session open, so the KDE sharing indicator shows. It closes when the last phone leaves the list. A stream closes it too, and the preview pauses while any stream runs.

The preview needs the GStreamer `jpegenc` element (gst-plugins-good) and the PipeWire plugin. Without them the host does not offer `thumb`. GStreamer comes from the system, so the AppImage does not change.

### Media

Install a player that supports MPRIS (most do). The phone shows the player that is playing, or else the first one with a track loaded. Play, pause, next and previous do nothing when no player is running. Volume steps are 5 percent. `wpctl` and `pactl` must be in the `PATH` of the Secpar process.

### Gamepad

The host offers `pad` only when it can write to `/dev/uinput`. Hosting settings show one of these lines:

- `Gamepad ready`
- `Gamepad unavailable: no write access to /dev/uinput (see docs/linux.md)`
- `Gamepad unavailable: /dev/uinput is missing (load the uinput kernel module)`

Most distributions give `/dev/uinput` to root only. To allow the users in the `input` group, add the rule below as `/etc/udev/rules.d/60-secpar-uinput.rules`, add your user to that group, then log in again:

```
KERNEL=="uinput", SUBSYSTEM=="misc", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"
```

Run `sudo udevadm control --reload && sudo udevadm trigger /dev/uinput` to apply it without a restart. If the file is missing, run `sudo modprobe uinput`, and put `uinput` in `/etc/modules-load.d/uinput.conf` to load it at every boot. A distribution package for Secpar should ship the rule.

Each controller appears as "Microsoft X-Box 360 pad" (045e:028e), so SDL, Steam and Proton map it without setup. The d-pad is a hat, the triggers are `ABS_Z` and `ABS_RZ`, and the sticks follow the kernel Xbox driver (up is negative). The phone's pad stays plugged in for 5 s after it disconnects and is removed at once when the same phone reconnects.

### Files

Turn on "Receive and browse files" in Hosting, Settings. It is off by default. Received files go to `Secpar` inside your Downloads folder (`XDG_DOWNLOAD_DIR`, from `~/.config/user-dirs.dirs`, else `~/Downloads`). A name clash adds ` (1)`. Interrupted uploads resume from a `.part` file. The host shows a notice through `notify-send` when it is installed.

On the phone, the browse root lists Home, your Desktop, Documents, Downloads, Pictures, Videos and Music folders, and `/`. Dot-files are hidden in listings. Downloading a folder offers every file in it, hidden ones included. The host checks free disk space with `statvfs` before it accepts an upload.

File names from the phone are cleaned the same way as on Windows (characters Windows forbids become `_`), so a file keeps one name on every host. Accounts/friends in the connecting client remain available. macOS hosting is still unfinished; internal Mac client builds are not the finished combined product.

Build an optimized package with `scripts/build-linux-release.sh vX.Y.Z`. It pins and verifies appimagetool 1.9.1 and the 20251108 runtime, strips debug symbols, checks the embedded release tag, bundles media libraries/plugins and includes license notices. Build prerequisites include Rust, Python, pkg-config, binutils, GStreamer development libraries and the app/core/audio/PulseAudio/PipeWire/video/parser/x264 plugins. CI also runs the Linux host tests (synthetic video, a private D-Bus session bus for MPRIS, fake mixer tools and a fake uinput sink) and the shared protocol/client tests.
