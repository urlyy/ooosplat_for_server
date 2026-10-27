use std::{io::Write, path::PathBuf, sync::Mutex};

use clap::{Parser, Subcommand};
use ooo_splat::{
    engines::health::check_colmap_acceleration_cached,
    engines::{ffmpeg::extract_selected_frames, ffprobe::probe_video},
    error::{Result, SplatError},
    pipeline::estimate::estimate_runtime_with_brush_and_resolution,
    pipeline::runner::{default_engine_paths, PipelineRunner},
    presets::{resolve_brush_training_preset, resolve_brush_training_preset_for_plan, Quality},
    process::ProcessManager,
    project::catalog,
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

#[derive(Debug, Clone, Default, clap::Args)]
struct PolicyArgs {
    /// Disable automatic failure retries for this run.
    #[arg(long)]
    no_auto_retry: bool,
    /// Fraction of source video frames to sample (0.001..=1).
    #[arg(long)]
    frame_sampling_ratio: Option<f64>,
}

impl PolicyArgs {
    fn policy(&self) -> Result<ooo_splat::pipeline::generation_policy::GenerationPolicy> {
        ooo_splat::pipeline::generation_policy::GenerationPolicy {
            auto_retry_enabled: !self.no_auto_retry,
            frame_sampling_ratio: self.frame_sampling_ratio,
            ..Default::default()
        }
        .validate()
    }
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Validate FFmpeg, FFprobe, CPU COLMAP and Brush.
    Health,
    /// Read video metadata or image-sequence information.
    Probe { input: PathBuf },
    /// Show the video frame plan or image-sequence plan.
    Plan {
        #[command(flatten)]
        policy: PolicyArgs,
        input: PathBuf,
        #[arg(long, value_enum, default_value_t = Quality::Balanced)]
        quality: Quality,
    },
    /// Compare estimated total runtime ranges before choosing a video quality.
    Estimate {
        #[command(flatten)]
        policy: PolicyArgs,
        input: PathBuf,
        /// Estimate only one mode (default: compare all three).
        #[arg(long, value_enum)]
        quality: Option<Quality>,
        /// Match submit --no-optimize.
        #[arg(long)]
        no_optimize: bool,
        /// Print structured estimates in milliseconds.
        #[arg(long)]
        json: bool,
    },
    /// Prepare video frames or an image sequence and optional COLMAP masks.
    Extract {
        #[command(flatten)]
        policy: PolicyArgs,
        input: PathBuf,
        output: PathBuf,
        #[arg(long, value_enum, default_value_t = Quality::Balanced)]
        quality: Quality,
    },
    /// Run the end-to-end pipeline after all fixed engine CLIs are verified.
    Generate {
        #[arg(long, value_enum, default_value_t = ooo_splat::video::panorama::VideoProjection::Auto)]
        video_projection: ooo_splat::video::panorama::VideoProjection,
        #[command(flatten)]
        policy: PolicyArgs,
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
        Commands::Plan {
            input,
            quality,
            policy,
        } => {
            let policy = policy.policy()?;
            let plan = if input.is_dir() {
                create_image_plan(&analyze_image_sequence(&input)?, &quality.preset())
            } else {
                let video =
                    probe_video(&engines.ffprobe, &input, None, &ProcessManager::new()).await?;
                policy.frame_plan(&video, quality)
            };
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
        Commands::Estimate {
            policy,
            input,
            quality,
            no_optimize,
            json,
        } => {
            let policy = policy.policy()?;
            if no_optimize && policy.frame_sampling_ratio.is_some() {
                return Err(SplatError::Process(
                    "--frame-sampling-ratio requires optimization".into(),
                ));
            }
            let video = probe_video(&engines.ffprobe, &input, None, &ProcessManager::new()).await?;
            let samples = catalog::runtime_samples().await;
            let acceleration = check_colmap_acceleration_cached(&engines).await;
            let memory = acceleration.planning_gpu_total_memory_mb();
            let (width, height) = if video.rotation.rem_euclid(180) == 90 {
                (video.height, video.width)
            } else {
                (video.width, video.height)
            };
            let qualities = quality.map_or_else(
                || vec![Quality::Fast, Quality::Balanced, Quality::High],
                |quality| vec![quality],
            );
            let mut rows = Vec::new();
            for quality in qualities {
                let plan = if no_optimize {
                    UniformRatioFrameSelection.create_plan(&video, &quality.preset())
                } else {
                    policy.frame_plan(&video, quality)
                };
                let resolution =
                    (!no_optimize).then(|| policy.resolution(quality, memory, width, height, true));
                let brush = resolution.map_or_else(
                    || resolve_brush_training_preset(quality, false, memory, width.max(height), 0),
                    |resolution| {
                        resolve_brush_training_preset_for_plan(
                            quality,
                            memory,
                            width.max(height),
                            0,
                            &resolution,
                        )
                    },
                );
                let estimate = estimate_runtime_with_brush_and_resolution(
                    &video,
                    &plan,
                    quality,
                    &samples,
                    Some(&brush),
                    resolution.map(|r| r.policy_version),
                    resolution.map_or(width.max(height), |r| r.working_long_edge()),
                );
                rows.push(serde_json::json!({
                    "quality": quality, "plan": plan, "estimate": estimate,
                }));
            }
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "input": input, "video": video, "optimize": !no_optimize, "modes": rows,
                    }))?
                );
            } else {
                println!(
                    "视频: {}（{:.1} 秒，{}×{}）",
                    input.display(),
                    video.duration,
                    video.width,
                    video.height
                );
                println!("自动优化: {}", !no_optimize);
                println!("模式        预计总耗时范围（分钟）   参考值（分钟）   计划帧数   置信度 / 历史样本");
                for row in &rows {
                    let estimate = &row["estimate"];
                    println!(
                        "{:<10}  {:.1}–{:.1}                 {:.1}             {}       {} / {}",
                        row["quality"].as_str().unwrap_or("-"),
                        estimate["lowerBoundMs"].as_f64().unwrap_or(0.0) / 60_000.0,
                        estimate["upperBoundMs"].as_f64().unwrap_or(0.0) / 60_000.0,
                        estimate["estimatedMs"].as_f64().unwrap_or(0.0) / 60_000.0,
                        row["plan"]["estimatedFrames"],
                        estimate["confidence"].as_str().unwrap_or("low"),
                        estimate["sampleCount"]
                    );
                    println!("  依据: {}", estimate["basis"].as_str().unwrap_or("-"));
                }
                println!("范围仅供选择，不含排队和首次编译；视频内容、实际优化和 GPU 负载会影响耗时。未抽帧或启动生成任务。");
            }
        }
        Commands::Extract {
            policy,
            input,
            output,
            quality,
        } => {
            let policy = policy.policy()?;
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
            let plan = policy.frame_plan(&video, quality);
            let (w, h) = if video.rotation.rem_euclid(180) == 90 {
                (video.height, video.width)
            } else {
                (video.width, video.height)
            };
            let resolution = policy.resolution(quality, None, w, h, true);
            let extraction = extract_selected_frames(
                &engines.ffmpeg,
                &input,
                &output,
                &colmap_frames,
                &masks,
                &plan.selected_frames,
                video.has_alpha,
                Some((resolution.working_width, resolution.working_height)),
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
            policy,
            video_projection,
            input,
            projects_root,
            quality,
            no_optimize,
            events,
        } => {
            let policy = policy.policy()?;
            if no_optimize && policy.frame_sampling_ratio.is_some() {
                return Err(SplatError::Process(
                    "--frame-sampling-ratio requires optimization".into(),
                ));
            }
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
            })
            .with_video_projection(video_projection)
            .with_generation_policy((!no_optimize).then_some(policy));
            let root = projects_root.unwrap_or_else(|| PathBuf::from("projects"));
            let run = runner.generate(&input, quality, &root);
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
