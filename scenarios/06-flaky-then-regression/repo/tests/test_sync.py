def checksum(buf: bytes) -> int:
    return sum(buf) & 0xFFFF


def test_sync():
    # Seeded regression: producer writes 3 bytes, consumer expects 4.
    payload = b"abc"
    assert checksum(payload) == checksum(b"abcd")
