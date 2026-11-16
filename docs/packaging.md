# Packaging Secpar 4

One application per operating system: `Secpar-Windows-x64-setup.exe` for Windows
and `Secpar-Linux-x64.AppImage` for Linux. Each is the whole app — window, app
library, background part, Sunshine engine, Moonlight stream component — and each
updates itself. Android ships one client-only APK.

The version is in `version.txt` at the repository root and is read from there by
the window (CMake), the app library (its build script), the Windows installer
and the AppImage. Changing it in one place changes every package and the
updater's idea of the current version.

## Files

| Path | What it does |
| --- | --- |
| `packaging/windows/installer.nsi` | The NSIS script: per-user install, shortcuts, Apps & Features entry, login entry, uninstaller. |
| `packaging/windows/stop-secpar.ps1` | Stops only the parts started from the folder being installed over. |
| `packaging/windows/build-installer.ps1` | Stages the built folder and runs `makensis`. |
| `packaging/linux/build-appimage.sh` | Builds the AppDir and the AppImage, with update information and the `.zsync`. |
| `packaging/linux/appimage-install.sh` | `Secpar-Linux-x64.AppImage --install`: copies the running file to `~/.local/share/secpar`, adds the menu entry and the login entry for the background part. |
| `packaging/linux/THIRD-PARTY.md` | What the package contains from other projects. |

## Windows

```
powershell -File packaging\windows\build-installer.ps1
```

- Installs the whole folder into `%LOCALAPPDATA%\Programs\Secpar` for the current
  person. No administrator rights, so updates never prompt.
- Start menu entry, Apps & Features entry (`HKCU\…\Uninstall\Secpar`), silent
  uninstaller (`Uninstall.exe /S`).
- Starts the background part with `secpar.exe --start-host`, which runs
  `secpar-service.exe --state %LOCALAPPDATA%\Secpar4 --channel secpar-service
  --engine <install>\engine\sunshine.exe --tunnel-port 46900` without a console
  window. The same command is written to
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, so hosting comes up at
  every sign-in.
