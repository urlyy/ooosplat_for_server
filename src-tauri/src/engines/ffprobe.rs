use std::{ffi::OsString, path::Path};

use crate::{
    error::{Result, SplatError},
    process::{ProcessManager, ProcessSpec},
    video::{parse_ffprobe_json, VideoInfo},
};

pub async fn probe_video(
    executable: &Path,
    input: &Path,
    log_path: Option<std::path::PathBuf>,
    process_manager: &ProcessManager,
) -> Result<VideoInfo> {
    if !input.is_file() {
        return Err(SplatError::InvalidPath(input.to_path_buf()));
    }
    let output = process_manager
        .run(ProcessSpec {
            executable: executable.to_path_buf(),
            args: vec![
                OsString::from("-v"),
                OsString::from("error"),
                OsString::from("-select_streams"),
                OsString::from("v:0"),
                // Full stream metadata includes rotation side data on both FFprobe 4.4 and newer.
                OsString::from("-show_streams"),
                OsString::from("-show_format"),
                OsString::from("-of"),
                OsString::from("json"),
                input.as_os_str().to_owned(),
            ],
            working_directory: input.parent().map(Path::to_path_buf),
            log_path,
            observer: None,
        })
        .await?;
    if !output.success {
        return Err(SplatError::InvalidVideo(format!(
            "FFprobe 无法读取视频：{}",
            output.failure_detail()
        )));
    }
    parse_ffprobe_json(&output.stdout)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires system FFmpeg and FFprobe (Ubuntu 22.04 compatibility check)"]
    async fn system_ffprobe_reads_video_and_rotation() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("input.mp4");
        let rotated = temp.path().join("rotated.mp4");
        let result = std::process::Command::new("/usr/bin/ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=red:s=64x48:r=10:d=1",
                "-c:v",
                "mpeg4",
            ])
            .arg(&input)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let result = std::process::Command::new("/usr/bin/ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&input)
            .args(["-c", "copy", "-metadata:s:v:0", "rotate=90"])
            .arg(&rotated)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let video = probe_video(
            Path::new("/usr/bin/ffprobe"),
            &rotated,
            None,
            &ProcessManager::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            (video.width, video.height, video.total_frames),
            (64, 48, 10)
        );
        assert_eq!(video.rotation.abs(), 90);
        assert!((video.duration - 1.0).abs() < 0.05);
    }
}
