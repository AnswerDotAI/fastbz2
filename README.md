# fastbz2

An active compression-format research workbench with fast bzip2 and gzip decompression.

`fastbz2` provides a native CLI, a Rust library, and a Python module. The CLI auto-selects an in-repo bzip2 or gzip decoder from the filename extension, falling back to stream magic when needed. Both decoders handle concatenated streams and fully validate their checksums. The bzip2 implementation also provides parallel decoding and persistent random-access indexes.

Compression and additional formats are planned, but the current implementation decompresses bzip2 and gzip.

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

Decoding is the default operation. `.bz2`, `.bzip2`, `.gz`, and `.gzip` select their corresponding decoder and are removed from the output name. `.tbz`, `.tbz2`, and `.tgz` produce a `.tar` filename; this currently decompresses the tar stream rather than extracting its entries. For stdin and unrecognised extensions, bzip2 or gzip magic selects the decoder. Other input names gain `.out`.

```bash
fastbz2 dump.xml.bz2                 # write dump.xml
fastbz2 events.json.gz               # write events.json
fastbz2 dump.xml.bz2 -o result.xml   # choose the output path
fastbz2 dump.xml.bz2 -o -            # write plaintext to stdout
fastbz2 -                             # read compressed data from stdin
```

Multiple inputs are decoded in order, with parallelism applied inside each file. `-C/--output-dir` collects their outputs in one directory:

```bash
fastbz2 data/*.bz2 logs/*.gz -C decoded
fastbz2 data/*.bz2 logs/*.gz -C decoded --skip-existing
```

The alternative modes are flags rather than subcommands:

```bash
fastbz2 --test dump.xml.bz2          # fully decode and validate, writing nothing
fastbz2 --index dump.xml.bz2         # write dump.xml.bz2.fbz2i (bzip2 only)
fastbz2 --list events.json.gz        # print the validated member/block layout
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

`-P/--threads 0`, the default, uses the machine's available parallelism; an explicit positive value is honoured by either codec. `--memory-limit` bounds speculative output in the shared scheduler and defaults to `1G`. Gzip uses parallel dynamic-block discovery only when the input and memory budget can amortize it, otherwise selecting its serial path automatically.

## Python

The Python API currently exposes the bzip2 backend. Unified Python dispatch will follow the CLI workbench rather than being designed ahead of it.

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

The in-repo gzip decoder is available separately so callers can choose explicitly:

```rust
let plain = fastbz2::gzip::decompress(&compressed_gzip)?;
```

`gzip::decompress_to_writer` and `gzip::decompress_to_writer_with_options` return a validated report containing gzip member metadata, each DEFLATE block's kind and ranges, and counts of accepted speculative and serial-fallback chunks. They support stored, fixed-Huffman, and dynamic-Huffman blocks, optional gzip headers, and concatenated members.

## Performance

These are single local release-mode runs on the primary 18-core Apple Silicon development machine. Bzip2 parallel rows use 18 workers. The gzip comparison warms each executable with the 5% fixture, then measures exactly one full validation run; peak physical footprint comes from a separate sampled run because process inspection can perturb such a short workload. These are observations rather than statistical aggregates. Modes are stated per row because a streaming sink, an in-memory `Vec<u8>`, and a CLI pipeline have different allocation and I/O costs; compare rows using the same method most directly.

Full Simple English Wikipedia (`338 MB` compressed, `1,688,460,257` bytes decoded):

| Decoder | Mode | Seconds |
|---|---|---:|
| fastbz2 | parallel, 18 threads, streaming sink | 2.515 |
| crabz2 0.4.0 | parallel | 4.460 |
| bzip2 | serial CLI | 20.310 |
| pbzip2 1.1.13 | CLI | 20.240 |
| libbz2-rs 0.2.5 | serial, in process | 20.700 |
| fastbz2 | serial, in process | 21.279 |


Full SimpleWiki recompressed with system `gzip -6` (`438,904,466` bytes compressed, `1,688,460,257` bytes decoded):

| Decoder | Mode | Seconds | Peak physical footprint |
|---|---|---:|---:|
| rapidgzip-rust, local checkout | auto parallel, validation sink | 0.363 | 585 MiB |
| fastbz2 | auto parallel, validation sink | 0.357 | 552 MiB |
| Apple gzip | serial, stdout discarded | 1.371 | 1.2 MiB |

The memory values use macOS physical footprint rather than `ru_maxrss`. The fastbz2 CLI memory-maps its 419 MiB input, so clean reclaimable file pages make RSS look roughly 419 MiB larger; `pread`-based tools leave the same cached pages outside process RSS. Physical footprint makes the comparison meaningful.

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

The production codec logic is portable Rust. The bzip2 decoder uses a tuned 4096-entry Huffman lookup table for codes up to 12 bits and canonical fallback for longer codes. A structural scan finds possible non-byte-aligned block markers; these remain speculative until ordered decoding establishes the exact stream chain and validates all block and combined-stream CRCs. A rolling scheduler keeps workers busy across concatenated streams while bounding decoded results awaiting validation.

The gzip backend implements RFC 1952 framing and DEFLATE directly in this repository. For sufficiently large dynamic-Huffman inputs it discovers independently decodable boundaries, decodes speculative chunks through the shared byte-budgeted scheduler, and represents unknown predecessor bytes as compact markers. The ordered coordinator resolves only the suffix needed to derive the next 32 KiB history window; full marker resolution and per-chunk CRC run as priority work on the same staged worker queue, and CRCs are combined in order. Once a chunk has a marker-free window, the same decoder switches its remaining output from `u16` markers to ordinary bytes. Small, stored-heavy, fixed-heavy, one-thread, and low-memory inputs use the serial path; concatenated members may independently choose either path. FHCRC, CRC32, and ISIZE are always validated. `crc32fast` is the only production codec helper; `flate2` is dev-only.

Legacy randomized blocks generated by bzip2 releases before 0.9.5 are intentionally unsupported. Normal `BZh1` through `BZh9` streams and concatenated streams are supported.

## Research lineage and credits

The gzip work builds on Maximilian Knespel and Holger Brunst's HPDC '23 paper, [*Rapidgzip: Parallel Decompression and Seeking in Gzip Files Using Cache Prefetching*](https://doi.org/10.1145/3588195.3592992). In particular, fastbz2 adapts its central idea of starting DEFLATE decoding without the preceding 32 KiB window, representing uncertain output until the true history becomes available, and committing independently decoded chunks in order.

The open-source implementations and codebases consulted were:

- [`rapidgzip`](https://github.com/mxmlnkn/rapidgzip), the C++ implementation described by the paper.
- [`rapidgzip-rust`](https://github.com/COMBINE-lab/rapidgzip-rust), a pure-Rust reimplementation and fastbz2's local gzip performance and memory reference.
- [`librapidarchive`](https://github.com/mxmlnkn/librapidarchive), an experimental shared architecture for parallel bzip2 and gzip access.
- [`indexed_bzip2`](https://github.com/mxmlnkn/indexed_bzip2), for non-byte-aligned marker scanning, independent bzip2 block decoding, ordered prefetch, and indexed seeking.
- Rob Landley's 0BSD [`bzcat` implementation in Toybox](https://github.com/landley/toybox), from which fastbz2's specialised bzip2 decoder is derived.

## Development

[DEV.md](DEV.md) documents the architecture, test strategy, benchmark fixture generation, build commands, and release process.

`fastbz2` is licensed under the [Apache License 2.0](LICENSE).
