//! Header-only grouping. Never infer a burst by consecutive filenames or time.
use anyhow::{Result, ensure};
use pixelshift_io::{Capture, profiles::Catalog};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
};

type GroupKey = (PathBuf, Option<String>, Option<String>, u32);

pub fn group(records: Vec<(PathBuf, Capture)>, catalog: &Catalog) -> Value {
    let mut candidates: BTreeMap<GroupKey, Vec<(PathBuf, Capture)>> = BTreeMap::new();
    let mut ignored = Vec::new();
    for (path, capture) in records {
        if let Some(shift) = &capture.pixel_shift {
            let key = (
                path.parent().unwrap_or(std::path::Path::new("")).to_owned(),
                capture.model.clone(),
                capture.serial.clone(),
                shift.group_id,
            );
            candidates.entry(key).or_default().push((path, capture));
        } else {
            ignored.push(json!({"path":path,"reason":"Not a Pixel Shift source"}));
        }
    }
    let mut groups = Vec::new();
    for ((folder, camera, serial, id), mut records) in candidates {
        records
            .sort_by_key(|(path, c)| (c.pixel_shift.as_ref().unwrap().shot_number, path.clone()));
        let (paths, captures): (Vec<_>, Vec<_>) = records.into_iter().unzip();
        let n = captures[0].pixel_shift.as_ref().unwrap().shot_count;
        let mut seen = HashSet::new();
        let conflict = captures.iter().any(|c| {
            let s = c.pixel_shift.as_ref().unwrap();
            s.shot_count != n
                || s.shot_number == 0
                || s.shot_number > n
                || !seen.insert(s.shot_number)
        });
        let missing: Vec<_> = (1..=n)
            .filter(|i| {
                !captures
                    .iter()
                    .any(|c| c.pixel_shift.as_ref().unwrap().shot_number == *i)
            })
            .collect();
        let supported = camera
            .as_deref()
            .and_then(|m| catalog.find(m).ok())
            .is_some_and(|p| {
                crate::detection::mode(None, usize::from(n)).is_ok_and(|m| p.supports(m))
            });
        let (status, reason) = if conflict {
            (
                "ambiguous",
                "Duplicate shot indices or conflicting sequence metadata".to_owned(),
            )
        } else if !supported {
            ("unsupported", "No supported camera/mode profile".to_owned())
        } else if !missing.is_empty() {
            (
                "incomplete",
                format!("Missing {} of {n} shots", missing.len()),
            )
        } else {
            let mode = crate::detection::mode(None, usize::from(n)).unwrap();
            match crate::detection::validate(&captures, mode, None, catalog) {
                Ok(_) => ("complete", "Complete Pixel Shift sequence".to_owned()),
                Err(e) => ("ambiguous", format!("{e:#}")),
            }
        };
        groups.push(
            json!({"status":status,"reason":reason,"folder":folder,"camera":camera,
            "group_id":id,"shot_count":n,"files":paths,"missing_shots":missing,
            "identity_warning":if serial.as_deref().is_none_or(str::is_empty) {
                Some("Camera serial unavailable; review camera identity before grouping")
            } else {None}}),
        );
    }
    json!({"version":1,"status":"detected","groups":groups,"ignored":ignored,"errors":[]})
}

