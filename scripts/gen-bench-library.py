#!/usr/bin/env python3
"""Generates a synthetic photo library for benchmarking the indexer.

Usage: gen-bench-library.py <output-dir> <file-count>

Creates tiny JPEGs (EXIF Make/Model/DateTimeOriginal, no pixel data) in folders of 100,
spread over 25 year directories.
"""
import os
import random
import struct
import sys


def exif_tiff(make, model, dt):
    def ascii(s):
        return s.encode() + b"\0"

    make, model, dt = ascii(make), ascii(model), ascii(dt)
    ifd_len = lambda n: 2 + n * 12 + 4
    ifd0 = 8
    exif = ifd0 + ifd_len(3)
    data_at = exif + ifd_len(1)
    out = bytearray(b"II*\0" + struct.pack("<I", ifd0))
    data = bytearray()

    def entry(tag, typ, value):
        nonlocal data_at
        out.extend(struct.pack("<HHII", tag, typ, len(value), data_at))
        data.extend(value)
        data_at += len(value)

    out.extend(struct.pack("<H", 3))
    entry(0x010F, 2, make)
    entry(0x0110, 2, model)
    out.extend(struct.pack("<HHII", 0x8769, 4, 1, exif))
    out.extend(struct.pack("<I", 0))
    out.extend(struct.pack("<H", 1))
    entry(0x9003, 2, dt)
    out.extend(struct.pack("<I", 0))
    return bytes(out + data)


def jpeg(dt):
    app1 = b"Exif\0\0" + exif_tiff("PixelCam", "Model 1", dt)
    sof0 = bytes([0xFF, 0xC0, 0, 11, 8, 0, 3, 0, 4, 1, 1, 0x11, 0])
    return b"\xff\xd8\xff\xe1" + struct.pack(">H", len(app1) + 2) + app1 + sof0 + b"\xff\xd9"


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    root, count = sys.argv[1], int(sys.argv[2])
    per_folder = 100
    random.seed(1)
    for i in range(count):
        album = i // per_folder
        year = 2000 + album % 25
        folder = os.path.join(root, str(year), f"album-{album:04d}")
        if i % per_folder == 0:
            os.makedirs(folder, exist_ok=True)
        dt = f"{year}:{random.randint(1, 12):02d}:{random.randint(1, 28):02d} 12:{i % 60:02d}:00"
        with open(os.path.join(folder, f"IMG_{i:06d}.jpg"), "wb") as f:
            f.write(jpeg(dt))


if __name__ == "__main__":
    main()
