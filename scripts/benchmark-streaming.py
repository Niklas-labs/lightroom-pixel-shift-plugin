#!/usr/bin/env python3
"""Synthetic process-level streaming baseline; not a real-camera quality benchmark."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import subprocess
import tempfile
import time


def benchmark(args, root):
    root.mkdir(parents=True, exist_ok=True)
    if any(root.iterdir()): raise ValueError('Benchmark work directory must be empty')
    count = 16 if args.mode == 'sixteen' else 4
    inputs = []
    noise = [((i * 1664525 + 1013904223) % 65536 / 65536 - 0.5) * 0.006 for i in range(4096)]
    for shot in range(count):
        integer = [(1,0),(1,1),(0,1),(0,0)][shot % 4]
        half = [(0.5,0),(0,0),(0.5,0.5),(0,0.5)][shot // 4] if count == 16 else (0,0)
        sx,sy = (0,0) if args.mode == 'denoise' else (integer[0]+half[0],integer[1]+half[1])
        xs = [0.07 * math.sin((x+sx)*0.15) for x in range(args.width)]
        ys = [0.04 * math.cos((y+sy)*0.11) for y in range(args.height)]
        pixels = [round(0.25+0.05*[0,1,1,2][(y%2)*2+x%2]+xs[x]+ys[y]+noise[(x+y*47+shot*101)%4096],6)
                  for y in range(args.height) for x in range(args.width)]
        path=root / f'frame-{shot+1:02}.json'
        path.write_text(json.dumps({'frame':{'width':args.width,'height':args.height,'pixels':pixels,
            'cfa':[0,1,1,2],'model':'ILCE-7CR','sequence':shot+1},'color':{'white_balance':[2,1,1.5],
            'xyz_to_camera':[0.82,-0.2976,-0.0719,-0.4296,1.2053,0.2532,-0.0429,0.1282,0.5774],
            'xyz_to_camera_a':[0.9185,-0.4857,0.0505,-0.3651,1.1061,0.2982,-0.0161,0.0698,0.6769]}},separators=(',',':')))
        inputs.append(path.name)
    del pixels
    import gc
    gc.collect()
    reports=[]
    outputs=[]
    for label, reference, compression in [('reference-none',True,'none'),('streamed-none',False,'none'),('streamed-lossless',False,'lossless')]:
        output=root / f'{label}.dng'
        if output.exists(): raise ValueError(f'Benchmark output exists: {output}')
        spec={'version':1,'camera':'auto','mode':args.mode,'inputs':inputs,'output':output.name,
              'alignment_radius':0,'motion_threshold':0,'compression':compression,'strip_rows':args.strip_rows}
        job=root / f'{label}-job.json';job.write_text(json.dumps(spec))
        command=[str(args.engine.resolve()),'merge','--fixtures','--job',str(job)]
        if reference: command.append('--reference')
        started=time.perf_counter()
        result=subprocess.run(command,check=True,capture_output=True,text=True,
            env=dict(os.environ,RAYON_NUM_THREADS=str(args.workers)))
        wall=time.perf_counter()-started
        (root / f'{label}.log').write_text(result.stderr)
        report=json.loads(result.stdout)
        outputs.append(output)
        reports.append({'variant':label,'wall_seconds':wall,'execution':report['execution']})
        resources=report['execution']['resources']
        print(f"{label}: {wall:.3f}s wall; peak RSS {resources['peak_resident_bytes']/1048576:.1f} MiB; output {output.stat().st_size/1048576:.1f} MiB",flush=True)
    # Decode only after every engine process has exited. Importing NumPy and
    # holding decoded images in the launching Python process can inflate a
    # fork/exec child's inherited RSS high-water mark on some operating systems.
    import tifffile
    expected=None
    for output,report in zip(outputs,reports):
        with tifffile.TiffFile(output) as dng:
            digest=hashlib.sha256(dng.pages[0].asarray().tobytes()).hexdigest()
        if expected is None: expected=digest
        assert digest==expected,'Streaming/compression changed decoded sample bits'
        report['decoded_sha256']=digest
    summary={'fixture':'synthetic textured/noisy Bayer; six decimal input samples','width':args.width,'height':args.height,
             'mode':args.mode,'workers':args.workers,'strip_rows':args.strip_rows,'platform':platform.platform(),
             'machine':platform.machine(),'cache_state':'uncontrolled; generated fixtures may be cached','runs':reports}
    if args.report:
        args.report.write_text(json.dumps(summary,indent=2)+'\n')
    return summary


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine',type=Path,required=True)
    parser.add_argument('--width',type=int,default=512)
    parser.add_argument('--height',type=int,default=384)
    parser.add_argument('--mode',choices=['four','sixteen','denoise'],default='sixteen')
    parser.add_argument('--workers',type=int,default=4)
    parser.add_argument('--strip-rows',type=int,default=32)
    parser.add_argument('--work-dir',type=Path)
    parser.add_argument('--report',type=Path)
    args=parser.parse_args()
    if min(args.width,args.height)<16 or args.workers<1: parser.error('Dimensions must be >=16 and workers positive')
    if args.work_dir: benchmark(args,args.work_dir.resolve())
    else:
        with tempfile.TemporaryDirectory(prefix='pixelshift streaming baseline ') as temporary:
            benchmark(args,Path(temporary))
