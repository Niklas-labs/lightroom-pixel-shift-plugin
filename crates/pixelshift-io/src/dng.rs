//! Scene-linear camera RGB DNG, with bounded strip writes and no baked development.
use super::{Color, OutputWriter, render_srgb, write_aligned_tag, write_exif};
use anyhow::{Result, ensure};
use pixelshift_core::Image;
use std::{
    io::{BufWriter, Write},
    path::Path,
};
use tiff::{
    encoder::{Rational, SRational, TiffEncoder, colortype},
    tags::Tag,
};

fn rational(value: f32) -> Result<Rational> {
    ensure!(
        value.is_finite() && (0.0..4294.0).contains(&value),
        "invalid DNG rational"
    );
    Ok(Rational {
        n: (value * 1_000_000.0).round() as u32,
        d: 1_000_000,
    })
}
fn signed(value: f32) -> Result<SRational> {
    ensure!(
        value.is_finite() && value.abs() < 2147.0,
        "invalid DNG color matrix"
    );
    Ok(SRational {
        n: (value * 1_000_000.0).round() as i32,
        d: 1_000_000,
    })
}

/// Publish a DNG atomically. Strip payloads reference the existing image buffer;
/// only a small reduced preview is developed to sRGB. Float samples retain black
/// noise and highlight headroom, with BlackLevel=0 and WhiteLevel=1.
pub fn write_dng(path: &Path, image: &Image, color: &Color) -> Result<()> {
    ensure!(
        path.extension()
            .is_some_and(|v| v.eq_ignore_ascii_case("dng")),
        "output must be DNG"
    );
    ensure!(!path.exists(), "output already exists");
    ensure!(
        image.width > 0
            && image.height > 0
            && image
                .width
                .checked_mul(image.height)
                .and_then(|n| n.checked_mul(3))
                == Some(image.rgb.len()),
        "invalid DNG image buffer"
    );
    ensure!(
        image.rgb.iter().all(|v| v.is_finite()),
        "non-finite DNG sample"
    );
    ensure!((1..=8).contains(&color.orientation), "invalid orientation");
    ensure!(
        color
            .white_balance
            .iter()
            .all(|v| v.is_finite() && *v > 0.0),
        "invalid white balance"
    );
    super::inverse(color.xyz_to_camera)?;
    // Classic TIFF offsets are sufficient for a7CR 244 MP RGB32 (~2.93 GB).
    // Reject larger output before creating a file instead of overflowing offsets.
    ensure!(
        (image.rgb.len() as u64) * 4 + 16 * 1024 * 1024 < u32::MAX as u64,
        "DNG exceeds classic TIFF size limit"
    );
    let matrix = color
        .xyz_to_camera
        .map(signed)
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    let neutral = color
        .white_balance
        .map(|v| rational(color.white_balance[1] / v))
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        neutral.iter().all(|v| v.n > 0),
        "DNG white balance exceeds rational precision"
    );
    let width = u32::try_from(image.width)?;
    let height = u32::try_from(image.height)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    {
        let mut writer = BufWriter::new(temporary.as_file_mut());
        {
            let mut encoder = TiffEncoder::new(&mut writer)?;
            let exif = write_exif(&mut encoder, &color.photo_metadata, u16::MAX)?;
            // A bounded reduced preview is a separate image; never the raw payload.
            let factor = image.width.max(image.height).div_ceil(512);
            let pw = image.width.div_ceil(factor);
            let ph = image.height.div_ceil(factor);
            let mut rgb = Vec::with_capacity(pw * ph * 3);
            for y in 0..ph {
                for x in 0..pw {
                    let offset = ((y * factor) * image.width + x * factor) * 3;
                    rgb.extend_from_slice(&image.rgb[offset..offset + 3]);
                }
            }
            let preview = render_srgb(
                &Image {
                    width: pw,
                    height: ph,
                    rgb,
                },
                color,
            )?
            .into_iter()
            .map(|v| (v >> 8) as u8)
            .collect::<Vec<_>>();
            let mut thumb = encoder.extra_directory()?;
            let preview_offset = thumb.write_data(&preview[..])?;
            for (tag, value) in [
                (Tag::NewSubfileType, 1u32),
                (Tag::ImageWidth, pw as u32),
                (Tag::ImageLength, ph as u32),
                (Tag::RowsPerStrip, ph as u32),
                (Tag::StripByteCounts, preview.len() as u32),
                (Tag::StripOffsets, preview_offset as u32),
            ] {
                write_aligned_tag(&mut thumb, tag, value)?;
            }
            for (tag, value) in [
                (Tag::Compression, 1u16),
                (Tag::PhotometricInterpretation, 2),
                (Tag::SamplesPerPixel, 3),
                (Tag::PlanarConfiguration, 1),
            ] {
                write_aligned_tag(&mut thumb, tag, value)?;
            }
            write_aligned_tag(&mut thumb, Tag::BitsPerSample, &[8u16, 8, 8][..])?;
            write_aligned_tag(&mut thumb, Tag::SampleFormat, &[1u16, 1, 1][..])?;
            write_aligned_tag(&mut thumb, Tag::Unknown(50970), 2u32)?;
            write_aligned_tag(
                &mut thumb,
                Tag::IccProfile,
                &include_bytes!("../assets/srgb.icc")[..],
            )?;
            let preview_ifd = thumb.finish_with_offsets()?.offset;
            let mut raw = encoder.new_image::<colortype::RGB32Float>(width, height)?;
            raw.rows_per_strip(32)?;
            let tags = raw.encoder();
            write_aligned_tag(tags, Tag::SubIfd, preview_ifd)?;
            // RGB32Float defines the storage layout only. 34892 declares the
            // camera-linear raw interpretation; no RGB/ICC transfer is implied.
            write_aligned_tag(tags, Tag::PhotometricInterpretation, 34892u16)?;
            write_aligned_tag(tags, Tag::NewSubfileType, 0u32)?;
            write_aligned_tag(tags, Tag::Orientation, color.orientation)?;
            write_aligned_tag(tags, Tag::ExifDirectory, exif)?;
            write_aligned_tag(tags, Tag::Software, "sony-pixelshift 0.1.0")?;
            write_aligned_tag(
                tags,
                Tag::ImageDescription,
                "Sony Pixel Shift experimental merge; scene-linear camera RGB; source RAWs retained separately",
            )?;
            write_aligned_tag(tags, Tag::Unknown(50706), &[1u8, 4, 0, 0][..])?; // DNGVersion
            write_aligned_tag(tags, Tag::Unknown(50707), &[1u8, 4, 0, 0][..])?; // float needs 1.4
            let make = color.photo_metadata.make.as_deref().unwrap_or("SONY");
            let model = color
                .photo_metadata
                .model
                .as_deref()
                .unwrap_or("Unknown camera");
            write_aligned_tag(tags, Tag::Make, make)?;
            write_aligned_tag(tags, Tag::Model, model)?;
            let unique = format!(
                "{} {}",
                if make.eq_ignore_ascii_case("sony") {
                    "Sony"
                } else {
                    make
                },
                model
            );
            write_aligned_tag(tags, Tag::Unknown(50708), unique.as_str())?;
            write_aligned_tag(tags, Tag::Unknown(50721), &matrix[..])?; // XYZ -> camera, NOT inverse
            write_aligned_tag(tags, Tag::Unknown(50778), 21u16)?; // D65
            write_aligned_tag(tags, Tag::Unknown(50728), &neutral[..])?; // reciprocal WB
            write_aligned_tag(tags, Tag::Unknown(50713), &[1u16, 1][..])?;
            // TIFF Rational is not Copy; write the three channel black levels explicitly.
            write_aligned_tag(
                tags,
                Tag::Unknown(50714),
                &[rational(0.0)?, rational(0.0)?, rational(0.0)?][..],
            )?;
            write_aligned_tag(tags, Tag::Unknown(50717), &[1u32, 1, 1][..])?;
            write_aligned_tag(
                tags,
                Tag::Unknown(50718),
                &[rational(1.0)?, rational(1.0)?][..],
            )?;
            write_aligned_tag(tags, Tag::Unknown(50719), &[0u32, 0][..])?;
            write_aligned_tag(tags, Tag::Unknown(50720), &[width, height][..])?;
            write_aligned_tag(tags, Tag::Unknown(50829), &[0u32, 0, height, width][..])?;
            write_aligned_tag(tags, Tag::Unknown(50730), signed(0.0)?)?; // no arbitrary exposure shift
            write_aligned_tag(tags, Tag::Unknown(50731), rational(1.0)?)?;
            write_aligned_tag(tags, Tag::Unknown(50732), rational(1.0)?)?;
            for strip in image.rgb.chunks(image.width * 3 * 32) {
                raw.write_strip(strip)?;
            }
            raw.finish()?;
        }
        writer.flush()?;
    }
    temporary.as_file().sync_all()?;
    temporary.persist_noclobber(path).map_err(|e| e.error)?;
    Ok(())
}

