#!/usr/bin/env python3
"""Local RAM/disk benchmark of one real Sony Pixel Shift capture.

The two engine runs finish before importing image readers. Hash DNG strips
sequentially afterwards; never allocate a complete decoded 244 MP image.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import time


def hash_dng(path):
    import numpy as np
    import tifffile
    digest = hashlib.sha256()
    total = peak = next_y = 0
    with tifffile.TiffFile(path) as tif:
        page = tif.pages[0]
        if int(page.photometric) != 34892 or page.dtype != np.dtype('float32') or page.shape != (page.imagelength,page.imagewidth,3) or page.is_tiled:
            raise ValueError('Expected strip-based LinearRaw RGB float32 DNG')
        for data,index,_ in page.segments(maxworkers=1,buffersize=8*1024*1024):
            if data is None or index != (0,0,next_y,0,0):
                raise ValueError('Unexpected DNG strip order or missing data')
            if data.shape[0] != 1 or data.shape[2:] != (page.imagewidth,3):
                raise ValueError('Unexpected DNG strip shape')
            next_y += data.shape[1]
            if next_y > page.imagelength:
                raise ValueError('Decoded strip extends outside image')
            payload = np.ascontiguousarray(data,dtype='<f4').tobytes()
            digest.update(payload)
            total += len(payload)
            peak = max(peak,len(payload))
        if total != page.imagewidth*page.imagelength*12 or next_y != page.imagelength:
            raise ValueError('Decoded DNG byte count mismatch')
        preview_hash = None
        if page.pages is not None:
            preview = page.pages[0]
            if max(preview.shape[:2]) > 512 or preview.dtype != np.dtype('uint8'):
                raise ValueError('Unexpected preview size/type')
            preview_hash = hashlib.sha256(preview.asarray().tobytes()).hexdigest()
        crop_origin = list(page.tags['DefaultCropOrigin'].value) if 'DefaultCropOrigin' in page.tags else [0,0]
        crop_size = list(page.tags['DefaultCropSize'].value) if 'DefaultCropSize' in page.tags else [page.imagewidth,page.imagelength]
        orientation = int(page.tags['Orientation'].value) if 'Orientation' in page.tags else 1
        return dict(dimensions=[page.imagewidth,page.imagelength],visible_dimensions=crop_size,crop_origin=crop_origin,orientation=orientation,decoded_bytes=total,
                    decoded_sha256=digest.hexdigest(),preview_sha256=preview_hash,
                    max_decoded_strip_bytes=peak)


def benchmark(args):
    inputs = [p.resolve() for p in args.inputs]
    required = 16 if args.mode == 'sixteen' else 4
    if len(inputs) != required or len(set(inputs)) != required or not all(p.is_file() for p in inputs):
        raise ValueError(f'Select exactly {required} distinct existing source files from one capture')
    root = args.out.resolve()
    if root.exists() and any(root.iterdir()):
        raise ValueError('Output directory must be new or empty; existing benchmark files are never overwritten')
    engine = args.engine.resolve()
    info = json.loads(subprocess.run([str(engine),'info'],check=True,capture_output=True,text=True).stdout)
    if not all(s in info.get('source_storages',[]) for s in ('ram','disk')):
        raise ValueError('Engine must support RAM and Disk source storage')
    root.mkdir(parents=True,exist_ok=True)
    manifest = engine.parent.parent/'manifest.json'
    revision = json.loads(manifest.read_text()).get('source_revision') if manifest.is_file() else None
    runs = []
    first = args.first_storage
    order = [first,'disk' if first == 'ram' else 'ram']
    for storage in order:
        output = root/f'{storage}.dng'
        job = root/f'{storage}-job.json'
        spec = dict(version=1,camera='auto',mode=args.mode,inputs=[str(p) for p in inputs],output=str(output),
                    motion_threshold=.08,alignment_radius=2,compression='lossless',strip_rows=args.strip_rows,
                    source_storage=storage,scratch_directory=str(root))
        with job.open('x') as stream:
            stream.write(json.dumps(spec,indent=2)+'\n')
        command = [str(engine),'merge','--job',str(job),'--result',str(root/f'{storage}-result.json')]
        if args.fixtures:
            command.append('--fixtures')
        env = dict(os.environ)
        if args.workers is not None:
            env['RAYON_NUM_THREADS'] = str(args.workers)
        print(f'Merging {args.mode} in {storage.upper()} mode...',flush=True)
        start = time.perf_counter()
        with (root/f'{storage}-engine.log').open('x') as log:
            run = subprocess.run(command,stdout=subprocess.PIPE,stderr=log,text=True,env=env)
        if run.returncode:
            raise ValueError(f'{storage} engine failed (exit {run.returncode}); inspect {root/f"{storage}-engine.log"}')
        wall = time.perf_counter()-start
        result = json.loads(run.stdout)
        if result.get('status') != 'complete' or result.get('output') != str(output):
            raise ValueError('Unexpected engine result')
        runs.append(dict(storage=storage,wall_seconds=wall,execution=result['execution'],merge_report=result['report']))
        rss = result['execution']['resources']['peak_resident_bytes']
        print(f'{storage}: {wall:.2f}s wall, engine peak RSS {rss/1048576:.1f} MiB',flush=True)
    # Heavy reader imports/decoded buffers must not inflate a later engine's
    # inherited RSS high-water mark. Both engine processes have exited here.
    for run in runs:
        print(f'Hashing {run["storage"]}.dng one strip at a time...',flush=True)
        start = time.perf_counter()
        run['dng'] = hash_dng(root/f'{run["storage"]}.dng')
        run['hash_seconds'] = time.perf_counter()-start
    equal = all(runs[0]['dng'][key] == runs[1]['dng'][key]
                for key in ('dimensions','visible_dimensions','crop_origin','orientation','decoded_sha256','preview_sha256'))
    input_hashes = []
    for path in inputs:
        digest = hashlib.sha256()
        with path.open('rb') as stream:
            for block in iter(lambda: stream.read(8*1024*1024),b''):
                digest.update(block)
        input_hashes.append(digest.hexdigest())
    report = dict(version=1,engine_version=info['engine_version'],source_revision=revision,
                  capture_type='synthetic fixtures' if args.fixtures else 'real original RAW capture',
                  mode=args.mode,source_count=len(inputs),input_sha256=input_hashes,platform=platform.platform(),machine=platform.machine(),
                  workers_request=args.workers,worker_environment=os.environ.get('RAYON_NUM_THREADS'),
                  cpu_count=os.cpu_count(),run_order=order,cache_state='uncontrolled; later run may benefit from filesystem cache',
                  settings=dict(motion_threshold=.08,alignment_radius=2,strip_rows=args.strip_rows,compression='lossless'),
                  decoded_outputs_identical=equal,runs=runs,
                  scope='RAM/disk numerical parity and engine performance only; not a sharpness, motion, Sony-reference or Lightroom acceptance test.')
    with (root/'report.json').open('x') as stream:
        stream.write(json.dumps(report,indent=2)+'\n')
    if not equal:
        raise ValueError('RAM and Disk decoded outputs differ; inspect report.json')
    print(f'RAM/Disk samples and previews match exactly. Share {root/"report.json"}.',flush=True)
    print('Jobs and logs contain local paths; the summary omits input paths and camera serials.',flush=True)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine',type=Path,required=True)
    parser.add_argument('--mode',choices=['four','sixteen'],default='sixteen')
    parser.add_argument('--out',type=Path,required=True)
    parser.add_argument('--first-storage',choices=['ram','disk'],default='ram')
    parser.add_argument('--workers',type=int)
    parser.add_argument('--strip-rows',type=int,default=32)
    parser.add_argument('--fixtures',action='store_true',help='Development synthetic JSON sources only')
    parser.add_argument('inputs',type=Path,nargs='+',help='All original ARWs from one complete capture')
    args = parser.parse_args()
    if args.workers is not None and args.workers < 1:
        parser.error('--workers must be positive')
    if not 1 <= args.strip_rows <= 256:
        parser.error('--strip-rows must be 1..256')
    try:
        benchmark(args)
    except (ValueError,OSError,subprocess.CalledProcessError) as exc:
        parser.exit(2,f'Benchmark failed: {exc}\n')


if __name__ == '__main__':
    main()