pub fn scan(paths: Vec<PathBuf>, catalog: &Catalog) -> Result<Value> {
    ensure!(
        !paths.is_empty() && paths.len() <= 100_000,
        "Select 1–100000 source files"
    );
    let mut records = Vec::new();
    let mut errors = Vec::new();
    let mut seen = HashSet::new();
    for path in paths {
        let result = (|| {
            let canonical = path.canonicalize()?;
            ensure!(seen.insert(canonical.clone()), "Duplicate input path");
            let capture = pixelshift_io::inspect_capture(&canonical)?;
            // Preserve caller spelling for Lightroom's path-to-photo lookup,
            // especially Windows canonical paths with the extended-length prefix.
            Ok::<_, anyhow::Error>((path.clone(), capture))
        })();
        match result {
            Ok(record) => records.push(record),
            Err(error) => errors.push(json!({"path":path,"reason":format!("{error:#}")})),
        }
    }
    let mut report = group(records, catalog);
    report["errors"] = json!(errors);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sequence(folder: &str, id: u32, count: u8, serial: &str) -> Vec<(PathBuf, Capture)> {
        (1..=count)
            .rev()
            .map(|shot| {
                (
                    PathBuf::from(format!("{folder}/DSC-{id}-{shot}.ARW")),
                    serde_json::from_value(json!({"model":"ILCE-7CR","dimensions":[9504,6336],
                "SerialNumber":serial,"ISO":100,"ExposureTime":0.01,"FNumber":8,
                "pixel_shift":{"group_id":id,"shot_number":shot,"shot_count":count}}))
                    .unwrap(),
                )
            })
            .collect()
    }
    #[test]
    fn groups_multiple_modes_folders_and_bodies_in_shot_order() {
        let mut records = sequence("a", 10, 4, "one");
        records.extend(sequence("a", 11, 16, "one"));
        records.extend(sequence("b", 10, 4, "one"));
        records.extend(sequence("a", 10, 4, "two"));
        let report = group(records, &Catalog::embedded().unwrap());
        let groups = report["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 4);
        for g in groups {
            assert_eq!(g["status"], "complete");
            assert!(g["files"][0].as_str().unwrap().ends_with("-1.ARW"));
        }
    }
    #[test]
    fn rejects_missing_duplicates_conflicts_unknown_models_and_reused_ids() {
        for case in 0..7 {
            let mut records = sequence("a", 10, 4, "one");
            match case {
                0 => {
                    records.pop();
                }
                1 => records[1].1.pixel_shift.as_mut().unwrap().shot_number = 4,
                2 => records[1].1.pixel_shift.as_mut().unwrap().shot_count = 16,
                3 => records
                    .iter_mut()
                    .for_each(|(_, c)| c.model = Some("Unknown".into())),
                4 => records[1].1.iso = Some(200.0),
                5 => records[1].1.dimensions = Some((10, 10)),
                _ => records.extend(sequence("a", 10, 4, "one")),
            }
            let report = group(records, &Catalog::embedded().unwrap());
            let statuses: Vec<_> = report["groups"]
                .as_array()
                .unwrap()
                .iter()
                .map(|g| g["status"].as_str().unwrap())
                .collect();
            assert!(!statuses.contains(&"complete"), "case {case}");
        }
    }
    #[test]
    fn ordinary_bursts_are_ignored_and_missing_identity_is_visible() {
        let mut records = sequence("a", 10, 4, "one");
        for (_, c) in &mut records {
            c.serial = None;
        }
        let mut ordinary = sequence("a", 11, 4, "one");
        for (_, c) in &mut ordinary {
            c.pixel_shift = None;
        }
        records.extend(ordinary);
        let report = group(records, &Catalog::embedded().unwrap());
        assert_eq!(report["groups"][0]["status"], "complete");
        assert!(report["groups"][0]["identity_warning"].is_string());
        assert_eq!(report["ignored"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn unreadable_and_duplicate_paths_are_reported_without_aborting_scan() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.ARW");
        std::fs::write(&path, b"not TIFF").unwrap();
        let report = scan(
            vec![path.clone(), path, dir.path().join("missing.ARW")],
            &Catalog::embedded().unwrap(),
        )
        .unwrap();
        assert_eq!(report["errors"].as_array().unwrap().len(), 3);
        assert!(
            report["errors"][1]["reason"]
                .as_str()
                .unwrap()
                .contains("Duplicate input")
        );
        assert!(report["groups"].as_array().unwrap().is_empty());
        assert!(scan(Vec::new(), &Catalog::embedded().unwrap()).is_err());
    }
}
