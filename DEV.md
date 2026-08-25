# Development

`fastbz2` is a mixed Rust/PyO3 project. The Rust crate is the implementation and public Rust API; `python/fastbz2/` is the public Python package over the private `fastbz2._core` extension.

## Architecture

```text
src/bitreader.rs      bounded MSB-first in-memory bit reads
src/block.rs          independently decodable block construction and validation
src/crc.rs            bzip2 block and combined-stream CRC primitives
src/decode.rs         serial/parallel decode scheduling and index construction
src/decoder.rs        bzip2 block machinery and 12-bit Huffman fast tables
src/format.rs         cheap structural scan for header and marker candidates
src/gzip.rs           gzip framing, LSB-first DEFLATE, CRC32, and block reports
src/output.rs         owned/borrowed decoded-output sink abstraction
src/pipeline.rs       shared ordered, byte-budgeted, staged worker scheduler
src/index.rs           stable persistent index format
src/indexed.rs         seekable decoded view and block cache
src/lib.rs            public Rust API and private PyO3 binding
src/source.rs         owned and memory-mapped compressed sources
src/bin/fastbz2/tar_extract.rs  bounded decode-to-tar bridge, staging, and commit
python/fastbz2/       thin Python I/O wrapper over fastbz2._core
build_backend.py      stage the native CLI for PEP 517 wheel builds
tests/corpus/         selected upstream conformance and corruption fixtures
tests/                Rust CLI/corpus and Python API integration tests
tools/stage_binaries.py copy the release executable into Maturin wheel data
```

The current bzip2 scanner deliberately does not treat 48-bit marker matches or later `BZh` headers as validated structure. Full decoding must establish the exact block chain and validate every block CRC plus the combined stream CRC before marker candidates can become trusted index entries. Python integration tests use standard-library `bz2`/libbz2 as an independent fixture generator.

The gzip decoder is an in-repo RFC 1952/RFC 1951 implementation rather than a wrapper around a production codec. It parses optional headers and concatenated members, decodes stored/fixed/dynamic blocks, maintains the 32 KiB LZ77 history, and validates FHCRC, CRC32, and ISIZE. For large inputs, independently discovered dynamic-block boundaries seed unknown history with compact markers. Primary jobs compute the CRC of each known clean suffix before ordered resolution. Resolution workers resolve the marker prefix, hash that prefix, and combine the two CRCs without rescanning the clean bytes. A marker-free history switches the same decoder to byte output. Reports retain member boundaries, DEFLATE block ranges, and accepted/fallback chunk counts. `crc32fast` is the sole production helper; `flate2` is dev-only.

The decoder remains independent of files, threads, Python, and the CLI. Parallel scanning/decoding and indexed seeking are layered over it. Native workers never call Python. Large offsets use explicit 64-bit bit/byte types, and speculative block-marker hits are accepted only when they form an exact stream chain with valid block and combined stream CRCs.

Both core decode APIs report completed compressed and decoded byte counts without knowing anything about terminals. The CLI selects bzip2 or gzip by a recognised extension and falls back to magic for stdin or unknown names. It layers delayed, rate-limited TTY progress rendering over the shared callbacks; redirected stderr and `--quiet` produce no progress output. Decoded files use same-directory temporary files and atomic persistence, then inherit the compressed input's modification time and permissions. `--rm` removes an input only after decode, persistence, and metadata copying all succeed. An `OutputSink` wrapper enforces output-size limits, so each decoder has one code path for files, stdout, validation, listing, and tar extraction.

Tar format semantics use the mature `tar` crate, pinned from 0.4.46 and built without its optional xattr feature. It handles streaming GNU/PAX/long-name/link entries and confines extracted paths to the destination. A zero-capacity rendezvous channel transfers each owned decoder chunk and its live suffix offset to `tar::Archive`. The channel queues no chunks and applies backpressure. `tar::Archive` pulls data through `Read`, which copies once from the current chunk into its request buffer. Extraction writes immediately into a same-filesystem temporary directory, drains all trailing tar padding so codec validation completes, then preflights every destination conflict and moves entries into place with renames. Multiple inputs remain sequential so their per-codec worker pools cannot oversubscribe the global thread budget.

