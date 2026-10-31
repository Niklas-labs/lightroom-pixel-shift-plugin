#!/usr/bin/env python3
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import numpy as np
import tifffile

spec = importlib.util.spec_from_file_location('benchmark',Path(__file__).with_name('benchmark-capture.py'))
benchmark = importlib.util.module_from_spec(spec)
spec.loader.exec_module(benchmark)
ENGINE = None


class BenchmarkTests(unittest.TestCase):
    def test_strip_hash_is_exact_across_compression_and_partial_bands(self):
        values = np.arange(19*31*3,dtype=np.float32).reshape(19,31,3)/1000-.1
        expected = hashlib.sha256(values.astype('<f4').tobytes()).hexdigest()
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'strip.dng'
            for rows,compression in [(1,None),(7,'deflate'),(256,'deflate')]:
                tifffile.imwrite(path,values,photometric=34892,planarconfig='contig',rowsperstrip=rows,compression=compression,metadata=None)
                with mock.patch.object(tifffile.TiffPage,'asarray',side_effect=AssertionError('full-frame decode')):
                    report=benchmark.hash_dng(path)
                self.assertEqual(report['decoded_sha256'],expected)
                self.assertEqual(report['decoded_bytes'],values.nbytes)
                self.assertLessEqual(report['max_decoded_strip_bytes'],min(rows,19)*31*12)
            values[0,0,0]+=.001
            tifffile.imwrite(path,values,photometric=34892,planarconfig='contig',metadata=None)
            self.assertNotEqual(benchmark.hash_dng(path)['decoded_sha256'],expected)
            tifffile.imwrite(path,np.zeros((4,4,3),np.uint16),photometric='rgb',metadata=None)
            with self.assertRaisesRegex(ValueError,'LinearRaw'):
                benchmark.hash_dng(path)

    def test_real_controller_with_explicit_synthetic_inputs_and_no_overwrite(self):
        if ENGINE is None:
            self.skipTest('Pass --engine to test CLI orchestration')
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            subprocess.run([sys.executable,str(Path(__file__).with_name('generate-demo.py')),str(root/'fixtures')],check=True,capture_output=True)
            inputs=sorted((root/'fixtures').glob('frame-*.json'))
            output=root/'benchmark'
            command=[sys.executable,str(Path(__file__).with_name('benchmark-capture.py')),'--engine',str(ENGINE),
                     '--out',str(output),'--fixtures','--strip-rows','7']+[str(p) for p in inputs]
            subprocess.run(command,check=True,capture_output=True,text=True)
            report=json.loads((output/'report.json').read_text())
            self.assertTrue(report['decoded_outputs_identical'])
            self.assertEqual(report['capture_type'],'synthetic fixtures')
            self.assertEqual(report['source_count'],16)
            self.assertEqual(report['run_order'],['ram','disk'])
            self.assertNotIn(str(root),json.dumps(report))
            self.assertGreater(report['runs'][1]['execution']['scratch_bytes'],0)
            original=(output/'report.json').read_bytes()
            second=subprocess.run(command,capture_output=True,text=True)
            self.assertEqual(second.returncode,2)
            self.assertIn('new or empty',second.stderr)
            self.assertEqual((output/'report.json').read_bytes(),original)


if __name__ == '__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--engine',type=Path)
    args=parser.parse_args()
    ENGINE=args.engine.resolve() if args.engine else None
    unittest.main(argv=[sys.argv[0]])
