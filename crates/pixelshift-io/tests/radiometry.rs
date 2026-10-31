//! Absolute photometry oracle: expected values come from an analytic scene,
//! not from another merge implementation or a rendered preview.
use pixelshift_core::{
    ImageSource, Mode, Options, PreparedMerge, SensorInfo, SensorSource, SonyA7Cr,
};
use pixelshift_io::{
    Color, OutputCompression, PackedSensor, SourceStorage, WriteOptions, write_dng_from_source,
};

fn shift(mode: Mode, i: usize) -> (f32, f32) {
    // Independent chronological acquisition table, deliberately not CameraProfile::shift.
    if mode == Mode::Denoise {
        return (0., 0.);
    }
    let (x, y) = [(1., 0.), (1., 1.), (0., 1.), (0., 0.)][i % 4];
    let (hx, hy) = if mode == Mode::Sixteen {
        [(0.5, 0.), (0., 0.), (0.5, 0.5), (0., 0.5)][i / 4]
    } else {
        (0., 0.)
    };
    (x + hx, y + hy)
}

fn scene(case: usize, x: f32, y: f32, c: usize) -> f32 {
    match case {
        0 => -1. / 64., // Preserve below-black noise; no premature clipping.
        1 => 1. / 128.,
        2 => 0.1875,
        3 => 0.5,
        4 => 1.125, // Preserve highlight headroom.
        _ => [-0.03125, 0.1875, 1.125][c] + (c + 1) as f32 * x / 2048. + (3 - c) as f32 * y / 4096.,
    }
}

#[test]
fn native_sensor_to_dng_preserves_absolute_brightness_and_color() {
    let temp = tempfile::tempdir().unwrap();
    let mut worst_error = 0.0f64;
    let mut worst_ev = 0.0f64;
    let mut cases = 0;
    for mode in [Mode::Four, Mode::Sixteen, Mode::Denoise] {
        for storage in [SourceStorage::Ram, SourceStorage::Disk] {
            // Case 6 forces reference fallback; case 7 averages balanced noise.
            for case in 0..8 {
                if case == 7 && mode != Mode::Denoise {
                    continue;
                }
                let count = if mode == Mode::Sixteen { 16 } else { 4 };
                let sources: Vec<Box<dyn SensorSource>> = (0..count)
                    .map(|i| {
                        let cfa = [0, 1, 1, 2];
                        let black = [512., 1024., 768., 1280.];
                        let range = [8192., 16384., 8192., 16384.];
                        let white = std::array::from_fn(|p| black[p] + range[p]);
                        let (sx, sy) = shift(mode, i);
                        let data = (0..40 * 32)
                            .map(|p| {
                                let (x, y) = (p % 40, p / 40);
                                let phase = (y % 2) * 2 + x % 2;
                                let mut value =
                                    scene(case, x as f32 + sx, y as f32 + sy, cfa[phase]);
                                if case == 6 && i > 0 {
                                    value += 0.25;
                                } else if case == 7 {
                                    value += if i % 2 == 0 { -1. / 1024. } else { 1. / 1024. };
                                }
                                let code = black[phase] + value * range[phase];
                                // All analytic values are exactly representable native codes:
                                // quantization cannot hide a normalization or merge bias.
                                assert_eq!(code, code.round());
                                assert!((0.0..=65535.0).contains(&code));
                                code as u16
                            })
                            .collect();
                        Box::new(
                            PackedSensor::from_u16(
                                SensorInfo {
                                    width: 40,
                                    height: 32,
                                    cfa,
                                    model: "ILCE-7CR".into(),
                                    sequence: i + 1,
                                },
                                data,
                                black,
                                white,
                                storage,
                                Some(temp.path()),
                            )
                            .unwrap(),
                        ) as Box<dyn SensorSource>
                    })
                    .collect();
                let merge = PreparedMerge::from_sources(
                    &sources,
                    &SonyA7Cr,
                    &Options {
                        mode,
                        motion_threshold: 0.08,
                        alignment_radius: 0,
                    },
                )
                .unwrap();
                let color = Color {
                    default_crop: None,
                    photo_metadata: Default::default(),
                    white_balance: [2., 1., 1.5],
                    xyz_to_camera: [
                        0.82, -0.2976, -0.0719, -0.4296, 1.2053, 0.2532, -0.0429, 0.1282, 0.5774,
                    ],
                    dng_rendering: None,
                    xyz_to_camera_a: None,
                    orientation: 1,
                };
                let path = temp.path().join(format!("{mode:?}-{storage:?}-{case}.dng"));
                let stats = write_dng_from_source(
                    &path,
                    &merge,
                    &color,
                    WriteOptions {
                        compression: OutputCompression::None,
                        strip_rows: 7, // Odd bands exercise source-window phase handling.
                    },
                    &mut |_, _| Ok(()),
                )
                .unwrap();
                if case == 6 {
                    assert_eq!(
                        stats.fallback_pixels,
                        merge.dimensions().0 * merge.dimensions().1
                    );
                } else {
                    assert_eq!(stats.fallback_pixels, 0);
                }
                let decoded = rawler::decode_file(&path).unwrap();
                let rawler::RawImageData::Float(rgb) = decoded.data else {
                    panic!("expected linear float DNG");
                };
                let origin = merge.report(0).sensor_origin;
                let scale = if mode == Mode::Sixteen { 2. } else { 1. };
                let mut actual_sum = [0.0f64; 3];
                let mut expected_sum = [0.0f64; 3];
                for y in 0..decoded.height {
                    for x in 0..decoded.width {
                        for c in 0..3 {
                            let expected = scene(
                                case,
                                origin.0 as f32 + x as f32 / scale,
                                origin.1 as f32 + y as f32 / scale,
                                c,
                            );
                            let actual = rgb[(y * decoded.width + x) * 3 + c];
                            let error = (actual as f64 - expected as f64).abs();
                            worst_error = worst_error.max(error);
                            assert!(
                                error < 1e-6,
                                "{mode:?}/{storage:?}/case {case} ({x},{y},{c}): {actual} != {expected}"
                            );
                            actual_sum[c] += actual as f64;
                            expected_sum[c] += expected as f64;
                        }
                    }
                }
                for c in 0..3 {
                    if expected_sum[c] > 0. {
                        let ev = (actual_sum[c] / expected_sum[c]).log2();
                        worst_ev = worst_ev.max(ev.abs());
                        assert!(ev.abs() < 1e-6, "channel brightness bias: {ev} EV");
                    }
                }
                cases += 1;
            }
        }
    }
    println!(
        "{cases} native sensor -> DNG cases: max sample error {worst_error:.9}, max mean channel bias {worst_ev:.9} EV"
    );
}