The shared `pipeline.rs` scheduler provides ordered results, byte-budgeted admission, cancellation, and a staged priority queue. Bzip2 uses the rolling candidate path: workers reserve the maximum possible decoded block size, then shrink that reservation to actual retained output until ordered validation consumes or rejects it. Gzip uses the staged path: native workers alternate speculative DEFLATE decoding with higher-priority marker resolution, while the coordinator advances only the 32 KiB dependency windows and emits resolved chunks in order. Decode results and outstanding resolution results have separate bounded horizons, preventing either dependency stalls or unbounded memory.

The bzip2 decoder is safe scalar Rust designed for LLVM auto-vectorisation. Huffman decoding uses a 4096-entry direct table for codes up to 12 bits and canonical fallback for longer codes. Gzip uses full canonical lookup tables packed into `u16`, a branch-free 64 KiB marker-resolution lookup for large chunks, and `crc32fast::Hasher::combine` so CRC scanning runs with the resolution workers rather than serially in the coordinator. The only unsafe codec operation marks a just-initialized `Vec` result as initialized after writing every spare-capacity byte. Add architecture-specific SIMD or further cross-codec abstraction only after profiling; `libbz2-rs-sys` and `flate2` remain dev-only differential oracles.

## Commands

```bash
cargo test
cargo test --release
cargo check --all-features
cargo build --release --bins
python tools/stage_binaries.py
maturin develop --release
pytest -q
ship-rs-build
```

Run `cargo fmt --check` after Rust edits and `chkstyle` after Python edits once tests pass.

## Correctness and performance acceptance

The normal release test path decodes selected valid and corrupt cases from the maintained upstream `bzip2-testfiles` collection. Generated byte distributions add differential coverage. Valid bzip2 outputs are compared byte-for-byte with `libbz2-rs-sys`. Gzip tests cover stored, fixed-Huffman, and dynamic-Huffman blocks; optional headers and FHCRC; concatenated members; truncation; and trailer corruption across varied inputs and compression levels generated by `flate2`. Both oracles are dev-only and never part of production decoding. CLI tests generate tar archives and cover gzip/bzip2 wrappers, compound-extension dispatch, long names, stdin, raw-tar output, output limits, overwrite preflight, late checksum failure, and traversal confinement.

The normal release path contains warmed end-to-end performance regression gates capped at 1.3 times each oracle, allowing for noise on shared runners. The gzip gates independently exercise a highly compressible LZ77-heavy shape and an incompressible literal-heavy shape against `flate2`; the bzip2 gate uses `libbz2-rs-sys`. Representative local acceptance remains 1.2 times the corresponding oracle. The ignored full-wiki gzip test applies that threshold to rapidgzip-rust. Keep the whole release test suite below five seconds on the primary development laptop; individual timed workloads should normally be about 0.1 seconds or less.

### Local archive extraction benchmarks

`tests/archive_perf.rs` measures the tar layer on the real `meta/simplewiki-first-5pct.xml.bz2` corpus. Fixture decoding and gzip/bzip2 recompression finish before timing. Each ignored test warms one target and measures it once. Run only the implementation changed:

```bash
cargo test --release --test archive_perf tgz_fastbz2_overhead -- --ignored --exact --nocapture
cargo test --release --test archive_perf tgz_system_reference -- --ignored --exact --nocapture
cargo test --release --test archive_perf tbz2_fastbz2_overhead -- --ignored --exact --nocapture
cargo test --release --test archive_perf tbz2_system_reference -- --ignored --exact --nocapture
cargo test --release --test archive_perf tar_crate_reference -- --ignored --exact --nocapture
FASTBZ2_THREADS=18 cargo test --release --test archive_perf tgz_output_cadence -- --ignored --exact --nocapture
```

These are single runs after owned-suffix transfer and the 512 KiB gzip grid change:

| Format | Raw decode | fastbz2 extraction | Extraction/raw | System `tar` | Extraction/system |
|---|---:|---:|---:|---:|---:|
| `.tgz` | 39.919 ms | 56.850 ms | 1.424x | 117.962 ms | 0.482x |
| `.tar.bz2` | 148.411 ms | 151.783 ms | 1.023x | 1.168 s | 0.130x |

Direct extraction of the uncompressed in-memory tar through the `tar` crate took 32.069 ms. Raw gzip decode plus direct tar extraction totals 71.988 ms. The combined pipeline takes 56.850 ms and hides 15.138 ms, or 47%, of the direct tar work.

