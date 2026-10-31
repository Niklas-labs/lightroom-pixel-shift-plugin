# Compare Imaging Edge and plugin brightness locally

Use the same source ARWs to create an Imaging Edge ARQ and a plugin DNG. Import
both into Lightroom, then export from Lightroom so the rendering application is
held constant. This tool compares rendered images; it does not develop RAWs,
align images, or infer an underlying RAW exposure correction.

## Prepare two Lightroom exports

Use virtual copies if you want to preserve your current edits.

- Adobe Color on both. Auto off, Exposure 0, Contrast/Highlights/Shadows/Whites/
  Blacks 0. Use the same remaining tone and color settings, including tone curve.
- For the first pair, use **5850 K / +15 Tint on both** (or another matching
  numeric value). This controls the displayed WB parameters; it does not prove
  identical camera gains/calibration. For a second pair, use As Shot on both.
  Comparing both pairs helps identify WB-dependent differences.
- Turn off lens corrections, chromatic-aberration removal, sharpening, noise
  reduction, AI adjustments, masks, grain, and local edits on both.
- Use matching full-image crops. Turn off resize, output sharpening, watermark,
  and export presets that alter development.
- Export **TIFF, 16 bits/component, sRGB, Compression: None** at full resolution.
  Name them `Sony.tif` (from ARQ) and `Plugin.tif` (from DNG). Leave embedded color
  profiles enabled. Resolution in pixels/inch does not affect the comparison.

If full-image dimensions differ, stop and record both sizes before changing
crops. The tool deliberately rejects mismatched dimensions rather than resampling
one image and disguising a geometry problem. Small crop/registration differences
can still affect equal-size exports: use the preview to check corresponding
features. Smooth-region masking reduces edge sensitivity, not alignment errors.

## Run on macOS

Requires Python 3.11 or later. No Rust or ExifTool installation is needed.
Download [compare-lightroom.py](https://github.com/Niklas-labs/lightroom-pixel-shift-plugin/blob/main/scripts/compare-lightroom.py)
using GitHub's **Download raw file** button while signed in, and save it beside
the TIFFs. In Terminal, change to that folder, then run:

```sh
python3 -m venv .comparison-venv
.comparison-venv/bin/python -m pip install numpy==2.3.5 tifffile==2026.10.15 Pillow==12.3.0
.comparison-venv/bin/python compare-lightroom.py Sony.tif Plugin.tif --out comparison-matched-wb
```

From a local checkout, use
`scripts/compare-lightroom.py` and install dependencies with
`python -m pip install -r scripts/requirements-comparison.txt`.

Optionally include `--dng "original-plugin-output.dng"` to record the plugin
DNG's calibration tags. This reads the TIFF directory, not its full pixel data.
It excludes private MakerNotes, GPS and XMP. No file is uploaded by the tool.

Repeat for As Shot exports, using `--out comparison-as-shot`, if convenient.
Record the Lightroom version, the profile and WB settings alongside the report.

## Share only the small results

Send `report.json` and `comparison.png` from the output folder. The TIFFs and RAWs
stay local. The preview contains scene content, and the report contains export
basenames, so review them before sharing.

Negative EV means the plugin export is darker. For example, -0.1 EV means about
0.933 times the reference's rendered linear-light brightness. The script undoes
the sRGB transfer curve before computing ratios; it avoids clipped/deep-shadow
samples and prioritizes smooth neighborhoods. It reports median and percentile
ratios, RGB-channel ratios, shadow/midtone/highlight bands, and center/outer areas.
These are differences **after Adobe rendering**, not a recommendation to apply
that exposure correction to RAW data.

The preview shows Sony, plugin, and a difference map: red is plugin darker, blue
is brighter, white is equal, and gray is excluded. Full color represents at least
0.5 EV. Images use a uniform sampling grid of at most 1024 pixels on the longest
axis, not a full-resolution difference scan. Uncompressed TIFFs are memory-mapped;
only sampled arrays are allocated. The tool refuses other ICC profiles or
compressed TIFFs instead of silently using an incorrect transfer curve or loading
large images in RAM.

A nearly constant EV difference across tone bands suggests a consistent rendered
brightness scale. Channel-dependent differences suggest color/WB interpretation;
edge-dependent differences can involve corrections or geometry. Neither result
alone identifies a faulty DNG tag: real decoder levels, camera calibration,
profiles and Adobe default rendering still require separate verification.
