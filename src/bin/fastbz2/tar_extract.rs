use std::{
    cmp,
    io::{self, Read},
    path::Path,
    sync::mpsc::{Receiver, SyncSender, sync_channel},
    thread,
};

use fastbz2::{Error, OutputSink, Result};

use super::archive_extract;

struct Chunk {
    bytes: Vec<u8>,
    offset: usize,
}

pub(super) struct PipeWriter {
    sender: SyncSender<Chunk>,
}

impl PipeWriter {
    fn send(&self, bytes: Vec<u8>, offset: usize) -> io::Result<()> {
        let suffix = bytes.get(offset..).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "owned chunk start exceeds its length"))?;
        if suffix.is_empty() {
            return Ok(());
        }
        self.sender.send(Chunk { bytes, offset }).map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "tar extractor stopped reading"))
    }
}

impl OutputSink for PipeWriter {
    fn write_borrowed(&mut self, buffer: &[u8]) -> io::Result<()> {
        self.send(buffer.to_vec(), 0)
    }

    fn write_owned_from(&mut self, buffer: Vec<u8>, start: usize) -> io::Result<()> {
        self.send(buffer, start)
    }
}

struct PipeReader {
    receiver: Receiver<Chunk>,
    chunk: Vec<u8>,
    offset: usize,
}

impl Read for PipeReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.offset == self.chunk.len() {
            match self.receiver.recv() {
                Ok(chunk) => {
                    self.chunk = chunk.bytes;
                    self.offset = chunk.offset;
                }
                Err(_) => return Ok(0),
            }
        }
        let count = cmp::min(buffer.len(), self.chunk.len() - self.offset);
        buffer[..count].copy_from_slice(&self.chunk[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
}

fn pipe() -> (PipeWriter, PipeReader) {
    let (sender, receiver) = sync_channel(0);
    (PipeWriter { sender }, PipeReader { receiver, chunk: Vec::new(), offset: 0 })
}

fn broken_pipe(error: &Error) -> bool {
    matches!(error, Error::Io(source) if source.kind() == io::ErrorKind::BrokenPipe)
}

pub(super) fn unpack<F>(destination: &Path, overwrite: bool, decode: F) -> Result<()>
where
    F: FnOnce(&mut PipeWriter) -> Result<()> + Send,
{
    let staging = archive_extract::staging(destination)?;
    thread::scope(|scope| {
        let (mut writer, mut reader) = pipe();
        let decoder = scope.spawn(move || decode(&mut writer));
        let extracted = {
            let mut archive = tar::Archive::new(&mut reader);
            archive.set_overwrite(true);
            archive.unpack(staging.path()).map_err(Error::from)
        };

        let drained = if extracted.is_ok() { io::copy(&mut reader, &mut io::sink()).map(|_| ()).map_err(Error::from) } else { Ok(()) };
        drop(reader);

        let decoded = decoder.join().map_err(|_| Error::InvalidConfiguration("decoder worker panicked while extracting tar".into()))?;
        match (decoded, extracted, drained) {
            (Ok(()), Ok(()), Ok(())) => archive_extract::commit(staging.path(), destination, overwrite),
            (Err(error), Err(archive_error), _) if broken_pipe(&error) => Err(archive_error),
            (Err(error), _, _) => Err(error),
            (Ok(()), Err(error), _) | (Ok(()), Ok(()), Err(error)) => Err(error),
        }
    })
}
