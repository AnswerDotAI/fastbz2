from collections import namedtuple
import io, os
from pathlib import Path

from ._core import BadCompressedFile, _IndexedReader, _Reader, __version__, _build_index, _compress, _decompress, _scan, _test_bytes, _test_path, bz2_crc32

DEFAULT_MEMORY_LIMIT = 1024 * 1024 * 1024
DEFAULT_CACHE_LIMIT = 64 * 1024 * 1024

StreamHeaderCandidate = namedtuple("StreamHeaderCandidate", "byte_offset block_size_100k")
BlockCandidate = namedtuple("BlockCandidate", "bit_offset expected_crc randomized orig_ptr")
EndCandidate = namedtuple("EndCandidate", "bit_offset expected_stream_crc")
ScanResult = namedtuple("ScanResult", "streams blocks stream_ends")

def scan(data: bytes) -> ScanResult:
    """Find candidate stream headers and bit-level block markers in *data*.

    This is a structural scan rather than full validation. Marker patterns after
    the first header remain candidates until a decoder validates the stream.
    """
    streams, blocks, stream_ends = _scan(data)
    streams = [StreamHeaderCandidate(*item) for item in streams]
    blocks = [BlockCandidate(*item) for item in blocks]
    stream_ends = [EndCandidate(*item) for item in stream_ends]
    return ScanResult(streams, blocks, stream_ends)

def decompress(data: bytes, format=None, *, threads=0, memory_limit=DEFAULT_MEMORY_LIMIT) -> bytes:
    "Decompress and validate bzip2, gzip, or LZ4 bytes, selected by magic unless *format* is given."
    return _decompress(data, format, threads, memory_limit)

def compress(data: bytes, format: str, *, threads=0, memory_limit=DEFAULT_MEMORY_LIMIT, level=None) -> bytes:
    "Compress *data* as bzip2, gzip, or LZ4."
    return _compress(data, format, threads, memory_limit, level)

class IndexedBzip2File(io.RawIOBase):
    "Seekable binary reader backed by a validated bzip2 block index."

    def __init__(self, source, *, threads=0, index=None, memory_limit=DEFAULT_MEMORY_LIMIT, cache_limit=DEFAULT_CACHE_LIMIT):
        super().__init__()
        if isinstance(source, (bytes, bytearray, memoryview)):
            if index is not None and not isinstance(index, (bytes, bytearray, memoryview)): index = Path(index).read_bytes()
            self._reader = _IndexedReader.from_bytes(bytes(source), threads, memory_limit, index, cache_limit)
        else:
            if index is not None and isinstance(index, (bytes, bytearray, memoryview)):
                raise TypeError("an in-memory index requires an in-memory bzip2 source")
            index_path = None if index is None else os.fspath(index)
            self._reader = _IndexedReader.from_path(os.fspath(source), threads, memory_limit, index_path, cache_limit)

    def readable(self): return True
    def seekable(self): return True

    def read(self, size=-1):
        self._checkClosed()
        return self._reader.read(size)

    def readinto(self, buffer):
        data = self.read(len(buffer))
        buffer[:len(data)] = data
        return len(data)

    def seek(self, offset, whence=io.SEEK_SET):
        self._checkClosed()
        return self._reader.seek(offset, whence)

    def tell(self):
        self._checkClosed()
        return self._reader.tell()

    @property
    def size(self):
        self._checkClosed()
        return self._reader.size

    def index_bytes(self):
        self._checkClosed()
        return self._reader.index_bytes()

    def close(self):
        self._reader = None
        super().close()

class Reader(io.RawIOBase):
    "Streaming binary reader for a bzip2, gzip, or LZ4 file."

    def __init__(self, path, *, format=None, threads=0, memory_limit=DEFAULT_MEMORY_LIMIT):
        super().__init__()
        self._reader = _Reader.from_path(os.fspath(path), format, threads, memory_limit)

    def readable(self): return True

    def read(self, size=-1):
        self._checkClosed()
        return self._reader.read(size)

    def readinto(self, buffer):
        self._checkClosed()
        return self._reader.readinto(buffer)

    def close(self):
        self._reader = None
        super().close()

def open(path, *, format=None, threads=0, memory_limit=DEFAULT_MEMORY_LIMIT):
    "Open a bzip2, gzip, or LZ4 path as a streaming binary file."
    return Reader(path, format=format, threads=threads, memory_limit=memory_limit)

def open_indexed(source, *, threads=0, index=None, memory_limit=DEFAULT_MEMORY_LIMIT, cache_limit=DEFAULT_CACHE_LIMIT):
    "Open a path or bytes object as a seekable indexed bzip2 binary file."
    return IndexedBzip2File(source, threads=threads, index=index, memory_limit=memory_limit, cache_limit=cache_limit)

def build_index(source, path=None, *, threads=0, memory_limit=DEFAULT_MEMORY_LIMIT) -> bytes:
    "Fully validate *source* and return its source-bound binary block index."
    if isinstance(source, (bytes, bytearray, memoryview)):
        reader = _IndexedReader.from_bytes(bytes(source), threads, memory_limit, None, DEFAULT_CACHE_LIMIT)
        encoded = reader.index_bytes()
    else: encoded = _build_index(os.fspath(source), threads, memory_limit)
    if path is not None: Path(path).write_bytes(encoded)
    return encoded

def test(source, format=None, *, threads=0, memory_limit=DEFAULT_MEMORY_LIMIT):
    "Fully decode and validate a bzip2, gzip, or LZ4 *source*, returning ``None`` on success."
    if isinstance(source, (bytes, bytearray, memoryview)): _test_bytes(bytes(source), format, threads, memory_limit)
    else: _test_path(os.fspath(source), format, threads, memory_limit)

__all__ = [
    "__version__", "BadCompressedFile", "BlockCandidate", "DEFAULT_CACHE_LIMIT", "DEFAULT_MEMORY_LIMIT", "EndCandidate",
    "IndexedBzip2File", "Reader", "ScanResult", "StreamHeaderCandidate", "build_index", "bz2_crc32", "compress", "decompress", "open", "open_indexed", "scan", "test"
]
