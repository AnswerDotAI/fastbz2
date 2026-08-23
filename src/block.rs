use crate::{Result, decoder};

pub const MAX_ENCODED_BLOCK: usize = 900_000;
pub const MAX_DECODED_BLOCK: usize = MAX_ENCODED_BLOCK / 5 * 259 + 4;

/// Decode and CRC-validate one bzip2 block bounded by two exact markers.
pub fn decode_block(data: &[u8], start_bit: u64, end_bit: u64, level: u8, expected_crc: u32) -> Result<Vec<u8>> {
    decoder::decode_block(data, start_bit, end_bit, level, expected_crc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{combine_stream_crc, scan};

    const HELLO: &[u8] = &[
        0x42, 0x5a, 0x68, 0x39, 0x31, 0x41, 0x59, 0x26, 0x53, 0x59, 0x71, 0x1c, 0x50, 0xc0, 0x00, 0x00, 0x03, 0xd9, 0x80, 0x00, 0x10, 0x40, 0x00, 0x10, 0x00,
        0x3a, 0x44, 0x90, 0x10, 0x20, 0x00, 0x31, 0x03, 0x40, 0xd0, 0x29, 0x80, 0x1e, 0xa2, 0xe0, 0x4c, 0xed, 0x69, 0xe0, 0xe1, 0x77, 0x24, 0x53, 0x85, 0x09,
        0x07, 0x11, 0xc5, 0x0c, 0x00,
    ];

    #[test]
    fn decodes_scanned_block() {
        let scan = scan(HELLO).unwrap();
        let block = &scan.blocks[0];
        let end = &scan.stream_ends[0];
        let out = decode_block(HELLO, block.bit_offset, end.bit_offset, 9, block.expected_crc).unwrap();
        assert_eq!(out, b"hello crabz2\n");
        assert_eq!(combine_stream_crc(0, block.expected_crc), end.expected_stream_crc);
    }
}
