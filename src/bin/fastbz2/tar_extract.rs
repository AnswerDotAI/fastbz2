use std::{
    cmp, fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, SyncSender, sync_channel},
    thread,
};

use fastbz2::{Error, OutputSink, Result};
use tempfile::TempDir;

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

fn path_metadata(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn children(path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut result = fs::read_dir(path)?.map(|entry| entry.map(|entry| entry.path())).collect::<io::Result<Vec<_>>>()?;
    result.sort_unstable();
    Ok(result)
}

fn existing_error(path: &Path) -> Error {
    Error::Io(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists (use --force)", path.display())))
}
fn directory_collision(path: &Path) -> Error {
    Error::Io(io::Error::new(io::ErrorKind::AlreadyExists, format!("refusing to replace directory {} with an archive entry", path.display())))
}

fn preflight(source: &Path, target: &Path, overwrite: bool) -> Result<()> {
    let Some(target_metadata) = path_metadata(target)? else {
        return Ok(());
    };
    let source_metadata = fs::symlink_metadata(source)?;
    if source_metadata.is_dir() && target_metadata.is_dir() {
        for child in children(source)? {
            preflight(&child, &target.join(child.file_name().unwrap()), overwrite)?;
        }
        return Ok(());
    }
    if target_metadata.is_dir() {
        return Err(directory_collision(target));
    }
    if overwrite { Ok(()) } else { Err(existing_error(target)) }
}

#[cfg(unix)]
fn make_directory_mutable(path: &Path, metadata: &fs::Metadata) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(metadata.permissions().mode() | 0o700))
}

#[cfg(not(unix))]
fn make_directory_mutable(path: &Path, metadata: &fs::Metadata) -> io::Result<()> {
    let mut permissions = metadata.permissions();
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions)
}

fn commit_entry(source: &Path, target: &Path, overwrite: bool) -> Result<()> {
    let source_metadata = fs::symlink_metadata(source)?;
    let Some(target_metadata) = path_metadata(target)? else {
        fs::rename(source, target)?;
        return Ok(());
    };
    if source_metadata.is_dir() && target_metadata.is_dir() {
        make_directory_mutable(source, &source_metadata)?;
        for child in children(source)? {
            commit_entry(&child, &target.join(child.file_name().unwrap()), overwrite)?;
        }
        fs::remove_dir(source)?;
        return Ok(());
    }
    if target_metadata.is_dir() {
        return Err(directory_collision(target));
    }
    if !overwrite {
        return Err(existing_error(target));
    }
    fs::remove_file(target)?;
    fs::rename(source, target)?;
    Ok(())
}

fn staging(destination: &Path) -> Result<TempDir> {
    let parent = match path_metadata(destination)? {
        Some(metadata) if metadata.is_dir() => destination,
        Some(_) => {
            return Err(Error::Io(io::Error::new(io::ErrorKind::NotADirectory, format!("{} is not a directory", destination.display()))));
        }
        None => destination.parent().filter(|path| !path.as_os_str().is_empty()).unwrap_or_else(|| Path::new(".")),
    };
    fs::create_dir_all(parent)?;
    TempDir::new_in(parent).map_err(Error::from)
}

fn commit(staging: &Path, destination: &Path, overwrite: bool) -> Result<()> {
    if path_metadata(destination)?.is_none() {
        fs::create_dir(destination)?;
    }
    let entries = children(staging)?;
    for source in &entries {
        preflight(source, &destination.join(source.file_name().unwrap()), overwrite)?;
    }
    for source in entries {
        commit_entry(&source, &destination.join(source.file_name().unwrap()), overwrite)?;
    }
    Ok(())
}

pub(super) fn unpack<F>(destination: &Path, overwrite: bool, decode: F) -> Result<()>
where
    F: FnOnce(&mut PipeWriter) -> Result<()> + Send,
{
    let staging = staging(destination)?;
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
            (Ok(()), Ok(()), Ok(())) => commit(staging.path(), destination, overwrite),
            (Err(error), Err(archive_error), _) if broken_pipe(&error) => Err(archive_error),
            (Err(error), _, _) => Err(error),
            (Ok(()), Err(error), _) | (Ok(()), Ok(()), Err(error)) => Err(error),
        }
    })
}
