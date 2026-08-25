use std::path::Path;

use crate::{DecodeOptions, DecodeProgress, Error, OutputSink, Result, gzip, lz4};

/// A compression or archive format recognized by fbz.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Bzip2,
    Gzip,
    Lz4,
    Zip,
}

impl Format {
    /// Detect a format from stream magic, falling back to the path extension.
    pub fn detect(path: impl AsRef<Path>, data: &[u8]) -> Result<Self> {
        if let Some(format) = Self::from_magic(data) {
            return Ok(format);
        }
        if let Some(format) = Self::from_path(path.as_ref()) {
            return Ok(format);
        }
        Err(Error::UnsupportedFormat(format!(
            "cannot determine compression format for {}; expected a bzip2, gzip, LZ4, or ZIP extension or magic",
            path.as_ref().display()
        )))
    }

    /// Infer a format from a recognized filename extension.
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "bz2" | "bzip2" | "tbz" | "tbz2" => Some(Self::Bzip2),
            "gz" | "gzip" | "tgz" => Some(Self::Gzip),
            "lz4" => Some(Self::Lz4),
            "zip" => Some(Self::Zip),
            _ => None,
        }
    }

    /// Infer a format from its leading magic bytes.
    pub fn from_magic(data: &[u8]) -> Option<Self> {
        if data.starts_with(b"BZh") {
            Some(Self::Bzip2)
        } else if data.starts_with(&[0x1f, 0x8b]) {
            Some(Self::Gzip)
        } else if data.starts_with(&[0x04, 0x22, 0x4d, 0x18])
            || data.get(..4).is_some_and(|magic| (0x184d_2a50..=0x184d_2a5f).contains(&u32::from_le_bytes(magic.try_into().unwrap())))
        {
            Some(Self::Lz4)
        } else if data.starts_with(b"PK\x03\x04") || data.starts_with(b"PK\x05\x06") || data.starts_with(b"PK\x07\x08") {
            Some(Self::Zip)
        } else {
            None
        }
    }
}

/// Decode a single-stream format through the shared owned-chunk output path.
#[doc(hidden)]
pub fn decode_stream_to_sink_with_progress(
    format: Format,
    data: &[u8],
    output: &mut impl OutputSink,
    options: DecodeOptions,
    progress: impl FnMut(DecodeProgress),
) -> Result<()> {
    match format {
        Format::Bzip2 => crate::decompress_to_sink_with_progress(data, output, options, progress),
        Format::Gzip => gzip::decompress_to_sink_with_options_and_progress(data, output, options, progress).map(|_| ()),
        Format::Lz4 => lz4::decompress_to_sink_with_options_and_progress(data, output, options, progress).map(|_| ()),
        Format::Zip => Err(Error::UnsupportedFormat("ZIP archives do not have one decoded byte stream".into())),
    }
}
