#!/usr/bin/env python3
import copy
import importlib.util
from pathlib import Path
import tempfile
import unittest

import numpy as np
import tifffile

spec = importlib.util.spec_from_file_location('check', Path(__file__).with_name('check-raw-levels.py'))
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


def fixture(white=16383, offset=(2, -2)):
    codes = np.random.default_rng(42).integers(1200, 10000, (88, 100), dtype=np.uint16)
    crop = dict(x=4, y=4, width=80, height=64)
    samples = []
    for y in np.linspace(0, 62, 24, dtype=int)//2*2:
        for x in np.linspace(0, 78, 32, dtype=int)//2*2:
            for dy in range(2):
                for dx in range(2):
                    sx, sy = int(x+dx+4), int(y+dy+4)
                    phase = (sy % 2)*2+sx % 2
                    code = int(codes[sy, sx])
                    samples.append(dict(x=sx-4, y=sy-4, phase=phase, channel=[0,1,1,2][phase], code=code,
                                        normalized=float(np.float32(code-512)/np.float32(white-512))))
    sensor = dict(crop=crop, cfa=[0,1,1,2], black=[512]*4, white=[white]*4, samples=samples)
    dx, dy = offset
    reference_codes = np.roll(codes, (6+dy-4, 6+dx-4), axis=(0, 1))
    reference = dict(crop=dict(x=6, y=6, width=80, height=64), active_origin=[0,0],
                     cfa=np.array([[0,1],[1,2]]), black=np.full((2,2),512), white=16383, linearization=None)
    return sensor, reference_codes, reference


class SensorTests(unittest.TestCase):
    def test_equal_levels_and_exact_integer_registration(self):
        for offset in [(0,0),(2,-2),(-4,4)]:
            report = check.compare(*fixture(offset=offset))
            self.assertEqual(report['status'], 'correspondence_confirmed')
            self.assertEqual(report['alignment']['reference_offset'], list(offset))
            self.assertAlmostEqual(report['plugin_vs_adobe_normalized_ev']['median'], 0, places=6)
            for phase in report['phases']:
                fit = phase['native_code_fit_plugin_from_adobe']
                self.assertAlmostEqual(fit['gain'], 1, places=8)
                self.assertAlmostEqual(fit['offset'], 0, places=8)

    def test_white_level_mismatch_is_measured_without_rescaling_it_away(self):
        report = check.compare(*fixture(white=15500))
        expected = np.log2((16383-512)/(15500-512))
        self.assertEqual(report['status'], 'correspondence_confirmed')
        self.assertAlmostEqual(report['plugin_vs_adobe_normalized_ev']['median'], expected, places=6)
        for phase in report['phases']:
            self.assertAlmostEqual(phase['normalized_fit_plugin_from_adobe']['gain'], 2**expected, places=6)

    def test_formula_error_and_size_mismatch_rejected(self):
        sensor, codes, reference = fixture()
        broken = copy.deepcopy(sensor)
        broken['samples'][0]['normalized'] += .1
        with self.assertRaisesRegex(ValueError, 'normalization disagrees'):
            check.compare(broken, codes, reference)
        reference['crop']['width'] -= 1
        with self.assertRaisesRegex(ValueError, 'crops differ'):
            check.compare(sensor, codes, reference)

    def test_unrelated_sensor_pixels_do_not_claim_correspondence(self):
        sensor, codes, reference = fixture()
        codes = np.random.default_rng(99).integers(1200, 10000, codes.shape, dtype=np.uint16)
        report = check.compare(sensor, codes, reference)
        self.assertEqual(report['status'], 'inconclusive_registration')
        self.assertIsNone(report['plugin_vs_adobe_normalized_ev'])

    def test_reference_dng_tags_and_libraw_codes(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'source.dng'
            codes = np.arange(32*40, dtype=np.uint16).reshape(32,40)+512
            tags = [(50706,'B',4,(1,4,0,0),False), (50707,'B',4,(1,4,0,0),False),
                    (50708,'s',0,'Synthetic sensor',False), (33421,'H',2,(2,2),False),
                    (33422,'B',4,(0,1,1,2),False), (50713,'H',2,(2,2),False),
                    (50714,'2I',4,(512,1)*4,False), (50717,'I',1,16383,False),
                    (50719,'2I',2,(2,1,4,1),False), (50720,'2I',2,(36,1,24,1),False),
                    (50829,'I',4,(0,0,32,40),False),
                    (50721,'2i',9,(1,1,0,1,0,1,0,1,1,1,0,1,0,1,0,1,1,1),False),
                    (50778,'H',1,21,False)]
            tifffile.imwrite(path,codes,photometric=32803,metadata=None,extratags=tags)
            with check.reference_sensor(path) as (decoded, reference):
                np.testing.assert_array_equal(decoded,codes)
                np.testing.assert_array_equal(reference['cfa'], [[0,1],[1,2]])
                self.assertEqual(reference['crop'],dict(x=2,y=4,width=36,height=24))
                self.assertEqual(reference['white'],16383)
                np.testing.assert_array_equal(reference['black'],np.full((2,2),512))


if __name__ == '__main__':
    unittest.main()
