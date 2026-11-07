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
PLUGINS = Path(subprocess.check_output(["pkg-config","--variable=pluginsdir","gstreamer-1.0"],text=True).strip())
DEST = ROOT/"dist-desktop/Secpar-Linux-x64.AppImage"
DEST.parent.mkdir(exist_ok=True)
# Drivers, their Wayland ABI and the C runtime must come from the user's distribution.
# ALSA/Pulse/PipeWire also load distribution plugins and configuration; bundling a
# Debian audio library breaks Arch's plugin directory and can silently disable output.
SYSTEM = re.compile(r"^(?:ld-linux|lib(?:c\.so|m\.so|pthread|dl\.so|rt\.so|resolv|util\.so|GL|EGL|OpenGL|GLES|vulkan|drm|gbm|va\.so|va-drm|va-x11|nvidia|cuda|wayland-|asound\.so|pulse|pipewire-|spa-))")
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
    plugin_dir=lib/"gstreamer-1.0";plugin_dir.mkdir()
    names=["app","coreelements","audioconvert","audioresample","pulseaudio","pipewire","videoconvertscale","videorate","videoparsersbad","x264","openh264","va","nvcodec"]
    for name in names:
        src=PLUGINS/("libgst"+name+".so")
        if not src.exists():
            if name in {"va","nvcodec","openh264"}:continue
            raise RuntimeError("Missing required GStreamer plugin: "+name)
        shutil.copy2(src,plugin_dir/src.name);pending.append(src);distributed.append(src)
    scanners=list(Path("/usr/lib").glob("**/gstreamer-1.0/gst-plugin-scanner"))
    if not scanners:raise RuntimeError("GStreamer plugin scanner not found")
    shutil.copy2(scanners[0],exe/"gst-plugin-scanner");pending.append(scanners[0])
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
export LD_LIBRARY_PATH="$APPDIR/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export GST_PLUGIN_SYSTEM_PATH_1_0="$APPDIR/usr/lib/gstreamer-1.0"
export GST_PLUGIN_PATH_1_0="$APPDIR/usr/lib/gstreamer-1.0"
export GST_PLUGIN_SCANNER="$APPDIR/usr/bin/gst-plugin-scanner"
export GST_REGISTRY="${XDG_CACHE_HOME:-$HOME/.cache}/secpar/appimage-gst-registry.bin"
mkdir -p "$(dirname -- "$GST_REGISTRY")"
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
