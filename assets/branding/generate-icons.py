"""Render the approved SVG geometry into native platform icon assets."""
from pathlib import Path
import os
import shutil
import subprocess
import tempfile
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
BRAND = ROOT / "assets/branding"
NODE = os.environ.get("NODE_EXE") or shutil.which("node")
if not NODE and os.name == "nt":
    NODE = "C:/Program Files/nodejs/node.exe"
CLI = ROOT / "host/node_modules/@tauri-apps/cli/tauri.js"
if not CLI.is_file():
    raise SystemExit("Install the host build dependencies first")

def render(name, temporary):
    folder = Path(temporary) / name
    subprocess.run([NODE, str(CLI), "icon", str(BRAND / (name + ".svg")), "-p", "512", "-o", str(folder)], check=True)
    return Image.open(folder / "512x512.png").convert("RGBA")

with tempfile.TemporaryDirectory(prefix="secpar-icons-") as temporary:
    desktop = render("secpar-icon", temporary)
    foreground = render("secpar-foreground", temporary)
    android = render("secpar-android", temporary)
    desktop.save(BRAND / "secpar-icon.png")
    icons = ROOT / "host/src-tauri/icons"
    shutil.copyfile(BRAND / "secpar-icon.svg", icons / "icon.svg")
    desktop.save(icons / "icon.png")
    for size, name in [(32,"32x32.png"),(128,"128x128.png"),(256,"128x128@2x.png")]:
        desktop.resize((size,size),Image.Resampling.LANCZOS).save(icons / name)
    desktop.save(icons / "icon.ico",sizes=[(n,n) for n in (16,24,32,48,64,128,256)])
    for density, scale in [("mdpi",1),("hdpi",1.5),("xhdpi",2),("xxhdpi",3),("xxxhdpi",4)]:
        folder = ROOT / "app/src/main/res" / ("mipmap-"+density)
        for image, dp, name in [(foreground,108,"ic_launcher_foreground.png"),(android,48,"ic_launcher.png")]:
            pixels = round(dp*scale)
            image.resize((pixels,pixels),Image.Resampling.LANCZOS).save(folder/name)
    assert desktop.getpixel((256,20))[3] == 0, "Desktop background must be transparent"
    assert android.getpixel((256,20)) == (255,255,255,255), "Legacy Android background must be white"
    assert foreground.getbbox()[3]-foreground.getbbox()[1] <= 250, "Adaptive foreground safe padding"
    print("Native icons generated: transparent desktop, padded Android on white")
