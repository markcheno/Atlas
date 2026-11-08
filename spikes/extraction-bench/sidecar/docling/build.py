"""Freezes atlas_docling.py into a standalone folder (PyInstaller onedir),
with the models it needs, and reports the cost.

    uv run build.py             # → build/dist/atlas-docling/, build/size.json

One-file mode is avoided on purpose: it unpacks everything to a temporary
folder on every start, which would make cold starts far worse.
"""

import json
import os
import platform
import shutil
import subprocess
import sys
import time
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
BUILD = HERE / "build"
MODELS = BUILD / "models"
BUNDLE_MODELS = BUILD / "bundle-models"
# Only what the PDF pipeline (layout + tables, no OCR) loads; see docs/spike-0.9.md.
NEEDED = ["docling-project--docling-layout-heron", "docling-project--docling-models"]
EXTRA_COLLECT = ["latex2mathml", "transformers", "torchvision"]


def tree_bytes(p: Path) -> tuple[int, int]:
    files = [f for f in p.rglob("*") if f.is_file()]
    return sum(f.stat().st_size for f in files), len(files)


def main() -> None:
    if not all((MODELS / n).exists() for n in NEEDED):
        subprocess.run([sys.executable, "-m", "docling.cli.tools", "models", "download", "layout", "tableformer", "-o", str(MODELS)], check=True)
    shutil.rmtree(BUNDLE_MODELS, ignore_errors=True)
    for n in NEEDED:
        shutil.copytree(MODELS / n, BUNDLE_MODELS / n)

    started = time.time()
    subprocess.run([
        sys.executable, "-m", "PyInstaller", "--noconfirm", "--clean", "--onedir", "--name", "atlas-docling",
        "--distpath", str(BUILD / "dist"), "--workpath", str(BUILD / "work"), "--specpath", str(BUILD),
        "--collect-all", "docling", "--collect-all", "docling_parse", "--collect-all", "docling_ibm_models",
        "--collect-all", "docling_core",
        # Found by running the frozen build: packages whose data files
        # PyInstaller doesn't collect on its own.
        *[a for pkg in EXTRA_COLLECT for a in ("--collect-all", pkg)],
        "--add-data", f"{BUNDLE_MODELS}{os.pathsep}models",
        str(HERE / "atlas_docling.py"),
    ], check=True)
    build_s = time.time() - started

    dist = BUILD / "dist" / "atlas-docling"
    total, count = tree_bytes(dist)
    models, _ = tree_bytes(BUNDLE_MODELS)
    zipped = BUILD / "atlas-docling.zip"
    with zipfile.ZipFile(zipped, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as z:
        for f in dist.rglob("*"):
            if f.is_file():
                z.write(f, f.relative_to(dist.parent))
    exe = dist / ("atlas-docling.exe" if sys.platform == "win32" else "atlas-docling")
    report = {
        "platform": f"{sys.platform}-{platform.machine()}",
        "python": platform.python_version(),
        "pyinstallerSeconds": round(build_s, 1),
        "onedirBytes": total,
        "onedirFiles": count,
        "modelBytes": models,
        "zippedBytes": zipped.stat().st_size,
        "executable": str(exe),
    }
    (BUILD / "size.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
