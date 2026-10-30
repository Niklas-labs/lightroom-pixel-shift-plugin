# Streamed reconstruction and lossless output

The CLI now prepares alignment once, reconstructs output regions on demand and
writes each region before requesting the next. The normal path never allocates
the complete reconstructed RGB image or TIFF display buffer. The whole-image
merger remains a numerical reference behind `merge --fixtures --reference`;
`--reference` cannot be used on real ARWs.

## Current boundary and limits

`pixelshift-core::ImageSource` supplies globally anchored `Region` rectangles.
`PreparedMerge` borrows validated sensor sources and exposes this interface. Its
coordinates, CFA sampling, accumulation order and motion fallback are identical
to the whole-image reference. Tests stitch irregular 2D regions and compare float
bits and fallback counts across worker counts and all merge modes.

DNG/TIFF writers pull one full-width band at a time. The default is 32 rows; job
`strip_rows` accepts 1–256. They validate each returned region, encode/write it,
then release it. A small preview is collected from these same pixels, so making
the preview does not reconstruct pixels again or inflate fallback counts. At
19176 output columns, a 32-row camera-RGB region is about 7.4 MB decimal. Codec,
preview, thread and metadata allocations are additional. Writer backpressure
keeps a single region in flight; there is no unbounded queue of completed tiles.

## Packed sources and disk mode

ARWs are decoded sequentially into native `u16` Bayer samples. RAM mode retains
these samples, rather than normalized float32 copies. Sixteen 61 MP sources
occupy about **1.95 GB decimal** before capacity/decoder overhead, versus 3.9 GB
for the previous float32 stores. No precision is discarded: black/white levels
remain per-CFA-phase float32 metadata, and normalization uses the original
`(sample - black) / (white - black)` expression only when rows are requested.
Negative black noise and samples above white level remain valid.

Disk mode writes each decoded sensor to a process-private anonymous temporary
file, then drops its full-size decoder buffer before decoding the next source.
Reconstruction reads contiguous full-width rows into bounded normalized windows;
there is no file I/O or locking in the per-pixel Rayon workers. Each window has
an even sensor-row origin, preserving Bayer phase, and a conservative halo
covering the supported integer translations, nominal shifts and bilinear lattice.
Coordinates remain global. Small alignment row windows use the same samples and
score accumulation order as the previous full-frame alignment. Guides are read
on demand; an ingestion-time guide cache/prefetch remains a performance follow-up.

The Lightroom merge dialog exposes **Memory: RAM / Disk (less RAM)** and remembers
the selection. Both produce the same pixels. RAM is the default. CLI jobs accept:

```json
{
  "source_storage": "disk",
  "scratch_directory": "/Volumes/FastSSD/PixelShiftScratch",
  "strip_rows": 32
}
```

These are fields to add to a normal version-1 job, not a complete job. Omitted
`source_storage` means `ram`; omitted `scratch_directory` uses the OS temporary
directory. A supplied directory must already exist; relative paths resolve
beside the job. Disk scratch needs approximately two bytes per sensor pixel per
input (about 1.95 GB for sixteen 61 MP shots), plus the separate output file.
File creation/write/read errors abort with diagnostics, scratch handles close on
success/error, and partial output is not published. Anonymous temporary files
are removed when their last handle closes, including process termination;
there is no reusable disk cache or recovery manifest. Available disk-space
preflight is not implemented; the files grow as sources are ingested.

Whole-frame rawler decoding still sets a memory floor: one decoded RAW and its
compressed input/decoder workspaces can coexist. Allocator retention, OS page
cache, codec/preview/thread buffers and metadata add overhead. There is no hard
whole-process memory cap, automatic storage selection or validated 512 MiB–1 GiB
full-size target yet. Float JSON fixtures deliberately retain float32 values in
both stores; they are never quantized and do not demonstrate native u16 packing.

Changing region sizes does not change image quality. This work does not change
alignment accuracy, motion masks, camera color, white balance or baseline
exposure, and does not claim to fix the reported softness/darker rendering.

## Compression

Normal jobs use `"compression": "lossless"`: independent Deflate/Zlib strips at
fast compression level, without quantization or floating-point predictors. DNG
keeps IEEE float32 camera samples, negative black noise and highlight headroom.
TIFF keeps the existing rendered sRGB RGB16 representation and ICC metadata.
Compression operates on the native-endian samples declared in the TIFF header.

Use `"compression": "none"` for an uncompressed diagnostic output. Omitted fields
remain valid protocol-1 jobs; `strip_rows` defaults to 32. Unknown compression
values or unsafe row counts are rejected before output is written. Files remain
classic TIFF/DNG; worst-case size is checked, and final publication uses a
same-directory temporary file, sync and no-clobber rename. Late reconstruction,
encoding or progress-callback failures remove the temporary output.

