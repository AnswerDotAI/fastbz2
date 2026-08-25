import subprocess, sys
from pathlib import Path

import maturin

def _stage_cli():
    subprocess.run(["cargo", "build", "--release", "--bin", "fbz"], check=True)
    subprocess.run([sys.executable, "tools/stage_binaries.py"], check=True)

def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    _stage_cli()
    return maturin.build_wheel(wheel_directory, config_settings, metadata_directory)

def build_editable(wheel_directory, config_settings=None, metadata_directory=None):
    _stage_cli()
    return maturin.build_editable(wheel_directory, config_settings, metadata_directory)

def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
    Path("target/wheel-data").mkdir(parents=True, exist_ok=True)
    return maturin.prepare_metadata_for_build_wheel(metadata_directory, config_settings)

build_sdist = maturin.build_sdist
get_requires_for_build_wheel = maturin.get_requires_for_build_wheel
get_requires_for_build_editable = maturin.get_requires_for_build_editable
get_requires_for_build_sdist = maturin.get_requires_for_build_sdist
prepare_metadata_for_build_editable = prepare_metadata_for_build_wheel
