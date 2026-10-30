# Roadmap

The first installable alpha supports Sony a7CR four-shot and sixteen-shot Pixel
Shift, RAW burst denoise, and experimental linear DNG/TIFF output. This roadmap tracks follow-ups; items
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
acceptance. The current path stores native u16 sensors in RAM or temporary disk files and
normalizes row windows while streaming output. A full reconstructed image is no longer retained.

## 1. Validate the alpha with real captures

- [ ] Complete installation, engine launch, merge and catalog import acceptance
  on supported macOS and Windows systems. Initial macOS a7CR four-shot feedback
  confirms merge/import, AI Denoise/Adaptive access and correct lens recognition;
  Lightroom version and broader platform/capture coverage remain unrecorded.
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
- [x] Replace hardcoded a7CR checks in the CLI/decoder with registered camera
  capabilities using a versioned YAML catalog and JSON Schema. Reuse verified
  shift layouts across models where appropriate.
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

- [x] Add initial preflight/decode, validation+alignment, region reconstruction,
  encode/write, preview and total timings, output bytes and OS peak resident
  memory. Report CPU/block-operation counters where available.
- [ ] Split internal decode/guide/alignment and compression/write timing further
  when those stages gain separate interfaces; block operations are not byte I/O.
- [ ] Establish full-size four-shot/sixteen-shot baselines on Apple Silicon,
  Intel and Windows hardware. Report hardware, storage and cold/warm cache state.
- [ ] Profile repeated interpolation, allocations and per-pixel bookkeeping.
- [ ] Measure CPU utilization and memory-bandwidth limits before parallelizing
  more stages. Coordinate decoder, Rayon and codec workers to avoid oversubscription.

Acceptance: reproducible stage-level benchmarks and a measured memory/time
tradeoff. CI runner timing is a regression signal, not a product speed promise.

## 4. Tile processing and offer memory/performance preferences

- [x] Introduce output-region and pull-based streaming writer interfaces. Keep
  the whole-image fixture merger as a numerical reference.
- [x] Add raw input-row sources and bounded normalized source windows for
  interchangeable packed/disk storage; no per-pixel disk reads.
- [x] Decode each ARW once into packed `u16` sensor storage, with interchangeable
  RAM and disk stores. Normalize only the regions currently being processed.
- [ ] Build small alignment guides during ingestion; release whole-frame decoder
  buffers after storing each source.
- [x] Reconstruct output bands with globally anchored, even-origin source halos
  covering supported integer transforms and bilinear CFA interpolation. Test
  partial strips/2D regions against full-frame output; future motion kernels must
  extend the halo contract.
- [x] Stream DNG/TIFF reconstruction and encoding without a full RGB output.
  Stitch irregular regions with global coordinates; verify float-bit equality,
  preview parity, motion fallback and worker-count independence.
- [x] Pull one output band at a time with writer backpressure, collect a bounded
  preview from those pixels, and clean partial output on late failures.
- [ ] Bound source caches, active tiles, codec workspace and pending writes under
  one memory budget, with writer backpressure.
- [x] Expose **RAM / Disk (less RAM)** in Lightroom with a remembered preference;
  add CLI scratch-directory selection and report packed/scratch/window bytes.
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

Prioritize a small-image interoperability proof now, alongside tiled processing:
real TIFF testing has exposed Adobe profile and RAW editing limitations.

- [ ] Verify Adaptive Color/Adaptive B&W profiles on merged linear DNG in current
  Lightroom Classic. Test AI Denoise, Raw Details, Super Resolution and adaptive
  presets/masks separately; document availability by Lightroom version rather
  than assuming all DNGs qualify for every RAW feature.
- [ ] Preserve camera/lens/focal-length/exposure identity. Resolve Sony lens IDs
  where standard EXIF lacks a lens name; do not guess ambiguous lenses.
- [ ] Validate RAW lens-profile matching and Sony built-in corrections, including
  distortion, vignetting and CA. Recompute correction coordinates for output crop
  and sixteen-shot scale; never blindly copy source geometry/opcodes.
- [ ] Compare merged DNG and source ARW under matched WB, exposure and Adobe
  profile. Check neutral patches, chart colors, highlight recovery and tone.
  Initial real four-shot feedback reports a slightly darker DNG and softer detail
  with AI Denoise off on both and matched Detail settings. Diagnose alignment,
  reconstruction, baseline exposure and profile interpretation; don't add an
  arbitrary contrast/sharpening/exposure correction. Later screenshots confirm
  both Adobe Color / As Shot / Exposure 0.00; DNG WB is 6350 K/+18 versus ARW
  5850 K/+15. Dual-illuminant A/D65 output is implemented as a calibration
  improvement, without changing WB gains, raw samples or baseline exposure;
  its actual Lightroom rendering remains to verify.
