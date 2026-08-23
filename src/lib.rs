//! Portable bzip2 primitives and format inspection.

mod bitreader;
mod block;
mod crc;
mod decode;
mod decoder;
mod error;
mod format;
mod index;
mod indexed;
mod source;

pub use bitreader::BitReader;
pub use block::{MAX_DECODED_BLOCK, MAX_ENCODED_BLOCK, decode_block};
pub use crc::{bz2_crc32, combine_stream_crc};
pub use decode::{
    DEFAULT_MEMORY_LIMIT, DecodeOptions, DecodeProgress, build_index, build_index_with_progress, decode_to_writer, decode_to_writer_with_progress, decompress,
    decompress_to_writer, decompress_to_writer_with_progress,
};
pub use error::{DecodeError, Error, Result};
pub use format::{BLOCK_MAGIC, BlockCandidate, END_MAGIC, EndCandidate, ScanResult, StreamHeaderCandidate, scan};
pub use index::{BlockIndex, Index, StreamIndex};
pub use indexed::{DEFAULT_CACHE_LIMIT, IndexedReader};
pub use source::Source;

#[cfg(feature = "python")]
mod python {
    use std::{
        io::{Read, Seek, SeekFrom},
        sync::Mutex,
    };

    use pyo3::{
        create_exception,
        exceptions::{PyOSError, PyValueError},
        prelude::*,
        types::PyBytes,
    };

    create_exception!(fastbz2, BadBzip2File, PyOSError);

    fn python_error(error: crate::Error) -> PyErr {
        match error {
            crate::Error::Io(source) => PyOSError::new_err(source.to_string()),
            crate::Error::InvalidConfiguration(message) | crate::Error::InvalidIndex(message) => PyValueError::new_err(message),
            error => BadBzip2File::new_err(error.to_string()),
        }
    }

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

    #[pyfunction(name = "_decompress", signature = (data, threads=0, memory_limit=crate::DEFAULT_MEMORY_LIMIT))]
    fn py_decompress(py: Python<'_>, data: &[u8], threads: usize, memory_limit: usize) -> PyResult<Py<PyBytes>> {
        let data = data.to_vec();
        let output = py.detach(move || crate::decompress(&data, crate::DecodeOptions { threads, memory_limit })).map_err(python_error)?;
        Ok(PyBytes::new(py, &output).unbind())
    }

    #[pyfunction(name = "_build_index", signature = (path, threads=0, memory_limit=crate::DEFAULT_MEMORY_LIMIT))]
    fn py_build_index(py: Python<'_>, path: String, threads: usize, memory_limit: usize) -> PyResult<Py<PyBytes>> {
        let encoded = py
            .detach(move || {
                let source = crate::Source::open(path)?;
                Ok::<_, crate::Error>(crate::build_index(source.as_slice(), crate::DecodeOptions { threads, memory_limit })?.to_bytes())
            })
            .map_err(python_error)?;
        Ok(PyBytes::new(py, &encoded).unbind())
    }

    #[pyfunction(name = "_test", signature = (path, threads=0, memory_limit=crate::DEFAULT_MEMORY_LIMIT))]
    fn py_test(py: Python<'_>, path: String, threads: usize, memory_limit: usize) -> PyResult<()> {
        py.detach(move || {
            let source = crate::Source::open(path)?;
            crate::build_index(source.as_slice(), crate::DecodeOptions { threads, memory_limit })?;
            Ok::<_, crate::Error>(())
        })
        .map_err(python_error)
    }

    #[pyclass(name = "_IndexedReader")]
    struct PyIndexedReader {
        inner: Mutex<crate::IndexedReader>,
    }

    #[pymethods]
    impl PyIndexedReader {
        #[staticmethod]
        #[pyo3(signature = (path, threads=0, memory_limit=crate::DEFAULT_MEMORY_LIMIT, index_path=None, cache_limit=crate::DEFAULT_CACHE_LIMIT))]
        fn from_path(py: Python<'_>, path: String, threads: usize, memory_limit: usize, index_path: Option<String>, cache_limit: usize) -> PyResult<Self> {
            let inner = py
                .detach(move || match index_path {
                    Some(index_path) => crate::IndexedReader::open_with_index(path, index_path, cache_limit),
                    None => crate::IndexedReader::from_source(crate::Source::open(path)?, None, crate::DecodeOptions { threads, memory_limit }, cache_limit),
                })
                .map_err(python_error)?;
            Ok(Self { inner: Mutex::new(inner) })
        }

