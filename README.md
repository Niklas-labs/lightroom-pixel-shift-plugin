# Sony Pixel Shift for Lightroom Classic

An executable Rust prototype and Lightroom Classic Lua plugin for **Sony a7CR
(ILCE-7CR)** 4-shot / 16-shot Pixel Shift and multi-frame RAW denoise.
Camera profiles are experimental: synthetic reconstruction is tested; real-camera
and Lightroom host acceptance is still required. No original RAW is modified.

## Build and run

Install stable Rust and [ExifTool](https://exiftool.org/) (on PATH), then:

```sh
cargo build --release --locked
./target/release/sony-pixelshift cameras
./target/release/sony-pixelshift merge --job /photos/capture/job.json
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
lua scripts/test-plugin.lua
```

Example `job.json` (input and output paths resolve against its directory):

```json
{
  "version": 1,
  "camera": "ILCE-7CR",
  "mode": "four",
  "inputs": ["DSC00001.ARW", "DSC00002.ARW", "DSC00003.ARW", "DSC00004.ARW"],
  "output": "merged.tif",
  "motion_threshold": 0.08,
  "alignment_radius": 2
}
```

Use `sixteen` with 16 files for twice the sensor width and height before cropping;
use `denoise` for 2–64 ordinary RAW burst frames. Each input must have a unique
Sony SequenceNumber forming 1..N; sorting uses metadata, not filenames. Burst
sequences without these tags are currently rejected. Exposure, model, dimensions,
CFA and camera serial (when available) must agree. JSON job version and fields
are checked strictly. Existing output files are always refused; DNG output is
explicitly rejected until its writer is implemented.

The CLI prints decode progress and errors to stderr and one JSON completion
record (including crop origin, residual alignment and fallback count) to stdout.
Exit codes are 0 for success, 1 for processing failure, 2 for argument errors.

For a small repeatable demonstration without RAW files:

```sh
python3 scripts/generate-demo.py /tmp/pixelshift-demo
cargo run --locked -p pixelshift-cli -- merge --fixtures --job /tmp/pixelshift-demo/job.json
```

`--fixtures` explicitly enables synthetic JSON sensor inputs; it is never enabled
by the Lightroom plugin. See the integration tests for its schema.

## Lightroom Classic scaffold

Add `lightroom/SonyPixelShift.lrplugin` in File → Plug-in Manager. Select one
complete a7CR capture, then Library → Plug-in Extras → Merge Sony a7CR Pixel Shift.
For a normal burst use the separate denoise menu. On first use choose the compiled
engine executable; choose a new TIFF destination. The plugin invokes the engine
on original ARWs and imports the TIFF only after successful completion.

The plugin uses asynchronous SDK tasks, validates offline originals, writes a
temporary versioned job, quotes process arguments and imports under catalog write
access. ExifTool must be visible in Lightroom's process environment (GUI PATH may
differ from a terminal). macOS/Windows host execution and catalog import are
**not yet tested here**. Cancellation during a running engine process, progress
polling, job logging and a plugin settings panel remain planned. Failed import
leaves the completed TIFF available to import manually.

## Implemented and next

- `pixelshift-core`: camera profile seam, Bayer sample reconstruction, 4-shot full
  color, 16-shot half-pixel interleaving, sensor-domain averaging, integer residual
  translation search and local reference fallback.
- `pixelshift-io`: pinned rawler ARW decoder, ExifTool capture validation,
  normalized black/white levels, single D65 color rendering, ICC-tagged RGB16 TIFF,
  temporary-file publication with no overwrite. RAW decoder and output writer
  traits are extension boundaries.
- `pixelshift-cli`: strict versioned jobs, explicit synthetic input mode and
  structured results. The a7CR is the only registered camera.
- Lua: selection → job → process → TIFF import scaffold with protocol checks.

Next: real a7CR validation, tiled processing / memory budgets, subpixel alignment
and spatial motion masks, then **linear DNG** with camera calibration and source
metadata. Other cameras and pixel-shift formats get new profiles/decoders; none
are implemented now. Denoising a single Pixel Shift sequence only averages
redundant same-color observations (principally green); it cannot provide a
16-frame noise reduction gain for every RGB channel. Ordinary burst denoise
averages all aligned frames before color rendering.

The prototype keeps all decoded frames and the RGB result in memory. A 61 MP
16-shot capture requires roughly 7–10 GB including render buffers and decoder
overhead; use a 16 GB+ machine and expect a slow first implementation. It has no
out-of-core path or production performance claim. TIFF is classic TIFF, RGB16
with sRGB transfer and ICC profile; highlights are clipped, with no tone mapping.
The crop excludes a border of `4 + alignment_radius` sensor pixels on every side.
Orientation is preserved in a TIFF tag; broader metadata preservation is pending.

Read [dependency evaluation](docs/dependencies.md), [architecture](docs/architecture.md)
and [validation requirements](docs/validation.md) before calling a profile stable.
