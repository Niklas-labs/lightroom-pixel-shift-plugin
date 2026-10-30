//! Output-region reconstruction with bounded normalized source windows.
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
enum Sources<'a> {
    Frames(&'a [Frame]),
    Stores(&'a [Box<dyn SensorSource>]),
}
impl Sources<'_> {
    fn infos(&self) -> Vec<SensorInfo> {
        match self {
            Self::Frames(f) => f.iter().map(SensorSource::info).collect(),
            Self::Stores(s) => s.iter().map(|s| s.info()).collect(),
        }
    }
    fn source(&self, i: usize) -> &dyn SensorSource {
        match self {
            Self::Frames(f) => &f[i],
            Self::Stores(s) => s[i].as_ref(),
        }
    }
}
// Same sample positions and accumulation order as the whole-frame alignment.
// Load one guide row window at a time, independent of sensor height.
fn stored_alignment(
    reference: &dyn SensorSource,
    frame: &dyn SensorSource,
    base: (f32, f32),
    shift: (f32, f32),
    radius: i32,
) -> Result<(i32, i32)> {
    let info = reference.info();
    let step = (info.width.min(info.height) / 64).max(2);
    let margin = radius as usize + 4;
    let mut candidates: Vec<_> = (-radius..=radius)
        .flat_map(|y| (-radius..=radius).map(move |x| (x, y)))
        .collect();
    candidates.sort_by_key(|(x, y)| x * x + y * y);
    let mut errors = vec![0.0f64; candidates.len()];
    let mut n = 0;
    for y in (margin..info.height.saturating_sub(margin)).step_by(step) {
        let start = y.saturating_sub(radius as usize + 4) / 2 * 2;
        let end = (y + radius as usize + 5).min(info.height);
        let a = reference.read_rows(start, end - start)?;
        let b = frame.read_rows(start, end - start)?;
        for x in (margin..info.width.saturating_sub(margin)).step_by(step) {
            let av = a.sample(x as f32 - base.0, y as f32 - base.1 - start as f32, 1);
            for (i, (dx, dy)) in candidates.iter().enumerate() {
                let bv = b.sample(
                    x as f32 - shift.0 + *dx as f32,
                    y as f32 - shift.1 + *dy as f32 - start as f32,
                    1,
                );
                errors[i] += (av - bv).abs().min(0.1) as f64;
            }
            n += 1;
        }
    }
    let mut best = (f64::INFINITY, 0, 0);
    for ((dx, dy), error) in candidates.into_iter().zip(errors) {
        if n > 0 && error / (n as f64) < best.0 {
            best = (error / n as f64, dx, dy);
        }
    }
    Ok((best.1, best.2))
}
/// Validated geometry/alignment without allocating the reconstructed image.
pub struct PreparedMerge<'a> {
    sources: Sources<'a>,
    pub(crate) opts: Options,
    pub(crate) shifts: Vec<(f32, f32)>,
    pub(crate) residual: Vec<(i32, i32)>,
    pub(crate) margin: usize,
    pub(crate) scale: usize,
    pub(crate) width: usize,
    pub(crate) height: usize,
    peak_source_region_bytes: std::sync::atomic::AtomicUsize,
}
impl<'a> PreparedMerge<'a> {
    pub fn new(frames: &'a [Frame], profile: &dyn CameraProfile, opts: &Options) -> Result<Self> {
        for frame in frames {
            frame.validate()?;
        }
        Self::prepare(Sources::Frames(frames), profile, opts)
    }
    pub fn from_sources(
        sources: &'a [Box<dyn SensorSource>],
        profile: &dyn CameraProfile,
        opts: &Options,
    ) -> Result<Self> {
        Self::prepare(Sources::Stores(sources), profile, opts)
    }
    fn prepare(sources: Sources<'a>, profile: &dyn CameraProfile, opts: &Options) -> Result<Self> {
        let frames = sources.infos();
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
            .map(|(i, _f)| {
                if i == 0 {
                    Ok((0, 0))
                } else {
                    match &sources {
                        Sources::Frames(frames) => Ok(alignment(
                            &frames[0],
                            &frames[i],
                            shifts[0],
                            shifts[i],
                            opts.alignment_radius,
                        )),
                        Sources::Stores(_) => stored_alignment(
                            sources.source(0),
                            sources.source(i),
                            shifts[0],
                            shifts[i],
                            opts.alignment_radius,
                        ),
                    }
                }
            })
            .collect::<Result<Vec<_>>>()?;
        let margin = 4 + opts.alignment_radius as usize;
        ensure!(
            first.width > 2 * margin && first.height > 2 * margin,
            "sensor too small for alignment/crop"
        );
        let scale = if opts.mode == Mode::Sixteen { 2 } else { 1 };
        let width = (first.width - 2 * margin) * scale;
        let height = (first.height - 2 * margin) * scale;

        Ok(Self {
            sources,
            peak_source_region_bytes: std::sync::atomic::AtomicUsize::new(0),
            opts: opts.clone(),
            shifts,
            residual,
            margin,
            scale,
            width,
            height,
        })
    }
    /// Largest simultaneously resident normalized source window payload during output.
    /// Excludes packed stores, decoder workspaces, alignment guides and allocator overhead.
    pub fn peak_source_region_bytes(&self) -> usize {
        self.peak_source_region_bytes
            .load(std::sync::atomic::Ordering::Relaxed)
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
        let info = self.sources.infos()[0].clone();
        let origin = match &self.sources {
            Sources::Frames(_) => 0,
            Sources::Stores(_) => {
                (self.margin + region.y / self.scale)
                    .saturating_sub(self.opts.alignment_radius as usize + 6)
                    / 2
                    * 2
            }
        };
        let loaded;
        let frames = match &self.sources {
            Sources::Frames(frames) => *frames,
            Sources::Stores(stores) => {
                let end = (self.margin
                    + (region.y + region.height).div_ceil(self.scale)
                    + self.opts.alignment_radius as usize
                    + 6)
                .min(info.height);
                loaded = stores
                    .iter()
                    .map(|source| source.read_rows(origin, end - origin))
                    .collect::<Result<Vec<_>>>()?;
                self.peak_source_region_bytes.fetch_max(
                    loaded.iter().map(|f| f.pixels.capacity() * 4).sum(),
                    std::sync::atomic::Ordering::Relaxed,
                );
                &loaded
            }
        };
        let sample = |f: &Frame, x: f32, y: f32, c: usize| f.sample(x, y - origin as f32, c);
        let (opts, shifts, residual, margin, scale) = (
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
                    let reference: [f32; 3] = std::array::from_fn(|c| sample(first, refx, refy, c));
                    let moving = opts.motion_threshold > 0.0
                        && frames.iter().enumerate().skip(1).any(|(i, f)| {
                            (0..3).any(|c| {
                                (sample(
                                    f,
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
                            row[ox * 3 + channel] = sample(first, refx, refy, channel);
                            continue;
                        }
                        let mut sum = 0.0;
                        let mut count = 0;
                        for (i, f) in frames.iter().enumerate() {
                            let sx = x - shifts[i].0 + residual[i].0 as f32;
                            let sy = y - shifts[i].1 + residual[i].1 as f32;
                            if opts.mode == Mode::Denoise {
                                sum += sample(f, sx, sy, channel);
                                count += 1;
                            } else if (sx - sx.round()).abs() < 1e-5
                                && (sy - sy.round()).abs() < 1e-5
                            {
                                let ix = sx.round() as usize;
                                let iy = sy.round() as usize;
                                if f.cfa[(iy % 2) * 2 + ix % 2] == channel {
                                    sum += f.pixels[(iy - origin) * f.width + ix];
                                    count += 1;
                                }
                            }
                        }
                        row[ox * 3 + channel] = if count > 0 {
                            sum / count as f32
                        } else {
                            sample(first, refx, refy, channel)
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
