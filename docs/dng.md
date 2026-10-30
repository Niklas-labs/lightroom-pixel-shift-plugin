# Experimental linear DNG output

The Lightroom plugin saves `.dng` beside its sources. CLI jobs choose DNG or the
existing rendered TIFF writer by `.dng` versus `.tif`/`.tiff` extension. Pixel Shift
four-shot, sixteen-shot and ordinary burst denoise share this output boundary.
Menu commands are registered under **Library → Plug-in Extras** only.

## Representation

- DNGVersion and DNGBackwardVersion 1.4.0.0; classic TIFF offsets.
- Main IFD: LinearRaw photometric interpretation (34892), three interleaved
  camera-RGB channels, 32-bit IEEE float, uncompressed 32-row strips.
- Samples retain the merger's normalized black-subtracted camera values, including
  negative noise and values above nominal white. BlackLevel is zero and WhiteLevel
  is one per channel. No WB, gamma, display matrix, tone curve or Lightroom edits
  are baked into this image. No fabricated CFA tags are written.
- Original XYZ-to-camera matrices for illuminant A (tungsten, slot 1) and D65
  (daylight, slot 2), when both exist in rawler's camera catalog. Otherwise retain
  single-D65 slot 1. Reciprocal green-normalized as-shot WB remains unchanged.
  These are catalog calibrations, not a claim of identical Adobe ARW rendering.
  The small preview/TIFF renderer still uses D65 only; compare Lightroom Develop
  rendering, rather than an embedded thumbnail, for the calibration test.
- Original camera/lens/exposure EXIF when available, orientation, software and
  experimental merge description. UniqueCameraModel uses the camera identity;
  no Adobe profile, Sony MakerNote, serial, GPS or lens-correction opcode is copied.
- ActiveArea describes the valid reconstructed buffer. DefaultCrop maps the
  reference ARW's preferred decoder crop into this buffer, including nominal
  sensor shift, merger origin and sixteen-shot scale. It is intersected with
  valid output, so safety borders can reduce the visible size slightly. The
  preview uses that same crop. If no source crop is known, use the full valid
  buffer; do not infer a centered crop from advertised megapixels. Source lens
  geometry must not be copied unchanged into this coordinate system.
- A separate sRGB RGB8 reduced preview, at most 512 pixels per side, is referenced
  through SubIFDs. There are no chained image IFDs. Root orientation applies.

Strip writes borrow the existing merged RGB buffer; preview memory is bounded.
Sources are released before writing. This avoids the full-size TIFF render
buffer but **does not make reconstruction bounded-memory**. Uncompressed output
is approximately 12 bytes per pixel: 0.73 GB at 61 MP, 2.9 GB at 244 MP. Lossless
compression and tiled reconstruction remain follow-ups. Larger images that would
exceed classic TIFF offsets are rejected before creating output. Publication uses
a same-directory temporary file, sync and no-clobber rename.

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
with XMP and JPEG disabled (the tested files are uncompressed and have no XMP).
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

The user confirmed the corrected output size and native Lightroom progress UI
work on the tested macOS installation. Subsequent screenshots show Adobe Color,
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
