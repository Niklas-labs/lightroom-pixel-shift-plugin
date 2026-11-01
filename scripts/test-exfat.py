#!/usr/bin/env python3
"""Exercise real macOS exFAT output publication with tiny synthetic captures."""
import argparse
import json
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def disk_command(args):
    result = subprocess.run(args, capture_output=True, text=True, timeout=120)
    if result.returncode:
        raise RuntimeError(f'Disk-image command failed: {args!r}\n{result.stdout}\n{result.stderr}')
    return result


def run_engine(engine, spec, job):
    job.write_text(json.dumps(spec), encoding='utf-8')
    result = subprocess.run([str(engine), 'merge', '--fixtures', '--job', str(job)],
                            capture_output=True, text=True, timeout=120)
    if result.returncode:
        raise AssertionError(f'exFAT fixture merge failed:\n{result.stderr}\n{result.stdout}')
    return json.loads(result.stdout)


def check(engine):
    if sys.platform != 'darwin':
        raise RuntimeError('This integration check requires macOS and hdiutil')
    with tempfile.TemporaryDirectory(prefix='pixelshift exFAT test ') as directory:
        root = Path(directory)
        fixtures = root / 'fixtures'
        subprocess.run([sys.executable, str(ROOT / 'scripts/generate-demo.py'), str(fixtures)],
                       check=True, capture_output=True)
        original = json.loads((fixtures / 'job.json').read_text())
        original['inputs'] = [str(fixtures / name) for name in original['inputs']]
        image = root / 'test.dmg'
        mount = root / 'exFAT volume'
        mount.mkdir()
        # Avoid a default partition layout consuming the small test image, and
        # leave enough space for exFAT's minimum formatting requirements.
        disk_command(['hdiutil', 'create', '-size', '512m', '-layout', 'NONE',
                      '-fs', 'ExFAT', '-volname', 'PixelShiftExFAT', str(image)])
        attached = False
        try:
            disk_command(['hdiutil', 'attach', '-nobrowse', '-mountpoint', str(mount), str(image)])
            attached = True
            info = plistlib.loads(subprocess.check_output(['diskutil', 'info', '-plist', str(mount)]))
            assert str(info.get('FilesystemType', '')).lower() == 'exfat', info
            for mode in ('four', 'sixteen'):
                for extension in ('dng', 'tif'):
                    for storage in ('ram', 'disk'):
                        name = f'{mode}-{extension}-{storage}'
                        target = mount / name
                        target.mkdir()
                        scratch = target / 'scratch'
                        scratch.mkdir()
                        inputs = original['inputs'] if mode == 'sixteen' else original['inputs'][:4]
                        spec = dict(original, inputs=inputs, mode=mode, source_storage=storage,
                                    save_diagnostics=True, strip_rows=7,
                                    scratch_directory=str(scratch), output=str(target / f'result.{extension}'))
                        report = run_engine(engine, spec, target / 'job.json')
                        publication = report['execution']['output_publication']
                        assert publication in ('native_no_clobber', 'exclusive_copy'), publication
                        output = Path(spec['output'])
                        expected = output.read_bytes()
                        assert expected and not list(scratch.iterdir())
                        for key in ('mask', 'overlay', 'summary'):
                            assert Path(report['merge_diagnostics'][key]).is_file()
                        # A failed rerun must preserve the complete original result.
                        retry = subprocess.run([str(engine), 'merge', '--fixtures', '--job', str(target / 'job.json')],
                                               capture_output=True, text=True, timeout=120)
                        assert retry.returncode != 0
                        assert output.read_bytes() == expected
                        # Compare the full encoded image with the normal local filesystem.
                        local = root / name
                        local.mkdir()
                        local_scratch = local / 'scratch'
                        local_scratch.mkdir()
                        baseline = dict(spec, save_diagnostics=False, scratch_directory=str(local_scratch),
                                        output=str(local / f'result.{extension}'))
                        run_engine(engine, baseline, local / 'job.json')
                        assert Path(baseline['output']).read_bytes() == expected
                        print(f'exFAT {mode}/{extension}/{storage}: {publication}; diagnostics, exact bytes, no overwrite and cleanup passed', flush=True)
        finally:
            if attached or mount.is_mount():
                disk_command(['hdiutil', 'detach', str(mount)])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', type=Path, required=True)
    check(parser.parse_args().engine.resolve())
