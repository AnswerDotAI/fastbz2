import os, shutil
from pathlib import Path

root = Path(__file__).resolve().parents[1]
source = root / "target" / "release"
destination = root / "target" / "wheel-data" / "scripts"
destination.mkdir(parents=True, exist_ok=True)
suffix = ".exe" if os.name == "nt" else ""
shutil.copy2(source / f"fastbz2{suffix}", destination / f"fastbz2{suffix}")
