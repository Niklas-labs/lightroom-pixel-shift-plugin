# Dependency evaluation — 2026-10-29

This evaluation inspected upstream documentation/source and the downloaded
**rawler 0.8.0** crate. Real a7CR files were not available for comparative runs.

| Project | Evidence and fit | Decision |
| --- | --- | --- |
| [make_arq](https://github.com/agriggio/make_arq) | Python/NumPy/tifffile + ExifTool, optional C acceleration. README documents 4/16 source shots, pseudo-ARQ that differs from Imaging Edge metadata and is incompatible with Sony software. Current source explicitly lists ILCE-7CR, validates SequenceNumber and maps Sony integer/half-pixel positions. Assumes direct access to uncompressed Sony strips; no local-motion or registration pipeline in the inspected merge routine. GPL-3.0-or-later. | Use as a shot-layout and optional numeric reference, not runtime dependency. No pseudo-ARQ output planned initially. Independent Rust reconstruction; GPL-compatible project license. |
| [rawler](https://github.com/dnglab/dnglab/tree/main/rawler) | Rust sensor decode API, Bayer layout, black/white levels, WB, matrices and Sony camera catalog. Published 0.8.0 includes `data/cameras/sony/a7cr.toml`, RGGB, D65/A calibration; library exposes `decode_file`. LGPL-2.1 package metadata; API explicitly not stable/SemVer. | Selected, pinned to `=0.8.0`, lockfile checked in. Default features disabled (unneeded Canon CRX support). Thin IO adapter isolates upstream API changes. Compiled adapter verified; compression variants still require real fixtures. |
| [DNGLab](https://github.com/dnglab/dnglab) | Rust application using rawler; converts RAW to DNG, has analyze/metadata/raw-pixel diagnostics and linear ARQ handling. README calls it alpha; conversion is not equivalent to registering and merging multiple ARWs. Application GPL-3.0, rawler LGPL-2.1. Upstream main differs from published rawler. | No subprocess dependency for merging. Use as validation tool and DNG design/reference for next milestone; do not pretend RAW→DNG conversion performs Pixel Shift reconstruction. |

Sources inspected:

- [make_arq README](https://github.com/agriggio/make_arq/blob/master/README.md),
  [Sony sequence and placement code](https://github.com/agriggio/make_arq/blob/master/make_arq.py)
- [DNGLab README](https://github.com/dnglab/dnglab/blob/main/README.md),
  [rawler manifest](https://github.com/dnglab/dnglab/blob/main/rawler/Cargo.toml),
  [published crate](https://crates.io/crates/rawler/0.8.0)

Rawler's catalog establishes decoder coverage, not verified pixel-shift behavior
for this engine. Likewise make_arq's a7CR entry is evidence for an initial shift
profile, not a substitute for comparing our TIFF to Imaging Edge.

The generated sRGB ICC asset was created with Pillow's LittleCMS binding; it is
a standard matrix/TRC profile, not a copied vendor profile. Regenerate with
`ImageCms.ImageCmsProfile(ImageCms.createProfile('sRGB')).tobytes()` if needed.
Dependency and distribution license/notice packaging is a release requirement.

## Installable alpha metadata decision

The package uses rawler's existing native Rust TIFF/EXIF implementation, with a
small Sony MakerNote adapter, rather than installing another generic EXIF parser
or executing ExifTool. ExifTool remains a development reference for Sony tag
semantics. Its [Sony tag definitions](https://github.com/exiftool/exiftool/blob/master/lib/Image/ExifTool/Sony.pm)
identify top-level MakerNote `0xb04a` as SequenceNumber and `0x202f` as the
six-byte PixelShiftInfo record (group ID, shot number, shot count).

This avoids a Perl/runtime dependency in the Lightroom distribution. It is a
camera-specific metadata subset, not a replacement for ExifTool's broad format
coverage. Synthetic TIFF/MakerNote tests establish parsing behavior; real a7CR
captures still need comparison against ExifTool and Sony Imaging Edge.
