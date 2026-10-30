//! Pull-based, bounded output encoding shared by TIFF and linear DNG.
use anyhow::{Result, ensure};
use pixelshift_core::{ImageSource, Region, RenderedRegion};
use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum OutputCompression {
    #[default]
    Lossless,
    None,
}
#[derive(Clone, Copy, Debug)]
pub struct WriteOptions {
    pub compression: OutputCompression,
    pub strip_rows: usize,
}
impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            compression: OutputCompression::Lossless,
            strip_rows: 32,
        }
    }
}
impl WriteOptions {
    pub fn validate(self) -> Result<()> {
        ensure!(
            (1..=256).contains(&self.strip_rows),
            "output strip rows must be 1..256"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct OutputStats {
    pub regions: usize,
    pub fallback_pixels: usize,
    pub peak_region_pixels: usize,
    pub reconstruction_seconds: f64,
    pub encode_write_seconds: f64,
    pub preview_seconds: f64,
    pub total_seconds: f64,
    pub output_bytes: u64,
}
impl OutputStats {
    pub(crate) fn pull(
        &mut self,
        source: &dyn ImageSource,
        region: Region,
    ) -> Result<RenderedRegion> {
        let now = Instant::now();
        let tile = source.read_region(region)?;
        self.reconstruction_seconds += now.elapsed().as_secs_f64();
        ensure!(
            (tile.image.width, tile.image.height) == (region.width, region.height)
                && region
                    .width
                    .checked_mul(region.height)
                    .and_then(|v| v.checked_mul(3))
                    == Some(tile.image.rgb.len()),
            "source returned invalid output region"
        );
        ensure!(
            tile.image.rgb.iter().all(|v| v.is_finite()),
            "non-finite output sample"
        );
        self.regions += 1;
        self.fallback_pixels += tile.fallback_pixels;
        self.peak_region_pixels = self.peak_region_pixels.max(region.width * region.height);
        Ok(tile)
    }
}

pub(crate) fn write_payload<W: std::io::Write + std::io::Seek>(
    directory: &mut tiff::encoder::DirectoryEncoder<'_, W, tiff::encoder::TiffKindStandard>,
    bytes: &[u8],
    compression: OutputCompression,
) -> Result<(u32, u32)> {
    use std::io::Write;
    if directory.write_data(&[] as &[u8])? % 2 != 0 {
        directory.write_data(&[0u8][..])?;
    }
    let (offset, size) = match compression {
        OutputCompression::None => (directory.write_data(bytes)?, bytes.len()),
        OutputCompression::Lossless => {
            let mut encoder =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            encoder.write_all(bytes)?;
            let encoded = encoder.finish()?;
            (directory.write_data(&encoded[..])?, encoded.len())
        }
    };
    Ok((offset.try_into()?, size.try_into()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Color, Crop, PhotoMetadata, write_dng_from_source, write_tiff_from_source};
    use pixelshift_core::Image;
    use std::{
        fs::File,
        io::{Read, Seek, SeekFrom},
        sync::atomic::{AtomicUsize, Ordering},
    };
    struct Procedural {
        calls: AtomicUsize,
        fail_after: usize,
    }
    impl ImageSource for Procedural {
        fn dimensions(&self) -> (usize, usize) {
            (96, 70)
        }
        fn read_region(&self, r: Region) -> Result<RenderedRegion> {
            ensure!(
                r.height <= 17,
                "writer requested an unbounded source region"
            );
            if self.calls.fetch_add(1, Ordering::SeqCst) >= self.fail_after {
                anyhow::bail!("injected late reconstruction failure");
            }
            let image = Image {
                width: r.width,
                height: r.height,
                rgb: (0..r.width * r.height * 3)
                    .map(|i| {
                        let c = i % 3;
                        let x = r.x + i / 3 % r.width;
                        let y = r.y + i / 3 / r.width;
                        [-0.01, 0.18, 1.4][c] + (x + y) as f32 / 10000.0
                    })
                    .collect(),
            };
            Ok(RenderedRegion {
                image,
                fallback_pixels: r.width * r.height,
            })
        }
    }
    fn calibration() -> Color {
        Color {
            default_crop: Some(Crop {
                x: 2,
                y: 3,
                width: 90,
                height: 64,
            }),
            photo_metadata: PhotoMetadata {
                make: Some("SONY".into()),
                model: Some("ILCE-7CR".into()),
                ..Default::default()
            },
            white_balance: [2.0, 1.0, 1.5],
            xyz_to_camera: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            xyz_to_camera_a: None,
            orientation: 1,
        }
    }
    fn samples(path: &std::path::Path) -> Vec<f32> {
        use rawler::formats::tiff::{GenericTiffReader, reader::TiffReader};
        let parsed =
            GenericTiffReader::new(&mut File::open(path).unwrap(), 0, 0, Some(4), &[]).unwrap();
        let root = parsed.root_ifd();
        let offsets = &root.get_entry(273u16).unwrap().value;
        let sizes = &root.get_entry(279u16).unwrap().value;
        let compressed = root.get_entry(259u16).unwrap().force_u16(0) == 8;
        let mut samples = Vec::new();
        let mut file = File::open(path).unwrap();
        for index in 0..offsets.count() {
            let mut bytes = vec![0; sizes.force_u32(index) as usize];
            file.seek(SeekFrom::Start(offsets.force_u32(index) as u64))
                .unwrap();
            file.read_exact(&mut bytes).unwrap();
            if compressed {
                let mut decoded = Vec::new();
                flate2::read::ZlibDecoder::new(&bytes[..])
                    .read_to_end(&mut decoded)
                    .unwrap();
                bytes = decoded;
            }
            for value in bytes.as_chunks::<4>().0 {
                samples.push(match root.endian {
                    rawler::bits::Endian::Little => f32::from_le_bytes(*value),
                    rawler::bits::Endian::Big => f32::from_be_bytes(*value),
                });
            }
        }
        samples
    }
    #[test]
    fn bounded_compressed_and_plain_dng_payloads_are_identical() {
        let dir = tempfile::tempdir().unwrap();
        let color = calibration();
        let mut results = Vec::new();
        for compression in [OutputCompression::None, OutputCompression::Lossless] {
            let path = dir.path().join(format!("{compression:?}.dng"));
            let source = Procedural {
                calls: AtomicUsize::new(0),
                fail_after: usize::MAX,
            };
            let mut events = Vec::new();
            let stats = write_dng_from_source(
                &path,
                &source,
                &color,
                WriteOptions {
                    compression,
                    strip_rows: 17,
                },
                &mut |done, total| {
                    events.push((done, total));
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(source.calls.load(Ordering::SeqCst), 5);
            assert_eq!(stats.peak_region_pixels, 96 * 17);
            assert_eq!(stats.fallback_pixels, 96 * 70);
            assert_eq!(events, vec![(1, 5), (2, 5), (3, 5), (4, 5), (5, 5)]);
            results.push(samples(&path));
        }
        assert_eq!(
            results[0].iter().map(|n| n.to_bits()).collect::<Vec<_>>(),
            results[1].iter().map(|n| n.to_bits()).collect::<Vec<_>>()
        );
        assert!(
            dir.path().join("Lossless.dng").metadata().unwrap().len()
                < dir.path().join("None.dng").metadata().unwrap().len()
        );
    }
    #[test]
    fn late_source_and_progress_errors_remove_temporary_output() {
        let dir = tempfile::tempdir().unwrap();
        let color = calibration();
        for dng in [true, false] {
            for callback_failure in [true, false] {
                let path = dir
                    .path()
                    .join(if dng { "failed.dng" } else { "failed.tif" });
                let source = Procedural {
                    calls: AtomicUsize::new(0),
                    fail_after: if callback_failure { usize::MAX } else { 2 },
                };
                let mut callback = |done, _| {
                    if callback_failure && done == 3 {
                        anyhow::bail!("cancelled/progress failure");
                    }
                    Ok(())
                };
                let options = WriteOptions {
                    strip_rows: 17,
                    ..WriteOptions::default()
                };
                let result = if dng {
                    write_dng_from_source(&path, &source, &color, options, &mut callback)
                } else {
                    write_tiff_from_source(&path, &source, &color, options, &mut callback)
                };
                assert!(result.is_err());
                assert!(!path.exists());
                assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
            }
        }
    }
}
