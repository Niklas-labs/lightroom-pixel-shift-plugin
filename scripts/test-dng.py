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
                np.testing.assert_allclose(raw.tags['BaselineExposure'].value[0]/raw.tags['BaselineExposure'].value[1], .35, atol=1e-6)
                assert raw.tags['CameraCalibrationSignature'].value == 'com.adobe'
                assert raw.tags['ProfileCalibrationSignature'].value == 'com.adobe'
                for name, matrix in [('CameraCalibration1', [.9487,0,0,0,1,0,0,0,1.0363]), ('CameraCalibration2', [.9487,0,0,0,1,0,0,0,1.0363]),
                                     ('ForwardMatrix1', [.4616,.3771,.1256,.2079,.7562,.0359,.0798,.0011,.7442]),
                                     ('ForwardMatrix2', [.4677,.3226,.1740,.2670,.6792,.0539,.1207,.0037,.7007])]:
                    entries = raw.tags[name].value
                    np.testing.assert_allclose([entries[i]/entries[i+1] for i in range(0,18,2)], matrix, rtol=0, atol=1e-6)
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
            for reference,compression,rows,storage in [(True,'lossless',17,'ram'),(False,'none',1,'ram'),(False,'lossless',256,'ram'),(False,'lossless',17,'disk')]:
                spec=dict(job,mode=mode,inputs=job['inputs'] if mode=='sixteen' else job['inputs'][:4],
                    output=f'{mode}-{reference}-{compression}-{rows}-{storage}.dng',compression=compression,strip_rows=rows,source_storage=storage,scratch_directory=str(root))
                path=root / 'variant-job.json'
                path.write_text(json.dumps(spec))
                command=[str(engine),'merge','--fixtures','--job',str(path)]
                if reference: command.append('--reference')
                run=subprocess.run(command,check=True,capture_output=True,text=True)
                report=json.loads(run.stdout)
                assert report['execution']['streamed_output'] == (not reference)
                assert report['execution']['compression'] == compression
                assert report['execution']['source_storage'] == ('normalized_f32_ram' if reference else f'float32_{storage}')
                if storage == 'disk':
                    assert report['execution']['source_buffer_bytes'] == 0
                    assert report['execution']['scratch_bytes'] > 0
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
            assert 'CameraCalibration2' not in raw.tags and 'ForwardMatrix2' not in raw.tags
            entries = raw.tags['ForwardMatrix1'].value
            np.testing.assert_allclose([entries[i]/entries[i+1] for i in range(0,18,2)], [.4677,.3226,.1740,.2670,.6792,.0539,.1207,.0037,.7007], atol=1e-6)
            np.testing.assert_allclose(raw.asarray(),np.broadcast_to(expected,raw.shape),rtol=0,atol=1e-6)
        print('single-D65 fallback: calibration and unmodified sample checks passed')
        check_gradient_photometry(engine, root, job)
        check_calibrated_preview(engine, root, job)
        check_merge_diagnostics(engine, root, job)


def check_calibrated_preview(engine, root, job):
    # A camera-neutral field must become neutral sRGB with the tagged exposure
    # gain, while the camera-RGB main image retains its exact unbalanced values.
    neutral = np.array([.09,.18,.12],dtype=np.float32)
    for dual in (False,True):
        for name in job['inputs'][:4]:
            path=root/name
            frame=json.loads(path.read_text())
            frame.pop('source_crop',None)
            w,h=frame['frame']['width'],frame['frame']['height']
            frame['frame']['pixels']=[float(neutral[[0,1,1,2][(y%2)*2+x%2]]) for y in range(h) for x in range(w)]
            frame['color']['white_balance']=[2,1,1.5]
            frame['color']['xyz_to_camera_a']=[.9185,-.4857,.0505,-.3651,1.1061,.2982,-.0161,.0698,.6769] if dual else None
            path.write_text(json.dumps(frame))
        spec=dict(job,mode='four',inputs=job['inputs'][:4],output=f'preview-neutral-{dual}.dng')
        path=root/'preview-job.json'
        path.write_text(json.dumps(spec))
        subprocess.run([str(engine),'merge','--fixtures','--job',str(path)],check=True,capture_output=True)
        with tifffile.TiffFile(root/spec['output']) as dng:
            raw=dng.pages[0]
            np.testing.assert_array_equal(raw.asarray(),np.broadcast_to(neutral,raw.shape))
            preview=raw.pages[0]
            assert preview.tags['PreviewApplicationName'].value=='Sony Pixel Shift'
            assert preview.tags['PreviewSettingsName'].value=='Calibrated As Shot v1'
            level=.18*2**.35
            expected=round((1.055*level**(1/2.4)-.055)*255)
            assert np.max(np.abs(preview.asarray().astype(int)-expected))<=1
            assert raw.tags['BaselineExposure'].value==(350000,1000000)
    print('Calibrated single/dual preview: neutral/exposure match with exact untouched RAW samples')


