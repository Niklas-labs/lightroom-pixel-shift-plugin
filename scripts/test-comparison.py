#!/usr/bin/env python3
"""Photometric sign/scale and export-safety checks for the local diagnostic."""
import importlib.util
from pathlib import Path
import tempfile
import json
import subprocess
import sys
import unittest

import numpy as np
import tifffile
from PIL import ImageCms

spec = importlib.util.spec_from_file_location('comparison', Path(__file__).with_name('compare-lightroom.py'))
comparison = importlib.util.module_from_spec(spec)
spec.loader.exec_module(comparison)


def encode(linear):
    return np.where(linear <= .0031308, linear*12.92, 1.055*linear**(1/2.4)-.055).astype(np.float32)


class ComparisonTests(unittest.TestCase):
    def test_known_exposure_shifts_and_neutral(self):
        y, x = np.mgrid[:64, :80]
        linear = np.repeat((.05+x/400+y/800)[..., None], 3, axis=2)
        for shift in (0., -.25, .125):
            a, b = [np.rint(encode(v)*65535).astype(np.uint16).astype(np.float32)/65535
                    for v in (linear, linear*2**shift)]
            result, _, _ = comparison.measure(a, b)
            self.assertAlmostEqual(result['luminance_ev']['median'], shift, delta=3e-5)
            self.assertAlmostEqual(result['neutral_luminance_ev']['median'], shift, delta=3e-5)
            for name in 'RGB':
                self.assertAlmostEqual(result['channel_ev'][name]['median'], shift, delta=3e-5)

    def test_clipped_and_black_are_inconclusive(self):
        for value in (0., 1.):
            rgb = np.full((32, 32, 3), value, dtype=np.float32)
            result, _, mask = comparison.measure(rgb, rgb)
            self.assertIsNone(result['luminance_ev'])
            self.assertFalse(mask.any())

    def test_color_change_and_edges(self):
        linear = np.full((64, 80, 3), .2, dtype=np.float32)
        changed = linear * np.array([.9, 1, 1.1])
        result, _, _ = comparison.measure(encode(linear), encode(changed))
        for name, ratio in zip('RGB', (.9, 1, 1.1)):
            self.assertAlmostEqual(result['channel_ev'][name]['median'], np.log2(ratio), places=5)
        image = encode(linear)
        image[:, 40:] = .9
        _, _, mask = comparison.measure(image, image)
        self.assertFalse(mask[:, 39:41].any())

    def test_cli_report_and_dimension_rejection(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            icc = ImageCms.ImageCmsProfile(ImageCms.createProfile('sRGB')).tobytes()
            a = np.full((32, 40, 3), .2, dtype=np.float32)
            for name, linear in [('sony', a), ('plugin', a*2**-.25)]:
                tifffile.imwrite(root/f'{name}.tif', np.rint(encode(linear)*65535).astype(np.uint16),
                                 photometric='rgb', metadata=None, iccprofile=icc)
            command = [sys.executable, str(Path(__file__).with_name('compare-lightroom.py')),
                       str(root/'sony.tif'), str(root/'plugin.tif'), '--out', str(root/'results')]
            run = subprocess.run(command, capture_output=True, text=True, check=True)
            report = json.loads((root/'results/report.json').read_text())
            self.assertAlmostEqual(report['measurements']['luminance_ev']['median'], -.25, delta=3e-5)
            self.assertEqual(report['reference']['file'], 'sony.tif')
            self.assertNotIn(str(root), json.dumps(report))
            self.assertIn('Plugin vs Sony', run.stdout)
            self.assertTrue((root/'results/comparison.png').is_file())
            tifffile.imwrite(root/'plugin.tif', np.ones((30, 40, 3), dtype=np.uint16),
                             photometric='rgb', metadata=None, iccprofile=icc)
            run = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(run.returncode, 2)
            self.assertIn('dimensions differ', run.stderr)

    def test_uncompressed_memmap_profile_and_preview(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'test.tif'
            rgb = np.full((32, 40, 3), 30000, dtype=np.uint16)
            icc = ImageCms.ImageCmsProfile(ImageCms.createProfile('sRGB')).tobytes()
            tifffile.imwrite(path, rgb, photometric='rgb', metadata=None, iccprofile=icc)
            mapped, info = comparison.open_export(path)
            np.testing.assert_array_equal(mapped, rgb)
            self.assertEqual(info['dimensions'], [40, 32])
            a = mapped.astype(np.float32)/65535
            _, ev, mask = comparison.measure(a, a)
            comparison.save_preview(Path(directory)/'preview.png', a, a, ev, mask)
            self.assertTrue((Path(directory)/'preview.png').is_file())
            del mapped
            tifffile.imwrite(path, rgb, photometric='rgb', metadata=None)
            with self.assertRaisesRegex(ValueError, 'missing embedded'):
                comparison.open_export(path)
            tifffile.imwrite(path, rgb, photometric='rgb', metadata=None, iccprofile=icc, compression='deflate')
            with self.assertRaisesRegex(ValueError, 'compression None'):
                comparison.open_export(path)
            adobe = ImageCms.ImageCmsProfile(ImageCms.createProfile('LAB')).tobytes()
            tifffile.imwrite(path, rgb, photometric='rgb', metadata=None, iccprofile=adobe)
            with self.assertRaisesRegex(ValueError, 'expected sRGB'):
                comparison.open_export(path)


if __name__ == '__main__':
    unittest.main()
