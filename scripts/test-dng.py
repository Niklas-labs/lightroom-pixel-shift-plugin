#!/usr/bin/env python3
"""Cross-check CLI linear DNGs with independent tifffile and LibRaw readers."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile

import numpy as np
import rawpy
import tifffile

ROOT = Path(__file__).resolve().parents[1]


def check(engine):
    with tempfile.TemporaryDirectory(prefix='pixelshift dng ') as temporary:
        root = Path(temporary)
        subprocess.run([sys.executable, str(ROOT / 'scripts/generate-demo.py'), str(root)], check=True, capture_output=True)
        job = json.loads((root / 'job.json').read_text())
        # Constant sensor-color fields prove that no WB, gamma or display matrix
        # has been baked into the main camera RGB, including noise/headroom.
        expected = np.array([-0.01, 0.18, 1.4], dtype=np.float32)
        for name in job['inputs']:
            path = root / name
            frame = json.loads(path.read_text())
            width, height = frame['frame']['width'], frame['frame']['height']
            frame['frame']['pixels'] = [float(expected[[0, 1, 1, 2][(y % 2)*2+x % 2]]) for y in range(height) for x in range(width)]
            frame['color']['white_balance'] = [2, 1, 1.5]
            path.write_text(json.dumps(frame))
        for mode in ('four', 'sixteen', 'denoise'):
            spec = dict(job, mode=mode, output=f'{mode}.dng', inputs=job['inputs'] if mode == 'sixteen' else job['inputs'][:4])
            path = root / f'{mode}-job.json'
            path.write_text(json.dumps(spec))
            subprocess.run([str(engine), 'merge', '--fixtures', '--job', str(path)], check=True, capture_output=True)
            result = root / spec['output']
            scale = 2 if mode == 'sixteen' else 1
            with tifffile.TiffFile(result) as dng:
                assert len(dng.pages) == 1
                raw = dng.pages[0]
                preview = raw.pages[0]
                assert int(raw.photometric) == 34892
                assert raw.dtype == np.dtype('float32')
                assert raw.shape == (40*scale, 56*scale, 3)
                assert raw.tags['DNGVersion'].value == bytes([1, 4, 0, 0])
                assert raw.tags['DNGBackwardVersion'].value == bytes([1, 4, 0, 0])
                assert 'ICCProfile' not in raw.tags
                assert 'CFARepeatPatternDim' not in raw.tags
                np.testing.assert_allclose(raw.asarray(), np.broadcast_to(expected, raw.shape), rtol=0, atol=1e-6)
                neutral = raw.tags['AsShotNeutral'].value
                np.testing.assert_allclose([neutral[i]/neutral[i+1] for i in range(0,6,2)], [0.5,1,2/3], atol=1e-6)
                assert preview.shape[-1] == 3 and max(preview.shape[:2]) <= 512
                assert preview.dtype == np.dtype('uint8') and preview.asarray().size > 0
            with rawpy.imread(str(result)) as raw:
                assert raw.sizes.width == 56*scale and raw.sizes.height == 40*scale
                assert raw.raw_image.shape == (40*scale, 56*scale, 3)
                assert raw.postprocess().size > 0
            print(f'{mode}: independent linear DNG sample/calibration/preview/development checks passed')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', type=Path, required=True)
    check(parser.parse_args().engine.resolve())
