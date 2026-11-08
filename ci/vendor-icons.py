#!/usr/bin/env -S uv run --python 3.15
"""Regenerate the application-private fallback theme from pinned COSMIC icons."""
import pathlib
import shutil
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
COMMIT = "f1a282a0822aed1f0083dd2ab2e6c65bd5b401eb"
URL = "https://github.com/pop-os/cosmic-icons.git"
dest = ROOT / "resources/icons/GearLeverCosmic"
if dest.exists():
    raise SystemExit("Refusing to overwrite existing theme. Move it aside before regenerating.")
with tempfile.TemporaryDirectory(prefix="gearlever-icons-") as work:
    source = pathlib.Path(work) / "source"
    subprocess.run(["git", "clone", "--quiet", URL, str(source)], check=True)
    subprocess.run(["git", "-C", str(source), "checkout", "--quiet", COMMIT], check=True)
    dest.mkdir(parents=True)
    for tree in ("freedesktop", "extra"):
        for svg in sorted((source / tree / "scalable").rglob("*.svg")):
            target = dest / "scalable" / svg.relative_to(source / tree / "scalable")
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(svg.resolve(), target)
    shutil.copy2(source / "LICENSE", dest / "LICENSE")
    directories = sorted(str(p.relative_to(dest)) for p in (dest / "scalable").iterdir() if p.is_dir())
    index = "[Icon Theme]\nName=Gear Lever COSMIC\nComment=COSMIC fallback icons for Gear Lever COSMIC\nInherits=breeze,Adwaita,hicolor\nDirectories=" + ",".join(directories) + "\n"
    for directory in directories:
        index += f"\n[{directory}]\nSize=16\nMinSize=8\nMaxSize=512\nType=Scalable\nContext={directory.split('/')[-1].title()}\n"
    (dest / "index.theme").write_text(index)
    (dest / "SOURCE.txt").write_text(f"Cosmic Icons by System76\n{URL}\nCommit: {COMMIT}\nIcon assets: CC-BY-SA-4.0 (see LICENSE).\nScalable icons were combined into an application-private theme.\n")
print(dest)
