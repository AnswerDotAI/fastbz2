# Development

`fastbz2` is a mixed Rust/PyO3 project. The Rust crate is the implementation and public Rust API; `python/fastbz2/` is the public Python package over the private `fastbz2._core` extension.

## Architecture

```text
src/bitreader.rs      bounded MSB-first in-memory bit reads
src/crc.rs            bzip2 block and combined-stream CRC primitives
src/format.rs         cheap structural scan for header and marker candidates
src/lib.rs            public Rust API and private PyO3 binding
python/fastbz2/       thin Python I/O wrapper over fastbz2._core
tests/                Python API and integration tests
```

The current scanner deliberately does not treat 48-bit marker matches or later `BZh` headers as validated structure. Full decoding must establish the exact block chain and validate every block CRC plus the combined stream CRC before marker candidates can become trusted index entries. Python integration tests use standard-library `bz2`/libbz2 as an independent fixture generator.

The decoder remains independent of files, threads, Python, and the CLI. Parallel scanning/decoding and indexed seeking are layered over it. Native workers never call Python. Large offsets use explicit 64-bit bit/byte types, and speculative block-marker hits are accepted only when they form an exact stream chain with valid block and combined stream CRCs.

Start with safe scalar Rust designed for LLVM auto-vectorisation. Add narrowly scoped unsafe or architecture-specific SIMD only after profiling, with the safe implementation retained as a differential oracle.

## Commands

```bash
cargo test
cargo check --all-features
maturin develop
pytest -q
ship-rs-build
```

Run `cargo fmt --check` after Rust edits and `chkstyle` after Python edits once tests pass.

## Performance acceptance

Build the checked-out `librapidarchive` implementation and compare on the same host, input, output sink, cache condition, and thread counts. Compare several-run medians for single-thread and parallel throughput, scaling, peak RSS, and time to first output. Within 20% end-to-end throughput is good enough when correctness and memory requirements pass.

## Platforms

CI tests and builds Linux on x86-64 and ARM64, and macOS on ARM64. macOS Intel remains best-effort and should not add implementation complexity. Keep the core portable: no required mmap, custom allocator, `io_uring`, assembly, or native-endian parsing. Platform-specific positional I/O belongs behind a small source abstraction.

## Versioning

The canonical version lives in `Cargo.toml`. `pyproject.toml` gets the Python package version from Cargo via `dynamic = ["version"]`.

## Release

1. Run `maturin develop && pytest -q`.
2. Confirm the release version in `Cargo.toml` (`[package].version`).
3. Run `ship-release`.

Fastship pushes the version tag for GitHub Actions, then bumps and pushes `Cargo.toml`.
