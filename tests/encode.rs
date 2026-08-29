use std::io::Write;

use fbz::{DecodeOptions, EncodeFormat, EncodeOptions, Encoder, Format, compress, decompress};

fn decode(encoded: &[u8]) -> Vec<u8> { decompress(encoded, DecodeOptions::default()).unwrap() }

#[test]
fn unified_encoder_covers_every_stream_format() {
    let input = b"one streaming compression API".repeat(10_000);
    for (format, magic) in [(EncodeFormat::Bzip2, Format::Bzip2), (EncodeFormat::Gzip, Format::Gzip), (EncodeFormat::Lz4, Format::Lz4)] {
        let encoded = compress(&input, format, EncodeOptions { threads: 3, memory_limit: 128 * 1024 * 1024, level: None }).unwrap();
        assert_eq!(Format::from_magic(&encoded), Some(magic));
        assert_eq!(decode(&encoded), input);

        let mut incremental = Vec::new();
        let mut encoder = Encoder::new(&mut incremental, format, EncodeOptions { threads: 2, memory_limit: 128 * 1024 * 1024, level: None }).unwrap();
        for chunk in input.chunks(7777) { encoder.write_all(chunk).unwrap(); }
        let (_, report) = encoder.finish().unwrap();
        assert_eq!(report.format, format);
        assert_eq!(report.input_len, input.len() as u64);
        assert_eq!(decode(&incremental), input);
    }
}
