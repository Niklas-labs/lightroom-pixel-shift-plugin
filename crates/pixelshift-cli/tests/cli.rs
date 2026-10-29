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
