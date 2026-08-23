//! Portable bzip2 primitives and format inspection.

mod bitreader;
mod crc;
mod error;
mod format;

pub use bitreader::BitReader;
pub use crc::{bz2_crc32, combine_stream_crc};
pub use error::{Error, Result};
pub use format::{BLOCK_MAGIC, BlockCandidate, END_MAGIC, EndCandidate, ScanResult, StreamHeaderCandidate, scan};

#[cfg(feature = "python")]
mod python {
    use pyo3::{exceptions::PyValueError, prelude::*};

    #[pyfunction(name = "_scan")]
    #[allow(clippy::type_complexity)]
    fn py_scan(data: &[u8]) -> PyResult<(Vec<(u64, u8)>, Vec<(u64, u32, bool, u32)>, Vec<(u64, u32)>)> {
        let result = crate::scan(data).map_err(|err| PyValueError::new_err(err.to_string()))?;
        let streams = result.streams.into_iter().map(|stream| (stream.byte_offset, stream.block_size_100k)).collect();
        let blocks = result.blocks.into_iter().map(|block| (block.bit_offset, block.expected_crc, block.randomized, block.orig_ptr)).collect();
        let stream_ends = result.stream_ends.into_iter().map(|end| (end.bit_offset, end.expected_stream_crc)).collect();
        Ok((streams, blocks, stream_ends))
    }

    #[pyfunction(name = "bz2_crc32")]
    fn py_bz2_crc32(data: &[u8]) -> u32 {
        crate::bz2_crc32(data)
    }

    #[pymodule]
    fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add_function(wrap_pyfunction!(py_scan, m)?)?;
        m.add_function(wrap_pyfunction!(py_bz2_crc32, m)?)?;
        m.add("__version__", env!("CARGO_PKG_VERSION"))?;
        Ok(())
    }
}
