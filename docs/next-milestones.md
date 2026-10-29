# Next milestones: distribution, linear DNG, and bounded-memory reconstruction

Planning draft, 2026-10-29. These are proposed changes, not features already
implemented. Scope remains Sony a7CR four-shot / sixteen-shot Pixel Shift and
ordinary RAW burst denoise. Other camera profiles and formats retain extension
points only.

## Outcomes and constraints

- Install by extracting a platform ZIP and adding its `.lrplugin` directory in
  Lightroom Classic. No Rust, Python, Perl, ExifTool, or PATH setup by the user.
- Make validated linear DNG the preferred Lightroom output; retain RGB16 TIFF.
- Reconstruct without allocating a full-resolution output buffer. Keep source
  data packed as u16 and normalize only requested regions.
- Use the same numerical pipeline for disk and RAM storage. Resource preferences
  must not silently lower image quality, change alignment, or drop frames.
- Make all large queues and caches bounded, including decode and compression.
- Treat one whole-frame RAW decode as the initial memory floor. Profile upstream
  decoder allocations and mapped-input residency before promising peak limits.
- No claim of production camera support until real captures and Lightroom host
  tests pass. The initial GitHub Validate workflow passed on its configured OSes.

## 1. Establish capture correctness and remove runtime tool dependencies

Assemble a private reference corpus: real a7CR 4/16-shot sets, a normal burst,
supported compression variants, static fine detail, motion, subpixel camera drift,
high ISO, portrait orientation, and deliberately mixed/incomplete captures. Keep
fixture manifests/checksums in git; originals outside git. Use ExifTool and Sony
Imaging Edge as development references, not assumed installed runtime tools.

Introduce CaptureMetadata and SensorCalibration separately from rendered color.
Retain CFA origin, active area, crop, black/white levels, original matrices and
illuminants, as-shot neutral/WB, orientation, exposure, lens, serial, capture group,
shot count, and shot index. Confirm which Sony fields identify a true Pixel Shift
capture; do not infer it from four or sixteen consecutive filenames or sequence
numbers. Normal-burst denoise needs its own ordering rules and must not require
Pixel Shift sequence tags.

First do a focused feasibility check for native metadata extraction using
rawler's TIFF infrastructure, with an isolated Sony MakerNote adapter or a small
upstream extension. The published ARW decoder's MakerNote state is private and
does not expose a ready-made Pixel Shift capture API; this is actual work.
Compare extraction against the reference tools and real files.

Preferred release path: all required runtime metadata parsing in Rust. If that
spike cannot establish correct extraction promptly, explicitly choose a bundled,
version-pinned ExifTool plus its complete runtime as an interim package. Packaging
only the script or relying on macOS system Perl is insufficient. Either approach
must pass a clean-machine, offline test with no developer tools on PATH.

Exit: reject mixed/non-Pixel-Shift sequences correctly, preserve required DNG
calibration, and have a concrete self-contained runtime dependency inventory.

## 2. Produce installable artifacts in CI

Add a package assembler shared by local development and CI. It creates:

```
SonyPixelShift.lrplugin/
  Info.lua
  ...Lua files...
  bin/sony-pixelshift       # macOS universal, or .exe on Windows
  manifest.json            # protocol, plugin/engine versions, build revision
  licenses/
```

Build macOS aarch64-apple-darwin and x86_64-apple-darwin release binaries and
combine them using lipo; build x86_64-pc-windows-msvc for Windows. Linux remains
an engine/test platform. Windows ARM is outside this milestone. Set and test the
minimum supported macOS deployment target; audit deployment/runtime dependencies
for each package. Preserve execute permission on macOS. No compilation optimized
only for the CI host CPU. If a bundled metadata tool is selected, include its
complete dependencies under the plugin directory and invoke it by absolute path.

Resolve the bundled helper relative to `_PLUGIN.path`, check its protocol and
version, and retain a developer executable override. Prefer a universal macOS
helper to trying to infer architecture from Lightroom/Rosetta. Validate launch
paths with spaces, Unicode, and shell metacharacters on both OSes.

CI packaging tests extract the ZIP and run the embedded helper in a clean test
environment on native target architectures, run small merge fixtures, inspect
output, and validate manifest contents, notices, and executable permissions.
Native Intel/Apple Silicon execution is separate from merely cross-compiling.
Do not claim automated SDK host tests: actual Lightroom menu, invocation and
catalog import checks are a separate acceptance gate.

Pull requests and main builds upload installable development artifacts; tagged
releases publish versioned ZIPs plus checksums. macOS signing/notarization and
Windows signing should be release stages using configured signing identities.
Label unsigned development artifacts clearly; do not conflate successful ZIP
assembly with Gatekeeper acceptance. Signed/notarized distribution needs the
appropriate credentials and a tested package/notarization route.