LibRaw/rawpy, tifffile and local Adobe DNG SDK validation accept the tested
Deflate float32 DNG strips. Rawler 0.8's reader does **not** decode Deflate strips;
its roundtrip tests explicitly use uncompressed DNG, with independent readers
covering compressed files. Lightroom must still confirm compressed output on
real captures. No DNG input support has been added to the Sony ARW adapter.

## Measurements

The result record and diagnostic `performance:` line include:

- Preflight, decode, validation/alignment, region reconstruction, encode/write,
  preview and total elapsed times. TIFF color rendering is included in its
  encode/write timer. Whole-image fixture references also report
  their separate compute time. Output metadata/finalization is included in output
  total time but not the encode/write strip timer.
- Allocated source-buffer capacity, scratch bytes, maximum normalized input-window
  payload and maximum float32 output-region payload.
  These are allocation estimates, not whole-process memory measurements.
- OS peak resident memory: Linux current-address-space `VmHWM` with rusage
  fallback, macOS rusage bytes, and Windows peak working set. Linux uses `VmHWM`
  to avoid a large pre-exec launcher's inherited rusage peak. CPU user/system and
  block-operation counts are reported where available; unsupported values are
  null. Block operations are OS counters, not portable byte counts.
- Final file bytes, region count and motion-fallback pixels.

Whole-frame ARW decoding is not instrumented internally. Validation/alignment
share a timer. Scratch writes are included in decode, alignment reads in
validation/alignment, and reconstruction reads in reconstruction time. Timing and compression-ratio values vary with image content,
CPU, storage and cache state; the synthetic baseline is not a camera benchmark.

## Reproduce a synthetic release baseline

```sh
cargo build --release --locked -p pixelshift-cli
python -m pip install tifffile==2026.10.15
python scripts/benchmark-streaming.py \
  --engine target/release/sony-pixelshift --width 1024 --height 768 \
  --mode sixteen --workers 4 --report /tmp/streaming-report.json
```

The script creates deterministic textured/noisy Bayer fixtures, runs separate
processes for whole-image/uncompressed, RAM/uncompressed, RAM/lossless and
disk/lossless output, and verifies equal decoded SHA-256 values with tifffile. It
releases generated arrays and defers importing image readers/decoding outputs
until every engine run has exited, to avoid contaminating child memory peaks.
Fixture JSON reads are buffered. Cache and machine state are not controlled;
record hardware and repeat measurements before making performance claims.

The recorded small Linux baseline is in
[benchmarks/streaming-linux.json](benchmarks/streaming-linux.json). Full-size
Apple Silicon, Intel and Windows ARW baselines remain required.

One recorded Linux/container run (1024×768 sources, 16 shots, 2032×1520 output,
4 Rayon workers, 32-row bands, optimized build) produced:

| Variant | Peak RSS | Wall time | DNG size |
| --- | ---: | ---: | ---: |
| Whole-image fixture reference, uncompressed | 91.5 MiB | 1.013 s | 35.9 MiB |
| Streamed, uncompressed | 56.5 MiB | 1.002 s | 35.9 MiB |
| Streamed, lossless Deflate | 56.2 MiB | 1.108 s | 30.5 MiB |

Decoded float32 SHA-256 values match in all three cases. This is a single small
synthetic run, with uncontrolled host load/cache state. It demonstrates removal
of the full output buffer; it does not establish full-size camera performance,
expected photo compression ratio or the low-memory source-store target.

## Packed-store validation

Rust tests construct native integer sources with odd sensor widths, reversed
Bayer phase, phase-dependent levels, and negative/headroom samples. RAM and disk
reconstruction must match the full-float reference **bit for bit**, including
alignment residuals and motion fallback counts, across four/sixteen/burst modes,
one-row/irregular strips and 2D regions. Tests also cover malformed bounds/levels,
truncated scratch reads, cleanup and CLI failure after ingestion. Independent
DNG readers compare disk/RAM previews and samples; native package smoke tests
exercise disk output with an empty PATH. Full-size real-ARW memory and speed
measurements on supported platforms remain required.

A later small synthetic process run is recorded in
[benchmarks/packed-stores-linux.json](benchmarks/packed-stores-linux.json):
16 × 1024×768 **float JSON** sources, 4 workers, 32-row bands, optimized build.
RAM/lossless peaked at 57.0 MiB versus disk/lossless at 9.8 MiB; wall times were
1.128 s and 1.113 s, with identical decoded hashes. Warm OS cache and host load
were uncontrolled. This demonstrates bounded source residency for the disk
path, not the native packing ratio or full-size ARW performance. Disk is not
expected to outperform RAM consistently; measure using the intended drive.
