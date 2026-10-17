use crate::error::{Result, SplatError};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn authorize_input(path: &Path, roots: &[PathBuf]) -> Result<PathBuf> {
    let path =
        std::fs::canonicalize(path).map_err(|_| SplatError::InvalidPath(path.to_path_buf()))?;
    if !roots
        .iter()
        .filter_map(|r| {
            std::fs::canonicalize(r)
                .ok()
                .filter(|resolved| resolved == r)
        })
        .any(|r| path.starts_with(r))
    {
        return Err(SplatError::Process("素材路径不在授权目录内".into()));
    }
    Ok(path)
}

pub(crate) fn checkpoint_snapshot(state: &crate::project::PipelineStateFile) -> Value {
    serde_json::json!({"quality":state.preset,"planner_enabled":state.planner_enabled,"resolution_policy_version":state.resolution_policy_version,"resolution_plan":state.resolution_plan,"brush_training":state.brush_training,"frame_plan":{"sampling_fps":state.frames.as_ref().map(|f|f.sampling_fps),"estimated_frames":state.frames.as_ref().map(|f|f.estimated_frames),"extracted_frames":state.frames.as_ref().and_then(|f|f.extracted_frames),"mask_count":state.frames.as_ref().and_then(|f|f.mask_count)}})
}
