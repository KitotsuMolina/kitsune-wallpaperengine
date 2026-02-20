use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

fn is_video_like(path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "mp4" | "webm" | "mkv" | "mov" | "avi" | "m4v" | "gif"
    )
}

fn is_outdated(src: &Path, out: &Path) -> bool {
    let Ok(src_meta) = std::fs::metadata(src) else {
        return true;
    };
    let Ok(out_meta) = std::fs::metadata(out) else {
        return true;
    };

    let Ok(src_m) = src_meta.modified() else {
        return true;
    };
    let Ok(out_m) = out_meta.modified() else {
        return true;
    };

    out_m < src_m
}

pub fn maybe_build_optimized_proxy(
    input: &Path,
    session_dir: &Path,
    width: u32,
    fps: u32,
    crf: u8,
    dry_run: bool,
) -> Result<PathBuf> {
    if !is_video_like(input) {
        return Ok(input.to_path_buf());
    }

    let stem = input
        .file_stem()
        .map(|v| v.to_string_lossy().replace(' ', "_"))
        .unwrap_or_else(|| "scene_proxy".to_string());

    let proxy_dir = session_dir.join("proxy-opt");
    let out = proxy_dir.join(format!("{}_opt_{}w_{}fps_crf{}.mp4", stem, width, fps, crf));

    if out.is_file() && !is_outdated(input, &out) {
        return Ok(out);
    }

    if dry_run {
        println!(
            "[dry-run] ffmpeg -hide_banner -loglevel error -y -i '{}' -an -vf \"scale='min(iw,{})':-2:flags=bicubic,fps={},format=yuv420p\" -c:v libx264 -preset veryfast -crf {} -movflags +faststart '{}'",
            input.display(),
            width,
            fps,
            crf,
            out.display()
        );
        return Ok(out);
    }

    std::fs::create_dir_all(&proxy_dir).with_context(|| {
        format!(
            "Failed to create optimized proxy dir {}",
            proxy_dir.display()
        )
    })?;

    let vf = format!(
        "scale='min(iw,{})':-2:flags=bicubic,fps={},format=yuv420p",
        width, fps
    );

    let output = Command::new("ffmpeg")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-an")
        .arg("-vf")
        .arg(vf)
        .arg("-c:v")
        .arg("libx264")
        .arg("-preset")
        .arg("veryfast")
        .arg("-crf")
        .arg(crf.to_string())
        .arg("-movflags")
        .arg("+faststart")
        .arg(&out)
        .output()
        .context("Failed running ffmpeg for optimized scene proxy")?;

    if output.status.success() {
        Ok(out)
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        eprintln!(
            "[warn] could not build optimized scene proxy, using original media: {}",
            err.trim()
        );
        Ok(input.to_path_buf())
    }
}

fn probe_duration_seconds(input: &Path) -> Result<f64> {
    let output = Command::new("ffprobe")
        .arg("-v")
        .arg("error")
        .arg("-show_entries")
        .arg("format=duration")
        .arg("-of")
        .arg("default=noprint_wrappers=1:nokey=1")
        .arg(input)
        .output()
        .context("Failed running ffprobe for video duration")?;

    if !output.status.success() {
        return Ok(0.0);
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(raw.parse::<f64>().unwrap_or(0.0))
}

pub fn maybe_build_loop_crossfade_proxy(
    input: &Path,
    session_dir: &Path,
    width: u32,
    fps: u32,
    crf: u8,
    crossfade_seconds: f32,
    dry_run: bool,
) -> Result<PathBuf> {
    if !is_video_like(input) {
        return Ok(input.to_path_buf());
    }

    let fade = crossfade_seconds.clamp(0.05, 2.5) as f64;
    let duration = probe_duration_seconds(input)?;
    if duration <= fade + 0.15 {
        eprintln!(
            "[warn] loop-crossfade skipped: video is too short for fade window (duration={:.3}s, fade={:.3}s)",
            duration, fade
        );
        return maybe_build_optimized_proxy(input, session_dir, width, fps, crf, dry_run);
    }

    let stem = input
        .file_stem()
        .map(|v| v.to_string_lossy().replace(' ', "_"))
        .unwrap_or_else(|| "scene_proxy".to_string());

    let proxy_dir = session_dir.join("proxy-loop");
    let out = proxy_dir.join(format!(
        "{}_loopxfade_{}w_{}fps_crf{}_f{:.2}.mp4",
        stem, width, fps, crf, fade
    ));

    if out.is_file() && !is_outdated(input, &out) {
        return Ok(out);
    }

    let offset = (duration - fade).max(0.0);
    let trim_start = fade;
    let trim_end = duration + fade;
    let vf = format!(
        "[0:v]setpts=PTS-STARTPTS,scale='min(iw,{w})':-2:flags=bicubic,fps={fps},format=yuv420p[v0];\
[1:v]setpts=PTS-STARTPTS,scale='min(iw,{w})':-2:flags=bicubic,fps={fps},format=yuv420p[v1];\
[v0][v1]xfade=transition=fade:duration={fade:.5}:offset={offset:.5}[mix];\
[mix]trim=start={trim_start:.5}:end={trim_end:.5},setpts=PTS-STARTPTS[v]",
        w = width,
        fps = fps,
        fade = fade,
        offset = offset,
        trim_start = trim_start,
        trim_end = trim_end
    );

    if dry_run {
        println!(
            "[dry-run] ffmpeg -hide_banner -loglevel error -y -i '{in0}' -i '{in1}' -filter_complex \"{vf}\" -map '[v]' -an -c:v libx264 -preset veryfast -crf {crf} -movflags +faststart '{out}'",
            in0 = input.display(),
            in1 = input.display(),
            vf = vf,
            crf = crf,
            out = out.display()
        );
        return Ok(out);
    }

    std::fs::create_dir_all(&proxy_dir)
        .with_context(|| format!("Failed to create loop proxy dir {}", proxy_dir.display()))?;

    let output = Command::new("ffmpeg")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-i")
        .arg(input)
        .arg("-filter_complex")
        .arg(vf)
        .arg("-map")
        .arg("[v]")
        .arg("-an")
        .arg("-c:v")
        .arg("libx264")
        .arg("-preset")
        .arg("veryfast")
        .arg("-crf")
        .arg(crf.to_string())
        .arg("-movflags")
        .arg("+faststart")
        .arg(&out)
        .output()
        .context("Failed running ffmpeg for loop-crossfade proxy")?;

    if output.status.success() {
        Ok(out)
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        eprintln!(
            "[warn] loop-crossfade proxy failed, falling back to optimized proxy: {}",
            err.trim()
        );
        maybe_build_optimized_proxy(input, session_dir, width, fps, crf, dry_run)
    }
}