        #[staticmethod]
        #[pyo3(signature = (data, threads=0, memory_limit=crate::DEFAULT_MEMORY_LIMIT, index=None, cache_limit=crate::DEFAULT_CACHE_LIMIT))]
        fn from_bytes(py: Python<'_>, data: &[u8], threads: usize, memory_limit: usize, index: Option<&[u8]>, cache_limit: usize) -> PyResult<Self> {
            let data = data.to_vec();
            let index = index.map(<[u8]>::to_vec);
            let inner = py
                .detach(move || match index {
                    Some(index) => crate::IndexedReader::from_bytes_with_index(data, &index, cache_limit),
                    None => {
                        crate::IndexedReader::from_source(crate::Source::from_bytes(data), None, crate::DecodeOptions { threads, memory_limit }, cache_limit)
                    }
                })
                .map_err(python_error)?;
            Ok(Self { inner: Mutex::new(inner) })
        }

        fn read(&self, py: Python<'_>, size: i64) -> PyResult<Py<PyBytes>> {
            let output = py
                .detach(|| {
                    let mut reader = self.inner.lock().map_err(|_| crate::Error::InvalidConfiguration("reader lock poisoned".into()))?;
                    let remaining = reader.size() - reader.position();
                    let count = if size < 0 { remaining } else { remaining.min(size as u64) };
                    let count = usize::try_from(count).map_err(|_| crate::Error::InvalidConfiguration("requested read does not fit this platform".into()))?;
                    let mut output = vec![0; count];
                    reader.read_exact(&mut output)?;
                    Ok::<_, crate::Error>(output)
                })
                .map_err(python_error)?;
            Ok(PyBytes::new(py, &output).unbind())
        }

        #[pyo3(signature = (offset, whence=0))]
        fn seek(&self, py: Python<'_>, offset: i64, whence: i32) -> PyResult<u64> {
            py.detach(|| {
                let mut reader = self.inner.lock().map_err(|_| crate::Error::InvalidConfiguration("reader lock poisoned".into()))?;
                let position = match whence {
                    0 if offset >= 0 => SeekFrom::Start(offset as u64),
                    0 => return Err(crate::Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "negative absolute seek"))),
                    1 => SeekFrom::Current(offset),
                    2 => SeekFrom::End(offset),
                    _ => return Err(crate::Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid whence"))),
                };
                reader.seek(position).map_err(crate::Error::from)
            })
            .map_err(python_error)
        }

        fn tell(&self) -> PyResult<u64> {
            self.inner.lock().map(|reader| reader.position()).map_err(|_| PyValueError::new_err("reader lock poisoned"))
        }

        #[getter]
        fn size(&self) -> PyResult<u64> {
            self.inner.lock().map(|reader| reader.size()).map_err(|_| PyValueError::new_err("reader lock poisoned"))
        }

        fn index_bytes(&self, py: Python<'_>) -> PyResult<Py<PyBytes>> {
            let encoded = self.inner.lock().map(|reader| reader.index().to_bytes()).map_err(|_| PyValueError::new_err("reader lock poisoned"))?;
            Ok(PyBytes::new(py, &encoded).unbind())
        }
    }

    #[pymodule]
    fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add_function(wrap_pyfunction!(py_scan, m)?)?;
        m.add_function(wrap_pyfunction!(py_bz2_crc32, m)?)?;
        m.add_function(wrap_pyfunction!(py_decompress, m)?)?;
        m.add_function(wrap_pyfunction!(py_build_index, m)?)?;
        m.add_function(wrap_pyfunction!(py_test, m)?)?;
        m.add_class::<PyIndexedReader>()?;
        m.add("BadBzip2File", m.py().get_type::<BadBzip2File>())?;
        m.add("__version__", env!("CARGO_PKG_VERSION"))?;
        Ok(())
    }
}
