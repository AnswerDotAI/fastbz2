import gzip, os, subprocess, sysconfig
from pathlib import Path

import fbz

def test_pip_installs_native_cli():
    executable = Path(sysconfig.get_path("scripts")) / ("fbz.exe" if os.name == "nt" else "fbz")
    assert executable.exists()
    assert not executable.read_bytes().startswith(b"#!")
    result = subprocess.run([executable, "--version"], check=True, capture_output=True, text=True)
    assert result.stdout.strip() == f"fbz {fbz.__version__}"

def test_pip_installed_cli_decodes_gzip(tmp_path):
    executable = Path(sysconfig.get_path("scripts")) / ("fbz.exe" if os.name == "nt" else "fbz")
    plain = b"wheel-installed gzip decoder" * 10_000
    source = tmp_path / "sample.gz"
    source.write_bytes(gzip.compress(plain, mtime=0))
    subprocess.run([executable, source], check=True)
    assert source.with_suffix("").read_bytes() == plain
