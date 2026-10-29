# Roadmap

The first installable alpha supports Sony a7CR four-shot and sixteen-shot Pixel
Shift, RAW burst denoise, and TIFF output. This roadmap tracks follow-ups; items
are complete only when their acceptance criteria have been verified.

## Delivered

- [x] Rust workspace and CLI with synthetic reconstruction tests.
- [x] Native Rust TIFF/EXIF parsing through rawler, with Sony sequence and
  PixelShiftInfo handling; no installed ExifTool runtime required.
- [x] Lightroom plugin with bundled-engine discovery, version/protocol checks,
  diagnostic logs, structured results and guarded TIFF import.
- [x] CI artifacts for macOS universal (Apple Silicon + Intel) and Windows x64,
  including checksums, dependency notices and installation instructions.
- [x] Native package smoke tests on Apple Silicon, Intel and Windows, including
  metadata inspection, merge, no-overwrite and launcher protocol checks.
- [x] Automatic camera/mode detection for the registered a7CR profile, with
  metadata preflight of count, group, shot indices, dimensions and exposure.
- [x] Optimized release builds with thin LTO and Rayon-parallel reconstruction.

These checks do not establish real-camera image quality or Lightroom SDK host
acceptance. The alpha still holds whole frames and the output in memory.

## 1. Validate the alpha with real captures

- [ ] Test installation, engine launch, merge and catalog import in Lightroom
  Classic on supported macOS and Windows systems.
- [ ] Collect reference a7CR four-shot, sixteen-shot and ordinary burst sets,
  covering supported compression variants, high ISO, orientation, fine detail,
  camera drift and moving subjects. Keep original captures outside git; record
  fixture metadata and checksums.
- [ ] Compare shot placement, crop, color and detail against Sony Imaging Edge;
  compare native metadata extraction against ExifTool as a development reference.
- [ ] Exercise incomplete sets, duplicate shots, mixed capture groups/cameras,
  mismatched exposures and corrupt files, with useful diagnostics.

Acceptance: repeatable Lightroom host results and documented camera/firmware/RAW
mode coverage. Synthetic fixtures alone must not mark a camera profile stable.

## 2. Auto-detect capture capabilities and generalize camera support

Use selected image count, dimensions and extension as hints to simplify the UI.
Confirm the interpretation from decoded metadata before choosing a reconstruction
layout. Four ARWs of the same size can also be an ordinary burst; sixteen selected
files can be incomplete parts of different captures.

- [x] Inspect the actual RAW container and make/model, rather than identifying a
  camera solely from `.arw` or trusting a renamed extension.
- [x] Read declared Pixel Shift shot count, source-shot index and group identity;
  derive four-shot/sixteen-shot mode and order from those records. Check that the
  selection is exactly one complete compatible capture. Keep ordinary burst
  denoise an explicit intent rather than automatically converting a failed Pixel
  Shift selection into denoise.
- [ ] Obtain sensor dimensions, active area/crop, CFA pattern/origin, bit depth,
  black/white levels, WB and camera matrices from decoder metadata/calibration.
- [ ] Replace hardcoded a7CR checks in the CLI/decoder with registered camera
  capabilities. Reuse verified shift layouts across models where appropriate.
- [ ] Keep commanded shift direction/order, mode availability, metadata quirks
  and supported RAW variants in explicit capabilities where they cannot be
  reliably derived. Dimensions cannot establish these properties or color/noise
  calibration by themselves.
- [ ] Add other Sony models incrementally with reference captures and acceptance
  tests. Leave seams for other pixel-shift formats without implementing them yet.
- [ ] Consider batch grouping of larger selections later; group by capture
  metadata rather than splitting every four/sixteen consecutive filenames.

Acceptance: automatic selection for verified captures, actionable rejection for
ambiguous/unsupported inputs, and no silent use of the a7CR layout for an unknown
camera. Keep manual camera configuration out of the normal supported workflow.

## 3. Measure performance before tuning

- [ ] Record decode, guide generation, alignment, reconstruction, rendering,
  compression and writing time separately, alongside peak memory and disk I/O.
- [ ] Establish full-size four-shot/sixteen-shot baselines on Apple Silicon,
  Intel and Windows hardware. Report hardware, storage and cold/warm cache state.
- [ ] Profile repeated interpolation, allocations and per-pixel bookkeeping.
- [ ] Measure CPU utilization and memory-bandwidth limits before parallelizing
  more stages. Coordinate decoder, Rayon and codec workers to avoid oversubscription.

Acceptance: reproducible stage-level benchmarks and a measured memory/time
tradeoff. CI runner timing is a regression signal, not a product speed promise.

## 4. Tile processing and offer memory/performance preferences

- [ ] Introduce region-source and streaming-output interfaces, retaining the
  current small-image merger as a numerical reference during migration.
