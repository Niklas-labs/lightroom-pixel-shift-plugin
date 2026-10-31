#!/usr/bin/env python3
"""Compare matching full-size, uncompressed 16-bit sRGB Lightroom TIFF exports.

Reads a bounded sampling grid using memory maps; never develops RAWs or modifies
inputs. Negative EV means the plugin export is darker than the Sony export.
"""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
import tifffile
from PIL import Image, ImageCms, ImageDraw


def linearize(rgb):
    return np.where(rgb <= 0.04045, rgb / 12.92, ((rgb + 0.055) / 1.055) ** 2.4)


def open_export(path):
    with tifffile.TiffFile(path) as tif:
        page = tif.pages[0]
        if page.dtype != np.dtype('uint16') or len(page.shape) != 3 or page.shape[2] != 3 or int(page.photometric) != 2:
            raise ValueError(f'{path.name}: export RGB 16-bit TIFF, without transparency')
        if int(page.compression) != 1 or page.is_tiled or int(page.planarconfig) != 1:
            raise ValueError(f'{path.name}: export with TIFF compression None (interleaved strips)')
        orientation = page.tags.get('Orientation')
        if orientation and orientation.value != 1:
            raise ValueError(f'{path.name}: export with image orientation baked in (Orientation 1)')
        profile = page.tags.get('InterColorProfile') or page.tags.get('ICCProfile')
        if profile is None:
            raise ValueError(f'{path.name}: missing embedded sRGB ICC profile')
        icc = profile.value
        import io
        try:
            description = ImageCms.getProfileDescription(ImageCms.ImageCmsProfile(io.BytesIO(icc))).strip()
        except ImageCms.PyCMSError as exc:
            raise ValueError(f'{path.name}: unreadable ICC profile') from exc
        if 'srgb' not in description.lower():
            raise ValueError(f'{path.name}: expected sRGB export; ICC profile is {description!r}')
        metadata = dict(file=path.name, dimensions=[page.imagewidth, page.imagelength],
                        icc_description=description, icc_sha256=hashlib.sha256(icc).hexdigest())
    return tifffile.memmap(path, page=0, mode='r'), metadata


def statistics(values):
    if not values.size:
        return None
    return dict(count=int(values.size), median=float(np.median(values)),
                p10=float(np.percentile(values, 10)), p90=float(np.percentile(values, 90)))


def measure(reference, candidate):
    a, b = linearize(reference), linearize(candidate)
    weights = np.array([0.2126, 0.7152, 0.0722], dtype=np.float32)
    ya, yb = a @ weights, b @ weights
    # Avoid noise-floor ratios and clipping. Keep comparisons spatially paired.
    valid = (ya > 0.02) & (ya < 0.8) & (yb > 0.02) & (yb < 0.8)
    valid &= np.all((reference > 0.002) & (reference < 0.995) &
                    (candidate > 0.002) & (candidate < 0.995), axis=-1)
    # Small crop/registration differences are particularly misleading at edges.
    # Restrict primary measurement to smooth neighborhoods in BOTH exports.
    smooth = np.ones(ya.shape, dtype=bool)
    for y in (ya, yb):
        for axis in (0, 1):
            delta = np.abs(np.diff(y, axis=axis))
            if axis == 0:
                smooth[1:] &= delta < 0.02
                smooth[:-1] &= delta < 0.02
            else:
                smooth[:, 1:] &= delta < 0.02
                smooth[:, :-1] &= delta < 0.02
    smooth[[0, -1], :] = False
    smooth[:, [0, -1]] = False
    mask = valid & smooth
    ev = np.log2(np.maximum(yb, 1e-12) / np.maximum(ya, 1e-12))
    channel_ev = np.log2(np.maximum(b, 1e-12) / np.maximum(a, 1e-12))
    neutral = mask & ((reference.max(axis=-1) - reference.min(axis=-1)) < 0.05)
    h, w = mask.shape
    yy, xx = np.mgrid[:h, :w]
    center = (xx >= w / 4) & (xx < 3*w / 4) & (yy >= h / 4) & (yy < 3*h / 4)
    result = dict(
        sampled_pixels=int(mask.size), usable_smooth_pixels=int(mask.sum()),
        luminance_ev=statistics(ev[mask]),
        channel_ev={name: statistics(channel_ev[..., c][mask]) for c, name in enumerate('RGB')},
        neutral_luminance_ev=statistics(ev[neutral]),
        center_luminance_ev=statistics(ev[mask & center]),
        outer_luminance_ev=statistics(ev[mask & ~center]),
        tone_luminance_ev={name: statistics(ev[mask & (ya >= lo) & (ya < hi)])
                           for name, lo, hi in [('shadows', .02, .1), ('midtones', .1, .4), ('highlights', .4, .8)]},
    )
    if result['luminance_ev'] is not None:
        result['equivalent_brightness_ratio'] = 2 ** result['luminance_ev']['median']
    return result, ev, mask


