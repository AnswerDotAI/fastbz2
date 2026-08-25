//! LZ4 frame and block decompression implemented in safe Rust.

use std::{hash::Hasher, io::Write};

use rayon::ThreadPoolBuilder;
use twox_hash::XxHash32;

use crate::history::extend_match;
use crate::pipeline::{Job, PipelineLimits, run_ordered};
use crate::{DecodeOptions, DecodeProgress, Error, OutputSink, Result, WriterSink};

const FRAME_MAGIC: u32 = 0x184d_2204;
const LEGACY_MAGIC: u32 = 0x184c_2102;
const SKIPPABLE_MAGIC_START: u32 = 0x184d_2a50;
const SKIPPABLE_MAGIC_END: u32 = 0x184d_2a5f;
const UNCOMPRESSED_BIT: u32 = 1 << 31;
const WINDOW_SIZE: usize = 64 * 1024;
const MIN_PARALLEL_INPUT: usize = 1024 * 1024;
const AUTO_THREAD_LIMIT: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockMode {
    Independent,
    Linked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub compressed_start: u64,
    pub compressed_end: u64,
    pub decoded_start: u64,
    pub decoded_len: u64,
    pub block_max_size: u32,
    pub block_mode: BlockMode,
    pub block_checksums: bool,
    pub content_checksum: bool,
    pub declared_content_size: Option<u64>,
    pub first_block: usize,
    pub block_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub frame: u32,
    pub compressed_start: u64,
    pub compressed_end: u64,
    pub decoded_start: u64,
    pub decoded_len: u64,
    pub stored: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub source_len: u64,
    pub decoded_len: u64,
    pub frames: Vec<Frame>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug)]
struct BlockLayout {
    data_start: usize,
    data_end: usize,
    stored: bool,
    expected_checksum: Option<u32>,
}

#[derive(Clone, Debug)]
struct FrameLayout {
    source_start: usize,
    source_end: usize,
    max_block_size: usize,
    mode: BlockMode,
    block_checksums: bool,
    content_checksum: bool,
    content_size: Option<u64>,
    expected_content_checksum: Option<u32>,
    blocks: Vec<BlockLayout>,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidLz4(message.into())
}

fn worker_threads(options: DecodeOptions) -> usize {
    let requested = options.resolved_threads();
    if options.threads == 0 { requested.min(AUTO_THREAD_LIMIT) } else { requested }
}

