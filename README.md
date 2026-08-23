# fastbz2

Fast parallel and indexed bzip2 decompression for Rust and Python.

`fastbz2` is initially focused solely on bzip2: a portable Rust core, a native CLI, and a thin PyO3 seekable-file API. The primary targets are Linux on x86-64 and ARM64, and macOS on ARM64; macOS Intel is best-effort. Correct output, block and stream CRC validation, bounded memory, and deterministic behaviour across thread counts are hard requirements.

The performance floor is end-to-end decompression within 20% of the maintained pure-Rust `libbz2-rs-sys` decoder on a representative corpus. Portable, SIMD-friendly Rust comes first; architecture-specific SIMD is added only when profiles justify it. The much larger Simple English Wikipedia dump is used for local throughput measurements.

The implementation includes a safe structural scanner, an in-repo decoder with a tuned 12-bit Huffman lookup table, CRC-validated block decoding, memory-bounded rolling parallel scheduling, persistent indexes, a native CLI, and a seekable Python file API. Marker scans remain speculative until decoding establishes an exact stream chain and validates block and combined-stream CRCs.

## Performance

These are single local release-mode runs on the primary Apple Silicon development machine, using 18 workers for parallel rows. They are observations rather than statistically aggregated benchmarks. Modes are stated per row because a streaming sink, an in-memory `Vec<u8>`, and a CLI pipeline have different allocation and I/O costs; compare rows using the same method most directly.

Full Simple English Wikipedia (`338 MB` compressed, `1,688,460,257` bytes decoded):

| Decoder | Mode | Seconds |
|---|---:|---:|
| fastbz2 | parallel, 18 threads, streaming sink | 2.244 |
| crabz2 0.4.0 | parallel | 4.460 |
| bzip2 | serial CLI | 20.310 |
| pbzip2 1.1.13 | CLI | 20.240 |
| libbz2-rs 0.2.5 | serial, in process | 20.700 |
| fastbz2 | serial, in process | 21.279 |

The first 1,000 streams of English Wikipedia (`654,362,682` bytes compressed, `2,715,335,085` bytes decoded, 99,853 pages) exercise scheduling across many short concatenated streams:

| Decoder | Mode | Seconds |
|---|---:|---:|
| crabz2 0.4.0 | parallel, in process | 3.815 |
| fastbz2 | parallel, 18 threads, in process | 3.881 |
| fastbz2 | serial, in process | 37.198 |
| crabz2 0.4.0 | serial, in process | 40.602 |
| pbzip2 1.1.13 | 18-thread CLI + byte comparison | 88.080 |
| bzip2 | serial CLI + byte comparison | 92.960 |

The CLI rows in the second table stream 2.5 GB through `cmp` against the validated XML, so their absolute times are not directly comparable with the in-process rows. DEV “Local Wikipedia benchmarks” documents exact fixture generation and commands.

Homebrew `pbzip2` 1.1.13 could not safely decompress the complete 26,668,484,995-byte English Wikipedia multistream dump on this machine. It segfaulted, and repeated attempts produced divergent and truncated plaintext. Its successful 1,000-stream result above does not establish full-file reliability.

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
