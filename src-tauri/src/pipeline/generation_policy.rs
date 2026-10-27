//! Versioned defaults for new runs. Absence in a saved state means legacy behavior.
use serde::{Deserialize, Serialize};

use crate::{
    error::{Result, SplatError},
    presets::{self, PlannerResolutionPlan, Quality, ResolvedBrushTrainingPreset},
    video::{FramePlan, FrameSelectionStrategy, QualityV2FrameSelection, VideoInfo},
};

pub const VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct GenerationPolicy {
    pub version: u32,
    pub auto_retry_enabled: bool,
    /// None retains the exact tier FPS default, without UI rounding.
    pub frame_sampling_ratio: Option<f64>,
}

impl Default for GenerationPolicy {
    fn default() -> Self {
        Self {
            version: VERSION,
            auto_retry_enabled: true,
            frame_sampling_ratio: None,
        }
    }
}

impl GenerationPolicy {
    pub fn oom_retry(
        self,
        resolved: ResolvedBrushTrainingPreset,
        consumed: bool,
        last_count: Option<u64>,
        initial: u64,
    ) -> Option<ResolvedBrushTrainingPreset> {
        if !self.auto_retry_enabled || consumed {
            return None;
        }
        oom_retry_preset(resolved, last_count, initial)
    }
    pub fn validate(self) -> Result<Self> {
        if !(1..=VERSION).contains(&self.version)
            || self
                .frame_sampling_ratio
                .is_some_and(|r| !r.is_finite() || !(0.001..=1.0).contains(&r))
        {
            return Err(SplatError::Process("Invalid generation policy".into()));
        }
        Ok(self)
    }

    pub fn frame_plan(self, video: &VideoInfo, quality: Quality) -> FramePlan {
        let mut preset = quality.preset();
        if let Some(ratio) = self.frame_sampling_ratio {
            preset.initial_fps = Some(video.fps * ratio);
        }
        QualityV2FrameSelection.create_plan(video, &preset)
    }

    pub fn resolution(
        self,
        quality: Quality,
        memory: Option<u64>,
        width: u32,
        height: u32,
        video: bool,
    ) -> PlannerResolutionPlan {
        // HighLarge is the existing native-resolution recipe. Hardware no longer
        // selects a lower High profile; legacy states keep their saved profile.
        presets::resolve_planner_resolution_plan(
            quality,
            if quality == Quality::High {
                Some(u64::MAX)
            } else {
                memory
            },
            width,
            height,
            video,
        )
    }

    pub fn training(
        self,
        quality: Quality,
        memory: Option<u64>,
        resolution: &PlannerResolutionPlan,
        sparse_points: u64,
        initial_points: u64,
        fine_budget: Option<&super::splat_budget::FineSplatBudget>,
    ) -> Result<ResolvedBrushTrainingPreset> {
        u32::try_from(initial_points).map_err(|_| {
            SplatError::Process("Initialization exceeds the supported Splat count range".into())
        })?;
        let mut resolved = presets::resolve_brush_training_preset_for_plan(
            quality,
            memory,
            resolution.working_long_edge(),
            initial_points,
            resolution,
        );
        if quality == Quality::High {
            if self.version >= 2 {
                let budget = fine_budget.ok_or_else(|| {
                    SplatError::Process("Detailed Splat budget is missing or stale".into())
                })?;
                apply_budget(
                    &mut resolved,
                    budget.validated_ceiling(sparse_points)?,
                    initial_points,
                )?;
            } else {
                apply_sparse_budget(&mut resolved, sparse_points, initial_points)?;
            }
            resolved.preset.max_resolution = resolution.working_long_edge();
        }
        Ok(resolved)
    }

    pub fn panorama_training(
        self,
        quality: Quality,
        memory: Option<u64>,
        face_size: u32,
        sparse_points: u64,
        initial_points: u64,
    ) -> Result<ResolvedBrushTrainingPreset> {
        let mut resolved = presets::resolve_panorama_training_preset(
            quality,
            true,
            memory,
            face_size,
            if quality == Quality::High {
                0
            } else {
                initial_points
            },
        )?;
        if quality == Quality::High {
            apply_sparse_budget(&mut resolved, sparse_points, initial_points)?;
            let snapshot = resolved.panorama.as_mut().expect("panorama recipe");
            snapshot.recipe_version = 2;
            snapshot.memory_max_splats = None;
            snapshot.initial_points = initial_points;
            snapshot.initial_points_override = initial_points > sparse_points * 40;
        }
        Ok(resolved)
    }
}

fn apply_sparse_budget(
    resolved: &mut ResolvedBrushTrainingPreset,
    sparse: u64,
    initial: u64,
) -> Result<()> {
    let requested = sparse
        .checked_mul(40)
        .filter(|n| *n > 0)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| {
            SplatError::Process(
                "COLMAP sparse point budget exceeds the supported Splat count range".into(),
            )
        })?;
    apply_budget(resolved, requested, initial)
}

