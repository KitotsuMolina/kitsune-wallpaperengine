use crate::audio::{AudioLevelFrame, AudioStreamResult, stream_audio_levels};
use crate::scene_pkg::{
    default_scene_cache_root, extract_entry_to_cache, find_entry, parse_scene_pkg,
};
use crate::scene_plan::{ScenePlan, build_scene_plan};
use anyhow::{Result, bail};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct UniformFrame {
    pub frame_index: u64,
    pub time_s: f32,
    pub rms: f32,
    pub peak: f32,
    pub energy: f32,
    pub beat: f32,
}

#[derive(Debug, Serialize)]
pub struct SceneRuntimeResult {
    pub scene_plan: ScenePlan,
    pub used_audio_source: String,
    pub frame_ms: u64,
    pub uniforms: Vec<UniformFrame>,
    pub extracted_music_path: Option<String>,
    pub notes: Vec<String>,
}

fn pick_pkg_path(root: &Path) -> Option<PathBuf> {
    if root.join("scene.pkg").is_file() {
        Some(root.join("scene.pkg"))
    } else if root.join("gifscene.pkg").is_file() {
        Some(root.join("gifscene.pkg"))
    } else {
        None
    }
}

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

fn build_uniform_timeline(frames: &[AudioLevelFrame], frame_ms: u64) -> Vec<UniformFrame> {
    let mut out = Vec::with_capacity(frames.len());
    let mut ema_rms = 0.0f32;

    for f in frames {
        ema_rms = (ema_rms * 0.85) + (f.rms * 0.15);
        let energy = clamp01((f.rms * 3.0).powf(0.75));
        let threshold = (ema_rms * 1.55).max(0.02);
        let beat = if f.peak > threshold {
            clamp01((f.peak - threshold) * 4.5)
        } else {
            0.0
        };

        out.push(UniformFrame {
            frame_index: f.frame_index,
            time_s: (f.frame_index as f32 * frame_ms as f32) / 1000.0,
            rms: f.rms,
            peak: f.peak,
            energy,
            beat,
        });
    }

    out
}

fn silent_audio_stream(source: Option<String>, seconds: u64, frame_ms: u64) -> AudioStreamResult {
    let frame_count = ((seconds.saturating_mul(1000)) / frame_ms.max(1)).max(1);
    let frames = (0..frame_count)
        .map(|i| AudioLevelFrame {
            frame_index: i,
            peak: 0.0,
            rms: 0.0,
        })
        .collect::<Vec<_>>();

    AudioStreamResult {
        source: source.unwrap_or_else(|| "silent-fallback".to_string()),
        sample_rate: 48_000,
        channels: 2,
        frame_ms,
        duration_ms: seconds.saturating_mul(1000),
        samples: 0,
        frames,
    }
}

pub fn run_scene_runtime(
    root: &Path,
    source: Option<String>,
    seconds: u64,
    frame_ms: u64,
    extract_music: bool,
) -> Result<SceneRuntimeResult> {
    let plan = build_scene_plan(root)?;

    if plan.scene_json_entry.is_none() {
        bail!("scene runtime requires scene.json/gifscene.json inside package");
    }

    let mut notes = Vec::new();
    if plan.likely_audio_reactive {
        notes.push("Audio-reactive hints found in scene plan".to_string());
    }

    let stream = match stream_audio_levels(source.clone(), seconds, frame_ms) {
        Ok(stream) => stream,
        Err(err) => {
            notes.push(format!(
                "Live audio capture unavailable, using silent fallback timeline: {}",
                err
            ));
            silent_audio_stream(source, seconds, frame_ms)
        }
    };
    let uniforms = build_uniform_timeline(&stream.frames, frame_ms);

    let mut extracted_music_path = None;
    if extract_music {
        if let Some(pkg_path) = pick_pkg_path(root) {
            if let Some(music_name) = &plan.primary_music_asset {
                let pkg = parse_scene_pkg(&pkg_path)?;
                if let Some(entry) = find_entry(&pkg, music_name) {
                    let cache_key = root
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| root.to_string_lossy().replace('/', "_"));
                    let cache_root = default_scene_cache_root(&cache_key).join("runtime-assets");
                    let extracted = extract_entry_to_cache(&pkg, &entry, &cache_root)?;
                    extracted_music_path = Some(extracted.to_string_lossy().to_string());
                } else {
                    notes.push(format!(
                        "Primary music asset '{}' was not found in package",
                        music_name
                    ));
                }
            } else {
                notes.push("Scene plan has no primary music asset".to_string());
            }
        } else {
            notes.push("No scene.pkg/gifscene.pkg found for music extraction".to_string());
        }
    }

    Ok(SceneRuntimeResult {
        scene_plan: plan,
        used_audio_source: stream.source,
        frame_ms,
        uniforms,
        extracted_music_path,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_is_generated() {
        let frames = vec![
            AudioLevelFrame {
                frame_index: 0,
                peak: 0.1,
                rms: 0.05,
            },
            AudioLevelFrame {
                frame_index: 1,
                peak: 0.3,
                rms: 0.12,
            },
        ];

        let uniforms = build_uniform_timeline(&frames, 50);
        assert_eq!(uniforms.len(), 2);
        assert!(uniforms[1].time_s > uniforms[0].time_s);
        assert!(uniforms[1].energy >= uniforms[0].energy);
    }
}