#[test]
fn profiled_a7cr_native_merge_matches_adobe_normalized_sensor_scale() {
    use pixelshift_core::{CameraProfile, ImageSource};
    use pixelshift_io::profiles::Catalog;
    let temp = tempfile::tempdir().unwrap();
    let catalog = Catalog::embedded().unwrap();
    let profile = catalog.find("ILCE-7CR").unwrap();
    let native_scene = |x: f32, y: f32, c: usize| 2000. + c as f32 * 2000. + 4. * x + 2. * y;
    for mode in [Mode::Four, Mode::Sixteen, Mode::Denoise] {
        for storage in [SourceStorage::Ram, SourceStorage::Disk] {
            let count = if mode == Mode::Sixteen { 16 } else { 4 };
            let sources: Vec<Box<dyn SensorSource>> = (0..count)
                .map(|i| {
                    let (dx, dy) = shift(mode, i);
                    assert_eq!(profile.shift(mode, i), (dx, dy));
                    let cfa = [0, 1, 1, 2];
                    let codes = (0..40 * 32)
                        .map(|p| {
                            let (x, y) = (p % 40, p / 40);
                            let c = cfa[(y % 2) * 2 + x % 2];
                            (512. + native_scene(x as f32 + dx, y as f32 + dy, c)) as u16
                        })
                        .collect();
                    let mut store = PackedSensor::from_u16(
                        SensorInfo {
                            width: 40,
                            height: 32,
                            cfa,
                            model: "ILCE-7CR".into(),
                            sequence: i + 1,
                        },
                        codes,
                        [512.; 4],
                        [15360.; 4],
                        storage,
                        Some(temp.path()),
                    )
                    .unwrap();
                    store
                        .apply_reference_normalization(profile.raw_normalization.as_ref().unwrap())
                        .unwrap();
                    Box::new(store) as Box<dyn SensorSource>
                })
                .collect();
            let source = PreparedMerge::from_sources(
                &sources,
                profile,
                &Options {
                    mode,
                    motion_threshold: 0.08,
                    alignment_radius: 0,
                },
            )
            .unwrap();
            let mut color: Color = serde_json::from_value(serde_json::json!({
                "white_balance": [2.,1.,1.5],
                "xyz_to_camera": [0.82,-0.2976,-0.0719,-0.4296,1.2053,0.2532,-0.0429,0.1282,0.5774]
            }))
            .unwrap();
            color.dng_rendering = profile.dng_rendering.clone();
            let path = temp
                .path()
                .join(format!("profiled-{mode:?}-{storage:?}.dng"));
            write_dng_from_source(
                &path,
                &source,
                &color,
                WriteOptions {
                    compression: OutputCompression::None,
                    strip_rows: 7,
                },
                &mut |_, _| Ok(()),
            )
            .unwrap();
            let decoded = rawler::decode_file(&path).unwrap();
            let rawler::RawImageData::Float(rgb) = decoded.data else {
                panic!("expected float DNG")
            };
            let (width, height) = source.dimensions();
            let origin = source.report(0).sensor_origin;
            let scale = if mode == Mode::Sixteen { 2. } else { 1. };
            for y in 0..height {
                for x in 0..width {
                    for c in 0..3 {
                        let expected = native_scene(
                            origin.0 as f32 + x as f32 / scale,
                            origin.1 as f32 + y as f32 / scale,
                            c,
                        ) as f64
                            / (16383. - 512.);
                        let actual = rgb[(y * width + x) * 3 + c] as f64;
                        assert!(
                            (actual - expected).abs() < 1e-6,
                            "{mode:?}/{storage:?}: {actual} vs {expected}"
                        );
                    }
                }
            }
        }
    }
}
