use crate::scene_pkg::{find_entry, parse_scene_pkg, read_entry_bytes};
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct AssetCandidate {
    pub filename: String,
    pub length: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScenePlan {
    pub pkg_path: String,
    pub entries_count: usize,
    pub scene_json_entry: Option<String>,
    pub scene_json_parse_ok: bool,
    pub primary_visual_asset: Option<String>,
    pub primary_music_asset: Option<String>,
    pub texture_candidates: Vec<AssetCandidate>,
    pub image_candidates: Vec<AssetCandidate>,
    pub audio_candidates: Vec<AssetCandidate>,
    pub reactive_hints: Vec<String>,
    pub likely_audio_reactive: bool,
    pub notes: Vec<String>,
}

fn has_ext(name: &str, exts: &[&str]) -> bool {
    let ext = name
        .to_ascii_lowercase()
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_string();
    exts.contains(&ext.as_str())
}

fn collect_reactive_hints(value: &Value, path: &str, out: &mut Vec<String>) {
    const TOKENS: [&str; 8] = [
        "audio",
        "visualizer",
        "spectrum",
        "fft",
        "bass",
        "beat",
        "vu",
        "music",
    ];

    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let k_lower = k.to_ascii_lowercase();
                if TOKENS.iter().any(|t| k_lower.contains(t)) && out.len() < 64 {
                    let hint = format!("{}.{}", path, k);
                    if !out.contains(&hint) {
                        out.push(hint);
                    }
                }
                let next = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{}.{}", path, k)
                };
                collect_reactive_hints(v, &next, out);
            }
        }
        Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                let next = format!("{}[{}]", path, i);
                collect_reactive_hints(v, &next, out);
            }
        }
        _ => {}
    }
}

fn to_candidates(mut entries: Vec<(String, u32)>) -> Vec<AssetCandidate> {
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    entries
        .into_iter()
        .map(|(filename, length)| AssetCandidate { filename, length })
        .collect()
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

pub fn build_scene_plan(root: &Path) -> Result<ScenePlan> {
    let pkg_path = pick_pkg_path(root)
        .with_context(|| format!("No scene.pkg/gifscene.pkg found in {}", root.display()))?;
    let pkg = parse_scene_pkg(&pkg_path)?;

    let scene_entry = find_entry(&pkg, "scene.json").or_else(|| find_entry(&pkg, "gifscene.json"));

    let texture_entries: Vec<(String, u32)> = pkg
        .entries
        .iter()
        .filter(|e| has_ext(&e.filename, &["tex"]))
        .map(|e| (e.filename.clone(), e.length))
        .collect();

    let image_entries: Vec<(String, u32)> = pkg
        .entries
        .iter()
        .filter(|e| has_ext(&e.filename, &["png", "jpg", "jpeg", "webp", "bmp", "gif"]))
        .map(|e| (e.filename.clone(), e.length))
        .collect();

    let audio_entries: Vec<(String, u32)> = pkg
        .entries
        .iter()
        .filter(|e| has_ext(&e.filename, &["mp3", "ogg", "wav", "flac", "m4a"]))
        .map(|e| (e.filename.clone(), e.length))
        .collect();

    let texture_candidates = to_candidates(texture_entries);
    let image_candidates = to_candidates(image_entries);
    let audio_candidates = to_candidates(audio_entries);

    let primary_visual_asset = texture_candidates
        .first()
        .map(|v| v.filename.clone())
        .or_else(|| image_candidates.first().map(|v| v.filename.clone()));

    let primary_music_asset = audio_candidates.first().map(|v| v.filename.clone());

    let mut reactive_hints = Vec::new();
    let mut scene_json_parse_ok = false;

    if let Some(entry) = &scene_entry {
        let bytes = read_entry_bytes(&pkg, entry)?;
        let scene_json: Value = serde_json::from_slice(&bytes)
            .with_context(|| format!("Invalid JSON in pkg entry {}", entry.filename))?;
        scene_json_parse_ok = true;
        collect_reactive_hints(&scene_json, "", &mut reactive_hints);
    }

    let likely_audio_reactive = !reactive_hints.is_empty();

    let mut notes = Vec::new();
    if scene_entry.is_none() {
        notes.push("scene.json not found in package".to_string());
    }
    if primary_visual_asset.is_none() {
        notes.push("No texture/image asset candidate found".to_string());
    }
    if primary_music_asset.is_none() {
        notes.push("No audio asset candidate found".to_string());
    }
    if likely_audio_reactive {
        notes.push("Audio-reactive hints detected in scene.json keys".to_string());
    }

    Ok(ScenePlan {
        pkg_path: pkg_path.to_string_lossy().to_string(),
        entries_count: pkg.entries.len(),
        scene_json_entry: scene_entry.map(|e| e.filename),
        scene_json_parse_ok,
        primary_visual_asset,
        primary_music_asset,
        texture_candidates,
        image_candidates,
        audio_candidates,
        reactive_hints,
        likely_audio_reactive,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_ext_works() {
        assert!(has_ext("x/abc.tex", &["tex"]));
        assert!(has_ext("x/abc.MP3", &["mp3"]));
        assert!(!has_ext("x/abc.bin", &["mp3"]));
    }

    #[test]
    fn collects_audio_keys() {
        let v: Value = serde_json::json!({
            "general": {"supportsaudioprocessing": true},
            "effects": [{"visualizer": {"fft": true}}]
        });
        let mut out = Vec::new();
        collect_reactive_hints(&v, "", &mut out);
        assert!(!out.is_empty());
    }
}
