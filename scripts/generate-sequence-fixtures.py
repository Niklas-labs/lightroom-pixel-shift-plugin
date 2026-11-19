#!/usr/bin/env python3
"""Tiny native Sony TIFF/EXIF headers with no sensor pixels, for discovery tests."""
import json
from pathlib import Path
import struct
import sys


def capture(group, shot, count, iso=100, model='ILCE-7CR', ordinary=False):
    data = bytearray(512)
    def u16(offset, value): struct.pack_into('<H', data, offset, value)
    def u32(offset, value): struct.pack_into('<I', data, offset, value)
    def entry(offset, tag, kind, count, value):
        struct.pack_into('<HHI', data, offset, tag, kind, count)
        (u16 if kind == 3 and count == 1 else u32)(offset+8, value)
    data[:2] = b'II'; u16(2, 42); u32(4, 8); u16(8, 5)
    entry(10, 0x010f, 2, 5, 80)
    encoded = model.encode() + b'\0'
    entry(22, 0x0110, 2, len(encoded), 96)
    entry(34, 0x0100, 4, 1, 9504); entry(46, 0x0101, 4, 1, 6336)
    entry(58, 0x8769, 4, 1, 128)
    data[80:85] = b'SONY\0'; data[96:96+len(encoded)] = encoded
    u16(128, 5)
    entry(130, 0x829a, 5, 1, 256); entry(142, 0x829d, 5, 1, 264)
    entry(154, 0x8827, 3, 1, iso); entry(166, 0x927c, 7, 54, 320)
    entry(178, 0xa431, 2, 9, 288)
    u32(256, 1); u32(260, 100); u32(264, 8); u32(268, 1)
    data[288:297] = b'camera-x\0'
    data[320:332] = b'SONY DSC \0\0\0'; u16(332, 2)
    entry(334, 0x0001 if ordinary else 0x202f, 7, 6, 400)
    entry(346, 0xb04a, 3, 1, shot)
    u32(400, group); data[404:406] = bytes([shot, count])
    return data


def generate(root):
    folder = root / 'captures'
    folder.mkdir(parents=True, exist_ok=True)
    paths = []
    for group, count in [(100,4), (200,16), (300,4), (400,4), (500,4), (600,4), (700,4)]:
        shots = [1] if group == 500 else list(range(1,count+1))
        if group == 300: shots.pop()
        if group == 400: shots.append(2)
        for ordinal, shot in enumerate(shots):
            path = folder / f'{group}-{ordinal}.ARW'
            path.write_bytes(capture(group,shot,count,iso=200 if group==600 and shot==2 else 100,
                                     model='UNKNOWN1' if group==700 else 'ILCE-7CR',ordinary=group==500))
            paths.append(str(path.relative_to(root)))
    broken = folder / 'broken.ARW'; broken.write_bytes(b'not a raw')
    paths.append(str(broken.relative_to(root)))
    request = root / 'sequence-request.json'
    request.write_text(json.dumps({'version':1,'inputs':list(reversed(paths))}),encoding='utf-8')
    return request


if __name__ == '__main__':
    print(generate(Path(sys.argv[1])))
