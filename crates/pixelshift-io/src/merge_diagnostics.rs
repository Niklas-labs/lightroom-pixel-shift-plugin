//! Optional streamed reference-fallback diagnostics. Never modifies camera RGB.
use crate::{Color, Crop, OutputCompression};
use anyhow::{Context, Result, ensure};
use pixelshift_core::{Image, ImageSource, Region, RenderedRegion};
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};
use tiff::{encoder::TiffEncoder, tags::Tag};

const ROWS: usize = 32;
const LEGEND: &str = "Reference fallback: white mask pixels / red overlay show where frame disagreement exceeded the threshold. This is not proof of motion: noise, detail and lighting changes can also trigger fallback. Black mask pixels use multi-frame reconstruction. In 16-shot mode fallback uses interpolated first-frame data, not full 2x detail. Overlay cells show any fallback within the cell; use the full-resolution mask for exact boundaries. Overlay brightness is an approximate As Shot preview, not Lightroom Develop rendering.";

pub struct MergeDiagnostics<'a> {
    source: &'a dyn ImageSource,
    // Close handles before TempDir cleanup, including on Windows failures.
    state: Mutex<State>,
    staging: tempfile::TempDir,
    destination: PathBuf,
}
struct State {
    encoder: Option<TiffEncoder<File>>,
    offsets: Vec<u32>,
    counts: Vec<u32>,
    pending: Vec<u8>,
    next_y: usize,
    fallback: usize,
    peak_mask_bytes: usize,
    crop: Crop,
    color: Color,
    factor: usize,
    preview: Image,
    marked: Vec<bool>,
    completed: bool,
}
impl<'a> MergeDiagnostics<'a> {
    pub fn new(source: &'a dyn ImageSource, output: &Path, color: &Color) -> Result<Self> {
        let (width, height) = source.dimensions();
        let crop = color.default_crop.unwrap_or(Crop {
            x: 0,
            y: 0,
            width,
            height,
        });
        crop.validate(width, height)?;
        ensure!(
            (1..=8).contains(&color.orientation),
            "invalid diagnostics orientation"
        );
        let mut destination = output.as_os_str().to_os_string();
        destination.push(".diagnostics");
        let destination = PathBuf::from(destination);
        ensure!(
            !destination.exists(),
            "diagnostics directory already exists"
        );
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let staging = tempfile::Builder::new()
            .prefix(".pixelshift-diagnostics-")
            .tempdir_in(parent)?;
        let encoder =
            TiffEncoder::new(File::create(staging.path().join("reference-fallback.tif"))?)?;
        let factor = crop.width.max(crop.height).div_ceil(512);
        let pw = crop.width.div_ceil(factor);
        let ph = crop.height.div_ceil(factor);
        Ok(Self {
            source,
            staging,
            destination,
            state: Mutex::new(State {
                encoder: Some(encoder),
                offsets: vec![],
                counts: vec![],
                pending: Vec::with_capacity(crop.width * ROWS),
                next_y: 0,
                fallback: 0,
                peak_mask_bytes: 0,
                crop,
                color: color.clone(),
                factor,
                preview: Image {
                    width: pw,
                    height: ph,
                    rgb: vec![0.; pw * ph * 3],
                },
                marked: vec![false; pw * ph],
                completed: false,
            }),
        })
    }
    /// Publish only after the primary image has been written successfully.
    pub fn publish(self, context: Value) -> Result<Value> {
        let state = self
            .state
            .into_inner()
            .map_err(|_| anyhow::anyhow!("diagnostics lock poisoned"))?;
        ensure!(state.completed, "incomplete diagnostics stream");
        let summary = json!({"version":1,"kind":"reference_fallback","legend":LEGEND,
            "crop":state.crop,"orientation":state.color.orientation,
            "fallback_pixels":state.fallback,"total_pixels":state.crop.width*state.crop.height,
            "fallback_fraction":state.fallback as f64/(state.crop.width*state.crop.height) as f64,
            "peak_mask_band_bytes":state.peak_mask_bytes,"context":context});
        std::fs::write(
            self.staging.path().join("summary.json"),
            serde_json::to_vec_pretty(&summary)?,
        )?;
        std::fs::write(
            self.staging.path().join("README.txt"),
            format!("{LEGEND}\n"),
        )?;
        // create_dir reserves the final name without replacing any existing directory.
        std::fs::create_dir(&self.destination)
            .context("DNG saved, but diagnostics directory could not be created")?;
        let files = [
            "reference-fallback.tif",
            "overlay.tif",
            "summary.json",
            "README.txt",
        ];
        let result = (|| -> Result<()> {
            for name in files {
                std::fs::rename(self.staging.path().join(name), self.destination.join(name))?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            for name in files {
                let _ = std::fs::remove_file(self.destination.join(name));
            }
            let _ = std::fs::remove_dir(&self.destination);
            return Err(error.context("DNG saved, but publishing diagnostics failed"));
        }
        Ok(
            json!({"directory":self.destination,"mask":self.destination.join("reference-fallback.tif"),
            "overlay":self.destination.join("overlay.tif"),"summary":self.destination.join("summary.json")}),
        )
    }
}
impl ImageSource for MergeDiagnostics<'_> {
    fn dimensions(&self) -> (usize, usize) {
        self.source.dimensions()
    }
    fn read_region(&self, region: Region) -> Result<RenderedRegion> {
        let mut tile = self.source.read_region_with_mask(region)?;
        let mask = tile
            .fallback_mask
            .take()
            .context("source did not return a fallback mask")?;
        ensure!(
            mask.len() == region.width * region.height && tile.image.rgb.len() == mask.len() * 3,
            "invalid diagnostic region"
        );
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("diagnostics lock poisoned"))?;
        ensure!(
            !state.completed
                && region.x == 0
                && region.width == self.dimensions().0
                && region.y == state.next_y,
            "diagnostics requires sequential full-width bands"
        );
        let crop = state.crop;
        state.peak_mask_bytes = state
            .peak_mask_bytes
            .max(mask.len() + state.pending.capacity());
        for y in region.y.max(crop.y)..(region.y + region.height).min(crop.y + crop.height) {
            let start = (y - region.y) * region.width + crop.x;
            let row = &mask[start..start + crop.width];
            ensure!(
                row.iter().all(|v| *v == 0 || *v == 255),
                "invalid fallback mask values"
            );
            state.fallback += row.iter().filter(|v| **v != 0).count();
            let py = (y - crop.y) / state.factor;
            for (x, value) in row.iter().enumerate() {
                let px = x / state.factor;
                let i = py * state.preview.width + px;
                state.marked[i] |= *value != 0;
                if (y - crop.y) % state.factor == 0 && x % state.factor == 0 {
                    state.preview.rgb[i * 3..i * 3 + 3]
                        .copy_from_slice(&tile.image.rgb[(start + x) * 3..(start + x) * 3 + 3]);
                }
            }
            state.pending.extend_from_slice(row);
            if state.pending.len() == crop.width * ROWS {
                state.flush_mask()?;
            }
        }
        state.next_y += region.height;
        if state.next_y == self.dimensions().1 {
            state.finish(self.staging.path())?;
        }
        Ok(tile)
    }
}
impl State {
    fn flush_mask(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let mut payload = self.encoder.as_mut().unwrap().extra_directory()?;
        let (offset, count) =
            crate::output::write_payload(&mut payload, &self.pending, OutputCompression::Lossless)?;
        payload.finish()?;
        self.offsets.push(offset);
        self.counts.push(count);
        self.pending.clear();
        Ok(())
    }
    fn finish(&mut self, directory: &Path) -> Result<()> {
        self.flush_mask()?;
        let mut encoder = self.encoder.take().context("mask already finalized")?;
        let mut image = encoder.image_directory()?;
        for (tag, value) in [
            (Tag::ImageWidth, self.crop.width as u32),
            (Tag::ImageLength, self.crop.height as u32),
            (Tag::RowsPerStrip, ROWS as u32),
        ] {
            crate::write_aligned_tag(&mut image, tag, value)?;
        }
        for (tag, value) in [
            (Tag::BitsPerSample, 8u16),
            (Tag::SamplesPerPixel, 1),
            (Tag::PhotometricInterpretation, 1),
            (Tag::Compression, 8),
            (Tag::Orientation, self.color.orientation),
            (Tag::PlanarConfiguration, 1),
        ] {
            crate::write_aligned_tag(&mut image, tag, value)?;
        }
        crate::write_aligned_tag(&mut image, Tag::StripOffsets, &self.offsets[..])?;
        crate::write_aligned_tag(&mut image, Tag::StripByteCounts, &self.counts[..])?;
        image.write_tag(
            Tag::ImageDescription,
            "Reference fallback: white=reference; black=multi-frame",
        )?;
        image.finish()?;
        // Mask writes use File directly, so write failures propagate through the encoder.
        drop(encoder);
        std::fs::OpenOptions::new()
            .write(true)
            .open(directory.join("reference-fallback.tif"))?
            .sync_all()?;
        let rgb = crate::preview::render_preview(&self.preview, &self.color)?;
        let mut rgb: Vec<u8> = rgb.into_iter().map(|v| (v >> 8) as u8).collect();
        for (i, marked) in self.marked.iter().enumerate() {
            if *marked {
                rgb[i * 3] = ((rgb[i * 3] as u16 + 255) / 2) as u8;
                rgb[i * 3 + 1] /= 2;
                rgb[i * 3 + 2] /= 2;
            }
        }
        let mut file = BufWriter::new(File::create(directory.join("overlay.tif"))?);
        {
            let mut encoder = TiffEncoder::new(&mut file)?;
            let mut image = encoder.new_image::<tiff::encoder::colortype::RGB8>(
                self.preview.width as u32,
                self.preview.height as u32,
            )?;
            image
                .encoder()
                .write_tag(Tag::Orientation, self.color.orientation)?;
            image
                .encoder()
                .write_tag(Tag::IccProfile, &include_bytes!("../assets/srgb.icc")[..])?;
            image.encoder().write_tag(Tag::ImageDescription, LEGEND)?;
            image.write_data(&rgb)?;
        }
        file.flush()?;
        file.get_ref().sync_all()?;
        self.completed = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiff::decoder::{Decoder, DecodingResult};
    fn color() -> Color {
        serde_json::from_value(json!({"white_balance":[1,1,1],"xyz_to_camera":[1,0,0,0,1,0,0,0,1]}))
            .unwrap()
    }
    struct Source;
    impl ImageSource for Source {
        fn dimensions(&self) -> (usize, usize) {
            (70, 75)
        }
        fn read_region(&self, r: Region) -> Result<RenderedRegion> {
            Image {
                width: 70,
                height: 75,
                rgb: vec![0.2; 70 * 75 * 3],
            }
            .read_region(r)
        }
        fn read_region_with_mask(&self, r: Region) -> Result<RenderedRegion> {
            let mut tile = self.read_region(r)?;
            let mask: Vec<u8> = (r.y..r.y + r.height)
                .flat_map(|y| {
                    (r.x..r.x + r.width).map(move |x| if (x + y) % 5 == 0 { 255 } else { 0 })
                })
                .collect();
            tile.fallback_pixels = mask.iter().filter(|v| **v != 0).count();
            tile.fallback_mask = Some(mask);
            Ok(tile)
        }
    }
    #[test]
    fn streams_cropped_mask_and_oriented_overlay_with_partial_strips() {
        for orientation in 1..=8 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("result.dng");
            let crop = Crop {
                x: 7,
                y: 5,
                width: 57,
                height: 66,
            };
            let color = Color {
                orientation,
                default_crop: Some(crop),
                ..color()
            };
            let diagnostics = MergeDiagnostics::new(&Source, &path, &color).unwrap();
            for y in (0..75).step_by(7) {
                let r = Region {
                    x: 0,
                    y,
                    width: 70,
                    height: 7.min(75 - y),
                };
                let tile = diagnostics.read_region(r).unwrap();
                assert!(tile.fallback_mask.is_none());
                assert_eq!(tile.image.rgb, Source.read_region(r).unwrap().image.rgb);
            }
            let result = diagnostics.publish(json!({})).unwrap();
            let mut decoder =
                Decoder::new(File::open(result["mask"].as_str().unwrap()).unwrap()).unwrap();
            assert_eq!(decoder.dimensions().unwrap(), (57, 66));
            assert_eq!(
                decoder.get_tag_u32(Tag::Orientation).unwrap(),
                orientation as u32
            );
            let expected: Vec<u8> = (5..71)
                .flat_map(|y| (7..64).map(move |x| if (x + y) % 5 == 0 { 255 } else { 0 }))
                .collect();
            let DecodingResult::U8(mask) = decoder.read_image().unwrap() else {
                panic!("expected mask bytes")
            };
            assert_eq!(mask, expected);
            let summary: Value = serde_json::from_slice(
                &std::fs::read(result["summary"].as_str().unwrap()).unwrap(),
            )
            .unwrap();
            assert_eq!(
                summary["fallback_pixels"],
                expected.iter().filter(|v| **v != 0).count()
            );
            assert!(summary["peak_mask_band_bytes"].as_u64().unwrap() <= (70 * 7 + 57 * 32) as u64);
            let mut overlay =
                Decoder::new(File::open(result["overlay"].as_str().unwrap()).unwrap()).unwrap();
            assert_eq!(overlay.dimensions().unwrap(), (57, 66));
            assert_eq!(
                overlay.get_tag_u32(Tag::Orientation).unwrap(),
                orientation as u32
            );
            let DecodingResult::U8(rgb) = overlay.read_image().unwrap() else {
                panic!("expected RGB bytes")
            };
            for (pixel, m) in rgb.as_chunks::<3>().0.iter().zip(mask) {
                if m != 0 {
                    assert!(pixel[0] > pixel[1] && pixel[0] > pixel[2]);
                }
            }
            assert!(MergeDiagnostics::new(&Source, &path, &color).is_err());
        }
    }
    #[test]
    fn interrupted_diagnostics_leave_no_sidecars_and_reject_unordered_regions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("result.dng");
        {
            let diagnostics = MergeDiagnostics::new(&Source, &path, &color()).unwrap();
            assert!(
                diagnostics
                    .read_region(Region {
                        x: 0,
                        y: 7,
                        width: 70,
                        height: 7
                    })
                    .is_err()
            );
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
