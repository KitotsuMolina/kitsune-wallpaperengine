use crate::scene_effect_proxy::build_scene_audio_bars_overlay;
use crate::scene_gpu_graph::build_scene_gpu_graph;
use crate::scene_plan::build_scene_plan;
use crate::types::WallpaperType;
use crate::wallpaper::inspect_wallpaper;
use anyhow::Result;
use chrono::Local;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct EffectFrequency {
    pub effect_file: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct WallpaperCompatStatus {
    pub id: String,
    pub root: String,
    pub title: Option<String>,
    pub wallpaper_type: WallpaperType,
    pub compatibility_percent: u8,
    pub quality_tier: String,
    pub capabilities: Vec<String>,
    pub issues: Vec<String>,
    pub effect_nodes: usize,
    pub likely_audio_reactive: bool,
    pub audio_overlay_plan_available: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LibraryScanReport {
    pub generated_at: String,
    pub downloads_root: String,
    pub wallpapers_scanned: usize,
    pub average_compatibility_percent: f32,
    pub counts_by_type: HashMap<String, usize>,
    pub top_effects: Vec<EffectFrequency>,
    pub wallpapers: Vec<WallpaperCompatStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoadmapEffectItem {
    pub rank: usize,
    pub effect_file: String,
    pub wallpapers_affected: usize,
    pub avg_current_score: f32,
    pub estimated_coverage_gain_points: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct LibraryRoadmapReport {
    pub generated_at: String,
    pub downloads_root: String,
    pub wallpapers_scanned: usize,
    pub baseline_average_compatibility_percent: f32,
    pub estimated_average_after_top_n: f32,
    pub top_recommendations: Vec<RoadmapEffectItem>,
}

fn clamp_score(v: i32) -> u8 {
    v.clamp(0, 100) as u8
}

fn tier_for(score: u8) -> String {
    if score >= 90 {
        "excellent".to_string()
    } else if score >= 75 {
        "good".to_string()
    } else if score >= 55 {
        "partial".to_string()
    } else {
        "limited".to_string()
    }
}

pub fn scan_library(
    downloads_root: &Path,
    top_effects: usize,
    summary_only: bool,
) -> Result<LibraryScanReport> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(downloads_root)? {
        let entry = match entry {
            Ok(v) => v,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    dirs.sort();

    let mut scanned = Vec::<WallpaperCompatStatus>::new();
    let mut counts_by_type = HashMap::<String, usize>::new();
    let mut effect_hist = HashMap::<String, usize>::new();

    for dir in dirs {
        let id = dir
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| dir.to_string_lossy().to_string());
        let mut capabilities = Vec::<String>::new();
        let mut issues = Vec::<String>::new();
        let mut score: i32 = 35;
        let mut wtype = WallpaperType::Unknown;
        let mut title = None;
        let mut effect_nodes = 0usize;
        let mut likely_audio_reactive = false;
        let mut audio_overlay_plan_available = false;

        match inspect_wallpaper(&dir.to_string_lossy(), downloads_root) {
            Ok(info) => {
                wtype = info.wallpaper_type.clone();
                title = info.title.clone();
                *counts_by_type
                    .entry(format!("{:?}", info.wallpaper_type).to_ascii_lowercase())
                    .or_insert(0) += 1;

                match info.wallpaper_type {
                    WallpaperType::Video => {
                        score = 92;
                        capabilities.push("video-playback".to_string());
                        if info.entry.is_none() {
                            score -= 50;
                            issues.push("No playable video entry detected".to_string());
                        }
                    }
                    WallpaperType::Scene => {
                        score = 78;
                        capabilities.push("scene-runtime".to_string());

                        match build_scene_plan(Path::new(&info.root)) {
                            Ok(plan) => {
                                if plan.scene_json_parse_ok {
                                    capabilities.push("scene-json-parsed".to_string());
                                } else {
                                    score -= 20;
                                    issues.push("scene.json parse failed".to_string());
                                }
                                if plan.primary_visual_asset.is_some() {
                                    capabilities.push("primary-visual-detected".to_string());
                                } else {
                                    score -= 30;
                                    issues.push("No primary visual asset in scene.pkg".to_string());
                                }
                                likely_audio_reactive = plan.likely_audio_reactive;
                                if likely_audio_reactive {
                                    capabilities.push("audio-reactive-detected".to_string());
                                    score -= 18;
                                    issues.push(
                                        "Audio-reactive path is experimental (not fully stable)"
                                            .to_string(),
                                    );
                                }
                            }
                            Err(err) => {
                                score = 20;
                                issues.push(format!("Scene plan failed: {}", err));
                            }
                        }

                        match build_scene_gpu_graph(Path::new(&info.root)) {
                            Ok(graph) => {
                                effect_nodes = graph.effect_nodes.len();
                                if effect_nodes > 0 {
                                    capabilities.push("effect-graph-detected".to_string());
                                    score -= (effect_nodes as i32).min(28);
                                }
                                for node in graph.effect_nodes {
                                    if node.effect_file.is_empty() {
                                        continue;
                                    }
                                    *effect_hist.entry(node.effect_file).or_insert(0) += 1;
                                }
                            }
                            Err(err) => {
                                issues.push(format!("GPU graph parse failed: {}", err));
                                score -= 10;
                            }
                        }

                        match build_scene_audio_bars_overlay(Path::new(&info.root)) {
                            Ok(Some(_overlay)) => {
                                audio_overlay_plan_available = true;
                                capabilities.push("audio-overlay-plan".to_string());
                                if likely_audio_reactive {
                                    score += 8;
                                }
                            }
                            Ok(None) => {}
                            Err(err) => {
                                issues.push(format!("Audio overlay parse failed: {}", err));
                            }
                        }
                    }
                    WallpaperType::Web | WallpaperType::Application => {
                        score = 25;
                        issues.push(
                            "Web/Application wallpapers are not implemented in this engine"
                                .to_string(),
                        );
                    }
                    WallpaperType::Unknown => {
                        score = 15;
                        issues.push("Unknown wallpaper type".to_string());
                    }
                }
            }
            Err(err) => {
                issues.push(format!("Inspect failed: {}", err));
                *counts_by_type.entry("unknown".to_string()).or_insert(0) += 1;
            }
        }

        let compatibility_percent = clamp_score(score);
        let tier = tier_for(compatibility_percent);
        scanned.push(WallpaperCompatStatus {
            id,
            root: dir.to_string_lossy().to_string(),
            title,
            wallpaper_type: wtype,
            compatibility_percent,
            quality_tier: tier,
            capabilities,
            issues,
            effect_nodes,
            likely_audio_reactive,
            audio_overlay_plan_available,
        });
    }

    scanned.sort_by(|a, b| {
        a.compatibility_percent
            .cmp(&b.compatibility_percent)
            .then_with(|| a.id.cmp(&b.id))
    });

    let avg = if scanned.is_empty() {
        0.0
    } else {
        scanned
            .iter()
            .map(|s| s.compatibility_percent as f32)
            .sum::<f32>()
            / scanned.len() as f32
    };

    let mut top: Vec<EffectFrequency> = effect_hist
        .into_iter()
        .map(|(effect_file, count)| EffectFrequency { effect_file, count })
        .collect();
    top.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.effect_file.cmp(&b.effect_file))
    });
    if top.len() > top_effects {
        top.truncate(top_effects);
    }

    Ok(LibraryScanReport {
        generated_at: Local::now().format("%Y-%m-%d %H:%M:%S %z").to_string(),
        downloads_root: downloads_root.to_string_lossy().to_string(),
        wallpapers_scanned: scanned.len(),
        average_compatibility_percent: ((avg * 100.0).round() / 100.0),
        counts_by_type,
        top_effects: top,
        wallpapers: if summary_only { Vec::new() } else { scanned },
    })
}

pub fn build_library_roadmap(downloads_root: &Path, top_n: usize) -> Result<LibraryRoadmapReport> {
    let report = scan_library(downloads_root, 500, false)?;
    let mut effect_to_scores = HashMap::<String, Vec<u8>>::new();
    let mut dirs = Vec::new();
    for entry in fs::read_dir(downloads_root)? {
        let entry = match entry {
            Ok(v) => v,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    dirs.sort();

    let mut score_by_id = HashMap::<String, u8>::new();
    for w in &report.wallpapers {
        score_by_id.insert(w.id.clone(), w.compatibility_percent);
    }

    for dir in dirs {
        let id = match dir.file_name() {
            Some(v) => v.to_string_lossy().to_string(),
            None => continue,
        };
        let Some(score) = score_by_id.get(&id).copied() else {
            continue;
        };
        let Ok(graph) = build_scene_gpu_graph(&dir) else {
            continue;
        };
        if graph.effect_nodes.is_empty() {
            continue;
        }
        let mut uniq = HashMap::<String, bool>::new();
        for node in graph.effect_nodes {
            if node.effect_file.is_empty() {
                continue;
            }
            uniq.insert(node.effect_file, true);
        }
        for effect in uniq.into_keys() {
            effect_to_scores.entry(effect).or_default().push(score);
        }
    }

    let mut items = Vec::<RoadmapEffectItem>::new();
    for (effect_file, mut scores) in effect_to_scores {
        if scores.is_empty() {
            continue;
        }
        scores.sort();
        let affected = scores.len();
        let avg_score = scores.iter().map(|v| *v as f32).sum::<f32>() / affected as f32;
        let lower_pressure = ((100.0 - avg_score) / 100.0).clamp(0.0, 1.0);
        let effect_weight = (affected as f32).sqrt().clamp(1.0, 12.0);
        let gain = (lower_pressure * effect_weight * 3.2).clamp(0.0, 15.0);
        items.push(RoadmapEffectItem {
            rank: 0,
            effect_file,
            wallpapers_affected: affected,
            avg_current_score: ((avg_score * 100.0).round()) / 100.0,
            estimated_coverage_gain_points: ((gain * 100.0).round()) / 100.0,
        });
    }

    items.sort_by(|a, b| {
        b.estimated_coverage_gain_points
            .partial_cmp(&a.estimated_coverage_gain_points)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.wallpapers_affected.cmp(&a.wallpapers_affected))
            .then_with(|| a.effect_file.cmp(&b.effect_file))
    });
    if items.len() > top_n.max(1) {
        items.truncate(top_n.max(1));
    }
    for (i, item) in items.iter_mut().enumerate() {
        item.rank = i + 1;
    }

    let baseline = report.average_compatibility_percent;
    let estimated_after = (baseline
        + items
            .iter()
            .map(|x| x.estimated_coverage_gain_points)
            .sum::<f32>()
            .min(30.0))
    .min(100.0);

    Ok(LibraryRoadmapReport {
        generated_at: Local::now().format("%Y-%m-%d %H:%M:%S %z").to_string(),
        downloads_root: downloads_root.to_string_lossy().to_string(),
        wallpapers_scanned: report.wallpapers_scanned,
        baseline_average_compatibility_percent: baseline,
        estimated_average_after_top_n: ((estimated_after * 100.0).round()) / 100.0,
        top_recommendations: items,
    })
}
