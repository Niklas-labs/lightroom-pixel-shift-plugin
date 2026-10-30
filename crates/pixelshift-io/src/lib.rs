//! Decoder and output boundaries. No RAW development is performed before merging.
use anyhow::{Context, Result, bail, ensure};
use pixelshift_core::{Frame, Image};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};
use tiff::encoder::{TiffEncoder, colortype};

mod geometry;
pub use geometry::{Crop, map_reference_crop};

mod dng;
pub use dng::{DngWriter, write_dng};

mod metadata;
pub mod profiles;
pub use metadata::{PixelShiftInfo, inspect_capture, metadata_backend};

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Capture {
    #[serde(default)]
    pub photo_metadata: PhotoMetadata,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub dimensions: Option<(usize, usize)>,
    #[serde(rename = "SequenceNumber")]
    pub sequence: Option<usize>,
    #[serde(rename = "ISO")]
    pub iso: Option<f64>,
    #[serde(rename = "ExposureTime")]
    pub exposure: Option<f64>,
    #[serde(rename = "FNumber")]
    pub aperture: Option<f64>,
    #[serde(rename = "SerialNumber", default, deserialize_with = "serial_number")]
    pub serial: Option<String>,
    #[serde(default)]
    pub pixel_shift: Option<PixelShiftInfo>,
}
fn serial_number<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        None => Ok(None),
        Some(serde_json::Value::String(value)) => Ok(Some(value)),
        Some(serde_json::Value::Number(value)) => Ok(Some(value.to_string())),
        _ => Err(serde::de::Error::custom("invalid camera serial number")),
    }
}
/// Whitelisted source EXIF. Geometry-dependent RAW tags and private MakerNotes
/// must not be copied into a cropped, rendered image.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PhotoMetadata {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens_make: Option<String>,
    pub lens_model: Option<String>,
    pub lens_spec: Option<[rawler::formats::tiff::Rational; 4]>,
    pub focal_length: Option<rawler::formats::tiff::Rational>,
    pub exposure_time: Option<rawler::formats::tiff::Rational>,
    pub fnumber: Option<rawler::formats::tiff::Rational>,
    pub iso: Option<u32>,
    pub date_time_original: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Color {
    /// Display crop in reconstructed output pixels, not source sensor pixels.
    #[serde(default)]
    pub default_crop: Option<Crop>,
    #[serde(default)]
    pub photo_metadata: PhotoMetadata,
    pub white_balance: [f32; 3],
    /// XYZ D65 to camera, row major.
    pub xyz_to_camera: [f32; 9],
    /// Optional XYZ -> camera calibration under standard illuminant A (tungsten).
    /// DNG uses this alongside D65 for illuminant interpolation.
    #[serde(default)]
    pub xyz_to_camera_a: Option<[f32; 9]>,
    #[serde(default = "normal_orientation")]
    pub orientation: u16,
}
fn normal_orientation() -> u16 {
    1
}
pub struct Decoded {
    pub frame: Frame,
    pub color: Color,
    signature: Option<Capture>,
    source_crop: Option<Crop>,
}
impl Decoded {
    pub fn source_crop(&self) -> Option<Crop> {
        self.source_crop
    }
    pub fn capture_metadata(&self) -> Option<&Capture> {
        self.signature.as_ref()
    }
    pub fn from_sensor(frame: Frame, color: Color) -> Result<Self> {
        frame.validate()?;
        Ok(Self {
            frame,
            color,
            signature: None,
            source_crop: None,
        })
    }
    pub fn compatible(&self, other: &Self) -> bool {
        if self.source_crop != other.source_crop {
            return false;
        }
        match (&self.signature, &other.signature) {
            (Some(a), Some(b)) => {
                a.iso == b.iso
                    && a.exposure == b.exposure
                    && a.aperture == b.aperture
                    && a.serial == b.serial
                    && a.pixel_shift.as_ref().map(|v| (v.group_id, v.shot_count))
                        == b.pixel_shift.as_ref().map(|v| (v.group_id, v.shot_count))
            }
            (None, None) => self.color == other.color,
            _ => false,
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    #[serde(default)]
    pub source_crop: Option<Crop>,
    pub frame: Frame,
    pub color: Color,
}

pub fn decode(path: &Path, fixture: bool) -> Result<Decoded> {
    if fixture {
        let input: Fixture = serde_json::from_reader(File::open(path)?)?;
        input.frame.validate()?;
        return Ok(Decoded {
            frame: input.frame,
            color: input.color,
            signature: None,
            source_crop: input.source_crop,
        });
    }
    ensure!(
        path.extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("arw")),
        "only ARW input is supported (fixtures require --fixtures)"
    );
    let capture = inspect_capture(path)
        .with_context(|| format!("reading native EXIF from {}", path.display()))?;
    // A missing sequence is retained as zero for ordinary burst denoise. Pixel
    // Shift processing still requires a complete ordered 1..N sequence.
    let sequence = capture.sequence.unwrap_or(0);
    let raw = rawler::decode_file(path).with_context(|| format!("decoding {}", path.display()))?;
    ensure!(
        raw.cpp == 1 && raw.make.eq_ignore_ascii_case("SONY"),
        "RAW adapter supports Sony Bayer ARW only"
    );
    let config = match &raw.photometric {
        rawler::rawimage::RawPhotometricInterpretation::Cfa(c) => c,
        _ => bail!("expected Bayer sensor data"),
    };
    ensure!(
        config.cfa.width == 2 && config.cfa.height == 2,
        "expected 2x2 Bayer"
    );
    let cfa = std::array::from_fn(|i| config.cfa.color_at(i / 2, i % 2));
    let black = raw.blacklevel.as_bayer_array();
    let white = raw.whitelevel.as_bayer_array();
    ensure!((0..4).all(|i| white[i] > black[i]), "invalid RAW levels");
    let data = match raw.data {
        rawler::RawImageData::Integer(data) => data,
        _ => bail!("floating RAW unsupported"),
    };
    let pixels = data
        .into_iter()
        .enumerate()
        .map(|(i, v)| {
            let phase = ((i / raw.width) % 2) * 2 + (i % raw.width) % 2;
            (v as f32 - black[phase]) / (white[phase] - black[phase])
        })
        .collect();
    // Rawler matrices are XYZ -> camera. Keep D65 for rendering and include
    // illuminant A, when available, for dual-illuminant DNG interpolation.
    let matrix = raw
        .camera
        .color_matrix
        .get(&rawler::imgop::xyz::Illuminant::D65)
        .context("missing D65 camera matrix")?;
    ensure!(matrix.len() == 9, "expected 3x3 camera matrix");
    let xyz_to_camera_a = raw
        .camera
        .color_matrix
        .get(&rawler::imgop::xyz::Illuminant::A)
        .map(|matrix| {
            matrix
                .clone()
                .try_into()
                .map_err(|_| anyhow::anyhow!("expected 3x3 illuminant A camera matrix"))
        })
        .transpose()?;
    let source_crop = raw.crop_area.or(raw.active_area).map(|area| Crop {
        x: area.p.x,
        y: area.p.y,
        width: area.d.w,
        height: area.d.h,
    });
    if let Some(crop) = source_crop {
        crop.validate(raw.width, raw.height)?;
    }
    let color = Color {
        default_crop: None,
        xyz_to_camera_a,
        photo_metadata: capture.photo_metadata.clone(),
        orientation: raw.orientation.to_u16(),
        white_balance: [raw.wb_coeffs[0], raw.wb_coeffs[1], raw.wb_coeffs[2]],
        xyz_to_camera: matrix
            .clone()
            .try_into()
            .map_err(|_| anyhow::anyhow!("bad matrix"))?,
    };
    Ok(Decoded {
        frame: Frame {
            width: raw.width,
            height: raw.height,
            pixels,
            cfa,
            model: raw.model,
            sequence,
        },
        color,
        signature: Some(capture),
        source_crop,
    })
}

fn inverse(m: [f32; 9]) -> Result<[f32; 9]> {
    let [a, b, c, d, e, f, g, h, i] = m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    ensure!(det.is_finite() && det.abs() > 1e-8, "singular color matrix");
    Ok([
        e * i - f * h,
        c * h - b * i,
        b * f - c * e,
        f * g - d * i,
        a * i - c * g,
        c * d - a * f,
        d * h - e * g,
        b * g - a * h,
        a * e - b * d,
    ]
    .map(|v| v / det))
}
/// Convert linear camera RGB to display sRGB; normalize matrix rows for neutral response.
/// This initial single-illuminant development has no tone curve beyond the sRGB transfer.
pub fn render_srgb(image: &Image, color: &Color) -> Result<Vec<u16>> {
    ensure!(
        image
            .width
            .checked_mul(image.height)
            .and_then(|n| n.checked_mul(3))
            == Some(image.rgb.len()),
        "invalid RGB buffer"
    );
    ensure!(
        image.rgb.iter().all(|v| v.is_finite()),
        "non-finite RGB sample"
    );
    ensure!((1..=8).contains(&color.orientation), "invalid orientation");
    ensure!(
        color
            .white_balance
            .iter()
            .all(|v| v.is_finite() && *v > 0.0),
        "invalid white balance"
    );
    ensure!(
        color.xyz_to_camera.iter().all(|v| v.is_finite()),
        "invalid color matrix"
    );
    let mut xyz_to_cam = color.xyz_to_camera;
    // D65 white normalization, as camera matrices include channel sensitivity.
    for row in xyz_to_cam.chunks_mut(3) {
        let sum = row[0] * 0.95047 + row[1] + row[2] * 1.08883;
        ensure!(sum.abs() > 1e-8, "invalid matrix row");
        for v in row {
            *v /= sum;
        }
    }
    let inv = inverse(xyz_to_cam)?;
    let xyz_to_srgb = [
        3.2404542, -1.5371385, -0.4985314, -0.969266, 1.8760108, 0.041556, 0.0556434, -0.2040259,
        1.0572252,
    ];
    let mut transform = [0.0; 9];
    for r in 0..3 {
        for c in 0..3 {
            for k in 0..3 {
                transform[r * 3 + c] += xyz_to_srgb[r * 3 + k] * inv[k * 3 + c];
            }
        }
    }
    let wb = color.white_balance.map(|v| v / color.white_balance[1]);
    let mut output = Vec::with_capacity(image.rgb.len());
    for pixel in image.rgb.as_chunks::<3>().0 {
        for r in 0..3 {
            let linear = (0..3)
                .map(|c| transform[r * 3 + c] * pixel[c] * wb[c])
                .sum::<f32>()
                .clamp(0.0, 1.0);
            let encoded = if linear <= 0.0031308 {
                12.92 * linear
            } else {
                1.055 * linear.powf(1.0 / 2.4) - 0.055
            };
            output.push((encoded * 65535.0).round() as u16);
        }
    }
    Ok(output)
}

/// TIFF/DNG requires out-of-line values at word-aligned offsets. The TIFF
/// encoder currently leaves consecutive ASCII payloads unpadded.
fn write_aligned_tag<W: Write + std::io::Seek, T: tiff::encoder::TiffValue>(
    directory: &mut tiff::encoder::DirectoryEncoder<'_, W, tiff::encoder::TiffKindStandard>,
    tag: tiff::tags::Tag,
    value: T,
) -> Result<()> {
    if directory.write_data(&[] as &[u8])? % 2 != 0 {
        directory.write_data(&[0u8][..])?;
    }
    directory.write_tag(tag, value)?;
    Ok(())
}

/// Shared, whitelisted EXIF. DNG camera channels are uncalibrated (65535),
/// whereas rendered TIFF explicitly carries sRGB (1).
fn write_exif<W: Write + std::io::Seek>(
    encoder: &mut TiffEncoder<W>,
    metadata: &PhotoMetadata,
    color_space: u16,
) -> Result<u32> {
    let mut exif = encoder.extra_directory()?;
    let rational = |v: rawler::formats::tiff::Rational| tiff::encoder::Rational { n: v.n, d: v.d };
    for (tag, value) in [
        (0x829a, metadata.exposure_time),
        (0x829d, metadata.fnumber),
        (0x920a, metadata.focal_length),
    ] {
        if let Some(value) = value.filter(|v| v.d != 0) {
            write_aligned_tag(&mut exif, tiff::tags::Tag::Unknown(tag), rational(value))?;
        }
    }
    if let Some(values) = metadata
        .lens_spec
        .filter(|values| values.iter().all(|v| v.d != 0))
    {
        write_aligned_tag(
            &mut exif,
            tiff::tags::Tag::Unknown(0xa432),
            &values.map(rational)[..],
        )?;
    }
    for (tag, value) in [
        (0xa433, &metadata.lens_make),
        (0xa434, &metadata.lens_model),
        (0x9003, &metadata.date_time_original),
    ] {
        if let Some(value) = value {
            write_aligned_tag(&mut exif, tiff::tags::Tag::Unknown(tag), value.as_str())?;
        }
    }
    if let Some(iso) = metadata.iso {
        write_aligned_tag(
            &mut exif,
            tiff::tags::Tag::Unknown(0x8827),
            iso.min(u16::MAX as u32) as u16,
        )?;
        if iso >= u16::MAX as u32 {
            write_aligned_tag(&mut exif, tiff::tags::Tag::Unknown(0x8830), 3u16)?;
            write_aligned_tag(&mut exif, tiff::tags::Tag::Unknown(0x8833), iso)?;
        }
    }
    write_aligned_tag(&mut exif, tiff::tags::Tag::Unknown(0xa001), color_space)?;
    if exif.write_data(&[] as &[u8])? % 2 != 0 {
        exif.write_data(&[0u8][..])?;
    }
    Ok(exif.finish_with_offsets()?.offset)
}

/// Same-directory temporary file + no-clobber publication. Never modifies a source file.
pub fn write_tiff(path: &Path, image: &Image, color: &Color) -> Result<()> {
    ensure!(
        path.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("tif") || e.eq_ignore_ascii_case("tiff")),
        "TIFF writer requires a .tif or .tiff extension"
    );
    ensure!(!path.exists(), "output already exists");
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    let data = render_srgb(image, color)?;
    {
        let mut writer = BufWriter::new(temporary.as_file_mut());
        {
            let mut encoder = TiffEncoder::new(&mut writer)?;
            let metadata = &color.photo_metadata;
            let exif_offset = write_exif(&mut encoder, metadata, 1)?;
            let mut output = encoder
                .new_image::<colortype::RGB16>(image.width.try_into()?, image.height.try_into()?)?;
            output
                .encoder()
                .write_tag(tiff::tags::Tag::ExifDirectory, exif_offset)?;
            for (tag, value) in [
                (tiff::tags::Tag::Make, &metadata.make),
                (tiff::tags::Tag::Model, &metadata.model),
            ] {
                if let Some(value) = value {
                    output.encoder().write_tag(tag, value.as_str())?;
                }
            }
            output
                .encoder()
                .write_tag(tiff::tags::Tag::Orientation, color.orientation)?;
            output.encoder().write_tag(
                tiff::tags::Tag::ImageDescription,
                "Sony Pixel Shift experimental; sRGB IEC 61966-2-1; no tone mapping",
            )?;
            output.encoder().write_tag(
                tiff::tags::Tag::IccProfile,
                &include_bytes!("../assets/srgb.icc")[..],
            )?;
            output
                .encoder()
                .write_tag(tiff::tags::Tag::Software, "sony-pixelshift 0.1.0")?;
            output.write_data(&data)?;
        }
        writer.flush()?;
    }
    temporary.as_file().sync_all()?;
    temporary.persist_noclobber(path).map_err(|e| e.error)?;
    Ok(())
}

