#!/usr/bin/env python3
"""Generate a small independent chronological 16-shot Sony sensor fixture."""
import json
import math
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
root.mkdir(parents=True, exist_ok=True)
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
(root / "job.json").write_text(json.dumps({"version": 1, "camera": "ILCE-7CR", "mode": "sixteen",
    "inputs": inputs, "output": "demo.tif", "alignment_radius": 0, "motion_threshold": 0}))
print(root / "job.json")
