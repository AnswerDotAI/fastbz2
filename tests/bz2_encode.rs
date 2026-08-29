mod support;

use std::{fs, io::Write, process::Command};

use fbz::{Bzip2Encoder, DecodeOptions, EncodeFormat, EncodeOptions, compress, decompress};

#[test]
fn bzip2_encoder_roundtrips_levels_and_system_decoder() {
    let inputs = support::compression_inputs(b"hello bzip2", 30_000);
    for level in [1, 6, 9] {
        for input in &inputs {
            let options = EncodeOptions { threads: 3, memory_limit: 256 * 1024 * 1024, level: Some(level) };
            let encoded = compress(input, EncodeFormat::Bzip2, options).unwrap();
            assert_eq!(decompress(&encoded, DecodeOptions::default()).unwrap(), *input);
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("data.bz2");
            fs::write(&path, encoded).unwrap();
            assert!(Command::new("bzip2").args(["-t", path.to_str().unwrap()]).status().unwrap().success());
        }
    }
}

#[test]
fn bzip2_encoder_parallel_blocks_and_incremental_writes() {
    let input = support::patterned_bytes_with(220_000, 17, 101);
    let mut encoded = Vec::new();
    let mut encoder = Bzip2Encoder::new(&mut encoded, EncodeOptions { threads: 4, memory_limit: 64 * 1024 * 1024, level: Some(1) }).unwrap();
    for chunk in input.chunks(7777) { encoder.write_all(chunk).unwrap(); }
    encoder.flush().unwrap();
    let (_, report) = encoder.finish().unwrap();
    assert_eq!(report.input_len, input.len() as u64);
    assert!(report.blocks >= 2);
    assert_eq!(decompress(&encoded, DecodeOptions::default()).unwrap(), input);
}

#[test]
fn bzip2_encoder_default_is_level_nine_and_memory_is_bounded() {
    let encoded = compress(b"default level", EncodeFormat::Bzip2, EncodeOptions::default()).unwrap();
    assert_eq!(&encoded[..4], b"BZh9");
    let error = compress(b"too little memory", EncodeFormat::Bzip2, EncodeOptions { memory_limit: 1024 * 1024, level: Some(9), ..EncodeOptions::default() })
        .unwrap_err();
    assert!(error.to_string().contains("memory limit"));
}
