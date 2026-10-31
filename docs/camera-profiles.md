# Camera profile configuration

`config/camera-profiles.yaml` is the source of truth for registered camera
capabilities. `config/camera-profiles.schema.json` is its JSON Schema (Draft
2020-12), validating the parsed YAML document. The schema itself stays JSON.
Editors can use the `yaml-language-server` directive at the top of the YAML file
for completion and validation. Runtime parsing rejects unknown fields and unsupported versions,
then validates model uniqueness, supported adapters, CFA and complete shift grids.
CI validates the shipped catalog with the full JSON Schema validator as well.

Only Sony a7CR is registered initially. A new entry authorizes that model for
processing; adding its name does not establish correct results. Require reference
captures and leave `validation_status` experimental until acceptance passes.

| Field | Meaning |
| --- | --- |
| `schema_version` | Configuration format version; currently 1 |
| `make`, `model` | Exact decoded camera identity; make currently `SONY` |
| `name` | Human-readable camera name |
| `validation_status` | `experimental` or `validated`; an evidence/review status, not a processing setting |
| `decoder` | Supported RAW adapter; currently `sony_arw` |
| `metadata` | Capture-tag interpretation; currently `sony_pixel_shift_v1` |
| `calibration_source` | Currently `raw_metadata_and_rawler_catalog` |
| `raw_normalization` | Optional guarded reference white scale and provenance for native-u16 samples |
| `dng_rendering` | Optional reference-backed baseline exposure, A/D65 camera-calibration and forward matrices, matching calibration signature, and provenance |
| `cfa` | Expected row-major uncropped 2x2 Bayer pattern |
| `denoise` | Whether ordinary burst noise reduction is allowed |
| `pixel_shift.four` | Optional four-shot layout, scale 1, exactly four distinct integer offsets |
| `pixel_shift.sixteen` | Optional sixteen-shot layout, scale 2, exactly sixteen distinct half-pixel offsets |
| `notes` | Layout provenance and validation limitations |

Omit unsupported Pixel Shift modes; do not use `null`. At least one Pixel Shift
layout is required in this schema. Pure burst-only camera registration can get a
separate capability change later. Model names must be unique across entries; this
cross-entry rule is enforced by the runtime in addition to JSON Schema checks.

Each offset is `[x, y]` in **source sensor pixels**, ordered by chronological
Sony source-shot index, starting at shot 1. A sample at `(x, y)` in a source with
offset `(dx, dy)` contributes at scene coordinate `(x + dx, y + dy)`. Positive x
is right and positive y down. The sixteen-shot output has twice the linear grid
density; offsets remain in source-pixel units. Deliberate Pixel Shift offsets are
separate from estimated unintended camera motion.

The current engine supports complete 2x2 integer grids for four shots and 4x4
half-pixel grids for sixteen shots. Supported coordinates are explicitly limited
by the schema and runtime. Different layouts, offset ranges, sensor types, output
scales, metadata formats or decoder adapters require implementation support and a
schema revision; configuration must not silently approximate them.

Sensor dimensions/active area, actual CFA, black/white levels, WB, orientation and
color matrices continue to come from the RAW decoder/catalog. Decoded CFA is
checked against the configured expectation. Do not copy guessed calibration or
61 MP dimensions into every model entry. Compression decoding remains rawler's
responsibility; reference-test the variants claimed for each model.

`dng_rendering` adds rendering metadata without changing those decoded values.
Its optional, complete record contains `source`, `baseline_exposure_ev`,
`calibration_signature`, `camera_calibration_a`, `camera_calibration_d65`,
`forward_matrix_a`, and `forward_matrix_d65`. Each matrix is nine row-major
numbers. Runtime validation requires finite invertible matrices, nonempty
provenance, an ASCII signature, and baseline exposure in [-5, 5] EV. Both DNG
calibration signatures receive the configured value so readers can apply the
camera-calibration matrices. A/D65 records follow their matching color-matrix
illuminants; a single-D65 output uses the D65 records in slot 1.

