import gzip, os, subprocess, sysconfig
from pathlib import Path

import fastbz2

def test_pip_installs_native_cli():
    executable = Path(sysconfig.get_path("scripts")) / ("fastbz2.exe" if os.name == "nt" else "fastbz2")
    assert executable.exists()
    assert not executable.read_bytes().startswith(b"#!")
    result = subprocess.run([executable, "--version"], check=True, capture_output=True, text=True)
    assert result.stdout.strip() == f"fastbz2 {fastbz2.__version__}"

def test_pip_installed_cli_decodes_gzip(tmp_path):
    executable = Path(sysconfig.get_path("scripts")) / ("fastbz2.exe" if os.name == "nt" else "fastbz2")
    plain = b"wheel-installed gzip decoder" * 10_000
    source = tmp_path / "sample.gz"
    source.write_bytes(gzip.compress(plain, mtime=0))
    subprocess.run([executable, source], check=True)
    assert source.with_suffix("").read_bytes() == plain
