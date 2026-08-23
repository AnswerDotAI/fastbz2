import os, subprocess, sysconfig
from pathlib import Path

import fastbz2

def test_pip_installs_native_cli():
    executable = Path(sysconfig.get_path("scripts")) / ("fastbz2.exe" if os.name == "nt" else "fastbz2")
    assert executable.exists()
    assert not executable.read_bytes().startswith(b"#!")
    result = subprocess.run([executable, "--version"], check=True, capture_output=True, text=True)
    assert result.stdout.strip() == f"fastbz2 {fastbz2.__version__}"
