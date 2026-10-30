use serde_json::json;
use std::{fs, process::Command};
fn setup(mode: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let n = if mode == "sixteen" { 16 } else { 4 };
    let mut inputs = Vec::new();
    for i in 1..=n {
        let name = format!("sensor-{i}.json");
        let pixels = vec![0.3f32; 32 * 24];
        let data = json!({"frame":{"width":32,"height":24,"pixels":pixels,"cfa":[0,1,1,2],"model":"ILCE-7CR","sequence":i},"color":{"white_balance":[1,1,1],"xyz_to_camera":[1,0,0,0,1,0,0,0,1]}});
        fs::write(dir.path().join(&name), serde_json::to_vec(&data).unwrap()).unwrap();
        inputs.push(name);
    }
    inputs.reverse(); // CLI must use sequence metadata, not input filename order.
    let job = dir.path().join("job.json");
    fs::write(&job,serde_json::to_vec(&json!({"version":1,"camera":"ILCE-7CR","mode":mode,"inputs":inputs,"output":"result.tif","alignment_radius":0})).unwrap()).unwrap();
    (dir, job)
}
#[test]
fn fixture_to_tagged_rgb16_tiff_and_no_overwrite() {
    for mode in ["four", "sixteen", "denoise"] {
        let (dir, job) = setup(mode);
        let run = || {
            Command::new(env!("CARGO_BIN_EXE_sony-pixelshift"))
                .args(["merge", "--fixtures", "--job"])
                .arg(&job)
                .output()
                .unwrap()
        };
        let out = run();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(report["status"], "complete");
        let bytes = fs::read(dir.path().join("result.tif")).unwrap();
        let mut decoder = tiff::decoder::Decoder::new(std::io::Cursor::new(&bytes)).unwrap();
        let scale = if mode == "sixteen" { 2 } else { 1 };
        assert_eq!(decoder.dimensions().unwrap(), (24 * scale, 16 * scale));
        assert_eq!(decoder.colortype().unwrap(), tiff::ColorType::RGB(16));
        assert!(decoder.get_tag(tiff::tags::Tag::IccProfile).is_ok());
        assert!(matches!(
            decoder.read_image().unwrap(),
            tiff::decoder::DecodingResult::U16(_)
        ));
        assert!(!run().status.success());
        assert_eq!(fs::read(dir.path().join("result.tif")).unwrap(), bytes);
    }
}
#[test]
fn rejects_duplicates_and_unknown_job_versions() {
    let (dir, job) = setup("four");
    let mut spec: serde_json::Value = serde_json::from_slice(&fs::read(&job).unwrap()).unwrap();
    for bad_version in [false, true] {
        if bad_version {
            spec["version"] = json!(2);
        } else {
            spec["inputs"][1] = spec["inputs"][0].clone();
        }
        fs::write(&job, serde_json::to_vec(&spec).unwrap()).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_sony-pixelshift"))
            .args(["merge", "--fixtures", "--job"])
            .arg(&job)
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(!dir.path().join("result.tif").exists());
    }
}

#[test]
fn handshake_and_persistent_failure_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let info = dir.path().join("engine info.json");
    let out = Command::new(env!("CARGO_BIN_EXE_sony-pixelshift"))
        .arg("info")
        .arg("--output")
        .arg(&info)
        .output()
        .unwrap();
    assert!(out.status.success());
    let data: serde_json::Value = serde_json::from_slice(&fs::read(&info).unwrap()).unwrap();
    assert_eq!(data["protocol_version"], 1);
    assert_eq!(data["camera_models"][0], "ILCE-7CR");
    assert_eq!(data["output_formats"], json!(["dng", "tiff"]));
    let result = dir.path().join("failure.json");
    let log = dir.path().join("failure.log");
    let out = Command::new(env!("CARGO_BIN_EXE_sony-pixelshift"))
        .args(["merge", "--job"])
        .arg(dir.path().join("missing-job.json"))
        .arg("--result")
        .arg(&result)
        .arg("--log")
        .arg(&log)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let data: serde_json::Value = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
    assert_eq!(data["status"], "error");
    assert!(data["message"].as_str().unwrap().contains("opening job"));
    assert!(fs::read_to_string(log).unwrap().contains("opening job"));
}
#[test]
fn successful_result_file_matches_stdout() {
    let (dir, job) = setup("four");
    let result = dir.path().join("success.json");
    let log = dir.path().join("success.log");
    fs::write(&log, "Lightroom plugin log header\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_sony-pixelshift"))
        .args(["merge", "--fixtures", "--job"])
        .arg(&job)
        .arg("--result")
        .arg(&result)
        .arg("--log")
        .arg(&log)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let data: serde_json::Value = serde_json::from_slice(&fs::read(result).unwrap()).unwrap();
    assert_eq!(
        data,
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
    );
    let log_text = fs::read_to_string(log).unwrap();
    assert!(log_text.starts_with("Lightroom plugin log header\n"));
    assert!(log_text.contains("complete in"));
}

