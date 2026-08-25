//! Raw DEFLATE decompression shared by gzip and ZIP framing.

use crate::{DecodeOptions, DecodeProgress, OutputSink, Result, gzip};

pub use gzip::DeflateReport as Report;

/// Decode one raw DEFLATE stream into an output sink.
#[doc(hidden)]
pub fn decompress_to_sink_with_options_and_progress(
    data: &[u8],
    output: &mut impl OutputSink,
    options: DecodeOptions,
    progress: impl FnMut(DecodeProgress),
) -> Result<Report> {
    gzip::decompress_deflate_to_sink_with_options_and_progress(data, output, options, progress)
}
