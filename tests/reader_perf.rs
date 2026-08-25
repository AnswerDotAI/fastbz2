use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use fbz::{DecodeOptions, Reader, Source, decompress_to_writer, gzip};

const DECODED_LEN: u64 = 84_423_012;

fn fixture(extension: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("meta/simplewiki-first-5pct.xml.{extension}"))
}

fn reader_run(path: &Path) -> (Duration, u64) {
    let started = Instant::now();
    let mut reader = Reader::open(path, DecodeOptions::default()).unwrap();
    let decoded = io::copy(&mut reader, &mut io::sink()).unwrap();
    (started.elapsed(), decoded)
}

fn writer_run(path: &Path, gzip_input: bool) -> Duration {
    let started = Instant::now();
    let source = Source::open(path).unwrap();
    if gzip_input {
        gzip::decompress_to_writer_with_options(source.as_slice(), &mut io::sink(), DecodeOptions::default()).unwrap();
    } else {
        decompress_to_writer(source.as_slice(), &mut io::sink(), DecodeOptions::default()).unwrap();
    }
    started.elapsed()
}

#[test]
#[ignore = "local single-run Reader latency and writer-path comparison"]
fn reader_writer_comparison() {
    for (extension, gzip_input) in [("bz2", false), ("gz", true)] {
        let path = fixture(extension);
        let first_started = Instant::now();
        let mut reader = Reader::open(&path, DecodeOptions::default()).unwrap();
        let mut first = [0];
        reader.read_exact(&mut first).unwrap();
        let first_byte = first_started.elapsed();
        drop(reader);

        let (reader_time, decoded) = reader_run(&path);
        let writer_time = writer_run(&path, gzip_input);
        assert_eq!(decoded, DECODED_LEN);
        println!(
            "{extension}: first byte {first_byte:?}, Reader {reader_time:?}, writer {writer_time:?}, ratio {:.3}x",
            reader_time.as_secs_f64() / writer_time.as_secs_f64()
        );
    }
}
