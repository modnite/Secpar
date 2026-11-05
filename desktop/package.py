"""Package existing native builds; does not install or launch them."""
import os
from pathlib import Path
import struct
import plistlib
import shutil
import subprocess
import sys
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parent.parent
TARGET = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "desktop/target"))
DIST = ROOT / "dist-desktop"
DIST.mkdir(exist_ok=True)
kind = sys.argv[1]
if kind not in {"windows", "linux", "macos"}:
    raise SystemExit("Expected windows, linux or macos")
binary = TARGET / "release" / ("secpar-desktop.exe" if kind == "windows" else "secpar-desktop")
if not binary.is_file():
    raise SystemExit("Build the desktop binary before packaging")
if kind == "windows":
    dest = DIST / "Secpar-Desktop-development-windows-x64.exe"
    shutil.copy2(binary, dest)
    print(dest.name)
    raise SystemExit(0)

with tempfile.TemporaryDirectory(prefix="secpar-package-") as temporary:
    stage = Path(temporary) / "Secpar"
    stage.mkdir()
    shutil.copy2(ROOT / "desktop/README.md", stage / "README.md")
    if kind == "linux":
        shutil.copy2(binary, stage / "secpar-desktop")
        (stage / "secpar-desktop").chmod(0o755)
        shutil.copy2(ROOT / "desktop/packaging/install-user.sh", stage / "install-user.sh")
        (stage / "install-user.sh").chmod(0o755)
        shutil.copy2(ROOT / "assets/branding/secpar-icon.png", stage / "secpar-icon.png")
        name = "Secpar-Desktop-development-linux-x64.tar.gz"
    else:
        app = stage / "Secpar.app/Contents"
        (app / "MacOS").mkdir(parents=True)
        (app / "Resources").mkdir()
        shutil.copy2(binary, app / "MacOS/SecparDesktop")
        (app / "MacOS/SecparDesktop").chmod(0o755)
        metadata = {
            "CFBundleIdentifier": "com.secpar.desktop",
            "CFBundleName": "Secpar", "CFBundleDisplayName": "Secpar",
            "CFBundleExecutable": "SecparDesktop", "CFBundlePackageType": "APPL",
            "CFBundleShortVersionString": "0.1.0", "CFBundleVersion": "4",
            "CFBundleIconFile": "Secpar.icns", "NSHighResolutionCapable": True,
            "LSMinimumSystemVersion": "13.0",
            "LSApplicationCategoryType": "public.app-category.productivity",
        }
        with (app / "Info.plist").open("wb") as file:
            plistlib.dump(metadata, file)
        iconset = Path(temporary) / "Secpar.iconset"
        iconset.mkdir()
        source = ROOT / "assets/branding/secpar-icon.png"
        for size in (16, 32, 128, 256, 512):
            for density in (1, 2):
                pixels = str(size * density)
                suffix = "@2x" if density == 2 else ""
                dest = iconset / f"icon_{size}x{size}{suffix}.png"
                subprocess.run(["sips", "-z", pixels, pixels, str(source), "--out", str(dest)], check=True, stdout=subprocess.DEVNULL)
        subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(app / "Resources/Secpar.icns")], check=True)
        # Reject dependencies which are available on the CI machine but absent on a normal Mac.
        links = subprocess.check_output(["otool", "-L", str(app / "MacOS/SecparDesktop")], text=True)
        for entry in links.splitlines()[1:]:
            dependency = entry.strip().split(" ", 1)[0]
            if not dependency.startswith(("/usr/lib/", "/System/Library/")):
                raise RuntimeError("Bundle requires an unbundled library: " + dependency)
        subprocess.run(["codesign", "--force", "--deep", "--sign", "-", str(app.parent)], check=True)
        subprocess.run(["codesign", "--verify", "--deep", "--strict", str(app.parent)], check=True)
        # Derive the label from the packaged Mach-O, including cross builds.
        magic, cpu = struct.unpack("<II", binary.read_bytes()[:8])
        if magic != 0xfeedfacf or cpu not in (0x1000007, 0x100000c):
            raise RuntimeError("Unexpected Mac executable architecture")
        arch = "arm64" if cpu == 0x100000c else "x64"
        name = f"Secpar-Desktop-development-macos-{arch}.tar.gz"
    with tarfile.open(DIST / name, "w:gz") as archive:
        archive.add(stage, arcname="Secpar")
    print(name)
