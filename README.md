# fastbz2

Fast parallel and indexed bzip2 decompression for Rust and Python.

`fastbz2` is initially focused solely on bzip2: a portable Rust core, a native CLI, and a thin PyO3 seekable-file API. The primary targets are Linux on x86-64 and ARM64, and macOS on ARM64; macOS Intel is best-effort. Correct output, block and stream CRC validation, bounded memory, and deterministic behaviour across thread counts are hard requirements.

The performance floor is end-to-end decompression within 20% of the maintained pure-Rust `libbz2-rs-sys` decoder on a representative corpus. Portable, SIMD-friendly Rust comes first; architecture-specific SIMD is added only when profiles justify it. The much larger Simple English Wikipedia dump is used for local throughput measurements.

The implementation includes a safe structural scanner, an in-repo decoder with a tuned 12-bit Huffman lookup table, CRC-validated block decoding, bounded parallel scheduling, persistent indexes, a native CLI, and a seekable Python file API. Marker scans remain speculative until decoding establishes an exact stream chain and validates block and combined-stream CRCs.

```python
import bz2
from fastbz2 import scan

result = scan(bz2.compress(b"hello"))
result.blocks[0].bit_offset
# 32
```

## Inspiration and credit

The architecture was inspired by Maximilian Knespel's [`librapidarchive`](https://github.com/mxmlnkn/librapidarchive) and [`indexed_bzip2`](https://github.com/mxmlnkn/indexed_bzip2): in particular, scanning for non-byte-aligned bzip2 block markers, independently decoding blocks, ordered prefetch, and indexed seeking. That project's specialised decoder is itself derived from Rob Landley's 0BSD [`bzcat` implementation in Toybox](https://github.com/landley/toybox).

## Development

```bash
pip install -e .[dev]
cargo build --release --bins && python tools/stage_binaries.py
cargo test --release
maturin develop --release && pytest -q
```

Python wheels also install the native `fastbz2` executable directly into the environment's scripts directory; it is not a Python entry point or wrapper.

## Build

```bash
ship-rs-build
```

## Release

```bash
cargo build --release --bins && python tools/stage_binaries.py
maturin develop --release && pytest -q
ship-release
```

`ship-release` tags the Cargo version, leaves wheel publication to GitHub Actions, then bumps the project.
