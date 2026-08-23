use std::{
    collections::HashMap,
    io::Write,
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
};

use rayon::{ThreadPool, ThreadPoolBuilder};

use crate::format::scan_with_pool;
use crate::{BlockCandidate, BlockIndex, DecodeError, EndCandidate, Error, Index, MAX_DECODED_BLOCK, Result, StreamIndex, combine_stream_crc, decoder};

pub const DEFAULT_MEMORY_LIMIT: usize = 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct DecodeOptions {
    /// Zero selects the process's available parallelism.
    pub threads: usize,
    /// Maximum decoded bytes reserved for in-flight and completed speculative blocks.
    pub memory_limit: usize,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self { threads: 0, memory_limit: DEFAULT_MEMORY_LIMIT }
    }
}

impl DecodeOptions {
    pub fn resolved_threads(self) -> usize {
        if self.threads != 0 { self.threads } else { thread::available_parallelism().map(usize::from).unwrap_or(1) }
    }

    fn validate(self) -> Result<Self> {
        if self.memory_limit < MAX_DECODED_BLOCK {
            return Err(Error::InvalidConfiguration(format!("memory limit must be at least {MAX_DECODED_BLOCK} bytes")));
        }
        Ok(self)
    }
}

#[derive(Clone, Debug)]
enum Marker {
    Block(BlockCandidate),
    End(EndCandidate),
}

impl Marker {
    fn bit_offset(&self) -> u64 {
        match self {
            Self::Block(block) => block.bit_offset,
            Self::End(end) => end.bit_offset,
        }
    }
}

pub fn decompress(data: &[u8], options: DecodeOptions) -> Result<Vec<u8>> {
    let options = options.validate()?;
    if options.resolved_threads() == 1 {
        let mut output = Vec::new();
        decoder::decode_serial(data, &mut output)?;
        return Ok(output);
    }
    let mut output = Vec::new();
    decompress_to_writer(data, &mut output, options)?;
    Ok(output)
}

/// Decode to a streaming output. A one-thread request uses the same pure-Rust
/// block codec without the indexing/speculation overhead.
pub fn decompress_to_writer(data: &[u8], output: &mut impl Write, options: DecodeOptions) -> Result<()> {
    let options = options.validate()?;
    if options.resolved_threads() == 1 {
        return decoder::decode_serial(data, output);
    }
    decode_to_writer(data, output, options).map(|_| ())
}

pub fn build_index(data: &[u8], options: DecodeOptions) -> Result<Index> {
    decode_to_writer(data, &mut std::io::sink(), options)
}

/// Decode a complete bzip2 input, validate every CRC, and build its block index.
pub fn decode_to_writer(data: &[u8], output: &mut impl Write, options: DecodeOptions) -> Result<Index> {
    let options = options.validate()?;
    let threads = options.resolved_threads();
    let pool = thread_pool(threads)?;
    let scanned = scan_with_pool(data, pool.as_deref())?;
    let mut markers: Vec<_> = scanned.blocks.into_iter().map(Marker::Block).collect();
    markers.extend(scanned.stream_ends.into_iter().map(Marker::End));
    markers.sort_unstable_by_key(Marker::bit_offset);
    markers.dedup_by_key(|marker| marker.bit_offset());

    if markers.len() > data.len() / 16 + 64 {
        return Err(Error::InvalidConfiguration("input contains too many speculative markers".into()));
    }

    let Some(pool) = pool else {
        let mut candidates = SerialCandidates { data, markers: &markers };
        return assemble(data, output, &markers, &mut candidates);
    };
    let jobs: Vec<_> = markers
        .iter()
        .enumerate()
        .filter_map(|(marker_index, marker)| match marker {
            Marker::Block(block) => Some(CandidateJob { marker_index, start_bit: block.bit_offset, expected_crc: block.expected_crc }),
            Marker::End(_) => None,
        })
        .collect();
    let work = WorkQueue::new(options.memory_limit);
    let (sender, receiver) = mpsc::channel();

    thread::scope(|scope| {
        let worker = scope.spawn(|| {
            pool.broadcast(|_| worker_loop(data, &jobs, &work, &sender));
        });
        let result = {
            let mut candidates = ParallelCandidates { receiver, ready: HashMap::new(), work: &work };
            assemble(data, output, &markers, &mut candidates)
        };
        work.cancel();
        worker.join().map_err(|_| Error::InvalidConfiguration("parallel decoder worker panicked".into()))?;
        result
    })
}

