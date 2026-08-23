# fastbz2

Fast parallel and indexed bzip2 decompression for Rust and Python.

`fastbz2` is initially focused solely on bzip2: a portable Rust core, a native CLI, and a thin PyO3 seekable-file API. The primary targets are Linux on x86-64 and ARM64, and macOS on ARM64; macOS Intel is best-effort. Correct output, block and stream CRC validation, bounded memory, and deterministic behaviour across thread counts are hard requirements.

The first performance target is end-to-end throughput within 20% of `librapidarchive`'s `indexed_bzip2` on the same host and input. Portable, SIMD-friendly Rust comes first; architecture-specific SIMD is added only when profiles justify it.

The decoder is not implemented yet.

## Inspiration and credit

The architecture is inspired by Maximilian Knespel's [`librapidarchive`](https://github.com/mxmlnkn/librapidarchive) and [`indexed_bzip2`](https://github.com/mxmlnkn/indexed_bzip2): in particular, scanning for non-byte-aligned bzip2 block markers, independently decoding blocks, ordered prefetch, and indexed seeking. That project's specialised decoder is itself derived from Rob Landley's 0BSD [`bzcat` implementation in Toybox](https://github.com/landley/toybox). `fastbz2` is intended as a new Rust implementation, with the original project retained as the correctness and performance reference.

## Development

```bash
pip install -e .[dev]
maturin develop && pytest -q
```

## Build

```bash
ship-rs-build
```

## Release

```bash
maturin develop && pytest -q
ship-release
```

`ship-release` tags the Cargo version, leaves wheel publication to GitHub Actions, then bumps the project.
