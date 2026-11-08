"""Build the one Linux host/client AppImage from an optimized executable."""
import os, re, shutil, subprocess, tempfile
from pathlib import Path
ROOT = Path(__file__).resolve().parents[2]
BINARY = Path(os.environ.get("CARGO_TARGET_DIR", ROOT/"desktop/target"))/"release/secpar-desktop"
TOOL = Path(os.environ["SECPAR_APPIMAGETOOL"])
RUNTIME = Path(os.environ["SECPAR_APPIMAGE_RUNTIME"])
TAG = os.environ["SECPAR_RELEASE_TAG"]
if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", TAG):raise RuntimeError("Use a plain vX.Y.Z release tag")
if TAG.encode() not in BINARY.read_bytes():raise RuntimeError("Build the binary with the selected release tag")
if ".debug_" in subprocess.check_output(["readelf","-S",str(BINARY)],text=True):raise RuntimeError("Debug sections must not be published")
DEST = ROOT/"dist-desktop/Secpar-Linux-x64.AppImage"
DEST.parent.mkdir(exist_ok=True)
# GStreamer and its GLib come from the system too: the PipeWire plugin has to match the PipeWire in use.
# Drivers, their Wayland ABI and the C runtime must come from the user's distribution.
# ALSA/Pulse/PipeWire also load distribution plugins and configuration; bundling a
# Debian audio library breaks Arch's plugin directory and can silently disable output.
SYSTEM = re.compile(r"^(?:ld-linux|lib(?:c\.so|m\.so|pthread|dl\.so|rt\.so|resolv|util\.so|GL|EGL|OpenGL|GLES|vulkan|drm|gbm|va\.so|va-drm|va-x11|nvidia|cuda|wayland-|asound\.so|pulse|pipewire-|spa-|gst|glib-2|gobject-2|gio-2|gmodule-2|orc-0))")
with tempfile.TemporaryDirectory(prefix="secpar-appimage-") as temp:
    app=Path(temp)/"Secpar.AppDir"; lib=app/"usr/lib"; exe=app/"usr/bin"
    lib.mkdir(parents=True);exe.mkdir()
    shutil.copy2(BINARY,exe/"secpar-desktop")
    shutil.copy2(ROOT/"assets/branding/secpar-icon.png",app/"com.secpar.desktop.png")
    shutil.copy2(ROOT/"desktop/packaging/appimage-install.sh",exe/"secpar-install")
    (exe/"secpar-install").chmod(0o755)
    (app/"com.secpar.desktop.desktop").write_text('[Desktop Entry]\nType=Application\nName=Secpar\nExec=secpar-desktop\nIcon=com.secpar.desktop\nTerminal=false\nCategories=Network;RemoteAccess;\nX-AppImage-Version='+TAG[1:]+'\n')
    licenses=app/"usr/share/licenses/secpar";licenses.mkdir(parents=True)
    shutil.copy2(ROOT/"LICENSE.txt",licenses/"Secpar-GPL-3.0.txt")
    shutil.copy2(ROOT/"assets/icons/lucide/LICENSE",licenses/"Lucide-ISC.txt")
    pending=[BINARY]
    distributed=[BINARY]
    copied=set()
    while pending:
        src=pending.pop()
        links=subprocess.check_output(["ldd",str(src)],text=True)
        if "not found" in links:raise RuntimeError("Missing shared dependency for "+str(src))
        for line in links.splitlines():
            match=re.search(r"^\s*(\S+) => (/\S+)",line)
            if not match:continue
            name,path=match.groups()
            if SYSTEM.match(name) or name in copied:continue
            copied.add(name);shutil.copy2(path,lib/name);pending.append(Path(path));distributed.append(Path(path))
    packages=set()
    for source in distributed:
        result=subprocess.run(["dpkg-query","-S",str(source.resolve())],text=True,capture_output=True)
        for line in result.stdout.splitlines():
            if ": " in line:packages.add(line.split(": ",1)[0])
    for package in packages:
        copyright=Path("/usr/share/doc")/package.split(":")[0]/"copyright"
        if copyright.exists():shutil.copy2(copyright,licenses/(package.replace(":","-")+"-copyright.txt"))
    (licenses/"Debian-packages.txt").write_text(subprocess.check_output(["dpkg-query","-W","-f=${binary:Package} ${Version} (source: ${source:Package} ${source:Version})\\n",*sorted(packages)],text=True))
    (app/"AppRun").write_text("""#!/bin/sh
set -eu
APPDIR=${APPDIR:-$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)}
export APPDIR
# GStreamer comes from the system, not the bundle: its PipeWire plugin has to match the PipeWire in use
# (a bundled one fails desktop capture with "error alloc buffers" on current PipeWire).
system_gst=""
for dir in /usr/lib /usr/lib64 /usr/lib/x86_64-linux-gnu; do
    if [ -e "$dir/libgstreamer-1.0.so.0" ] && [ -e "$dir/gstreamer-1.0/libgstpipewire.so" ]; then system_gst=$dir; break; fi
done
if [ -z "$system_gst" ]; then
    echo "Secpar needs GStreamer with its PipeWire plugin installed (gst-plugins-base, gst-plugin-pipewire; for hosting also the x264 or VA-API encoder plugins)." >&2
    exit 1
fi
export LD_LIBRARY_PATH="$system_gst:$APPDIR/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
if [ "${1:-}" = --install ]; then
    shift
    exec "$APPDIR/usr/bin/secpar-install" "$@"
fi
exec "$APPDIR/usr/bin/secpar-desktop" "$@"
""")
    (app/"AppRun").chmod(0o755)
    env=dict(os.environ,ARCH="x86_64",APPIMAGE_EXTRACT_AND_RUN="1")
    subprocess.run([str(TOOL),"--runtime-file",str(RUNTIME),"--no-appstream",str(app),str(DEST)],check=True,env=env)
    print(DEST)
