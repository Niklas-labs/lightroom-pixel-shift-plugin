# Experimental linear DNG output

The Lightroom plugin saves `.dng` beside its sources. CLI jobs choose DNG or the
existing rendered TIFF writer by `.dng` versus `.tif`/`.tiff` extension. Pixel Shift
four-shot, sixteen-shot and ordinary burst denoise share this output boundary.
Menu commands are registered under **Library → Plug-in Extras** only.

## Representation

- DNGVersion and DNGBackwardVersion 1.4.0.0; classic TIFF offsets.
- Main IFD: LinearRaw photometric interpretation (34892), three interleaved
  camera-RGB channels, 32-bit IEEE float, lossless Deflate 32-row strips by default
  (uncompressed remains available through job configuration).
- Samples retain the merger's normalized black-subtracted camera values, including
  negative noise and values above nominal white. The a7CR profile normalizes
  native codes against the measured Adobe white reference 16383 (black remains
  decoder-owned, 512 in the validated capture). BlackLevel is zero and WhiteLevel
  is one per channel. No WB, gamma, display matrix, tone curve or Lightroom edits
  are baked into this image. No fabricated CFA tags are written.
- Original XYZ-to-camera matrices for illuminant A (tungsten, slot 1) and D65
  (daylight, slot 2), when both exist in rawler's camera catalog. Otherwise retain
  single-D65 slot 1. Reciprocal green-normalized as-shot WB remains unchanged.
  These are catalog calibrations, not a claim of identical Adobe ARW rendering.
  The small preview/TIFF renderer still uses D65 only; compare Lightroom Develop
  rendering, rather than an embedded thumbnail, for the calibration test.
- Optional model-profile DNG rendering metadata: reference-backed BaselineExposure,
  CameraCalibration1/2, ForwardMatrix1/2 and matching calibration signatures.
  The a7CR profile now uses +0.35 EV and the calibration measured in a Lightroom
  conversion of an original ARW. No adjustment is multiplied into sensor pixels;
  original AsShotNeutral and ColorMatrix values remain unchanged. Profiles without
  this metadata keep 0 EV. See [camera configuration](camera-profiles.md).
- Original camera/lens/exposure EXIF when available, orientation, software and
  experimental merge description. UniqueCameraModel uses the camera identity;
  no Adobe look table, tone curve, profile name, Sony MakerNote, serial, GPS or
  lens-correction opcode is copied.
- ActiveArea describes the valid reconstructed buffer. DefaultCrop maps the
  reference ARW's preferred decoder crop into this buffer, including nominal
  sensor shift, merger origin and sixteen-shot scale. It is intersected with
  valid output, so safety borders can reduce the visible size slightly. The
  preview uses that same crop. If no source crop is known, use the full valid
  buffer; do not infer a centered crop from advertised megapixels. Source lens
  geometry must not be copied unchanged into this coordinate system.
- A separate sRGB RGB8 reduced preview, at most 512 pixels per side, is referenced
  through SubIFDs. There are no chained image IFDs. Root orientation applies.

The production CLI reconstructs each output band as the writer requests it; no
full reconstructed RGB image is allocated. The preview is collected from these
same bands. Native u16 sources use packed RAM/disk stores, with normalized
row windows and CFA-aware halos. One-frame RAW decoding and allocator overhead
remain; there is no hard process memory cap yet. Lossless compression preserves sample bits;
`compression: none` produces an uncompressed diagnostic DNG. See
[streaming architecture and measurements](streaming.md).

Uncompressed payloads are about 0.73 GB at 61 MP and 2.9 GB at 244 MP. Compressed
size depends on content/noise. Larger images exceeding classic TIFF offsets are
rejected, including compression overhead allowance. Publication remains atomic
and no-clobber; late failures remove partial output.

## Automated evidence

Rust tests cover exact float sample roundtrips via rawler (including negative
noise/headroom), WB, matrix direction/values, EXIF, orientation, preview decoding,
invalid data and no-overwrite. CLI tests cover all three modes and retain TIFF
coverage. Package smoke tests generate sixteen-shot DNG using the embedded engine.

`scripts/test-dng.py` checks all modes using independent tifffile and LibRaw
(rawpy) readers. Constant CFA color fields with non-unit WB verify that samples
were not gamma encoded, white balanced or converted to sRGB; it also checks DNG
version/tags, dimensions, preview and successful development. Linux packaging is
gated on these checks, with pinned reader versions.

