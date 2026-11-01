use pixelshift_core::*;
fn scene(x: f32, y: f32, c: usize) -> f32 {
    0.3 + c as f32 * 0.04 + 0.08 * (x * 1.7).sin() + 0.04 * (y * 1.1).cos()
}
fn frames(mode: Mode) -> Vec<Frame> {
    let n = if mode == Mode::Sixteen { 16 } else { 4 };
    (0..n)
        .map(|i| {
            // Independent chronological Sony sequence table, not the profile implementation.
            let integer = [(1., 0.), (1., 1.), (0., 1.), (0., 0.)][i % 4];
            let half = if mode == Mode::Sixteen {
                [(0.5, 0.), (0., 0.), (0.5, 0.5), (0., 0.5)][i / 4]
            } else {
                (0., 0.)
            };
            let shift = if mode == Mode::Denoise {
                (0., 0.)
            } else {
                (integer.0 + half.0, integer.1 + half.1)
            };
            let cfa = [0, 1, 1, 2];
            Frame {
                width: 40,
                height: 32,
                cfa,
                sequence: i + 1,
                model: "ILCE-7CR".into(),
                pixels: (0..40 * 32)
                    .map(|p| {
                        scene(
                            (p % 40) as f32 + shift.0,
                            (p / 40) as f32 + shift.1,
                            cfa[(p / 40 % 2) * 2 + p % 2],
                        )
                    })
                    .collect(),
            }
        })
        .collect()
}
fn options(mode: Mode) -> Options {
    Options {
        mode,
        motion_threshold: 0.,
        alignment_radius: 0,
    }
}
#[test]
fn four_shots_recover_direct_color_samples() {
    let (im, report) = merge(&frames(Mode::Four), &SonyA7Cr, &options(Mode::Four)).unwrap();
    assert_eq!((im.width, im.height), (32, 24));
    for y in 0..im.height {
        for x in 0..im.width {
            for c in 0..3 {
                assert!(
                    (im.rgb[(y * im.width + x) * 3 + c] - scene((x + 4) as f32, (y + 4) as f32, c))
                        .abs()
                        < 1e-6
                );
            }
        }
    }
    assert_eq!(report.fallback_pixels, 0);
}
#[test]
fn sixteen_shots_recover_true_half_pixel_detail() {
    let (im, _) = merge(&frames(Mode::Sixteen), &SonyA7Cr, &options(Mode::Sixteen)).unwrap();
    assert_eq!((im.width, im.height), (64, 48));
    for y in 0..im.height {
        for x in 0..im.width {
            for c in 0..3 {
                assert!(
                    (im.rgb[(y * im.width + x) * 3 + c]
                        - scene(4. + x as f32 / 2., 4. + y as f32 / 2., c))
                    .abs()
                        < 1e-6
                );
            }
        }
    }
}
#[test]
fn denoise_reduces_independent_sensor_noise() {
    let mut fs = frames(Mode::Denoise);
    let mut seed = 42u32;
    for f in &mut fs {
        for p in &mut f.pixels {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            *p += ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.04;
        }
    }
    let clean = frames(Mode::Denoise);
    let (im, _) = merge(&fs, &SonyA7Cr, &options(Mode::Denoise)).unwrap();
    let mut baseline = 0.;
    let mut reduced = 0.;
    for y in 0..im.height {
        for x in 0..im.width {
            for c in 0..3 {
                let expected = clean[0].sample((x + 4) as f32, (y + 4) as f32, c);
                baseline += (fs[0].sample((x + 4) as f32, (y + 4) as f32, c) - expected).powi(2);
                reduced += (im.rgb[(y * im.width + x) * 3 + c] - expected).powi(2);
            }
        }
    }
    assert!(
        reduced < baseline * 0.4,
        "variance ratio {}",
        reduced / baseline
    );
}
#[test]
fn moving_region_falls_back_to_reference() {
    let mut fs = frames(Mode::Denoise);
    for y in 10..20 {
        for x in 10..20 {
            fs[1].pixels[y * 40 + x] += 0.4;
        }
    }
    let mut opts = options(Mode::Denoise);
    opts.motion_threshold = 0.1;
    let (im, r) = merge(&fs, &SonyA7Cr, &opts).unwrap();
    assert!(r.fallback_pixels > 0 && r.fallback_pixels < im.width * im.height);
    for c in 0..3 {
        assert_eq!(
            im.rgb[((14 - 4) * im.width + 14 - 4) * 3 + c],
            fs[0].sample(14., 14., c)
        );
    }
}
#[test]
fn translation_alignment_recovers_shift() {
    let mut fs = frames(Mode::Denoise);
    // Move every channel consistently by two sensor pixels.
    for y in 0..32 {
        for x in 0..40 {
            let c = fs[1].cfa[(y % 2) * 2 + x % 2];
            fs[1].pixels[y * 40 + x] = scene(x as f32 - 2., y as f32, c);
        }
    }
    let mut opts = options(Mode::Denoise);
    opts.alignment_radius = 3;
    let (_, report) = merge(&fs, &SonyA7Cr, &opts).unwrap();
    assert_eq!(report.residual_shifts[1], (2, 0));
}
#[test]
fn reject_bad_captures_and_options() {
    let fs = frames(Mode::Four);
    assert!(merge(&fs[..3], &SonyA7Cr, &options(Mode::Four)).is_err());
    for bad in 0..5 {
        let mut fs = fs.clone();
        match bad {
            0 => fs[1].sequence = 1,
            1 => fs[1].model = "other".into(),
            2 => fs[1].width = 12,
            3 => fs[1].pixels[0] = f32::NAN,
            _ => fs[1].cfa = [0, 0, 0, 0],
        }
        assert!(merge(&fs, &SonyA7Cr, &options(Mode::Four)).is_err());
    }
    let mut opts = options(Mode::Four);
    opts.motion_threshold = f32::NAN;
    assert!(merge(&fs, &SonyA7Cr, &opts).is_err());
}

