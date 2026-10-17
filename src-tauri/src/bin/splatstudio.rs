use std::{io::Write, path::PathBuf, sync::Mutex};

use clap::{Parser, Subcommand};
use ooo_splat::{
    engines::{ffmpeg::extract_uniform_frames, ffprobe::probe_video},
    error::{Result, SplatError},
    pipeline::runner::{default_engine_paths, PipelineRunner},
    presets::Quality,
    process::ProcessManager,
    video::{
        analyze_image_sequence, create_image_plan, prepare_image_sequence, FrameSelectionStrategy,
        UniformRatioFrameSelection,
    },
};

#[derive(Debug, Parser)]
#[command(name = "splatstudio", version, about = "OOOSplat local pipeline CLI")]
struct Cli {
    /// Override the bundled engine directory (also supports OOOSPLAT_ENGINE_DIR).
    #[arg(long, global = true)]
    engine_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Validate FFmpeg, FFprobe, CPU COLMAP and Brush.
    Health,
    /// Read video metadata or image-sequence information.
    Probe { input: PathBuf },
    /// Show the video frame plan or image-sequence plan.
    Plan {
        input: PathBuf,
        #[arg(long, value_enum, default_value_t = Quality::Balanced)]
        quality: Quality,
    },
    /// Prepare video frames or an image sequence and optional COLMAP masks.
    Extract {
        input: PathBuf,
        output: PathBuf,
        #[arg(long, value_enum, default_value_t = Quality::Balanced)]
        quality: Quality,
    },
    /// Run the end-to-end pipeline after all fixed engine CLIs are verified.
    Generate {
        input: PathBuf,
        /// Output directory (defaults to ./projects).
        #[arg(long)]
        projects_root: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = Quality::Balanced)]
        quality: Quality,
        /// Disable automatic frame/resolution/training optimization.
        #[arg(long)]
        no_optimize: bool,
        /// Persist full progress events as newline-delimited JSON.
        #[arg(long)]
        events: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
    if let Err(error) = execute(Cli::parse()).await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn execute(cli: Cli) -> Result<()> {
    let engines = default_engine_paths(cli.engine_dir);
    match cli.command {
        Commands::Health => {
            let status = engines.check_all().await;
            println!("{}", serde_json::to_string_pretty(&status)?);
            if status.iter().any(|engine| !engine.can_start) {
                return Err(SplatError::Process("引擎检查失败，请查看上面的详情".into()));
            }
        }
        Commands::Probe { input } => {
            if input.is_dir() {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&analyze_image_sequence(&input)?)?
                );
            } else {
                let video =
                    probe_video(&engines.ffprobe, &input, None, &ProcessManager::new()).await?;
                println!("{}", serde_json::to_string_pretty(&video)?);
            }
        }
        Commands::Plan { input, quality } => {
            let plan = if input.is_dir() {
                create_image_plan(&analyze_image_sequence(&input)?, &quality.preset())
            } else {
                let video =
                    probe_video(&engines.ffprobe, &input, None, &ProcessManager::new()).await?;
                UniformRatioFrameSelection.create_plan(&video, &quality.preset())
            };
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
        Commands::Extract {
            input,
            output,
            quality,
        } => {
            let masks = output
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("masks");
            let colmap_frames = output
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("colmap_frames");
            if input.is_dir() {
                let extraction = prepare_image_sequence(&input, &output, &colmap_frames, &masks)?;
                println!(
                    "prepared {} images in {} and {} masks in {}",
                    extraction.image_count,
                    output.display(),
                    extraction.mask_count,
                    masks.display()
                );
                return Ok(());
            }
            ensure_engine(&engines.ffprobe)?;
            ensure_engine(&engines.ffmpeg)?;
            let video = probe_video(&engines.ffprobe, &input, None, &ProcessManager::new()).await?;
            let plan = UniformRatioFrameSelection.create_plan(&video, &quality.preset());
            let extraction = extract_uniform_frames(
                &engines.ffmpeg,
                &input,
                &output,
                &colmap_frames,
                &masks,
                &plan,
                video.has_alpha,
                None,
                None,
                &ProcessManager::new(),
                None,
            )
            .await?;
            if extraction.has_alpha {
                println!(
                    "extracted {} RGBA frames to {} and {} masks to {}",
                    extraction.frame_count,
                    output.display(),
                    extraction.mask_count,
                    masks.display()
                );
            } else {
                println!(
                    "extracted {} frames to {}",
                    extraction.frame_count,
                    output.display()
                );
            }
        }
        Commands::Generate {
            input,
            projects_root,
            quality,
            no_optimize,
            events,
        } => {
            let event_file = events
                .map(|path| {
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                })
                .transpose()?
                .map(Mutex::new);
            let runner = PipelineRunner::new_with_planner(engines, !no_optimize, move |event| {
                if let Some(file) = &event_file {
                    let mut file = file.lock().unwrap_or_else(|e| e.into_inner());
                    if let Err(error) = serde_json::to_writer(&mut *file, &event)
                        .map_err(std::io::Error::other)
                        .and_then(|_| file.write_all(b"\n"))
                        .and_then(|_| file.flush())
                    {
                        eprintln!("progress write failed: {error}");
                    }
                }
                eprintln!(
                    "{:>6.2}% {:?}: {}",
                    event.progress, event.stage, event.message
                );
            });
            let root = projects_root.unwrap_or_else(|| PathBuf::from("projects"));
            let run = runner.generate_for_diagnostics(&input, quality, &root);
            tokio::pin!(run);
            let result = tokio::select! {
                result = &mut run => result,
                _ = shutdown_signal() => {
                    runner.cancel();
                    run.await
                }
            }?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }
    Ok(())
}

fn ensure_engine(path: &std::path::Path) -> Result<()> {
    if path.is_file() {
        Ok(())
    } else {
        Err(SplatError::EngineMissing(path.display().to_string()))
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