fn assemble(data: &[u8], output: &mut impl Write, markers: &[Marker], candidates: &mut impl Candidates) -> Result<Index> {
    let mut blocks = Vec::new();
    let mut streams = Vec::new();
    let mut decoded_offset = 0_u64;
    let mut header_byte = 0_u64;

    while header_byte < data.len() as u64 {
        let level = parse_header(data, header_byte)?;
        let stream_number = streams.len() as u64;
        let first_block = blocks.len() as u64;
        let stream_decoded_start = decoded_offset;
        let mut combined_crc = 0_u32;
        let mut current_bit = header_byte.checked_mul(8).and_then(|bit| bit.checked_add(32)).ok_or_else(offset_overflow)?;
        let mut marker_index = marker_at(markers, current_bit)?;
        loop {
            match &markers[marker_index] {
                Marker::End(end) => {
                    if end.expected_stream_crc != combined_crc {
                        return Err(Error::Decode { bit_offset: end.bit_offset, source: DecodeError::CrcMismatch });
                    }
                    let after_eos = end.bit_offset.checked_add(80).ok_or_else(offset_overflow)?;
                    header_byte = after_eos.checked_add(7).ok_or_else(offset_overflow)? / 8;
                    streams.push(StreamIndex {
                        compressed_header_byte: current_stream_header(current_bit, first_block, &blocks, header_byte),
                        block_size_100k: level,
                        first_block,
                        block_count: blocks.len() as u64 - first_block,
                        decoded_start: stream_decoded_start,
                        decoded_len: decoded_offset - stream_decoded_start,
                        eos_bit: end.bit_offset,
                        expected_stream_crc: end.expected_stream_crc,
                    });
                    break;
                }
                Marker::Block(block) => {
                    let decoded = candidates.take(marker_index)?;
                    if decoded.block_len > usize::from(level) * 100_000 {
                        return Err(Error::Decode { bit_offset: block.bit_offset, source: DecodeError::BlockOverflow });
                    }
                    let end_index = marker_at(markers, decoded.end_bit)?;
                    candidates.discard_before(end_index);
                    output.write_all(&decoded.output)?;
                    let decoded_len = decoded.output.len() as u64;
                    blocks.push(BlockIndex {
                        compressed_start_bit: block.bit_offset,
                        compressed_end_bit: markers[end_index].bit_offset(),
                        decoded_start: decoded_offset,
                        decoded_len,
                        expected_crc: block.expected_crc,
                        stream: stream_number,
                    });
                    decoded_offset = decoded_offset.checked_add(decoded_len).ok_or_else(offset_overflow)?;
                    combined_crc = combine_stream_crc(combined_crc, block.expected_crc);
                    current_bit = markers[end_index].bit_offset();
                    marker_index = end_index;
                }
            }
        }
        if header_byte == data.len() as u64 {
            break;
        }
        if header_byte > data.len() as u64 {
            return Err(Error::Decode { bit_offset: data.len() as u64 * 8, source: DecodeError::Truncated });
        }
    }

    output.flush()?;
    Ok(Index::new(data, decoded_offset, streams, blocks))
}

trait Candidates {
    fn take(&mut self, marker_index: usize) -> Result<decoder::DecodedCandidate>;
    fn discard_before(&mut self, marker_index: usize);
}

struct SerialCandidates<'a> {
    data: &'a [u8],
    markers: &'a [Marker],
}

