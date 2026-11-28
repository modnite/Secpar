"""Build the Arch Linux package from the same optimized executable as the AppImage.

Runs on Arch (the release build container). Libraries come from the system, so the package depends
on the Arch packages that own what the executable links, plus what it loads at run time: GStreamer
plugins for screen capture, encoding and decoding, and the Wayland/X11 and OpenGL libraries.
"""
import os, re, shutil, subprocess, tempfile
from pathlib import Path
ROOT = Path(__file__).resolve().parents[2]
BINARY = Path(os.environ.get("CARGO_TARGET_DIR", ROOT/"desktop/target"))/"release/secpar-desktop"
TAG = os.environ["SECPAR_RELEASE_TAG"]
if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+(-r[0-9]+)?", TAG):raise RuntimeError("Use a vX.Y.Z or vX.Y.Z-rN release tag")
if TAG.encode() not in BINARY.read_bytes():raise RuntimeError("Build the binary with the selected release tag")
if ".debug_" in subprocess.check_output(["readelf","-S",str(BINARY)],text=True):raise RuntimeError("Debug sections must not be published")
DEST = ROOT/"dist-desktop/Secpar-Linux-x64.pkg.tar.zst"
DEST.parent.mkdir(exist_ok=True)
# pkgver may not contain '-': 4.2.0-r28 becomes 4.2.0.r28, which pacman orders the same way.
VERSION = TAG[1:].replace("-", ".")
# Loaded at run time, so ldd doesn't show them: the pipeline elements (pipewiresrc, pulsesrc,
# videoconvert, x264enc, va*/nvh264enc/openh264enc, vah264dec) and the windowing libraries.
RUNTIME = ["gst-plugins-base", "gst-plugins-good", "gst-plugins-bad", "gst-plugins-ugly", "gst-plugin-pipewire",
           "wayland", "libxkbcommon", "libglvnd", "hicolor-icon-theme"]
OPTIONAL = ["gstreamer-vaapi: VA-API encoding on GPUs the va plugin doesn't cover",
            "libx11: X11 sessions", "libxcursor: X11 sessions", "libxrandr: X11 sessions", "libxi: X11 sessions"]

linked = set()
for line in subprocess.check_output(["ldd", str(BINARY)], text=True).splitlines():
    match = re.search(r"=> (/\S+)", line)
    if match:
        owner = subprocess.run(["pacman", "-Qqo", match.group(1)], text=True, capture_output=True)
        if owner.returncode == 0:
            linked.update(owner.stdout.split())
depends = sorted(linked | set(RUNTIME))

def quote(items):
    return " ".join("'" + i.replace("'", "") + "'" for i in items)

with tempfile.TemporaryDirectory(prefix="secpar-archpkg-") as temp:
    work = Path(temp)
    shutil.copy2(BINARY, work/"secpar-desktop")
    shutil.copy2(ROOT/"assets/branding/secpar-icon.png", work/"com.secpar.desktop.png")
    shutil.copy2(ROOT/"LICENSE.txt", work/"Secpar-GPL-3.0.txt")
    shutil.copy2(ROOT/"assets/icons/lucide/LICENSE", work/"Lucide-ISC.txt")
    (work/"com.secpar.desktop.desktop").write_text("[Desktop Entry]\nType=Application\nName=Secpar\nExec=secpar-desktop\nIcon=com.secpar.desktop\nStartupWMClass=secpar-desktop\nTerminal=false\nCategories=Network;RemoteAccess;\n")
    (work/"PKGBUILD").write_text(f"""pkgname=secpar
pkgver={VERSION}
pkgrel=1
pkgdesc='Remote desktop: host this computer and connect to others'
arch=('x86_64')
url='https://secpar.win'
license=('GPL-3.0-only')
depends=({quote(depends)})
optdepends=({quote(OPTIONAL)})
options=('!strip' '!debug')
package() {{
  install -Dm755 "$startdir/secpar-desktop" "$pkgdir/usr/bin/secpar-desktop"
  install -Dm644 "$startdir/com.secpar.desktop.desktop" "$pkgdir/usr/share/applications/com.secpar.desktop.desktop"
  install -Dm644 "$startdir/com.secpar.desktop.png" "$pkgdir/usr/share/icons/hicolor/512x512/apps/com.secpar.desktop.png"
  install -Dm644 "$startdir/Secpar-GPL-3.0.txt" "$pkgdir/usr/share/licenses/secpar/Secpar-GPL-3.0.txt"
  install -Dm644 "$startdir/Lucide-ISC.txt" "$pkgdir/usr/share/licenses/secpar/Lucide-ISC.txt"
}}
""")
    env = dict(os.environ, PKGDEST=str(work/"out"), BUILDDIR=str(work/"build"), SRCDEST=str(work), PKGEXT=".pkg.tar.zst", HOME=str(work))
    (work/"out").mkdir()
    command = ["makepkg", "--nodeps", "--noconfirm", "--force"]
    # makepkg refuses to run as root; the build container is root.
    if os.geteuid() == 0:
        shutil.chown(work, "nobody")
        for path in work.rglob("*"):
            shutil.chown(path, "nobody")
        command = ["runuser", "-u", "nobody", "--", "env", *[f"{k}={env[k]}" for k in ("PKGDEST", "BUILDDIR", "SRCDEST", "PKGEXT", "HOME")], *command]
    subprocess.run(command, cwd=work, env=env, check=True)
    built = list((work/"out").glob("secpar-*.pkg.tar.zst"))
    if len(built) != 1:raise RuntimeError("makepkg made no package")
    shutil.copy2(built[0], DEST)
    print(DEST)
