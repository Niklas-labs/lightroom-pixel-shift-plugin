//! Decoder and output boundaries. No RAW development is performed before merging.
use anyhow::{Context, Result, bail, ensure};
use pixelshift_core::{Frame, Image};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    process::Command,
};
use tiff::encoder::{TiffEncoder, colortype};

#[derive(Debug, PartialEq, Deserialize)]
struct Capture {
    #[serde(rename = "SequenceNumber")]
    sequence: Option<usize>,
    #[serde(rename = "ISO")]
    iso: Option<f64>,
    #[serde(rename = "ExposureTime")]
    exposure: Option<f64>,
    #[serde(rename = "FNumber")]
    aperture: Option<f64>,
    #[serde(rename = "SerialNumber", default, deserialize_with = "serial_number")]
    serial: Option<String>,
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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Color {
    pub white_balance: [f32; 3],
    /// XYZ D65 to camera, row major.
    pub xyz_to_camera: [f32; 9],
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
}
impl Decoded {
    pub fn from_sensor(frame: Frame, color: Color) -> Result<Self> {
        frame.validate()?;
        Ok(Self {
            frame,
            color,
            signature: None,
        })
    }
    pub fn compatible(&self, other: &Self) -> bool {
        match (&self.signature, &other.signature) {
            (Some(a), Some(b)) => {
                a.iso == b.iso
                    && a.exposure == b.exposure
                    && a.aperture == b.aperture
                    && a.serial == b.serial
            }
            (None, None) => self.color == other.color,
            _ => false,
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
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
        });
    }
    ensure!(
        path.extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("arw")),
        "only ARW input is supported (fixtures require --fixtures)"
    );
    let output = Command::new("exiftool")
        .args([
            "-json",
            "-n",
            "-SequenceNumber",
            "-ISO",
            "-ExposureTime",
            "-FNumber",
            "-SerialNumber",
            "--",
        ])
        .arg(path)
        .output()
        .context("ExifTool is required to validate Sony shot order and exposure")?;
    ensure!(
        output.status.success(),
        "ExifTool failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut captures: Vec<Capture> = serde_json::from_slice(&output.stdout)?;
    ensure!(captures.len() == 1, "expected one metadata record");
    let capture = captures.remove(0);
    ensure!(
        capture.iso.is_some() && capture.exposure.is_some() && capture.aperture.is_some(),
        "missing exposure metadata"
    );
    let sequence = capture
        .sequence
        .context("missing Sony SequenceNumber; input must be a complete capture sequence")?;
    let raw = rawler::decode_file(path).with_context(|| format!("decoding {}", path.display()))?;
    ensure!(
        raw.cpp == 1 && raw.model == "ILCE-7CR",
        "initial RAW adapter supports ILCE-7CR Bayer ARW only"
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
    // Rawler's camera catalog matrices are XYZ -> camera. Select D65 explicitly.
    let matrix = raw
        .camera
        .color_matrix
        .get(&rawler::imgop::xyz::Illuminant::D65)
        .context("missing D65 camera matrix")?;
    ensure!(matrix.len() == 9, "expected 3x3 camera matrix");
    let color = Color {
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

/// Same-directory temporary file + no-clobber publication. Never modifies a source file.
pub fn write_tiff(path: &Path, image: &Image, color: &Color) -> Result<()> {
    ensure!(
        path.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("tif") || e.eq_ignore_ascii_case("tiff")),
        "output must be TIFF; DNG is planned next"
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
            let mut output = encoder
                .new_image::<colortype::RGB16>(image.width.try_into()?, image.height.try_into()?)?;
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
/// A future linear DNG writer receives the scene-linear image and camera calibration.
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
