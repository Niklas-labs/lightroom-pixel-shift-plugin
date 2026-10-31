#!/usr/bin/env python3
"""Portable tests of package layout, version metadata, and ZIP permissions."""
import hashlib
import importlib.util
import json
import os
import subprocess
from pathlib import Path
import stat
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('package_plugin', ROOT / 'scripts' / 'package-plugin.py')
PACKAGER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKAGER)
SMOKE_SPEC = importlib.util.spec_from_file_location('test_package', ROOT / 'scripts' / 'test-package.py')
SMOKE = importlib.util.module_from_spec(SMOKE_SPEC)
SMOKE_SPEC.loader.exec_module(SMOKE)


class PackagingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.binary = self.root / 'engine'
        self.binary.write_bytes(b'fixture executable bytes')
        self.info = {'protocol_version': 1, 'engine_version': '0.1.0', 'metadata_backend': 'native-rust'}
        license_root = self.root / 'dependency'
        license_root.mkdir()
        (license_root / 'Cargo.toml').write_text('[package]\nname="fixture"', encoding='utf-8')
        (license_root / 'LICENSE-MIT').write_text('Fixture license notice', encoding='utf-8')
        self.metadata = {'packages': [{'name': 'fixture', 'version': '1.0.0', 'license': 'MIT',
                                      'id': 'fixture-id', 'manifest_path': str(license_root / 'Cargo.toml')}],
                         'resolve': {'nodes': [{'id': 'fixture-id'}]}}

    def package(self, platform):
        return PACKAGER.build_package(self.binary, platform, self.root / 'dist', 'a' * 40,
                                      self.metadata, self.info)

    def test_macos_zip_preserves_helper_permission_and_notices(self):
        archive = self.package('macos-universal')
        with zipfile.ZipFile(archive) as zipped:
            name = 'SonyPixelShift.lrplugin/bin/sony-pixelshift'
            entry = zipped.getinfo(name)
            self.assertTrue((entry.external_attr >> 16) & stat.S_IXUSR)
            self.assertEqual(zipped.read(name), self.binary.read_bytes())
            self.assertEqual(zipped.read('SonyPixelShift.lrplugin/licenses/dependencies/fixture-1.0.0/LICENSE-MIT'),
                             b'Fixture license notice')
            manifest = json.loads(zipped.read('SonyPixelShift.lrplugin/manifest.json'))
            self.assertEqual(manifest['engine_sha256'], hashlib.sha256(self.binary.read_bytes()).hexdigest())
            self.assertEqual(manifest['source_revision'], 'a' * 40)
            self.assertFalse(manifest['notarized'])
            self.assertIn('SonyPixelShift.lrplugin/Info.lua', zipped.namelist())
            self.assertIn('INSTALL.txt', zipped.namelist())
            self.assertIn('SonyPixelShift.lrplugin/config/camera-profiles.yaml', zipped.namelist())
            self.assertNotIn('SonyPixelShift.lrplugin/config/camera-profiles.json', zipped.namelist())
            self.assertIn('SonyPixelShift.lrplugin/config/camera-profiles.schema.json', zipped.namelist())
            self.assertFalse(any('/target/' in name or '/.cargo/' in name for name in zipped.namelist()))
        checksum = archive.with_suffix('.zip.sha256').read_text(encoding='ascii').split()[0]
        self.assertEqual(checksum, hashlib.sha256(archive.read_bytes()).hexdigest())

    @unittest.skipUnless(os.environ.get('PIXELSHIFT_TEST_LUA'), 'Set PIXELSHIFT_TEST_LUA to Lua 5.1')
    def test_extracted_toolkit_loads_without_source_tree_fallback(self):
        archive = self.package('macos-universal')
        destination = self.root / 'installed package'
        SMOKE.extract_package(archive, destination)
        plugin = destination / 'SonyPixelShift.lrplugin'
        command = [os.environ['PIXELSHIFT_TEST_LUA'], str(ROOT / 'scripts/test-plugin-load.lua'), str(plugin)]
        result = subprocess.run(command, cwd=self.root, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        # A missing dependency must fail this gate, as it would in Lightroom.
        (plugin / 'PixelShiftProgress.lua').unlink()
        missing = subprocess.run(command, cwd=self.root, capture_output=True, text=True)
        self.assertNotEqual(missing.returncode, 0)
        self.assertIn('PixelShiftProgress.lua', missing.stderr)

    def test_signed_package_requires_matching_receipt_and_labels_helper(self):
        receipt = {'status': 'Accepted', 'id': '00000000-0000-0000-0000-000000000002',
                   'engine_sha256': hashlib.sha256(self.binary.read_bytes()).hexdigest()}
        archive = PACKAGER.build_package(self.binary, 'macos-universal', self.root / 'dist', 'a'*40,
                                        self.metadata, self.info, macos_signing='developer-id',
                                        notarization_receipt=receipt)
        with zipfile.ZipFile(archive) as zipped:
            manifest=json.loads(zipped.read('SonyPixelShift.lrplugin/manifest.json'))
            self.assertTrue(manifest['notarized'])
            self.assertEqual(manifest['macos_signing'],'developer-id')
            self.assertEqual(manifest['distribution'],'developer-id')
            self.assertEqual(manifest['notarization_submission_id'],receipt['id'])
            self.assertIn(b'Developer ID signed and notarized',zipped.read('INSTALL.txt'))
            self.assertNotIn(b'This development build is not notarized',zipped.read('INSTALL.txt'))
            self.assertFalse(any('openbao' in name for name in zipped.namelist()))
        with self.assertRaises(ValueError):
            PACKAGER.build_package(self.binary,'windows-x86_64',self.root/'bad','a'*40,
                                   self.metadata,self.info,macos_signing='developer-id',notarization_receipt=receipt)

    def test_windows_uses_exe_and_records_platform(self):
        archive = self.package('windows-x86_64')
        with zipfile.ZipFile(archive) as zipped:
            manifest = json.loads(zipped.read('SonyPixelShift.lrplugin/manifest.json'))
            self.assertEqual(manifest['engine'], 'bin/sony-pixelshift.exe')
            self.assertEqual(manifest['platform'], 'windows-x86_64')
            self.assertEqual(zipped.read('SonyPixelShift.lrplugin/bin/sony-pixelshift.exe'), self.binary.read_bytes())

    def test_rejects_unidentified_revision_and_unknown_platform(self):
        for platform, revision in [('linux', 'a' * 40), ('macos-universal', 'unknown')]:
            with self.assertRaises(ValueError):
                PACKAGER.build_package(self.binary, platform, self.root / 'dist', revision,
                                       self.metadata, self.info)

    def test_windows_command_matches_lua_launcher_quoting(self):
        command = SMOKE.windows_command(r"C:\Program Files (x86)\a&b.exe", ['info', '--output', r"C:\写真\a'b.json"])
        self.assertEqual(command, '\"\"C:\\Program Files (x86)\\a&b.exe\" \"info\" \"--output\" \"C:\\写真\\a\'b.json\"\"')
        for bad in ['%TEMP%', 'a!b', 'a^b', 'a"b', 'a\nb']:
            with self.assertRaises(ValueError):
                SMOKE.windows_command(bad, [])

    def test_rejects_unsafe_version_filename(self):
        self.info['engine_version'] = '../bad'
        with self.assertRaises(ValueError):
            self.package('macos-universal')


if __name__ == '__main__':
    unittest.main()