#[test]
fn red_only_motion_is_detected() {
    let mut fs = frames(Mode::Denoise);
    for y in 10..20 {
        for x in 10..20 {
            if x % 2 == 0 && y % 2 == 0 {
                fs[1].pixels[y * 40 + x] += 0.4;
            }
        }
    }
    let mut opts = options(Mode::Denoise);
    opts.motion_threshold = 0.1;
    let (im, report) = merge(&fs, &SonyA7Cr, &opts).unwrap();
    assert!(report.fallback_pixels > 0);
    assert_eq!(
        im.rgb[((14 - 4) * im.width + 14 - 4) * 3],
        fs[0].sample(14., 14., 0)
    );
}

#[test]
fn output_regions_match_whole_image_reference_across_seams_and_workers() {
    for mode in [Mode::Four, Mode::Sixteen, Mode::Denoise] {
        let mut sources = frames(mode);
        for y in 10..19 {
            for x in 14..26 {
                sources[1].pixels[y * 40 + x] += 0.25;
            }
        }
        let opts = Options {
            mode,
            motion_threshold: 0.06,
            alignment_radius: 1,
        };
        let (reference, report) = merge(&sources, &SonyA7Cr, &opts).unwrap();
        for workers in [1, 3] {
            for (tw, th) in [(1, 1), (7, 5), (13, 11)] {
                rayon::ThreadPoolBuilder::new()
                    .num_threads(workers)
                    .build()
                    .unwrap()
                    .install(|| {
                        let prepared = PreparedMerge::new(&sources, &SonyA7Cr, &opts).unwrap();
                        let (width, height) = prepared.dimensions();
                        let mut assembled = vec![0.0; width * height * 3];
                        let mut fallback = 0;
                        for y in (0..height).step_by(th) {
                            for x in (0..width).step_by(tw) {
                                let rw = tw.min(width - x);
                                let rh = th.min(height - y);
                                let region = prepared
                                    .read_region(Region {
                                        x,
                                        y,
                                        width: rw,
                                        height: rh,
                                    })
                                    .unwrap();
                                fallback += region.fallback_pixels;
                                for row in 0..rh {
                                    let to = ((y + row) * width + x) * 3;
                                    let from = row * rw * 3;
                                    assembled[to..to + rw * 3]
                                        .copy_from_slice(&region.image.rgb[from..from + rw * 3]);
                                }
                            }
                        }
                        assert_eq!(
                            assembled.iter().map(|n| n.to_bits()).collect::<Vec<_>>(),
                            reference
                                .rgb
                                .iter()
                                .map(|n| n.to_bits())
                                .collect::<Vec<_>>()
                        );
                        assert_eq!(fallback, report.fallback_pixels);
                        assert_eq!(
                            prepared.report(fallback).residual_shifts,
                            report.residual_shifts
                        );
                    });
            }
        }
    }
}
#[test]
fn output_region_bounds_are_checked() {
    let frames = frames(Mode::Four);
    let plan = PreparedMerge::new(&frames, &SonyA7Cr, &options(Mode::Four)).unwrap();
    for region in [
        Region {
            x: 0,
            y: 0,
            width: 0,
            height: 1,
        },
        Region {
            x: 31,
            y: 0,
            width: 2,
            height: 1,
        },
        Region {
            x: 0,
            y: 23,
            width: 1,
            height: 2,
        },
        Region {
            x: usize::MAX,
            y: 0,
            width: 2,
            height: 1,
        },
    ] {
        assert!(plan.read_region(region).is_err());
    }
}

#[test]
fn diagnostic_mask_matches_fallback_without_changing_rgb() {
    for mode in [Mode::Four, Mode::Sixteen, Mode::Denoise] {
        let mut frames = frames(mode);
        // Inject a local disagreement while leaving most of the scene stationary.
        for y in 13..19 {
            for x in 15..21 {
                frames[1].pixels[y * 40 + x] += 0.7;
            }
        }
        for threshold in [0., 0.2] {
            let mut options = options(mode);
            options.motion_threshold = threshold;
            let prepared = PreparedMerge::new(&frames, &SonyA7Cr, &options).unwrap();
            let (width, height) = prepared.dimensions();
            let whole = prepared
                .read_region_with_mask(Region {
                    x: 0,
                    y: 0,
                    width,
                    height,
                })
                .unwrap();
            let mask = whole.fallback_mask.as_ref().unwrap();
            assert_eq!(
                whole.fallback_pixels,
                mask.iter().filter(|v| **v == 255).count()
            );
            if threshold > 0. {
                assert!(whole.fallback_pixels > 0 && whole.fallback_pixels < width * height);
            } else {
                assert_eq!(whole.fallback_pixels, 0);
            }
            let mut band_mask = vec![];
            let mut band_rgb = vec![];
            for y in (0..height).step_by(7) {
                let region = Region {
                    x: 0,
                    y,
                    width,
                    height: 7.min(height - y),
                };
                let ordinary = prepared.read_region(region).unwrap();
                let diagnostic = prepared.read_region_with_mask(region).unwrap();
                assert!(ordinary.fallback_mask.is_none());
                assert_eq!(ordinary.image.rgb, diagnostic.image.rgb);
                assert_eq!(ordinary.fallback_pixels, diagnostic.fallback_pixels);
                band_mask.extend(diagnostic.fallback_mask.unwrap());
                band_rgb.extend(diagnostic.image.rgb);
            }
            assert_eq!(band_mask, *mask);
            assert_eq!(band_rgb, whole.image.rgb);
        }
    }
}
