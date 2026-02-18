use crate::cli::{PlaybackProfile, ProxyPreset};
use anyhow::{Context, Result};
use std::process::Command;

#[derive(Debug, Clone, Copy)]
pub struct ProxyTune {
    pub width: u32,
    pub fps: u32,
    pub crf: u8,
    pub playback_profile: PlaybackProfile,
}

pub fn preset_values(preset: ProxyPreset) -> ProxyTune {
    match preset {
        ProxyPreset::Eco => ProxyTune {
            width: 1280,
            fps: 24,
            crf: 30,
            playback_profile: PlaybackProfile::Performance,
        },
        ProxyPreset::Balanced => ProxyTune {
            width: 1920,
            fps: 30,
            crf: 28,
            playback_profile: PlaybackProfile::Performance,
        },
        ProxyPreset::Ultra => ProxyTune {
            width: 2560,
            fps: 60,
            crf: 22,
            playback_profile: PlaybackProfile::Balanced,
        },
    }
}

fn has_discrete_gpu() -> Result<bool> {
    let out = Command::new("lspci")
        .output()
        .context("Failed to execute lspci for hardware auto-tune")?;

    if !out.status.success() {
        return Ok(false);
    }

    let text = String::from_utf8_lossy(&out.stdout).to_ascii_lowercase();
    let has_vga = text.contains("vga compatible controller") || text.contains("3d controller");
    let has_nvidia = text.contains(" nvidia ") || text.contains("geforce");
    let has_amd = text.contains(" advanced micro devices")
        || text.contains("radeon")
        || text.contains(" rx ");
    let has_intel = text.contains("intel corporation") && has_vga;

    // Heuristic: if NVIDIA or non-iGPU AMD is present, treat as discrete-capable.
    Ok(has_nvidia || (has_amd && !has_intel))
}

fn is_laptop() -> bool {
    std::fs::read_dir("/sys/class/power_supply")
        .ok()
        .map(|mut it| {
            it.any(|e| {
                e.ok()
                    .and_then(|v| v.file_name().into_string().ok())
                    .map(|n| n.starts_with("BAT"))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn cpu_core_count() -> Result<u32> {
    let out = Command::new("lscpu")
        .output()
        .context("Failed to execute lscpu for hardware auto-tune")?;

    if !out.status.success() {
        return Ok(4);
    }

    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("cpu(s):") {
            let v = line
                .split(':')
                .nth(1)
                .map(str::trim)
                .and_then(|n| n.parse::<u32>().ok())
                .unwrap_or(4);
            return Ok(v.max(1));
        }
    }

    Ok(4)
}

pub fn auto_tune_preset() -> ProxyPreset {
    let discrete = has_discrete_gpu().unwrap_or(false);
    let cores = cpu_core_count().unwrap_or(4);
    let laptop = is_laptop();

    // Conservative auto-tuning: prioritize stable thermals/noise over max quality.
    if laptop {
        return ProxyPreset::Eco;
    }

    if discrete && cores >= 10 {
        ProxyPreset::Balanced
    } else {
        ProxyPreset::Eco
    }
}
