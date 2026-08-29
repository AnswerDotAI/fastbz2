use std::{fs::File, io, path::Path, sync::Arc};

use memmap2::{Mmap, MmapOptions};

#[derive(Clone)]
pub struct Source(Arc<SourceInner>);

enum SourceInner { Bytes(Vec<u8>), Mmap(Mmap) }

impl Source {
    pub fn from_bytes(data: Vec<u8>) -> Self { Self(Arc::new(SourceInner::Bytes(data))) }

    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = File::open(path)?;
        if file.metadata()?.len() == 0 { return Ok(Self::from_bytes(Vec::new())); }
        // SAFETY: the read-only mapping owns no borrowed file state and `Mmap`
        // keeps the mapping alive until the last cloned `Source` is dropped.
        let mmap = unsafe { MmapOptions::new().map(&file)? };
        Ok(Self(Arc::new(SourceInner::Mmap(mmap))))
    }

    pub fn as_slice(&self) -> &[u8] { match self.0.as_ref() { SourceInner::Bytes(data) => data, SourceInner::Mmap(data) => data } }

    pub fn len(&self) -> usize { self.as_slice().len() }

    pub fn is_empty(&self) -> bool { self.as_slice().is_empty() }
}

impl AsRef<[u8]> for Source { fn as_ref(&self) -> &[u8] { self.as_slice() } }
