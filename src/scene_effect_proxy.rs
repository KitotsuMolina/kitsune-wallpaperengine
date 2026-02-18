use crate::scene_pkg::{extract_entry_to_cache, find_entry, parse_scene_pkg, read_entry_bytes};
use crate::scene_gpu_graph::{SceneGpuGraph, build_scene_gpu_graph};
use crate::scene_native_runtime::{NativeSupportTier, build_native_runtime_plan};
use crate::tex_payload::extract_playable_proxy_from_tex;
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

const EFFECT_LAYER_LIMIT: usize = 1;
const EFFECT_LAYER_ALPHA: f32 = 0.35;
const DEFAULT_CONTRAST: f32 = 1.01;
const DEFAULT_SATURATION: f32 = 1.02;

fn is_image_like(path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "bmp")
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

#[derive(Debug, Clone, Copy)]
enum MotionProfile {
    Iris,
    Shake,
    Drift,
    Pulse,
}

#[derive(Debug, Clone)]
struct EffectLayerRef {
    texture_ref: String,
    profile: MotionProfile,
    alpha: f32,
    family: String,
    center_x: f32,
    center_y: f32,
    width: f32,
    height: f32,
    angle_rad: f32,
}

#[derive(Debug, Clone)]
struct EffectLayer {
    mask_image: PathBuf,
    profile: MotionProfile,
    alpha: f32,
    family: String,
    center_x: f32,
    center_y: f32,
    width: f32,
    height: f32,
    angle_rad: f32,
}

#[derive(Debug, Clone, Copy)]
struct VisualTuning {
    drift_amp_x: f32,
    drift_amp_y: f32,
    drift_freq_x: f32,
    drift_freq_y: f32,
    contrast: f32,
    saturation: f32,
    layer_alpha: f32,
}

