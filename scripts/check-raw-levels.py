#!/usr/bin/env python3
"""Compare production Rust ARW samples against an Adobe-converted mosaic DNG.

No RAW development, white balance, baseline exposure, or display profiles are
applied. Decodes one file at a time; only a small summary is shared.
"""
import argparse
from contextlib import contextmanager
import json
from pathlib import Path
import subprocess

import numpy as np
import rawpy
import tifffile


def numbers(tag):
    values = np.frombuffer(tag.value, dtype=np.uint8) if isinstance(tag.value, bytes) else np.asarray(tag.value).reshape(-1)
    if int(tag.dtype) in (5, 10):
        pairs = values.reshape(-1, 2)
        return pairs[:, 0].astype(float)/pairs[:, 1]
    return values.astype(float)


@contextmanager
def reference_sensor(path):
    with tifffile.TiffFile(path) as tif:
        root = tif.pages[0]
        if 'DNGVersion' not in root.tags:
            raise ValueError('Reference must be the Adobe-converted DNG from the SAME original ARW')
        def pages(directories, depth=0):
            for page in directories:
                yield page
                if depth < 4 and page.pages is not None:
                    yield from pages(page.pages, depth+1)
        page = next((p for p in pages(tif.pages) if int(p.photometric) == 32803), None)
        if page is None:
            raise ValueError('Reference must be a mosaic DNG converted from one ARW, not a merged/linear DNG')
        def tag(name):
            return page.tags.get(name) or root.tags.get(name)
        for name in ('BlackLevelDeltaH', 'BlackLevelDeltaV'):
            if tag(name) is not None:
                raise ValueError(f'Reference has {name}; this diagnostic does not yet support variable black levels')
        active = numbers(tag('ActiveArea')).astype(int) if tag('ActiveArea') else np.array([0, 0, page.imagelength, page.imagewidth])
        origin = numbers(tag('DefaultCropOrigin')) if tag('DefaultCropOrigin') else np.array([0, 0])
        size = numbers(tag('DefaultCropSize')) if tag('DefaultCropSize') else np.array([active[3]-active[1], active[2]-active[0]])
        if not (np.equal(origin, np.floor(origin)).all() and np.equal(size, np.floor(size)).all()):
            raise ValueError('Fractional reference crop is not supported by the sensor diagnostic')
        if tuple(numbers(tag('CFARepeatPatternDim')).astype(int)) != (2, 2):
            raise ValueError('Expected 2x2 Bayer reference')
        pattern = numbers(tag('CFAPattern')).astype(int).reshape(2, 2)
        planes = numbers(tag('CFAPlaneColor')).astype(int) if tag('CFAPlaneColor') else np.array([0, 1, 2])
        pattern = planes[pattern]
        if sorted(pattern.ravel()) != [0, 1, 1, 2]:
            raise ValueError('Expected RGB Bayer reference')
        repeat = tuple(numbers(tag('BlackLevelRepeatDim')).astype(int)) if tag('BlackLevelRepeatDim') else (1, 1)
        if repeat not in ((1, 1), (2, 2)):
            raise ValueError('Unsupported reference black-level repeat dimensions')
        black = numbers(tag('BlackLevel')).reshape(repeat) if tag('BlackLevel') else np.zeros(repeat)
        white = numbers(tag('WhiteLevel'))
        if len(white) != 1 or not np.all(white[0] > black):
            raise ValueError('Invalid or unsupported reference white level')
        if tag('LinearizationTable') is not None:
            raise ValueError('Reference has a LinearizationTable; this diagnostic cannot yet verify whether LibRaw already applied it')
        linearization = None
        original_name = str(tag('OriginalRawFileName').value).replace('\\', '/').rsplit('/', 1)[-1] if tag('OriginalRawFileName') else None
        shape = (page.imagelength, page.imagewidth)
        crop = dict(x=int(active[1]+origin[0]), y=int(active[0]+origin[1]), width=int(size[0]), height=int(size[1]))
        if crop['x'] < 0 or crop['y'] < 0 or crop['x']+crop['width'] > shape[1] or crop['y']+crop['height'] > shape[0]:
            raise ValueError('Reference crop outside sensor')
    with rawpy.imread(str(path)) as raw:
        codes = raw.raw_image
        if codes.shape != shape or codes.ndim != 2 or codes.dtype != np.uint16:
            raise ValueError(f'Independent decoder sensor shape/type differs from DNG tags: {codes.shape} vs {shape}')
        yield codes, dict(crop=crop, active_origin=[int(active[1]), int(active[0])],
                          black=black, white=float(white[0]), cfa=pattern, linearization=linearization,
                          dimensions=[shape[1], shape[0]], original_name=original_name)


