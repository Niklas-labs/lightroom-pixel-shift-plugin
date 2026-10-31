#!/usr/bin/env python3
"""Photometric sign/scale and export-safety checks for the local diagnostic."""
import importlib.util
from pathlib import Path
import tempfile
import json
import subprocess
import sys
import unittest
from unittest import mock

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

    def test_geometry_translation_is_detected_without_aligning_photometry(self):
        image = np.random.default_rng(7).uniform(.1,.7,(192,256))
        for _ in range(3):
            for axis in (0,1):
                image = (.25*np.roll(image,-1,axis)+.5*image+.25*np.roll(image,1,axis))
        rgb = np.repeat(image[...,None],3,axis=2)
        shifted = np.roll(rgb,(-2,3),axis=(0,1))*.9
        original = shifted.copy()
        report = comparison.geometry_check(rgb,shifted,10)
        self.assertEqual(report['status'],'geometry_difference_detected')
        dx,dy = report['global_probe']['candidate_displacement_full_pixels']
        self.assertAlmostEqual(dx,30,delta=1)
        self.assertAlmostEqual(dy,-20,delta=1)
        np.testing.assert_array_equal(shifted,original)
        report = comparison.geometry_check(rgb,rgb*.9,10)
        self.assertEqual(report['status'],'no_large_shift_detected')

    def test_geometry_flat_or_unrelated_images_are_inconclusive(self):
        constant = np.full((128,128,3),.5)
        self.assertEqual(comparison.geometry_check(constant,constant,1)['status'],'inconclusive')
        rng = np.random.default_rng(21)
        a,b = rng.uniform(.1,.7,(2,128,128,3))
        self.assertEqual(comparison.geometry_check(a,b,1)['status'],'inconclusive')

    def test_calibration_subifd_inheritance_without_decoding_pixels(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'reference.dng'
            with tifffile.TiffWriter(path) as writer:
                writer.write(np.zeros((8, 8, 3), dtype=np.uint8), photometric='rgb',
                             metadata=None, subifds=1,
                             extratags=[(50706, 'B', 4, (1, 4, 0, 0), False),
                                        (50730, '2i', 1, (1, 3), False)])
                writer.write(np.zeros((16, 16), dtype=np.uint16), photometric=32803,
                             metadata=None, extratags=[(50717, 'I', 1, 16383, False)])
            with mock.patch.object(tifffile.TiffPage, 'asarray', side_effect=AssertionError('pixel decode')):
                tags = comparison.dng_metadata(path)
            self.assertEqual(tags['WhiteLevel'], 16383)
            self.assertEqual(tags['BaselineExposure'], (1, 3))
            self.assertEqual(tags['PhotometricInterpretation'], 32803)
            self.assertEqual(tags['BitsPerSample'], 16)
            self.assertNotIn('MakerNote', tags)
            command = [sys.executable, str(Path(__file__).with_name('compare-lightroom.py')),
                       '--metadata-only', '--reference-dng', str(path), '--dng', str(path),
                       '--out', str(Path(directory)/'report')]
            subprocess.run(command, capture_output=True, text=True, check=True)
            report = json.loads((Path(directory)/'report/report.json').read_text())
            self.assertEqual(report['mode'], 'calibration-only')
            self.assertEqual(report['reference_dng_calibration']['WhiteLevel'], 16383)
            self.assertEqual(report['plugin_dng_calibration']['WhiteLevel'], 16383)
            self.assertFalse((Path(directory)/'report/comparison.png').exists())
            # Root LinearRaw should work as well as Adobe-style CFA SubIFDs.
            tifffile.imwrite(path, np.zeros((16, 16, 3), dtype=np.float32),
                             photometric=34892, metadata=None,
                             extratags=[(50706, 'B', 4, (1, 4, 0, 0), False)])
            self.assertEqual(comparison.dng_metadata(path)['PhotometricInterpretation'], 34892)

    def test_calibration_rejects_non_dng_and_missing_raw_ifd(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'not-dng.tif'
            tifffile.imwrite(path, np.zeros((8, 8, 3), dtype=np.uint8), photometric='rgb', metadata=None)
            with self.assertRaisesRegex(ValueError, 'not a DNG'):
                comparison.dng_metadata(path)
            tifffile.imwrite(path, np.zeros((8, 8, 3), dtype=np.uint8), photometric='rgb', metadata=None,
                             extratags=[(50706, 'B', 4, (1, 4, 0, 0), False)])
            with self.assertRaisesRegex(ValueError, 'missing CFA or LinearRaw'):
                comparison.dng_metadata(path)
            command = [sys.executable, str(Path(__file__).with_name('compare-lightroom.py')), '--metadata-only']
            run = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(run.returncode, 2)
            self.assertIn('requires --dng and --reference-dng', run.stderr)

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
