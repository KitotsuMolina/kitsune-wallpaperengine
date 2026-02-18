use crate::scene_pkg::{
    default_scene_cache_root, extract_entry_to_cache, find_entry, parse_scene_pkg,
};
use crate::scene_runtime::{SceneRuntimeResult, run_scene_runtime};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct UniformSchema {
    pub time: String,
    pub rms: String,
    pub peak: String,
    pub energy: String,
    pub beat: String,
}

#[derive(Debug, Serialize)]
pub struct SceneRenderSession {
    pub session_dir: String,
    pub visual_asset_path: String,
    pub music_asset_path: Option<String>,
    pub uniforms_path: String,
    pub manifest_path: String,
    pub runtime: SceneRuntimeResult,
    pub uniform_schema: UniformSchema,
    pub frame_count: usize,
}

#[derive(Debug, Serialize)]
struct SessionManifest {
    pub version: u32,
    pub visual_asset_path: String,
    pub music_asset_path: Option<String>,
    pub uniforms_path: String,
    pub frame_count: usize,
    pub frame_ms: u64,
    pub uniform_schema: UniformSchema,
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

pub fn build_scene_render_session(
    root: &Path,
    source: Option<String>,
    seconds: u64,
    frame_ms: u64,
) -> Result<SceneRenderSession> {
    let runtime = run_scene_runtime(root, source, seconds, frame_ms, true)?;

    let pkg_path = pick_pkg_path(root)
        .with_context(|| format!("No scene.pkg/gifscene.pkg found in {}", root.display()))?;
    let pkg = parse_scene_pkg(&pkg_path)?;

    let visual_name = runtime
        .scene_plan
        .primary_visual_asset
        .as_ref()
        .context("scene plan does not provide primary visual asset")?;

    let visual_entry = find_entry(&pkg, visual_name).with_context(|| {
        format!(
            "Primary visual asset '{}' not found in package",
            visual_name
        )
    })?;

    let cache_key = root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| root.to_string_lossy().replace('/', "_"));

    let session_dir = default_scene_cache_root(&cache_key).join("render-session");
    fs::create_dir_all(&session_dir)
        .with_context(|| format!("Failed to create session dir {}", session_dir.display()))?;

    let assets_dir = session_dir.join("assets");
    let visual_asset = extract_entry_to_cache(&pkg, &visual_entry, &assets_dir)?;

    let music_asset = if let Some(music_name) = &runtime.scene_plan.primary_music_asset {
        if let Some(entry) = find_entry(&pkg, music_name) {
            Some(extract_entry_to_cache(&pkg, &entry, &assets_dir)?)
        } else {
            None
        }
    } else {
        None
    };

    let uniforms_path = session_dir.join("uniforms.json");
    let uniforms_json = serde_json::to_vec_pretty(&runtime.uniforms)?;
    fs::write(&uniforms_path, uniforms_json)
        .with_context(|| format!("Failed writing uniforms file {}", uniforms_path.display()))?;

    let uniform_schema = UniformSchema {
        time: "u_time".to_string(),
        rms: "u_audio_rms".to_string(),
        peak: "u_audio_peak".to_string(),
        energy: "u_audio_energy".to_string(),
        beat: "u_audio_beat".to_string(),
    };

    let manifest = SessionManifest {
        version: 1,
        visual_asset_path: visual_asset.to_string_lossy().to_string(),
        music_asset_path: music_asset
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        uniforms_path: uniforms_path.to_string_lossy().to_string(),
        frame_count: runtime.uniforms.len(),
        frame_ms: runtime.frame_ms,
        uniform_schema: UniformSchema {
            time: uniform_schema.time.clone(),
            rms: uniform_schema.rms.clone(),
            peak: uniform_schema.peak.clone(),
            energy: uniform_schema.energy.clone(),
            beat: uniform_schema.beat.clone(),
        },
        notes: vec![
            "Headless render session generated (GPU backend pending)".to_string(),
            "Use uniform_schema names directly as shader uniforms".to_string(),
        ],
    };

    let manifest_path = session_dir.join("manifest.json");
    let manifest_json = serde_json::to_vec_pretty(&manifest)?;
    fs::write(&manifest_path, manifest_json)
        .with_context(|| format!("Failed writing manifest file {}", manifest_path.display()))?;

    if runtime.uniforms.is_empty() {
        bail!("No uniforms generated for render session");
    }

    Ok(SceneRenderSession {
        session_dir: session_dir.to_string_lossy().to_string(),
        visual_asset_path: visual_asset.to_string_lossy().to_string(),
        music_asset_path: music_asset.map(|p| p.to_string_lossy().to_string()),
        uniforms_path: uniforms_path.to_string_lossy().to_string(),
        manifest_path: manifest_path.to_string_lossy().to_string(),
        frame_count: runtime.uniforms.len(),
        runtime,
        uniform_schema,
    })
}
