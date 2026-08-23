use std::{error, fmt};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidBitCount(u8),
    InvalidBitOffset { bit_offset: u64, len_bits: u64 },
    UnexpectedEof { bit_offset: u64, requested: u64, remaining: u64 },
    InvalidStreamHeader,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBitCount(count) => write!(f, "cannot read {count} bits at once; maximum is 64"),
            Self::InvalidBitOffset { bit_offset, len_bits } => {
                write!(f, "bit offset {bit_offset} is beyond the {len_bits}-bit input")
            }
            Self::UnexpectedEof { bit_offset, requested, remaining } => {
                write!(f, "unexpected end of input at bit {bit_offset}: requested {requested} bits, {remaining} remain")
            }
            Self::InvalidStreamHeader => write!(f, "input does not start with a bzip2 BZh1-BZh9 header"),
        }
    }
}

impl error::Error for Error {}
