import bz2, gzip, io
from threading import Event, Thread

import pytest

import fbz

def patterned(size): return bytes((i * 37 + i // 251) & 255 for i in range(size))

@pytest.mark.parametrize("level", range(1, 10))
def test_decompress_matches_libbz2_at_every_level(level):
    plain = patterned(20_000)
    assert fbz.decompress(bz2.compress(plain, compresslevel=level), threads=2) == plain

def test_compress_stream_formats_and_options():
    plain = patterned(300_000)
    assert bz2.decompress(fbz.compress(plain, "bzip2", threads=2, level=3)) == plain
    assert gzip.decompress(fbz.compress(plain, "gzip", threads=2, level=6)) == plain
    assert fbz.compress(plain, "lz4", threads=2)[:4] == b"\x04\x22\x4d\x18"
    with pytest.raises(ValueError, match="format"): fbz.compress(plain, "zip")

def test_parallel_multiblock_and_concatenated_are_deterministic():
    first, second = patterned(350_000), patterned(75_000)
    compressed = bz2.compress(first, compresslevel=1) + bz2.compress(second, compresslevel=9)
    expected = first + second
    for threads in (1, 2, 4, 0): assert fbz.decompress(compressed, threads=threads) == expected

def test_bad_crc_raises_package_error():
    compressed = bytearray(bz2.compress(b"integrity matters"))
    compressed[-2] ^= 1
    with pytest.raises(fbz.BadBzip2File): fbz.decompress(bytes(compressed))

def test_seekable_file_and_persisted_index(tmp_path):
    plain = patterned(350_000)
    compressed = bz2.compress(plain, compresslevel=1)
    source = tmp_path / "data.bz2"
    index_path = tmp_path / "data.fbz2i"
    source.write_bytes(compressed)
    encoded = fbz.build_index(source, index_path, threads=2)

    with fbz.open(source, index=index_path) as handle:
        assert handle.size == len(plain)
        assert handle.seek(99_990) == 99_990
        assert handle.read(40) == plain[99_990:100_030]
        handle.seek(-17, io.SEEK_END)
        target = bytearray(20)
        assert handle.readinto(target) == 17
        assert target[:17] == plain[-17:]
        assert handle.index_bytes() == encoded
    assert handle.closed

def test_index_is_bound_to_source():
    first = bz2.compress(b"first")
    second = bz2.compress(b"other")
    index = fbz.build_index(first)
    with pytest.raises(ValueError, match="source identity mismatch"): fbz.open(second, index=index)

def test_buffered_reader_compatibility():
    plain = patterned(180_000)
    with io.BufferedReader(fbz.open(bz2.compress(plain, compresslevel=1))) as handle:
        assert handle.read(1234) == plain[:1234]
        handle.seek(100_000)
        assert handle.read() == plain[100_000:]

def test_native_decode_releases_gil():
    plain = patterned(2_000_000)
    compressed = bz2.compress(plain, compresslevel=1)
    started, stop = Event(), Event()
    counter = [0]

    def spin():
        started.set()
        while not stop.is_set(): counter[0] += 1

    thread = Thread(target=spin)
    thread.start()
    started.wait()
    before = counter[0]
    try: assert fbz.decompress(compressed, threads=2) == plain
    finally:
        stop.set()
        thread.join()
    assert counter[0] > before
