//! Small display preview only. Never apply this transform to linear RAW payloads.
//! DNG 1.7.1 chapter 6: inverse-temperature calibration interpolation and
//! FM * diag(1 / ReferenceNeutral) * inverse(CameraCalibration), with AB = I.
use crate::{Color, inverse};
use anyhow::{Result, ensure};
use pixelshift_core::Image;

type Matrix = [f32; 9];
const D50_TO_SRGB: Matrix = [
    3.133_856, -1.6168667, -0.4906146, -0.9787684, 1.9161415, 0.0334540, 0.0719453, -0.2289914,
    1.4052427,
];
fn multiply(a: Matrix, b: Matrix) -> Matrix {
    std::array::from_fn(|i| (0..3).map(|k| a[i / 3 * 3 + k] * b[k * 3 + i % 3]).sum())
}
fn vector(a: Matrix, v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| (0..3).map(|c| a[r * 3 + c] * v[c]).sum())
}
fn interpolate(a: Matrix, d65: Matrix, daylight: f32) -> Matrix {
    std::array::from_fn(|i| a[i] * (1. - daylight) + d65[i] * daylight)
}
/// McCamy's approximate CCT is sufficient for this A/D65-only display preview.
/// It is not an Adobe Temp/Tint solver. Out-of-range/undefined CCT falls back
/// to D65; rendering of the RAW in Lightroom is governed by its original tags.
fn daylight_weight(xyz: [f32; 3]) -> f32 {
    let sum: f64 = xyz.iter().map(|v| *v as f64).sum();
    if !sum.is_finite() || sum <= 0. {
        return 1.;
    }
    let x = xyz[0] as f64 / sum;
    let y = xyz[1] as f64 / sum;
    if (0.1858 - y).abs() < 1e-8 {
        return 1.;
    }
    let n = (x - 0.3320) / (0.1858 - y);
    let temperature = 449. * n * n * n + 3525. * n * n + 6823.3 * n + 5520.33;
    if !temperature.is_finite() || temperature <= 0. {
        return 1.;
    }
    let a = 1. / 2856.;
    let d65 = 1. / 6504.;
    ((a - 1. / temperature) / (a - d65)).clamp(0., 1.) as f32
}
fn as_shot_weight(color: &Color, neutral: [f32; 3]) -> Result<f32> {
    let Some(matrix_a) = color.xyz_to_camera_a else {
        return Ok(1.);
    };
    let rendering = color.dng_rendering.as_ref().expect("calibrated renderer");
    let mut weight = 0.5;
    for _ in 0..48 {
        let cm = interpolate(matrix_a, color.xyz_to_camera, weight);
        let cc = interpolate(
            rendering.camera_calibration_a,
            rendering.camera_calibration_d65,
            weight,
        );
        let xyz = vector(inverse(multiply(cc, cm))?, neutral);
        let next = daylight_weight(xyz);
        if (next - weight).abs() < 1e-6 {
            return Ok(next);
        }
        // Damping makes iterative camera-neutral/white-point estimation stable.
        weight = (weight + next) * 0.5;
    }
    Ok(weight)
}
fn transform(color: &Color) -> Result<Matrix> {
    let rendering = color.dng_rendering.as_ref().expect("calibrated renderer");
    rendering.validate()?;
    ensure!(
        color.white_balance.iter().all(|v| v.is_finite() && *v > 0.),
        "invalid preview white balance"
    );
    inverse(color.xyz_to_camera)?;
    let neutral = color.white_balance.map(|wb| color.white_balance[1] / wb);
    let weight = as_shot_weight(color, neutral)?;
    let cc_inverse = inverse(interpolate(
        rendering.camera_calibration_a,
        rendering.camera_calibration_d65,
        weight,
    ))?;
    let reference_neutral = vector(cc_inverse, neutral);
    ensure!(
        reference_neutral.iter().all(|v| v.is_finite() && *v > 0.),
        "invalid preview reference neutral"
    );
    let fm = interpolate(
        rendering.forward_matrix_a,
        rendering.forward_matrix_d65,
        weight,
    );
    let diagonal: Matrix = [
        1. / reference_neutral[0],
        0.,
        0.,
        0.,
        1. / reference_neutral[1],
        0.,
        0.,
        0.,
        1. / reference_neutral[2],
    ];
    let xyz = multiply(fm, multiply(diagonal, cc_inverse));
    let gain = rendering.baseline_exposure_ev.exp2();
    let output = multiply(D50_TO_SRGB, xyz).map(|v| v * gain);
    ensure!(
        output.iter().all(|v| v.is_finite()),
        "invalid preview transform"
    );
    Ok(output)
}
pub(crate) fn render_preview(image: &Image, color: &Color) -> Result<Vec<u16>> {
    if color.dng_rendering.is_none() {
        return crate::render_srgb(image, color);
    }
    ensure!(
        image
            .width
            .checked_mul(image.height)
            .and_then(|n| n.checked_mul(3))
            == Some(image.rgb.len()),
        "invalid preview RGB buffer"
    );
    ensure!(
        image.rgb.iter().all(|v| v.is_finite()),
        "non-finite preview sample"
    );
    ensure!(
        (1..=8).contains(&color.orientation),
        "invalid preview orientation"
    );
    let transform = transform(color)?;
    let mut output = Vec::with_capacity(image.rgb.len());
    for pixel in image.rgb.as_chunks::<3>().0 {
        let rgb = vector(transform, *pixel);
        for value in rgb {
            // Display clipping happens only after the full linear color/exposure
            // transform. Negative noise/headroom stay intact in the RAW payload.
            let linear = value.clamp(0., 1.);
            let encoded = if linear <= 0.0031308 {
                12.92 * linear
            } else {
                1.055 * linear.powf(1. / 2.4) - 0.055
            };
            output.push((encoded * 65535.).round() as u16);
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::{Catalog, DngRendering};
    const SRGB_TO_XYZ_D50: Matrix = [
        0.4360747, 0.3850649, 0.1430804, 0.2225045, 0.7168786, 0.0606169, 0.0139322, 0.0971045,
        0.7141733,
    ];
    fn linear_srgb_camera() -> Color {
        let mut color: Color = serde_json::from_value(serde_json::json!({
            "white_balance": [1,1,1],
            "xyz_to_camera": [3.2404542,-1.5371385,-0.4985314,-0.969266,1.8760108,0.041556,0.0556434,-0.2040259,1.0572252]
        })).unwrap();
        color.dng_rendering = Some(DngRendering {
            source: "analytic sRGB reference camera".into(),
            baseline_exposure_ev: 0.,
            calibration_signature: "test".into(),
            camera_calibration_a: [1., 0., 0., 0., 1., 0., 0., 0., 1.],
            camera_calibration_d65: [1., 0., 0., 0., 1., 0., 0., 0., 1.],
            forward_matrix_a: SRGB_TO_XYZ_D50,
            forward_matrix_d65: SRGB_TO_XYZ_D50,
        });
        color
    }
    fn encoded(linear: f32) -> i32 {
        let v = linear.clamp(0., 1.);
        let s = if v <= 0.0031308 {
            12.92 * v
        } else {
            1.055 * v.powf(1. / 2.4) - 0.055
        };
        (s * 65535.).round() as i32
    }
    #[test]
    fn forward_matrix_and_exposure_are_applied_before_display_encoding() {
        let image = Image {
            width: 3,
            height: 1,
            rgb: vec![-0.02, 0., 0.001, 0.18, 0.35, 0.7, 1.4, 0.06, 0.4],
        };
        let original = image.rgb.clone();
        for ev in [-1., 0., 1.] {
            let mut color = linear_srgb_camera();
            color.dng_rendering.as_mut().unwrap().baseline_exposure_ev = ev;
            let preview = render_preview(&image, &color).unwrap();
            for (sample, value) in preview.iter().zip(&image.rgb) {
                assert!((*sample as i32 - encoded(value * ev.exp2())).abs() <= 5);
            }
        }
        assert_eq!(image.rgb, original); // Display clipping never mutates RAW samples.
    }
    #[test]
    fn nondiagonal_camera_calibration_and_reference_white_balance_are_respected() {
        let mut color = linear_srgb_camera();
        let cc = [1., 0.1, 0., 0., 1., 0., 0.02, 0., 1.];
        let reference_neutral = [0.5, 1., 2. / 3.];
        let neutral = vector(cc, reference_neutral);
        color.white_balance = neutral.map(|n| 1. / n);
        let rendering = color.dng_rendering.as_mut().unwrap();
        rendering.camera_calibration_a = cc;
        rendering.camera_calibration_d65 = cc;
        let target = [0.3, 0.1, 0.2];
        let raw = vector(
            cc,
            std::array::from_fn(|i| target[i] * reference_neutral[i]),
        );
        let preview = render_preview(
            &Image {
                width: 1,
                height: 1,
                rgb: raw.to_vec(),
            },
            &color,
        )
        .unwrap();
        for c in 0..3 {
            assert!((preview[c] as i32 - encoded(target[c])).abs() <= 5);
        }
    }
    #[test]
    fn dual_illuminant_neutral_estimation_and_reciprocal_temperature_weights() {
        assert!(daylight_weight([1.09850, 1., 0.35585]) < 0.002);
        assert!(daylight_weight([0.95047, 1., 1.08883]) > 0.998);
        assert_eq!(daylight_weight([0., 0., 0.]), 1.);
        let mut color = linear_srgb_camera();
        color.xyz_to_camera = [
            0.82, -0.2976, -0.0719, -0.4296, 1.2053, 0.2532, -0.0429, 0.1282, 0.5774,
        ];
        color.xyz_to_camera_a = Some([
            0.9185, -0.4857, 0.0505, -0.3651, 1.1061, 0.2982, -0.0161, 0.0698, 0.6769,
        ]);
        let neutral = [0.372228, 1., 0.645243];
        color.white_balance = neutral.map(|n| 1. / n);
        color.dng_rendering = Catalog::embedded()
            .unwrap()
            .find("ILCE-7CR")
            .unwrap()
            .dng_rendering
            .clone();
        let weight = as_shot_weight(&color, neutral).unwrap();
        // The validated capture is near 5850K; using identity calibration instead
        // moves this substantially toward D65 (the former 6350K discrepancy).
        assert!((0.90..0.93).contains(&weight), "weight={weight}");
        let raw = neutral.map(|n| n * 0.18);
        let preview = render_preview(
            &Image {
                width: 1,
                height: 1,
                rgb: raw.to_vec(),
            },
            &color,
        )
        .unwrap();
        for value in preview {
            assert!((value as i32 - encoded(0.18 * 0.35f32.exp2())).abs() < 20);
        }
    }
    #[test]
    fn legacy_fallback_and_invalid_preview_inputs() {
        let image = Image {
            width: 1,
            height: 1,
            rgb: vec![0.18; 3],
        };
        let mut color = linear_srgb_camera();
        color.dng_rendering = None;
        assert_eq!(
            render_preview(&image, &color).unwrap(),
            crate::render_srgb(&image, &color).unwrap()
        );
        color = linear_srgb_camera();
        color.white_balance[0] = 0.;
        assert!(render_preview(&image, &color).is_err());
        color = linear_srgb_camera();
        color.dng_rendering.as_mut().unwrap().forward_matrix_d65 = [0.; 9];
        assert!(render_preview(&image, &color).is_err());
        assert!(
            render_preview(
                &Image {
                    width: 1,
                    height: 1,
                    rgb: vec![f32::NAN; 3]
                },
                &linear_srgb_camera()
            )
            .is_err()
        );
    }
}