fn read_u32(data: &[u8], position: usize, context: &str) -> Result<u32> {
    let bytes = data.get(position..position.saturating_add(4)).ok_or_else(|| invalid(format!("truncated {context} at byte {position}")))?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

fn xxhash32(data: &[u8]) -> u32 {
    let mut hasher = XxHash32::with_seed(0);
    hasher.write(data);
    hasher.finish() as u32
}

fn parse_frame(data: &[u8], start: usize) -> Result<FrameLayout> {
    let flg = *data.get(start + 4).ok_or_else(|| invalid("truncated frame descriptor"))?;
    let bd = *data.get(start + 5).ok_or_else(|| invalid("truncated frame descriptor"))?;
    if flg & 0xc0 != 0x40 {
        return Err(invalid(format!("unsupported frame version bits {:02x}", flg & 0xc0)));
    }
    if flg & 0x02 != 0 || bd & 0x8f != 0 {
        return Err(invalid("reserved frame descriptor bits are set"));
    }
    let mode = if flg & 0x20 != 0 { BlockMode::Independent } else { BlockMode::Linked };
    let block_checksums = flg & 0x10 != 0;
    let has_content_size = flg & 0x08 != 0;
    let content_checksum = flg & 0x04 != 0;
    let has_dictionary = flg & 0x01 != 0;
    let max_block_size = match (bd >> 4) & 0x07 {
        4 => 64 * 1024,
        5 => 256 * 1024,
        6 => 1024 * 1024,
        7 => 4 * 1024 * 1024,
        value => return Err(invalid(format!("unsupported block maximum-size code {value}"))),
    };
    let mut position = start + 6;
    let content_size = if has_content_size {
        let bytes = data.get(position..position.saturating_add(8)).ok_or_else(|| invalid("truncated frame content size"))?;
        position += 8;
        Some(u64::from_le_bytes(bytes.try_into().unwrap()))
    } else {
        None
    };
    if has_dictionary {
        let dictionary = read_u32(data, position, "dictionary ID")?;
        return Err(invalid(format!("external dictionary {dictionary:08x} is not supported")));
    }
    let expected_header_checksum = *data.get(position).ok_or_else(|| invalid("truncated frame header checksum"))?;
    let descriptor = data.get(start + 4..position).ok_or_else(|| invalid("invalid frame descriptor range"))?;
    let header_checksum = (xxhash32(descriptor) >> 8) as u8;
    if header_checksum != expected_header_checksum {
        return Err(invalid(format!("header checksum mismatch: expected {expected_header_checksum:02x}, decoded {header_checksum:02x}")));
    }
    position += 1;
    let mut blocks = Vec::new();
    loop {
        let value = read_u32(data, position, "block header")?;
        position += 4;
        if value == 0 {
            break;
        }
        let stored = value & UNCOMPRESSED_BIT != 0;
        let size = (value & !UNCOMPRESSED_BIT) as usize;
        if size == 0 || size > max_block_size {
            return Err(invalid(format!("block at byte {} has invalid size {size} for {max_block_size}-byte frames", position - 4)));
        }
        let data_start = position;
        let data_end = position.checked_add(size).filter(|&end| end <= data.len()).ok_or_else(|| invalid("block data exceeds the frame"))?;
        position = data_end;
        let expected_checksum = if block_checksums {
            let checksum = read_u32(data, position, "block checksum")?;
            position += 4;
            Some(checksum)
        } else {
            None
        };
        blocks.push(BlockLayout { data_start, data_end, stored, expected_checksum });
    }
    let expected_content_checksum = if content_checksum {
        let checksum = read_u32(data, position, "content checksum")?;
        position += 4;
        Some(checksum)
    } else {
        None
    };
    Ok(FrameLayout {
        source_start: start,
        source_end: position,
        max_block_size,
        mode,
        block_checksums,
        content_checksum,
        content_size,
        expected_content_checksum,
        blocks,
    })
}

fn parse(data: &[u8]) -> Result<Vec<FrameLayout>> {
    let mut frames = Vec::new();
    let mut position = 0;
    while position < data.len() {
        let magic = read_u32(data, position, "frame magic")?;
        if (SKIPPABLE_MAGIC_START..=SKIPPABLE_MAGIC_END).contains(&magic) {
            let length = read_u32(data, position + 4, "skippable-frame length")? as usize;
            position = position
                .checked_add(8)
                .and_then(|value| value.checked_add(length))
                .filter(|&end| end <= data.len())
                .ok_or_else(|| invalid("skippable frame exceeds the input"))?;
            continue;
        }
        if magic == LEGACY_MAGIC {
            return Err(invalid("legacy LZ4 frames are not supported"));
        }
        if magic != FRAME_MAGIC {
            return Err(invalid(format!("wrong magic {magic:08x} at byte {position}")));
        }
        let frame = parse_frame(data, position)?;
        position = frame.source_end;
        frames.push(frame);
    }
    if frames.is_empty() {
        return Err(invalid("input contains no LZ4 frames"));
    }
    Ok(frames)
}

fn read_length(input: &[u8], position: &mut usize, initial: usize) -> Result<usize> {
    if initial != 15 {
        return Ok(initial);
    }
    let mut length = initial;
    loop {
        let extra = *input.get(*position).ok_or_else(|| invalid("truncated extended sequence length"))? as usize;
        *position += 1;
        length = length.checked_add(extra).ok_or_else(|| invalid("sequence length overflows usize"))?;
        if extra != 255 {
            return Ok(length);
        }
    }
}

fn copy_match(output: &mut Vec<u8>, dictionary: &[u8], offset: usize, mut length: usize) -> Result<()> {
    if offset == 0 || offset > dictionary.len() + output.len() {
        return Err(invalid(format!("match offset {offset} exceeds the available {}-byte history", dictionary.len() + output.len())));
    }
    let dictionary_needed = offset.saturating_sub(output.len());
    if dictionary_needed != 0 {
        let start = dictionary.len().checked_sub(dictionary_needed).ok_or_else(|| invalid("match offset exceeds the external dictionary"))?;
        let take = length.min(dictionary_needed);
        output.extend_from_slice(&dictionary[start..start + take]);
        length -= take;
    }
    if length != 0 {
        extend_match(output, offset, length);
    }
    Ok(())
}

fn decompress_block(input: &[u8], dictionary: &[u8], max_output: usize) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(max_output);
    let mut position = 0;
    while position < input.len() {
        let token = input[position];
        position += 1;
        let literal_len = read_length(input, &mut position, (token >> 4) as usize)?;
        let literal_end = position.checked_add(literal_len).filter(|&end| end <= input.len()).ok_or_else(|| invalid("literal run exceeds the block"))?;
        if output.len().checked_add(literal_len).is_none_or(|length| length > max_output) {
            return Err(invalid(format!("decoded block exceeds its {max_output}-byte maximum")));
        }
        output.extend_from_slice(&input[position..literal_end]);
        position = literal_end;
        if position == input.len() {
            break;
        }
        let offset_bytes = input.get(position..position.saturating_add(2)).ok_or_else(|| invalid("truncated match offset"))?;
        position += 2;
        let offset = u16::from_le_bytes(offset_bytes.try_into().unwrap()) as usize;
        let match_len = read_length(input, &mut position, (token & 0x0f) as usize)?.checked_add(4).ok_or_else(|| invalid("match length overflows usize"))?;
        if output.len().checked_add(match_len).is_none_or(|length| length > max_output) {
            return Err(invalid(format!("decoded block exceeds its {max_output}-byte maximum")));
        }
        copy_match(&mut output, dictionary, offset, match_len)?;
    }
    Ok(output)
}

