"""Render pinned Lucide SVGs without changing their geometry."""
from pathlib import Path
import shutil, subprocess, tempfile
ROOT=Path(__file__).resolve().parents[3]
SOURCE=Path(__file__).resolve().parent
NODE=shutil.which("node") or "C:/Program Files/nodejs/node.exe"
CLI=ROOT/"host/node_modules/@tauri-apps/cli/tauri.js"
PNG=SOURCE/"png";PNG.mkdir(exist_ok=True)
ANDROID=ROOT/"app/src/main/res/drawable-nodpi";ANDROID.mkdir(exist_ok=True)
with tempfile.TemporaryDirectory(prefix="secpar-lucide-") as folder:
    folder=Path(folder)
    for svg in sorted(SOURCE.glob("*.svg")):
        rendered=folder/svg.stem;rendered.mkdir()
        white=folder/svg.name
        white.write_bytes(svg.read_bytes().replace(b"currentColor",b"#ffffff"))
        subprocess.run([NODE,str(CLI),"icon",str(white),"-p","96","-o",str(rendered)],check=True,stdout=subprocess.DEVNULL)
        shutil.copy2(rendered/"96x96.png",PNG/(svg.stem+".png"))
        shutil.copy2(rendered/"96x96.png",ANDROID/("lucide_"+svg.stem.replace("-","_")+".png"))
license_dir=ROOT/"app/src/main/assets/licenses";license_dir.mkdir(parents=True,exist_ok=True)
shutil.copy2(SOURCE/"LICENSE",license_dir/"lucide.txt")
print("Rendered original Lucide SVGs for desktop and Android")