- [ ] Inherit the reference photo's lens-correction enable preference for a
  compatible merged DNG. Initial feedback required manually enabling the
  correctly preselected lens profile. Keep this separate from copying source
  geometry/opcodes or arbitrary Develop edits.
- [ ] Verify decoder-crop mapping against the reported 9504×6336 ARW visible area.
  Initial 9588×6364 DNG exposed more of the decoded buffer. Preserve the actual
  source crop and common valid intersection, accounting for shift and 16-shot
  scale; do not hardcode a centered crop or promise identical edge coverage.
  User confirms correct output size on the initial macOS four-shot retest.

- [x] Prove an initial strip writer using the TIFF library, preserving linear
  samples without a second full-image output allocation. The existing merger
  still requires a whole image; an incremental region sink remains to implement.
- [x] Write initial scene-linear camera RGB with calibration, illuminant, as-shot neutral,
  black/white semantics, crop, orientation, provenance and a small preview.
  Do not bake WB, sRGB conversion, gamma or tone mapping into the RAW image.
- [x] Establish float32 storage that preserves relevant headroom and
  negative black noise, with DNG 1.4 semantics and no quantization to 16-bit.
  Evaluate compressed storage separately; lossless compression does not remove
  integer quantization.
- [x] Add lossless Deflate strips (fast level, no float quantization), with
  uncompressed diagnostic output and independent/Adobe reader checks.
- [ ] Add true 2D output tiles and bounded parallel compression if profiling
  justifies it. Avoid embedding all source ARWs by default.
- [ ] Validate with Adobe DNG tooling, an independent reader and Lightroom import/
  development, including WB, exposure recovery, color, crop and orientation.
- [x] Retain TIFF as a CLI alternative. The plugin now selects experimental DNG
  explicitly for Lightroom host testing; this is not a stable-output claim.
- [x] Stream reconstructed float32 regions into lossless/uncompressed strips,
  with a bounded preview SubIFD and interchangeable packed RAM/disk sources.
- [x] Check synthetic sample/WB/matrix/metadata roundtrips, independent tifffile
  and LibRaw development, and local Adobe DNG SDK validation. Full-size captures,
  Adobe Adaptive Color/B&W and AI Denoise remain host acceptance gates.

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

- [x] Add Lightroom stage progress with live decode counts, indeterminate merge/
  write/import stages, and scope cleanup on success/failure. Poll appended engine
  diagnostics while the helper runs in a separate SDK task; verify in Lightroom.
- [ ] Add safe cooperative cancellation and finer engine progress. Account for
  cancellation latency during a whole-frame upstream decode; the current scope
  disables cancellation rather than pretending it can stop the engine.
- [ ] Re-verify progress in Lightroom after the reported `Progress` toolkit module
  load failure. Use a project-named plain-table module, increment the SDK build,
  validate extracted ZIP Lua loading in isolation, and require full replacement
  with Lightroom closed. The screenshot alone does not identify the root cause.
- [ ] Expose validated output and resource preferences in the plugin UI and
  version the engine/job protocol accordingly.
- [ ] Sign the macOS helper with a Developer ID Application certificate, hardened
  runtime and a secure timestamp. Current ad-hoc signing is not publisher trust.
- [ ] Notarize the macOS distribution with `notarytool`; choose a supported DMG
  or PKG container and staple its ticket. Standalone Mach-O binaries and ZIPs
  cannot themselves carry a stapled ticket. Retain ZIP development artifacts.
- [ ] Add CI signing/notarization credentials through repository secrets, verify
  signatures and notarization results, and test a normal browser download with
  quarantine intact on a fresh macOS account (Intel and Apple Silicon). Installation
  should not require `xattr` or a global Gatekeeper change.
- [ ] Add tagged release assets and Windows publisher signing; preserve native
  architecture and extracted-package tests.

Next implementation order: full-size packed-store measurements and ingestion
guide caching → measured memory budgets/automatic storage selection → subpixel reconstruction/motion →
tuning and release polish. Real-capture acceptance and color/loader validation
continue alongside this work. Streamed output and lossless DNG are implemented. Run small DNG and
subpixel feasibility tests early enough to shape the region/calibration APIs.

See [validation requirements](docs/validation.md) and the earlier
[detailed milestone design](docs/next-milestones.md) for supporting detail.

See [streamed reconstruction and baseline measurements](docs/streaming.md) for
current limits and reproducible synthetic comparisons.
