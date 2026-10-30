//! Scene-linear camera RGB DNG, with bounded strip writes and no baked development.
use super::{Color, OutputWriter, render_srgb, write_aligned_tag, write_exif};
use super::{OutputCompression, OutputStats, WriteOptions};
use anyhow::{Result, ensure};
use pixelshift_core::{Image, ImageSource, Region};
use std::time::Instant;
use std::{
    io::{BufWriter, Write},
    path::Path,
};
use tiff::{
    encoder::{Rational, SRational, TiffEncoder},
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

/// Convenience wrapper; production can pull output regions without a full image.
pub fn write_dng(path: &Path, image: &Image, color: &Color) -> Result<()> {
    write_dng_from_source(path, image, color, WriteOptions::default(), &mut |_, _| {
        Ok(())
    })
    .map(|_| ())
}
/// Reconstruct/write one strip at a time, retaining only a bounded small preview.
pub fn write_dng_from_source(
    path: &Path,
    source: &dyn ImageSource,
    color: &Color,
    options: WriteOptions,
    progress: &mut dyn FnMut(usize, usize) -> Result<()>,
) -> Result<OutputStats> {
    options.validate()?;
    let started = Instant::now();
    let mut stats = OutputStats::default();
    let (image_width, image_height) = source.dimensions();
    ensure!(
        path.extension()
            .is_some_and(|v| v.eq_ignore_ascii_case("dng")),
        "output must be DNG"
    );
    ensure!(!path.exists(), "output already exists");
    ensure!(
        image_width > 0 && image_height > 0,
        "invalid DNG dimensions"
    );
    let samples = image_width
        .checked_mul(image_height)
        .and_then(|v| v.checked_mul(3))
        .ok_or_else(|| anyhow::anyhow!("DNG dimensions overflow"))?;
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
        (samples as u128) * 4 + (samples as u128) / 25 + 16 * 1024 * 1024 < u32::MAX as u128,
        "DNG exceeds classic TIFF size limit"
    );
    let matrix = color
        .xyz_to_camera
        .map(signed)
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    let matrix_a = color
        .xyz_to_camera_a
        .map(|matrix| -> Result<Vec<SRational>> {
            super::inverse(matrix)?;
            matrix.map(signed).into_iter().collect::<Result<Vec<_>>>()
        })
        .transpose()?;
    let neutral = color
        .white_balance
        .map(|v| rational(color.white_balance[1] / v))
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        neutral.iter().all(|v| v.n > 0),
        "DNG white balance exceeds rational precision"
    );
    let crop = color.default_crop.unwrap_or(super::Crop {
        x: 0,
        y: 0,
        width: image_width,
        height: image_height,
    });
    crop.validate(image_width, image_height)?;
    let width = u32::try_from(image_width)?;
    let height = u32::try_from(image_height)?;
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
            let factor = crop.width.max(crop.height).div_ceil(512);
            let pw = crop.width.div_ceil(factor);
            let ph = crop.height.div_ceil(factor);
            let mut rgb = vec![0.0; pw * ph * 3];
            let mut offsets = Vec::new();
            let mut byte_counts = Vec::new();
            let total = image_height.div_ceil(options.strip_rows);
            // A scratch directory exposes bounded payload writes; it is not an
            // image IFD and is never linked into the file's directory graph.
            let mut payload = encoder.extra_directory()?;
            for (index, y) in (0..image_height).step_by(options.strip_rows).enumerate() {
                let rows = options.strip_rows.min(image_height - y);
                let tile = stats.pull(
                    source,
                    Region {
                        x: 0,
                        y,
                        width: image_width,
                        height: rows,
                    },
                )?;
                // Accumulate the original nearest-sample preview from pixels
                // already reconstructed, without additional source evaluations.
                for py in 0..ph {
                    let sy = crop.y + py * factor;
                    if sy >= y && sy < y + rows {
                        for px in 0..pw {
                            let from = ((sy - y) * image_width + crop.x + px * factor) * 3;
                            let to = (py * pw + px) * 3;
                            rgb[to..to + 3].copy_from_slice(&tile.image.rgb[from..from + 3]);
                        }
                    }
                }
                let encode_start = Instant::now();
                let (offset, size) = super::output::write_payload(
                    &mut payload,
                    bytemuck::cast_slice(&tile.image.rgb),
                    options.compression,
                )?;
                offsets.push(offset);
                byte_counts.push(size);
                stats.encode_write_seconds += encode_start.elapsed().as_secs_f64();
                progress(index + 1, total)?;
            }
            payload.finish()?;
            let preview_start = Instant::now();
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
            stats.preview_seconds += preview_start.elapsed().as_secs_f64();
            let mut directory = encoder.image_directory()?;
            let tags = &mut directory;
            for (tag, value) in [
                (Tag::ImageWidth, width),
                (Tag::ImageLength, height),
                (Tag::RowsPerStrip, options.strip_rows as u32),
            ] {
                write_aligned_tag(tags, tag, value)?;
            }
            write_aligned_tag(tags, Tag::SamplesPerPixel, 3u16)?;
            write_aligned_tag(tags, Tag::PlanarConfiguration, 1u16)?;
            write_aligned_tag(tags, Tag::BitsPerSample, &[32u16, 32, 32][..])?;
            write_aligned_tag(tags, Tag::SampleFormat, &[3u16, 3, 3][..])?;
            write_aligned_tag(
                tags,
                Tag::Compression,
                if options.compression == OutputCompression::Lossless {
                    8u16
                } else {
                    1u16
                },
            )?;
            write_aligned_tag(tags, Tag::StripOffsets, &offsets[..])?;
            write_aligned_tag(tags, Tag::StripByteCounts, &byte_counts[..])?;
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
            // Keep original, unnormalized XYZ -> camera matrices. Slot 1 is
            // tungsten A and slot 2 is daylight D65 when both are available.
            if let Some(matrix_a) = &matrix_a {
                write_aligned_tag(tags, Tag::Unknown(50721), &matrix_a[..])?;
                write_aligned_tag(tags, Tag::Unknown(50778), 17u16)?;
                write_aligned_tag(tags, Tag::Unknown(50722), &matrix[..])?;
                write_aligned_tag(tags, Tag::Unknown(50779), 21u16)?;
            } else {
                write_aligned_tag(tags, Tag::Unknown(50721), &matrix[..])?;
                write_aligned_tag(tags, Tag::Unknown(50778), 21u16)?;
            }
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
            write_aligned_tag(
                tags,
                Tag::Unknown(50719),
                &[crop.x as u32, crop.y as u32][..],
            )?;
            write_aligned_tag(
                tags,
                Tag::Unknown(50720),
                &[crop.width as u32, crop.height as u32][..],
            )?;
            write_aligned_tag(tags, Tag::Unknown(50829), &[0u32, 0, height, width][..])?;
            write_aligned_tag(tags, Tag::Unknown(50730), signed(0.0)?)?; // no arbitrary exposure shift
            write_aligned_tag(tags, Tag::Unknown(50731), rational(1.0)?)?;
            write_aligned_tag(tags, Tag::Unknown(50732), rational(1.0)?)?;
            directory.finish()?;
        }
        writer.flush()?;
    }
    temporary.as_file().sync_all()?;
    stats.output_bytes = temporary.as_file().metadata()?.len();
    temporary.persist_noclobber(path).map_err(|e| e.error)?;
    stats.total_seconds = started.elapsed().as_secs_f64();
    Ok(stats)
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
            xyz_to_camera_a: None,
            default_crop: Some(super::super::Crop {
                x: 2,
                y: 3,
                width: 90,
                height: 64,
            }),
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
        write_dng_from_source(
            &path,
            &image,
            &color,
            WriteOptions {
                compression: OutputCompression::None,
                ..WriteOptions::default()
            },
            &mut |_, _| Ok(()),
        )
        .unwrap();
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
        assert_eq!(root.get_entry(50719u16).unwrap().force_usize(0), 2);
        assert_eq!(root.get_entry(50719u16).unwrap().force_usize(1), 3);
        assert_eq!(root.get_entry(50720u16).unwrap().force_usize(0), 90);
        assert_eq!(root.get_entry(50720u16).unwrap().force_usize(1), 64);
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
    fn dual_illuminant_tags_preserve_white_balance_samples_and_exposure() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("dual.dng");
        let mut color = color();
        let matrix_a = [
            0.9185, -0.4857, 0.0505, -0.3651, 1.1061, 0.2982, -0.0161, 0.0698, 0.6769,
        ];
        color.xyz_to_camera_a = Some(matrix_a);
        let image = Image {
            width: 96,
            height: 70,
            rgb: vec![0.18; 96 * 70 * 3],
        };
        write_dng_from_source(
            &path,
            &image,
            &color,
            WriteOptions {
                compression: OutputCompression::None,
                ..WriteOptions::default()
            },
            &mut |_, _| Ok(()),
        )
        .unwrap();
        let parsed =
            GenericTiffReader::new(&mut File::open(&path).unwrap(), 0, 0, Some(4), &[]).unwrap();
        let root = parsed.root_ifd();
        assert_eq!(root.get_entry(50778u16).unwrap().force_u16(0), 17);
        assert_eq!(root.get_entry(50779u16).unwrap().force_u16(0), 21);
        for (tag, matrix) in [(50721u16, matrix_a), (50722u16, color.xyz_to_camera)] {
            for (index, value) in matrix.iter().enumerate() {
                assert!((root.get_entry(tag).unwrap().force_f32(index) - value).abs() < 0.00001);
            }
        }
        for (index, value) in [0.5, 1.0, 2.0 / 3.0].iter().enumerate() {
            assert!((root.get_entry(50728u16).unwrap().force_f32(index) - value).abs() < 0.00001);
        }
        assert_eq!(root.get_entry(50730u16).unwrap().force_f32(0), 0.0);
        let raw = rawler::decode_file(&path).unwrap();
        assert!(
            raw.camera
                .color_matrix
                .contains_key(&rawler::imgop::xyz::Illuminant::A)
        );
        assert!(
            raw.camera
                .color_matrix
                .contains_key(&rawler::imgop::xyz::Illuminant::D65)
        );
        match raw.data {
            RawImageData::Float(samples) => assert_eq!(samples, image.rgb),
            _ => panic!("expected float RAW"),
        }
        let bad = temp.path().join("invalid-dual.dng");
        color.xyz_to_camera_a = Some([0.0; 9]);
        assert!(write_dng(&bad, &image, &color).is_err());
        assert!(!bad.exists());
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
