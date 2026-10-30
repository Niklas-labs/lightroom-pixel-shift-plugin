# Camera profile configuration

`config/camera-profiles.json` is the source of truth for registered camera
capabilities. `config/camera-profiles.schema.json` is its JSON Schema (Draft
2020-12). Editors can use the catalog's relative `$schema` reference for completion
and validation. Runtime parsing rejects unknown fields and unsupported versions,
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

## Loading and distribution

The CLI loads `config/camera-profiles.json` beside the packaged `bin` directory.
Missing or invalid configuration in an installed plugin is an error. Standalone
development binaries fall back to the same JSON embedded at build time, ensuring
that they run outside the repository. For explicit testing:

```sh
sony-pixelshift --camera-profiles /path/to/camera-profiles.json info
sony-pixelshift --camera-profiles /path/to/camera-profiles.json cameras
sony-pixelshift --camera-profiles /path/to/camera-profiles.json merge --job job.json
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
production CLI uses the JSON profile for both detection and reconstruction.
