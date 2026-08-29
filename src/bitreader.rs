use crate::{Error, Result};

/// An MSB-first reader over an in-memory bzip2 bitstream.
#[derive(Clone, Debug)]
pub struct BitReader<'a> { data: &'a [u8], bit_offset: u64 }

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self { Self { data, bit_offset: 0 } }

    pub fn at(data: &'a [u8], bit_offset: u64) -> Result<Self> {
        let reader = Self { data, bit_offset };
        if bit_offset > reader.len_bits() { return Err(Error::InvalidBitOffset { bit_offset, len_bits: reader.len_bits() }); }
        Ok(reader)
    }

    #[inline]
    pub fn position(&self) -> u64 { self.bit_offset }

    #[inline]
    pub fn len_bits(&self) -> u64 { u64::try_from(self.data.len()).unwrap_or(u64::MAX / 8).saturating_mul(8) }

    #[inline]
    pub fn remaining(&self) -> u64 { self.len_bits().saturating_sub(self.bit_offset) }

    #[inline]
    pub fn read_bit(&mut self) -> Result<bool> {
        if self.bit_offset >= self.len_bits() { return Err(self.eof(1)); }
        let byte = self.data[(self.bit_offset / 8) as usize];
        let shift = 7 - (self.bit_offset & 7);
        self.bit_offset += 1;
        Ok((byte >> shift) & 1 != 0)
    }

    /// Read up to 64 bits, returning them in the low bits of a `u64`.
    #[inline]
    pub fn read_bits(&mut self, count: u8) -> Result<u64> {
        if count > 64 { return Err(Error::InvalidBitCount(count)); }
        if u64::from(count) > self.remaining() { return Err(self.eof(u64::from(count))); }
        if count == 0 { return Ok(0); }

        let first_byte = (self.bit_offset / 8) as usize;
        let skipped = (self.bit_offset & 7) as u32;
        let byte_count = (skipped + u32::from(count)).div_ceil(8) as usize;
        let mut word = 0_u128;
        for &byte in &self.data[first_byte..first_byte + byte_count] { word = (word << 8) | u128::from(byte); }
        let available = (byte_count * 8) as u32;
        let trailing = available - skipped - u32::from(count);
        let mask = if count == 64 { u128::from(u64::MAX) } else { (1_u128 << count) - 1 };
        self.bit_offset += u64::from(count);
        Ok(((word >> trailing) & mask) as u64)
    }

    #[inline]
    pub fn skip(&mut self, count: u64) -> Result<()> {
        if count > self.remaining() { return Err(self.eof(count)); }
        self.bit_offset += count;
        Ok(())
    }

    fn eof(&self, requested: u64) -> Error { Error::UnexpectedEof { bit_offset: self.bit_offset, requested, remaining: self.remaining() } }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(data: &[u8], offset: usize, count: usize) -> u64 {
        (offset..offset + count).fold(0, |result, bit| (result << 1) | u64::from((data[bit / 8] >> (7 - bit % 8)) & 1))
    }

    #[test]
    fn reads_every_alignment_and_width() {
        let data = [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x55];
        for offset in 0..8 {
            for count in 0..=64.min(data.len() * 8 - offset) {
                let mut reader = BitReader::at(&data, offset as u64).unwrap();
                assert_eq!(reader.read_bits(count as u8).unwrap(), reference(&data, offset, count));
                assert_eq!(reader.position(), (offset + count) as u64);
            }
        }
    }

    #[test]
    fn reads_single_bits_msb_first() {
        let mut reader = BitReader::new(&[0b1010_0001]);
        let bits: Vec<_> = (0..8).map(|_| reader.read_bit().unwrap()).collect();
        assert_eq!(bits, [true, false, true, false, false, false, false, true]);
        assert!(matches!(reader.read_bit(), Err(Error::UnexpectedEof { .. })));
    }
}