- Installing over an existing installation keeps the folder (the uninstall
  entry's `InstallLocation`), stops this app's own processes from that folder,
  replaces the files and keeps `%LOCALAPPDATA%\Secpar4` and
  `%LOCALAPPDATA%\Secpar`.
- Uninstalling removes the program files, the shortcut and the registry entries
  and leaves both data folders alone.
- The installer is not code-signed: Windows will show its unknown-publisher
  warning until a signing certificate is bought.

## Linux

```
sh packaging/linux/build-appimage.sh <extracted Linux package folder> <output folder>
```

Layout inside the file: `AppRun`, `com.secpar.desktop`, the icon,
`usr/bin/secpar`, `usr/lib/secpar/` (the package as shipped: `secpar`,
`secpar-service`, `libsecpar_app.so`, `lib/`, `qt6/`, `engine/`, `stream/`),
`usr/share/applications`, `usr/share/icons`, `usr/share/licenses/secpar`.

`AppRun` sets `LD_LIBRARY_PATH` to `usr/lib` and the package's own `lib`, and
`QT_PLUGIN_PATH`/`QML2_IMPORT_PATH` to the bundled Qt 6 plugins and QML modules,
then runs the window. `--install` installs a copy, `--host` runs the background
part. The AppImage carries the update information
`gh-releases-zsync|modnite|Secpar|latest|Secpar-Linux-x64.AppImage.zsync` and its
`.zsync` is produced beside it, so AppImageUpdate can do delta updates too.

### Building on Linux

The Linux release is built against Debian 13, so its glibc baseline is the one
Debian 13 has (2.41), not Arch's. The build image `secpar-linux-deb` is
`debian:trixie` with the distribution's Qt 6.8.2, a Rust toolchain and the
packages Sunshine needs; `secpar-hk` is not used for releases and stays
untouched.

```
docker run --rm \
    -v "$PWD":/src \
    -v /opt/secpar4/scratch-linux-res:/eng:ro \
    -v /opt/secpar4/qt-stream-res:/stream:ro \
    -v "$PWD/dist-desktop":/out \
    -v /opt/secpar4/cargo:/cargo -e CARGO_HOME=/cargo \
    secpar-linux-deb:latest sh -c 'cd /src &&
        sh scripts/build-secpar4-linux.sh \
            /eng/build \
            /stream/out/secpar-stream \
            /out'
```

The arguments are the engine build folder, the stream component and the output
folder; the defaults are the build box's paths. The engine is built with the
same image, which is what makes it runnable on Debian:

```
docker run --rm -v /opt/secpar4/scratch-linux-res:/src -v "$PWD":/repo:ro \
    secpar-linux-deb:latest sh -c 'cd /src && SECPAR_RELOCATABLE=1 \
        sh /repo/scripts/build-sunshine-linux.sh /src'
```

In one run the release script reads `version.txt`, builds `crates/secpar-app`,
`crates/secpar-service` and the window from `desktop-qt/` with CMake and Ninja,
and assembles `build-linux/pkg/secpar4-qt` in the published layout: the four
programs, `stream/secpar-stream`, `engine/` with Sunshine, its assets (both
trees the engine looks in) and its OpenSSL and UPnP libraries, `lib/` with every
library `ldd` resolves from the programs, plugins and QML modules,
`qt6/plugins` and `qt6/qml` (QtQuick with Controls, Layouts and Dialogs,
Qt.labs.platform, QtQml, QtCore, Qt), one font family, the icon, the two
launcher scripts and the licences. Then it calls
`packaging/linux/build-appimage.sh` and prints the size and SHA-256 of the
AppImage and its `.zsync`.

It stops rather than produce something that would not run: a missing part, a
`.debug_` section in any of the five programs, a missing `gh-releases-zsync`
line, an AppImage whose copies differ from the package, or an AppImage that will
not start and draw a window with the build image's Qt plugin and QML folders
moved out of reach. That check saves `started-640x400.png` beside the AppImage;
`SECPAR_SKIP_START_CHECK=1` skips it where moving those folders is not safe.

What the package carries and what it leaves to the machine:

- Bundled, because a bare system has none of it: Qt (all libraries, with their
  second-level dependencies), X11 and xcb, libxkbcommon,
  freetype, fontconfig, harfbuzz, libpng16, the ICU libraries, glib and gobject,
  pcre2, libb2, double-conversion, md4c, libproxy, brotli, liburing,
  libgssapi_krb5, OpenSSL, curl with its whole HTTP/2 and HTTP/3 chain, and
  DejaVu Sans.
- The engine is built from the same pin **without**
  `0003-secpar-live-resolution.patch`, because that patch regressed engine
  start-up on Windows; it is not in this release (see `docs/live-resolution.md`).
  The stream component is built on Debian as well, so its Qt, libplacebo, FFmpeg
  and SDL2 match the rest of the package.
- Before the AppImage is made, the script resolves every program against the
  bundle alone (`LD_BIND_NOW=1 ldd -r`), fails on an undefined symbol or an
  unresolved library, and fails when a dependency resolves outside the bundle
  unless it is on the machine's list: the C runtime, GL and its dispatch
  libraries when the machine has them, Vulkan, DRM, GBM, VA-API,
  ALSA/Pulse/PipeWire/GStreamer, the Wayland server and the streaming stack
  (libplacebo, FFmpeg, SDL2). That gate caught the ngtcp2 family: Debian's core
  had been bundled while the crypto module came from the machine, and the engine
  died with `undefined symbol: ngtcp2_conn_get_remote_transport_params2`. The
  engine's curl chain is now carried whole.
- From the system, because they belong to the machine: the C and C++ runtime,
  the GL loader and the GPU drivers, Vulkan, DRM, GBM, VA-API, ALSA, PulseAudio,
  PipeWire, GStreamer and the Wayland server.
- **The graphics stack and the Wayland client library are the machine's.** Mesa
  (`libEGL*`, `libGLX*`, `libGLES*`, `libOpenGL*`, `libGLdispatch*`), `libgbm*`,
  `libdrm*`, `libvulkan*`, `libva*`, the `gbm/` and `dri/` folders, the Wayland
  client libraries (`libwayland-client`, `libwayland-cursor`, `libwayland-egl`,
  `libwayland-server`), the KDE Wayland client (`libKWaylandClient*`) and the Qt
  eglfs/kms and KDE decoration plugins all come from the desktop. A vendor
  driver only works with the loader, the EGL/GLX modules and the Wayland client
  built with it: a copy from the build image made the packaged engine fail on
  the owner's laptop first with `Couldn't open EGL display: EGL_BAD_PARAMETER`
  and, once Mesa was out but the bundled `libwayland-client` remained, with the
  same EGL error while `libgbm`, `libdrm` and `libva` came from the machine.
  With all of it removed, EGL opened and the engine built an encoder. The
  packaging step removes any of those names wherever they appear and **fails the
  build** if one is left, so nobody can "fix" a rendering problem by bundling
  them again. Qt's own Wayland modules stay in the package; they use the
  machine's `libwayland` at run time. The only exception is `gl/`: four GLVND
  dispatch libraries that AppRun puts on the search path **only** when the
  machine has no `libGLX.so.0` at all, so that the window can still load on a
  bare system (in software).
- **The window draws in software by default.** AppRun sets
  `QT_QUICK_BACKEND=software`, because a Qt that travels with the app cannot
  reliably create a GL context against the machine's vendor drivers, and the
  window is a small 2D interface: this costs only the window's own drawing, and
  the picture inside a stream is decoded by the stream component, not here.
  `SECPAR_RENDERER=gl`, or setting `QT_QUICK_BACKEND` yourself, tries hardware
  rendering instead.
- **It runs from a writable copy, not from the mount.** AppRun extracts the
  payload once per version into `$XDG_CACHE_HOME/secpar/app-<version>`
  (`~/.cache/secpar/app-4.0.0` by default) and starts everything from there,
  including the background part; later starts reuse that copy and copies of
  other versions are deleted. Started straight from the compressed mount, the
  engine stopped right after its KMS monitor list, while the same binary from an
  extracted folder went on to open EGL and find an encoder. `--install` still
  runs from the mounted file, because the file itself is what gets copied into
  `~/.local/share/secpar`.
- Fonts: a machine that has `/etc/fonts/fonts.conf` keeps its own fontconfig and
  its own fonts; only a system without any gets the small configuration AppRun
  writes, which points at the bundled DejaVu Sans. The window also takes its
  interface family from `~/.config/kdeglobals` `[General] font=`, so a Plasma
  desktop shows its own font.

Two runs from clean copies give the same size but different hashes, because the
squashfs records file times.


### A clean Debian without Qt

`debian:trixie` is the acceptance test: no Qt, no build tools, no desktop,
nothing installed.

```
docker run --rm \
    -v "$PWD/dist-desktop/Secpar-Linux-x64.AppImage":/app/Secpar-Linux-x64.AppImage:ro \
    -v "$PWD/shots":/shots \
    debian:trixie sh -c 'for size in 640x400 1280x720; do \
        APPIMAGE_EXTRACT_AND_RUN=1 QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software \
        /app/Secpar-Linux-x64.AppImage --sample --size $size --tab 0 \
            --screenshot /shots/debian-$size.png; done'
```

This passes: exit code 0 at both sizes, each with a real window in its
screenshot (`appimage-debian-trixie-640x400.png`). Nothing is installed in the
test container. `QT_QUICK_BACKEND` does not have to be given — AppRun already
chooses the software backend — so the check proves that Qt and the runtime
libraries travel with the app while the window still starts on a system with no
GL and no Qt; hardware rendering, where it works, comes from the desktop. What each program asks of glibc: `secpar` 2.34, `secpar-service` and
`libsecpar_app.so` 2.39, the stream component 2.34, the engine 2.38 — all at or
below Debian 13's 2.41.

A launched window also starts the packaged background part itself when nothing
answers on `$XDG_RUNTIME_DIR/secpar-service.sock`, with
`--state $HOME/.local/share/Secpar4 --engine <package>/engine/sunshine
--tunnel-port 46900` — the same state folder, channel and port as
`packaging/linux/start-hosting.sh`, so a computer keeps one identity whichever
started it. In a bare container the service exits with
`device identity: StoreUnavailable` because there is no credential store; the
window's start-up is visible from the state folder it creates. On a desktop with
KWallet or gnome-keyring the service stays up and answers.

The AppImage has no `--version` flag of its own; `--appimage-version` prints the
runtime's version.

## Updating from inside the app

`crates/secpar-app/src/update.rs` (reached from the window as `update_check` and
`update_install`) reads
`https://api.github.com/repos/modnite/Secpar/releases?per_page=100` and uses a
release only when it is published, not a preview, newer than this copy, and has
an asset with this system's name at the release's own download address with a
size and `sha256:` digest. The download is hashed as it is written and is
deleted unless both match.

- Windows: the installer is started two seconds later by a detached helper (so
  the window can close and free its files) with `/S /RESTART`, which installs
  over the same folder and reopens the window.
- Linux: the file named by `$APPIMAGE` is replaced by writing a copy beside it
  and renaming it over the original, then the new copy is started one second
  later. If the app was not started from an AppImage, it says so and changes
  nothing.
- Settings → About shows the version, the state (Up to date, Update available
  vX.Y.Z, Downloading, Restart to update) and asks once before installing,
  because installing ends a stream.

Setting `SECPAR_UPDATE_ORIGIN` to a `http://127.0.0.1:…` address points a test
build at a feed on the same computer; any other address is ignored.

## Checking the update path without GitHub

`crates/secpar-app/tests/updates.rs` serves a feed and an asset from a socket on
127.0.0.1 and checks: a newer release is offered, the same or an older one is
not, the downloaded bytes are the served ones, a wrong SHA-256 or size is
refused with nothing left on disk, and (Linux) the file the app was started from
is replaced and stays executable.

```
SECPAR_UPDATE_TEST_ASSET=<built installer or AppImage> cargo test --test updates -- --nocapture
```

Without that variable a small stand-in file is served.

## Android

```
python scripts/build-android.py <pinned moonlight-android> <work folder> <folder with libsecpar_android.so> assembleNonRootRelease
```

The version and the Android version code both come from `version.txt`: 4.0.0
becomes `versionName 4.0.0`, `versionCode 40000`. Android will not install a
build whose version code is not higher than the installed one, so the code must
follow the release rather than Moonlight's own counter (which it replaced: 315).

After the Gradle task, the unsigned release APK is aligned and signed with
Secpar's own key (kept outside the repository; `~/.pcremote/keystore.properties`
for the 3.x app is a different identity):

```
zipalign -p -f 4 app-nonRoot-release-unsigned.apk Secpar-4.0.0.apk
apksigner sign --ks <secpar-release.jks> --ks-key-alias secpar --ks-pass pass:<pw> Secpar-4.0.0.apk
```

Never publish the debug variant: it is signed with the debug key, carries the
`(test)` label and cannot be updated over.