impl Candidates for SerialCandidates<'_> {
    fn take(&mut self, marker_index: usize) -> Result<decoder::DecodedCandidate> {
        let Marker::Block(block) = &self.markers[marker_index] else { return Err(required_marker(self.markers[marker_index].bit_offset())) };
        decoder::decode_candidate(self.data, block.bit_offset, block.expected_crc)
    }

    fn discard_before(&mut self, _marker_index: usize) {}
}

#[derive(Clone, Copy)]
struct CandidateJob {
    marker_index: usize,
    start_bit: u64,
    expected_crc: u32,
}

struct WorkState {
    next: usize,
    reserved: usize,
    cancelled: bool,
}

struct WorkQueue {
    limit: usize,
    state: Mutex<WorkState>,
    wake: Condvar,
}

impl WorkQueue {
    fn new(limit: usize) -> Self {
        Self { limit, state: Mutex::new(WorkState { next: 0, reserved: 0, cancelled: false }), wake: Condvar::new() }
    }

    fn next(&self, job_count: usize) -> Option<usize> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        while state.reserved > self.limit - MAX_DECODED_BLOCK && state.next < job_count && !state.cancelled {
            state = self.wake.wait(state).unwrap_or_else(|error| error.into_inner());
        }
        if state.cancelled || state.next >= job_count {
            return None;
        }
        let next = state.next;
        state.next += 1;
        state.reserved += MAX_DECODED_BLOCK;
        Some(next)
    }

    fn complete(&self, decoded_len: usize) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.reserved -= MAX_DECODED_BLOCK - decoded_len;
        self.wake.notify_all();
    }

    fn retire(&self, decoded_len: usize) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.reserved -= decoded_len;
        self.wake.notify_all();
    }

    fn cancel(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.cancelled = true;
        self.wake.notify_all();
    }
}

type CandidateResult = (usize, Result<decoder::DecodedCandidate>);

fn worker_loop(data: &[u8], jobs: &[CandidateJob], work: &WorkQueue, sender: &mpsc::Sender<CandidateResult>) {
    while let Some(index) = work.next(jobs.len()) {
        let job = jobs[index];
        let result = decoder::decode_candidate(data, job.start_bit, job.expected_crc);
        let decoded_len = candidate_len(&result);
        work.complete(decoded_len);
        if sender.send((job.marker_index, result)).is_err() {
            work.retire(decoded_len);
            return;
        }
    }
}

fn candidate_len(result: &Result<decoder::DecodedCandidate>) -> usize {
    result.as_ref().map_or(0, |decoded| decoded.output.len())
}

struct ParallelCandidates<'a> {
    receiver: mpsc::Receiver<CandidateResult>,
    ready: HashMap<usize, Result<decoder::DecodedCandidate>>,
    work: &'a WorkQueue,
}

impl Candidates for ParallelCandidates<'_> {
    fn take(&mut self, marker_index: usize) -> Result<decoder::DecodedCandidate> {
        while !self.ready.contains_key(&marker_index) {
            let (index, result) = self.receiver.recv().map_err(|_| Error::InvalidConfiguration("parallel decoder stopped early".into()))?;
            if index < marker_index {
                self.work.retire(candidate_len(&result));
            } else {
                self.ready.insert(index, result);
            }
        }
        let result = self.ready.remove(&marker_index).unwrap();
        self.work.retire(candidate_len(&result));
        result
    }

    fn discard_before(&mut self, marker_index: usize) {
        let stale: Vec<_> = self.ready.keys().copied().filter(|&index| index < marker_index).collect();
        for index in stale {
            let result = self.ready.remove(&index).unwrap();
            self.work.retire(candidate_len(&result));
        }
    }
}

fn thread_pool(threads: usize) -> Result<Option<Arc<ThreadPool>>> {
    if threads <= 1 {
        return Ok(None);
    }
    ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|number| format!("fastbz2-{number}"))
        .build()
        .map(Arc::new)
        .map(Some)
        .map_err(|error| Error::InvalidConfiguration(error.to_string()))
}

