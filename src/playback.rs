use crate::cli::PlaybackProfile;
use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

pub fn build_mpv_options(
    profile: PlaybackProfile,
    mute_audio: bool,
    _display_fps: Option<u32>,
) -> String {
    build_mpv_options_with_extra(profile, mute_audio, _display_fps, None)
}

pub fn build_mpv_options_with_extra(
    profile: PlaybackProfile,
    mute_audio: bool,
    _display_fps: Option<u32>,
    extra_opt: Option<&str>,
) -> String {
    // Keep options compatible with mpvpaper (mpvpaper embeds libmpv and rejects vo overrides).
    let mut parts: Vec<String> = vec![
        "--loop-file=inf".to_string(),
        "hwdec=auto-safe".to_string(),
        "keep-open=yes".to_string(),
    ];

    if let Some(extra) = extra_opt {
        if !extra.trim().is_empty() {
            parts.push(extra.to_string());
        }
    }

    match profile {
        PlaybackProfile::Performance => {
            parts.push("profile=fast".to_string());
            parts.push("interpolation=no".to_string());
            parts.push("video-sync=audio".to_string());
            parts.push("deband=no".to_string());
        }
        PlaybackProfile::Balanced => {
            parts.push("interpolation=no".to_string());
            parts.push("video-sync=display-resample".to_string());
            parts.push("scale=ewa_lanczossharp".to_string());
            parts.push("cscale=ewa_lanczossharp".to_string());
            parts.push("dscale=mitchell".to_string());
            parts.push("correct-downscaling=yes".to_string());
            parts.push("deband=yes".to_string());
        }
        PlaybackProfile::Quality => {
            parts.push("profile=high-quality".to_string());
            parts.push("interpolation=yes".to_string());
            parts.push("video-sync=display-resample".to_string());
            parts.push("tscale=oversample".to_string());
            parts.push("scale=ewa_lanczossharp".to_string());
            parts.push("cscale=ewa_lanczossharp".to_string());
            parts.push("dscale=mitchell".to_string());
            parts.push("correct-downscaling=yes".to_string());
            parts.push("deband=yes".to_string());
        }
    }

    if mute_audio {
        parts.push("no-audio".to_string());
    }

    parts.join(" ")
}

pub fn stop_existing_mpvpaper_for_monitor(monitor: &str, dry_run: bool) -> Result<()> {
    let out = Command::new("pgrep")
        .arg("-fa")
        .arg("mpvpaper")
        .output()
        .context("Failed to run pgrep for mpvpaper")?;

    if !out.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    for line in stdout.lines() {
        let mut parts = line.splitn(2, ' ');
        let pid = match parts.next().and_then(|p| p.parse::<u32>().ok()) {
            Some(v) => v,
            None => continue,
        };
        let cmd = parts.next().unwrap_or_default();
        if !cmd.contains(monitor) {
            continue;
        }
        // Only kill mpvpaper sessions started by kitsune-livewallpaper.
        // This avoids killing Kitsune Spectrum (which may run on its own mpvpaper/layer stack).
        let is_kwe_session = cmd.contains("kitsune-livewallpaper")
            || cmd.contains(".cache/kitsune-livewallpaper")
            || cmd.contains("render-session")
            || cmd.contains("udp://127.0.0.1:");
        if !is_kwe_session {
            continue;
        }

        if dry_run {
            println!("[dry-run] kill {}  # {}", pid, cmd);
            continue;
        }

        let kill_out = Command::new("kill").arg(pid.to_string()).output();
        match kill_out {
            Ok(res) if res.status.success() => {
                println!("[ok] killed old mpvpaper pid={} monitor={}", pid, monitor);
            }
            Ok(res) => {
                let err = String::from_utf8_lossy(&res.stderr);
                eprintln!("[warn] failed to kill {}: {}", pid, err.trim());
            }
            Err(err) => {
                eprintln!("[warn] failed to kill {}: {}", pid, err);
            }
        }
    }

    Ok(())
}

