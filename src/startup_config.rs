use crate::cli::PlaybackProfile;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartupConfig {
    pub version: u32,
    pub entries: Vec<MonitorEntry>,
}

impl Default for StartupConfig {
    fn default() -> Self {
        Self {
            version: 1,
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorEntry {
    pub monitor: String,
    #[serde(flatten)]
    pub command: StartupCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum StartupCommand {
    Video {
        video: String,
        downloads_root: PathBuf,
        keep_services: bool,
        mute_audio: bool,
        profile: PlaybackProfile,
        display_fps: Option<u32>,
        seamless_loop: bool,
        loop_crossfade: bool,
        loop_crossfade_seconds: f32,
        optimize: bool,
        proxy_width: u32,
        proxy_fps: u32,
        proxy_crf: u8,
    },
    Apply {
        wallpaper: String,
        downloads_root: PathBuf,
        keep_services: bool,
        mute_audio: bool,
        profile: PlaybackProfile,
        display_fps: Option<u32>,
        allow_scene_preview_fallback: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StartupState {
    pub monitor_fingerprints: BTreeMap<String, u64>,
}

pub fn load_config(path: &Path) -> Result<StartupConfig> {
    if !path.is_file() {
        return Ok(StartupConfig::default());
    }
    let raw = fs::read(path).with_context(|| format!("Failed reading {}", path.display()))?;
    let cfg: StartupConfig = serde_json::from_slice(&raw)
        .with_context(|| format!("Invalid JSON in {}", path.display()))?;
    Ok(cfg)
}

pub fn save_config(path: &Path, cfg: &StartupConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed creating {}", parent.display()))?;
    }
    fs::write(path, serde_json::to_vec_pretty(cfg)?)
        .with_context(|| format!("Failed writing {}", path.display()))?;
    Ok(())
}

pub fn upsert_entry(cfg: &mut StartupConfig, entry: MonitorEntry) {
    if let Some(existing) = cfg.entries.iter_mut().find(|e| e.monitor == entry.monitor) {
        *existing = entry;
    } else {
        cfg.entries.push(entry);
    }
    cfg.entries.sort_by(|a, b| a.monitor.cmp(&b.monitor));
}

pub fn remove_entry(cfg: &mut StartupConfig, monitor: &str) -> bool {
    let before = cfg.entries.len();
    cfg.entries.retain(|e| e.monitor != monitor);
    before != cfg.entries.len()
}

fn default_state_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local/state/kitsune-livewallpaper/start-config-state.json");
    }
    PathBuf::from("/tmp/kitsune-livewallpaper-start-config-state.json")
}

pub fn load_state() -> Result<StartupState> {
    let path = default_state_path();
    if !path.is_file() {
        return Ok(StartupState::default());
    }
    let raw = fs::read(&path).with_context(|| format!("Failed reading {}", path.display()))?;
    let state: StartupState = serde_json::from_slice(&raw)
        .with_context(|| format!("Invalid JSON in {}", path.display()))?;
    Ok(state)
}

pub fn save_state(state: &StartupState) -> Result<()> {
    let path = default_state_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed creating {}", parent.display()))?;
    }
    fs::write(&path, serde_json::to_vec_pretty(state)?)
        .with_context(|| format!("Failed writing {}", path.display()))?;
    Ok(())
}

pub fn entry_fingerprint(entry: &MonitorEntry) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    entry.monitor.hash(&mut hasher);
    if let Ok(bytes) = serde_json::to_vec(entry) {
        bytes.hash(&mut hasher);
    }
    hasher.finish()
}
