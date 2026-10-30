//! Map reference-camera crop coordinates into the reconstructed image grid.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Crop {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}
impl Crop {
    pub fn validate(self, width: usize, height: usize) -> Result<()> {
        ensure!(
            self.width > 0
                && self.height > 0
                && self.x.checked_add(self.width).is_some_and(|v| v <= width)
                && self.y.checked_add(self.height).is_some_and(|v| v <= height),
            "invalid image crop"
        );
        Ok(())
    }
}

/// The first shot observes scene coordinate (sensor + nominal shift). Intersect
/// its preferred crop with the merger's valid output, retaining integer samples.
/// This changes DNG display metadata only; it does not resample or resize pixels.
pub fn map_reference_crop(
    source: Crop,
    sensor_size: (usize, usize),
    sensor_origin: (usize, usize),
    reference_shift: (f32, f32),
    scale: usize,
    output_size: (usize, usize),
) -> Result<Crop> {
    source.validate(sensor_size.0, sensor_size.1)?;
    ensure!(matches!(scale, 1 | 2), "invalid crop scale");
    let axis = |start: usize,
                length: usize,
                origin: usize,
                shift: f32,
                limit: usize|
     -> Result<(usize, usize)> {
        ensure!(shift.is_finite(), "invalid crop shift");
        let begin = (start as f64 + shift as f64 - origin as f64) * scale as f64;
        let end = begin + length as f64 * scale as f64;
        let left = begin.max(0.0).ceil() as usize;
        let right = end.min(limit as f64).floor().max(0.0) as usize;
        ensure!(
            right > left,
            "source crop does not intersect valid merged image"
        );
        Ok((left, right - left))
    };
    let (x, width) = axis(
        source.x,
        source.width,
        sensor_origin.0,
        reference_shift.0,
        output_size.0,
    )?;
    let (y, height) = axis(
        source.y,
        source.height,
        sensor_origin.1,
        reference_shift.1,
        output_size.1,
    )?;
    let result = Crop {
        x,
        y,
        width,
        height,
    };
    result.validate(output_size.0, output_size.1)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_crop_accounts_for_shift_origin_and_sixteen_shot_scale() {
        let source = Crop {
            x: 48,
            y: 20,
            width: 9504,
            height: 6336,
        };
        assert_eq!(
            map_reference_crop(source, (9600, 6376), (6, 6), (1.0, 0.0), 1, (9588, 6364)).unwrap(),
            Crop {
                x: 43,
                y: 14,
                width: 9504,
                height: 6336
            }
        );
        assert_eq!(
            map_reference_crop(source, (9600, 6376), (6, 6), (1.5, 0.0), 2, (19176, 12728))
                .unwrap(),
            Crop {
                x: 87,
                y: 28,
                width: 19008,
                height: 12672
            }
        );
    }
    #[test]
    fn safety_border_intersection_and_invalid_geometry() {
        let source = Crop {
            x: 0,
            y: 0,
            width: 9504,
            height: 6336,
        };
        assert_eq!(
            map_reference_crop(source, (9600, 6376), (6, 6), (1.0, 0.0), 1, (9588, 6364)).unwrap(),
            Crop {
                x: 0,
                y: 0,
                width: 9499,
                height: 6330
            }
        );
        assert!(
            map_reference_crop(
                Crop {
                    x: 0,
                    y: 0,
                    width: 9601,
                    height: 6336
                },
                (9600, 6376),
                (6, 6),
                (0.0, 0.0),
                1,
                (9588, 6364)
            )
            .is_err()
        );
    }
}