impl Default for VisualTuning {
    fn default() -> Self {
        Self {
            drift_amp_x: 3.0,
            drift_amp_y: 2.0,
            drift_freq_x: 1.7,
            drift_freq_y: 1.4,
            contrast: DEFAULT_CONTRAST,
            saturation: DEFAULT_SATURATION,
            layer_alpha: EFFECT_LAYER_ALPHA,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RealtimeEffectPlan {
    pub inputs: Vec<PathBuf>,
    pub filter_complex: String,
    pub scene_width: u32,
    pub scene_height: u32,
    pub needs_audio_input: bool,
    pub audio_bars_overlay: Option<AudioBarsOverlay>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioBarsOverlay {
    pub center_x: i32,
    pub center_y: i32,
    pub width: u32,
    pub height: u32,
    pub angle_rad: f32,
    pub opacity: f32,
    pub transparency_mode: u8,
    pub scene_width: u32,
    pub scene_height: u32,
    pub center_x_norm: f32,
    pub center_y_norm: f32,
    pub width_norm: f32,
    pub height_norm: f32,
}

fn infer_motion_profile(effect_file: &str) -> MotionProfile {
    let f = effect_file.to_ascii_lowercase();
    if f.contains("iris") || f.contains("eye") || f.contains("blink") {
        MotionProfile::Iris
    } else if f.contains("shake") || f.contains("quake") || f.contains("jitter") {
        MotionProfile::Shake
    } else if f.contains("pulse") || f.contains("beat") || f.contains("audio") {
        MotionProfile::Pulse
    } else {
        MotionProfile::Drift
    }
}

fn parse_f32_value(v: &Value) -> Option<f32> {
    match v {
        Value::Number(n) => n.as_f64().map(|x| x as f32),
        Value::String(s) => s.parse::<f32>().ok(),
        _ => None,
    }
}

fn visual_tuning_from_graph(graph: &SceneGpuGraph) -> VisualTuning {
    let mut tuning = VisualTuning::default();
    let mut scroll_x = 0.0f32;
    let mut scroll_y = 0.0f32;
    let mut bright = 1.0f32;
    let mut power = 1.0f32;
    let mut alpha = 1.0f32;
    let mut found_scroll = false;

    for node in &graph.effect_nodes {
        for pass in &node.passes {
            if let Some(v) = pass.effective_uniforms.get("g_ScrollX").and_then(parse_f32_value) {
                scroll_x += v.abs();
                found_scroll = true;
            }
            if let Some(v) = pass.effective_uniforms.get("g_ScrollY").and_then(parse_f32_value) {
                scroll_y += v.abs();
                found_scroll = true;
            }
            if let Some(v) = pass
                .effective_uniforms
                .get("g_Brightness")
                .and_then(parse_f32_value)
            {
                bright = (bright + v).max(0.01);
            }
            if let Some(v) = pass.effective_uniforms.get("g_Power").and_then(parse_f32_value) {
                power = (power + v).max(0.01);
            }
            if let Some(v) = pass
                .effective_uniforms
                .get("g_UserAlpha")
                .and_then(parse_f32_value)
            {
                alpha = alpha.min(v.clamp(0.0, 1.0));
            }
        }
    }

    if found_scroll {
        tuning.drift_amp_x = (2.0 + scroll_x * 6.0).clamp(1.0, 10.0);
        tuning.drift_amp_y = (1.5 + scroll_y * 5.0).clamp(1.0, 8.0);
        tuning.drift_freq_x = (1.2 + scroll_x * 1.8).clamp(0.6, 6.0);
        tuning.drift_freq_y = (1.0 + scroll_y * 1.6).clamp(0.6, 5.5);
    }
    tuning.saturation = (1.0 + (bright - 1.0) * 0.14).clamp(0.70, 1.45);
    tuning.contrast = (1.0 + (power - 1.0) * 0.08).clamp(0.85, 1.35);
    tuning.layer_alpha = (alpha * 0.45).clamp(0.10, 0.65);
    tuning
}

fn collect_effect_layer_refs(scene: &Value, max_candidates: usize) -> Vec<EffectLayerRef> {
    let mut out = Vec::new();
    let mut seen = HashSet::<String>::new();
    let Some(objects) = scene.get("objects").and_then(|v| v.as_array()) else {
        return out;
    };
    for object in objects {
        let Some(effects) = object.get("effects").and_then(|v| v.as_array()) else {
            continue;
        };
        for effect in effects {
            let file = effect
                .get("file")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let profile = infer_motion_profile(file);
            let Some(passes) = effect.get("passes").and_then(|v| v.as_array()) else {
                continue;
            };
            for pass in passes {
                let Some(textures) = pass.get("textures").and_then(|v| v.as_array()) else {
                    continue;
                };
                for tex in textures {
                    if let Some(name) = tex.as_str() {
                        let trimmed = name.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        let key = trimmed.to_ascii_lowercase();
                        if seen.insert(key) {
                            out.push(EffectLayerRef {
                                texture_ref: trimmed.to_string(),
                                profile,
                                alpha: EFFECT_LAYER_ALPHA,
                                family: "legacy-effects".to_string(),
                                center_x: 0.0,
                                center_y: 0.0,
                                width: 0.0,
                                height: 0.0,
                                angle_rad: 0.0,
                            });
                            if out.len() >= max_candidates.max(1) {
                                return out;
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

fn collect_effect_layer_refs_from_native_plan(
    graph: &SceneGpuGraph,
    max_candidates: usize,
) -> Vec<EffectLayerRef> {
    let plan = build_native_runtime_plan(graph);
    let mut out = Vec::<EffectLayerRef>::new();
    let mut seen = HashSet::<String>::new();

    for layer in plan.draw_layers {
        if !matches!(layer.tier, NativeSupportTier::Ready) {
            continue;
        }
        let Some(texture_ref) = layer.primary_texture else {
            continue;
        };
        let key = texture_ref.to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }

        let profile = infer_motion_profile(&format!(
            "{} {}",
            layer.shader_family, layer.shader
        ));
        out.push(EffectLayerRef {
            texture_ref,
            profile,
            alpha: layer.alpha.clamp(0.05, 1.0),
            family: layer.shader_family,
            center_x: layer.center_x,
            center_y: layer.center_y,
            width: layer.width,
            height: layer.height,
            angle_rad: layer.angle_rad,
        });
        if out.len() >= max_candidates.max(1) {
            break;
        }
    }

    out
}

fn find_tex_entry_for_texture_ref(
    pkg: &crate::scene_pkg::ScenePkg,
    texture_ref: &str,
) -> Option<crate::scene_pkg::ScenePkgEntry> {
    let mut candidates = Vec::new();
    let mut needle = texture_ref.to_ascii_lowercase();
    if needle.ends_with(".tex") {
        needle = needle.trim_end_matches(".tex").to_string();
    }
    let needle_path = Path::new(&needle);
    let needle_base = needle_path
        .file_stem()
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or_else(|| needle.clone());

    for e in &pkg.entries {
        let f = e.filename.to_ascii_lowercase();
        if !f.ends_with(".tex") {
            continue;
        }
        if f.ends_with(&format!("{needle}.tex"))
            || f.contains(&format!("/{needle}.tex"))
            || f.ends_with(&format!("{needle_base}.tex"))
            || f.contains(&format!("/{needle_base}.tex"))
        {
            candidates.push(e.clone());
        }
    }
    candidates.into_iter().next()
}

fn filter_for_profile(profile: MotionProfile, src: &str, out: &str, tuning: &VisualTuning) -> String {
    match profile {
        MotionProfile::Iris => format!(
            "[{src}]crop=iw-10:ih-10:x='5+sin(t*2.9)*{:.3}':y='5+cos(t*2.5)*{:.3}',pad=iw+10:ih+10:5:5:color=black@0[{out}]",
            (tuning.drift_amp_x * 1.15).clamp(2.0, 12.0),
            (tuning.drift_amp_y * 1.10).clamp(1.5, 10.0)
        ),
        MotionProfile::Shake => format!(
            "[{src}]crop=iw-6:ih-6:x='3+sin(t*{:.3})*{:.3}':y='3+cos(t*{:.3})*{:.3}',pad=iw+6:ih+6:3:3:color=black@0[{out}]",
            (tuning.drift_freq_x * 2.7).clamp(3.0, 14.0),
            (tuning.drift_amp_x * 0.85).clamp(1.0, 6.0),
            (tuning.drift_freq_y * 2.8).clamp(3.0, 14.0),
            (tuning.drift_amp_y * 0.90).clamp(1.0, 6.0)
        ),
        MotionProfile::Pulse => format!(
            "[{src}]crop=iw-8:ih-8:x='4+sin(t*{:.3})*{:.3}':y='4+cos(t*{:.3})*{:.3}',pad=iw+8:ih+8:4:4:color=black@0[{out}]",
            (tuning.drift_freq_x * 2.0).clamp(1.8, 10.0),
            (tuning.drift_amp_x * 1.0).clamp(1.0, 8.0),
            (tuning.drift_freq_y * 2.1).clamp(1.8, 10.0),
            (tuning.drift_amp_y * 1.0).clamp(1.0, 7.0)
        ),
        MotionProfile::Drift => format!(
            "[{src}]crop=iw-8:ih-8:x='4+sin(t*{:.3})*{:.3}':y='4+cos(t*{:.3})*{:.3}',pad=iw+8:ih+8:4:4:color=black@0[{out}]",
            tuning.drift_freq_x,
            tuning.drift_amp_x,
            tuning.drift_freq_y,
            tuning.drift_amp_y
        ),
    }
}

fn parse_vec3(value: &str) -> Option<(f32, f32, f32)> {
    let mut it = value.split_whitespace();
    let x = it.next()?.parse::<f32>().ok()?;
    let y = it.next()?.parse::<f32>().ok()?;
    let z = it.next()?.parse::<f32>().ok()?;
    Some((x, y, z))
}

fn parse_vec2(value: &str) -> Option<(f32, f32)> {
    let mut it = value.split_whitespace();
    let x = it.next()?.parse::<f32>().ok()?;
    let y = it.next()?.parse::<f32>().ok()?;
    Some((x, y))
}

fn parse_scene_size(scene_json: &Value) -> (u32, u32) {
    let scene_w = scene_json
        .get("general")
        .and_then(|v| v.get("orthogonalprojection"))
        .and_then(|v| v.get("width"))
        .and_then(|v| v.as_u64())
        .unwrap_or(1920) as u32;
    let scene_h = scene_json
        .get("general")
        .and_then(|v| v.get("orthogonalprojection"))
        .and_then(|v| v.get("height"))
        .and_then(|v| v.as_u64())
        .unwrap_or(1080) as u32;
    (scene_w.max(1), scene_h.max(1))
}

fn detect_audio_bars_overlay(scene_json: &Value, scene_w: u32, scene_h: u32) -> Option<AudioBarsOverlay> {
    let objects = scene_json.get("objects")?.as_array()?;
    for object in objects {
        let effects = object.get("effects").and_then(|v| v.as_array())?;
        let mut has_audio_bars = false;
        let mut opacity = 1.0f32;
        let mut transparency_mode = 1u8;
        for effect in effects {
            let file = effect
                .get("file")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if file.contains("simple_audio_bars/effect.json") {
                has_audio_bars = true;
                if let Some(passes) = effect.get("passes").and_then(|v| v.as_array()) {
                    if let Some(first) = passes.first() {
                        if let Some(t) = first
                            .get("combos")
                            .and_then(|m| m.get("TRANSPARENCY"))
                            .and_then(|v| v.as_u64())
                        {
                            transparency_mode = (t as u8).min(5);
                        }
                        if let Some(v) = first
                            .get("constantshadervalues")
                            .and_then(|m| m.get("ui_editor_properties_opacity"))
                            .and_then(|v| v.as_f64())
                        {
                            opacity = (v as f32).clamp(0.0, 1.0);
                        }
                    }
                }
            }
        }
        if !has_audio_bars {
            continue;
        }

        let origin = object
            .get("origin")
            .and_then(|v| v.as_str())
            .and_then(parse_vec3)
            .unwrap_or((scene_w as f32 / 2.0, scene_h as f32 / 2.0, 0.0));
        let size = object
            .get("size")
            .and_then(|v| v.as_str())
            .and_then(parse_vec2)
            .unwrap_or((scene_w as f32, (scene_h as f32 * 0.25).max(1.0)));
        let scale = object
            .get("scale")
            .and_then(|v| v.as_str())
            .and_then(parse_vec3)
            .unwrap_or((1.0, 1.0, 1.0));
        let angle = object
            .get("angles")
            .and_then(|v| v.as_str())
            .and_then(parse_vec3)
            .map(|(_, _, z)| z)
            .unwrap_or(0.0);

        let raw_w = (size.0 * scale.0.abs()).max(64.0);
        let s = scale.1.abs().clamp(0.12, 3.0);
        let mut w = (size.0 * s).max(64.0).min(scene_w as f32) as u32;
        let mut h = (size.1 * s).max(24.0).min(scene_h as f32 * 0.55) as u32;
        if raw_w >= scene_w as f32 * 0.72 {
            w = scene_w;
        }
        if w < 160 {
            w = 160;
        }
        if h < 48 {
            h = 48;
        }

        let mut center_x = origin.0.round() as i32;
        // Scene coordinates are bottom-origin; convert to top-origin center.
        let mut center_y = (scene_h as f32 - origin.1).round() as i32;
        center_x = center_x.clamp(0, scene_w as i32);
        center_y = center_y.clamp(0, scene_h as i32);

        let scene_wf = scene_w.max(1) as f32;
        let scene_hf = scene_h.max(1) as f32;
        return Some(AudioBarsOverlay {
            center_x,
            center_y,
            width: w.max(16),
            height: h.max(16),
            angle_rad: angle,
            opacity,
            transparency_mode,
            scene_width: scene_w.max(1),
            scene_height: scene_h.max(1),
            center_x_norm: (center_x as f32 / scene_wf).clamp(0.0, 1.0),
            center_y_norm: (center_y as f32 / scene_hf).clamp(0.0, 1.0),
            width_norm: (w.max(16) as f32 / scene_wf).clamp(0.0, 1.0),
            height_norm: (h.max(16) as f32 / scene_hf).clamp(0.0, 1.0),
        });
    }
    None
}

fn build_masked_animated_proxy(
    base_image: &Path,
    layers: &[EffectLayer],
    scene_w: u32,
    scene_h: u32,
    out: &Path,
    dry_run: bool,
) -> Result<PathBuf> {
    if layers.is_empty() {
        return build_simple_animated_proxy(base_image, out, dry_run);
    }

    let tuning = VisualTuning::default();
    let filter = build_masked_filter(layers, scene_w, scene_h, None, &tuning);

    if dry_run {
        let mut args = format!(
            "[dry-run] ffmpeg -hide_banner -loglevel error -y -loop 1 -i '{}'",
            base_image.display()
        );
        for layer in layers {
            args.push_str(&format!(" -loop 1 -i '{}'", layer.mask_image.display()));
        }
        args.push_str(&format!(
            " -filter_complex \"{}\" -map '[v]' -t 20 -r 60 -c:v libx264 -preset veryfast -crf 20 '{}'",
            filter,
            out.display()
        ));
        println!("{}", args);
        return Ok(out.to_path_buf());
    }

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed creating animated proxy dir {}", parent.display()))?;
    }

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-loop")
        .arg("1")
        .arg("-i")
        .arg(base_image);
    for layer in layers {
        cmd.arg("-loop").arg("1").arg("-i").arg(&layer.mask_image);
    }
    let output = cmd
        .arg("-filter_complex")
        .arg(filter)
        .arg("-map")
        .arg("[v]")
        .arg("-t")
        .arg("20")
        .arg("-r")
        .arg("60")
        .arg("-c:v")
        .arg("libx264")
        .arg("-preset")
        .arg("veryfast")
        .arg("-crf")
        .arg("20")
        .arg(out)
        .output()
        .context("Failed running ffmpeg for masked animated proxy")?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ffmpeg masked animated proxy failed: {}", err.trim());
    }

    Ok(out.to_path_buf())
}

fn build_masked_filter(
    layers: &[EffectLayer],
    scene_w: u32,
    scene_h: u32,
    audio_bars: Option<&AudioBarsOverlay>,
    tuning: &VisualTuning,
) -> String {
    let scene_w = scene_w.max(1);
    let scene_h = scene_h.max(1);
    let mut filter = String::new();
    let mut split_tags = Vec::new();
    for i in 0..=layers.len() {
        split_tags.push(format!("[s{}]", i));
    }
    filter.push_str(&format!(
        "[0:v]format=rgba,scale={}:{}:force_original_aspect_ratio=increase,crop={}:{},split={}{};",
        scene_w,
        scene_h,
        scene_w,
        scene_h,
        layers.len() + 1,
        split_tags.join("")
    ));

    for (i, layer) in layers.iter().enumerate() {
        let src = format!("s{}", i + 1);
        let moved = format!("m{}", i);
        let mask_scaled = format!("ms{}", i);
        let mask = format!("k{}", i);
        let mask_rot = format!("kr{}", i);
        let mask_canvas = format!("kc{}", i);
        let mask_placed = format!("kp{}", i);
        let masked = format!("a{}", i);
        let in_comp = format!("c{}", i);
        let out_comp = format!("c{}", i + 1);
        if i == 0 {
            filter.push_str("[s0]copy[c0];");
        }
        filter.push_str(&filter_for_profile(layer.profile, &src, &moved, tuning));
        filter.push(';');
        let layer_w = layer.width.max(8.0).min(scene_w as f32 * 2.0).round() as u32;
        let layer_h = layer.height.max(8.0).min(scene_h as f32 * 2.0).round() as u32;
        let center_x = if layer.center_x <= 0.0 {
            scene_w as f32 * 0.5
        } else {
            layer.center_x.clamp(0.0, scene_w as f32)
        };
        let center_y = if layer.center_y <= 0.0 {
            scene_h as f32 * 0.5
        } else {
            layer.center_y.clamp(0.0, scene_h as f32)
        };
        filter.push_str(&format!(
            "[{}:v]scale={}:{}:flags=bicubic[{}];",
            i + 1,
            layer_w,
            layer_h,
            mask_scaled
        ));
        filter.push_str(&format!("[{}]format=gray,boxblur=2:1[{}];", mask_scaled, mask));
        if layer.angle_rad.abs() > 0.001 {
            filter.push_str(&format!(
                "[{}]rotate={:.6}:c=black:ow=rotw(iw):oh=roth(ih)[{}];",
                mask, -layer.angle_rad, mask_rot
            ));
        } else {
            filter.push_str(&format!("[{}]copy[{}];", mask, mask_rot));
        }
        filter.push_str(&format!(
            "color=c=black:s={}x{}:d=1,format=gray[{}];[{}][{}]overlay='{}-(overlay_w/2)':'{}-(overlay_h/2)':format=auto[{}];",
            scene_w, scene_h, mask_canvas, mask_canvas, mask_rot, center_x, center_y, mask_placed
        ));
        filter.push_str(&format!(
            "[{}][{}]alphamerge[tmp{}];[tmp{}]format=rgba,colorchannelmixer=aa={:.3}[{}];",
            moved,
            mask_placed,
            i,
            i,
            (tuning.layer_alpha * layer.alpha).clamp(0.03, 0.95),
            masked
        ));
        filter.push_str(&format!(
            "[{}][{}]overlay=0:0:format=auto[{}];",
            in_comp, masked, out_comp
        ));
    }
    let final_comp = format!("c{}", layers.len());
    if let Some(bars) = audio_bars {
        let aidx = layers.len() + 1;
        let opacity = bars.opacity.clamp(0.0, 1.0);
        let scene_w = scene_w.max(1);
        let scene_h = scene_h.max(1);
        let bars_x = bars.center_x;
        let bars_y = bars.center_y;
        filter.push_str(&format!(
            "[{}:a]aformat=channel_layouts=stereo,showfreqs=s={}x{}:mode=bar:ascale=sqrt:fscale=lin:colors=White,format=rgba,colorchannelmixer=aa={:.3}",
            aidx,
            bars.width,
            bars.height,
            opacity
        ));
        if bars.angle_rad.abs() > 0.001 {
            filter.push_str(&format!(
                ",rotate={:.6}:c=none:ow=rotw(iw):oh=roth(ih)[ab];",
                -bars.angle_rad
            ));
        } else {
            filter.push_str("[ab];");
        }
        filter.push_str(&format!(
            "color=c=black@0.0:s={}x{}:d=1,format=rgba[abcanvas];[abcanvas][ab]overlay='{}-(overlay_w/2)':'{}-(overlay_h/2)':format=auto[abp];",
            scene_w,
            scene_h,
            bars_x,
            bars_y
        ));
        match bars.transparency_mode {
            2 => filter.push_str(&format!(
                "[{}][abp]blend=all_mode=addition:all_opacity=1[cv];",
                final_comp
            )),
            3 => filter.push_str(&format!(
                "[{}][abp]blend=all_mode=subtract:all_opacity=1[cv];",
                final_comp
            )),
            4 => filter.push_str(&format!(
                "[{}][abp]blend=all_mode=multiply:all_opacity=1[cv];",
                final_comp
            )),
            _ => filter.push_str(&format!(
                "[{}][abp]overlay=0:0:format=auto[cv];",
                final_comp
            )),
        }
        filter.push_str(&format!(
            "[cv]eq=contrast={:.3}:saturation={:.3},format=yuv420p[v]",
            tuning.contrast, tuning.saturation
        ));
    } else {
        filter.push_str(&format!(
            "[{}]eq=contrast={:.3}:saturation={:.3},format=yuv420p[v]",
            final_comp, tuning.contrast, tuning.saturation
        ));
    }
    filter
}

fn build_simple_filter(
    audio_bars: Option<&AudioBarsOverlay>,
    scene_w: u32,
    scene_h: u32,
    tuning: &VisualTuning,
) -> String {
    if let Some(bars) = audio_bars {
        let opacity = bars.opacity.clamp(0.0, 1.0);
        let scene_w = scene_w.max(1);
        let scene_h = scene_h.max(1);
        let bars_x = bars.center_x;
        let bars_y = bars.center_y;
        let mut f = format!(
            "[0:v]crop=iw-8:ih-8:x='4+sin(t*{:.3})*{:.3}':y='4+cos(t*{:.3})*{:.3}',pad=iw+8:ih+8:4:4:color=black[base];",
            tuning.drift_freq_x, tuning.drift_amp_x, tuning.drift_freq_y, tuning.drift_amp_y
        )
        .to_string();
        f.push_str(&format!(
            "[1:a]aformat=channel_layouts=stereo,showfreqs=s={}x{}:mode=bar:ascale=sqrt:fscale=lin:colors=White,format=rgba,colorchannelmixer=aa={:.3}",
            bars.width, bars.height, opacity
        ));
        if bars.angle_rad.abs() > 0.001 {
            f.push_str(&format!(
                ",rotate={:.6}:c=none:ow=rotw(iw):oh=roth(ih)[ab];",
                -bars.angle_rad
            ));
        } else {
            f.push_str("[ab];");
        }
        f.push_str(&format!(
            "color=c=black@0.0:s={}x{}:d=1,format=rgba[abcanvas];[abcanvas][ab]overlay='{}-(overlay_w/2)':'{}-(overlay_h/2)':format=auto[abp];",
            scene_w,
            scene_h,
            bars_x,
            bars_y
        ));
        match bars.transparency_mode {
            2 => f.push_str("[base][abp]blend=all_mode=addition:all_opacity=1[mix];"),
            3 => f.push_str("[base][abp]blend=all_mode=subtract:all_opacity=1[mix];"),
            4 => f.push_str("[base][abp]blend=all_mode=multiply:all_opacity=1[mix];"),
            _ => f.push_str("[base][abp]overlay=0:0:format=auto[mix];"),
        }
        f.push_str(&format!(
            " [mix]eq=contrast={:.3}:saturation={:.3},format=yuv420p[v]",
            tuning.contrast, tuning.saturation
        ));
        return f;
    }
    format!(
        "[0:v]crop=iw-8:ih-8:x='4+sin(t*{:.3})*{:.3}':y='4+cos(t*{:.3})*{:.3}',pad=iw+8:ih+8:4:4:color=black,eq=contrast={:.3}:saturation={:.3},format=yuv420p[v]",
        tuning.drift_freq_x,
        tuning.drift_amp_x,
        tuning.drift_freq_y,
        tuning.drift_amp_y,
        tuning.contrast,
        tuning.saturation
    )
}

pub fn build_scene_realtime_effect_plan(
    root: &Path,
    session_dir: &Path,
    entry: &Path,
) -> Result<Option<RealtimeEffectPlan>> {
    if !is_image_like(entry) {
        return Ok(None);
    }

    let Some(pkg_path) = pick_pkg_path(root) else {
        return Ok(None);
    };
    let pkg = parse_scene_pkg(&pkg_path)?;
    let scene_entry =
        find_entry(&pkg, "scene.json").or_else(|| find_entry(&pkg, "gifscene.json"));
    let Some(scene_entry) = scene_entry else {
        return Ok(None);
    };
    let scene_json: Value = serde_json::from_slice(&read_entry_bytes(&pkg, &scene_entry)?)?;
    let (scene_w, scene_h) = parse_scene_size(&scene_json);
    let audio_bars = detect_audio_bars_overlay(&scene_json, scene_w, scene_h);
    let graph = build_scene_gpu_graph(root).ok();
    let tuning = graph
        .as_ref()
        .map(visual_tuning_from_graph)
        .unwrap_or_default();

    let masks_src = session_dir.join("effect-proxy/masks-src");
    let masks_proxy = session_dir.join("effect-proxy/masks-proxy");

    let mut layers = Vec::<EffectLayer>::new();
    let layer_refs = graph
        .as_ref()
        .map(|g| collect_effect_layer_refs_from_native_plan(g, 24))
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| collect_effect_layer_refs(&scene_json, 24));

    for layer_ref in layer_refs {
        let Some(tex_entry) = find_tex_entry_for_texture_ref(&pkg, &layer_ref.texture_ref) else {
            continue;
        };
        let Some(mask_image) = extract_entry_to_cache(&pkg, &tex_entry, &masks_src)
            .ok()
            .and_then(|tex_path| extract_playable_proxy_from_tex(&tex_path, &masks_proxy).ok())
            .flatten()
        else {
            continue;
        };
        layers.push(EffectLayer {
            mask_image,
            profile: layer_ref.profile,
            alpha: layer_ref.alpha,
            family: layer_ref.family.clone(),
            center_x: layer_ref.center_x,
            center_y: layer_ref.center_y,
            width: layer_ref.width,
            height: layer_ref.height,
            angle_rad: layer_ref.angle_rad,
        });
        if layers.len() >= EFFECT_LAYER_LIMIT {
            break;
        }
    }

    let mut inputs = vec![entry.to_path_buf()];
    let filter_complex = if layers.is_empty() {
        // Audio bars are intentionally not burned into mp4/native ffmpeg output anymore.
        build_simple_filter(None, scene_w, scene_h, &tuning)
    } else {
        for layer in &layers {
            inputs.push(layer.mask_image.clone());
        }
        if !layers.is_empty() {
            eprintln!(
                "[ok] native families in realtime plan: {:?}",
                layers
                    .iter()
                    .map(|l| l.family.clone())
                    .collect::<Vec<_>>()
            );
        }
        build_masked_filter(&layers, scene_w, scene_h, None, &tuning)
    };

    Ok(Some(RealtimeEffectPlan {
        inputs,
        filter_complex,
        scene_width: scene_w.max(1),
        scene_height: scene_h.max(1),
        needs_audio_input: false,
        audio_bars_overlay: audio_bars,
    }))
}

pub fn build_scene_audio_bars_overlay(root: &Path) -> Result<Option<AudioBarsOverlay>> {
    let Some(pkg_path) = pick_pkg_path(root) else {
        return Ok(None);
    };
    let pkg = parse_scene_pkg(&pkg_path)?;
    let scene_entry =
        find_entry(&pkg, "scene.json").or_else(|| find_entry(&pkg, "gifscene.json"));
    let Some(scene_entry) = scene_entry else {
        return Ok(None);
    };
    let scene_json: Value = serde_json::from_slice(&read_entry_bytes(&pkg, &scene_entry)?)?;
    let (scene_w, scene_h) = parse_scene_size(&scene_json);
    Ok(detect_audio_bars_overlay(&scene_json, scene_w, scene_h))
}

fn build_simple_animated_proxy(base_image: &Path, out: &Path, dry_run: bool) -> Result<PathBuf> {
    let filter = concat!(
        "[0:v]crop=iw-8:ih-8:x='4+sin(t*1.7)*3':y='4+cos(t*1.4)*2',",
        "pad=iw+8:ih+8:4:4:color=black,",
        "eq=contrast=1.01:saturation=1.02,format=yuv420p[v]"
    );

    if dry_run {
        println!(
            "[dry-run] ffmpeg -hide_banner -loglevel error -y -loop 1 -i '{}' -filter_complex \"{}\" -map '[v]' -t 20 -r 60 -c:v libx264 -preset veryfast -crf 21 '{}'",
            base_image.display(),
            filter,
            out.display()
        );
        return Ok(out.to_path_buf());
    }

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed creating animated proxy dir {}", parent.display()))?;
    }

    let output = Command::new("ffmpeg")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-loop")
        .arg("1")
        .arg("-i")
        .arg(base_image)
        .arg("-filter_complex")
        .arg(filter)
        .arg("-map")
        .arg("[v]")
        .arg("-t")
        .arg("20")
        .arg("-r")
        .arg("60")
        .arg("-c:v")
        .arg("libx264")
        .arg("-preset")
        .arg("veryfast")
        .arg("-crf")
        .arg("21")
        .arg(out)
        .output()
        .context("Failed running ffmpeg for simple animated proxy")?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ffmpeg simple animated proxy failed: {}", err.trim());
    }

    Ok(out.to_path_buf())
}

pub fn maybe_build_scene_animated_proxy(
    root: &Path,
    session_dir: &Path,
    entry: &Path,
    dry_run: bool,
) -> Result<Option<PathBuf>> {
    if !is_image_like(entry) {
        return Ok(None);
    }

    let Some(pkg_path) = pick_pkg_path(root) else {
        return Ok(None);
    };
    let pkg = parse_scene_pkg(&pkg_path)?;
    let scene_entry =
        find_entry(&pkg, "scene.json").or_else(|| find_entry(&pkg, "gifscene.json"));
    let Some(scene_entry) = scene_entry else {
        return Ok(None);
    };
    let scene_json: Value = serde_json::from_slice(&read_entry_bytes(&pkg, &scene_entry)?)?;
    let scene_w = scene_json
        .get("general")
        .and_then(|v| v.get("orthogonalprojection"))
        .and_then(|v| v.get("width"))
        .and_then(|v| v.as_u64())
        .unwrap_or(1920) as u32;
    let scene_h = scene_json
        .get("general")
        .and_then(|v| v.get("orthogonalprojection"))
        .and_then(|v| v.get("height"))
        .and_then(|v| v.as_u64())
        .unwrap_or(1080) as u32;

    let masks_src = session_dir.join("effect-proxy/masks-src");
    let masks_proxy = session_dir.join("effect-proxy/masks-proxy");
    let out_proxy = session_dir.join("effect-proxy/scene_animated_proxy.mp4");

    let mut layers = Vec::<EffectLayer>::new();
    for layer_ref in collect_effect_layer_refs(&scene_json, 24) {
        let Some(tex_entry) = find_tex_entry_for_texture_ref(&pkg, &layer_ref.texture_ref) else {
            continue;
        };
        let Some(mask_image) = extract_entry_to_cache(&pkg, &tex_entry, &masks_src)
            .ok()
            .and_then(|tex_path| extract_playable_proxy_from_tex(&tex_path, &masks_proxy).ok())
            .flatten()
        else {
            continue;
        };
        layers.push(EffectLayer {
            mask_image,
            profile: layer_ref.profile,
            alpha: layer_ref.alpha,
            family: layer_ref.family.clone(),
            center_x: layer_ref.center_x,
            center_y: layer_ref.center_y,
            width: layer_ref.width,
            height: layer_ref.height,
            angle_rad: layer_ref.angle_rad,
        });
        if layers.len() >= EFFECT_LAYER_LIMIT {
            break;
        }
    }

    if layers.is_empty() {
        eprintln!("[warn] scene effect proxy using procedural fallback (no effect masks)");
    } else {
        eprintln!("[ok] scene effect proxy using {} effect mask layer(s)", layers.len());
    }
    let built = build_masked_animated_proxy(entry, &layers, scene_w, scene_h, &out_proxy, dry_run)?;
    Ok(Some(built))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_gpu_graph::{GpuEffectNode, GpuPassSpec, SceneGpuGraph};
    use serde_json::json;

    fn base_graph_with_uniforms() -> SceneGpuGraph {
        SceneGpuGraph {
            pkg_path: String::new(),
            scene_json_entry: "scene.json".to_string(),
            scene_width: 1920,
            scene_height: 1080,
            global_assets_root: None,
            user_properties: Value::Null,
            script_properties: Value::Null,
            script_assignments: Vec::new(),
            effect_nodes: vec![GpuEffectNode {
                object_id: 1,
                object_name: "obj".to_string(),
                object_kind: "image".to_string(),
                object_asset: Some("models/a.json".to_string()),
                object_origin: Some([960.0, 540.0, 0.0]),
                object_scale: Some([1.0, 1.0, 1.0]),
                object_angles: Some([0.0, 0.0, 0.0]),
                object_size: Some([1920.0, 1080.0]),
                object_asset_size: Some([1920.0, 1080.0]),
                object_parallax_depth: Some([1.0, 1.0]),
                object_visible: true,
                effect_file: "materials/a.json".to_string(),
                effect_name: "genericimage".to_string(),
                material_asset: Some("materials/a.json".to_string()),
                pass_shader: "genericimage".to_string(),
                pass_index: 0,
                passes: vec![GpuPassSpec {
                    pass_index: 0,
                    shader: "genericimage".to_string(),
                    combos: Value::Null,
                    shader_defines: Vec::new(),
                    blending: Some("normal".to_string()),
                    depth_test: Some("disabled".to_string()),
                    depth_write: Some("disabled".to_string()),
                    cull_mode: Some("nocull".to_string()),
                    constant_shader_values: Value::Null,
                    user_shader_values: Value::Null,
                    textures: vec!["materials/mask_a.tex".to_string()],
                    texture_refs: vec!["mask_a".to_string()],
                    effective_uniforms: [
                        ("g_ScrollX".to_string(), json!(0.5)),
                        ("g_ScrollY".to_string(), json!(0.25)),
                        ("g_Brightness".to_string(), json!(1.4)),
                        ("g_Power".to_string(), json!(1.6)),
                        ("g_UserAlpha".to_string(), json!(0.4)),
                    ]
                    .into_iter()
                    .collect(),
                }],
                shader_vert: None,
                shader_frag: None,
                material_json: Some("materials/a.json".to_string()),
                uniform_bindings: Vec::new(),
            }],
            notes: Vec::new(),
        }
    }

    #[test]
    fn tuning_uses_effective_uniforms() {
        let graph = base_graph_with_uniforms();
        let tuning = visual_tuning_from_graph(&graph);
        assert!(tuning.drift_amp_x > 3.0);
        assert!(tuning.drift_amp_y > 2.0);
        assert!(tuning.saturation > DEFAULT_SATURATION);
        assert!(tuning.contrast > DEFAULT_CONTRAST);
        assert!(tuning.layer_alpha < EFFECT_LAYER_ALPHA);
    }

    #[test]
    fn layer_refs_can_come_from_graph() {
        let graph = base_graph_with_uniforms();
        let refs = collect_effect_layer_refs_from_native_plan(&graph, 8);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].texture_ref, "materials/mask_a.tex");
        assert_eq!(refs[0].family, "genericimage");
        assert!((refs[0].alpha - 0.4).abs() < 0.0001);
    }
}
