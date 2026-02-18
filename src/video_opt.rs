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
