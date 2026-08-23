import subprocess, sys

import maturin

def _stage_cli():
    subprocess.run(["cargo", "build", "--release", "--bin", "fastbz2"], check=True)
    subprocess.run([sys.executable, "tools/stage_binaries.py"], check=True)

def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    _stage_cli()
    return maturin.build_wheel(wheel_directory, config_settings, metadata_directory)

def build_editable(wheel_directory, config_settings=None, metadata_directory=None):
    _stage_cli()
    return maturin.build_editable(wheel_directory, config_settings, metadata_directory)

build_sdist = maturin.build_sdist
get_requires_for_build_wheel = maturin.get_requires_for_build_wheel
get_requires_for_build_editable = maturin.get_requires_for_build_editable
get_requires_for_build_sdist = maturin.get_requires_for_build_sdist
prepare_metadata_for_build_wheel = maturin.prepare_metadata_for_build_wheel
prepare_metadata_for_build_editable = maturin.prepare_metadata_for_build_editable
