# Sony Pixel Shift for Lightroom Classic

An executable Rust prototype and Lightroom Classic Lua plugin for **Sony a7CR
(ILCE-7CR)** 4-shot / 16-shot Pixel Shift and multi-frame RAW denoise.
Camera profiles are experimental: synthetic reconstruction is tested; real-camera
and Lightroom host acceptance is still required. No original RAW is modified.

## Install the CI build in Lightroom Classic

Download the latest successful **Package Lightroom Plugin** workflow artifact:
`SonyPixelShift-macos-universal` (Apple Silicon and Intel) or
`SonyPixelShift-windows-x86_64`. Extract the GitHub artifact ZIP, then extract the
distribution ZIP inside it. Move `SonyPixelShift.lrplugin` to a permanent location
and add that directory using **File → Plug-in Manager → Add**.

The package embeds the matching Rust executable. No separate engine selection,
Rust, Python, Perl or ExifTool installation is required. CI development builds are
not publisher-signed/notarized; macOS may require approving the specific helper
in Privacy & Security. See [installation and testing](docs/lightroom-testing.md)
and [CI packaging](docs/packaging.md).

Select exactly four or sixteen original a7CR Pixel Shift ARWs, then choose
**Library → Plug-in Extras → Merge Sony a7CR Pixel Shift** and a new TIFF output.
The plugin checks its engine version, runs asynchronously and imports only a
validated completed result. A separate menu handles RAW burst denoise. Failed
jobs and diagnostic logs remain next to the chosen output for troubleshooting.

Start with four shots; sixteen shots still use the original high-memory engine.
DNG and tiled processing are not included in this first installable alpha.

## Build and run

Install stable Rust, then:

```sh
cargo build --release --locked
./target/release/sony-pixelshift info
./target/release/sony-pixelshift merge --job /photos/capture/job.json
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
lua scripts/test-plugin.lua
python3 scripts/test-package-unit.py
```

Example `job.json` (paths resolve against its directory):

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

Use `sixteen` with sixteen files for twice the sensor width and height before
cropping. `denoise` uses 2–64 RAW burst files. Pixel Shift ordering comes from
Sony metadata rather than filename order. Capture validation and native metadata
coverage are experimental; retain your originals and report unsupported cases.

The CLI prints progress/errors to stderr and a JSON completion record to stdout.
`info --output FILE` writes the version/protocol handshake. Optional merge flags
`--result FILE --log FILE` persist a completion/error record and diagnostics for
the plugin. Logs append to their destination; result and image files are never
overwritten. Exit codes: 0 success, 1 processing error, 2 argument error.

For an independent small demonstration without RAW files:

```sh
python3 scripts/generate-demo.py /tmp/pixelshift-demo
cargo run --locked -p pixelshift-cli -- merge --fixtures --job /tmp/pixelshift-demo/job.json
```

`--fixtures` explicitly enables synthetic JSON sensor inputs; the Lightroom
plugin never enables it. See integration tests for the schema.

## Implemented and next

- `pixelshift-core`: camera profile seam, Bayer sample reconstruction, 4-shot full
  color, 16-shot half-pixel interleaving, sensor-domain averaging, integer residual
  translation search and local reference fallback.
- `pixelshift-io`: pinned rawler ARW decoder, native capture metadata validation,
  normalized black/white levels, single D65 color rendering, ICC-tagged RGB16 TIFF,
  temporary-file publication with no overwrite. RAW decoder and output writer
  traits are extension boundaries.
- `pixelshift-cli`: strict versioned jobs, explicit synthetic input mode and
  structured results. The a7CR is the only registered camera.
- Lua: bundled-engine discovery, version checks, selection → job → process → TIFF
  import, persistent diagnostics and mocked SDK workflow checks.

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
