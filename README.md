# fbz

**Fast, reliable parallel decompression.**

`fbz` is one decompression CLI for bzip2, gzip, LZ4, ZIP, and compressed tar archives. It selects the format automatically, uses the available CPU cores, validates every stream, and safely extracts archives. The same engine is available as a Rust crate and Python module.

`fbz` was created because we found existing tools tended to be too slow (as the benchmarks below show) or too unreliable (e.g `pbzip2` 1.1.13 fails to decompress the full English Wikipedia archive). And we wanted a single tool we could use for all common formats with a single CLI interface.

## Performance

The same 80.5 MiB SimpleWiki XML payload is used in every row with automatic thread selection. Stream formats are fully decoded and validated without writing output; archives are extracted. Each CLI is warmed once, then measured once on the primary Apple Silicon development machine.

| Format | `fbz` | Familiar tool | Speedup |
|---|---:|---:|---:|
| `.bz2` | 163 ms | `bzip2`: 1.17 s | 7.2x |
| `.tar.bz2` | 152 ms | `tar`: 1.17 s | 7.7x |
| `.zip` (18 files) | 30 ms | `unzip`: 360 ms | 12x |
| `.gz` | 31 ms | `gzip`: 76 ms | 2.5x |
| `.tar.gz` | 57 ms | `tar`: 118 ms | 2.1x |
| `.lz4` | 55 ms | `lz4`: 56 ms | 1.0x |

