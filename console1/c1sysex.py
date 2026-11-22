"""Console 1 SysEx (Softube mode) framing: F0 7D <cmd> 00 00 00 <len> 00 <data...> <crc5> F7.
crc5 = zlib.crc32 of everything from F0 up to the data's end, as 4 big-endian bytes' low 7 bits
followed by one byte holding their top bits (byte 0's top bit in bit 3 ... byte 3's in bit 0)."""
import zlib

def crc5(b: bytes) -> bytes:
    c = zlib.crc32(b) & 0xFFFFFFFF
    bs = c.to_bytes(4, "big")
    return bytes([x & 0x7F for x in bs] + [sum(((x >> 7) & 1) << (3 - i) for i, x in enumerate(bs))])

def frame(cmd: int, data: bytes) -> bytes:
    head = bytes([0xF0, 0x7D, cmd, 0, 0, 0, len(data) & 0x7F, 0]) + bytes(data)
    return head + crc5(head) + b"\xF7"

def parse(raw: bytes):
    """-> (cmd, data) or None if the check fails."""
    if len(raw) < 15 or raw[0] != 0xF0 or raw[1] != 0x7D:
        return None
    n = raw[6]
    data = raw[8:8 + n]
    if raw[8 + n:8 + n + 5] != crc5(raw[:8 + n]):
        return None
    return raw[2], data
