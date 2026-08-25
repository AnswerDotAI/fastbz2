use std::io::{self, Write};

/// Receives decoded bytes and can take ownership of decoder chunks.
///
/// Implementations that cannot use ownership only need to implement
/// `write_borrowed`. The default `write_owned_from` forwards the suffix.
pub trait OutputSink {
    fn write_borrowed(&mut self, bytes: &[u8]) -> io::Result<()>;

    fn write_owned_from(&mut self, bytes: Vec<u8>, start: usize) -> io::Result<()> {
        let suffix = bytes.get(start..).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "owned chunk start exceeds its length"))?;
        self.write_borrowed(suffix)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<S: OutputSink + ?Sized> OutputSink for &mut S {
    fn write_borrowed(&mut self, bytes: &[u8]) -> io::Result<()> {
        (**self).write_borrowed(bytes)
    }

    fn write_owned_from(&mut self, bytes: Vec<u8>, start: usize) -> io::Result<()> {
        (**self).write_owned_from(bytes, start)
    }

    fn flush(&mut self) -> io::Result<()> {
        (**self).flush()
    }
}

/// Adapts any `std::io::Write` destination to an `OutputSink`.
pub struct WriterSink<W> {
    writer: W,
}

impl<W> WriterSink<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }
}

impl<W: Write> OutputSink for WriterSink<W> {
    fn write_borrowed(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_all(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}
