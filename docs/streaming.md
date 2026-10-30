# Streamed reconstruction and lossless output

The CLI now prepares alignment once, reconstructs output regions on demand and
writes each region before requesting the next. The normal path never allocates
the complete reconstructed RGB image or TIFF display buffer. The whole-image
merger remains a numerical reference behind `merge --fixtures --reference`;
`--reference` cannot be used on real ARWs.

## Current boundary and limits

`pixelshift-core::ImageSource` supplies globally anchored `Region` rectangles.
`PreparedMerge` borrows validated source frames and exposes this interface. Its
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

**Input storage remains normalized float32 RAM.** Sixteen 61 MP sources alone
occupy about 3.9 GB decimal, with decoder/allocator working memory on top. There
is no scratch-disk source store or enforced global memory budget yet. This is
an output-memory milestone, not the proposed 512 MiB–1 GiB low-memory mode.
Packed `u16` RAM/disk source stores and explicit input-region halos are next.

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
- Allocated source-buffer capacity and maximum float32 output-region payload.
  These are allocation estimates, not whole-process memory measurements.
- OS peak resident memory: Linux current-address-space `VmHWM` with rusage
  fallback, macOS rusage bytes, and Windows peak working set. Linux uses `VmHWM`
  to avoid a large pre-exec launcher's inherited rusage peak. CPU user/system and
  block-operation counts are reported where available; unsupported values are
  null. Block operations are OS counters, not portable byte counts.
- Final file bytes, region count and motion-fallback pixels.

Whole-frame ARW decoding is not instrumented internally. Validation/alignment
currently share a timer; guide-generation and disk-store I/O stages do not exist
in this path yet. Timing and compression-ratio values vary with image content,
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
processes for whole-image/uncompressed, streamed/uncompressed and streamed/
lossless output, and verifies equal decoded SHA-256 values with tifffile. It
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
