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


def translation_probe(reference, candidate):
    """Diagnostic phase correlation on edge features; never resample input pixels."""
    h, w = reference.shape
    if min(h, w) < 24:
        return None
    def edges(image):
        gy, gx = np.gradient(image.astype(np.float64))
        return np.hypot(gx, gy)
    a, b = edges(reference), edges(candidate)
    if min(float(a.std()), float(b.std())) < 1e-5:
        return None
    window = np.outer(np.hanning(h), np.hanning(w))
    fa, fb = [np.fft.rfft2((v-v.mean())*window) for v in (a, b)]
    cross = fa*np.conj(fb)
    cross /= np.maximum(np.abs(cross), 1e-12)
    correlation = np.fft.irfft2(cross, s=(h, w))
    py, px = np.unravel_index(np.argmax(correlation), correlation.shape)
    peak = float(correlation[py, px])
    psr = (peak-float(correlation.mean()))/max(float(correlation.std()), 1e-12)
    def refined(index, size, left, center, right):
        denominator = left-2*center+right
        fraction = .5*(left-right)/denominator if abs(denominator) > 1e-12 else 0.
        return float(index if index <= size//2 else index-size)+float(np.clip(fraction, -.5, .5))
    sx = refined(px,w,correlation[py,(px-1)%w],peak,correlation[py,(px+1)%w])
    sy = refined(py,h,correlation[(py-1)%h,px],peak,correlation[(py+1)%h,px])
    dx, dy = -int(round(sx)), -int(round(sy))
    if abs(dx) > w//4 or abs(dy) > h//4:
        return None
    ax, bx = max(0,-dx), max(0,dx)
    ay, by = max(0,-dy), max(0,dy)
    width, height = w-abs(dx), h-abs(dy)
    va = a[ay:ay+height,ax:ax+width].ravel()
    vb = b[by:by+height,bx:bx+width].ravel()
    if min(float(va.std()), float(vb.std())) < 1e-5:
        return None
    ncc = float(np.corrcoef(va,vb)[0,1])
    return dict(candidate_displacement=[-sx,-sy], edge_correlation=ncc,
                peak_sigma=psr, reliable=bool(ncc >= .8 and psr >= 8))


def geometry_check(reference, candidate, sampling_step):
    # Bound FFT workspaces independently of full-resolution TIFF dimensions.
    stride = max(1,(max(reference.shape[:2])+511)//512)
    weights = np.array([.2126,.7152,.0722])
    a, b = [v[::stride,::stride] @ weights for v in (reference,candidate)]
    unit = stride*sampling_step
    h, w = a.shape
    probes = []
    global_probe = translation_probe(a,b)
    if global_probe:
        global_probe['candidate_displacement_full_pixels'] = [v*unit for v in global_probe['candidate_displacement']]
    ph, pw = h//2, w//2
    for y in sorted(set([0,(h-ph)//2,h-ph])):
        for x in sorted(set([0,(w-pw)//2,w-pw])):
            probe = translation_probe(a[y:y+ph,x:x+pw],b[y:y+ph,x:x+pw])
            if probe:
                probe.update(center_fraction=[(x+pw/2)/w,(y+ph/2)/h],
                             candidate_displacement_full_pixels=[v*unit for v in probe['candidate_displacement']])
                probes.append(probe)
    reliable = [p for p in probes if p['reliable']]
    tested = reliable+([global_probe] if global_probe and global_probe['reliable'] else [])
    shifted = any(np.hypot(*p['candidate_displacement']) > .5 for p in tested)
    status = 'geometry_difference_detected' if shifted else 'no_large_shift_detected' if len(reliable)>=3 and global_probe and global_probe['reliable'] else 'inconclusive'
    return dict(status=status, geometry_sampling_step=unit, global_probe=global_probe, patches=probes,
                caveat='Translation probes only; no alignment applied. Sampling can alias fine detail. Small shifts, lens warps, crops and transforms are not ruled out by a negative check.')


def dng_metadata(path):
    # Whitelist calibration tags only: do not copy private MakerNotes, GPS or XMP.
    names = ['Make', 'Model', 'UniqueCameraModel', 'DNGVersion', 'BlackLevel',
             'WhiteLevel', 'BaselineExposure', 'BaselineExposureOffset', 'AsShotNeutral',
             'AnalogBalance', 'ColorMatrix1', 'ColorMatrix2', 'CameraCalibration1',
             'CameraCalibration2', 'ForwardMatrix1', 'ForwardMatrix2',
             'CalibrationIlluminant1', 'CalibrationIlluminant2', 'ActiveArea',
             'DefaultCropOrigin', 'DefaultCropSize', 'ProfileName',
             'AsShotWhiteXY', 'ColorimetricReference', 'CameraCalibrationSignature',
             'ProfileCalibrationSignature', 'DefaultBlackRender', 'LinearResponseLimit',
             'PhotometricInterpretation', 'BitsPerSample', 'SampleFormat',
             'BlackLevelRepeatDim', 'CFARepeatPatternDim', 'CFAPattern']
    with tifffile.TiffFile(path) as tif:
        root = tif.pages[0]
        if 'DNGVersion' not in root.tags:
            raise ValueError(f'{path.name}: not a DNG (missing DNGVersion)')
        # Adobe's mosaic DNG often has an RGB preview at IFD0 and the actual
        # sensor image in a SubIFD. Collect inherited root calibration, then
        # override image-specific tags from the first CFA/LinearRaw directory.
        def pages(directories, depth=0):
            for page in directories:
                yield page
                if depth < 4 and page.pages is not None:
                    yield from pages(page.pages, depth+1)
        raw = next((page for page in pages(tif.pages)
                    if int(page.photometric) in (32803, 34892)), None)
        if raw is None:
            raise ValueError(f'{path.name}: missing CFA or LinearRaw image directory')
        tags = {name: root.tags[name].value for name in names if name in root.tags}
        tags.update({name: raw.tags[name].value for name in names if name in raw.tags})
        return tags


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
    parser.add_argument('sony', type=Path, nargs='?', help='Lightroom TIFF exported from Sony ARQ')
    parser.add_argument('plugin', type=Path, nargs='?', help='Lightroom TIFF exported from plugin DNG')
    parser.add_argument('--dng', type=Path, help='Optional original plugin DNG, for calibration tags only')
    parser.add_argument('--reference-dng', type=Path, help='Adobe DNG Converter output from one original ARW')
    parser.add_argument('--metadata-only', action='store_true', help='Compare DNG calibration tags without TIFF exports')
    parser.add_argument('--out', type=Path, default=Path('pixelshift-comparison'))
    args = parser.parse_args()
    if args.metadata_only:
        if not (args.dng and args.reference_dng):
            parser.error('--metadata-only requires --dng and --reference-dng')
        if args.sony or args.plugin:
            parser.error('--metadata-only does not take TIFF export arguments')
    elif not (args.sony and args.plugin):
        parser.error('provide both TIFF exports, or use --metadata-only with two DNGs')
    try:
        if args.metadata_only:
            report = dict(version=2, mode='calibration-only',
                          plugin_dng_calibration=dng_metadata(args.dng),
                          reference_dng_calibration=dng_metadata(args.reference_dng),
                          limitations='Camera metadata only; no pixels or embedded Adobe profiles decoded. Normalized LinearRaw and mosaic RAW levels need not match numerically.')
            args.out.mkdir(parents=True, exist_ok=True)
            destination = args.out / 'report.json'
            destination.write_text(json.dumps(report, indent=2, default=json_value)+'\n')
            print(f'Share {destination}; no TIFF export or RAW pixel decode was performed.')
            return
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
        geometry = geometry_check(a,b,step)
        for info, image in [(sony_info,a),(plugin_info,b)]:
            info['sampled_rgb_sha256'] = hashlib.sha256(image.astype('<f4').tobytes()).hexdigest()
        report = dict(version=3, reference=sony_info, candidate=plugin_info,
                      sampling_step=step, ev_sign='negative means plugin darker',
                      method='sRGB linearized; paired uniform samples; smooth nonclipped midrange mask; no automatic alignment',
                      limitations='Rendered-light EV differences, not inferred RAW exposure. Small registration differences and Adobe rendering can affect results.',
                      measurements=result, geometry_check=geometry)
        if args.dng:
            report['plugin_dng_calibration'] = dng_metadata(args.dng)
        if args.reference_dng:
            report['reference_dng_calibration'] = dng_metadata(args.reference_dng)
        args.out.mkdir(parents=True, exist_ok=True)
        destination = args.out / 'report.json'
        destination.write_text(json.dumps(report, indent=2, default=json_value)+'\n')
        save_preview(args.out/'comparison.png', a, b, ev, mask)
        if result['luminance_ev'] is None:
            print('No usable smooth midrange samples. Report is inconclusive.')
        else:
            median = result['luminance_ev']['median']
            print(f'Plugin vs Sony: {median:+.4f} EV; ratio {2**median:.4f}; {result["usable_smooth_pixels"]} usable samples')
        print(f'Geometry check: {geometry["status"]} (no alignment applied)')
        print(f'Share {destination} and {args.out / "comparison.png"}; originals stay local.')
    except (ValueError, OSError, tifffile.TiffFileError) as exc:
        parser.exit(2, f'Comparison failed: {exc}\n')


if __name__ == '__main__':
    main()
