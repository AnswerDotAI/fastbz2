import bz2

import pytest

from fbz import bz2_crc32, scan

def combined_crc(blocks):
    crc = 0
    for block in blocks: crc = ((crc << 1) | (crc >> 31)) & 0xffffffff ^ block.expected_crc
    return crc

def test_single_block_matches_libbz2():
    raw = b"The quick brown fox jumps over the lazy dog\n" * 100
    result = scan(bz2.compress(raw))

    assert result.streams == [(0, 9)]
    assert len(result.blocks) == len(result.stream_ends) == 1
    assert result.blocks[0].bit_offset == 32
    assert result.blocks[0].expected_crc == bz2_crc32(raw)
    assert result.blocks[0].randomized is False
    assert result.stream_ends[0].expected_stream_crc == result.blocks[0].expected_crc

def test_multiblock_combined_crc_matches_libbz2():
    raw = bytes(range(256)) * 1000
    result = scan(bz2.compress(raw, compresslevel=1))

    assert len(result.blocks) >= 2
    assert len(result.stream_ends) == 1
    assert combined_crc(result.blocks) == result.stream_ends[0].expected_stream_crc

def test_concatenated_stream_headers():
    first = bz2.compress(b"first stream")
    second = bz2.compress(b"second stream", compresslevel=3)
    result = scan(first + second)

    assert result.streams == [(0, 9), (len(first), 3)]
    assert len(result.blocks) == len(result.stream_ends) == 2

def test_rejects_non_bzip_input():
    with pytest.raises(ValueError, match="does not start with a bzip2"): scan(b"not bzip2")