enum DecodedBlock {
    Stored { start: usize, end: usize },
    Decoded(Vec<u8>),
}

impl DecodedBlock {
    fn bytes<'a>(&'a self, source: &'a [u8]) -> &'a [u8] {
        match self {
            Self::Stored { start, end } => &source[*start..*end],
            Self::Decoded(bytes) => bytes,
        }
    }

    fn retained_bytes(&self) -> usize {
        match self {
            Self::Stored { .. } => 0,
            Self::Decoded(bytes) => bytes.capacity(),
        }
    }
}

fn decode_layout_block(data: &[u8], block: &BlockLayout, dictionary: &[u8], max_block_size: usize) -> Result<DecodedBlock> {
    let encoded = &data[block.data_start..block.data_end];
    if let Some(expected) = block.expected_checksum {
        let actual = xxhash32(encoded);
        if actual != expected {
            return Err(invalid(format!("block checksum mismatch at byte {}: expected {expected:08x}, decoded {actual:08x}", block.data_start)));
        }
    }
    if block.stored {
        Ok(DecodedBlock::Stored { start: block.data_start, end: block.data_end })
    } else {
        Ok(DecodedBlock::Decoded(decompress_block(encoded, dictionary, max_block_size)?))
    }
}

struct FrameCommitter<'a, S, P> {
    data: &'a [u8],
    output: &'a mut S,
    progress: &'a mut P,
    hasher: Option<XxHash32>,
    decoded_base: u64,
    decoded: u64,
    frame_number: u32,
    blocks: &'a mut Vec<Block>,
}

impl<S: OutputSink, P: FnMut(DecodeProgress)> FrameCommitter<'_, S, P> {
    fn commit(&mut self, layout: &BlockLayout, decoded: DecodedBlock) -> Result<()> {
        let bytes = decoded.bytes(self.data);
        if let Some(hasher) = &mut self.hasher {
            hasher.write(bytes);
        }
        let decoded_len = bytes.len() as u64;
        match decoded {
            DecodedBlock::Stored { start, end } => self.output.write_borrowed(&self.data[start..end])?,
            DecodedBlock::Decoded(bytes) => self.output.write_owned_from(bytes, 0)?,
        }
        self.blocks.push(Block {
            frame: self.frame_number,
            compressed_start: layout.data_start as u64,
            compressed_end: layout.data_end as u64,
            decoded_start: self.decoded_base + self.decoded,
            decoded_len,
            stored: layout.stored,
        });
        self.decoded = self.decoded.checked_add(decoded_len).ok_or_else(|| invalid("decoded length overflows u64"))?;
        (self.progress)(DecodeProgress { compressed_bytes: layout.data_end as u64, decoded_bytes: self.decoded_base + self.decoded });
        Ok(())
    }
}

fn decode_independent<S: OutputSink, P: FnMut(DecodeProgress)>(
    data: &[u8],
    frame: &FrameLayout,
    options: DecodeOptions,
    committer: &mut FrameCommitter<'_, S, P>,
) -> Result<()> {
    let threads = worker_threads(options);
    let source_bytes = frame.source_end - frame.source_start;
    let parallel_work = frame.blocks.iter().any(|block| !block.stored || block.expected_checksum.is_some());
    if threads == 1 || frame.blocks.len() < 2 || source_bytes < MIN_PARALLEL_INPUT || !parallel_work {
        for block in &frame.blocks {
            let decoded = decode_layout_block(data, block, &[], frame.max_block_size)?;
            committer.commit(block, decoded)?;
        }
        return Ok(());
    }
    let jobs: Vec<_> = frame
        .blocks
        .iter()
        .cloned()
        .enumerate()
        .map(|(key, block)| Job { key, reservation: if block.stored { 0 } else { frame.max_block_size }, payload: block })
        .collect();
    let pool =
        ThreadPoolBuilder::new().num_threads(threads).thread_name(|index| format!("fbz-lz4-{index}")).build().map_err(|error| invalid(error.to_string()))?;
    run_ordered(
        &pool,
        &jobs,
        PipelineLimits { memory: options.memory_limit, active: threads.saturating_add(2) },
        |block| decode_layout_block(data, block, &[], frame.max_block_size),
        |result| result.as_ref().map_or(0, DecodedBlock::retained_bytes),
        |results| {
            for (key, block) in frame.blocks.iter().enumerate() {
                committer.commit(block, results.take(key)??)?;
            }
            Ok(())
        },
    )
}

