//! Versioned camera capabilities; numeric sensor calibration remains decoder-owned.
use anyhow::{Context, Result, ensure};
use pixelshift_core::{CameraProfile, Mode};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
const EMBEDDED: &str = include_str!("../../../config/camera-profiles.yaml");
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub schema_version: u32,
    pub cameras: Vec<Profile>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub make: String,
    pub model: String,
    pub name: String,
    pub validation_status: String,
    pub decoder: String,
    pub metadata: String,
    pub calibration_source: String,
    pub cfa: String,
    pub denoise: bool,
    pub pixel_shift: Layouts,
    pub notes: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Layouts {
    pub four: Option<Layout>,
    pub sixteen: Option<Layout>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub output_scale: usize,
    pub offsets: Vec<[f32; 2]>,
}
fn validate_yaml(value: &serde_yaml_ng::Value) -> Result<()> {
    use serde_yaml_ng::Value;
    match value {
        Value::Tagged(_) => anyhow::bail!("custom YAML tags are not supported"),
        Value::Mapping(mapping) => {
            for (key, value) in mapping {
                ensure!(key.is_string(), "YAML mapping keys must be strings");
                validate_yaml(value)?;
            }
        }
        Value::Sequence(values) => {
            for value in values {
                validate_yaml(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}
impl Catalog {
    pub fn parse(text: &str) -> Result<Self> {
        let yaml: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(text).context("invalid camera profile YAML")?;
        validate_yaml(&yaml)?;
        let raw: serde_json::Value = serde_json::to_value(yaml)
            .context("camera profiles must use YAML with string keys and JSON-compatible values")?;
        if let Some(schema) = raw.get("$schema") {
            ensure!(schema.is_string(), "$schema must be a string");
        }
        if let Some(cameras) = raw.get("cameras").and_then(|v| v.as_array()) {
            for camera in cameras {
                if let Some(layouts) = camera.get("pixel_shift").and_then(|v| v.as_object()) {
                    ensure!(
                        layouts.values().all(|v| v.is_object()),
                        "layouts must be objects, not null"
                    );
                }
            }
        }
        let catalog: Self =
            serde_json::from_value(raw).context("invalid camera profile YAML values")?;
        ensure!(
            catalog.schema_version == 1,
            "unsupported camera profile schema version"
        );
        ensure!(!catalog.cameras.is_empty(), "camera catalog is empty");
        let mut models = HashSet::new();
        for p in &catalog.cameras {
            ensure!(
                models.insert(&p.model),
                "duplicate camera model {}",
                p.model
            );
            ensure!(
                !p.model.trim().is_empty()
                    && p.model == p.model.trim()
                    && !p.name.trim().is_empty()
                    && !p.notes.trim().is_empty(),
                "empty or invalid camera identity/documentation"
            );
            ensure!(
                p.make == "SONY"
                    && p.decoder == "sony_arw"
                    && p.metadata == "sony_pixel_shift_v1"
                    && p.calibration_source == "raw_metadata_and_rawler_catalog",
                "unsupported camera adapter/calibration source"
            );
            ensure!(
                ["experimental", "validated"].contains(&p.validation_status.as_str()),
                "invalid validation status"
            );
            p.cfa_pattern()?;
            ensure!(
                p.pixel_shift.four.is_some() || p.pixel_shift.sixteen.is_some(),
                "profile needs a Pixel Shift layout"
            );
            for (layout, n, scale) in [(&p.pixel_shift.four, 4, 1), (&p.pixel_shift.sixteen, 16, 2)]
            {
                if let Some(layout) = layout {
                    ensure!(
                        layout.output_scale == scale && layout.offsets.len() == n,
                        "invalid {n}-shot scale/count for {}",
                        p.model
                    );
                    let mut offsets = HashSet::new();
                    for [x, y] in &layout.offsets {
                        let valid = |v: f32| {
                            v.is_finite()
                                && if n == 4 {
                                    [0., 1.].contains(&v)
                                } else {
                                    [0., 0.5, 1., 1.5].contains(&v)
                                }
                        };
                        ensure!(
                            valid(*x) && valid(*y),
                            "invalid sensor offset for {}",
                            p.model
                        );
                        ensure!(
                            offsets.insert(((*x * 2.) as i32, (*y * 2.) as i32)),
                            "duplicate sensor offset"
                        );
                    }
                }
            }
        }
        Ok(catalog)
    }
    pub fn embedded() -> Result<Self> {
        Self::parse(EMBEDDED)
    }
    pub fn load(path: Option<&Path>) -> Result<Self> {
        if let Some(path) = path {
            return Self::parse(
                &std::fs::read_to_string(path)
                    .with_context(|| format!("reading camera profiles {}", path.display()))?,
            );
        }
        let exe = std::env::current_exe()?;
        let package = exe
            .parent()
            .and_then(Path::parent)
            .unwrap_or(Path::new("."));
        let bundled: PathBuf = package.join("config/camera-profiles.yaml");
        if bundled.exists() || package.join("manifest.json").exists() {
            Self::load(Some(&bundled))
        } else {
            Self::embedded()
        }
    }
    pub fn find(&self, model: &str) -> Result<&Profile> {
        self.cameras
            .iter()
            .find(|p| p.model == model)
            .with_context(|| format!("unsupported camera {model}; no registered profile"))
    }
}
impl Profile {
    pub fn supports(&self, mode: Mode) -> bool {
        match mode {
            Mode::Four => self.pixel_shift.four.is_some(),
            Mode::Sixteen => self.pixel_shift.sixteen.is_some(),
            Mode::Denoise => self.denoise,
        }
    }
    pub fn cfa_pattern(&self) -> Result<[usize; 4]> {
        ensure!(
            ["RGGB", "BGGR", "GRBG", "GBRG"].contains(&self.cfa.as_str()),
            "unsupported CFA pattern"
        );
        Ok(std::array::from_fn(|i| match self.cfa.as_bytes()[i] {
            b'R' => 0,
            b'G' => 1,
            _ => 2,
        }))
    }
}
impl CameraProfile for Profile {
    fn model(&self) -> &str {
        &self.model
    }
    fn shift(&self, mode: Mode, index: usize) -> (f32, f32) {
        let layout = match mode {
            Mode::Four => self.pixel_shift.four.as_ref(),
            Mode::Sixteen => self.pixel_shift.sixteen.as_ref(),
            Mode::Denoise => return (0., 0.),
        };
        let [x, y] = layout
            .and_then(|l| l.offsets.get(index))
            .copied()
            .unwrap_or([f32::NAN; 2]);
        (x, y)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_ambiguous_or_unsafe_yaml() {
        for yaml in [
            "schema_version: 1\nschema_version: 2\ncameras: []",
            "schema_version: 1\ncameras: []\n---\nschema_version: 1\ncameras: []",
            "schema_version: !custom 1\ncameras: []",
            "1: cameras\nschema_version: 1",
        ] {
            assert!(Catalog::parse(yaml).is_err(), "accepted {yaml}");
        }
        let text = EMBEDDED.replace("denoise: true", "denoise: on");
        assert!(
            Catalog::parse(&text).is_err(),
            "implicit YAML 1.1 booleans must not be accepted"
        );
        let text = EMBEDDED.replace("notes: >-", "notes: !custom >-");
        assert!(Catalog::parse(&text).is_err());
    }
    #[test]
    fn bundled_matches_original_layout() {
        let c = Catalog::embedded().unwrap();
        let p = c.find("ILCE-7CR").unwrap();
        for mode in [Mode::Four, Mode::Sixteen] {
            for i in 0..if mode == Mode::Four { 4 } else { 16 } {
                assert_eq!(p.shift(mode, i), pixelshift_core::SonyA7Cr.shift(mode, i));
            }
        }
    }
    #[test]
    fn rejects_invalid_profiles() {
        let baseline: serde_json::Value = serde_yaml_ng::from_str(EMBEDDED).unwrap();
        for case in 0..7 {
            let mut v = baseline.clone();
            match case {
                0 => v["schema_version"] = 2.into(),
                1 => v["cameras"][0]["typo"] = true.into(),
                2 => {
                    v["cameras"][0]["pixel_shift"]["four"]["offsets"][0] =
                        serde_json::json!([0.5, 0.])
                }
                3 => {
                    v["cameras"][0]["pixel_shift"]["four"]["offsets"][0] = serde_json::json!([1, 1])
                }
                4 => v["cameras"][0]["cfa"] = "XXXX".into(),
                5 => {
                    let p = v["cameras"][0].clone();
                    v["cameras"].as_array_mut().unwrap().push(p);
                }
                _ => v["cameras"][0]["pixel_shift"]["sixteen"]["output_scale"] = 1.into(),
            };
            assert!(
                Catalog::parse(&serde_yaml_ng::to_string(&v).unwrap()).is_err(),
                "case {case}"
            );
        }
    }
}
