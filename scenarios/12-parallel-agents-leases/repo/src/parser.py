def crc16(data: bytes) -> int:
    # stub added by worker-1 before crash
    return sum(data) & 0xFFFF


def parse_frame(buf: bytes):
    # WIP: length-prefix incomplete
    if len(buf) < 2:
        raise ValueError("short frame")
    return {"crc": crc16(buf), "payload": buf[2:]}