fn find_running_mpvpaper_for_monitor(monitor: &str, entry: &str) -> Result<Option<u32>> {
    let out = Command::new("pgrep")
        .arg("-fa")
        .arg("mpvpaper")
        .output()
        .context("Failed to query mpvpaper process list")?;

    if !out.status.success() {
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    for line in stdout.lines() {
        let mut parts = line.splitn(2, ' ');
        let pid = match parts.next().and_then(|p| p.parse::<u32>().ok()) {
            Some(v) => v,
            None => continue,
        };
        let cmd = parts.next().unwrap_or_default();
        if cmd.contains(monitor) && cmd.contains(entry) {
            return Ok(Some(pid));
        }
    }

    Ok(None)
}

pub fn launch_mpvpaper(
    monitor: &str,
    entry: &str,
    profile: PlaybackProfile,
    mute_audio: bool,
    _display_fps: Option<u32>,
    dry_run: bool,
) -> Result<()> {
    launch_mpvpaper_with_extra(
        monitor,
        entry,
        profile,
        mute_audio,
        _display_fps,
        None,
        dry_run,
    )
}

pub fn launch_mpvpaper_with_extra(
    monitor: &str,
    entry: &str,
    profile: PlaybackProfile,
    mute_audio: bool,
    _display_fps: Option<u32>,
    extra_opt: Option<&str>,
    dry_run: bool,
) -> Result<()> {
    let mut opts = build_mpv_options_with_extra(profile, mute_audio, _display_fps, extra_opt);
    let mpv_log_enabled = std::env::var("KWE_MPV_LOG").ok().as_deref() == Some("1");
    if mpv_log_enabled && !opts.contains("msg-level=") {
        opts.push_str(" msg-level=all=v");
    }

    if _display_fps.is_some() {
        eprintln!("[warn] --display-fps is currently ignored for mpvpaper compatibility");
    }

    if dry_run {
        println!(
            "[dry-run] nohup mpvpaper -o '{}' {} {}",
            opts, monitor, entry
        );
        return Ok(());
    }

    let mut cmd = Command::new("nohup");
    cmd.arg("mpvpaper")
        .arg("-o")
        .arg(&opts)
        .arg(monitor)
        .arg(entry)
        .stdin(Stdio::null());

    if mpv_log_enabled {
        let log_path = "/tmp/kwe-mpvpaper.log";
        let logf = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .with_context(|| format!("Failed to open {}", log_path))?;
        let logf_err = logf
            .try_clone()
            .with_context(|| format!("Failed to clone log handle for {}", log_path))?;
        cmd.stdout(Stdio::from(logf)).stderr(Stdio::from(logf_err));
        eprintln!("[ok] mpvpaper log enabled: {}", log_path);
    } else {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }

    cmd.spawn()
        .context("Failed to spawn detached mpvpaper via nohup")?;

    thread::sleep(Duration::from_millis(500));
    let pid = find_running_mpvpaper_for_monitor(monitor, entry)?
        .context("mpvpaper process not found after launch")?;

    println!(
        "[ok] launched mpvpaper pid={} monitor={} profile={:?} entry={}",
        pid, monitor, profile, entry
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_quality_options() {
        let opts = build_mpv_options(PlaybackProfile::Quality, false, Some(144));
        assert!(opts.contains("profile=high-quality"));
        assert!(opts.contains("interpolation=yes"));
        assert!(!opts.contains("vo=gpu-next"));
    }

    #[test]
    fn builds_performance_options() {
        let opts = build_mpv_options(PlaybackProfile::Performance, true, None);
        assert!(opts.contains("profile=fast"));
        assert!(opts.contains("no-audio"));
        assert!(opts.contains("interpolation=no"));
    }

    #[test]
    fn builds_options_with_extra() {
        let opts = build_mpv_options_with_extra(
            PlaybackProfile::Performance,
            true,
            None,
            Some("vf=drawtext=text=%{localtime\\:%H\\\\:%M}"),
        );
        assert!(opts.contains("vf=drawtext"));
    }
}