fn update_history(history: &mut Vec<u8>, bytes: &[u8]) {
    if bytes.len() >= WINDOW_SIZE {
        history.clear();
        history.extend_from_slice(&bytes[bytes.len() - WINDOW_SIZE..]);
        return;
    }
    let excess = history.len().saturating_add(bytes.len()).saturating_sub(WINDOW_SIZE);
    if excess != 0 {
        history.drain(..excess);
    }
    history.extend_from_slice(bytes);
}

fn decode_linked<S: OutputSink, P: FnMut(DecodeProgress)>(data: &[u8], frame: &FrameLayout, committer: &mut FrameCommitter<'_, S, P>) -> Result<()> {
    let mut history = Vec::with_capacity(WINDOW_SIZE);
    for block in &frame.blocks {
        let decoded = decode_layout_block(data, block, &history, frame.max_block_size)?;
        update_history(&mut history, decoded.bytes(data));
        committer.commit(block, decoded)?;
    }
    Ok(())
}

pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    decompress_with_options(data, DecodeOptions::default())
}

pub fn decompress_with_options(data: &[u8], options: DecodeOptions) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    decompress_to_writer_with_options(data, &mut output, options)?;
    Ok(output)
}

pub fn decompress_to_writer(data: &[u8], output: &mut impl Write) -> Result<Report> {
    decompress_to_writer_with_options(data, output, DecodeOptions::default())
}

pub fn decompress_to_writer_with_options(data: &[u8], output: &mut impl Write, options: DecodeOptions) -> Result<Report> {
    decompress_to_writer_with_options_and_progress(data, output, options, |_| {})
}

pub fn decompress_to_writer_with_progress(data: &[u8], output: &mut impl Write, progress: impl FnMut(DecodeProgress)) -> Result<Report> {
    decompress_to_writer_with_options_and_progress(data, output, DecodeOptions::default(), progress)
}

pub fn decompress_to_writer_with_options_and_progress(
    data: &[u8],
    output: &mut impl Write,
    options: DecodeOptions,
    progress: impl FnMut(DecodeProgress),
) -> Result<Report> {
    let mut output = WriterSink::new(output);
    decompress_to_sink_with_options_and_progress(data, &mut output, options, progress)
}

