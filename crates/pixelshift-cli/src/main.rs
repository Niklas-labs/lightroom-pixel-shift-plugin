use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use pixelshift_core::{
    CameraProfile, ImageSource, Mode, Options, PreparedMerge, SensorSource, merge,
};
use pixelshift_io::{
    OutputCompression, RawDecoder, SourceStorage, WriteOptions, profiles::Catalog,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};

mod detection;
mod metrics;

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
        /// Development numerical reference; deliberately requires synthetic input.
        #[arg(long, requires = "fixtures")]
        reference: bool,
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
    /// Inspect Sony capture metadata; optionally decode one sensor for diagnostics.
    Inspect {
        input: PathBuf,
        /// Decode one RAW and return a small native/normalized sensor sample grid.
        #[arg(long)]
        sensor_samples: bool,
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
    #[serde(default)]
    compression: OutputCompression,
    #[serde(default = "strip_rows")]
    strip_rows: usize,
    #[serde(default)]
    source_storage: SourceStorage,
    /// Existing scratch directory; paths are relative to the job. OS temp by default.
    #[serde(default)]
    scratch_directory: Option<PathBuf>,
}
fn strip_rows() -> usize {
    32
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
            "output_compressions":["lossless","none"],
            "streamed_output":true,
            "source_storage":"packed_u16_ram",
            "source_storages":["ram","disk"],
            "experimental": true,
            "autodetection": true,
            "sensor_sample_diagnostics": true,
        })),
        Commands::Inspect {
            input,
            sensor_samples,
            ..
        } => {
            if *sensor_samples {
                // Deliberately omit private capture metadata from this shareable record.
                Ok(json!({"version": PROTOCOL_VERSION, "status": "sampled",
                    "sensor": pixelshift_io::inspect_sensor_samples(input, &catalog)?}))
            } else {
                Ok(json!({"version": PROTOCOL_VERSION, "status": "inspected",
                    "metadata_backend": pixelshift_io::metadata_backend(),
                    "capture": pixelshift_io::inspect_capture(input)?}))
            }
        }
        Commands::Cameras => {
            Ok(json!({"camera_models":models,"profiles":catalog.cameras,"experimental":true}))
        }
        Commands::Merge {
            job,
            fixtures,
            reference,
            log,
            ..
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
            let spec: Job = serde_json::from_reader(std::io::BufReader::new(
                fs::File::open(job).context("opening job")?,
            ))
            .context("invalid job JSON")?;
            ensure!(spec.version == PROTOCOL_VERSION, "unsupported job version");
            let write_options = WriteOptions {
                compression: spec.compression,
                strip_rows: spec.strip_rows,
            };
            write_options.validate()?;
            let mode = detection::mode(spec.mode.as_deref(), spec.inputs.len())?;
            let required = spec.inputs.len();
            let base = job.parent().unwrap_or(Path::new("."));
            let scratch = spec
                .scratch_directory
                .as_ref()
                .map(|path| resolve(base, path));
            if let Some(path) = &scratch {
                ensure!(
                    path.is_dir(),
                    "scratch_directory must be an existing directory"
                );
            }
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
            let preflight_seconds = started.elapsed().as_secs_f64();
            let decode_start = Instant::now();
            diagnostics.record(format!(
                "source storage: {:?}; scratch directory: {}",
                spec.source_storage,
                scratch
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "OS temporary directory".into())
            ))?;
            let mut decoded = Vec::new();
            let mut reference_frames = Vec::new();
            let mut sensor_stores = Vec::new();
            for (i, path) in paths.iter().enumerate() {
                diagnostics.record(format!("decode {}/{}: {}", i + 1, required, path.display()))?;
                let (frame, store) = if *reference {
                    let (metadata, frame) = pixelshift_io::decode(path, true)?.into_parts();
                    reference_frames.push(frame);
                    (metadata, None)
                } else {
                    let (metadata, mut store) = if *fixtures {
                        pixelshift_io::decode_stored(
                            path,
                            true,
                            spec.source_storage,
                            scratch.as_deref(),
                        )?
                    } else {
                        pixelshift_io::SonyArwDecoder.decode_to_store(
                            path,
                            spec.source_storage,
                            scratch.as_deref(),
                        )?
                    };
                    if !*fixtures && let Some(reference) = &profile.raw_normalization {
                        store.apply_reference_normalization(reference)?;
                        diagnostics.record(format!("RAW normalization: decoder white {:?}; reference white {:?}; black {:?}",
                            store.decoder_white_levels(), store.normalization_levels().1, store.normalization_levels().0))?;
                    }
                    (metadata, Some(store))
                };
                ensure!(
                    frame.info.cfa == profile.cfa_pattern()?,
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
                sensor_stores.push(store);
            }
            let decode_seconds = decode_start.elapsed().as_secs_f64();
            let mut paired: Vec<_> = decoded.into_iter().zip(sensor_stores).collect();
            paired.sort_by_key(|(d, _)| d.info.sequence);
            let (mut decoded, mut sensor_stores): (Vec<_>, Vec<_>) = paired.into_iter().unzip();
            if mode == Mode::Denoise {
                // Burst indices may start anywhere or be absent. Pixel Shift
                // modes retain strict source-shot completeness validation.
                for (index, decoded) in decoded.iter_mut().enumerate() {
                    decoded.info.sequence = index + 1;
                    if let Some(store) = &mut sensor_stores[index] {
                        store.set_sequence(index + 1);
                    }
                }
            }
            let source_crop = decoded[0].source_crop();
            let sensor_size = (decoded[0].info.width, decoded[0].info.height);
            diagnostics.record(format!(
                "reference sensor: {}x{}; preferred crop: {:?}",
                sensor_size.0, sensor_size.1, source_crop
            ))?;
            let mut color = decoded[0].color.clone();
            color.dng_rendering = profile.dng_rendering.clone();
            color
                .photo_metadata
                .model
                .get_or_insert_with(|| decoded[0].info.model.clone());
            color
                .photo_metadata
                .make
                .get_or_insert_with(|| "SONY".into());
            diagnostics.record(format!(
                "color calibration: {}; as-shot WB gains: {:?}; DNG baseline exposure: {} EV",
                if color.xyz_to_camera_a.is_some() {
                    "dual illuminant A + D65"
                } else {
                    "single illuminant D65"
                },
                color.white_balance,
                color
                    .dng_rendering
                    .as_ref()
                    .map_or(0., |r| r.baseline_exposure_ev)
            ))?;
            if let Some(rendering) = &color.dng_rendering {
                diagnostics.record(format!("DNG rendering reference: {}", rendering.source))?;
            }
            reference_frames.sort_by_key(|f| f.sequence);
            if mode == Mode::Denoise {
                for (i, frame) in reference_frames.iter_mut().enumerate() {
                    frame.sequence = i + 1;
                }
            }
            let frames = reference_frames;
            drop(decoded);
            let source_buffer_bytes = sensor_stores
                .iter()
                .flatten()
                .map(|s| s.resident_bytes())
                .sum::<u64>()
                + frames
                    .iter()
                    .map(|f| f.pixels.capacity() as u64 * 4)
                    .sum::<u64>();
            let scratch_bytes = sensor_stores
                .iter()
                .flatten()
                .map(|s| s.scratch_bytes())
                .sum::<u64>();
            let stores: Vec<Box<dyn SensorSource>> = sensor_stores
                .into_iter()
                .flatten()
                .map(|s| Box::new(s) as Box<dyn SensorSource>)
                .collect();
            diagnostics.record("merge and motion correction")?;
            let options = Options {
                mode,
                motion_threshold: spec.motion_threshold,
                alignment_radius: spec.alignment_radius,
            };
            let plan_start = Instant::now();
            let prepared = if *reference {
                PreparedMerge::new(&frames, profile, &options)?
            } else {
                PreparedMerge::from_sources(&stores, profile, &options)?
            };
            let validation_alignment_seconds = plan_start.elapsed().as_secs_f64();
            let geometry = prepared.report(0);
            let (width, height) = prepared.dimensions();
            if let Some(crop) = source_crop {
                color.default_crop = Some(pixelshift_io::map_reference_crop(
                    crop,
                    sensor_size,
                    geometry.sensor_origin,
                    profile.shift(mode, 0),
                    if mode == Mode::Sixteen { 2 } else { 1 },
                    (width, height),
                )?);
            }
            // Whole-image execution is a development oracle, not a memory mode.
            let reference_start = Instant::now();
            let reference_result = if *reference {
                Some(merge(&frames, profile, &options)?)
            } else {
                None
            };
            let reference_seconds = if *reference {
                reference_start.elapsed().as_secs_f64()
            } else {
                0.0
            };
            let source: &dyn ImageSource = match &reference_result {
                Some((image, _)) => image,
                None => &prepared,
            };
            diagnostics.record(format!(
                "writing {}x{} {}",
                width,
                height,
                format.to_uppercase()
            ))?;
            diagnostics.record(format!(
                "execution: {}; source buffers: {} bytes; compression: {:?}; strip rows: {}",
                if *reference {
                    "whole-image fixture reference"
                } else {
                    "streamed output"
                },
                source_buffer_bytes,
                spec.compression,
                spec.strip_rows
            ))?;
            let output_stats = {
                let mut progress =
                    |done, total| diagnostics.record(format!("output region {done}/{total}"));
                if format == "dng" {
                    pixelshift_io::write_dng_from_source(
                        &output,
                        source,
                        &color,
                        write_options,
                        &mut progress,
                    )?
                } else {
                    pixelshift_io::write_tiff_from_source(
                        &output,
                        source,
                        &color,
                        write_options,
                        &mut progress,
                    )?
                }
            };
            let report = match &reference_result {
                Some((_, report)) => prepared.report(report.fallback_pixels),
                None => prepared.report(output_stats.fallback_pixels),
            };
            let peak_source_region_bytes = prepared.peak_source_region_bytes();
            drop(reference_result);
            drop(prepared);
            drop(frames);
            drop(stores);
            let resources = metrics::snapshot();
            diagnostics.record(format!(
                "alignment residuals (sensor pixels): {:?}",
                report.residual_shifts
            ))?;
            diagnostics.record(format!(
                "motion reference fallback: {} / {} ({:.3}%); DNG default crop: {:?}",
                report.fallback_pixels,
                width * height,
                report.fallback_pixels as f64 / (width * height) as f64 * 100.0,
                color.default_crop
            ))?;
            let storage_name = if *reference {
                "normalized_f32_ram"
            } else {
                match (*fixtures, spec.source_storage) {
                    (true, SourceStorage::Ram) => "float32_ram",
                    (true, SourceStorage::Disk) => "float32_disk",
                    (false, SourceStorage::Ram) => "packed_u16_ram",
                    (false, SourceStorage::Disk) => "packed_u16_disk",
                }
            };
            let execution = json!({"streamed_output":!reference,"source_storage":storage_name,"scratch_bytes":scratch_bytes,"peak_source_region_bytes":peak_source_region_bytes,
                "source_buffer_bytes":source_buffer_bytes,"peak_output_region_bytes":output_stats.peak_region_pixels*3*4,
                "compression":spec.compression,"strip_rows":spec.strip_rows,
                "timings":{"preflight_seconds":preflight_seconds,"decode_seconds":decode_seconds,
                    "validation_alignment_seconds":validation_alignment_seconds,"reference_seconds":reference_seconds,
                    "output":output_stats,"total_seconds":started.elapsed().as_secs_f64()},"resources":resources});
            diagnostics.record(format!(
                "performance: {}",
                serde_json::to_string(&execution)?
            ))?;
            diagnostics.record(format!(
                "complete in {:.2}s; {} pixels used reference fallback",
                started.elapsed().as_secs_f64(),
                report.fallback_pixels
            ))?;
            Ok(
                json!({"version":PROTOCOL_VERSION,"status":"complete","output":output,"report":report,"execution":execution,"detected":{"camera":camera,"mode":mode,"source_count":required}}),
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
