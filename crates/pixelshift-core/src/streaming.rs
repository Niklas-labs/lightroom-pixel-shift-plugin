//! Output-region reconstruction. Source frames remain in RAM for this milestone.
use super::*;

#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}
impl Region {
    pub fn validate(self, width: usize, height: usize) -> Result<()> {
        ensure!(
            self.width > 0
                && self.height > 0
                && self.x.checked_add(self.width).is_some_and(|n| n <= width)
                && self.y.checked_add(self.height).is_some_and(|n| n <= height),
            "invalid output region"
        );
        Ok(())
    }
}
pub struct RenderedRegion {
    pub image: Image,
    pub fallback_pixels: usize,
}
/// Pull-based output boundary. A writer controls how many regions are resident.
pub trait ImageSource: Sync {
    fn dimensions(&self) -> (usize, usize);
    fn read_region(&self, region: Region) -> Result<RenderedRegion>;
}
impl ImageSource for Image {
    fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }
    fn read_region(&self, region: Region) -> Result<RenderedRegion> {
        region.validate(self.width, self.height)?;
        ensure!(
            self.width
                .checked_mul(self.height)
                .and_then(|v| v.checked_mul(3))
                == Some(self.rgb.len()),
            "invalid RGB buffer"
        );
        let mut rgb = Vec::with_capacity(region.width * region.height * 3);
        for y in region.y..region.y + region.height {
            let start = (y * self.width + region.x) * 3;
            rgb.extend_from_slice(&self.rgb[start..start + region.width * 3]);
        }
        Ok(RenderedRegion {
            image: Image {
                width: region.width,
                height: region.height,
                rgb,
            },
            fallback_pixels: 0,
        })
    }
}
/// Validated geometry/alignment without allocating the reconstructed image.
pub struct PreparedMerge<'a> {
    pub(crate) frames: &'a [Frame],
    pub(crate) opts: Options,
    pub(crate) shifts: Vec<(f32, f32)>,
    pub(crate) residual: Vec<(i32, i32)>,
    pub(crate) margin: usize,
    pub(crate) scale: usize,
    pub(crate) width: usize,
    pub(crate) height: usize,
}
impl<'a> PreparedMerge<'a> {
    pub fn new(frames: &'a [Frame], profile: &dyn CameraProfile, opts: &Options) -> Result<Self> {
        ensure!(!frames.is_empty(), "no frames");
        ensure!(
            opts.motion_threshold.is_finite() && opts.motion_threshold >= 0.0,
            "invalid motion threshold"
        );
        ensure!(
            (0..=8).contains(&opts.alignment_radius),
            "alignment radius must be 0..8"
        );
        let expected = match opts.mode {
            Mode::Four => 4,
            Mode::Sixteen => 16,
            Mode::Denoise => frames.len(),
        };
        ensure!(
            frames.len() == expected && (2..=64).contains(&expected),
            "incorrect frame count"
        );
        let first = &frames[0];
        for (i, f) in frames.iter().enumerate() {
            f.validate()?;
            ensure!(
                f.model == profile.model(),
                "unsupported camera: {}",
                f.model
            );
            ensure!(
                (f.width, f.height, f.cfa) == (first.width, first.height, first.cfa),
                "inconsistent sensor geometry/CFA"
            );
            ensure!(
                f.sequence == i + 1,
                "sequence must be unique, complete, and ordered from 1"
            );
        }
        let shifts: Vec<_> = (0..frames.len())
            .map(|i| profile.shift(opts.mode, i))
            .collect();
        ensure!(
            shifts
                .iter()
                .all(|(x, y)| x.is_finite() && y.is_finite() && x.abs() <= 2.0 && y.abs() <= 2.0),
            "unsupported or unsafe camera shift layout"
        );
        let residual: Vec<_> = frames
            .iter()
            .enumerate()
            .map(|(i, f)| {
                if i == 0 {
                    (0, 0)
                } else {
                    alignment(first, f, shifts[0], shifts[i], opts.alignment_radius)
                }
            })
            .collect();
        let margin = 4 + opts.alignment_radius as usize;
        ensure!(
            first.width > 2 * margin && first.height > 2 * margin,
            "sensor too small for alignment/crop"
        );
        let scale = if opts.mode == Mode::Sixteen { 2 } else { 1 };
        let width = (first.width - 2 * margin) * scale;
        let height = (first.height - 2 * margin) * scale;

        Ok(Self {
            frames,
            opts: opts.clone(),
            shifts,
            residual,
            margin,
            scale,
            width,
            height,
        })
    }
    pub fn report(&self, fallback_pixels: usize) -> Report {
        Report {
            width: self.width,
            height: self.height,
            residual_shifts: self.residual.clone(),
            fallback_pixels,
            sensor_origin: (self.margin, self.margin),
        }
    }
}
impl ImageSource for PreparedMerge<'_> {
    fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }
    fn read_region(&self, region: Region) -> Result<RenderedRegion> {
        region.validate(self.width, self.height)?;
        let (frames, opts, shifts, residual, margin, scale) = (
            self.frames,
            &self.opts,
            &self.shifts,
            &self.residual,
            self.margin,
            self.scale,
        );
        let first = &frames[0];
        let mut rgb = vec![0.0; region.width * region.height * 3];
        let fallback = std::sync::atomic::AtomicUsize::new(0);
        rgb.par_chunks_mut(region.width * 3)
            .enumerate()
            .for_each(|(oy, row)| {
                for ox in 0..region.width {
                    let x = margin as f32 + (ox + region.x) as f32 / scale as f32;
                    let y = margin as f32 + (oy + region.y) as f32 / scale as f32;
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
                            } else if (sx - sx.round()).abs() < 1e-5
                                && (sy - sy.round()).abs() < 1e-5
                            {
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
        Ok(RenderedRegion {
            image: Image {
                width: region.width,
                height: region.height,
                rgb,
            },
            fallback_pixels: fallback.load(std::sync::atomic::Ordering::Relaxed),
        })
    }
}
