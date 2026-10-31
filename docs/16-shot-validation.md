# Full-size a7CR 16-shot validation

Use a tripod and a still, detailed scene with stable lighting. Include fine text
or fabric and neutral colors; f/5.6–f/8 at base ISO is a useful first test. Enable
16-shot Pixel Shift in camera and retain all 16 original full-resolution ARWs.
Keep a second moving/drift capture for later motion testing. Ordinary bursts
are not interchangeable with a Pixel Shift sequence.

## RAM/Disk parity and performance

Download [benchmark-capture.py](https://github.com/Niklas-labs/lightroom-pixel-shift-plugin/blob/main/scripts/benchmark-capture.py)
with GitHub's Download raw file button. Use the bundled helper from the current
CI plugin, keeping its config/manifest beside it. Put exactly one complete
16-shot set in `~/Desktop/PixelShift16`, then run:

```sh
uv run --with numpy==2.3.5,tifffile==2026.10.15 \
  ~/Downloads/benchmark-capture.py \
  --engine ~/Downloads/SonyPixelShift.lrplugin/bin/sony-pixelshift \
  --mode sixteen --out ~/Desktop/pixelshift16-benchmark \
  ~/Desktop/PixelShift16/*.ARW
```

Adjust paths/case to match your files. The output folder must be new or empty.
The CLI independently checks camera, group, count, order and exposure metadata.
A four-shot baseline uses `--mode four` with exactly four originals. Advanced
`--fixtures` is only for synthetic development input and is labeled in reports.

The script runs both storage modes sequentially with identical default alignment,
motion, compression and strip settings. It preserves `ram.dng`, `disk.dng`, local
jobs, results and engine logs. Reserve space for two DNGs plus Disk mode's packed
sensor scratch files. Sixteen 61 MP u16 sources alone are about 2 GB; lossless
output size depends on image content. Whole-frame decoding remains the memory
floor even in Disk mode.

After both engine processes exit, the script hashes decoded float samples a strip
at a time, plus the small previews, and checks crop/orientation parity. It does
not allocate a full decoded 244 MP image. Heavy reader imports and hash work
happen after the engine runs, so they do not inflate measured engine RSS. The
report includes per-stage time, engine peak RSS, scratch/resident buffers, input
checksums, CPU/platform information and package revision when available.

Share only `pixelshift16-benchmark/report.json`. Jobs/results/logs contain local
paths; original RAWs and output images remain local. The report's source hashes
identify the corpus without exposing file paths or serials. Cache state is
uncontrolled: the second run can benefit from OS caching. For a second pass,
choose another empty output folder and `--first-storage disk` to reverse order;
record storage/hardware and other load when interpreting performance.

## Quality and Lightroom acceptance

1. Merge the same originals with Sony Imaging Edge and import its ARQ into
   Lightroom alongside `ram.dng` (or merge through the plugin for host workflow
   acceptance). The benchmark itself does not import files into the catalog.
2. Verify visible output dimensions are 19008×12672 for this full-size a7CR mode,
   allowing only a documented valid-border crop. The DNG main buffer can be larger
   than the visible DefaultCrop. Compare orientation, framing and shot placement.
3. Compare Library immediately after import with Develop at As Shot/Exposure 0,
   under the same profile. The embedded preview is calibrated but does not include
   Adobe profile looks or subsequent Develop edits.
4. Inspect matching fine-detail crops at 100% and compare to Sony's 16-shot merge,
   a four-shot merge, and a single source at equivalent scene magnification. More
   pixels alone do not establish recovered detail. Look for color artifacts,
   softness, seams and ghosting; record fallback fraction and residual shifts.
5. Export matched 16-bit uncompressed sRGB TIFFs with corrections/sharpening/noise
   reduction off and matching crop/Transform, then use the existing comparison
   script. First establish correspondence before interpreting exposure statistics.
6. Confirm Adaptive Color/B&W and AI Denoise eligibility separately. Test lens
   corrections later with matching settings; do not mix correction-on/off exports.
7. Repeat RAM/Disk checks at another ISO and supported compression variant before
   expanding the calibration claims. Unknown decoder white scales are rejected
   rather than forced into the initial 14-bit reference convention.

Acceptance requires real capture results, matching RAM/Disk decoded output,
measured memory/time and demonstrable detail versus reference. A synthetic test,
identical storage hashes or a successful merge alone does not validate Sony image
quality, arbitrary scene motion or every Lightroom feature.
