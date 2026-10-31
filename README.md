# Sony Pixel Shift for Lightroom Classic

An executable Rust prototype and Lightroom Classic Lua plugin for **Sony a7CR
(ILCE-7CR)** 4-shot / 16-shot Pixel Shift and multi-frame RAW denoise.
Camera profiles are experimental: synthetic reconstruction is tested; real-camera
and Lightroom host acceptance is still required. No original RAW is modified.

## Install the CI build in Lightroom Classic

Download the latest successful **Package Lightroom Plugin** workflow artifact:
`SonyPixelShift-<version>-macos-universal.zip` (Apple Silicon and Intel) or
`SonyPixelShift-<version>-windows-x86_64.zip`. Extract the downloaded ZIP once; `SonyPixelShift.lrplugin` is at its root. Move `SonyPixelShift.lrplugin` to a permanent location
and add that directory using **File → Plug-in Manager → Add**.

The package embeds the matching Rust executable. No separate engine selection,
Rust, Python, Perl or ExifTool installation is required. The macOS publishing
workflow requires Developer ID signing and Apple notarization acceptance. Older
or local development artifacts may still require approving the specific helper
in Privacy & Security; check INSTALL.txt and the package manifest. See [installation and testing](docs/lightroom-testing.md)
and [CI packaging](docs/packaging.md).

Select exactly four or sixteen original a7CR Pixel Shift ARWs, then choose
**Library → Plug-in Extras → Pixel Shift: more detail (4 or 16 RAWs)**.
The result is saved beside the sources, for example
`DSC01582-DSC01585-PixelShift.dng`; existing files receive a unique suffix.
Select sources from one folder. Burst results use `-NoiseReduced.dng`.
The plugin checks its engine version, runs asynchronously and imports only a
validated completed result, confirms its catalog entry and attempts to select it
without changing your library filters or current collection. A separate menu
handles RAW burst denoise. Failed jobs and diagnostic logs remain beside the
output for troubleshooting.

Start with four shots. Choose **Memory: Disk (less RAM)** to store sources temporarily on disk.
The plugin now outputs experimental linear DNG. Adaptive Color/B&W and AI Denoise
compatibility need Lightroom host testing. Output reconstruction and source normalization use bounded regions with packed RAM/disk input storage.
Lossless Deflate compression is now the default. Uncompressed equivalents are
about 0.73 GB at 61 MP or 2.9 GB at 244 MP; compressed size depends on content.

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
  "camera": "auto",
  "mode": "auto",
  "inputs": ["DSC00001.ARW", "DSC00002.ARW", "DSC00003.ARW", "DSC00004.ARW"],
  "output": "merged.dng",
  "motion_threshold": 0.08,
  "alignment_radius": 2
}
```

Camera and mode may be omitted or set to `auto`. The engine validates one complete
4/16-shot a7CR capture from native RAW headers before decoding sensor data. Other
Sony models remain unsupported. Explicit camera/mode values are checked against
metadata. Ordinary bursts require explicit `denoise`; they are never a fallback.

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

## Camera configuration

Registered model capabilities and chronological shift patterns live in
[config/camera-profiles.yaml](config/camera-profiles.yaml), with a
[JSON Schema](config/camera-profiles.schema.json). Detection and reconstruction
use the same validated catalog, bundled inside the plugin. See
[the configuration guide](docs/camera-profiles.md) for fields, loading rules and
adding a model. Only the a7CR is registered initially; RAW calibration stays with
the decoder.

## Implemented and next

See [ROADMAP.md](ROADMAP.md) for prioritized follow-ups and acceptance criteria,
including other Sony models, memory budgets, motion reconstruction and DNG validation.

- `pixelshift-core`: camera profile seam, Bayer sample reconstruction, 4-shot full
  color, 16-shot half-pixel interleaving, sensor-domain averaging, integer residual
  translation search and local reference fallback.
- `pixelshift-io`: pinned rawler ARW decoder, native capture metadata validation,
  normalized black/white levels, calibrated float32 linear DNG, optional sRGB RGB16 TIFF,
  temporary-file publication with no overwrite. RAW decoder and output writer
  traits are extension boundaries.
- `pixelshift-cli`: strict versioned jobs, explicit synthetic input mode and
  structured results. The a7CR is the only registered camera.
- Lua: bundled-engine discovery, version checks, selection → job → process → DNG
  import, persistent diagnostics and mocked SDK workflow checks.

Next: real a7CR validation, tiled processing / memory budgets, subpixel alignment
and spatial motion masks, plus Lightroom validation of compressed **linear DNG**. Other cameras and pixel-shift formats get new profiles/decoders; none
are implemented now. Denoising a single Pixel Shift sequence only averages
redundant same-color observations (principally green); it cannot provide a
16-frame noise reduction gain for every RGB channel. Ordinary burst denoise
averages all aligned frames before color rendering.

The CLI stores native `u16` sensor samples in RAM (default) or temporary disk
files, then normalizes source row windows and streams output regions. Sixteen
61 MP sources occupy about 1.95 GB of packed RAM, or that amount of scratch disk.
Disk mode retains source windows and one decoder's working memory instead of all
sources. Neither mode allocates a complete reconstructed RGB buffer. Whole-frame
RAW decoding and allocator overhead remain; a hard memory budget and full-size
peak measurements are pending. See [streaming and measurements](docs/streaming.md). TIFF is classic TIFF, RGB16
with sRGB transfer and ICC profile; highlights are clipped, with no tone mapping.
The merged buffer excludes a border of `4 + alignment_radius` sensor pixels on
all sides. DNG DefaultCrop additionally maps the source decoder's preferred crop
into the output grid, accounting for the reference shift and sixteen-shot scale.
Only the common valid region is displayed, so edge loss can make it slightly
smaller than the ARW's visible area. If the decoder has no preferred crop, the
valid merged buffer is used; no model-specific centered crop is guessed.
Orientation and available standard camera/lens/exposure EXIF are preserved. GPS,
serial numbers, private MakerNotes and source geometry tags are not copied.
Lens identification does not unlock Adobe lens profiles restricted to RAW files;
do not select an unrelated lens to compensate. Lens identity stored only in
proprietary Sony tags still needs a resolver. The TIFF renderer does not apply
Adobe camera profiles, tone curves, lens corrections or source Lightroom edits,
so its appearance can differ from the ARWs. Linear DNG preserves scene-linear
camera RGB, reciprocal as-shot WB, A/D65 matrices where available, black/white levels, orientation,
output crop and standard source EXIF. Only its small preview is developed to sRGB.
It reconstructs bounded regions directly into lossless float32 strips and
collects a small preview from those pixels. TIFF is streamed similarly. Sources
are read through interchangeable packed RAM/disk stores during reconstruction.
Use a `.tif`/`.tiff` output path in CLI jobs for the existing TIFF renderer.
Adaptive profile, AI Denoise and lens-correction compatibility must be tested in
Lightroom. DNG does not contain copied Sony MakerNotes or correction opcodes.

Read [dependency evaluation](docs/dependencies.md), [architecture](docs/architecture.md)
and [validation requirements](docs/validation.md) before calling a profile stable.

macOS CI uses Developer ID signing through [OpenBao and GitHub OIDC](docs/openbao-signing.md).
It reads separate P12 and notarization records and requires Apple acceptance
before publishing macOS artifacts. Online Gatekeeper verification remains part
of normal browser-download testing.
