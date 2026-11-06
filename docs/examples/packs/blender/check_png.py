"""Check the Blender pack test's output: a PNG of the expected size. Usage: python check_png.py OUT.png 160 120"""
import struct
import sys

path, width, height = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
data = open(path, "rb").read()
assert data[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG"
got = struct.unpack(">II", data[16:24])
assert got == (width, height), f"size {got}, expected {(width, height)}"
assert len(data) > 1000, f"only {len(data)} bytes: probably blank"
print("ok", got, len(data), "bytes")
