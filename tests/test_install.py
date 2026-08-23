import shutil, subprocess
from pathlib import Path

import fastbz2

def test_pip_installs_native_cli():
    executable = shutil.which("fastbz2")
    assert executable is not None
    assert not Path(executable).read_bytes().startswith(b"#!")
    result = subprocess.run([executable, "--version"], check=True, capture_output=True, text=True)
    assert result.stdout.strip() == f"fastbz2 {fastbz2.__version__}"