/// Future formats plug into this boundary without changing sensor reconstruction.
pub trait RawDecoder {
    fn decode(&self, path: &Path) -> Result<Decoded>;
}
pub struct SonyArwDecoder;
impl RawDecoder for SonyArwDecoder {
    fn decode(&self, path: &Path) -> Result<Decoded> {
        decode(path, false)
    }
}
/// Output writers receive the scene-linear image and camera calibration.
pub trait OutputWriter {
    fn write(&self, path: &Path, image: &Image, color: &Color) -> Result<()>;
}
pub struct TiffWriter;
impl OutputWriter for TiffWriter {
    fn write(&self, path: &Path, image: &Image, color: &Color) -> Result<()> {
        write_tiff(path, image, color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn calibration() -> Color {
        Color {
            xyz_to_camera_a: None,
            default_crop: None,
            photo_metadata: PhotoMetadata::default(),
            white_balance: [1.0; 3],
            xyz_to_camera: [
                0.82, -0.2976, -0.0719, -0.4296, 1.2053, 0.2532, -0.0429, 0.1282, 0.5774,
            ],
            orientation: 6,
        }
    }
    #[test]
    fn color_render_and_orientation_survive_tiff_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("portrait.tiff");
        let image = Image {
            width: 2,
            height: 2,
            rgb: vec![0.18; 12],
        };
        let color = calibration();
        let rendered = render_srgb(&image, &color).unwrap();
        // A neutral input must render neutral gray near the sRGB 18% value.
        assert!((rendered[0] as i32 - 30235).abs() < 15);
        assert!((rendered[0] as i32 - rendered[1] as i32).abs() < 15);
        write_tiff(&path, &image, &color).unwrap();
        let mut tiff = tiff::decoder::Decoder::new(File::open(path).unwrap()).unwrap();
        assert_eq!(tiff.get_tag_u32(tiff::tags::Tag::Orientation).unwrap(), 6);
    }
    #[test]
    fn source_exif_survives_without_private_or_geometry_tags() {
        use rawler::{
            exif::Exif,
            formats::tiff::{GenericTiffReader, Rational, reader::TiffReader},
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metadata.tif");
        let mut color = calibration();
        color.photo_metadata = PhotoMetadata {
            make: Some("SONY".into()),
            model: Some("ILCE-7CR".into()),
            lens_make: Some("Sony".into()),
            lens_model: Some("FE 20-70mm F4 G".into()),
            lens_spec: Some([
                Rational::new(20, 1),
                Rational::new(70, 1),
                Rational::new(4, 1),
                Rational::new(4, 1),
            ]),
            focal_length: Some(Rational::new(35, 1)),
            exposure_time: Some(Rational::new(1, 100)),
            fnumber: Some(Rational::new(8, 1)),
            iso: Some(100),
            date_time_original: Some("2026:10:29 12:00:00".into()),
        };
        write_tiff(
            &path,
            &Image {
                width: 2,
                height: 2,
                rgb: vec![0.18; 12],
            },
            &color,
        )
        .unwrap();
        let tiff =
            GenericTiffReader::new(&mut File::open(path).unwrap(), 0, 0, Some(4), &[]).unwrap();
        let root = tiff.root_ifd();
        assert_eq!(
            root.get_entry(0x010fu16)
                .unwrap()
                .value
                .as_string()
                .unwrap()
                .trim(),
            "SONY"
        );
        assert_eq!(
            root.get_entry(0x0110u16)
                .unwrap()
                .value
                .as_string()
                .unwrap()
                .trim(),
            "ILCE-7CR"
        );
        let exif = Exif::new(root).unwrap();
        let source = &color.photo_metadata;
        assert_eq!(exif.lens_make, source.lens_make);
        assert_eq!(exif.lens_model, source.lens_model);
        assert_eq!(exif.lens_spec, source.lens_spec);
        assert_eq!(exif.focal_length, source.focal_length);
        assert_eq!(exif.exposure_time, source.exposure_time);
        assert_eq!(exif.fnumber, source.fnumber);
        assert_eq!(exif.iso_speed_ratings, Some(100));
        assert_eq!(exif.date_time_original, source.date_time_original);
        assert_eq!(exif.color_space, Some(1));
        assert!(root.find_first_ifd_with_tag(0x927cu16).is_none()); // MakerNote
        assert!(root.find_first_ifd_with_tag(0x8825u16).is_none()); // GPS
        assert!(root.find_first_ifd_with_tag(0xa002u16).is_none()); // stale dimensions
    }
    #[test]
    fn rejects_invalid_render_data_and_dng_extension() {
        let dir = tempfile::tempdir().unwrap();
        let color = calibration();
        let mut image = Image {
            width: 1,
            height: 1,
            rgb: vec![0.2; 3],
        };
        assert!(write_tiff(&dir.path().join("wrong.dng"), &image, &color).is_err());
        image.rgb[0] = f32::NAN;
        assert!(render_srgb(&image, &color).is_err());
        image.rgb.clear();
        assert!(render_srgb(&image, &color).is_err());
        let mut bad = color;
        bad.xyz_to_camera = [0.0; 9];
        assert!(
            render_srgb(
                &Image {
                    width: 1,
                    height: 1,
                    rgb: vec![0.2; 3]
                },
                &bad
            )
            .is_err()
        );
    }
    #[test]
    fn capture_signatures_reject_mixed_exposure() {
        let parse = |s: &str| serde_json::from_str::<Capture>(s).unwrap();
        let a = parse(
            r#"{"SequenceNumber":1,"ISO":100,"ExposureTime":0.01,"FNumber":8,"SerialNumber":"x"}"#,
        );
        let b = parse(
            r#"{"SequenceNumber":2,"ISO":200,"ExposureTime":0.01,"FNumber":8,"SerialNumber":"x"}"#,
        );
        let make = |capture| Decoded {
            frame: Frame {
                width: 8,
                height: 8,
                pixels: vec![0.2; 64],
                cfa: [0, 1, 1, 2],
                model: "ILCE-7CR".into(),
                sequence: 1,
            },
            color: calibration(),
            signature: Some(capture),
            source_crop: None,
        };
        let numeric = parse(r#"{"SerialNumber":12345}"#);
        assert_eq!(numeric.serial.as_deref(), Some("12345"));
        let first = make(a);
        let mut second = make(b);
        assert!(!first.compatible(&second));
        second.signature.as_mut().unwrap().iso = Some(100.0);
        assert!(first.compatible(&second));
        second.signature.as_mut().unwrap().serial = Some("different-camera".into());
        assert!(!first.compatible(&second));
    }
}