Exit: installation on clean target machines needs only Lightroom and the ZIP.

## 3. Refactor the pipeline around regions and bounded resource use

Replace full-frame normalized `Frame` ownership in the processing API with a
source interface that reads sensor regions into caller-owned buffers. Provide
RAM and disk implementations backed by the same packed u16 samples. Retain the
small whole-image implementation as a synthetic reference during the transition.

Pipeline:

1. Inspect metadata and geometry; estimate memory, temporary space, and output
   size. Validate the selected scratch volume and leave working-space headroom.
2. Decode one ARW at a time into packed u16 storage. Produce small guides and
   calibration metadata during ingestion, then release decoder/source buffers.
3. Estimate global alignment on low-resolution, alias-suppressed guides. Retain
   only small guide pyramids and transforms.
4. Schedule output regions against one global memory budget. Gather source
   windows including the geometric/interpolation/motion halo, merge in f32,
   encode, and release.
5. Write tiles/strips to a temporary output in bounded order. Retain only tile
   offsets/counts and small preview accumulators. Finalize metadata and publish
   with no overwrite only on success.

Prototype 1024-square output tiles, corresponding to roughly 512-square sensor
regions for sixteen-shot output. Benchmark 512/1024/2048 output tile sizes.
Compute halos from the actual transform bounds, reconstruction kernel, mask
neighborhood, and interpolation support; do not assume a magic fixed margin.
Fix guide/block grids in global image coordinates so tile boundaries cannot
change motion decisions or reconstruction.

Disk storage starts with uncompressed, lossless u16 data, explicit bounded reads
and a small reusable region cache. Compare row-major and blocked layouts using
actual sequential-decode and overlapped-read workloads before choosing a complex
format. Avoid re-decoding compressed ARWs per tile. Avoid whole-cache mmap as a
claimed memory cap: resident mappings and filesystem caches need measurement.

Use a bounded compute/compression queue and writer backpressure. One slow early
tile must not allow an unbounded collection of later compressed results. The
scheduler accounts for RAW decode, source windows, pyramids, masks, f32 output,
codec workspace, and queued compressed data. Coordinate Rayon/codec parallelism
to avoid nested thread-pool oversubscription. Decode processes are not initially
parallel in low-memory mode.

Exit: TIFF output streams; disk/RAM paths match the reference; edges and tile
sizes do not introduce seams; cancellation and disk-full failures leave no final
output or abandoned caches. Profile actual whole-frame decoder peaks, including
rawler's mapped input and internal parallel workspaces.

## 4. Add a tile-streaming linear DNG writer

Use a tile/strip sink contract: begin(metadata), write(region), finish(). A TIFF
renderer and a DNG writer consume the same scene-linear camera RGB regions. The
existing whole-image OutputWriter contract needs to evolve; simply adding a
second whole-image implementation would retain the large-memory problem.

The rawler 0.8.0 high-level RGB/DNG APIs accept whole images and its RGB16 helper
copies the input. Investigate its lower-level TIFF directory writing and LJPEG
codec for a streaming adapter or an upstream writer extension. Do not assume the
existing high-level API is memory-bounded. First prove streaming uncompressed
DNG, then add independently lossless-compressed tiles with bounded parallelism.

Store full-color LinearRaw camera RGB, without baked WB, sRGB conversion, gamma,
tone mapping, or display clipping. Use camera matrices in their original
convention, calibration illuminants, AsShotNeutral, crop/active-area, orientation,
black/white semantics, source provenance, and a small rendered preview. Do not
blindly copy CFA-specific tags, geometry-dependent opcodes, or stale MakerNotes
into a reconstructed full-color image.

Prefer broadly compatible 16-bit integer samples plus lossless compression if
validated scaling and a black pedestal preserve negative black noise, relevant
headroom, and numerical precision well below sensor noise. Establish these
semantics in the DNG spike; do not silently clamp or claim that lossless
compression eliminates integer quantization. If the range/precision requirement
cannot be met, evaluate 32-bit floating-point DNG and Adobe compatibility before
choosing. Do not embed all sixteen originals by default.

Validate container tags and raw sample roundtrips independently of preview
appearance. Require Adobe DNG validation and real Lightroom import/development,
including WB changes, exposure recovery, colors, masks, portrait orientation and
crop. Check file offset/size limits before writing; do not assume BigTIFF is a
transparent DNG interoperability fix.

Exit: DNG becomes the default only after validation. TIFF remains available.

## 5. Add motion correction that remains local and memory-bounded

Keep deliberate Pixel Shift displacement distinct from unintended motion. Start
with subpixel global translation on guide pyramids; add a global affine model
only if it improves real captures. Estimate local residual translation on a
coarse, globally anchored block grid, refine near each processing region, and
attach confidence from texture, residuals, and matching consistency. Defer dense
full-image optical flow, learned models, GPU support, and rolling-shutter models.