The cadence benchmark identified ordered gzip output as the main overlap limit. With a 1 MiB speculative grid, output began at 8.491 ms, reached 25% at 29.595 ms, and completed at 33.724 ms. A 512 KiB grid began at 4.798 ms, reached 25% at 23.993 ms, and completed at 32.915 ms. A 256 KiB grid emitted earlier but slowed raw decode to 38.073 ms and extraction to 57.792 ms. The 512 KiB grid gave the best measured balance. Computing each clean suffix CRC in its primary job moved 25% output to 19.818 ms, 75% to 30.355 ms, and completion to 30.678 ms. The corresponding extraction run was effectively flat at 56.850 ms. Tar cannot process later bytes while an earlier ordered gzip segment remains incomplete. A custom tar parser would not remove that dependency. A one-chunk channel buffer regressed extraction to 59.698 ms, so the bridge retains its zero-capacity rendezvous.

System `tar` remains the external reference and 1.2x remains the research target. Raw-tar output is a lower bound rather than an extractor reference. The fastbz2 tests use a broad 3x raw-decode regression guard. Keep the measurements single-run; change an implementation before rerunning it.

Legacy randomized blocks produced by bzip2 versions before 0.9.5 are intentionally unsupported. Supporting that obsolete format would add complexity to the production decoder for data that is not realistically encountered today.

### Local Wikipedia benchmarks

`tests/wiki_perf.rs` contains release-mode local benchmarks that are skipped by default. The git-ignored SimpleWiki fixtures are a bzip2-compressed 5% prefix, the full bzip2 dump, the same 5% prefix recompressed as gzip, and the full XML recompressed with system `gzip -6`.

The quick bzip2 iteration test reads `meta/simplewiki-first-5pct.xml.bz2` before timing, then verifies decoded length, all CRCs, and BLAKE3:

```bash
cargo test --release --test wiki_perf simplewiki_first_five_percent -- --ignored --exact --nocapture
```

The full bzip2 confirmation streams to a counting sink and validates every block and stream CRC:

```bash
cargo test --release --test wiki_perf simplewiki_full -- --ignored --exact --nocapture
```

The fastbz2-only gzip test warms with `meta/simplewiki-first-5pct.xml.gz` and performs one full-dump validation. The ratio test warms both executables, measures each full dump once, and fails above 1.2x the sibling rapidgzip-rust checkout:

```bash
cargo test --release --test wiki_perf gzip_fastbz2_validation -- --ignored --exact --nocapture
cargo test --release --test wiki_perf gzip_reference_ratio -- --ignored --exact --nocapture
```

Set `FASTBZ2_THREADS` to use an explicit worker count. `RAPIDGZIP_BIN` can point at another reference executable. The warm-up is deliberately the small fixture, not an unreported repeat of the measured full workload.

Time/CPU/RSS and ungated macOS physical-footprint diagnostics are separate because process inspection can perturb sub-second parallel timings:

```bash
cargo test --release --test wiki_perf gzip_cli_process_metrics -- --ignored --exact --nocapture
cargo test --release --test wiki_perf rapidgzip_rust_process_metrics -- --ignored --exact --nocapture
cargo test --release --test wiki_perf system_gzip_process_metrics -- --ignored --exact --nocapture
```

The metrics helper uses `wait4` and, on macOS, `proc_pid_rusage` on its own child; it needs no task-inspection permission. Treat its wall time as diagnostic and use `gzip_reference_ratio` for the speed acceptance ratio.
The 1,000-stream enwiki comparison has a separate ignored test for each implementation and mode so a changed decoder can be measured without rerunning unchanged baselines. Each test reads the compressed fixture before starting its single timed decode, with no warmups or repeats:

```bash
cargo test --release --test wiki_perf enwiki_first_1000_fastbz2_parallel -- --ignored --exact --nocapture
cargo test --release --test wiki_perf enwiki_first_1000_crabz2_parallel -- --ignored --exact --nocapture
cargo test --release --test wiki_perf enwiki_first_1000_fastbz2_serial -- --ignored --exact --nocapture
cargo test --release --test wiki_perf enwiki_first_1000_crabz2_serial -- --ignored --exact --nocapture
```

