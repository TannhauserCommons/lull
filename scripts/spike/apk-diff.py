"""apk-diff.py A B: where two APKs differ, and how big the APK Signing Block of each is (what it holds, by pair id)."""
import struct
import sys


def layout(path):
    data = open(path, "rb").read()
    eocd = data.rfind(b"PK\x05\x06")
    cd = struct.unpack("<I", data[eocd + 16 : eocd + 20])[0]
    block = None
    if data[cd - 16 : cd] == b"APK Sig Block 42":
        size = struct.unpack("<Q", data[cd - 24 : cd - 16])[0]
        start = cd - size - 8
        pairs, pos = [], start + 8
        while pos < cd - 24:
            length = struct.unpack("<Q", data[pos : pos + 8])[0]
            pairs.append((hex(struct.unpack("<I", data[pos + 8 : pos + 12])[0]), length - 4))
            pos += 8 + length
        block = (start, cd, pairs)
    return data, cd, eocd, block


def region(lay, pos):
    _, cd, eocd, block = lay
    if block and block[0] <= pos < block[1]:
        return "signing block"
    if pos < (block[0] if block else cd):
        return "entries"
    return "central directory" if pos < eocd else "EOCD"


a, b = layout(sys.argv[1]), layout(sys.argv[2])
for name, lay in (("first", a), ("second", b)):
    data, cd, eocd, block = lay
    where = f"signing block {block[1] - block[0]} bytes at {block[0]}, pairs (id, value bytes) {block[2]}" if block else "no signing block"
    print(f"{name}: {len(data)} bytes, central directory at {cd}, {where}")
diff = [i for i, (x, y) in enumerate(zip(a[0], b[0])) if x != y]
print(f"byte positions that differ: {len(diff)} (sizes {len(a[0])} and {len(b[0])})")
print("first 10 (0-based offset: region in the first file): " + "; ".join(f"{i}: {region(a, i)}" for i in diff[:10]))
print("regions of all differing positions: " + ", ".join(sorted({region(a, i) for i in diff})))
