import os, shutil, sys
from pathlib import Path

root = Path(__file__).resolve().parents[1]
profile = sys.argv[1] if len(sys.argv) > 1 else "release"
source = root / "target" / profile
destination = root / "target" / "wheel-data" / "scripts"
destination.mkdir(parents=True, exist_ok=True)
suffix = ".exe" if os.name == "nt" else ""
shutil.copy2(source / f"fbz{suffix}", destination / f"fbz{suffix}")
