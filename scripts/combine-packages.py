#!/usr/bin/env python3
"""Combine matching, already tested platform ZIPs without modifying their helpers."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import stat
import zipfile

PLUGIN = 'SonyPixelShift.lrplugin/'
PLATFORMS = {'macos-universal': 'bin/sony-pixelshift',
             'windows-x86_64': 'bin/sony-pixelshift.exe'}


def read_package(archive, platform):
    with zipfile.ZipFile(archive) as zipped:
        payload = {}
        for entry in zipped.infolist():
            path = PurePosixPath(entry.filename)
            mode = entry.external_attr >> 16
            if (path.is_absolute() or '..' in path.parts or '\\' in entry.filename
                    or ':' in entry.filename or stat.S_ISLNK(mode)
                    or entry.filename in payload):
                raise ValueError('Unsafe or duplicate archive entry')
            if not entry.is_dir():
                payload[entry.filename] = zipped.read(entry)
    manifest = json.loads(payload[PLUGIN + 'manifest.json'])
    if manifest['schema_version'] != 1 or manifest['platform'] != platform:
        raise ValueError('Expected a single-platform package')
    if manifest['engine'] != PLATFORMS[platform]:
        raise ValueError('Unexpected helper path')
    engine = payload[PLUGIN + manifest['engine']]
    if hashlib.sha256(engine).hexdigest() != manifest['engine_sha256']:
        raise ValueError('Helper checksum mismatch')
    engines = {p for p in payload if p.startswith(PLUGIN + 'bin/')}
    if engines != {PLUGIN + manifest['engine']}:
        raise ValueError('Unexpected bundled helper')
    return payload, manifest


def combine(macos, windows, output_dir):
    mac_files, mac = read_package(macos, 'macos-universal')
    win_files, win = read_package(windows, 'windows-x86_64')
    keys = ('protocol_version', 'engine_version', 'plugin_version',
            'source_revision', 'metadata_backend')
    if any(mac[key] != win[key] for key in keys):
        raise ValueError('Platform packages must use the same revision, versions and backend')
    version = mac['engine_version']
    if not version or any(c not in 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-_' for c in version):
        raise ValueError('Unsafe engine version')
    # Only accepted publisher builds may become the combined distribution.
    if not (mac['publisher_signed'] and mac['notarized']
            and mac['macos_signing'] == 'developer-id'
            and mac['notarization_submission_id']):
        raise ValueError('The macOS helper must be Developer ID signed and notarized')
    shared = lambda files: {p: data for p, data in files.items()
                            if p.startswith(PLUGIN) and p != PLUGIN + 'manifest.json'
                            and not p.startswith(PLUGIN + 'bin/')}
    mac_shared, win_shared = shared(mac_files), shared(win_files)
    differences = [p for p in sorted(mac_shared.keys() | win_shared.keys())
                   if mac_shared.get(p) != win_shared.get(p)]
    if differences:
        raise ValueError('Shared plugin files, configuration and notices differ: ' + ', '.join(differences))
    if mac_files.get('TESTING.md') != win_files.get('TESTING.md'):
        raise ValueError('Testing instructions differ')
    manifest = {'schema_version': 2, 'platform': 'cross-platform',
                **{key: mac[key] for key in keys},
                'engines': {platform: {key: source[key] for key in (
                    'engine', 'engine_sha256', 'distribution', 'publisher_signed',
                    'notarized', 'macos_signing', 'notarization_submission_id')}
                    for platform, source in [('macos-universal', mac), ('windows-x86_64', win)]}}
    files = shared(mac_files)
    for platform, source, payload in [('macos-universal', mac, mac_files),
                                      ('windows-x86_64', win, win_files)]:
        path = PLUGIN + source['engine']
        files[path] = payload[path]
    files[PLUGIN + 'manifest.json'] = (json.dumps(manifest, indent=2) + '\n').encode()
    files['TESTING.md'] = mac_files['TESTING.md']
    files['INSTALL.txt'] = (
        'Sony Pixel Shift: macOS (Apple Silicon / Intel) and Windows x86-64\n\n'
        'Extract once. Keep the complete SonyPixelShift.lrplugin folder in a permanent\n'
        'location. In Lightroom Classic: File > Plug-in Manager > Add.\n'
        'Both helpers are embedded; the plugin chooses the correct one automatically.\n'
        'No separate Rust, Python, Perl, ExifTool or engine installation is needed.\n'
        'For updates, quit Lightroom on every computer using this folder before replacing it.\n'
        'A shared/synced copy can serve both systems. Lightroom records plugin paths\n'
        'per computer; register the local path on each computer once.\n'
        'The macOS helper is Developer ID signed and notarized by Apple. First-run\n'
        'verification may need internet; ZIPs cannot carry a stapled ticket.\n'
        'The Windows helper is unsigned. The manifest records signing per platform.\n'
        'When copying through Windows or a sync service, preserve the macOS helper\n'
        'executable permission. If it is lost, re-extract the original ZIP on the Mac.\n'
        'See TESTING.md for use, shortcuts, troubleshooting and experimental limitations.\n'
    ).encode()
    output_dir.mkdir(parents=True, exist_ok=True)
    archive = output_dir / f"SonyPixelShift-{mac['engine_version']}-cross-platform.zip"
    with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=6) as zipped:
        for path, data in sorted(files.items()):
            entry = zipfile.ZipInfo(path, date_time=(2026, 1, 1, 0, 0, 0))
            entry.create_system = 3
            mode = 0o755 if path == PLUGIN + PLATFORMS['macos-universal'] else 0o644
            entry.external_attr = (stat.S_IFREG | mode) << 16
            entry.compress_type = zipfile.ZIP_DEFLATED
            zipped.writestr(entry, data)
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix('.zip.sha256').write_text(f'{checksum}  {archive.name}\n', encoding='ascii')
    if os.environ.get('GITHUB_STEP_SUMMARY'):
        with open(os.environ['GITHUB_STEP_SUMMARY'], 'a', encoding='utf-8') as report:
            report.write(f'### Recommended download\n\n**{archive.name}** works on macOS '
                         '(Apple Silicon and Intel) and Windows x86-64. Extract once; '
                         '`SonyPixelShift.lrplugin` is at the ZIP root.\n\n'
                         f'SHA-256: `{checksum}`\n')
    return archive


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--macos', required=True, type=Path)
    parser.add_argument('--windows', required=True, type=Path)
    parser.add_argument('--output-dir', type=Path, default=Path('dist'))
    args = parser.parse_args()
    print(combine(args.macos, args.windows, args.output_dir))


if __name__ == '__main__':
    main()
