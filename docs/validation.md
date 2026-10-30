# Validation requirements

## Automated gates

Run `cargo fmt --all -- --check`, `cargo test --workspace --locked`,
`cargo clippy --workspace --all-targets --locked -- -D warnings` and
`lua scripts/test-plugin.lua`. CI runs these on Linux, macOS and Windows.

Synthetic tests cover direct 4-shot RGB reconstruction, high-frequency half-pixel
16-shot detail (compared against an analytic scene at every output pixel),
noise variance reduction, translation recovery, moving-patch reference fallback,
invalid buffers/models/CFA/sequence/options, CLI ordering, TIFF decode/bit depth,
ICC presence, source camera/lens/exposure EXIF roundtrips, duplicate paths, version rejection and no-clobber publication.
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
and Windows. Verify SDK menu registration, bundled executable launch,
spaces/apostrophes/Unicode, Windows rejected command characters,
offline originals, existing output, failure exit, disk full, engine crash,
concurrent jobs and catalog import failure. Ensure only completed DNG/TIFF results are
imported, originals remain untouched, and temporary manifests are cleaned up.
Verify deterministic input-range filenames beside the sources, collision suffixes,
confirmation by catalog lookup after committing the write, result selection and
filtered/collection-hidden results. Preserve existing filters and active sources;
never claim import succeeded solely because the write callback ran.
Lua protocol tests are not a substitute for running the Lightroom SDK host.

## Linear DNG acceptance

Require Adobe DNG validation, a second DNG reader, retained scene-linear values,
correct camera matrices/WB, crop/orientation, previews and a successful Lightroom
import/develop round trip. A `.dng` filename or TIFF container alone is not a
valid DNG implementation.

Record Adaptive Color/Adaptive B&W profile eligibility on linear DNG separately
from adaptive presets, masking, AI Denoise and other Enhance operations. Require
host evidence for each claimed feature. Check camera/lens recognition, RAW lens
profile matching and built-in corrections after crop and sixteen-shot scaling.

The initial float32 DNG writer now has Rust/raw-ler numerical roundtrips and
independent tifffile/LibRaw checks for four-shot, sixteen-shot and denoise. Run:

```sh
python -m pip install rawpy==0.27.1 tifffile==2026.10.15
cargo build --locked -p pixelshift-cli
python scripts/test-dng.py --engine target/debug/sony-pixelshift
```

A local Adobe DNG SDK 1.6 validator completes on the synthetic sample without
warnings/errors. See [DNG representation and evidence](dng.md). Real-capture
Lightroom Adaptive Color/B&W and AI Denoise remain unverified host gates.

The Lua workflow mocks cooperatively schedule engine execution and progress
polling. Verify decode/merge/write updates occur before the helper returns and
that scopes close before dialogs on normal completion, launch/engine exceptions,
invalid results and catalog failures. Partial-log tests cover incomplete lines,
CRLF and unrelated diagnostics. In the Lightroom host, confirm top-left activity
appears during a real long merge, stays responsive and clears on success/failure;
mock scheduling alone does not establish host behavior.

Packaging also runs `test-plugin-load.lua` on the extracted ZIP with no source
package path. It loads both menu entry points and their dependencies under a
limited toolkit-style global environment, then constructs progress without
metatables. A negative check removes `PixelShiftProgress.lua` and requires the
loader gate to fail. Run package units with `PIXELSHIFT_TEST_LUA` pointing to Lua
5.1; CI installs Lua and sets this explicitly. This catches missing packaged
modules and unsupported global dependencies, not all Lightroom host behaviors.

## Streamed output gates

Compare whole-image fixture reference versus irregular regions, worker counts,
compression modes and strip sizes using exact float bits. Include localized
motion/fallback and partial edge regions. Validate compressed/uncompressed DNG in
independent readers and Adobe tooling, plus rendered TIFF roundtrips. Inject late
region/progress errors and confirm no final or temporary file remains.

Performance assertions are structural (maximum region size, no repeated preview
reconstruction); OS RSS/time comparisons are recorded benchmarks, not flaky CI
thresholds. See [streaming](streaming.md). Full-size source-memory and Lightroom
compressed-output acceptance remain required.