fn marker_at(markers: &[Marker], bit_offset: u64) -> Result<usize> {
    markers.binary_search_by_key(&bit_offset, Marker::bit_offset).map_err(|_| required_marker(bit_offset))
}

fn parse_header(data: &[u8], byte_offset: u64) -> Result<u8> {
    let offset = usize::try_from(byte_offset).map_err(|_| offset_overflow())?;
    let Some(header) = data.get(offset..offset.saturating_add(4)) else {
        return Err(Error::Decode { bit_offset: byte_offset.saturating_mul(8), source: DecodeError::Truncated });
    };
    if &header[..3] != b"BZh" {
        return Err(Error::Decode { bit_offset: byte_offset * 8, source: DecodeError::InvalidMagic });
    }
    if !(b'1'..=b'9').contains(&header[3]) {
        return Err(Error::Decode { bit_offset: byte_offset * 8 + 24, source: DecodeError::InvalidLevel });
    }
    Ok(header[3] - b'0')
}

fn current_stream_header(current_bit: u64, first_block: u64, blocks: &[BlockIndex], next_header: u64) -> u64 {
    if let Some(first) = blocks.get(first_block as usize) {
        (first.compressed_start_bit - 32) / 8
    } else {
        // Empty streams have no block from which to derive the header. `current_bit`
        // is their EOS position, exactly 32 bits after the header.
        let candidate = current_bit.saturating_sub(32) / 8;
        candidate.min(next_header)
    }
}

fn required_marker(bit_offset: u64) -> Error {
    Error::Decode { bit_offset, source: DecodeError::InvalidMagic }
}

fn offset_overflow() -> Error {
    Error::InvalidConfiguration("offset arithmetic overflow".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crabz2::{Level, compress};

    fn patterned(size: usize) -> Vec<u8> {
        (0..size).map(|index| ((index * 37 + index / 251) & 255) as u8).collect()
    }

    #[test]
    fn decodes_multiple_blocks_at_every_thread_setting() {
        let plain = patterned(350_000);
        let compressed = compress(&plain, Level::FASTEST);
        for threads in [1, 2, 4, 0] {
            let options = DecodeOptions { threads, ..DecodeOptions::default() };
            assert_eq!(decompress(&compressed, options).unwrap(), plain);
        }
    }

    #[test]
    fn validates_concatenated_streams_and_indexes_them() {
        let first = patterned(180_000);
        let second = patterned(70_000);
        let mut compressed = compress(&first, Level::FASTEST);
        compressed.extend_from_slice(&compress(&second, Level::BEST));
        let mut expected = first;
        expected.extend_from_slice(&second);

        let mut output = Vec::new();
        let index = decode_to_writer(&compressed, &mut output, DecodeOptions::default()).unwrap();
        assert_eq!(output, expected);
        assert_eq!(index.streams.len(), 2);
        assert_eq!(index.decoded_len, expected.len() as u64);
        assert!(index.blocks.len() >= 3);
    }

    #[test]
    fn bounded_scheduler_crosses_many_short_streams() {
        let mut compressed = Vec::new();
        let mut expected = Vec::new();
        for stream in 0..32 {
            let plain = patterned(4_000 + stream * 31);
            compressed.extend_from_slice(&compress(&plain, Level::BEST));
            expected.extend_from_slice(&plain);
        }
        let options = DecodeOptions { threads: 4, memory_limit: MAX_DECODED_BLOCK * 2 };
        assert_eq!(decompress(&compressed, options).unwrap(), expected);
    }

    #[test]
    fn rejects_stream_crc_corruption() {
        let mut compressed = compress(b"integrity matters", Level::BEST);
        let last = compressed.len() - 2;
        compressed[last] ^= 1;
        assert!(matches!(decompress(&compressed, DecodeOptions::default()), Err(Error::Decode { .. })));
    }
}