See [Benchmarking details](#benchmarking-details) for the details.

## Install

PyPI wheels contain both the Python module and the native `fbz` executable—there is no Python CLI wrapper:

```bash
pip install fbz
```

Python 3.10 and later are supported. Prebuilt wheels target Linux on x86-64 and ARM64, and macOS on ARM64. macOS Intel is best-effort and can build from source.

Install the native CLI or add the Rust library from crates.io:

```bash
cargo install fbz
cargo add fbz
```

## CLI

Decoding is the default operation. `.bz2`, `.bzip2`, `.gz`, `.gzip`, and `.lz4` select their corresponding decoder and are removed from the output name. Compressed tar names—`.tar.bz2`, `.tar.bzip2`, `.tbz`, `.tbz2`, `.tar.gz`, `.tar.gzip`, `.tgz`, and `.tar.lz4`—and `.zip` automatically extract into the current directory or `-C/--output-dir`. `-x/--extract` forces archive extraction for stdin or an unusual filename; an explicit `-o/--output` instead writes a decoded tar stream, but is invalid for ZIP because ZIP has no single decoded byte stream. For stdin and unrecognised extensions, bzip2, gzip, LZ4, or ZIP magic selects the format. Other non-archive input names gain `.out`.

```bash
fbz dump.xml.bz2                   # write dump.xml
fbz events.json.gz                 # write events.json
fbz events.json.lz4                # write events.json
fbz source.tar.gz                  # extract into the current directory
fbz source.tar.lz4 -C unpacked     # stream-decode and extract
fbz source.tbz2 -C unpacked        # extract into unpacked/
fbz dataset.zip -C unpacked         # extract ZIP entries adaptively in parallel
fbz --extract -C unpacked -        # extract tar or ZIP data from stdin
fbz source.tgz -o source.tar       # decode without extracting
fbz dump.xml.bz2 -o result.xml     # choose the decoded output path
fbz dump.xml.bz2 -o -              # write decoded bytes to stdout
```

Multiple inputs are processed in order, with parallelism applied inside each compressed stream. `-C/--output-dir` collects decoded files and is the extraction root for archives:

```bash
fbz data/*.bz2 logs/*.gz -C decoded
fbz data/*.bz2 logs/*.gz -C decoded --skip-existing
fbz backups/*.tgz -C restored
fbz datasets/*.zip -C restored
```

Validation and inspection remain flags rather than subcommands:

```bash
fbz --test dump.xml.bz2          # fully decode and validate, writing nothing
fbz --index dump.xml.bz2         # write dump.xml.bz2.fbz2i (bzip2 only)
fbz --list events.json.gz        # print the validated member/block layout
fbz --list events.json.lz4       # print the validated frame/block layout
fbz --list dataset.zip           # print the validated entry layout
fbz --list --json dump.xml.bz2   # emit the complete layout as JSON
```

`--test`, `--index`, `--list`, and explicit `--extract` are mutually exclusive. Human-readable `--list` output labels each input when given multiple files; JSON output is one object for one input and an array for multiple inputs.

## Python

The Python API currently exposes the bzip2 backend. Unified Python dispatch will follow the CLI workbench rather than being designed ahead of it.

### One-shot decompression and validation

```python
import fbz

plain = fbz.decompress(compressed_bytes)
fbz.test("dump.xml.bz2")  # returns None after successful validation
```

`decompress` accepts a bytes-like object and returns `bytes`. `test` accepts either compressed bytes or a path and avoids retaining the decoded result.

### Seekable reads and persistent indexes

`fbz.open` returns a seekable binary `io.RawIOBase`. Opening without an index performs a complete validation pass and builds an in-memory block index; `build_index` can persist that work for later processes:

```python
import fbz

fbz.build_index("dump.xml.bz2", "dump.xml.bz2.fbz2i")

with fbz.open("dump.xml.bz2", index="dump.xml.bz2.fbz2i") as f:
    f.seek(1_000_000_000)
    chunk = f.read(64 * 1024)
    print(f.tell(), f.size)
```

Building an index fully decodes into a sink but does not write or retain the plaintext. Indexes contain compressed and decoded block offsets and are bound to the exact compressed source by its length and BLAKE3 hash. Loading one verifies that identity without decoding the whole payload; subsequent reads decode only the blocks needed for the requested range and cache recent blocks. `cache_limit` controls that cache. Path sources are memory-mapped, while bytes-like sources stay in memory.

### Structural scanning

`scan` cheaply finds candidate stream headers and bit-level block markers without decoding:

```python
import bz2
from fbz import scan

result = scan(bz2.compress(b"hello"))
assert result.blocks[0].bit_offset == 32
```

Scan results are deliberately untrusted candidates. Use `test`, `decompress`, `build_index`, or `open` when validation is required.

## Rust

The streaming API accepts any `Write` destination and uses the serial fast path when `threads` is one:

```rust
use fbz::{DecodeOptions, Source, decompress_to_writer};

fn main() -> fbz::Result<()> {
    let source = Source::open("dump.xml.bz2")?;
    let mut output = std::io::stdout().lock();
    decompress_to_writer(source.as_slice(), &mut output, DecodeOptions::default())?;
    Ok(())
}
```

`decompress` returns a `Vec<u8>`. `decode_to_writer` returns a validated `Index` while streaming output, `build_index` validates into a sink, and their `*_with_progress` variants report completed compressed and decoded byte counts. `IndexedReader` implements `Read` and `Seek`; it can build an index itself or load a persisted one with `open_with_index`.

The in-repo gzip decoder is available separately so callers can choose explicitly:

```rust
let plain = fbz::gzip::decompress(&compressed_gzip)?;
```

`gzip::decompress_to_writer` and `gzip::decompress_to_writer_with_options` return a validated report containing gzip member metadata, each DEFLATE block's kind and ranges, and counts of accepted speculative and serial-fallback chunks. They support stored, fixed-Huffman, and dynamic-Huffman blocks, optional gzip headers, and concatenated members.

The raw shared codec is available as `fbz::deflate::decompress_to_sink_with_options_and_progress`; gzip framing and ZIP extraction both use this exact decoder.

The in-repo LZ4 frame decoder has the same one-shot, writer, options, progress, and report shapes as gzip:

```rust
let plain = fbz::lz4::decompress(&compressed_lz4)?;
```

It accepts standard independent or linked blocks, stored blocks, all four standard block maxima, optional block/content checksums and sizes, concatenated frames, and skippable frames. External dictionaries and the obsolete legacy frame format are intentionally unsupported.

### Streaming reads

`fbz::Reader` provides a normal `std::io::Read` over bzip2, gzip, or LZ4 files without a preliminary indexing or validation pass:

```rust
use std::io::{BufReader, Read};
use fbz::{DecodeOptions, Reader};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let reader = Reader::open("dump.xml.bz2", DecodeOptions::default())?;
    let mut reader = BufReader::new(reader);
    let mut header = [0; 4096];
    reader.read_exact(&mut header)?;
    Ok(())
}
```

Magic takes priority over the filename extension, with the extension used as a fallback for damaged headers. The decoder runs on an owned worker thread and transfers completed decoder allocations through a zero-capacity pipe; it neither materializes the plaintext nor writes an intermediate file. `DecodeOptions` controls decoder threads and speculative memory. Dropping early disconnects the pipe, cancels outstanding work, and joins the worker.

Checksum errors discovered after output has begun are returned by a later `read()` call. Therefore only successful EOF establishes that the complete stream was valid; dropping early deliberately does not finish validation. Compressed tar inputs yield the decoded tar byte stream rather than extracting it. ZIP is not exposed through `Reader` because an archive has no single decoded byte stream.

## CLI output safety

- Existing decoded files and archive entries are rejected by default. `--force` replaces them; `--skip-existing` applies to decoded files rather than archives.
- Decoded-file outputs use a same-directory temporary file and become visible atomically only after successful checksum validation.
- Tar entries stream into a same-filesystem staging directory through a bounded pipe. ZIP entries decode directly into the same staging scheme. Entries are preflighted and moved into the destination only after every relevant compression stream and archive structure validates, so a late CRC failure leaves no extracted files.
- Tar and ZIP paths and link targets are confined to the destination. ZIP rejects unsafe or duplicate paths; tar safely skips unsafe entries. New entries use the archive's permissions and modification times where provided. Standalone decoded files inherit those values from the compressed input.
- `--rm` removes each compressed input only after its decoded file or all archive entries have been committed successfully.
- `--max-output SIZE` limits decoded bytes per input, including tar framing and padding. Sizes accept binary suffixes such as `K`, `MiB`, and `G`.

Long interactive operations report completion, decoded throughput, compression ratio, and ETA on stderr. Progress is disabled automatically when stderr is redirected; `-q/--quiet` also suppresses progress and skip notices.

`-P/--threads 0`, the default, uses the machine's available parallelism; an explicit positive value is honoured by every decoder. `--memory-limit` bounds speculative output in the shared scheduler and defaults to `1G`. Gzip uses parallel dynamic-block discovery only when the input and memory budget can amortize it. LZ4 frames with independent blocks decode those blocks concurrently and commit them in order; automatic LZ4 decoding caps this memory-bandwidth-bound work at four workers, while explicit `-P` values remain unchanged. Linked-block frames decode serially because each block depends on the preceding 64 KiB history. ZIP uses one level of parallelism at a time: large entries use the parallel DEFLATE engine, while archives of ordinary entries decode files concurrently without nested worker pools.

## Benchmarking details

The headline benchmark uses the first 84,423,012 decoded bytes of SimpleWiki for every format. The standalone bzip2 and gzip rows run each tool's validation mode. The ZIP row extracts 18 equal-sized files, while the compressed-tar rows extract one file; each archive comparison performs the same work on both sides. LZ4 uses `fbz`'s automatic four-worker limit. All results are single local release-mode observations after one untimed warm-up, not statistical aggregates.

The familiar reference CLIs are the system `bzip2`, `gzip`, and `tar`; Apple Info-ZIP `unzip` 6.00; and Homebrew `lz4` 1.10.0. The detailed fixture-generation and single-run commands live in [DEV.md](DEV.md), alongside in-process codec comparisons and separate memory diagnostics.

On the complete 1.57 GiB SimpleWiki XML recompressed with system `gzip -6`, `fbz` validated the stream in 0.33 seconds, compared with 0.36 seconds for a local `rapidgzip-rust` checkout and 1.37 seconds for Apple `gzip`. This larger result is kept here because it exercises sustained parallel gzip decoding; it is not mixed into the common-payload headline table.

### Reliability on large inputs

Homebrew `pbzip2` 1.1.13 could not safely decompress the complete 26,668,484,995-byte English Wikipedia multistream dump on this machine. It segfaulted, and repeated attempts produced divergent and truncated plaintext. Successful smaller-file benchmark results therefore do not establish full-file reliability, which is why `fbz` treats structural and checksum validation as part of decompression.

## Implementation and compatibility

The production codec logic is portable Rust. The bzip2 decoder uses a tuned 4096-entry Huffman lookup table for codes up to 12 bits and canonical fallback for longer codes. A structural scan finds possible non-byte-aligned block markers; these remain speculative until ordered decoding establishes the exact stream chain and validates all block and combined-stream CRCs. A rolling scheduler keeps workers busy across concatenated streams while bounding decoded results awaiting validation.

The gzip backend implements RFC 1952 framing and DEFLATE directly in this repository. For sufficiently large dynamic-Huffman inputs it discovers independently decodable boundaries, decodes speculative chunks through the shared byte-budgeted scheduler, and represents unknown predecessor bytes as compact markers. The ordered coordinator resolves only the suffix needed to derive the next 32 KiB history window; full marker resolution and per-chunk CRC run as priority work on the same staged worker queue, and CRCs are combined in order. Once a chunk has a marker-free window, the same decoder switches its remaining output from `u16` markers to ordinary bytes. Small, stored-heavy, fixed-heavy, one-thread, and low-memory inputs use the serial path; concatenated members may independently choose either path. FHCRC, CRC32, and ISIZE are always validated. LZ4 framing and block decoding are likewise implemented in safe Rust. It parses one frame header at a time and schedules independent blocks in bounded batches, so output can begin without a whole-frame layout pass. Independent blocks use the same ordered, byte-budgeted scheduler as bzip2; linked blocks retain only the preceding 64 KiB window. LZ4 and DEFLATE share one optimized overlapping back-reference expansion primitive. Header, block, and content XXH32 checksums are validated where present. ZIP reuses the raw DEFLATE core and uses the mature `zip` crate only for container structure and metadata. It supports stored and DEFLATE entries, Zip64, streaming data descriptors, Unix symlinks/modes, and Unix/NTFS modification-time fields; encryption and uncommon legacy compression methods are intentionally unsupported. `crc32fast` and `twox-hash` are the production checksum helpers; `flate2` and `lz4_flex` are dev-only differential oracles.

Legacy randomized blocks generated by bzip2 releases before 0.9.5 are intentionally unsupported. Normal `BZh1` through `BZh9` streams and concatenated streams are supported.

## Research lineage and credits

The gzip work builds on Maximilian Knespel and Holger Brunst's HPDC '23 paper, [*Rapidgzip: Parallel Decompression and Seeking in Gzip Files Using Cache Prefetching*](https://doi.org/10.1145/3588195.3592992). In particular, fbz adapts its central idea of starting DEFLATE decoding without the preceding 32 KiB window, representing uncertain output until the true history becomes available, and committing independently decoded chunks in order.

The open-source implementations and codebases consulted were:

- [`rapidgzip`](https://github.com/mxmlnkn/rapidgzip), the C++ implementation described by the paper.
- [`rapidgzip-rust`](https://github.com/COMBINE-lab/rapidgzip-rust), a pure-Rust reimplementation and fbz's local gzip performance and memory reference.
- [`librapidarchive`](https://github.com/mxmlnkn/librapidarchive), an experimental shared architecture for parallel bzip2 and gzip access.
- [`indexed_bzip2`](https://github.com/mxmlnkn/indexed_bzip2), for non-byte-aligned marker scanning, independent bzip2 block decoding, ordered prefetch, and indexed seeking.
- [`zip`](https://github.com/zip-rs/zip2), used without codec features for maintained ZIP structure and metadata handling.
- [`LZ4`](https://github.com/lz4/lz4), the reference format and Homebrew CLI performance baseline.
- [`lz4_flex`](https://github.com/pseitz/lz4_flex), used dev-only to generate a broad interoperability matrix and benchmark frames.
- [`lz4-rs`](https://github.com/bozaro/lz4-rs), consulted as a second local implementation reference.
- Rob Landley's 0BSD [`bzcat` implementation in Toybox](https://github.com/landley/toybox), from which fbz's specialised bzip2 decoder is derived.

## Development

[DEV.md](DEV.md) documents the architecture, test strategy, benchmark fixture generation, build commands, and release process.

`fbz` is licensed under the [Apache License 2.0](LICENSE).