The current exact-grid sample selection cannot use fractional residual shifts
correctly: it would drop most samples and fall back. Implement CFA-aware warped
sample reconstruction at the same time as subpixel correction. Use noise and
alignment confidence to weight same-color observations; compare kernels against
static analytic detail and controlled warped captures. Guides may be demosaiced;
the merge must retain sensor-domain information rather than average sixteen
fully demosaiced images.

Build motion masks from alias-suppressed, registered guides with noise-aware
thresholds and outlier checks. Make masks consistent across tiles; apply spatial
cleanup, dilation around uncertain edges, and controlled feathering. Use a
reference frame where no reliable correspondence exists. The sixteen-shot
moving-region fallback has native-reference detail interpolated to the larger
grid; report it, without implying recovered super-resolution there.

Exit: recover known fractional shifts, retain static detail, reduce ghosts on
moving objects, handle low-texture/occlusion failures conservatively, and show no
tile seams. No motion confidence algorithm may be justified only by visual
inspection of one favorable scene.

## Resource preferences and UI

Expose three execution preferences plus optional scratch location and an
advanced memory budget. Quality/motion options remain separate.

| Preference | Behavior | Initial target, subject to measurement |
| --- | --- | --- |
| Auto (default) | Choose cache residency and bounded workers from available RAM, Lightroom headroom and job geometry | Avoid OS memory pressure; show estimate before starting |
| Low memory | Disk-backed sensor storage, small region cache, limited workers | About 512 MiB–1 GiB total peak for tested a7CR inputs |
| High performance | Keep packed source frames in RAM when budget permits; more bounded workers | Roughly 3–4 GiB for a 16-shot set, without a full RGB output allocation |

Sixteen 61 MP u16 frames occupy about 1.95 GB decimal (1.82 GiB). High-performance
mode therefore does not inherently need the current 7–10 GB float pipeline.
Whether it is faster depends on decode, motion, compression and scratch-device
speed. Benchmark rather than label RAM caching as a guaranteed speedup.

Both modes use deliberate application-managed caching, not OS swap. A low-memory
preference is initially a budget/target, not a hard cross-platform RSS guarantee;
refuse a known impossible budget before decoding. Do not silently exceed it to
honor high-performance mode. If available resources change, adjust cache
residency/concurrency or stop with an actionable message, without changing image
quality. Maintain one fixed accumulation order per output sample for reproducible
results across storage modes.

Update the versioned job protocol with output format, resource preference,
memory budget, scratch location, motion settings and progress/cancellation.
Validate versions and engine/plugin compatibility. The Lightroom dialog should
show output format, execution preference, estimated scratch use and destination.
Use progress events/pollable state plus an explicit cancellation signal; stop
between bounded work units and clean temporary data on errors and recovery after
a crash. A running whole-frame upstream decode may delay cancellation; expose
that limit until decode can be interrupted or isolated safely.

## Benchmark and acceptance matrix

- Test four-shot, sixteen-shot and denoise; static, motion, high ISO, supported
  ARW compression variants; native Apple Silicon, Intel and Windows x64.
- Compare RAM/disk stores, tile sizes and worker limits with the same numeric
  settings. Record output quality, wall time and per-stage time, peak RSS/private
  memory, temporary bytes, read/write volume and compression time.
- Run cold and warm caches and report SSD type, CPU, RAM and codec settings.
  Shared CI runner timing is a smoke/regression signal, not a product benchmark.
- Use synthetic full-size fixtures to exercise memory scaling in CI; validate
  actual decoder peaks with the private capture corpus on controlled machines.
- Require equivalent numerical output across storage modes and tile sizes,
  within explicitly justified floating-point/quantization tolerance. Compare
  decoded image samples, not compressed file bytes or preview appearance alone.
- Default worker/tile choices should lie on measured memory/time tradeoffs.
  Establish a speed regression limit after recording a reproducible baseline;
  do not invent a seconds-per-merge promise now.

## Suggested delivery sequence

1. Capture/metadata feasibility and benchmarks; package assembler and bundled
   launcher can proceed independently. Deliver a self-contained TIFF alpha once
   the metadata dependency decision and clean-machine test pass.
2. Region-source/sink interfaces, packed RAM/disk stores, tiled reference merge
   and streaming TIFF, with resource accounting and job lifecycle support.
3. Streaming linear DNG using that sink, validated against Adobe/Lightroom.
4. Subpixel reconstruction and local motion refinement using the same regions.
5. Tune Auto/Low memory/High performance against the real benchmark matrix; finish
   signing/notarization and host acceptance for the distributable release.

The DNG-format and subpixel-motion spikes should happen early enough to influence
the region and calibration contracts, even though production implementations
follow the tiled refactor. No other camera support is added in these milestones.