#[test]
fn denoise_accepts_bursts_without_pixel_shift_indices() {
    let (dir, job) = setup("denoise");
    for i in 1..=4 {
        let path = dir.path().join(format!("sensor-{i}.json"));
        let mut fixture: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        fixture["frame"]["sequence"] = json!(0);
        fs::write(path, serde_json::to_vec(&fixture).unwrap()).unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_sony-pixelshift"))
        .args(["merge", "--fixtures", "--job"])
        .arg(job)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn auto_and_omitted_configuration_detect_both_supported_counts() {
    for mode in ["four", "sixteen"] {
        for omit in [true, false] {
            let (dir, job) = setup(mode);
            let mut spec: serde_json::Value =
                serde_json::from_slice(&fs::read(&job).unwrap()).unwrap();
            if omit {
                spec.as_object_mut().unwrap().remove("camera");
                spec.as_object_mut().unwrap().remove("mode");
            } else {
                spec["camera"] = json!("auto");
                spec["mode"] = json!("auto");
            }
            fs::write(&job, serde_json::to_vec(&spec).unwrap()).unwrap();
            let run = Command::new(env!("CARGO_BIN_EXE_sony-pixelshift"))
                .args(["merge", "--fixtures", "--job"])
                .arg(job)
                .output()
                .unwrap();
            assert!(
                run.status.success(),
                "{}",
                String::from_utf8_lossy(&run.stderr)
            );
            let result: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
            assert_eq!(result["detected"]["camera"], "ILCE-7CR");
            assert_eq!(result["detected"]["mode"], mode);
            assert!(dir.path().join("result.tif").exists());
        }
    }
}

#[test]
fn profile_override_controls_model_capabilities_and_cfa() {
    let baseline: serde_json::Value =
        serde_saphyr::from_str(include_str!("../../../config/camera-profiles.yaml")).unwrap();
    for case in 0..4 {
        let (dir, job) = setup(if case == 0 { "denoise" } else { "four" });
        let mut config = baseline.clone();
        match case {
            0 => config["cameras"][0]["denoise"] = json!(false),
            1 => config["cameras"][0]["cfa"] = json!("GBRG"),
            2 => config["schema_version"] = json!(99),
            _ => {
                config["cameras"][0]["model"] = json!("TEST-CAMERA");
                let mut spec: serde_json::Value =
                    serde_json::from_slice(&fs::read(&job).unwrap()).unwrap();
                spec["camera"] = json!("auto");
                fs::write(&job, serde_json::to_vec(&spec).unwrap()).unwrap();
                for i in 1..=4 {
                    let p = dir.path().join(format!("sensor-{i}.json"));
                    let mut f: serde_json::Value =
                        serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
                    f["frame"]["model"] = json!("TEST-CAMERA");
                    fs::write(p, serde_json::to_vec(&f).unwrap()).unwrap();
                }
            }
        }
        let path = dir.path().join("profiles.yaml");
        fs::write(&path, serde_saphyr::to_string(&config).unwrap()).unwrap();
        let run = Command::new(env!("CARGO_BIN_EXE_sony-pixelshift"))
            .arg("--camera-profiles")
            .arg(path)
            .args(["merge", "--fixtures", "--job"])
            .arg(job)
            .output()
            .unwrap();
        if case == 3 {
            assert!(
                run.status.success(),
                "{}",
                String::from_utf8_lossy(&run.stderr)
            );
            let result: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
            assert_eq!(result["detected"]["camera"], "TEST-CAMERA");
        } else {
            assert!(!run.status.success(), "case {case}");
            assert!(!dir.path().join("result.tif").exists());
        }
    }
}

#[test]
fn all_merge_modes_write_dng_and_reject_overwrite() {
    for mode in ["four", "sixteen", "denoise"] {
        let (dir, job) = setup(mode);
        let mut spec: serde_json::Value = serde_json::from_slice(&fs::read(&job).unwrap()).unwrap();
        spec["output"] = json!("merged.dng");
        fs::write(&job, serde_json::to_vec(&spec).unwrap()).unwrap();
        let run = || {
            Command::new(env!("CARGO_BIN_EXE_sony-pixelshift"))
                .args(["merge", "--fixtures", "--job"])
                .arg(&job)
                .output()
                .unwrap()
        };
        let result = run();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(
            report["output"],
            dir.path().join("merged.dng").to_str().unwrap()
        );
        let before = fs::read(dir.path().join("merged.dng")).unwrap();
        assert!(before.starts_with(b"II*\0") || before.starts_with(b"MM\0*"));
        assert!(!run().status.success());
        assert_eq!(fs::read(dir.path().join("merged.dng")).unwrap(), before);
        assert!(!dir.path().join("result.tif").exists());
    }
}

#[test]
fn streamed_execution_reports_bounds_resources_and_validates_options() {
    let (dir, job) = setup("sixteen");
    let mut spec: serde_json::Value = serde_json::from_slice(&fs::read(&job).unwrap()).unwrap();
    spec["strip_rows"] = json!(7);
    spec["compression"] = json!("none");
    spec["output"] = json!("streamed.dng");
    fs::write(&job, serde_json::to_vec(&spec).unwrap()).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_sony-pixelshift"))
        .args(["merge", "--fixtures", "--job"])
        .arg(&job)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let record: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let execution = &record["execution"];
    assert_eq!(execution["streamed_output"], true);
    assert_eq!(execution["source_storage"], "float32_ram");
    assert_eq!(execution["peak_output_region_bytes"], 48 * 7 * 3 * 4);
    assert_eq!(execution["timings"]["output"]["regions"], 5);
    assert!(
        execution["resources"]["peak_resident_bytes"]
            .as_u64()
            .unwrap()
            > 0
    );
    for (compression, rows) in [("lossy", 7), ("lossless", 0), ("lossless", 257)] {
        spec["output"] = json!("invalid.dng");
        spec["compression"] = json!(compression);
        spec["strip_rows"] = json!(rows);
        fs::write(&job, serde_json::to_vec(&spec).unwrap()).unwrap();
        let failed = Command::new(env!("CARGO_BIN_EXE_sony-pixelshift"))
            .args(["merge", "--fixtures", "--job"])
            .arg(&job)
            .output()
            .unwrap();
        assert!(!failed.status.success());
        assert!(!dir.path().join("invalid.dng").exists());
    }
    assert!(
        !Command::new(env!("CARGO_BIN_EXE_sony-pixelshift"))
            .args(["merge", "--reference", "--job"])
            .arg(&job)
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn disk_sources_are_lossless_and_release_scratch_on_success_and_failure() {
    let (dir, job) = setup("sixteen");
    let scratch = dir.path().join("scratch space");
    fs::create_dir(&scratch).unwrap();
    let mut spec: serde_json::Value = serde_json::from_slice(&fs::read(&job).unwrap()).unwrap();
    let mut outputs = Vec::new();
    for storage in ["ram", "disk"] {
        spec["source_storage"] = json!(storage);
        spec["scratch_directory"] = json!("scratch space");
        spec["strip_rows"] = json!(3);
        spec["output"] = json!(format!("{storage}.dng"));
        fs::write(&job, serde_json::to_vec(&spec).unwrap()).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_sony-pixelshift"))
            .args(["merge", "--fixtures", "--job"])
            .arg(&job)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let record: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        if storage == "disk" {
            assert_eq!(record["execution"]["source_buffer_bytes"], 0);
            assert!(record["execution"]["scratch_bytes"].as_u64().unwrap() > 0);
        }
        outputs.push(fs::read(dir.path().join(format!("{storage}.dng"))).unwrap());
        assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);
    }
    assert_eq!(outputs[0], outputs[1]);
    // Alignment validation fails only after all stores have been ingested.
    spec["alignment_radius"] = json!(9);
    spec["output"] = json!("failure.dng");
    fs::write(&job, serde_json::to_vec(&spec).unwrap()).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_sony-pixelshift"))
        .args(["merge", "--fixtures", "--job"])
        .arg(&job)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(!dir.path().join("failure.dng").exists());
    assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);
    spec["scratch_directory"] = json!("missing scratch");
    fs::write(&job, serde_json::to_vec(&spec).unwrap()).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_sony-pixelshift"))
        .args(["merge", "--fixtures", "--job"])
        .arg(&job)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("scratch_directory must be an existing directory")
    );
    spec["source_storage"] = json!("auto");
    fs::write(&job, serde_json::to_vec(&spec).unwrap()).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_sony-pixelshift"))
        .args(["merge", "--fixtures", "--job"])
        .arg(&job)
        .output()
        .unwrap();
    assert!(!out.status.success());
}
