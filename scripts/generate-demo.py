#!/usr/bin/env python3
"""Generate a small independent chronological 16-shot Sony sensor fixture."""
import json
import math
import pathlib
import sys
import struct

root = pathlib.Path(sys.argv[1])
root.mkdir(parents=True, exist_ok=True)
# Metadata-only TIFF: real native Sony EXIF/MakerNote parser smoke, no RAW pixels.
metadata = bytearray(256)
def u16(offset, value): struct.pack_into('<H', metadata, offset, value)
def u32(offset, value): struct.pack_into('<I', metadata, offset, value)
def entry(offset, tag, kind, count, value):
    struct.pack_into('<HHI', metadata, offset, tag, kind, count)
    (u16 if kind == 3 and count == 1 else u32)(offset + 8, value)
metadata[:2] = b'II'
u16(2, 42); u32(4, 8); u16(8, 2)
entry(10, 0x010f, 2, 5, 40); entry(22, 0x8769, 4, 1, 64)
metadata[40:45] = b'SONY\0'
u16(64, 5)
entry(66, 0x829a, 5, 1, 144); entry(78, 0x829d, 5, 1, 152)
entry(90, 0x8827, 3, 1, 100); entry(102, 0x927c, 7, 54, 192)
entry(114, 0xa431, 2, 9, 160)
u32(144, 1); u32(148, 100); u32(152, 8); u32(156, 1)
metadata[160:169] = b'camera-x\0'
metadata[192:204] = b'SONY DSC \0\0\0'; u16(204, 2)
entry(206, 0x202f, 7, 6, 240); entry(218, 0xb04a, 3, 1, 10)
u32(240, 42); metadata[244:246] = bytes([3, 16])
(root / 'capture-metadata.ARW').write_bytes(metadata)
inputs = []
for index in range(16):
    x, y = [(1, 0), (1, 1), (0, 1), (0, 0)][index % 4]
    hx, hy = [(0.5, 0), (0, 0), (0.5, 0.5), (0, 0.5)][index // 4]
    pixels = []
    for py in range(48):
        for px in range(64):
            channel = [0, 1, 1, 2][(py % 2) * 2 + px % 2]
            pixels.append(0.3 + 0.08 * channel + 0.1 * math.sin((px+x+hx)*1.7)
                          + 0.05 * math.cos((py+y+hy)*1.1))
    filename = f"frame-{index+1:02}.json"
    inputs.append(filename)
    (root / filename).write_text(json.dumps({
        "frame": {"width": 64, "height": 48, "pixels": pixels, "cfa": [0, 1, 1, 2],
                  "model": "ILCE-7CR", "sequence": index+1},
        "color": {"white_balance": [1, 1, 1],
                  "xyz_to_camera": [0.82, -0.2976, -0.0719, -0.4296, 1.2053, 0.2532,
                                    -0.0429, 0.1282, 0.5774]}}))
(root / "job.json").write_text(json.dumps({"version": 1, "camera": "auto", "mode": "auto",
    "inputs": inputs, "output": "demo.tif", "alignment_radius": 0, "motion_threshold": 0}))
print(root / "job.json")
