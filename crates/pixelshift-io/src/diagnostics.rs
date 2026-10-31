//! Read-only, bounded sensor sampling through the production RAW/store boundary.
use crate::{Crop, PackedSensor, RawDecoder, SonyArwDecoder, SourceStorage};
use anyhow::{Result, ensure};
use pixelshift_core::SensorSource;
use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
pub struct SensorSample {
    /// Coordinates relative to the decoder's preferred visible crop.
    x: usize,
    y: usize,
    phase: usize,
    channel: usize,
    code: u16,
    normalized: f32,
}
#[derive(Serialize)]
pub struct SensorDiagnostics {
    model: String,
    dimensions: (usize, usize),
    crop: Crop,
    cfa: [usize; 4],
    black: [f32; 4],
    white: [f32; 4],
    decoder_white: [f32; 4],
    normalization_source: Option<String>,
    normalization: &'static str,
    samples: Vec<SensorSample>,
}
pub fn inspect_sensor_samples(
    path: &Path,
    catalog: &crate::profiles::Catalog,
) -> Result<SensorDiagnostics> {
    let (metadata, mut store) = SonyArwDecoder.decode_to_store(path, SourceStorage::Ram, None)?;
    let info = store.info();
    let profile = catalog.find(&info.model)?;
    if let Some(reference) = &profile.raw_normalization {
        store.apply_reference_normalization(reference)?;
    }
    let mut report = sample_store(
        &store,
        metadata.source_crop().unwrap_or(Crop {
            x: 0,
            y: 0,
            width: info.width,
            height: info.height,
        }),
    )?;
    report.normalization_source = profile.raw_normalization.as_ref().map(|r| r.source.clone());
    Ok(report)
}
fn sample_store(store: &PackedSensor, crop: Crop) -> Result<SensorDiagnostics> {
    let info = store.info();
    crop.validate(info.width, info.height)?;
    ensure!(
        crop.width >= 4 && crop.height >= 4,
        "crop too small for sampling"
    );
    let (black, white) = store.normalization_levels();
    let cols = 32.min(crop.width / 2);
    let rows = 24.min(crop.height / 2);
    let mut samples = Vec::with_capacity(cols * rows * 4);
    for row in 0..rows {
        let y = 2 * (row * ((crop.height - 2) / 2) / (rows - 1));
        // An even absolute origin preserves the store's Bayer phase convention.
        let origin = (crop.y + y) / 2 * 2;
        let end = (crop.y + y + 2).min(info.height);
        let window = store.read_rows(origin, end - origin)?;
        for col in 0..cols {
            let x = 2 * (col * ((crop.width - 2) / 2) / (cols - 1));
            for dy in 0..2 {
                for dx in 0..2 {
                    let sx = crop.x + x + dx;
                    let sy = crop.y + y + dy;
                    let phase = (sy % 2) * 2 + sx % 2;
                    samples.push(SensorSample {
                        x: x + dx,
                        y: y + dy,
                        phase,
                        channel: info.cfa[phase],
                        code: store.native_code(sx, sy)?,
                        normalized: window.pixels[(sy - origin) * info.width + sx],
                    });
                }
            }
        }
    }
    Ok(SensorDiagnostics {
        model: info.model,
        dimensions: (info.width, info.height),
        crop,
        cfa: info.cfa,
        black,
        white,
        decoder_white: store.decoder_white_levels(),
        normalization_source: None,
        normalization: "(native_code - phase_black) / (phase_white - phase_black); no clipping",
        samples,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use pixelshift_core::SensorInfo;
    #[test]
    fn sample_grid_preserves_codes_normalization_and_odd_crop_phase() {
        let info = SensorInfo {
            width: 48,
            height: 40,
            cfa: [0, 1, 1, 2],
            model: "ILCE-7CR".into(),
            sequence: 1,
        };
        let data: Vec<u16> = (0..48 * 40).map(|i| (i * 3 + 200) as u16).collect();
        let black = [512., 1024., 768., 1280.];
        let white = [16000., 15000., 14000., 16000.];
        let store =
            PackedSensor::from_u16(info, data.clone(), black, white, SourceStorage::Ram, None)
                .unwrap();
        let crop = Crop {
            x: 3,
            y: 5,
            width: 32,
            height: 24,
        };
        let report = sample_store(&store, crop).unwrap();
        assert_eq!(report.samples.len(), 16 * 12 * 4);
        assert!(report.samples.iter().any(|p| p.normalized < 0.));
        for point in &report.samples {
            let (x, y) = (crop.x + point.x, crop.y + point.y);
            let phase = (y % 2) * 2 + x % 2;
            assert_eq!(point.phase, phase);
            assert_eq!(point.channel, [0, 1, 1, 2][phase]);
            assert_eq!(point.code, data[y * 48 + x]);
            let expected = (point.code as f64 - black[phase] as f64)
                / (white[phase] as f64 - black[phase] as f64);
            assert!((point.normalized as f64 - expected).abs() < 1e-7);
        }
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("SerialNumber"));
        assert!(!json.contains("path"));
        assert!(sample_store(&store, Crop { width: 1, ..crop }).is_err());
    }
}