def dng_metadata(path):
    # Whitelist calibration tags only: do not copy private MakerNotes, GPS or XMP.
    names = ['Make', 'Model', 'UniqueCameraModel', 'DNGVersion', 'BlackLevel',
             'WhiteLevel', 'BaselineExposure', 'BaselineExposureOffset', 'AsShotNeutral',
             'AnalogBalance', 'ColorMatrix1', 'ColorMatrix2', 'CameraCalibration1',
             'CameraCalibration2', 'ForwardMatrix1', 'ForwardMatrix2',
             'CalibrationIlluminant1', 'CalibrationIlluminant2', 'ActiveArea',
             'DefaultCropOrigin', 'DefaultCropSize', 'ProfileName']
    with tifffile.TiffFile(path) as tif:
        page = tif.pages[0]
        return {name: page.tags[name].value for name in names if name in page.tags}


def save_preview(path, reference, candidate, ev, mask):
    panels = []
    heat = np.full_like(reference, .5)
    strength = np.minimum(np.abs(ev) / .5, 1)
    # Red: plugin darker; blue: plugin brighter; gray: excluded samples.
    heat[mask] = 1
    heat[..., 1][mask] = 1-strength[mask]
    heat[..., 2][mask & (ev < 0)] = 1-strength[mask & (ev < 0)]
    heat[..., 0][mask & (ev >= 0)] = 1-strength[mask & (ev >= 0)]
    for array, title in [(reference, 'Sony ARQ export'), (candidate, 'Plugin DNG export'),
                         (heat, 'Red: darker | Blue: brighter | Full color: 0.5 EV')]:
        picture = Image.fromarray(np.rint(np.clip(array, 0, 1)*255).astype(np.uint8))
        picture.thumbnail((640, 640))
        panel = Image.new('RGB', (picture.width, picture.height+40), 'white')
        panel.paste(picture, (0, 40))
        ImageDraw.Draw(panel).text((8, 8), title, fill='black')
        panels.append(panel)
    canvas = Image.new('RGB', (sum(p.width for p in panels), max(p.height for p in panels)), 'white')
    offset = 0
    for panel in panels:
        canvas.paste(panel, (offset, 0))
        offset += panel.width
    canvas.save(path)


def json_value(value):
    if isinstance(value, bytes):
        return list(value)
    if isinstance(value, np.ndarray):
        return value.tolist()
    if isinstance(value, np.generic):
        return value.item()
    raise TypeError(f'Unexpected metadata type: {type(value).__name__}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('sony', type=Path, help='Lightroom TIFF exported from Sony ARQ')
    parser.add_argument('plugin', type=Path, help='Lightroom TIFF exported from plugin DNG')
    parser.add_argument('--dng', type=Path, help='Optional original plugin DNG, for calibration tags only')
    parser.add_argument('--out', type=Path, default=Path('pixelshift-comparison'))
    args = parser.parse_args()
    try:
        sony, sony_info = open_export(args.sony)
        plugin, plugin_info = open_export(args.plugin)
        if sony.shape != plugin.shape:
            raise ValueError(f'Export dimensions differ: {sony.shape} vs {plugin.shape}. Use the same full-size crop; do not resize to hide a mismatch.')
        if sony_info['icc_sha256'] != plugin_info['icc_sha256']:
            raise ValueError('Embedded ICC profiles differ; export both using the same sRGB profile')
        step = max(1, (max(sony.shape[:2])+1023)//1024)
        a = sony[::step, ::step].astype(np.float32)/65535
        b = plugin[::step, ::step].astype(np.float32)/65535
        result, ev, mask = measure(a, b)
        report = dict(version=1, reference=sony_info, candidate=plugin_info,
                      sampling_step=step, ev_sign='negative means plugin darker',
                      method='sRGB linearized; paired uniform samples; smooth nonclipped midrange mask; no automatic alignment',
                      limitations='Rendered-light EV differences, not inferred RAW exposure. Small registration differences and Adobe rendering can affect results.',
                      measurements=result)
        if args.dng:
            report['plugin_dng_calibration'] = dng_metadata(args.dng)
        args.out.mkdir(parents=True, exist_ok=True)
        destination = args.out / 'report.json'
        destination.write_text(json.dumps(report, indent=2, default=json_value)+'\n')
        save_preview(args.out/'comparison.png', a, b, ev, mask)
        if result['luminance_ev'] is None:
            print('No usable smooth midrange samples. Report is inconclusive.')
        else:
            median = result['luminance_ev']['median']
            print(f'Plugin vs Sony: {median:+.4f} EV; ratio {2**median:.4f}; {result["usable_smooth_pixels"]} usable samples')
        print(f'Share {destination} and {args.out / "comparison.png"}; originals stay local.')
    except (ValueError, OSError, tifffile.TiffFileError) as exc:
        parser.exit(2, f'Comparison failed: {exc}\n')


if __name__ == '__main__':
    main()
