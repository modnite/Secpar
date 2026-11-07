"""Render pinned Lucide SVGs without changing their geometry."""
from pathlib import Path
import shutil, subprocess, tempfile, re
import xml.etree.ElementTree as ET
ROOT=Path(__file__).resolve().parents[3]
SOURCE=Path(__file__).resolve().parent
NODE=shutil.which("node") or "C:/Program Files/nodejs/node.exe"
CLI=ROOT/"host/node_modules/@tauri-apps/cli/tauri.js"
PNG=SOURCE/"png";PNG.mkdir(exist_ok=True)
ANDROID=ROOT/"app/src/main/res/drawable";ANDROID.mkdir(exist_ok=True)
def android_vector(svg):
    paths=[]
    for node in ET.parse(svg).getroot():
        kind=node.tag.rsplit("}",1)[-1];a=node.attrib
        if kind=="path": d=a["d"]
        elif kind=="line":d=f'M {a["x1"]} {a["y1"]} L {a["x2"]} {a["y2"]}'
        elif kind in ("polyline","polygon"):
            points=re.findall(r"[-+]?(?:[0-9]*[.])?[0-9]+",a["points"])
            d="M "+" ".join(points[:2])+" L "+" ".join(points[2:])+(" Z" if kind=="polygon" else "")
        elif kind=="circle":
            x,y,r=(float(a[k]) for k in ["cx","cy","r"])
            d=f"M {x-r:g} {y:g} a {r:g} {r:g} 0 1 0 {2*r:g} 0 a {r:g} {r:g} 0 1 0 {-2*r:g} 0"
        elif kind=="rect":
            x,y,w,h=(float(a.get(k,0)) for k in ["x","y","width","height"])
            rx=min(float(a.get("rx",0)),w/2);ry=min(float(a.get("ry",rx)),h/2)
            if rx and ry:
                d=f"M {x+rx:g} {y:g} H {x+w-rx:g} A {rx:g} {ry:g} 0 0 1 {x+w:g} {y+ry:g} V {y+h-ry:g} A {rx:g} {ry:g} 0 0 1 {x+w-rx:g} {y+h:g} H {x+rx:g} A {rx:g} {ry:g} 0 0 1 {x:g} {y+h-ry:g} V {y+ry:g} A {rx:g} {ry:g} 0 0 1 {x+rx:g} {y:g} Z"
            else:d=f"M {x:g} {y:g} h {w:g} v {h:g} h {-w:g} Z"
        else:raise ValueError("Unsupported SVG primitive: "+kind)
        paths.append(f'    <path android:fillColor="#00000000" android:strokeColor="#FFFFFFFF" android:strokeWidth="2" android:strokeLineCap="round" android:strokeLineJoin="round" android:pathData="{d}"/>')
    return '<!-- Lucide0.468.0, original geometry. ISC license: assets/licenses/lucide.txt -->\n<vector xmlns:android="http://schemas.android.com/apk/res/android" android:width="24dp" android:height="24dp" android:viewportWidth="24" android:viewportHeight="24">\n'+"\n".join(paths)+'\n</vector>\n'

with tempfile.TemporaryDirectory(prefix="secpar-lucide-") as folder:
    folder=Path(folder)
    for svg in sorted(SOURCE.glob("*.svg")):
        rendered=folder/svg.stem;rendered.mkdir()
        white=folder/svg.name
        white.write_bytes(svg.read_bytes().replace(b"currentColor",b"#ffffff"))
        subprocess.run([NODE,str(CLI),"icon",str(white),"-p","96","-o",str(rendered)],check=True,stdout=subprocess.DEVNULL)
        shutil.copy2(rendered/"96x96.png",PNG/(svg.stem+".png"))
        name="lucide_"+svg.stem.replace("-","_")
        (ANDROID/(name+".xml")).write_bytes(android_vector(svg).encode("utf-8"))
        old=ROOT/"app/src/main/res/drawable-nodpi"/(name+".png")
        if old.exists():old.unlink()
license_dir=ROOT/"app/src/main/assets/licenses";license_dir.mkdir(parents=True,exist_ok=True)
shutil.copy2(SOURCE/"LICENSE",license_dir/"lucide.txt")
print("Rendered original Lucide SVGs for desktop and Android")
