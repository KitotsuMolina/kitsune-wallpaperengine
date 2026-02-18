use crate::scene_pkg::{extract_entry_to_cache, find_entry, parse_scene_pkg, read_entry_bytes};
use crate::tex_payload::extract_playable_proxy_from_tex;
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

const EFFECT_LAYER_LIMIT: usize = 1;
const EFFECT_LAYER_ALPHA: f32 = 0.35;

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
}

#[derive(Debug, Clone)]
struct EffectLayer {
    mask_image: PathBuf,
    profile: MotionProfile,
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

fn find_tex_entry_for_texture_ref(
    pkg: &crate::scene_pkg::ScenePkg,
    texture_ref: &str,
) -> Option<crate::scene_pkg::ScenePkgEntry> {
    let mut candidates = Vec::new();
    let needle = texture_ref.to_ascii_lowercase();
    for e in &pkg.entries {
        let f = e.filename.to_ascii_lowercase();
        if !f.ends_with(".tex") {
            continue;
        }
        if f.ends_with(&format!("{needle}.tex")) || f.contains(&format!("/{needle}.tex")) {
            candidates.push(e.clone());
        }
    }
    candidates.into_iter().next()
}

fn filter_for_profile(profile: MotionProfile, src: &str, out: &str) -> String {
    match profile {
        MotionProfile::Iris => format!(
            "[{src}]crop=iw-10:ih-10:x='5+sin(t*2.9)*4':y='5+cos(t*2.5)*3',pad=iw+10:ih+10:5:5:color=black@0[{out}]"
        ),
        MotionProfile::Shake => format!(
            "[{src}]crop=iw-6:ih-6:x='3+sin(t*5.3)*2':y='3+cos(t*4.7)*2',pad=iw+6:ih+6:3:3:color=black@0[{out}]"
        ),
        MotionProfile::Pulse => format!(
            "[{src}]crop=iw-8:ih-8:x='4+sin(t*3.4)*3':y='4+cos(t*3.0)*2',pad=iw+8:ih+8:4:4:color=black@0[{out}]"
        ),
        MotionProfile::Drift => format!(
            "[{src}]crop=iw-8:ih-8:x='4+sin(t*1.7)*3':y='4+cos(t*1.4)*2',pad=iw+8:ih+8:4:4:color=black@0[{out}]"
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

    let filter = build_masked_filter(layers, scene_w, scene_h, None);

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
        let masked = format!("a{}", i);
        let in_comp = format!("c{}", i);
        let out_comp = format!("c{}", i + 1);
        if i == 0 {
            filter.push_str("[s0]copy[c0];");
        }
        filter.push_str(&filter_for_profile(layer.profile, &src, &moved));
        filter.push(';');
        filter.push_str(&format!(
            "[{}:v]scale={}:{}:flags=bicubic[{}];",
            i + 1,
            scene_w,
            scene_h,
            mask_scaled
        ));
        filter.push_str(&format!("[{}]format=gray,boxblur=2:1[{}];", mask_scaled, mask));
        filter.push_str(&format!(
            "[{}][{}]alphamerge[tmp{}];[tmp{}]format=rgba,colorchannelmixer=aa={:.3}[{}];",
            moved, mask, i, i, EFFECT_LAYER_ALPHA, masked
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
        filter.push_str("[cv]eq=contrast=1.01:saturation=1.02,format=yuv420p[v]");
    } else {
        filter.push_str(&format!(
            "[{}]eq=contrast=1.01:saturation=1.02,format=yuv420p[v]",
            final_comp
        ));
    }
    filter
}

fn build_simple_filter(
    audio_bars: Option<&AudioBarsOverlay>,
    scene_w: u32,
    scene_h: u32,
) -> String {
    if let Some(bars) = audio_bars {
        let opacity = bars.opacity.clamp(0.0, 1.0);
        let scene_w = scene_w.max(1);
        let scene_h = scene_h.max(1);
        let bars_x = bars.center_x;
        let bars_y = bars.center_y;
        let mut f = concat!(
            "[0:v]crop=iw-8:ih-8:x='4+sin(t*1.7)*3':y='4+cos(t*1.4)*2',",
            "pad=iw+8:ih+8:4:4:color=black[base];"
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
        f.push_str(" [mix]eq=contrast=1.01:saturation=1.02,format=yuv420p[v]");
        return f;
    }
    concat!(
        "[0:v]crop=iw-8:ih-8:x='4+sin(t*1.7)*3':y='4+cos(t*1.4)*2',",
        "pad=iw+8:ih+8:4:4:color=black,",
        "eq=contrast=1.01:saturation=1.02,format=yuv420p[v]"
    )
    .to_string()
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

    let masks_src = session_dir.join("effect-proxy/masks-src");
    let masks_proxy = session_dir.join("effect-proxy/masks-proxy");

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
        });
        if layers.len() >= EFFECT_LAYER_LIMIT {
            break;
        }
    }

    let mut inputs = vec![entry.to_path_buf()];
    let filter_complex = if layers.is_empty() {
        // Audio bars are intentionally not burned into mp4/native ffmpeg output anymore.
        build_simple_filter(None, scene_w, scene_h)
    } else {
        for layer in &layers {
            inputs.push(layer.mask_image.clone());
        }
        build_masked_filter(&layers, scene_w, scene_h, None)
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
