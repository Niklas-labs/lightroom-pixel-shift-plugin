use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use pixelshift_core::{Mode, Options, merge};
use pixelshift_io::{OutputWriter, RawDecoder, profiles::Catalog};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};

mod detection;

const PROTOCOL_VERSION: u32 = 1;
#[derive(Parser)]
#[command(
    version,
    about = "Experimental Sony a7CR sensor-domain Pixel Shift merger"
)]
struct Cli {
    /// Override the bundled camera capability catalog (advanced/development use).
    #[arg(long, global = true)]
    camera_profiles: Option<PathBuf>,
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
        /// Write a machine-readable completion/error record to a new file.
        #[arg(long)]
        result: Option<PathBuf>,
        /// Append diagnostics to the Lightroom launcher log.
        #[arg(long)]
        log: Option<PathBuf>,
    },
    /// Machine-readable engine capabilities and plugin protocol handshake.
    Info {
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Inspect Sony capture metadata without decoding sensor pixels.
    Inspect {
        input: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// List registered camera profiles and validation status.
    Cameras,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Job {
    version: u32,
    #[serde(default)]
    camera: Option<String>,
    #[serde(default)]
    mode: Option<String>,
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
#[derive(Default)]
struct Diagnostics {
    file: Option<File>,
}
impl Diagnostics {
    fn record(&mut self, message: impl AsRef<str>) -> Result<()> {
        eprintln!("{}", message.as_ref());
        if let Some(file) = self.file.as_mut() {
            writeln!(file, "{}", message.as_ref()).context("writing diagnostic log")?;
            file.flush()?;
        }
        Ok(())
    }
}
fn new_file(path: &Path) -> Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| {
            format!(
                "creating {} (existing files are never overwritten)",
                path.display()
            )
        })
}
fn execute(cli: &Cli, diagnostics: &mut Diagnostics) -> Result<Value> {
    let catalog = Catalog::load(cli.camera_profiles.as_deref())?;
    let models: Vec<_> = catalog.cameras.iter().map(|p| p.model.as_str()).collect();
    match &cli.command {
        Commands::Info { .. } => Ok(json!({
            "protocol_version": PROTOCOL_VERSION,
            "engine_version": env!("CARGO_PKG_VERSION"),
            "camera_models": models,
            "camera_names": catalog.cameras.iter().map(|p|p.name.as_str()).collect::<Vec<_>>(),
            "camera_profile_schema_version": catalog.schema_version,
            "metadata_backend": pixelshift_io::metadata_backend(),
            "output_formats": ["dng", "tiff"],
            "experimental": true,
            "autodetection": true,
        })),
        Commands::Inspect { input, .. } => Ok(json!({
            "version": PROTOCOL_VERSION, "status": "inspected",
            "metadata_backend": pixelshift_io::metadata_backend(),
            "capture": pixelshift_io::inspect_capture(input)?,
        })),
        Commands::Cameras => {
            Ok(json!({"camera_models":models,"profiles":catalog.cameras,"experimental":true}))
        }
        Commands::Merge {
            job, fixtures, log, ..
        } => {
            if let Some(path) = log {
                diagnostics.file = Some(
                    OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .with_context(|| format!("opening diagnostic log {}", path.display()))?,
                );
            }
            diagnostics.record(format!(
                "sony-pixelshift {} / protocol {}",
                env!("CARGO_PKG_VERSION"),
                PROTOCOL_VERSION
            ))?;
            let started = Instant::now();
            let spec: Job = serde_json::from_reader(fs::File::open(job).context("opening job")?)
                .context("invalid job JSON")?;
            ensure!(spec.version == PROTOCOL_VERSION, "unsupported job version");
            let mode = detection::mode(spec.mode.as_deref(), spec.inputs.len())?;
            let required = spec.inputs.len();
            let base = job.parent().unwrap_or(Path::new("."));
            let output = resolve(base, &spec.output);
            let format = output
                .extension()
                .and_then(|v| v.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            ensure!(
                matches!(format.as_str(), "dng" | "tif" | "tiff"),
                "output must be DNG or TIFF"
            );
            ensure!(!output.exists(), "output already exists");
            let mut seen = std::collections::HashSet::new();
            let mut paths = Vec::new();
            for input in &spec.inputs {
                let path = resolve(base, input)
                    .canonicalize()
                    .with_context(|| format!("input {}", input.display()))?;
                ensure!(seen.insert(path.clone()), "duplicate input file");
                paths.push(path);
            }
            let camera = if *fixtures {
                if let Some(camera) = spec.camera.as_deref().filter(|c| *c != "auto") {
                    camera.to_owned()
                } else {
                    pixelshift_io::decode(&paths[0], true)?.frame.model
                }
            } else {
                detection::preflight(&mut paths, mode, spec.camera.as_deref(), &catalog)?
            };
            let profile = catalog.find(&camera)?;
            ensure!(
                profile.supports(mode),
                "camera profile does not support selected mode"
            );
            diagnostics.record(format!("detected {camera}, {mode:?}, {required} sources"))?;
            let mut decoded = Vec::new();
            for (i, path) in paths.iter().enumerate() {
                diagnostics.record(format!("decode {}/{}: {}", i + 1, required, path.display()))?;
                let frame = if *fixtures {
                    pixelshift_io::decode(path, true)?
                } else {
                    pixelshift_io::SonyArwDecoder.decode(path)?
                };
                ensure!(
                    frame.frame.cfa == profile.cfa_pattern()?,
                    "RAW CFA does not match camera profile"
                );
                if mode != Mode::Denoise && !fixtures {
                    let capture = frame
                        .capture_metadata()
                        .context("missing capture metadata")?;
                    let shift = capture.pixel_shift.as_ref().context(
                        "missing Sony PixelShiftInfo: select original Pixel Shift source ARWs; ordinary bursts use the denoise menu"
                    )?;
                    ensure!(
                        usize::from(shift.shot_count) == required,
                        "selected mode expects {required} shots, but Sony metadata declares {}",
                        shift.shot_count
                    );
                }
                if let Some(first) = decoded.first() {
                    ensure!(
                        frame.compatible(first),
                        "mixed exposure, camera serial, or input type"
                    );
                }
                decoded.push(frame);
            }
            decoded.sort_by_key(|d| d.frame.sequence);
            if mode == Mode::Denoise {
                // Burst indices may start anywhere or be absent. Pixel Shift
                // modes retain strict source-shot completeness validation.
                for (index, decoded) in decoded.iter_mut().enumerate() {
                    decoded.frame.sequence = index + 1;
                }
            }
            let mut color = decoded[0].color.clone();
            color
                .photo_metadata
                .model
                .get_or_insert_with(|| decoded[0].frame.model.clone());
            color
                .photo_metadata
                .make
                .get_or_insert_with(|| "SONY".into());
            let frames = decoded.into_iter().map(|d| d.frame).collect::<Vec<_>>();
            diagnostics.record("merge and motion correction")?;
            let (image, report) = merge(
                &frames,
                profile,
                &Options {
                    mode,
                    motion_threshold: spec.motion_threshold,
                    alignment_radius: spec.alignment_radius,
                },
            )?;
            drop(frames); // Release source buffers before writing the result.
            diagnostics.record(format!(
                "writing {}x{} {}",
                image.width,
                image.height,
                format.to_uppercase()
            ))?;
            if format == "dng" {
                pixelshift_io::DngWriter.write(&output, &image, &color)?;
            } else {
                pixelshift_io::TiffWriter.write(&output, &image, &color)?;
            }
            diagnostics.record(format!(
                "complete in {:.2}s; {} pixels used reference fallback",
                started.elapsed().as_secs_f64(),
                report.fallback_pixels
            ))?;
            Ok(
                json!({"version":PROTOCOL_VERSION,"status":"complete","output":output,"report":report,"detected":{"camera":camera,"mode":mode,"source_count":required}}),
            )
        }
    }
}
fn run(cli: Cli) -> i32 {
    let result_path = match &cli.command {
        Commands::Info { output } | Commands::Inspect { output, .. } => output.as_ref(),
        Commands::Merge { result, .. } => result.as_ref(),
        Commands::Cameras => None,
    };
    // Reserve the result before processing so an existing result can never cause
    // a successful merge to go unreported. Lightroom polls only after process exit.
    let mut result_file = match result_path.map(|p| new_file(p)).transpose() {
        Ok(file) => file,
        Err(error) => {
            eprintln!("{error:#}");
            return 1;
        }
    };
    let mut diagnostics = Diagnostics::default();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        execute(&cli, &mut diagnostics)
    }))
    .unwrap_or_else(|_| {
        Err(anyhow::anyhow!(
            "engine panicked while processing; inspect the log and input files"
        ))
    });
    let (record, status) = match outcome {
        Ok(record) => (record, 0),
        Err(error) => {
            let message = format!("{error:#}");
            let _ = diagnostics.record(&message);
            (
                json!({"version":PROTOCOL_VERSION,"status":"error","message":message}),
                1,
            )
        }
    };
    if let Some(file) = result_file.as_mut()
        && let Err(error) = serde_json::to_writer(&mut *file, &record)
            .map_err(anyhow::Error::from)
            .and_then(|_| file.flush().map_err(Into::into))
    {
        eprintln!("writing result: {error:#}");
        return 1;
    }
    if status == 0 {
        println!("{record}");
    } else {
        eprintln!("{record}");
    }
    status
}
fn main() {
    std::process::exit(run(Cli::parse()));
}
