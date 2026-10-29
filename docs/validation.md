# Validation requirements

## Automated gates

Run `cargo fmt --all -- --check`, `cargo test --workspace --locked`,
`cargo clippy --workspace --all-targets --locked -- -D warnings` and
`lua scripts/test-plugin.lua`. CI runs these on Linux, macOS and Windows.

Synthetic tests cover direct 4-shot RGB reconstruction, high-frequency half-pixel
16-shot detail (compared against an analytic scene at every output pixel),
noise variance reduction, translation recovery, moving-patch reference fallback,
invalid buffers/models/CFA/sequence/options, CLI ordering, TIFF decode/bit depth,
ICC presence, duplicate paths, version rejection and no-clobber publication.
Synthetic tests do not validate Sony decoder accuracy or real metadata.

## Required private capture corpus before claiming a7CR support is stable

Keep originals outside git. For each capture record SHA-256, firmware, lens,
compression, ISO, shutter/aperture, WB, exposure, sequence tags, orientation and
expected crop. Include complete 4-shot and 16-shot sequences at base/mid/high
ISO, uncompressed and each lossless-compression size supported by the camera.
Reject unsupported or resized sensor modes rather than silently altering CFA.
Include ordinary bursts with usable sequence tags, duplicates, missing frames,
reordered inputs, mismatched exposure, two cameras, truncated/corrupt ARWs and
non-Pixel-Shift files accidentally selected as a Pixel Shift sequence.

Compare:

1. Decode levels, CFA, dimensions and active area against DNGLab analyze and a
   second independent decoder. Confirm black and white normalization and as-shot
   WB, with no baked Lightroom edits.
2. Static chart / fine fabric: match Sony Imaging Edge output placement and
   color; inspect channel parity and false color at 100–400%. Verify 16-shot
   detail exceeds a simple 2x interpolation of the 4-shot output.
3. Gray patches and color chart: measure noise RMS and bias on linear data,
   clipping and color error. Target burst variance ≤1.25/N times the single-shot
   variance for a static repeated capture, bias <0.5% of full scale; account for
   reference interpolation and correlated sensor noise.
4. Moving leaves, people, water and imposed tripod drift: verify masks/fallback
   remove ghosts and preserve reference content. Measure translation error
   against known displacements; current integer search must recover exact
   in-range integer motion. Do not claim arbitrary motion correction.
5. Open RGB16 TIFFs in independent readers and Lightroom. Check ICC interpretation,
   orientation, crop, no unexpected color shift and no discarded highlights
   beyond the documented renderer clipping.
6. Record wall time and peak RSS for full 61 MP four-shot, 244 MP sixteen-shot and
   denoise jobs. Establish an agreed release memory/time budget, then implement
   tiles and cancellation; none is measured or promised by this prototype.

The installable alpha requires the native Sony PixelShiftInfo record for actual
Pixel Shift merges, validates declared shot count, and rejects mixed group IDs.
It prefers that record's source-shot index to the generic burst SequenceNumber.
Ordinary burst denoise can operate without Pixel Shift indices. Synthetic TIFF
fixtures test these fields in both byte orders; real a7CR firmware/compression
variants still need comparison with ExifTool and Imaging Edge.

## Lightroom host gate

Test current Lightroom Classic on macOS (Apple Silicon and Intel as available)
and Windows. Verify SDK menu registration, first-use executable choice, GUI PATH
for ExifTool, spaces/apostrophes/Unicode, Windows rejected command characters,
offline originals, existing output, failure exit, disk full, engine crash,
concurrent jobs and catalog import failure. Ensure only completed TIFFs are
imported, originals remain untouched, and temporary manifests are cleaned up.
Lua protocol tests are not a substitute for running the Lightroom SDK host.

## DNG next

Require Adobe DNG validation, a second DNG reader, retained scene-linear values,
correct camera matrices/WB, crop/orientation, previews and a successful Lightroom
import/develop round trip. A `.dng` filename or TIFF container alone is not a
valid DNG implementation.
