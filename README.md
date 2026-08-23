# fastbz2

Fast parallel and indexed bzip2 decompression for Rust and Python.

`fastbz2` provides a native CLI, a Rust library, and a Python module. It handles ordinary and concatenated bzip2 streams, validates every block and stream CRC, and keeps speculative parallel output within a configurable memory bound. Persistent indexes support efficient random access from Python without first expanding the whole file.

This project decompresses bzip2; it does not compress it.

## Install

PyPI wheels contain both the Python module and the native `fastbz2` executable—there is no Python CLI wrapper:

```bash
pip install fastbz2
```

Python 3.10 and later are supported. Prebuilt wheels target Linux on x86-64 and ARM64, and macOS on ARM64. macOS Intel is best-effort and can build from source.

The Rust crate is not yet published separately on crates.io. Install the CLI from the repository, or add the library as a Git dependency:

```bash
cargo install --git https://github.com/AnswerDotAI/fastbz2
cargo add fastbz2 --git https://github.com/AnswerDotAI/fastbz2
```

## CLI

Decoding is the default operation. A `.bz2` suffix is removed for the output name; other input names gain `.out`.

```bash
fastbz2 dump.xml.bz2                 # write dump.xml
fastbz2 dump.xml.bz2 -o result.xml   # choose the output path
fastbz2 dump.xml.bz2 -o -            # write plaintext to stdout
fastbz2 -                             # read compressed data from stdin
```

Multiple inputs are decoded in order, with parallelism applied inside each file. `-C/--output-dir` collects their outputs in one directory:

```bash
fastbz2 *.bz2 -C decoded
fastbz2 *.bz2 -C decoded --skip-existing
```

The alternative modes are flags rather than subcommands:

```bash
fastbz2 --test dump.xml.bz2          # fully decode and validate, writing nothing
fastbz2 --index dump.xml.bz2         # write dump.xml.bz2.fbz2i
fastbz2 --list dump.xml.bz2          # print the validated stream/block layout
fastbz2 --list --json dump.xml.bz2   # emit the complete layout as JSON
```

`--test`, `--index`, and `--list` are mutually exclusive. Human-readable `--list` output labels each input when given multiple files; JSON output is one object for one input and an array for multiple inputs.

### Output safety

- Existing outputs are rejected by default. Use `--force` to replace them or `--skip-existing` to leave them untouched.
- File outputs are written to a temporary file in the destination directory and persisted atomically only after successful CRC validation.
- Extracted files inherit the compressed input's permissions and modification time.
- `--rm` removes each compressed input only after its output has been persisted and its metadata copied successfully.
- `--max-output SIZE` limits decoded bytes per input. Sizes accept binary suffixes such as `K`, `MiB`, and `G`.

Long interactive operations report completion, decoded throughput, compression ratio, and ETA on stderr. Progress is disabled automatically when stderr is redirected; `-q/--quiet` also suppresses progress and skip notices.

`-P/--threads 0`, the default, uses all available CPUs. `--memory-limit` bounds speculative decoded output and defaults to `1G`.

## Python

### One-shot decompression and validation

```python
import fastbz2

plain = fastbz2.decompress(compressed_bytes)
fastbz2.test("dump.xml.bz2")  # returns None after successful validation
```

`decompress` accepts a bytes-like object and returns `bytes`. `test` accepts either compressed bytes or a path and avoids retaining the decoded result.

### Seekable reads and persistent indexes

`fastbz2.open` returns a seekable binary `io.RawIOBase`. Opening without an index performs a complete validation pass and builds an in-memory block index; `build_index` can persist that work for later processes:

```python
import fastbz2

fastbz2.build_index("dump.xml.bz2", "dump.xml.bz2.fbz2i")

with fastbz2.open("dump.xml.bz2", index="dump.xml.bz2.fbz2i") as f:
    f.seek(1_000_000_000)
    chunk = f.read(64 * 1024)
    print(f.tell(), f.size)
```

Building an index fully decodes into a sink but does not write or retain the plaintext. Indexes contain compressed and decoded block offsets and are bound to the exact compressed source by its length and BLAKE3 hash. Loading one verifies that identity without decoding the whole payload; subsequent reads decode only the blocks needed for the requested range and cache recent blocks. `cache_limit` controls that cache. Path sources are memory-mapped, while bytes-like sources stay in memory.

### Structural scanning

