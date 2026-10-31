//! Lossless sensor stores. Files contain process-private native-endian samples;
//! they are scratch data, never an interchange format or a persistent cache.
use anyhow::{Context, Result, ensure};
use pixelshift_core::{Frame, SensorInfo, SensorSource};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    sync::Mutex,
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SourceStorage {
    #[default]
    Ram,
    Disk,
}

enum Samples {
    U16(Vec<u16>),
    F32(Vec<f32>),
}
enum Backing {
    Ram(Samples),
    Disk { file: Mutex<File>, integer: bool },
}
pub struct PackedSensor {
    info: SensorInfo,
    black: [f32; 4],
    white: [f32; 4],
    decoder_white: [f32; 4],
    backing: Backing,
}
impl PackedSensor {
    pub fn from_u16(
        info: SensorInfo,
        data: Vec<u16>,
        black: [f32; 4],
        white: [f32; 4],
        storage: SourceStorage,
        scratch: Option<&Path>,
    ) -> Result<Self> {
        ensure!(
            (0..4).all(|i| black[i].is_finite() && white[i].is_finite() && white[i] > black[i]),
            "invalid RAW normalization levels"
        );
        Self::new(info, Samples::U16(data), black, white, storage, scratch)
    }
    /// Synthetic float fixtures retain all bits; never quantize them to u16.
    pub fn from_frame(
        frame: Frame,
        storage: SourceStorage,
        scratch: Option<&Path>,
    ) -> Result<Self> {
        frame.validate()?;
        Self::new(
            frame.info(),
            Samples::F32(frame.pixels),
            [0.; 4],
            [1.; 4],
            storage,
            scratch,
        )
    }
    fn new(
        info: SensorInfo,
        samples: Samples,
        black: [f32; 4],
        white: [f32; 4],
        storage: SourceStorage,
        scratch: Option<&Path>,
    ) -> Result<Self> {
        info.validate()?;
        let len = match &samples {
            Samples::U16(v) => v.len(),
            Samples::F32(v) => v.len(),
        };
        ensure!(
            info.width.checked_mul(info.height) == Some(len),
            "invalid packed sensor buffer"
        );
        let backing = if storage == SourceStorage::Ram {
            Backing::Ram(samples)
        } else {
            let mut file = match scratch {
                Some(dir) => tempfile::tempfile_in(dir),
                None => tempfile::tempfile(),
            }
            .context(
                "creating sensor scratch file (check scratch directory and free disk space)",
            )?;
            let (bytes, integer) = match &samples {
                Samples::U16(v) => (bytemuck::cast_slice::<u16, u8>(v), true),
                Samples::F32(v) => (bytemuck::cast_slice::<f32, u8>(v), false),
            };
            file.write_all(bytes)
                .context("writing sensor scratch file (check free disk space)")?;
            // Unbuffered file writes: reads see all samples without a durable fsync.
            Backing::Disk {
                file: Mutex::new(file),
                integer,
            }
        };
        Ok(Self {
            info,
            black,
            white,
            decoder_white: white,
            backing,
        })
    }
    /// Change only the normalization denominator; stored native samples stay intact.
    /// Accept either the documented decoder scale or an already matching reference
    /// scale. Unknown RAW variants fail rather than receive a guessed conversion.
    pub fn apply_reference_normalization(
        &mut self,
        record: &crate::profiles::RawNormalization,
    ) -> Result<()> {
        record.validate()?;
        ensure!(
            matches!(
                &self.backing,
                Backing::Ram(Samples::U16(_)) | Backing::Disk { integer: true, .. }
            ),
            "reference RAW normalization requires native u16 samples"
        );
        let expected = record.decoder_white_level as f32;
        let target = record.white_level as f32;
        ensure!(
            self.decoder_white == [expected; 4] || self.decoder_white == [target; 4],
            "unverified decoder white levels {:?}; profile expects {} or reference {}; update the camera normalization reference",
            self.decoder_white,
            expected,
            target
        );
        ensure!(
            self.black.iter().all(|black| target > *black),
            "reference white level must exceed every black level"
        );
        self.white = [target; 4];
        Ok(())
    }
    pub fn decoder_white_levels(&self) -> [f32; 4] {
        self.decoder_white
    }
    pub fn normalization_levels(&self) -> ([f32; 4], [f32; 4]) {
        (self.black, self.white)
    }
    pub(crate) fn native_code(&self, x: usize, y: usize) -> Result<u16> {
        ensure!(
            x < self.info.width && y < self.info.height,
            "sample outside sensor"
        );
        match &self.backing {
            Backing::Ram(Samples::U16(data)) => Ok(data[y * self.info.width + x]),
            _ => anyhow::bail!("native diagnostics require a resident integer sensor"),
        }
    }
    pub fn set_sequence(&mut self, sequence: usize) {
        self.info.sequence = sequence;
    }
    pub fn resident_bytes(&self) -> u64 {
        match &self.backing {
            Backing::Ram(Samples::U16(v)) => v.capacity() as u64 * 2,
            Backing::Ram(Samples::F32(v)) => v.capacity() as u64 * 4,
            Backing::Disk { .. } => 0,
        }
    }
    pub fn scratch_bytes(&self) -> u64 {
        match &self.backing {
            Backing::Ram(_) => 0,
            Backing::Disk { integer, .. } => {
                self.info.width as u64 * self.info.height as u64 * if *integer { 2 } else { 4 }
            }
        }
    }
    fn normalize(&self, values: &[u16], start: usize) -> Vec<f32> {
        let span: [f32; 4] = std::array::from_fn(|i| self.white[i] - self.black[i]);
        let mut pixels = Vec::with_capacity(values.len());
        for (y, row) in values.chunks_exact(self.info.width).enumerate() {
            let phase_y = ((y + start) % 2) * 2;
            pixels.extend(row.iter().enumerate().map(|(x, v)| {
                let phase = phase_y + x % 2;
                (*v as f32 - self.black[phase]) / span[phase]
            }));
        }
        pixels
    }
}
impl SensorSource for PackedSensor {
    fn info(&self) -> SensorInfo {
        self.info.clone()
    }
    fn read_rows(&self, start: usize, count: usize) -> Result<Frame> {
        ensure!(
            start.is_multiple_of(2)
                && count > 0
                && start
                    .checked_add(count)
                    .is_some_and(|end| end <= self.info.height),
            "invalid sensor row window"
        );
        let begin = start * self.info.width;
        let len = count * self.info.width;
        let pixels = match &self.backing {
            Backing::Ram(Samples::U16(v)) => self.normalize(&v[begin..begin + len], start),
            Backing::Ram(Samples::F32(v)) => v[begin..begin + len].to_vec(),
            Backing::Disk { file, integer } => {
                let mut file = file
                    .lock()
                    .map_err(|_| anyhow::anyhow!("sensor scratch lock poisoned"))?;
                let file: &mut File = &mut file;
                let bytes = if *integer { 2 } else { 4 };
                file.seek(SeekFrom::Start(begin as u64 * bytes))?;
                if *integer {
                    let mut values = vec![0u16; len];
                    file.read_exact(bytemuck::cast_slice_mut(&mut values))
                        .context("reading packed sensor rows")?;
                    self.normalize(&values, start)
                } else {
                    let mut values = vec![0f32; len];
                    file.read_exact(bytemuck::cast_slice_mut(&mut values))
                        .context("reading float fixture rows")?;
                    values
                }
            }
        };
        Ok(self.info.frame(pixels, count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pixelshift_core::{ImageSource, Mode, Options, PreparedMerge, Region, SonyA7Cr, merge};
    #[test]
    fn reference_white_normalization_preserves_native_codes_and_both_backends() {
        let reference = crate::profiles::RawNormalization {
            source: "matched Adobe DNG".into(),
            decoder_white_level: 15360,
            white_level: 16383,
        };
        let codes = [500u16, 512, 1024, 8192, 15360, 16383, 18000, 65535];
        for storage in [SourceStorage::Ram, SourceStorage::Disk] {
            let info = SensorInfo {
                width: 8,
                height: 8,
                cfa: [0, 1, 1, 2],
                model: "ILCE-7CR".into(),
                sequence: 1,
            };
            let data: Vec<_> = (0..64).map(|i| codes[i % 8]).collect();
            let mut source = PackedSensor::from_u16(
                info.clone(),
                data.clone(),
                [512.; 4],
                [15360.; 4],
                storage,
                None,
            )
            .unwrap();
            source.apply_reference_normalization(&reference).unwrap();
            source.apply_reference_normalization(&reference).unwrap(); // Idempotent: no sample gain baked in.
            assert_eq!(source.decoder_white_levels(), [15360.; 4]);
            assert_eq!(source.normalization_levels(), ([512.; 4], [16383.; 4]));
            let frame = source.read_rows(0, 8).unwrap();
            for (index, value) in frame.pixels.iter().enumerate() {
                let expected = (data[index] as f64 - 512.) / (16383. - 512.);
                assert!((*value as f64 - expected).abs() < 3e-7);
                if storage == SourceStorage::Ram {
                    assert_eq!(
                        source.native_code(index % 8, index / 8).unwrap(),
                        data[index]
                    );
                }
            }
            assert!(frame.pixels[0] < 0.);
            assert_eq!(frame.pixels[5], 1.);
            assert!(frame.pixels[6] > 1.);
            let mut already_reference = PackedSensor::from_u16(
                info.clone(),
                data.clone(),
                [512.; 4],
                [16383.; 4],
                storage,
                None,
            )
            .unwrap();
            already_reference
                .apply_reference_normalization(&reference)
                .unwrap();
            assert_eq!(
                frame.pixels,
                already_reference.read_rows(0, 8).unwrap().pixels
            );
            let mut unknown =
                PackedSensor::from_u16(info, data, [512.; 4], [12000.; 4], storage, None).unwrap();
            assert!(unknown.apply_reference_normalization(&reference).is_err());
            assert_eq!(unknown.normalization_levels().1, [12000.; 4]);
            let mut invalid = reference.clone();
            invalid.white_level = 500;
            assert!(source.apply_reference_normalization(&invalid).is_err());
        }
    }
    #[test]
    fn packed_backends_match_float_reference_at_every_seam() {
        let scratch = tempfile::tempdir().unwrap();
        for mode in [Mode::Four, Mode::Sixteen, Mode::Denoise] {
            let n = if mode == Mode::Sixteen { 16 } else { 4 };
            let mut frames = Vec::new();
            let mut ram: Vec<Box<dyn SensorSource>> = Vec::new();
            let mut disk: Vec<Box<dyn SensorSource>> = Vec::new();
            for i in 0..n {
                // Odd width catches accidental linear-offset CFA phasing.
                let info = SensorInfo {
                    width: 51,
                    height: 47,
                    cfa: [2, 1, 1, 0],
                    model: "ILCE-7CR".into(),
                    sequence: i + 1,
                };
                let black = [512., 257., 131., 67.];
                let white = [14000., 15000., 16000., 16383.];
                let data: Vec<_> = (0..51 * 47)
                    .map(|p| ((p * 97 + i * 53) % 18000) as u16)
                    .collect();
                let pixels = data
                    .iter()
                    .enumerate()
                    .map(|(p, v)| {
                        let phase = ((p / 51) % 2) * 2 + (p % 51) % 2;
                        (*v as f32 - black[phase]) / (white[phase] - black[phase])
                    })
                    .collect();
                frames.push(info.frame(pixels, 47));
                ram.push(Box::new(
                    PackedSensor::from_u16(
                        info.clone(),
                        data.clone(),
                        black,
                        white,
                        SourceStorage::Ram,
                        None,
                    )
                    .unwrap(),
                ));
                disk.push(Box::new(
                    PackedSensor::from_u16(
                        info,
                        data,
                        black,
                        white,
                        SourceStorage::Disk,
                        Some(scratch.path()),
                    )
                    .unwrap(),
                ));
            }
            for threshold in [0.0, 0.08] {
                let opts = Options {
                    mode,
                    motion_threshold: threshold,
                    alignment_radius: 2,
                };
                let (reference, report) = merge(&frames, &SonyA7Cr, &opts).unwrap();
                for stores in [&ram, &disk] {
                    let plan = PreparedMerge::from_sources(stores, &SonyA7Cr, &opts).unwrap();
                    assert_eq!(plan.report(0).residual_shifts, report.residual_shifts);
                    for rows in [1, 7, 17] {
                        let mut pixels = Vec::new();
                        let mut fallbacks = 0;
                        for y in (0..reference.height).step_by(rows) {
                            let region = plan
                                .read_region(Region {
                                    x: 0,
                                    y,
                                    width: reference.width,
                                    height: rows.min(reference.height - y),
                                })
                                .unwrap();
                            pixels.extend(region.image.rgb);
                            fallbacks += region.fallback_pixels;
                        }
                        let scale = if mode == Mode::Sixteen { 2 } else { 1 };
                        let row_bound =
                            rows.div_ceil(scale) + 2 * (opts.alignment_radius as usize + 6) + 2;
                        assert!(plan.peak_source_region_bytes() <= n * 51 * row_bound * 4);
                        assert_eq!(fallbacks, report.fallback_pixels);
                        assert_eq!(
                            pixels.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                            reference
                                .rgb
                                .iter()
                                .map(|v| v.to_bits())
                                .collect::<Vec<_>>()
                        );
                    }
                    let region = plan
                        .read_region(Region {
                            x: 3,
                            y: 5,
                            width: 7,
                            height: 11,
                        })
                        .unwrap();
                    for y in 0..11 {
                        for x in 0..7 {
                            for c in 0..3 {
                                assert_eq!(
                                    region.image.rgb[(y * 7 + x) * 3 + c].to_bits(),
                                    reference.rgb[((y + 5) * reference.width + x + 3) * 3 + c]
                                        .to_bits()
                                );
                            }
                        }
                    }
                }
            }
            drop(disk);
            assert_eq!(std::fs::read_dir(scratch.path()).unwrap().count(), 0);
        }
    }
    #[test]
    fn store_bounds_levels_truncation_and_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let info = SensorInfo {
            width: 16,
            height: 17,
            cfa: [0, 1, 1, 2],
            model: "ILCE-7CR".into(),
            sequence: 1,
        };
        assert!(
            PackedSensor::from_u16(
                info.clone(),
                vec![0; 272],
                [0.; 4],
                [0.; 4],
                SourceStorage::Ram,
                None
            )
            .is_err()
        );
        assert!(
            PackedSensor::from_u16(
                info.clone(),
                vec![0; 1],
                [0.; 4],
                [100.; 4],
                SourceStorage::Disk,
                Some(dir.path())
            )
            .is_err()
        );
        let store = PackedSensor::from_u16(
            info,
            vec![42; 272],
            [0.; 4],
            [100.; 4],
            SourceStorage::Disk,
            Some(dir.path()),
        )
        .unwrap();
        assert_eq!(store.resident_bytes(), 0);
        assert_eq!(store.scratch_bytes(), 544);
        assert!(store.read_rows(1, 8).is_err());
        assert!(store.read_rows(16, 2).is_err());
        if let Backing::Disk { file, .. } = &store.backing {
            file.lock().unwrap().set_len(3).unwrap();
        }
        assert!(store.read_rows(0, 8).is_err());
        drop(store);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
