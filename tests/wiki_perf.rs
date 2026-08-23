use std::{
    fs,
    io::{self, Write},
    path::Path,
    time::{Duration, Instant},
};

use fastbz2::{DecodeOptions, Source, decompress, decompress_to_writer};

const FIVE_PERCENT_LEN: usize = 84_423_012;
const FIVE_PERCENT_BLAKE3: &str = "69f41f28dc8ac74509d368c6aaec02f3cdf891c9da4ccf8caf625687dcd61908";
const FULL_LEN: u64 = 1_688_460_257;

#[derive(Default)]
struct CountingSink(u64);

impl Write for CountingSink {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0 += buffer.len() as u64;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn corpus_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("meta").join(name)
}

fn requested_threads() -> usize {
    std::env::var("FASTBZ2_THREADS").ok().map(|value| value.parse().expect("FASTBZ2_THREADS must be an integer")).unwrap_or(0)
}

fn timed_fastbz2(path: &Path, threads: usize) -> Duration {
    let start = Instant::now();
    let source = Source::open(path).unwrap();
    let mut output = CountingSink::default();
    decompress_to_writer(source.as_slice(), &mut output, DecodeOptions { threads, ..DecodeOptions::default() }).unwrap();
    let elapsed = start.elapsed();
    assert_eq!(output.0, FULL_LEN);
    elapsed
}

#[test]
#[ignore = "local SimpleWiki performance benchmark"]
fn simplewiki_first_five_percent() {
    let path = corpus_path("simplewiki-first-5pct.xml.bz2");
    let encoded = fs::read(&path).unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
    let threads = requested_threads();
    let options = DecodeOptions { threads, ..DecodeOptions::default() };
    let resolved_threads = options.resolved_threads();

    let start = Instant::now();
    let decoded = decompress(&encoded, options).unwrap();
    let elapsed = start.elapsed();
    let mib_per_second = decoded.len() as f64 / (1024.0 * 1024.0) / elapsed.as_secs_f64();
    let hash = blake3::hash(&decoded).to_hex().to_string();
    eprintln!("SimpleWiki 5%: {elapsed:.3?}, {mib_per_second:.1} MiB/s, {resolved_threads} threads, BLAKE3 {hash}");

    assert_eq!(decoded.len(), FIVE_PERCENT_LEN);
    assert_eq!(hash, FIVE_PERCENT_BLAKE3);
}

#[test]
#[ignore = "local full SimpleWiki performance benchmark"]
fn simplewiki_full() {
    let path = corpus_path("simplewiki-full.xml.bz2");
    let options = DecodeOptions { threads: requested_threads(), ..DecodeOptions::default() };

    let elapsed = timed_fastbz2(&path, options.threads);
    eprintln!("fastbz2 ({} threads): {elapsed:.3?}", options.resolved_threads());
}
