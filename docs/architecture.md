# Architecture and format seams

Dependency direction: CLI → IO → core. Core has no Lightroom, filesystem, RAW
container, color rendering, TIFF or DNG dependency.

`CameraProfile` describes model identity and a per-mode chronological sensor
translation. `SonyA7Cr` is the sole implementation. `RawDecoder` returns a
normalized Bayer `Frame` and separate `Color` calibration; `SonyArwDecoder` is
the only real-file adapter. `OutputWriter` consumes scene-linear camera RGB and
calibration; `TiffWriter` is the only writer. Future camera/container handling
belongs behind those boundaries and explicit CLI registration. Do not route
unknown models through the a7CR table. Future sensors with a non-Bayer layout
need a new sample-layout abstraction; current validation deliberately refuses
them. No other manufacturers' formats are implemented.

Frame coordinates remain in sensor orientation. For chronological shots 1..4,
Sony integer translations (x,y) are (1,0), (1,1), (0,1), (0,0). For 16 shots,
four consecutive groups add half-pixel translations (0.5,0), (0,0), (0.5,0.5),
(0,0.5). These conventions reflect make_arq's sorting/placement after expressing
them directly in shooting order. Synthetic fixtures independently encode the
same table. Camera acceptance must verify sequence-number origin and direction.

The core samples matching CFA channels directly at output grid points. Four
shots fill RGB at native resolution; sixteen fill a 2x linear-resolution grid.
Only redundant observations of the same channel/position are averaged. Missing
observations after correction use a bilinear reference interpolation. Denoise
mode instead resamples each unshifted CFA color lattice and averages all frames.

Registration compares interpolated green values over a sparse grid, searching
bounded integer residual translations in addition to commanded sensor shifts.
It uses a clipped absolute-difference score and prefers zero on ties. This is a
translation-only first implementation, without confidence estimation, rotation,
rolling-shutter compensation or subpixel residuals. Local motion detection
compares registered channels to the reference; moving pixels fall back to the
first chronological shot, so local super-resolution and denoise are sacrificed
to avoid ghosts. Thresholds use normalized linear sensor units. Motion masking
may misclassify aliasing/noise and needs spatial mask refinement.

The TIFF renderer applies reference-shot WB and a single D65 catalog matrix,
converts to sRGB, clips and applies the sRGB transfer. No exposure matching,
dual-illuminant interpolation, lens correction or highlight recovery. ICC is
embedded. A whitelist of standard camera/lens/exposure metadata is preserved; private
MakerNotes, GPS, serials and geometry-dependent correction tags are omitted.

The DNG writer stores camera-space float32 **linear DNG 1.4**, with a single
D65 matrix, as-shot neutral, black/white semantics, crop, orientation, standard
EXIF and a bounded sRGB preview SubIFD. Main RGB is not gamma encoded or white
balanced. Uncompressed strips borrow the merged RGB buffer; sources are released
before output. This bounds writer overhead but does not fix the whole-image
merger's peak memory. TIFF remains a separate rendered output.

Numerical roundtrips, independent tifffile/LibRaw checks and a local Adobe SDK
validator check cover synthetic output. Lightroom Adaptive profiles, AI Denoise,
real color and lens corrections remain explicit host gates. See [DNG](dng.md).

Follow-up plugin protocol should add persistent job diagnostics, machine-readable
progress, cancellation and settings. Current invocation is one job per process,
no shell-embedded input filenames; only executable and generated manifest paths
enter the command line. Windows shell expansion characters are rejected. Keep
Lightroom SDK calls outside the numerical engine.