def check_gradient_photometry(engine, root, job):
    """Independent spatial oracle through CLI + lossless DNG, not the preview."""
    base = np.array([0.015625, 0.1875, 0.75], dtype=np.float32)
    dx = np.array([1, 2, 3], dtype=np.float32) / 2048
    dy = np.array([3, 2, 1], dtype=np.float32) / 4096
    worst_error = worst_ev = 0.0
    for mode in ('four', 'sixteen', 'denoise'):
        names = job['inputs'] if mode == 'sixteen' else job['inputs'][:4]
        for index, name in enumerate(names):
            path = root / name
            frame = json.loads(path.read_text())
            frame.pop('source_crop', None)
            frame['color']['white_balance'] = [2, 1, 1.5]
            width, height = frame['frame']['width'], frame['frame']['height']
            sx, sy = [(1, 0), (1, 1), (0, 1), (0, 0)][index % 4]
            hx, hy = [(0.5, 0), (0, 0), (0.5, 0.5), (0, 0.5)][index // 4] if mode == 'sixteen' else (0, 0)
            sx, sy = (0, 0) if mode == 'denoise' else (sx + hx, sy + hy)
            frame['frame']['pixels'] = [
                float(base[c] + dx[c]*(x+sx) + dy[c]*(y+sy))
                for y in range(height) for x in range(width)
                for c in [[0, 1, 1, 2][(y % 2)*2+x % 2]]
            ]
            path.write_text(json.dumps(frame))
        scale = 2 if mode == 'sixteen' else 1
        y, x = np.mgrid[:(height-8)*scale, :(width-8)*scale]
        expected = base + (4+x[..., None]/scale)*dx + (4+y[..., None]/scale)*dy
        for storage in ('ram', 'disk'):
            for compression in ('none', 'lossless'):
                spec = dict(job, mode=mode, inputs=names,
                            output=f'gradient-{mode}-{storage}-{compression}.dng',
                            source_storage=storage, compression=compression,
                            strip_rows=7, motion_threshold=0.08, alignment_radius=0)
                path = root / 'gradient-job.json'
                path.write_text(json.dumps(spec))
                subprocess.run([str(engine), 'merge', '--fixtures', '--job', str(path)], check=True, capture_output=True)
                with tifffile.TiffFile(root / spec['output']) as dng:
                    actual = dng.pages[0].asarray()
                    np.testing.assert_allclose(actual, expected, rtol=0, atol=1e-6)
                    error = float(np.max(np.abs(actual-expected)))
                    ev = np.log2(actual.astype(np.float64).mean(axis=(0, 1))/expected.mean(axis=(0, 1)))
                    assert np.max(np.abs(ev)) < 1e-6, ev
                    worst_error = max(worst_error, error)
                    worst_ev = max(worst_ev, float(np.max(np.abs(ev))))
    print(f'12 gradient photometry cases: max sample error {worst_error:.9g}, max mean channel bias {worst_ev:.9g} EV')


def check_merge_diagnostics(engine, root, job):
    # Independent readers verify the diagnostic TIFF payload and that enabling
    # the wrapper changes neither the primary samples nor its embedded preview.
    color = json.loads((root / job['inputs'][0]).read_text())['color']
    color['orientation'] = 6
    for mode in ('four', 'sixteen', 'denoise'):
        names = job['inputs'] if mode == 'sixteen' else job['inputs'][:4]
        for index, name in enumerate(names):
            path = root / name
            frame = json.loads(path.read_text())
            width, height = frame['frame']['width'], frame['frame']['height']
            frame['frame']['pixels'] = [0.3] * (width * height)
            if index == 1:
                for y in range(15, 25):
                    for x in range(20, 30):
                        frame['frame']['pixels'][y*width+x] = 0.9
            frame['color'] = color
            frame.pop('source_crop', None)
            path.write_text(json.dumps(frame))
        primary = None
        for enabled in (False, True):
            spec = dict(job, mode=mode, inputs=names, output=f'diagnostics-{mode}-{enabled}.dng',
                        strip_rows=7, alignment_radius=0, motion_threshold=0.08,
                        save_diagnostics=enabled)
            path = root / 'diagnostics-job.json'
            path.write_text(json.dumps(spec))
            run = subprocess.run([str(engine), 'merge', '--fixtures', '--job', str(path)],
                                 check=True, capture_output=True)
            report = json.loads(run.stdout)
            data = (root / spec['output']).read_bytes()
            if not enabled:
                primary = data
                continue
            assert data == primary
            paths = report['merge_diagnostics']
            summary = json.loads(Path(paths['summary']).read_text())
            with tifffile.TiffFile(paths['mask']) as mask_file:
                mask = mask_file.asarray()
                assert mask.dtype == np.uint8
                assert set(np.unique(mask)) == {0, 255}
                assert int(mask_file.pages[0].tags['Orientation'].value) == 6
                assert int(mask_file.pages[0].compression) == 8
            with tifffile.TiffFile(paths['overlay']) as overlay_file:
                overlay = overlay_file.asarray()
                assert overlay.shape == (*mask.shape, 3)  # fixture fits in the preview
                assert int(overlay_file.pages[0].tags['Orientation'].value) == 6
                selected = overlay[mask != 0]
                assert np.all(selected[:, 0] > selected[:, 1])
                assert np.all(selected[:, 0] > selected[:, 2])
            assert summary['fallback_pixels'] == np.count_nonzero(mask)
            assert report['report']['fallback_pixels'] == summary['fallback_pixels']
            assert summary['total_pixels'] == mask.size
        print(f'{mode}: independent diagnostic mask/overlay and exact DNG parity passed')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', type=Path, required=True)
    check(parser.parse_args().engine.resolve())