def fit(x, y):
    """y = gain*x + offset; used for registration without assuming equal scale."""
    if len(x) < 16 or np.ptp(x) < .005:
        raise ValueError('Insufficient textured samples for a reliable sensor comparison')
    design = np.column_stack((x, np.ones_like(x)))
    gain, offset = np.linalg.lstsq(design, y, rcond=None)[0]
    residual = y-(gain*x+offset)
    return dict(gain=float(gain), offset=float(offset),
                correlation=float(np.corrcoef(x, y)[0, 1]),
                residual_p90=float(np.percentile(np.abs(residual), 90)),
                score=float(np.median(np.abs(residual))))


def summarize_ev(values):
    if not len(values):
        return None
    return dict(count=len(values), median=float(np.median(values)),
                p10=float(np.percentile(values, 10)), p90=float(np.percentile(values, 90)))


def compare(sensor, codes, reference, radius=8):
    crop = sensor['crop']
    if (crop['width'], crop['height']) != (reference['crop']['width'], reference['crop']['height']):
        raise ValueError('Visible crops differ in size; refusing to resample or guess geometry')
    samples = sensor['samples']
    x, y, phase, channel, native, normalized = [np.array([s[key] for s in samples])
                                               for key in ('x', 'y', 'phase', 'channel', 'code', 'normalized')]
    black, white = np.array(sensor['black']), np.array(sensor['white'])
    formula_error = float(np.max(np.abs(normalized-(native-black[phase])/(white[phase]-black[phase]))))
    if formula_error > 2e-6:
        raise ValueError('Production normalization disagrees with its reported levels')
    # Use the same interior samples for every offset in the registration search.
    keep = (x >= radius) & (y >= radius) & (x < crop['width']-radius) & (y < crop['height']-radius)
    x, y, phase, channel, native, normalized = [a[keep] for a in (x, y, phase, channel, native, normalized)]
    if len(x) < 128:
        raise ValueError('Not enough interior sensor samples')
    rc = reference['crop']
    active_x, active_y = reference['active_origin']
    def reference_points(dx, dy):
        rx, ry = rc['x']+x+dx, rc['y']+y+dy
        # CFA/black repeat grids are relative to the DNG ActiveArea.
        ax, ay = rx-active_x, ry-active_y
        colors = reference['cfa'][ay % 2, ax % 2]
        if not np.array_equal(colors, channel):
            return None
        values = codes[ry, rx].astype(np.float64)
        lut = reference['linearization']
        if lut is not None:
            values = lut[values.astype(int)]
        bh, bw = reference['black'].shape
        bk = reference['black'][ay % bh, ax % bw]
        return values, (values-bk)/(reference['white']-bk)
    candidates = []
    for dy in range(-radius, radius+1):
        for dx in range(-radius, radius+1):
            pair = reference_points(dx, dy)
            if pair is None:
                continue
            _, values = pair
            groups = []
            try:
                for p in range(4):
                    valid = (phase == p) & (values > .01) & (values < .9) & (normalized > .01) & (normalized < .9)
                    groups.append(fit(values[valid], normalized[valid]))
            except ValueError:
                continue
            candidates.append((sum(g['score'] for g in groups)/4, dx*dx+dy*dy, dx, dy, groups))
    if len(candidates) < 2:
        raise ValueError('No valid Bayer correspondence candidates')
    candidates.sort(key=lambda c: c[:4])
    score, _, dx, dy, groups = candidates[0]
    separation = candidates[1][0] > score*2+1e-5
    confirmed = separation and all(g['correlation'] > .9999 and g['residual_p90'] < 1e-4 and g['gain'] > 0 for g in groups)
    ref_codes, ref_normalized = reference_points(dx, dy)
    valid = (normalized > .02) & (normalized < .8) & (ref_normalized > .02) & (ref_normalized < .8)
    ev = np.log2(normalized[valid]/ref_normalized[valid])
    details = []
    for p in range(4):
        mask = valid & (phase == p)
        details.append(dict(sensor_phase=p, channel='RGB'[sensor['cfa'][p]],
                            normalized_fit_plugin_from_adobe=groups[p],
                            native_code_fit_plugin_from_adobe=fit(ref_codes[mask], native[mask]),
                            normalized_ev=summarize_ev(np.log2(normalized[mask]/ref_normalized[mask]))))
    return dict(
        status='correspondence_confirmed' if confirmed else 'inconclusive_registration',
        normalization_formula_max_error=formula_error,
        alignment=dict(reference_offset=[dx, dy], search_radius=radius, score=score,
                       runner_up_score=candidates[1][0], unique=bool(separation)),
        plugin_vs_adobe_normalized_ev=summarize_ev(ev) if confirmed else None,
        phases=details,
        caution='Phase fits are diagnostic only when correspondence is confirmed. No WB, baseline exposure, lens corrections, opcodes, or tone curves applied.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', required=True, type=Path)
    parser.add_argument('--arw', required=True, type=Path)
    parser.add_argument('--reference-dng', required=True, type=Path)
    parser.add_argument('--out', type=Path, default=Path('raw-level-comparison'))
    args = parser.parse_args()
    try:
        info = subprocess.run([str(args.engine.resolve()), 'info'], check=True, capture_output=True, text=True)
        if not json.loads(info.stdout).get('sensor_sample_diagnostics'):
            raise ValueError('This requires the helper from plugin build 8 or later')
        run = subprocess.run([str(args.engine.resolve()), 'inspect', str(args.arw.resolve()), '--sensor-samples'], check=True, capture_output=True, text=True)
        sensor = json.loads(run.stdout)['sensor']
        with reference_sensor(args.reference_dng) as (codes, reference):
            if reference['original_name'] and reference['original_name'].casefold() != args.arw.name.casefold():
                raise ValueError('Reference DNG OriginalRawFileName does not match the selected ARW')
            identity = 'OriginalRawFileName matched' if reference['original_name'] else 'OriginalRawFileName unavailable; verify that these are the exact same exposure'
            result = compare(sensor, codes, reference)
            levels = {k: v for k, v in reference.items() if k not in ('linearization', 'original_name')}
            levels['linearization_table_present'] = reference['linearization'] is not None
        report = dict(version=1, engine_version=json.loads(info.stdout)['engine_version'],
                      source={k: v for k, v in sensor.items() if k != 'samples'},
                      reference=levels, source_identity_check=identity, measurements=result,
                      scope='One original ARW and Adobe DNG converted from that exact ARW; native mosaics before RAW development. No original paths, serials or sample images included.')
        args.out.mkdir(parents=True, exist_ok=True)
        destination = args.out/'report.json'
        destination.write_text(json.dumps(report, indent=2, default=lambda v: v.tolist() if isinstance(v, np.ndarray) else v.item())+'\n')
        print(result['status'])
        if result['plugin_vs_adobe_normalized_ev'] is not None:
            print(f"Plugin vs Adobe normalized sensor: {result['plugin_vs_adobe_normalized_ev']['median']:+.6f} EV")
        print(f'Share {destination}; all RAW pixels stay local.')
    except subprocess.CalledProcessError as exc:
        parser.exit(2, f'Engine failed (exit {exc.returncode}): {exc.stderr[-4000:].strip()}\n')
    except (ValueError, OSError, rawpy.LibRawError) as exc:
        parser.exit(2, f'Check failed: {exc}\nIf the DNG compression is unsupported, re-export losslessly with Camera Raw 14.0 compatibility.\n')


if __name__ == '__main__':
    main()
