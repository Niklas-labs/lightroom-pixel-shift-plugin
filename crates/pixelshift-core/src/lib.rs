//! Sensor-domain processing. Coordinates and shifts always refer to the uncropped sensor.
use anyhow::{Result, ensure};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
mod streaming;
pub use streaming::{ImageSource, PreparedMerge, Region, RenderedRegion};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Four,
    Sixteen,
    Denoise,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    /// Normalized linear sensor values, black subtracted, no white balance.
    pub pixels: Vec<f32>,
    /// Row-major 2x2 Bayer layout; 0=R, 1=G, 2=B.
    pub cfa: [usize; 4],
    pub model: String,
    pub sequence: usize,
}
impl Frame {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.width >= 8 && self.height >= 8,
            "sensor must be at least 8x8"
        );
        ensure!(
            self.width.checked_mul(self.height) == Some(self.pixels.len()),
            "invalid sensor buffer"
        );
        ensure!(
            self.pixels.iter().all(|v| v.is_finite()),
            "non-finite sensor sample"
        );
        let mut c = self.cfa;
        c.sort();
        ensure!(c == [0, 1, 1, 2], "only 2x2 RGB Bayer sensors supported");
        Ok(())
    }
    /// Bilinear interpolation on each color's own 2-pixel lattice.
    pub fn sample(&self, x: f32, y: f32, channel: usize) -> f32 {
        let mut sum = 0.0;
        let mut count = 0;
        for py in 0..2 {
            for px in 0..2 {
                if self.cfa[py * 2 + px] != channel {
                    continue;
                }
                let gx = (x - px as f32) / 2.0;
                let gy = (y - py as f32) / 2.0;
                let ix = gx.floor() as isize;
                let iy = gy.floor() as isize;
                let fx = gx - gx.floor();
                let fy = gy - gy.floor();
                let mut value = 0.0;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let sx = (ix + dx) * 2 + px as isize;
                        let sy = (iy + dy) * 2 + py as isize;
                        let sx = sx
                            .clamp(px as isize, ((self.width - 1 - px) / 2 * 2 + px) as isize)
                            as usize;
                        let sy = sy
                            .clamp(py as isize, ((self.height - 1 - py) / 2 * 2 + py) as isize)
                            as usize;
                        let wx = if dx == 0 { 1.0 - fx } else { fx };
                        let wy = if dy == 0 { 1.0 - fy } else { fy };
                        value += self.pixels[sy * self.width + sx] * wx * wy;
                    }
                }
                sum += value;
                count += 1;
            }
        }
        sum / count as f32
    }
}

/// Sensor-layout abstraction; production profiles are loaded from the IO camera catalog.
pub trait CameraProfile {
    fn model(&self) -> &str;
    fn shift(&self, mode: Mode, index: usize) -> (f32, f32);
}
pub struct SonyA7Cr;
impl CameraProfile for SonyA7Cr {
    fn model(&self) -> &str {
        "ILCE-7CR"
    }
    fn shift(&self, mode: Mode, index: usize) -> (f32, f32) {
        if mode == Mode::Denoise {
            return (0.0, 0.0);
        }
        let (x, y) = [(1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.0, 0.0)][index % 4];
        let (hx, hy) = if mode == Mode::Sixteen {
            [(0.5, 0.0), (0.0, 0.0), (0.5, 0.5), (0.0, 0.5)][index / 4]
        } else {
            (0.0, 0.0)
        };
        (x + hx, y + hy)
    }
}
#[derive(Clone, Debug)]
pub struct Options {
    pub mode: Mode,
    /// Absolute difference in normalized linear camera RGB; zero disables local fallback.
    pub motion_threshold: f32,
    /// Integer residual translation search in sensor pixels (0 disables).
    pub alignment_radius: i32,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub width: usize,
    pub height: usize,
    pub residual_shifts: Vec<(i32, i32)>,
    pub fallback_pixels: usize,
    pub sensor_origin: (usize, usize),
}
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<f32>,
}

