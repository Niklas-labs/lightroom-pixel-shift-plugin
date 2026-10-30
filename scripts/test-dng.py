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
                assert int(raw.compression) == 8
                assert raw.shape == (40*scale, 56*scale, 3)
                assert raw.tags['DNGVersion'].value == bytes([1, 4, 0, 0])
                assert raw.tags['DNGBackwardVersion'].value == bytes([1, 4, 0, 0])
                assert 'ICCProfile' not in raw.tags
                assert 'CFARepeatPatternDim' not in raw.tags
                np.testing.assert_allclose(raw.asarray(), np.broadcast_to(expected, raw.shape), rtol=0, atol=1e-6)
                assert raw.tags['CalibrationIlluminant1'].value == 17
                assert raw.tags['CalibrationIlluminant2'].value == 21
                for tag, matrix in [('ColorMatrix1', frame['color']['xyz_to_camera_a']), ('ColorMatrix2', frame['color']['xyz_to_camera'])]:
                    values = raw.tags[tag].value
                    np.testing.assert_allclose([values[i]/values[i+1] for i in range(0,18,2)],matrix,rtol=0,atol=1e-6)
                assert raw.tags['BaselineExposure'].value == (0,1000000)
                neutral = raw.tags['AsShotNeutral'].value
                np.testing.assert_allclose([neutral[i]/neutral[i+1] for i in range(0,6,2)], [0.5,1,2/3], atol=1e-6)
                assert preview.shape[-1] == 3 and max(preview.shape[:2]) <= 512
                assert preview.dtype == np.dtype('uint8') and preview.asarray().size > 0
            with rawpy.imread(str(result)) as raw:
                assert raw.sizes.width == 56*scale and raw.sizes.height == 40*scale
                assert raw.raw_image.shape == (40*scale, 56*scale, 3)
                assert raw.postprocess().size > 0
            print(f'{mode}: independent linear DNG sample/calibration/preview/development checks passed')


        for mode in ('four','sixteen','denoise'):
            with tifffile.TiffFile(root / f'{mode}.dng') as initial:
                expected_pixels=initial.pages[0].asarray().view(np.uint32).copy()
                expected_preview=initial.pages[0].pages[0].asarray().copy()
            for reference,compression,rows in [(True,'lossless',17),(False,'none',1),(False,'lossless',256)]:
                spec=dict(job,mode=mode,inputs=job['inputs'] if mode=='sixteen' else job['inputs'][:4],
                    output=f'{mode}-{reference}-{compression}-{rows}.dng',compression=compression,strip_rows=rows)
                path=root / 'variant-job.json'
                path.write_text(json.dumps(spec))
                command=[str(engine),'merge','--fixtures','--job',str(path)]
                if reference: command.append('--reference')
                run=subprocess.run(command,check=True,capture_output=True,text=True)
                report=json.loads(run.stdout)
                assert report['execution']['streamed_output'] == (not reference)
                assert report['execution']['compression'] == compression
                assert report['execution']['peak_output_region_bytes'] <= expected_pixels.shape[1]*min(rows,expected_pixels.shape[0])*12
                assert report['execution']['resources']['peak_resident_bytes'] > 0
                with tifffile.TiffFile(root / spec['output']) as variant:
                    assert int(variant.pages[0].compression) == (8 if compression=='lossless' else 1)
                    np.testing.assert_array_equal(variant.pages[0].asarray().view(np.uint32),expected_pixels)
                    np.testing.assert_array_equal(variant.pages[0].pages[0].asarray(),expected_preview)
            print(f'{mode}: streaming/reference, compression, strip-size and preview bitwise parity passed')

        for name in job['inputs']:
            path = root / name
            frame = json.loads(path.read_text())
            frame['source_crop'] = dict(x=8, y=8, width=48, height=32)
            path.write_text(json.dumps(frame))
        for mode, origin in [('four',(5,4)), ('sixteen',(11,8)), ('denoise',(4,4))]:
            spec = dict(job, mode=mode, output=f'cropped-{mode}.dng', inputs=job['inputs'] if mode == 'sixteen' else job['inputs'][:4])
            path = root / f'cropped-{mode}-job.json'
            path.write_text(json.dumps(spec))
            subprocess.run([str(engine), 'merge', '--fixtures', '--job', str(path)], check=True, capture_output=True)
            scale = 2 if mode == 'sixteen' else 1
            with tifffile.TiffFile(root / spec['output']) as dng:
                raw = dng.pages[0]
                assert raw.tags['DefaultCropOrigin'].value == origin
                assert raw.tags['DefaultCropSize'].value == (48*scale,32*scale)
                assert raw.pages[0].shape == (32*scale,48*scale,3)
                np.testing.assert_allclose(raw.asarray(),np.broadcast_to(expected,raw.shape),rtol=0,atol=1e-6)
            print(f'{mode}: source crop mapping and cropped preview checks passed')

        # A decoder without illuminant A must retain valid single-D65 output.
        for name in job['inputs']:
            path = root / name
            frame = json.loads(path.read_text())
            frame['color']['xyz_to_camera_a'] = None
            path.write_text(json.dumps(frame))
        spec = dict(job, mode='four', output='single-d65.dng', inputs=job['inputs'][:4])
        path = root / 'single-job.json'
        path.write_text(json.dumps(spec))
        subprocess.run([str(engine), 'merge', '--fixtures', '--job', str(path)],check=True,capture_output=True)
        with tifffile.TiffFile(root / spec['output']) as dng:
            raw = dng.pages[0]
            assert raw.tags['CalibrationIlluminant1'].value == 21
            assert 'ColorMatrix2' not in raw.tags and 'CalibrationIlluminant2' not in raw.tags
            np.testing.assert_allclose(raw.asarray(),np.broadcast_to(expected,raw.shape),rtol=0,atol=1e-6)
        print('single-D65 fallback: calibration and unmodified sample checks passed')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', type=Path, required=True)
    check(parser.parse_args().engine.resolve())