`scan` cheaply finds candidate stream headers and bit-level block markers without decoding:

```python
import bz2
from fastbz2 import scan

result = scan(bz2.compress(b"hello"))
assert result.blocks[0].bit_offset == 32
```

Scan results are deliberately untrusted candidates. Use `test`, `decompress`, `build_index`, or `open` when validation is required.

## Rust

The streaming API accepts any `Write` destination and uses the serial fast path when `threads` is one:

```rust
use fastbz2::{DecodeOptions, Source, decompress_to_writer};

fn main() -> fastbz2::Result<()> {
    let source = Source::open("dump.xml.bz2")?;
    let mut output = std::io::stdout().lock();
    decompress_to_writer(source.as_slice(), &mut output, DecodeOptions::default())?;
    Ok(())
}
```

`decompress` returns a `Vec<u8>`. `decode_to_writer` returns a validated `Index` while streaming output, `build_index` validates into a sink, and their `*_with_progress` variants report completed compressed and decoded byte counts. `IndexedReader` implements `Read` and `Seek`; it can build an index itself or load a persisted one with `open_with_index`.

## Performance

These are single local release-mode runs on the primary Apple Silicon development machine, using 18 workers for parallel rows. They are observations rather than statistically aggregated benchmarks. Modes are stated per row because a streaming sink, an in-memory `Vec<u8>`, and a CLI pipeline have different allocation and I/O costs; compare rows using the same method most directly.

Full Simple English Wikipedia (`338 MB` compressed, `1,688,460,257` bytes decoded):

| Decoder | Mode | Seconds |
|---|---|---:|
| fastbz2 | parallel, 18 threads, streaming sink | 2.244 |
| crabz2 0.4.0 | parallel | 4.460 |
| bzip2 | serial CLI | 20.310 |
| pbzip2 1.1.13 | CLI | 20.240 |
| libbz2-rs 0.2.5 | serial, in process | 20.700 |
| fastbz2 | serial, in process | 21.279 |

The first 1,000 streams of English Wikipedia (`654,362,682` bytes compressed, `2,715,335,085` bytes decoded, 99,853 pages) exercise scheduling across many short concatenated streams:

| Decoder | Mode | Seconds |
|---|---|---:|
| crabz2 0.4.0 | parallel, in process | 3.815 |
| fastbz2 | parallel, 18 threads, in process | 3.881 |
| fastbz2 | serial, in process | 37.198 |
| crabz2 0.4.0 | serial, in process | 40.602 |
| pbzip2 1.1.13 | 18-thread CLI + byte comparison | 88.080 |
| bzip2 | serial CLI + byte comparison | 92.960 |

The CLI rows in the second table stream 2.5 GB through `cmp` against the validated XML, so their absolute times are not directly comparable with the in-process rows. [DEV.md](DEV.md#local-wikipedia-benchmarks) documents exact fixture generation and commands.

Homebrew `pbzip2` 1.1.13 could not safely decompress the complete 26,668,484,995-byte English Wikipedia multistream dump on this machine. It segfaulted, and repeated attempts produced divergent and truncated plaintext. Its successful 1,000-stream result above does not establish full-file reliability.

## Implementation and compatibility

The decoder is safe, portable Rust with a tuned 4096-entry Huffman lookup table for codes up to 12 bits and canonical fallback for longer codes. A structural scan finds possible non-byte-aligned block markers; these remain speculative until ordered decoding establishes the exact stream chain and validates all block and combined-stream CRCs. A rolling scheduler keeps workers busy across concatenated streams while bounding decoded results awaiting validation.

Legacy randomized blocks generated by bzip2 releases before 0.9.5 are intentionally unsupported. Normal `BZh1` through `BZh9` streams and concatenated streams are supported.

The architecture was inspired by Maximilian Knespel's [`librapidarchive`](https://github.com/mxmlnkn/librapidarchive) and [`indexed_bzip2`](https://github.com/mxmlnkn/indexed_bzip2): in particular, scanning for non-byte-aligned bzip2 block markers, independently decoding blocks, ordered prefetch, and indexed seeking. That project's specialised decoder is itself derived from Rob Landley's 0BSD [`bzcat` implementation in Toybox](https://github.com/landley/toybox).

## Development

[DEV.md](DEV.md) documents the architecture, test strategy, benchmark fixture generation, build commands, and release process.

`fastbz2` is licensed under the [Apache License 2.0](LICENSE).
