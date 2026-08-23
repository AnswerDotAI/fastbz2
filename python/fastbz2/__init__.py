from collections import namedtuple

from ._core import __version__, _scan, bz2_crc32

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

__all__ = [
    "__version__", "BlockCandidate", "EndCandidate", "ScanResult", "StreamHeaderCandidate", "bz2_crc32", "scan"
]
