use crate::asset_resolver::AssetResolver;
use crate::scene_native_runtime::{NativeRuntimePlan, NativeSupportTier};
use crate::tex_payload::extract_playable_proxy_from_tex;
use anyhow::{Context, Result};
use image::imageops::FilterType;
use image::{Rgba, RgbaImage};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
pub struct NativeLayerResult {
    pub object_id: u64,
    pub object_name: String,
    pub texture_ref: String,
    pub blend_mode: String,
    pub alpha: f32,
    pub loaded: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeStaticRenderReport {
    pub output_image: String,
    pub canvas_width: u32,
    pub canvas_height: u32,
    pub total_ready_layers: usize,
    pub rendered_layers: usize,
    pub layers: Vec<NativeLayerResult>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeAnimatedRenderReport {
    pub output_video: String,
    pub report_path: String,
    pub canvas_width: u32,
    pub canvas_height: u32,
    pub seconds: u64,
    pub fps: u32,
    pub total_ready_layers: usize,
    pub rendered_layers: usize,
    pub layers: Vec<NativeLayerResult>,
    pub notes: Vec<String>,
}

fn lower_ext(path: &str) -> String {
    Path::new(path)
        .extension()
        .map(|v| v.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn blend(dst: &mut Rgba<u8>, src: Rgba<u8>, alpha: f32, mode: &str, brightness: f32, tint: [f32; 3]) {
    let mut src_tinted = src;
    src_tinted[0] = ((src_tinted[0] as f32) * brightness * tint[0].clamp(0.0, 2.0))
        .clamp(0.0, 255.0) as u8;
    src_tinted[1] = ((src_tinted[1] as f32) * brightness * tint[1].clamp(0.0, 2.0))
        .clamp(0.0, 255.0) as u8;
    src_tinted[2] = ((src_tinted[2] as f32) * brightness * tint[2].clamp(0.0, 2.0))
        .clamp(0.0, 255.0) as u8;
    let a = (src_tinted[3] as f32 / 255.0) * alpha.clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    match mode {
        "additive" => {
            for c in 0..3 {
                let v = dst[c] as f32 + src_tinted[c] as f32 * a;
                dst[c] = v.clamp(0.0, 255.0) as u8;
            }
            dst[3] = 255;
        }
        "multiply" => {
            for c in 0..3 {
                let src_norm = src_tinted[c] as f32 / 255.0;
                let d = dst[c] as f32 / 255.0;
                let out = d * ((1.0 - a) + src_norm * a);
                dst[c] = (out * 255.0).clamp(0.0, 255.0) as u8;
            }
            dst[3] = 255;
        }
        _ => {
            for c in 0..3 {
                let d = dst[c] as f32;
                let src_chan = src_tinted[c] as f32;
                dst[c] = (src_chan * a + d * (1.0 - a)).clamp(0.0, 255.0) as u8;
            }
            dst[3] = 255;
        }
    }
}

fn decode_layer_png(bytes: &[u8]) -> Option<RgbaImage> {
    let dyn_img = image::load_from_memory(bytes).ok()?;
    Some(dyn_img.to_rgba8())
}

fn resolve_layer_image(
    resolver: &AssetResolver,
    texture_ref: &str,
    scratch_dir: &Path,
) -> Result<Option<Vec<u8>>> {
    let asset = match resolver.resolve(texture_ref) {
        Some(a) => a,
        None => return Ok(None),
    };

    let ext = lower_ext(&asset.resolved_path);
    if ext == "png" {
        return Ok(Some(asset.bytes));
    }

    if ext == "tex" {
        fs::create_dir_all(scratch_dir)
            .with_context(|| format!("Failed creating {}", scratch_dir.display()))?;
        let tex_file = scratch_dir.join(
            Path::new(&asset.resolved_path)
                .file_name()
                .map(|v| v.to_string_lossy().to_string())
                .unwrap_or_else(|| "layer.tex".to_string()),
        );
        fs::write(&tex_file, &asset.bytes)
            .with_context(|| format!("Failed writing {}", tex_file.display()))?;

        let proxy_dir = scratch_dir.join("proxy");
        let proxy = extract_playable_proxy_from_tex(&tex_file, &proxy_dir)?;
        let Some(proxy_path) = proxy else {
            return Ok(None);
        };
        let proxy_ext = proxy_path
            .extension()
            .map(|v| v.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if proxy_ext != "png" {
            return Ok(None);
        }
        let png = fs::read(&proxy_path)
            .with_context(|| format!("Failed reading {}", proxy_path.display()))?;
        return Ok(Some(png));
    }

    Ok(None)
}

fn layer_motion(uniforms: &std::collections::BTreeMap<String, serde_json::Value>, idx: usize) -> (f32, f32, f32, f32) {
    let sx = uniforms
        .get("g_ScrollX")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    let sy = uniforms
        .get("g_ScrollY")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    let fx = (1.1 + sx.abs() * 2.2 + idx as f32 * 0.09).clamp(0.7, 6.0);
    let fy = (0.9 + sy.abs() * 2.0 + idx as f32 * 0.07).clamp(0.7, 6.0);
    let ax = (2.0 + sx.abs() * 7.0 + idx as f32 * 0.2).clamp(1.0, 14.0);
    let ay = (1.5 + sy.abs() * 6.0 + idx as f32 * 0.16).clamp(1.0, 12.0);
    (fx, fy, ax, ay)
}

pub fn render_native_static_frame(
    root: &Path,
    session_dir: &Path,
    canvas_width: u32,
    canvas_height: u32,
    plan: &NativeRuntimePlan,
) -> Result<Option<NativeStaticRenderReport>> {
    let ready_layers: Vec<_> = plan
        .draw_layers
        .iter()
        .filter(|l| matches!(l.tier, NativeSupportTier::Ready) && l.primary_texture.is_some())
        .cloned()
        .collect();

    if ready_layers.is_empty() {
        return Ok(None);
    }

    let resolver = AssetResolver::new(root)?;
    let out_dir = session_dir.join("native-render");
    fs::create_dir_all(&out_dir).with_context(|| format!("Failed creating {}", out_dir.display()))?;
    let scratch = out_dir.join("scratch");

    let width = canvas_width.max(1);
    let height = canvas_height.max(1);
    let mut canvas = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255]));

    let mut results = Vec::<NativeLayerResult>::new();
    let mut rendered = 0usize;

    for layer in ready_layers {
        let texture_ref = layer.primary_texture.clone().unwrap_or_default();
        let mut record = NativeLayerResult {
            object_id: layer.object_id,
            object_name: layer.object_name.clone(),
            texture_ref: texture_ref.clone(),
            blend_mode: layer.blend_mode.clone(),
            alpha: layer.alpha,
            loaded: false,
            reason: None,
        };

        let bytes = resolve_layer_image(&resolver, &texture_ref, &scratch)?;
        let Some(bytes) = bytes else {
            record.reason = Some("texture unresolved or unsupported format (expects png/tex->png)".to_string());
            results.push(record);
            continue;
        };

        let Some(img) = decode_layer_png(&bytes) else {
            record.reason = Some("failed to decode image bytes as PNG".to_string());
            results.push(record);
            continue;
        };

        let layer_w = layer.width.max(8.0).min(width as f32 * 2.0).round() as u32;
        let layer_h = layer.height.max(8.0).min(height as f32 * 2.0).round() as u32;
        let scaled = image::imageops::resize(&img, layer_w, layer_h, FilterType::Triangle);
        let x0 = (layer.center_x - layer_w as f32 / 2.0).round() as i32;
        let y0 = (layer.center_y - layer_h as f32 / 2.0).round() as i32;
        for y in 0..layer_h {
            for x in 0..layer_w {
                let dst_x = x0 + x as i32;
                let dst_y = y0 + y as i32;
                if dst_x < 0 || dst_y < 0 || dst_x >= width as i32 || dst_y >= height as i32 {
                    continue;
                }
                let src = *scaled.get_pixel(x, y);
                let dst = canvas.get_pixel_mut(dst_x as u32, dst_y as u32);
                blend(
                    dst,
                    src,
                    layer.alpha,
                    &layer.blend_mode,
                    layer.brightness,
                    layer.tint,
                );
            }
        }

        record.loaded = true;
        rendered += 1;
        results.push(record);
    }

    if rendered == 0 {
        return Ok(None);
    }

    let output = out_dir.join("native_static_frame.png");
    canvas
        .save(&output)
        .with_context(|| format!("Failed writing {}", output.display()))?;

    let report = NativeStaticRenderReport {
        output_image: output.to_string_lossy().to_string(),
        canvas_width: width,
        canvas_height: height,
        total_ready_layers: results.len(),
        rendered_layers: rendered,
        layers: results,
        notes: vec![
            "Native static compositor built from ready draw layers".to_string(),
            "Current renderer supports png and tex->png layers".to_string(),
        ],
    };

    let report_path = out_dir.join("native_static_report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)
        .with_context(|| format!("Failed writing {}", report_path.display()))?;

    Ok(Some(report))
}

pub fn render_native_animated_proxy(
    root: &Path,
    session_dir: &Path,
    canvas_width: u32,
    canvas_height: u32,
    seconds: u64,
    fps: u32,
    dry_run: bool,
    plan: &NativeRuntimePlan,
) -> Result<Option<NativeAnimatedRenderReport>> {
    let ready_layers: Vec<_> = plan
        .draw_layers
        .iter()
        .filter(|l| matches!(l.tier, NativeSupportTier::Ready) && l.primary_texture.is_some())
        .cloned()
        .collect();
    if ready_layers.is_empty() {
        return Ok(None);
    }

    let resolver = AssetResolver::new(root)?;
    let out_dir = session_dir.join("native-render");
    fs::create_dir_all(&out_dir).with_context(|| format!("Failed creating {}", out_dir.display()))?;
    let scratch = out_dir.join("scratch-animated");
    fs::create_dir_all(&scratch).with_context(|| format!("Failed creating {}", scratch.display()))?;

    let width = canvas_width.max(1);
    let height = canvas_height.max(1);
    let duration = seconds.max(4);
    let out_video = out_dir.join("native_animated_proxy.mp4");

    let mut rendered = Vec::<NativeLayerResult>::new();
    let mut input_pngs = Vec::<PathBuf>::new();

    for (idx, layer) in ready_layers.iter().enumerate() {
        let texture_ref = layer.primary_texture.clone().unwrap_or_default();
        let mut record = NativeLayerResult {
            object_id: layer.object_id,
            object_name: layer.object_name.clone(),
            texture_ref: texture_ref.clone(),
            blend_mode: layer.blend_mode.clone(),
            alpha: layer.alpha,
            loaded: false,
            reason: None,
        };

        let bytes = resolve_layer_image(&resolver, &texture_ref, &scratch)?;
        let Some(bytes) = bytes else {
            record.reason = Some("texture unresolved or unsupported format (expects png/tex->png)".to_string());
            rendered.push(record);
            continue;
        };

        let Some(img) = decode_layer_png(&bytes) else {
            record.reason = Some("failed to decode image bytes as PNG".to_string());
            rendered.push(record);
            continue;
        };

        let png_path = out_dir.join(format!("layer_{idx:03}.png"));
        img.save(&png_path)
            .with_context(|| format!("Failed writing {}", png_path.display()))?;
        input_pngs.push(png_path);
        record.loaded = true;
        rendered.push(record);
    }

    if input_pngs.is_empty() {
        return Ok(None);
    }

    let mut filter = format!(
        "color=c=black@1.0:s={}x{}:d=1,format=rgba[comp0];",
        width, height
    );
    let mut comp_idx = 0usize;
    for (i, layer) in ready_layers.iter().enumerate() {
        if i >= input_pngs.len() {
            break;
        }
        let input_idx = i;
        let moved = format!("l{}_m", i);
        let colored = format!("l{}_c", i);
        let rotated = format!("l{}_r", i);
        let next_comp = format!("comp{}", comp_idx + 1);
        let (fx, fy, ax, ay) = layer_motion(&layer.uniforms, i);
        let layer_w = layer.width.max(8.0).min(width as f32 * 2.0).round() as u32;
        let layer_h = layer.height.max(8.0).min(height as f32 * 2.0).round() as u32;

        filter.push_str(&format!(
            "[{}:v]format=rgba,scale={}:{}:flags=bicubic,setsar=1,colorchannelmixer=rr={:.3}:gg={:.3}:bb={:.3}:aa={:.3}[{}];",
            input_idx,
            layer_w,
            layer_h,
            (layer.tint[0] * layer.brightness).clamp(0.0, 2.0),
            (layer.tint[1] * layer.brightness).clamp(0.0, 2.0),
            (layer.tint[2] * layer.brightness).clamp(0.0, 2.0),
            layer.alpha.clamp(0.02, 1.0),
            colored
        ));
        if layer.angle_rad.abs() > 0.001 {
            filter.push_str(&format!(
                "[{}]rotate={:.6}:c=none:ow=rotw(iw):oh=roth(ih)[{}];",
                colored, layer.angle_rad, rotated
            ));
        } else {
            filter.push_str(&format!("[{}]copy[{}];", colored, rotated));
        }
        filter.push_str(&format!(
            "[comp{}][{}]overlay=x='{:.3}-(overlay_w/2)+sin(t*{:.3})*{:.3}':y='{:.3}-(overlay_h/2)+cos(t*{:.3})*{:.3}':format=auto[{}];",
            comp_idx,
            rotated,
            layer.center_x,
            fx,
            ax,
            layer.center_y,
            fy,
            ay,
            moved
        ));

        filter.push_str(&format!("[{}]copy[{}];", moved, next_comp));
        comp_idx += 1;
    }
    filter.push_str(&format!(
        "[comp{}]format=yuv420p[v]",
        comp_idx
    ));

    if dry_run {
        let mut cmdline = "[dry-run] ffmpeg -hide_banner -loglevel error -y".to_string();
        for p in &input_pngs {
            cmdline.push_str(&format!(" -loop 1 -i '{}'", p.display()));
        }
        cmdline.push_str(&format!(
            " -filter_complex \"{}\" -map '[v]' -t {} -r {} -an -c:v libx264 -preset veryfast -crf 20 '{}'",
            filter,
            duration,
            fps.max(24),
            out_video.display()
        ));
        println!("{}", cmdline);
    } else {
        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-hide_banner").arg("-loglevel").arg("error").arg("-y");
        for p in &input_pngs {
            cmd.arg("-loop").arg("1").arg("-i").arg(p);
        }
        let out = cmd
            .arg("-filter_complex")
            .arg(&filter)
            .arg("-map")
            .arg("[v]")
            .arg("-t")
            .arg(duration.to_string())
            .arg("-r")
            .arg(fps.max(24).to_string())
            .arg("-an")
            .arg("-c:v")
            .arg("libx264")
            .arg("-preset")
            .arg("veryfast")
            .arg("-crf")
            .arg("20")
            .arg(&out_video)
            .output()
            .context("Failed running ffmpeg for native animated proxy")?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!("ffmpeg native animated proxy failed: {}", err.trim());
        }
    }

    let report = NativeAnimatedRenderReport {
        output_video: out_video.to_string_lossy().to_string(),
        report_path: out_dir
            .join("native_animated_report.json")
            .to_string_lossy()
            .to_string(),
        canvas_width: width,
        canvas_height: height,
        seconds: duration,
        fps: fps.max(24),
        total_ready_layers: ready_layers.len(),
        rendered_layers: input_pngs.len(),
        layers: rendered,
        notes: vec![
            "Native animated compositor built from ready draw layers".to_string(),
            "Current animation path is ffmpeg-based with per-layer motion + blend".to_string(),
        ],
    };
    fs::write(&report.report_path, serde_json::to_vec_pretty(&report)?)
        .with_context(|| format!("Failed writing {}", report.report_path))?;

    Ok(Some(report))
}