The initial a7CR record reproduces a user-supplied Lightroom-converted ARW
reference: +0.35 EV baseline, camera calibration diag(0.9487, 1, 1.0363), A/D65
forward matrices, and signature `com.adobe`. Sensor values, WB and original color
matrices stay unchanged. This is an experimental model-level reference from one
capture, not proof of every body's calibration or ISO-dependent behavior. Check
other bodies/ISO settings before declaring the model validated. No Adobe look
tables, tone curves or profile name are copied. TIFF and the approximate embedded
preview renderer do not use this DNG-only metadata.

`raw_normalization` contains `source`, `decoder_white_level`, and `white_level`.
The a7CR record uses decoder white 15360 and reference white 16383, established
by matching samples from an original ARW to its Adobe-converted mosaic DNG.
Native codes and decoder black levels are preserved; the effective denominator
becomes `16383 - black`. This aligns coordinate conventions with the +0.35 EV
Adobe baseline rather than applying an empirical rendered exposure correction.

The guard accepts exactly the configured decoder scale, or a decoder already
using the reference scale. Other decoder levels fail explicitly; do not reuse
this 14-bit reference for unverified RAW modes. Both storage backends and all
production merge modes apply the reference before alignment and reconstruction.
Float fixtures retain their supplied normalized values. Profiles without this
block keep decoder normalization. Sensor diagnostics report original
`decoder_white`, effective `white`, and provenance, using the same reference
application as production merging. Legacy decoder-only APIs remain unprofiled.

Profiles without `dng_rendering` retain identity camera calibration, no
forward matrices and 0 EV baseline. The schema remains version 1 with an additive
optional record; older catalog files remain valid with the new engine.

## Loading and distribution

The CLI loads `config/camera-profiles.yaml` beside the packaged `bin` directory.
Missing or invalid configuration in an installed plugin is an error. Standalone
development binaries fall back to the same YAML embedded at build time, ensuring
that they run outside the repository. For explicit testing:

```sh
sony-pixelshift --camera-profiles /path/to/camera-profiles.yaml info
sony-pixelshift --camera-profiles /path/to/camera-profiles.yaml cameras
sony-pixelshift --camera-profiles /path/to/camera-profiles.yaml merge --job job.json
python3 scripts/test-camera-schema.py
```

Both catalog and schema ship inside the `.lrplugin`. Ordinary users do not need
to edit them. Override files use the same strict validation; errors do not fall
back to the a7CR profile. `info` derives supported models from the loaded catalog.
Editing the catalog alone does not add a missing rawler decoder or calibration.

## Adding a model

1. Establish supported RAW variants, metadata layout, CFA origin, capture modes
   and chronological shifts from documented evidence and reference captures.
2. Add its exact identity and supported modes; cite layout evidence in `notes`.
3. Validate schema and runtime, then compare reconstruction, crop, color, motion
   and metadata against reference outputs. Confirm unsupported modes are rejected.
4. Package and test through Lightroom before changing validation status.

The original core SonyA7Cr implementation remains a regression reference. The
production CLI uses the YAML profile for both detection and reconstruction.

## YAML conventions

Use one document, string mapping keys, `true`/`false` for booleans, and compact
`[x, y]` coordinate pairs. Comments and folded text are supported. Duplicate
keys, custom tags, multiple documents, unknown fields and invalid values are
rejected. Merge keys (`<<`) are not a supported inheritance mechanism. Quote
strings that could be mistaken for numbers or booleans. `on`, `off`, `yes` and
`no` are strings rather than YAML 1.1 boolean shortcuts.

The Rust parser is `serde-saphyr`; no Python/YAML installation is needed by
Lightroom users. Python schema/package tooling uses PyYAML with matching boolean
and duplicate-key rules. Developer tooling requires `PyYAML==6.0.3` and
`jsonschema==4.26.0`, installed by CI. These do not ship as runtime dependencies.

Runtime parsing disables merge keys, custom tags, includes and property interpolation,
with duplicate-key errors, strict booleans and non-finite-value rejection.