fn alignment(
    reference: &Frame,
    frame: &Frame,
    base: (f32, f32),
    shift: (f32, f32),
    radius: i32,
) -> (i32, i32) {
    let mut best = (f64::INFINITY, 0, 0);
    let step = (reference.width.min(reference.height) / 64).max(2);
    // Prefer zero in textureless scenes and ties.
    let mut candidates: Vec<_> = (-radius..=radius)
        .flat_map(|y| (-radius..=radius).map(move |x| (x, y)))
        .collect();
    candidates.sort_by_key(|(x, y)| x * x + y * y);
    for (dx, dy) in candidates {
        let mut error = 0.0;
        let mut n = 0;
        let margin = radius as usize + 4;
        for y in (margin..reference.height.saturating_sub(margin)).step_by(step) {
            for x in (margin..reference.width.saturating_sub(margin)).step_by(step) {
                let a = reference.sample(x as f32 - base.0, y as f32 - base.1, 1);
                let b = frame.sample(
                    x as f32 - shift.0 + dx as f32,
                    y as f32 - shift.1 + dy as f32,
                    1,
                );
                error += (a - b).abs().min(0.1) as f64;
                n += 1;
            }
        }
        if n > 0 && error / (n as f64) < best.0 {
            best = (error / n as f64, dx, dy);
        }
    }
    (best.1, best.2)
}

pub fn merge(
    frames: &[Frame],
    profile: &dyn CameraProfile,
    opts: &Options,
) -> Result<(Image, Report)> {
    let prepared = PreparedMerge::new(frames, profile, opts)?;
    let first = &frames[0];
    let (shifts, residual, margin, scale, width, height) = (
        &prepared.shifts,
        &prepared.residual,
        prepared.margin,
        prepared.scale,
        prepared.width,
        prepared.height,
    );
    let mut rgb = vec![0.0; width * height * 3];
    let fallback = std::sync::atomic::AtomicUsize::new(0);
    rgb.par_chunks_mut(width * 3)
        .enumerate()
        .for_each(|(oy, row)| {
            for ox in 0..width {
                let x = margin as f32 + ox as f32 / scale as f32;
                let y = margin as f32 + oy as f32 / scale as f32;
                let refx = x - shifts[0].0;
                let refy = y - shifts[0].1;
                let reference: [f32; 3] = std::array::from_fn(|c| first.sample(refx, refy, c));
                let moving = opts.motion_threshold > 0.0
                    && frames.iter().enumerate().skip(1).any(|(i, f)| {
                        (0..3).any(|c| {
                            (f.sample(
                                x - shifts[i].0 + residual[i].0 as f32,
                                y - shifts[i].1 + residual[i].1 as f32,
                                c,
                            ) - reference[c])
                                .abs()
                                > opts.motion_threshold
                        })
                    });
                if moving {
                    fallback.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                for channel in 0..3 {
                    if moving {
                        row[ox * 3 + channel] = first.sample(refx, refy, channel);
                        continue;
                    }
                    let mut sum = 0.0;
                    let mut count = 0;
                    for (i, f) in frames.iter().enumerate() {
                        let sx = x - shifts[i].0 + residual[i].0 as f32;
                        let sy = y - shifts[i].1 + residual[i].1 as f32;
                        if opts.mode == Mode::Denoise {
                            sum += f.sample(sx, sy, channel);
                            count += 1;
                        } else if (sx - sx.round()).abs() < 1e-5 && (sy - sy.round()).abs() < 1e-5 {
                            let ix = sx.round() as usize;
                            let iy = sy.round() as usize;
                            if f.cfa[(iy % 2) * 2 + ix % 2] == channel {
                                sum += f.pixels[iy * f.width + ix];
                                count += 1;
                            }
                        }
                    }
                    row[ox * 3 + channel] = if count > 0 {
                        sum / count as f32
                    } else {
                        first.sample(refx, refy, channel)
                    };
                }
            }
        });
    Ok((
        Image { width, height, rgb },
        Report {
            width,
            height,
            residual_shifts: residual.clone(),
            fallback_pixels: fallback.load(std::sync::atomic::Ordering::Relaxed),
            sensor_origin: (margin, margin),
        },
    ))
}
