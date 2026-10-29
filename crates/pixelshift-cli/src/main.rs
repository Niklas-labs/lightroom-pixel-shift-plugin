use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use pixelshift_core::{CameraProfile, Mode, Options, SonyA7Cr, merge};
use pixelshift_io::{OutputWriter, RawDecoder};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(
    version,
    about = "Experimental Sony a7CR sensor-domain Pixel Shift merger"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}
#[derive(Subcommand)]
enum Commands {
    /// Merge a versioned JSON job. Paths are relative to the job file.
    Merge {
        #[arg(long)]
        job: PathBuf,
        /// Enable synthetic JSON sensor input (never automatic).
        #[arg(long)]
        fixtures: bool,
    },
    /// List registered camera profiles and validation status.
    Cameras,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Job {
    version: u32,
    camera: String,
    mode: Mode,
    inputs: Vec<PathBuf>,
    output: PathBuf,
    #[serde(default = "threshold")]
    motion_threshold: f32,
    #[serde(default = "radius")]
    alignment_radius: i32,
}
fn threshold() -> f32 {
    0.08
}
fn radius() -> i32 {
    2
}
fn resolve(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    }
}
fn run() -> Result<()> {
    match Cli::parse().command {
        Commands::Cameras => println!(
            "ILCE-7CR: four / sixteen / denoise (experimental; real-capture validation pending)"
        ),
        Commands::Merge { job, fixtures } => {
            let spec: Job = serde_json::from_reader(fs::File::open(&job).context("opening job")?)
                .context("invalid job JSON")?;
            ensure!(spec.version == 1, "unsupported job version");
            let profile = SonyA7Cr;
            ensure!(
                spec.camera == profile.model(),
                "unregistered camera profile"
            );
            let required = match spec.mode {
                Mode::Four => 4,
                Mode::Sixteen => 16,
                Mode::Denoise => spec.inputs.len(),
            };
            ensure!(
                spec.inputs.len() == required && (2..=64).contains(&required),
                "incorrect input count"
            );
            let base = job.parent().unwrap_or(Path::new("."));
            let output = resolve(base, &spec.output);
            ensure!(!output.exists(), "output already exists");
            let mut seen = std::collections::HashSet::new();
            let mut decoded = Vec::new();
            for (i, input) in spec.inputs.iter().enumerate() {
                let path = resolve(base, input)
                    .canonicalize()
                    .with_context(|| format!("input {}", input.display()))?;
                ensure!(seen.insert(path.clone()), "duplicate input file");
                eprintln!("decode {}/{}: {}", i + 1, required, path.display());
                let frame = if fixtures {
                    pixelshift_io::decode(&path, true)?
                } else {
                    pixelshift_io::SonyArwDecoder.decode(&path)?
                };
                if let Some(first) = decoded.first() {
                    ensure!(
                        frame.compatible(first),
                        "mixed exposure, camera serial, or input type"
                    );
                }
                decoded.push(frame);
            }
            decoded.sort_by_key(|d| d.frame.sequence);
            let color = decoded[0].color.clone();
            let frames = decoded.into_iter().map(|d| d.frame).collect::<Vec<_>>();
            let (image, report) = merge(
                &frames,
                &profile,
                &Options {
                    mode: spec.mode,
                    motion_threshold: spec.motion_threshold,
                    alignment_radius: spec.alignment_radius,
                },
            )?;
            eprintln!("writing {}x{} TIFF", image.width, image.height);
            pixelshift_io::TiffWriter.write(&output, &image, &color)?;
            println!(
                "{}",
                serde_json::json!({"version":1,"status":"complete","output":output,"report":report})
            );
        }
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!(
            "{}",
            serde_json::json!({"version":1,"status":"error","message":format!("{error:#}")})
        );
        std::process::exit(1);
    }
}
