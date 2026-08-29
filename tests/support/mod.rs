#![allow(dead_code)]

use std::{fs, io::Write, path::Path};

use fbz::{DecodeOptions, decompress};
use flate2::{Compression, write::DeflateEncoder};

#[derive(Clone, Copy)]
pub(crate) enum ZipMethod { #[allow(dead_code)] Stored, Deflate }

pub(crate) fn simplewiki_prefix() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("meta/simplewiki-first-5pct.xml.bz2");
    let encoded = fs::read(&path).unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
    let contents = decompress(&encoded, DecodeOptions::default()).unwrap();
    assert_eq!(contents.len(), 84_423_012);
    contents
}

pub(crate) fn patterned_bytes(size: usize) -> Vec<u8> { patterned_bytes_with(size, 31, 97) }

pub(crate) fn patterned_bytes_with(size: usize, stride: usize, period: usize) -> Vec<u8> {
    (0..size).map(|index| ((index * stride + index / period) & 255) as u8).collect()
}

pub(crate) fn compression_inputs(greeting: &[u8], size: usize) -> Vec<Vec<u8>> { vec![Vec::new(), greeting.to_vec(), vec![b'a'; size], patterned_bytes(size)] }

fn push_u16(output: &mut Vec<u8>, value: u16) { output.extend_from_slice(&value.to_le_bytes()); }

fn push_u32(output: &mut Vec<u8>, value: u32) { output.extend_from_slice(&value.to_le_bytes()); }

pub(crate) fn zip_with_modes(entries: &[(&str, &[u8], ZipMethod, u32)]) -> Vec<u8> {
    let mut archive = Vec::new();
    let mut central = Vec::new();
    for (path, contents, method, mode) in entries {
        let method_id = match method { ZipMethod::Stored => 0, ZipMethod::Deflate => 8 };
        let compressed = match method {
            ZipMethod::Stored => contents.to_vec(),
            ZipMethod::Deflate => {
                let mut encoder = DeflateEncoder::new(Vec::new(), Compression::new(6));
                encoder.write_all(contents).unwrap();
                encoder.finish().unwrap()
            }
        };
        let offset = archive.len() as u32;
        let crc = crc32fast::hash(contents);
        push_u32(&mut archive, 0x0403_4b50);
        for value in [20, 0, method_id, 0, 0] { push_u16(&mut archive, value); }
        for value in [crc, compressed.len() as u32, contents.len() as u32] { push_u32(&mut archive, value); }
        push_u16(&mut archive, path.len() as u16);
        push_u16(&mut archive, 0);
        archive.extend_from_slice(path.as_bytes());
        archive.extend_from_slice(&compressed);

        push_u32(&mut central, 0x0201_4b50);
        for value in [0x0314, 20, 0, method_id, 0, 0] { push_u16(&mut central, value); }
        for value in [crc, compressed.len() as u32, contents.len() as u32] { push_u32(&mut central, value); }
        for value in [path.len() as u16, 0, 0, 0, 0] { push_u16(&mut central, value); }
        push_u32(&mut central, mode << 16);
        push_u32(&mut central, offset);
        central.extend_from_slice(path.as_bytes());
    }
    let central_offset = archive.len() as u32;
    archive.extend_from_slice(&central);
    push_u32(&mut archive, 0x0605_4b50);
    for value in [0, 0, entries.len() as u16, entries.len() as u16] { push_u16(&mut archive, value); }
    push_u32(&mut archive, central.len() as u32);
    push_u32(&mut archive, central_offset);
    push_u16(&mut archive, 0);
    archive
}

pub(crate) fn zip_bytes(entries: &[(&str, &[u8], ZipMethod)]) -> Vec<u8> {
    let entries: Vec<_> = entries.iter().map(|(path, contents, method)| (*path, *contents, *method, 0o100640)).collect();
    zip_with_modes(&entries)
}
