use anyhow::{Context, Result, ensure};
use pixelshift_core::Mode;
use pixelshift_io::Capture;
use std::{collections::HashSet, path::PathBuf};

pub fn mode(requested: Option<&str>, count: usize) -> Result<Mode> {
    match requested.unwrap_or("auto") {
        "auto" => match count {
            4 => Ok(Mode::Four),
            16 => Ok(Mode::Sixteen),
            _ => anyhow::bail!("Pixel Shift requires exactly 4 or 16 source RAWs"),
        },
        "four" => {
            ensure!(count == 4, "four-shot mode requires 4 sources");
            Ok(Mode::Four)
        }
        "sixteen" => {
            ensure!(count == 16, "sixteen-shot mode requires 16 sources");
            Ok(Mode::Sixteen)
        }
        "denoise" => {
            ensure!((2..=64).contains(&count), "denoise requires 2..64 sources");
            Ok(Mode::Denoise)
        }
        value => anyhow::bail!("unsupported mode: {value}"),
    }
}
/// Validate and order headers before any full-resolution sensor decoding.
pub fn preflight(
    paths: &mut [PathBuf],
    mode: Mode,
    requested_camera: Option<&str>,
) -> Result<String> {
    let captures = paths
        .iter()
        .map(|p| pixelshift_io::inspect_capture(p))
        .collect::<Result<Vec<_>>>()?;
    let camera = validate(&captures, mode, requested_camera)?;
    let mut indices: Vec<_> = (0..paths.len()).collect();
    indices.sort_by_key(|i| captures[*i].sequence.unwrap_or(0));
    let ordered: Vec<_> = indices.iter().map(|i| paths[*i].clone()).collect();
    paths.clone_from_slice(&ordered);
    Ok(camera)
}
fn validate(captures: &[Capture], mode: Mode, requested: Option<&str>) -> Result<String> {
    let first = captures.first().context("no sources")?;
    let camera = first.model.as_deref().context("missing RAW camera model")?;
    ensure!(
        camera == "ILCE-7CR",
        "unsupported camera {camera}; supported: Sony a7CR (ILCE-7CR)"
    );
    ensure!(
        requested.is_none_or(|r| r == "auto" || r == camera),
        "requested camera disagrees with RAW metadata"
    );
    let dimensions = first.dimensions.context("missing RAW image dimensions")?;
    ensure!(
        dimensions.0 >= 8 && dimensions.1 >= 8,
        "invalid RAW dimensions"
    );
    let required = match mode {
        Mode::Four => 4,
        Mode::Sixteen => 16,
        Mode::Denoise => captures.len(),
    };
    ensure!(captures.len() == required, "incomplete capture");
    let mut shots = HashSet::new();
    for capture in captures {
        ensure!(
            capture.model.as_deref() == Some(camera),
            "mixed camera models"
        );
        ensure!(
            capture.dimensions == Some(dimensions),
            "mixed RAW dimensions"
        );
        ensure!(
            capture.iso == first.iso
                && capture.exposure == first.exposure
                && capture.aperture == first.aperture
                && capture.serial == first.serial,
            "mixed exposure or camera serial"
        );
        if mode != Mode::Denoise {
            let shift = capture
                .pixel_shift
                .as_ref()
                .context("missing Sony PixelShiftInfo; use denoise for ordinary bursts")?;
            let group = first
                .pixel_shift
                .as_ref()
                .context("missing Sony PixelShiftInfo")?;
            ensure!(
                shift.group_id == group.group_id,
                "mixed Pixel Shift capture groups"
            );
            ensure!(
                usize::from(shift.shot_count) == required,
                "selection does not match metadata shot count"
            );
            ensure!(
                (1..=required).contains(&usize::from(shift.shot_number)),
                "invalid source-shot index"
            );
            ensure!(
                shots.insert(shift.shot_number),
                "duplicate source-shot index"
            );
        }
    }
    Ok(camera.to_owned())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn captures(n: usize) -> Vec<Capture> {
        (1..=n).map(|i|serde_json::from_value(serde_json::json!({"model":"ILCE-7CR","dimensions":[9504,6336],"SequenceNumber":i,"ISO":100,"ExposureTime":0.01,"FNumber":8,"pixel_shift":{"group_id":42,"shot_number":i,"shot_count":n}})).unwrap()).collect()
    }
    #[test]
    fn detects_supported_modes_and_rejects_ambiguous_selections() {
        for n in [4, 16] {
            let c = captures(n);
            assert_eq!(
                validate(&c, mode(None, n).unwrap(), None).unwrap(),
                "ILCE-7CR"
            );
        }
        assert!(mode(None, 8).is_err());
        assert!(mode(Some("four"), 16).is_err());
    }
    #[test]
    fn rejects_mixed_incomplete_and_unsupported() {
        for case in 0..7 {
            let mut c = captures(4);
            match case {
                0 => c[1].model = Some("ILCE-7RM5".into()),
                1 => c[1].dimensions = Some((10, 10)),
                2 => c[1].pixel_shift.as_mut().unwrap().group_id = 99,
                3 => c[1].pixel_shift.as_mut().unwrap().shot_number = 1,
                4 => c[1].pixel_shift.as_mut().unwrap().shot_count = 16,
                5 => c[1].pixel_shift = None,
                _ => c[1].iso = Some(200.),
            };
            assert!(validate(&c, Mode::Four, None).is_err(), "case {case}");
        }
        assert!(validate(&captures(4), Mode::Four, Some("other")).is_err());
    }
}