A local development check also builds Adobe DNG SDK 1.6's `dng_validate` from the
[Linux SDK mirror](https://github.com/Autoenhance-ai/adobe-dng-sdk/tree/1.6-linux),
with XMP and JPEG disabled, with Zlib enabled (tested files have no XMP).
Both uncompressed and Deflate strips pass, including partial final strips.
The generated synthetic output must complete validation without warnings/errors.
The validator is a development dependency only, not bundled in the plugin. This
is evidence for the tested synthetic files, not a claim of Adobe certification.

## Lightroom host gates

Import/develop a real four-shot capture, then verify **Adaptive Color**, **Adaptive
B&W** and **AI Denoise** independently. Record Lightroom version, GPU, lens,
camera firmware and RAW compression mode. Check reopening/recalculation, WB,
exposure recovery, neutral/chart colors, lens matching/corrections, crop and
orientation. Repeat with sixteen shots and ordinary bursts after the initial
four-shot check. No host feature is marked working solely because a DNG reader
opens the file.

Adobe's [Enhance documentation](https://helpx.adobe.com/lightroom-classic/help/enhance-details.html)
lists linear DNG as supported for AI Denoise. Raw Details excludes linear DNG;
it is not an acceptance requirement for this already reconstructed RGB image.
Adaptive Color/B&W are [RAW profiles](https://helpx.adobe.com/lightroom-classic/desktop/process-and-develop-photos/image-tone-color.html).
Their eligibility and appearance on our files still need actual Lightroom tests.

## Initial real-capture feedback

The user reports that the a7CR four-shot DNG enables AI Denoise and Adaptive
features in Lightroom, and selects the correct lens profile once corrections are
manually enabled. The Lightroom version has not been recorded, so this is evidence
for that tested installation, not a version-wide compatibility claim.

The merged file was reported as 9588×6364 versus the ARW's 9504×6336 visible size.
The original writer ignored the decoder crop; the follow-up carries it into DNG
DefaultCrop and logs sensor dimensions, crop, alignment residuals and fallback
fraction. Real-capture crop verification remains needed. Slightly darker tone and
softer detail were reported with AI Denoise off on both and matched Detail
settings. Investigate registration/reconstruction and exposure/profile metadata;
do not compensate with arbitrary sharpening or exposure boosts. The screenshot
shows f/22, which affects both files and cannot explain their difference by itself.

## Exposure and white-balance comparison

The user confirmed the corrected output size on the tested macOS installation.
A subsequent toolkit `Progress` module load failure requires progress host
acceptance to be repeated after the loader hardening update. Subsequent screenshots show Adobe Color,
As Shot WB and Exposure 0.00 on both images: merged DNG 6350 K / +18 tint, source
ARW 5850 K / +15 tint, with uniform slight darkening reported. This rules out an
Exposure-slider mismatch for that comparison. Matching profile names and zero
sliders still do not establish identical camera calibration or default rendering.

The initial writer carried D65 only. The follow-up preserves both illuminant A
and D65 when available, while leaving samples, as-shot sensor gains and baseline
exposure (0 EV) unchanged. Independent/tag/Adobe validation passes; actual Lightroom
WB numbers, color and brightness remain to verify. Different As Shot temperature/
tint numbers alone do not prove an incorrect sensor white balance; do not force
matching displayed values or add a camera exposure offset without measurement.

## Reference-backed rendering calibration (build 7)

The user's Adobe-converted source DNG establishes that AsShotNeutral and both
ColorMatrix tags match ours numerically. Adobe additionally writes +0.35 EV
BaselineExposure, CameraCalibration1/2 diag(0.9487, 1, 1.0363), A/D65 forward
matrices and matching `com.adobe` calibration signatures. Build 7 emits those
reference-backed tags from YAML. The earlier 0 EV statements above describe the
previous builds, not the current configured a7CR output.

This is evidence for a metadata mismatch, not a measured final rendering fix.
Validate a newly merged DNG in Lightroom: compare As Shot WB with the source's
5850/+15 and repeat the Sony ARQ comparison. Avoid reusing a previous output or
its cached previews. Raw samples must remain bit-identical with/without this
metadata; no ad-hoc +0.433 EV pixel gain is used. Tests check tag values with an
independent reader and exact synthetic photometry after writing/decoding. Full
Adobe profile behavior, real RAW normalization, additional ISOs and other camera
bodies still require reference captures. The embedded preview/TIFF renderer is
an approximation that does not implement Adobe's rendering/profile pipeline.

## Reference white-scale correction (build 9)

Matching the same source ARW against its Adobe-converted mosaic DNG confirms
sampled native integer counts agree at zero pixel offset. Both report black 512,
but rawler supplies white 15360 and Adobe writes white 16383. The old normalization
therefore multiplies samples by `(16383-512)/(15360-512) = 1.068898`, or +0.096124 EV,
relative to Adobe. This is consistent with a larger rendered difference after
Lightroom's nonlinear tone mapping; the rendered +0.1593 EV is not itself a RAW
exposure offset.

Build 9 uses a guarded a7CR `raw_normalization` YAML record to select reference
white 16383 before any merge processing. Native codes and black levels stay
intact; AsShotNeutral, color/calibration matrices, forward matrices, and +0.35 EV
BaselineExposure are unchanged. Diagnostics report both original decoder white
and effective white. Unknown decoder white conventions are rejected instead of
silently applying this capture's scale to an unverified variant. Real normalized
sample parity and fresh Lightroom render comparison remain acceptance tests.

## Streamed/compressed output acceptance

Independent tests compare whole-image reference versus streamed reconstruction,
multiple strip sizes and compression settings, including exact sample bits,
preview bytes and source crop. Irregular-region tests cover motion fallback and
worker counts. Rawler 0.8 cannot read Deflate strips, so its legacy reader tests
use uncompressed files; tifffile/LibRaw and Adobe checks cover compressed DNG.
Real Lightroom compressed-DNG rendering/Adaptive/AI acceptance remains to verify.