fn apply_budget(
    resolved: &mut ResolvedBrushTrainingPreset,
    requested: u32,
    initial: u64,
) -> Result<()> {
    let initial = u32::try_from(initial).map_err(|_| {
        SplatError::Process("Initialization exceeds the supported Splat count range".into())
    })?;
    resolved.configured_max_splats = Some(requested);
    resolved.preset.max_splats = Some(requested.max(initial));
    Ok(())
}

/// No guessing from a configured ceiling, other process, or historical count.
pub fn oom_retry_preset(
    resolved: ResolvedBrushTrainingPreset,
    last_count: Option<u64>,
    initial: u64,
) -> Option<ResolvedBrushTrainingPreset> {
    let last = last_count?;
    let cap = u32::try_from(last.checked_mul(4)? / 5).ok()?;
    if cap == 0
        || u64::from(cap) < initial
        || resolved.preset.max_splats.is_some_and(|old| cap >= old)
    {
        return None;
    }
    let mut retry = resolved;
    retry.preset.max_splats = Some(cap);
    Some(retry)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn budget(points: u64) -> super::super::splat_budget::FineSplatBudget {
        super::super::splat_budget::FineSplatBudget {
            version: super::super::splat_budget::VERSION,
            coefficient: 0.5,
            registered_images: 10,
            effective_pixels: 100_000_000,
            sparse_points: points,
            tracked_points: points,
            observations: points * 5,
            mean_track_length: 5.,
            requested_max_splats: super::super::splat_budget::ceiling(
                100_000_000,
                points,
                points * 5,
            )
            .unwrap(),
        }
    }
    fn video(frames: u64, fps: f64) -> VideoInfo {
        VideoInfo {
            duration: frames as f64 / fps,
            fps,
            total_frames: frames,
            width: 1920,
            height: 1080,
            codec: "h264".into(),
            rotation: 0,
            pixel_format: "rgb24".into(),
            has_alpha: false,
        }
    }
    #[test]
    fn base_sampling_is_separate_from_backfill_and_manual_selection_is_never_capped() {
        for (quality, count, candidates) in [
            (Quality::Fast, 360, 540),
            (Quality::Balanced, 480, 720),
            (Quality::High, 720, 900),
        ] {
            let p = GenerationPolicy::default().frame_plan(&video(1800, 30.), quality);
            assert_eq!(p.estimated_frames, count);
            assert_eq!(p.rescue_max_frames, candidates);
            for ratio in [0.001, 0.1, 0.333, 0.9, 1.0] {
                let p = GenerationPolicy {
                    frame_sampling_ratio: Some(ratio),
                    ..Default::default()
                }
                .frame_plan(&video(1800, 30.), quality);
                assert!(p.estimated_frames >= 30);
                assert!(p
                    .selected_frames
                    .iter()
                    .all(|f| p.candidate_frames.contains(f)));
                assert!(p.rescue_max_frames >= p.estimated_frames);
                assert_eq!(p.estimated_frames, ((1800. * ratio).round() as u64).max(30));
            }
        }
        assert_eq!(
            GenerationPolicy::default()
                .frame_plan(&video(12, 30.), Quality::Fast)
                .estimated_frames,
            12
        );
    }
    #[test]
    fn retry_switch_does_not_change_initial_plan_or_preset_and_input_is_validated() {
        let off = GenerationPolicy {
            auto_retry_enabled: false,
            ..Default::default()
        };
        let on = GenerationPolicy::default();
        assert_eq!(
            off.frame_plan(&video(1800, 30.), Quality::Fast),
            on.frame_plan(&video(1800, 30.), Quality::Fast)
        );
        for quality in [Quality::Fast, Quality::Balanced, Quality::High] {
            let r = on.resolution(quality, None, 3840, 2160, true);
            assert_eq!(
                on.training(quality, None, &r, 10_000, 10_000, Some(&budget(10_000)))
                    .unwrap(),
                off.training(quality, None, &r, 10_000, 10_000, Some(&budget(10_000)))
                    .unwrap()
            );
        }
        for ratio in [0., -1., 1.1, f64::NAN, f64::INFINITY] {
            assert!(GenerationPolicy {
                frame_sampling_ratio: Some(ratio),
                ..on
            }
            .validate()
            .is_err());
        }
        assert_eq!(serde_json::from_str::<GenerationPolicy>("{}").unwrap(), on);
    }
    #[test]
    fn high_native_budget_is_not_vram_tiered_and_preserves_initialization() {
        for memory in [None, Some(4096), Some(8192), Some(24576)] {
            let p = GenerationPolicy::default();
            let r = p.resolution(Quality::High, memory, 8192, 5464, true);
            assert_eq!(r.working_long_edge(), 8192);
            let t = p
                .training(
                    Quality::High,
                    memory,
                    &r,
                    200_000,
                    9_000_000,
                    Some(&budget(200_000)),
                )
                .unwrap();
            assert_eq!(
                t.configured_max_splats,
                Some(budget(200_000).requested_max_splats)
            );
            assert_eq!(t.preset.max_splats, Some(9_000_000));
            assert_eq!(t.preset.max_resolution, 8192);
            let pano = p
                .panorama_training(Quality::High, memory, 2048, 400_000, 12_000_000)
                .unwrap();
            assert_eq!(pano.preset.max_splats, Some(16_000_000));
            assert!(p
                .training(Quality::High, memory, &r, u64::MAX, 1, None)
                .is_err());
            assert!(p.training(Quality::High, memory, &r, 0, 1, None).is_err());
        }
    }
    #[test]
    fn old_policy_keeps_sparse_budget_and_new_policy_changes_only_ordinary_high_ceiling() {
        let current = GenerationPolicy::default();
        let old = GenerationPolicy {
            version: 1,
            ..current
        };
        assert!(old.validate().is_ok());
        let r = current.resolution(Quality::High, None, 3840, 2560, false);
        let budget = budget(66_218);
        let legacy = old
            .training(Quality::High, None, &r, 66_218, 66_218, None)
            .unwrap();
        assert_eq!(legacy.preset.max_splats, Some(2_648_720));
        let modern = current
            .training(Quality::High, None, &r, 66_218, 66_218, Some(&budget))
            .unwrap();
        let mut same_recipe = modern;
        same_recipe.configured_max_splats = legacy.configured_max_splats;
        same_recipe.preset.max_splats = legacy.preset.max_splats;
        assert_eq!(same_recipe, legacy);
        assert!(current
            .training(Quality::High, None, &r, 66_218, 66_218, None)
            .is_err());
        for quality in [Quality::Fast, Quality::Balanced] {
            assert_eq!(
                old.training(quality, None, &r, 66_218, 66_218, None)
                    .unwrap(),
                current
                    .training(quality, None, &r, 66_218, 66_218, None)
                    .unwrap()
            );
        }
        for quality in [Quality::Fast, Quality::Balanced, Quality::High] {
            assert_eq!(
                old.panorama_training(quality, None, 1024, 66_218, 66_218)
                    .unwrap(),
                current
                    .panorama_training(quality, None, 1024, 66_218, 66_218)
                    .unwrap()
            );
        }
        let json = serde_json::to_string(&old).unwrap();
        let resumed: GenerationPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(resumed, old);
        assert_eq!(
            resumed
                .training(Quality::High, None, &r, 66_218, 66_218, None)
                .unwrap(),
            legacy
        );
    }
    #[test]
    fn oom_changes_only_ceiling_and_never_discards_initial_points() {
        let p = GenerationPolicy::default();
        let r = p.resolution(Quality::High, None, 4096, 2048, false);
        let t = p
            .training(
                Quality::High,
                None,
                &r,
                100_000,
                100_000,
                Some(&budget(100_000)),
            )
            .unwrap();
        assert!(oom_retry_preset(t, None, 1).is_none());
        assert!(oom_retry_preset(t, Some(100_000), 100_000).is_none());
        let retry = oom_retry_preset(t, Some(1_000_001), 100_000).unwrap();
        assert_eq!(retry.preset.max_splats, Some(800_000));
        let mut restored = retry;
        restored.preset.max_splats = t.preset.max_splats;
        assert_eq!(restored, t);
    }

    #[test]
    fn all_tiers_share_retry_consent_count_and_one_attempt_rules() {
        for quality in [Quality::Fast, Quality::Balanced, Quality::High] {
            let on = GenerationPolicy::default();
            let off = GenerationPolicy {
                auto_retry_enabled: false,
                ..on
            };
            let resolution = on.resolution(quality, None, 8192, 4096, false);
            let ordinary = on
                .training(
                    quality,
                    None,
                    &resolution,
                    10_000,
                    10_000,
                    Some(&budget(10_000)),
                )
                .unwrap();
            let panorama = on
                .panorama_training(quality, None, 512, 10_000, 10_000)
                .unwrap();
            for initial in [ordinary, panorama] {
                let retry = on.oom_retry(initial, false, Some(200_000), 10_000).unwrap();
                assert_eq!(retry.preset.max_splats, Some(160_000));
                assert!(off
                    .oom_retry(initial, false, Some(200_000), 10_000)
                    .is_none());
                assert!(on.oom_retry(initial, true, Some(200_000), 10_000).is_none());
                assert!(on.oom_retry(initial, false, None, 10_000).is_none());
            }
        }
    }
}