pub struct DngWriter;
impl OutputWriter for DngWriter {
    fn write(&self, path: &Path, image: &Image, color: &Color) -> Result<()> {
        write_dng(path, image, color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rawler::{
        RawImageData,
        exif::Exif,
        formats::tiff::{GenericTiffReader, reader::TiffReader},
    };
    use std::fs::{self, File};
    fn color() -> Color {
        Color {
            photo_metadata: super::super::PhotoMetadata {
                make: Some("SONY".into()),
                model: Some("ILCE-7CR".into()),
                lens_make: Some("Sony".into()),
                lens_model: Some("FE 20-70mm F4 G".into()),
                ..Default::default()
            },
            white_balance: [2.0, 1.0, 1.5],
            xyz_to_camera: [
                0.82, -0.2976, -0.0719, -0.4296, 1.2053, 0.2532, -0.0429, 0.1282, 0.5774,
            ],
            orientation: 6,
        }
    }
    #[test]
    fn linear_dng_preserves_samples_calibration_and_preview() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("linear.dng");
        let values = [-0.0125, 0.0, 0.00001, 0.18, 0.73, 1.0, 1.5, 3.25];
        let image = Image {
            width: 96,
            height: 70,
            rgb: (0..96 * 70 * 3).map(|i| values[i % values.len()]).collect(),
        };
        let color = color();
        write_dng(&path, &image, &color).unwrap();
        let raw = rawler::decode_file(&path).unwrap();
        assert_eq!((raw.width, raw.height, raw.cpp), (96, 70, 3));
        assert_eq!(raw.orientation.to_u16(), 6);
        assert_eq!(raw.whitelevel.0, vec![1, 1, 1]);
        assert!(matches!(
            raw.photometric,
            rawler::rawimage::RawPhotometricInterpretation::LinearRaw
        ));
        match raw.data {
            RawImageData::Float(samples) => assert_eq!(samples, image.rgb),
            _ => panic!("expected float RAW"),
        }
        assert!((raw.wb_coeffs[0] / raw.wb_coeffs[1] - 2.0).abs() < 0.0001);
        assert!((raw.wb_coeffs[2] / raw.wb_coeffs[1] - 1.5).abs() < 0.0001);
        let parsed =
            GenericTiffReader::new(&mut File::open(&path).unwrap(), 0, 0, Some(4), &[]).unwrap();
        let root = parsed.root_ifd();
        assert_eq!(
            root.get_entry(50706u16).unwrap().value.get_data(),
            &[1, 4, 0, 0]
        );
        for (index, value) in color.xyz_to_camera.iter().enumerate() {
            assert!((root.get_entry(50721u16).unwrap().force_f32(index) - value).abs() < 0.00001);
        }
        assert!(root.get_entry(0x8773u16).is_none()); // No ICC on camera RGB.
        assert!(root.get_entry(0x828du16).is_none()); // Not a fabricated CFA mosaic.
        assert!(root.find_first_ifd_with_tag(0x927cu16).is_none());
        let exif = Exif::new(root).unwrap();
        assert_eq!(exif.lens_model, color.photo_metadata.lens_model);
        assert_eq!(exif.color_space, Some(u16::MAX));
        // The generic TIFF decoder rejects LinearRaw. Point a test-only TIFF
        // header at the separate RGB preview; all offsets remain unchanged.
        let mut preview_bytes = fs::read(&path).unwrap();
        let offset = match root.endian {
            rawler::bits::Endian::Little => {
                root.get_entry(330u16).unwrap().force_u32(0).to_le_bytes()
            }
            rawler::bits::Endian::Big => root.get_entry(330u16).unwrap().force_u32(0).to_be_bytes(),
        };
        preview_bytes[4..8].copy_from_slice(&offset);
        let mut preview = tiff::decoder::Decoder::new(std::io::Cursor::new(preview_bytes)).unwrap();
        assert_eq!(preview.colortype().unwrap(), tiff::ColorType::RGB(8));
        assert!(preview.dimensions().unwrap().0 <= 512);
        assert!(matches!(
            preview.read_image().unwrap(),
            tiff::decoder::DecodingResult::U8(_)
        ));
        let original = fs::read(&path).unwrap();
        assert!(write_dng(&path, &image, &color).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
    }
    #[test]
    fn invalid_dng_data_never_publishes_a_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("invalid.dng");
        let mut image = Image {
            width: 8,
            height: 8,
            rgb: vec![0.18; 8 * 8 * 3],
        };
        let mut calibration = color();
        image.rgb[4] = f32::NAN;
        assert!(write_dng(&path, &image, &calibration).is_err());
        image.rgb[4] = 0.18;
        calibration.white_balance[0] = 0.0;
        assert!(write_dng(&path, &image, &calibration).is_err());
        calibration = color();
        calibration.xyz_to_camera = [0.0; 9];
        assert!(write_dng(&path, &image, &calibration).is_err());
        calibration = color();
        calibration.orientation = 0;
        assert!(write_dng(&path, &image, &calibration).is_err());
        assert!(!path.exists());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    }
}