#[doc(hidden)]
pub fn decompress_to_sink_with_options_and_progress<S: OutputSink, P: FnMut(DecodeProgress)>(
    data: &[u8],
    output: &mut S,
    options: DecodeOptions,
    mut progress: P,
) -> Result<Report> {
    let options = options.validate()?;
    let layouts = parse(data)?;
    let mut frames = Vec::with_capacity(layouts.len());
    let mut blocks = Vec::new();
    let mut decoded_total = 0_u64;
    for (frame_number, layout) in layouts.iter().enumerate() {
        let first_block = blocks.len();
        let mut committer = FrameCommitter {
            data,
            output,
            progress: &mut progress,
            hasher: layout.content_checksum.then(|| XxHash32::with_seed(0)),
            decoded_base: decoded_total,
            decoded: 0,
            frame_number: u32::try_from(frame_number).map_err(|_| invalid("too many frames"))?,
            blocks: &mut blocks,
        };
        match layout.mode {
            BlockMode::Independent => decode_independent(data, layout, options, &mut committer)?,
            BlockMode::Linked => decode_linked(data, layout, &mut committer)?,
        }
        if let Some(expected) = layout.content_size
            && committer.decoded != expected
        {
            return Err(invalid(format!("content size mismatch: expected {expected}, decoded {}", committer.decoded)));
        }
        if let Some(expected) = layout.expected_content_checksum {
            let actual = committer.hasher.take().unwrap().finish() as u32;
            if actual != expected {
                return Err(invalid(format!("content checksum mismatch: expected {expected:08x}, decoded {actual:08x}")));
            }
        }
        let decoded_len = committer.decoded;
        decoded_total = decoded_total.checked_add(decoded_len).ok_or_else(|| invalid("decoded length overflows u64"))?;
        frames.push(Frame {
            compressed_start: layout.source_start as u64,
            compressed_end: layout.source_end as u64,
            decoded_start: decoded_total - decoded_len,
            decoded_len,
            block_max_size: layout.max_block_size as u32,
            block_mode: layout.mode,
            block_checksums: layout.block_checksums,
            content_checksum: layout.content_checksum,
            declared_content_size: layout.content_size,
            first_block,
            block_count: blocks.len() - first_block,
        });
        progress(DecodeProgress { compressed_bytes: layout.source_end as u64, decoded_bytes: decoded_total });
    }
    output.flush()?;
    progress(DecodeProgress { compressed_bytes: data.len() as u64, decoded_bytes: decoded_total });
    Ok(Report { source_len: data.len() as u64, decoded_len: decoded_total, frames, blocks })
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use lz4_flex::frame::{BlockMode as OracleMode, BlockSize, FrameDecoder, FrameEncoder, FrameInfo};

    use super::*;

    fn oracle_frame(data: &[u8], mode: OracleMode, block_checksums: bool, content_checksum: bool) -> Vec<u8> {
        let info = FrameInfo::new()
            .block_size(BlockSize::Max64KB)
            .block_mode(mode)
            .block_checksums(block_checksums)
            .content_checksum(content_checksum)
            .content_size(Some(data.len() as u64));
        let mut encoder = FrameEncoder::with_frame_info(info, Vec::new());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn decodes_oracle_independent_and_linked_frames() {
        let data = [b"linked and independent LZ4 frames ".repeat(10_000), (0_u8..=255).collect::<Vec<_>>().repeat(1_000)].concat();
        for mode in [OracleMode::Independent, OracleMode::Linked] {
            for checksums in [false, true] {
                let encoded = oracle_frame(&data, mode, checksums, checksums);
                for threads in [1, 4] {
                    assert_eq!(decompress_with_options(&encoded, DecodeOptions { threads, ..DecodeOptions::default() }).unwrap(), data);
                }
            }
        }
    }

    #[test]
    fn handles_concatenated_and_skippable_frames() {
        let first = oracle_frame(b"first", OracleMode::Independent, true, true);
        let second = oracle_frame(b"second", OracleMode::Linked, false, false);
        let mut encoded = first;
        encoded.extend_from_slice(&SKIPPABLE_MAGIC_START.to_le_bytes());
        encoded.extend_from_slice(&3_u32.to_le_bytes());
        encoded.extend_from_slice(b"xyz");
        encoded.extend_from_slice(&second);
        let mut output = Vec::new();
        let report = decompress_to_writer_with_options(&encoded, &mut output, DecodeOptions { threads: 4, ..DecodeOptions::default() }).unwrap();
        assert_eq!(output, b"firstsecond");
        assert_eq!(report.frames.len(), 2);
        assert_eq!(report.decoded_len, 11);
    }

    #[test]
    fn rejects_corruption_and_output_overflow() {
        let data = b"checksum coverage".repeat(10_000);
        let mut encoded = oracle_frame(&data, OracleMode::Independent, true, true);
        let middle = encoded.len() / 2;
        encoded[middle] ^= 1;
        assert!(matches!(decompress(&encoded), Err(Error::InvalidLz4(_))));
        assert!(matches!(decompress_block(&[0x1f, 1, 0, 255, 255, 255, 255], &[], 32), Err(Error::InvalidLz4(_))));
    }

    #[test]
    fn oracle_decodes_a_minimal_fbz_block_frame() {
        let payload = b"literal-only interoperability";
        let mut encoded = Vec::new();
        encoded.extend_from_slice(&FRAME_MAGIC.to_le_bytes());
        let descriptor = [0x64, 0x40];
        encoded.extend_from_slice(&descriptor);
        encoded.push((xxhash32(&descriptor) >> 8) as u8);
        encoded.extend_from_slice(&((payload.len() as u32) | UNCOMPRESSED_BIT).to_le_bytes());
        encoded.extend_from_slice(payload);
        encoded.extend_from_slice(&0_u32.to_le_bytes());
        encoded.extend_from_slice(&xxhash32(payload).to_le_bytes());
        let mut decoder = FrameDecoder::new(encoded.as_slice());
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn automatic_threads_are_bounded_but_explicit_counts_are_preserved() {
        assert!(worker_threads(DecodeOptions::default()) <= AUTO_THREAD_LIMIT);
        assert_eq!(worker_threads(DecodeOptions { threads: 12, ..DecodeOptions::default() }), 12);
    }
}
