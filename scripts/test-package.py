#!/usr/bin/env python3
"""Extract a CI package and smoke-test its embedded engine with an empty PATH."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import stat
import subprocess
import sys
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def extract_package(archive, destination):
    with zipfile.ZipFile(archive) as zipped:
        for entry in zipped.infolist():
            path = Path(entry.filename)
            if path.is_absolute() or '..' in path.parts or '\\' in entry.filename:
                raise ValueError('Unsafe archive path')
            target = destination / path
            if entry.is_dir():
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(zipped.read(entry))
                target.chmod((entry.external_attr >> 16) & 0o777)


def windows_command(executable, args):
    # Match Engine.command / Protocol.quote rather than Python list2cmdline.
    parts = [str(executable)] + [str(value) for value in args]
    for value in parts:
        if any(c in value for c in '%!^"\r\n\0'):
            raise ValueError('Unsupported Windows shell argument')
    return '"' + ' '.join('"' + value + '"' for value in parts) + '"'


def run_engine(executable, args, env, check=False):
    if os.name == 'nt':
        comspec = os.environ.get('COMSPEC') or str(Path(os.environ['SystemRoot']) / 'System32' / 'cmd.exe')
        command = '"' + comspec + '" /d /s /c ' + windows_command(executable, args)
        return subprocess.run(command, executable=comspec, env=env, check=check,
                              capture_output=True, text=True, encoding='utf-8')
    return subprocess.run([str(executable)] + [str(value) for value in args], env=env, check=check,
                          capture_output=True, text=True, encoding='utf-8')


def smoke_test(archive, expected_platform, expected_machine=None):
    if expected_machine and platform.machine().lower() not in expected_machine.lower().split(','):
        raise AssertionError(f'Expected native {expected_machine}; running on {platform.machine()}')
    with tempfile.TemporaryDirectory(prefix='pixel shift package ') as temporary:
        root = Path(temporary) / "Unicode café 写真 and apostrophe's (test) & folder"
        extract_package(archive, root)
        plugin = root / 'SonyPixelShift.lrplugin'
        manifest = json.loads((plugin / 'manifest.json').read_text(encoding='utf-8'))
        assert manifest['platform'] == expected_platform
        engine = plugin / manifest['engine']
        assert hashlib.sha256(engine.read_bytes()).hexdigest() == manifest['engine_sha256']
        if expected_platform == 'macos-universal':
            assert engine.stat().st_mode & stat.S_IXUSR
        assert (plugin / 'Info.lua').is_file()
        assert (plugin / 'licenses' / 'PROJECT-GPL-3.0.txt').is_file()
        assert (plugin / 'licenses' / 'dependency-inventory.json').is_file()
        env = dict(os.environ, PATH='')
        info_file = root / 'engine-info.json'
        info_run = run_engine(engine, ['info', '--output', info_file], env, check=True)
        info = json.loads(info_run.stdout)
        assert json.loads(info_file.read_text(encoding='utf-8')) == info
        assert info['protocol_version'] == manifest['protocol_version'] == 1
        assert info['engine_version'] == manifest['engine_version']
        assert info['metadata_backend'] == manifest['metadata_backend']
        assert info['camera_profile_schema_version'] == 1
        assert info['camera_models'] == [c['model'] for c in json.loads((plugin / 'config' / 'camera-profiles.json').read_text(encoding='utf-8'))['cameras']]
        assert any(word in info['metadata_backend'].lower() for word in ('native', 'rust'))
        demo = root / 'demo'
        python_env = dict(os.environ, PYTHONIOENCODING='utf-8')
        subprocess.run([sys.executable, str(ROOT / 'scripts' / 'generate-demo.py'), str(demo)], env=python_env, check=True,
                       capture_output=True, text=True, encoding='utf-8')
        inspected = run_engine(engine, ['inspect', demo / 'capture-metadata.ARW'], env, check=True)
        capture = json.loads(inspected.stdout)['capture']
        assert capture['ISO'] == 100 and capture['SequenceNumber'] == 3
        assert capture['SerialNumber'] == 'camera-x'
        assert capture['pixel_shift'] == {'group_id': 42, 'shot_number': 3, 'shot_count': 16}
        result_file = root / 'merge-result.json'
        log_file = root / 'engine.log'
        header = 'Lightroom launcher log header\n'
        log_file.write_text(header, encoding='utf-8')
        merged = run_engine(engine, ['merge', '--fixtures', '--job', demo / 'job.json',
                                    '--result', result_file, '--log', log_file], env, check=True)
        report = json.loads(merged.stdout)
        assert report['status'] == 'complete'
        assert json.loads(result_file.read_text(encoding='utf-8')) == report
        log = log_file.read_text(encoding='utf-8')
        assert log.startswith(header) and 'decode 1/16' in log and 'complete in' in log
        output = demo / 'demo.tif'
        assert output.stat().st_size > 100
        assert output.read_bytes()[:4] in (b'II*\0', b'MM\0*')
        original_digest = hashlib.sha256(output.read_bytes()).hexdigest()
        failure_file = root / 'failed-result.json'
        repeat = run_engine(engine, ['merge', '--fixtures', '--job', demo / 'job.json',
                                    '--result', failure_file, '--log', log_file], env)
        assert repeat.returncode != 0, 'Existing output must not be overwritten'
        failure = json.loads(failure_file.read_text(encoding='utf-8'))
        assert failure['status'] == 'error' and 'already exists' in failure['message']
        assert hashlib.sha256(output.read_bytes()).hexdigest() == original_digest
        assert log_file.read_text(encoding='utf-8').startswith(header)
        catalog_file = plugin / 'config' / 'camera-profiles.json'
        original_catalog = catalog_file.read_bytes()
        bad_catalog = json.loads(original_catalog)
        bad_catalog['schema_version'] = 999
        catalog_file.write_text(json.dumps(bad_catalog), encoding='utf-8')
        invalid = run_engine(engine, ['info'], env)
        assert invalid.returncode != 0, 'Invalid bundled config must not fall back to embedded defaults'
        catalog_file.write_bytes(original_catalog)
        print(f'Extracted {expected_platform} package passed engine info and sixteen-shot merge smoke tests')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument('--archive', type=Path)
    source.add_argument('--archive-dir', type=Path)
    parser.add_argument('--platform', required=True)
    parser.add_argument('--machine', help='Required native machine identifier(s), comma-separated')
    args = parser.parse_args()
    archive = args.archive
    if args.archive_dir:
        matches = sorted(args.archive_dir.glob(f'*-{args.platform}.zip'))
        if len(matches) != 1:
            parser.error('Expected exactly one archive for the requested platform')
        archive = matches[0]
    smoke_test(archive, args.platform, args.machine)


if __name__ == '__main__':
    main()