It compares fastbz2 and crabz2 in parallel and serial modes. Set `FASTBZ2_THREADS` to give both parallel decoders an explicit thread count. Each implementation validates the bzip2 CRCs; the benchmark also checks the exact decoded length.

The decoded lengths and the 5% BLAKE3 in `tests/wiki_perf.rs` are acceptance values, not parameters used by the decoder. They prevent a truncated decode from appearing artificially fast without reading a separate multi-gigabyte reference during each timed run. Regenerating a fixture requires independently validating it and updating the corresponding acceptance value.

#### Regenerating the fixtures

Set `wiki` to the checkout containing the Wikimedia dumps:

```bash
wiki=/path/to/parse-wiki
```

Recreate the SimpleWiki fixtures from its validated compressed and decoded files:

```bash
head -c 84423012 "$wiki/data/simplewiki-latest-pages-articles.xml" | bzip2 -9c > meta/simplewiki-first-5pct.xml.bz2
ln -s "$wiki/data/simplewiki-latest-pages-articles.xml.bz2" meta/simplewiki-full.xml.bz2
head -c 84423012 "$wiki/data/simplewiki-latest-pages-articles.xml" | gzip -6c > meta/simplewiki-first-5pct.xml.gz
gzip -6c "$wiki/data/simplewiki-latest-pages-articles.xml" > meta/simplewiki-full.xml.gz
```

Run the 5% test to obtain and verify its decoded length and BLAKE3. Obtain the full decoded length with `stat`; update `FIVE_PERCENT_LEN`, `FIVE_PERCENT_BLAKE3`, or `FULL_LEN` only when intentionally changing a fixture.

The enwiki dump is multistream. Extract its unique compressed stream offsets from the official index, whose first page-bearing stream begins after an unindexed initial stream at byte zero:

```bash
bzcat "$wiki/data/enwiki-latest-pages-articles-multistream-index.txt.bz2" \
  | awk -F: '!seen[$1]++ {print $1}' > "$wiki/meta/enwiki-multistream-offsets.txt"
boundary=$(sed -n '1000p' "$wiki/meta/enwiki-multistream-offsets.txt")
head -c "$boundary" "$wiki/data/enwiki-latest-pages-articles-multistream.xml.bz2" \
  > "$wiki/data/enwiki-first-1000-streams.xml.bz2"
ln -s "$wiki/data/enwiki-first-1000-streams.xml.bz2" meta/enwiki-first-1000-streams.xml.bz2
```

Line 1000 is the start of stream 1001 because byte zero is stream 1 and is absent from the page index. Thus `[0, boundary)` contains exactly 1,000 complete bzip2 streams. For the 2026-08-01 dump, `boundary` is `654362682` and the decoded bzip2 payload length is `2715335085`.

To create the separately useful well-formed parser fixture, append only the XML root close after decoding; those 13 bytes are deliberately excluded from `ENWIKI_1000_LEN`:

```bash
fastbz2 "$wiki/data/enwiki-first-1000-streams.xml.bz2" -o "$wiki/data/enwiki-first-1000-streams.xml"
printf '</mediawiki>\n' >> "$wiki/data/enwiki-first-1000-streams.xml"
xmllint --stream --noout "$wiki/data/enwiki-first-1000-streams.xml"
```

## Platforms

CI tests and builds Linux on x86-64 and ARM64, and macOS on ARM64. macOS Intel remains best-effort and should not add implementation complexity. Keep the core portable: no required mmap, custom allocator, `io_uring`, assembly, or native-endian parsing. Platform-specific positional I/O belongs behind a small source abstraction.

## Versioning

The canonical version lives in `Cargo.toml`. `pyproject.toml` gets the Python package version from Cargo via `dynamic = ["version"]`.

The thin PEP 517 backend delegates to Maturin after building and staging the native executable. This makes wheels built from the sdist contain the same native CLI as CI-built wheels. Release CI verifies that path from a fresh Python 3.14 environment before publishing.

## Release

1. Run `cargo build --release --bins && python tools/stage_binaries.py`.
2. Run `maturin develop --release && pytest -q`.
3. Confirm the release version in `Cargo.toml` (`[package].version`).
4. Run `ship-release`.

Fastship pushes the version tag for GitHub Actions, then bumps and pushes `Cargo.toml`.
