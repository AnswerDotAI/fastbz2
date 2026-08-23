use crate::{BitReader, Error, Result};

pub const BLOCK_MAGIC: u64 = 0x3141_5926_5359;
pub const END_MAGIC: u64 = 0x1772_4538_5090;
const MAGIC_BITS: u8 = 48;
const MAGIC_MASK: u64 = (1_u64 << MAGIC_BITS) - 1;
const WINDOW_MASK: u64 = (1_u64 << 56) - 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamHeaderCandidate {
    pub byte_offset: u64,
    pub block_size_100k: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockCandidate {
    pub bit_offset: u64,
    pub expected_crc: u32,
    pub randomized: bool,
    pub orig_ptr: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndCandidate {
    pub bit_offset: u64,
    pub expected_stream_crc: u32,
}

/// Candidate stream headers and bit-level block markers found in a bzip2 input.
///
/// This is an intentionally cheap structural scan, not full validation: marker
/// bit patterns can occur in compressed payloads, so entries after the first
/// stream header are candidates until a decoder validates the chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanResult {
    pub streams: Vec<StreamHeaderCandidate>,
    pub blocks: Vec<BlockCandidate>,
    pub stream_ends: Vec<EndCandidate>,
}

pub fn scan(data: &[u8]) -> Result<ScanResult> {
    if !is_stream_header(data, 0) {
        return Err(Error::InvalidStreamHeader);
    }

    let streams = (0..=data.len().saturating_sub(4))
        .filter(|&offset| is_stream_header(data, offset))
        .map(|offset| StreamHeaderCandidate { byte_offset: offset as u64, block_size_100k: data[offset + 3] - b'0' })
        .collect();
    let mut blocks = Vec::new();
    let mut stream_ends = Vec::new();

    if data.len() < 7 {
        return Ok(ScanResult { streams, blocks, stream_ends });
    }

    // A 56-bit rolling window contains all eight 48-bit candidates beginning
    // in one byte. The simple fixed-width inner loop is intentionally friendly
    // to unrolling and auto-vectorisation on both x86-64 and ARM64.
    let mut window = data[..7].iter().fold(0_u64, |word, &byte| (word << 8) | u64::from(byte));
    for byte_offset in 0..=data.len() - 7 {
        for shift in 0..8_u32 {
            let marker = (window >> (8 - shift)) & MAGIC_MASK;
            let bit_offset = byte_offset as u64 * 8 + u64::from(shift);
            if marker == BLOCK_MAGIC {
                if let Some(block) = parse_block(data, bit_offset) {
                    blocks.push(block);
                }
            } else if marker == END_MAGIC
                && let Some(end) = parse_end(data, bit_offset)
            {
                stream_ends.push(end);
            }
        }
        if let Some(&next) = data.get(byte_offset + 7) {
            window = ((window << 8) & WINDOW_MASK) | u64::from(next);
        }
    }

    Ok(ScanResult { streams, blocks, stream_ends })
}

fn is_stream_header(data: &[u8], offset: usize) -> bool {
    data.get(offset..offset + 3) == Some(b"BZh") && matches!(data.get(offset + 3), Some(b'1'..=b'9'))
}

fn parse_block(data: &[u8], bit_offset: u64) -> Option<BlockCandidate> {
    let mut reader = BitReader::at(data, bit_offset + u64::from(MAGIC_BITS)).ok()?;
    Some(BlockCandidate {
        bit_offset,
        expected_crc: reader.read_bits(32).ok()? as u32,
        randomized: reader.read_bit().ok()?,
        orig_ptr: reader.read_bits(24).ok()? as u32,
    })
}

fn parse_end(data: &[u8], bit_offset: u64) -> Option<EndCandidate> {
    let mut reader = BitReader::at(data, bit_offset + u64::from(MAGIC_BITS)).ok()?;
    Some(EndCandidate { bit_offset, expected_stream_crc: reader.read_bits(32).ok()? as u32 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn append_bits(target: &mut Vec<bool>, value: u64, count: usize) {
        target.extend((0..count).rev().map(|shift| value >> shift & 1 != 0));
    }

    fn pack(bits: &[bool]) -> Vec<u8> {
        bits.chunks(8).map(|chunk| chunk.iter().fold(0, |byte, &bit| (byte << 1) | u8::from(bit)) << (8 - chunk.len())).collect()
    }

    #[test]
    fn finds_markers_at_every_bit_alignment() {
        for prefix in 0..8 {
            let mut bits = Vec::new();
            for byte in b"BZh9" {
                append_bits(&mut bits, u64::from(*byte), 8);
            }
            bits.extend(std::iter::repeat_n(false, prefix));
            append_bits(&mut bits, BLOCK_MAGIC, 48);
            append_bits(&mut bits, 0x1234_5678, 32);
            append_bits(&mut bits, 1, 1);
            append_bits(&mut bits, 0x00ab_cdef, 24);
            let data = pack(&bits);
            let result = scan(&data).unwrap();
            let [block] = result.blocks.as_slice() else {
                panic!("expected one block, found {:?}", result.blocks);
            };
            assert_eq!(block.bit_offset, 32 + prefix as u64);
            assert_eq!(block.expected_crc, 0x1234_5678);
            assert!(block.randomized);
            assert_eq!(block.orig_ptr, 0x00ab_cdef);
        }
    }

    #[test]
    fn rejects_non_bzip_input() {
        assert_eq!(scan(b"not bzip2"), Err(Error::InvalidStreamHeader));
    }
}
