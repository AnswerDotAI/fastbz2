const POLYNOMIAL: u32 = 0x04c1_1db7;

const fn make_table() -> [u32; 256] {
    let mut table = [0; 256];
    let mut byte = 0;
    while byte < 256 {
        let mut crc = (byte as u32) << 24;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ POLYNOMIAL } else { crc << 1 };
            bit += 1;
        }
        table[byte] = crc;
        byte += 1;
    }
    table
}

const TABLE: [u32; 256] = make_table();

#[derive(Clone, Copy)]
pub(crate) struct Bz2Crc {
    state: u32,
}

impl Bz2Crc {
    pub fn new() -> Self {
        Self { state: u32::MAX }
    }

    #[inline]
    pub fn push_repeat(&mut self, byte: u8, count: usize) {
        for _ in 0..count {
            let index = ((self.state >> 24) as u8 ^ byte) as usize;
            self.state = (self.state << 8) ^ TABLE[index];
        }
    }

    #[inline]
    pub fn update(&mut self, data: &[u8]) {
        for &byte in data {
            let index = ((self.state >> 24) as u8 ^ byte) as usize;
            self.state = (self.state << 8) ^ TABLE[index];
        }
    }

    pub fn finish(self) -> u32 {
        !self.state
    }
}

/// Compute the CRC used for an uncompressed bzip2 block.
pub fn bz2_crc32(data: &[u8]) -> u32 {
    let mut crc = Bz2Crc::new();
    crc.update(data);
    crc.finish()
}

/// Add a block CRC to bzip2's combined stream CRC.
#[inline]
pub fn combine_stream_crc(stream_crc: u32, block_crc: u32) -> u32 {
    stream_crc.rotate_left(1) ^ block_crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        assert_eq!(bz2_crc32(b""), 0);
        assert_eq!(bz2_crc32(b"123456789"), 0xfc89_1918);
    }

    #[test]
    fn combines_by_rotating_then_xoring() {
        assert_eq!(combine_stream_crc(0x8000_0001, 0x1234_5678), 0x1234_567b);
    }
}
