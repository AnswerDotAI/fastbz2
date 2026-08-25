use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

use crabz2::{Level, compress};
use fbz::{DecodeOptions, Error, Reader};
use flate2::{Compression, write::GzEncoder};

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::new(6));
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

fn read(path: &Path, options: DecodeOptions) -> std::io::Result<Vec<u8>> {
    let mut reader = Reader::open(path, options).unwrap();
    let mut output = Vec::new();
    reader.read_to_end(&mut output)?;
    Ok(output)
}

#[test]
fn reader_detects_stream_formats_without_prevalidation() {
    let directory = tempfile::tempdir().unwrap();
    let plain = b"streaming reader format detection ".repeat(20_000);
    let cases = [("bzip-magic.data", compress(&plain, Level::FASTEST)), ("gzip-magic.bz2", gzip(&plain))];
    for (name, encoded) in cases {
        let path = directory.path().join(name);
        fs::write(&path, encoded).unwrap();
        assert_eq!(read(&path, DecodeOptions { threads: 2, ..DecodeOptions::default() }).unwrap(), plain);
    }

    let corrupt = directory.path().join("corrupt.gz");
    fs::write(&corrupt, b"not a gzip stream").unwrap();
    let mut reader = Reader::open(corrupt, DecodeOptions::default()).unwrap();
    assert_eq!(reader.read(&mut [0; 1]).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn late_checksum_failure_is_an_error_after_the_decoded_prefix() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("late-error.gz");
    let plain = b"validated only at the trailer ".repeat(40_000);
    let mut encoded = gzip(&plain);
    let trailer_crc = encoded.len() - 8;
    encoded[trailer_crc] ^= 1;
    fs::write(&path, encoded).unwrap();

    let mut reader = Reader::open(path, DecodeOptions { threads: 2, ..DecodeOptions::default() }).unwrap();
    let mut output = Vec::new();
    let error = reader.read_to_end(&mut output).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(output, plain);
    assert_eq!(reader.read(&mut [0; 1]).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn bzip2_failure_does_not_turn_a_valid_prefix_into_eof() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("concatenated.bz2");
    let first = b"complete first bzip2 stream ".repeat(10_000);
    let mut encoded = compress(&first, Level::FASTEST);
    let second_start = encoded.len();
    encoded.extend(compress(b"corrupt second stream", Level::FASTEST));
    encoded[second_start + 10] ^= 1;
    fs::write(&path, encoded).unwrap();

    let mut reader = Reader::open(path, DecodeOptions { threads: 2, ..DecodeOptions::default() }).unwrap();
    let mut output = Vec::new();
    assert_eq!(reader.read_to_end(&mut output).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
    assert!(output.starts_with(&first));
}

#[test]
fn dropping_under_backpressure_joins_the_decoder() {
    let directory = tempfile::tempdir().unwrap();
    let inputs = [("early-drop.gz", gzip(&vec![7; 4 * 1024 * 1024]), 1), ("early-drop.bz2", compress(&vec![9; 2 * 1024 * 1024], Level::FASTEST), 4)];
    for (name, encoded, threads) in inputs {
        let path = directory.path().join(name);
        fs::write(&path, encoded).unwrap();
        let mut reader = Reader::open(path, DecodeOptions { threads, ..DecodeOptions::default() }).unwrap();
        assert_eq!(reader.read(&mut [0; 1]).unwrap(), 1);
        drop(reader);
    }
}

#[test]
fn reader_rejects_non_stream_archives_and_bad_options_at_open() {
    fn assert_send<T: Send>() {}
    assert_send::<Reader>();

    let directory = tempfile::tempdir().unwrap();
    let zip = directory.path().join("archive.zip");
    fs::write(&zip, b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0").unwrap();
    assert!(matches!(Reader::open(&zip, DecodeOptions::default()), Err(Error::UnsupportedFormat(_))));

    let lz4 = directory.path().join("data.lz4");
    fs::write(&lz4, [0x04, 0x22, 0x4d, 0x18]).unwrap();
    assert!(matches!(Reader::open(&lz4, DecodeOptions::default()), Err(Error::UnsupportedFormat(_))));

    let gzip_path = directory.path().join("data.gz");
    fs::write(&gzip_path, gzip(b"data")).unwrap();
    assert!(matches!(Reader::open(gzip_path, DecodeOptions { memory_limit: 1, ..DecodeOptions::default() }), Err(Error::InvalidConfiguration(_))));
}