- [ ] Decode each ARW once into packed `u16` sensor storage, with interchangeable
  RAM and disk stores. Normalize only the regions currently being processed.
- [ ] Build small alignment guides during ingestion; release whole-frame decoder
  buffers after storing each source.
- [ ] Reconstruct overlapping output tiles with halos derived from the transform,
  interpolation kernel and motion-mask neighborhood. Anchor coordinates globally
  so tile boundaries do not change results.
- [ ] Stream TIFF output without allocating the full reconstructed RGB image.
- [ ] Bound source caches, active tiles, codec workspace and pending writes under
  one memory budget, with writer backpressure.
- [ ] Expose **Auto**, **Low memory** and **High performance** execution preferences,
  plus optional scratch location and an advanced memory budget. Keep image-quality
  controls separate: changing storage preference must preserve numerical results.
- [ ] Estimate temporary-space requirements, handle disk-full conditions, clean
  failed/cancelled jobs and recover abandoned temporary files after crashes.

Initial targets to validate: approximately 512 MiB–1 GiB peak for disk-backed
processing and 3–4 GiB for RAM-backed sixteen-shot processing. These are targets,
not current guarantees. Sixteen packed 61 MP sources occupy about 1.95 GB decimal;
neither mode should retain the full output in RAM. Whole-frame rawler decoding
remains the initial memory floor. Application-managed disk caching should avoid
depending on OS swap; measure mapped-file residency and decoder working memory.

Acceptance: matching decoded results across storage modes, tile sizes and worker
counts within an explicit numerical tolerance; no seams; measured peak memory
and performance. Retain fixed accumulation order where practical.

## 5. Make linear DNG the preferred Lightroom output

- [ ] Prove a streaming writer using rawler's lower-level TIFF/LJPEG facilities
  or an upstream extension. Its existing high-level RGB/DNG API requires a whole
  image and is not a solution to the memory requirement.
- [ ] Write scene-linear camera RGB with calibration, illuminants, as-shot neutral,
  black/white semantics, crop, orientation, provenance and a small preview.
  Do not bake WB, sRGB conversion, gamma or tone mapping into the RAW image.
- [ ] Establish sample scaling and precision that preserve relevant headroom and
  negative black noise. Prefer compatible 16-bit storage if justified; evaluate
  floating-point DNG if the requirements cannot be met. Lossless compression does
  not remove integer quantization.
- [ ] Start with uncompressed tiles, then add bounded parallel lossless tile
  compression. Avoid embedding all source ARWs by default.
- [ ] Validate with Adobe DNG tooling, an independent reader and Lightroom import/
  development, including WB, exposure recovery, color, crop and orientation.
- [ ] Retain TIFF as an alternative and switch the default only after validation.

Acceptance: valid, interoperable linear DNG with tested sample/calibration
roundtrips and bounded memory. A renamed TIFF is not DNG support.

## 6. Improve motion correction and reconstruction together

- [ ] Estimate subpixel global translation on alias-suppressed guide pyramids;
  evaluate affine correction only if real captures justify it.
- [ ] Replace exact-grid sample selection with CFA-aware reconstruction of warped
  sensor observations. Fractional alignment alone would currently discard samples.
- [ ] Add local residual motion on a coarse globally anchored block grid, with
  texture, residual and matching-consistency confidence.
- [ ] Use noise-aware rejection, spatial mask cleanup, edge dilation and controlled
  feathering, consistent across processing tiles.
- [ ] Fall back to reference content where correspondence is unreliable; moving
  sixteen-shot regions must not be presented as recovered super-resolution.

Acceptance: controlled fractional-shift recovery, preserved static detail,
reduced ghosts and no tile seams across motion/occlusion/high-ISO fixtures.
Defer dense full-image optical flow, GPU/learned methods and rolling-shutter
models until evidence justifies their complexity. Consider SIMD/runtime CPU
dispatch only after profiling, while keeping distributed binaries compatible.

## 7. Finish the Lightroom experience and distribution

- [ ] Add stage progress, cancellation and clear completion/failure reporting.
  Account for cancellation latency during a whole-frame upstream decode.
- [ ] Expose validated output and resource preferences in the plugin UI and
  version the engine/job protocol accordingly.
- [ ] Add tagged release assets and publisher signing/notarization; preserve
  native architecture and extracted-package tests.

Implementation order: real-capture acceptance and baselines → capture
auto-detection/generalized capabilities → tiled stores/sinks → linear DNG →
subpixel reconstruction/motion → tuning and release polish. Run small DNG and
subpixel feasibility tests early enough to shape the region/calibration APIs.

See [validation requirements](docs/validation.md) and the earlier
[detailed milestone design](docs/next-milestones.md) for supporting detail.
