#!/usr/bin/env python3
"""Assemble a self-contained Lightroom plugin and a permission-preserving ZIP."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[1]
PLUGIN_NAME = 'SonyPixelShift.lrplugin'
PLATFORMS = {'macos-universal': 'sony-pixelshift', 'windows-x86_64': 'sony-pixelshift.exe'}


def read_engine_info(binary):
    result = subprocess.run([str(binary.resolve()), 'info'], check=True, capture_output=True, text=True, encoding='utf-8')
    info = json.loads(result.stdout)
    if info.get('protocol_version') != 1 or not isinstance(info.get('engine_version'), str):
        raise ValueError('Engine does not support packaging protocol 1')
    backend = info.get('metadata_backend', '')
    if not any(word in backend.lower() for word in ('native', 'rust')):
        raise ValueError('This package requires a native metadata backend; external ExifTool is not bundled')
    return info


def cargo_metadata():
    result = subprocess.run(['cargo', 'metadata', '--locked', '--format-version', '1'],
                            cwd=ROOT, check=True, capture_output=True, text=True, encoding='utf-8')
    return json.loads(result.stdout)


def copy_notices(destination, metadata):
    """Carry dependency declarations and upstream root license/notice files."""
    destination.mkdir(parents=True)
    shutil.copyfile(ROOT / 'LICENSE', destination / 'PROJECT-GPL-3.0.txt')
    packages = []
    # Only resolved packages are included; retain build-time licenses as well.
    resolved = {node['id'] for node in metadata.get('resolve', {}).get('nodes', [])}
    for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
        if resolved and package['id'] not in resolved:
            continue
        packages.append({key: package.get(key) for key in ('name', 'version', 'license', 'license_file', 'repository', 'source')})
        package_root = Path(package['manifest_path']).parent
        notice_dir = destination / 'dependencies' / f"{package['name']}-{package['version']}"
        candidates = [p for p in package_root.iterdir()
                      if p.name.lower().startswith(('license', 'copying', 'notice', 'copyright'))]
        if package.get('license_file'):
            candidates.append(package_root / package['license_file'])
        for candidate in candidates:
            if not candidate.exists():
                continue
            notice_dir.mkdir(parents=True, exist_ok=True)
            if candidate.is_file():
                shutil.copyfile(candidate, notice_dir / candidate.name)
            elif candidate.is_dir():
                shutil.copytree(candidate, notice_dir / candidate.name, dirs_exist_ok=True)
    (destination / 'dependency-inventory.json').write_text(json.dumps(packages, indent=2) + '\n', encoding='utf-8')
    (destination / 'README.txt').write_text(
        'Sony Pixel Shift is GPL-3.0-or-later. Upstream dependency declarations and\n'
        'available license/notice files are included here, including rawler LGPL.\n'
        'Corresponding project source and locked dependency versions are available at:\n'
        'https://github.com/Niklas-labs/lightroom-pixel-shift-plugin\n'
        'The package manifest records the exact source revision used for this build.\n', encoding='utf-8')


def build_package(binary, platform, output_dir, revision, metadata, info=None):
    if platform not in PLATFORMS:
        raise ValueError('Unsupported package platform')
    if not revision or any(c not in '0123456789abcdef' for c in revision.lower()) or len(revision) != 40:
        raise ValueError('revision must be the full 40-character Git commit SHA')
    info = info or read_engine_info(binary)
    version = info['engine_version']
    if not version or any(c not in 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-_' for c in version):
        raise ValueError('Invalid engine version')
    output_dir.mkdir(parents=True, exist_ok=True)
    stage = output_dir / platform
    if stage.exists():
        shutil.rmtree(stage)
    plugin = stage / PLUGIN_NAME
    plugin.mkdir(parents=True)
    for source in sorted((ROOT / 'lightroom' / PLUGIN_NAME).glob('*.lua')):
        shutil.copyfile(source, plugin / source.name)
    shutil.copytree(ROOT / 'config', plugin / 'config')
    engine = plugin / 'bin' / PLATFORMS[platform]
    engine.parent.mkdir()
    shutil.copyfile(binary, engine)
    if platform.startswith('macos'):
        engine.chmod(0o755)
    else:
        engine.chmod(0o644)
    manifest = {
        'schema_version': 1,
        'protocol_version': info['protocol_version'],
        'engine_version': version,
        'plugin_version': version,
        'platform': platform,
        'source_revision': revision,
        'metadata_backend': info['metadata_backend'],
        'engine': 'bin/' + PLATFORMS[platform],
        'engine_sha256': hashlib.sha256(engine.read_bytes()).hexdigest(),
        'distribution': 'development',
        'notarized': False,
    }
    (plugin / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n', encoding='utf-8')
    copy_notices(plugin / 'licenses', metadata)
    testing = ROOT / 'docs' / 'lightroom-testing.md'
    if testing.is_file():
        shutil.copyfile(testing, stage / 'TESTING.md')
    (stage / 'INSTALL.txt').write_text(
        f'Sony Pixel Shift {version} ({platform})\n\n'
        'Extract this ZIP completely, keep SonyPixelShift.lrplugin in a permanent\n'
        'location, then add that directory in Lightroom Classic: File > Plug-in\n'
        'Manager > Add. Select exactly 4 or 16 original a7CR Pixel Shift ARWs and\n'
        'use Library > Plug-in Extras > Pixel Shift: more detail (4 or 16 RAWs).\n\n'
        'No separate Rust, Python, Perl, ExifTool, or engine installation is needed.\n'
        'See TESTING.md for installation, troubleshooting, and diagnostic reporting.\n'
        'This development build is not notarized or publisher-signed. macOS may\n'
        'require approving the downloaded helper in Privacy & Security.\n'
        'For a trusted build, TESTING.md documents a manual per-helper xattr fix.\n'
        'The plugin never removes quarantine automatically.\n'
        'The camera profile and motion correction remain experimental. Output is\n'
        'linear DNG by default; TIFF remains available through the CLI.\n'
        'Adobe Adaptive profiles and AI Denoise need Lightroom host verification.\n'
        'Bounded-memory processing remains planned separately.\n'
        'Keep original RAWs. A 16-shot capture can currently use 7-10 GB RAM.\n', encoding='utf-8')
    archive = output_dir / f'SonyPixelShift-{version}-{platform}.zip'
    with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=6) as zipped:
        for path in sorted(stage.rglob('*')):
            if not path.is_file():
                continue
            name = path.relative_to(stage).as_posix()
            entry = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
            entry.create_system = 3
            mode = 0o755 if path == engine and platform.startswith('macos') else 0o644
            entry.external_attr = (stat.S_IFREG | mode) << 16
            entry.compress_type = zipfile.ZIP_DEFLATED
            zipped.writestr(entry, path.read_bytes())
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix('.zip.sha256').write_text(f'{checksum}  {archive.name}\n', encoding='ascii')
    summary = os.environ.get('GITHUB_STEP_SUMMARY')
    if summary:
        with open(summary, 'a', encoding='utf-8') as report:
            report.write(f'### {platform} plugin\n\n'
                         f'Download **{archive.name}** from this run’s artifacts and extract once. '
                         'Add `SonyPixelShift.lrplugin` in Lightroom Classic’s Plug-in Manager.\n\n'
                         f'SHA-256: `{checksum}`\n\n')
    return archive


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--platform', required=True, choices=PLATFORMS)
    parser.add_argument('--output-dir', type=Path, default=ROOT / 'dist')
    parser.add_argument('--revision', required=True)
    parser.add_argument('--metadata', type=Path, help='Previously captured cargo metadata JSON')
    args = parser.parse_args()
    metadata = json.loads(args.metadata.read_text(encoding='utf-8')) if args.metadata else cargo_metadata()
    print(build_package(args.binary, args.platform, args.output_dir, args.revision, metadata))


if __name__ == '__main__':
    main()
