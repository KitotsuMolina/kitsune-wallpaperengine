use crate::cli::{AudioBarsSource, GpuTransport, PlaybackProfile};
use crate::playback::launch_mpvpaper_with_extra;
use crate::audio::infer_default_monitor_source;
use crate::scene_effect_proxy::{
    build_scene_audio_bars_overlay, build_scene_realtime_effect_plan, maybe_build_scene_animated_proxy,
};
use crate::scene_gpu_graph::build_scene_gpu_graph;
use crate::scene_pkg::{extract_entry_to_cache, parse_scene_pkg};
use crate::scene_renderer::build_scene_render_session;
use crate::scene_text::{build_scene_drawtext_filter, start_text_refresh_daemon};
use crate::tex_payload::extract_playable_proxy_from_tex;
use crate::video_opt::maybe_build_optimized_proxy;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

#[derive(Debug, Serialize)]
pub struct SceneGpuPlayResult {
    pub final_entry: String,
    pub gpu_manifest_path: String,
    pub scene_session_dir: String,
    pub scene_manifest_path: String,
    pub gpu_effect_nodes: usize,
    pub requested_transport: String,
    pub effective_transport: String,
    pub audio_overlay_plan_path: Option<String>,
    pub kitsune_overlay_applied: bool,
    pub kitsune_overlay_message: Option<String>,
}

#[derive(Debug, Serialize)]
struct SceneGpuManifest {
    pub version: u32,
    pub pkg_path: String,
    pub scene_width: u32,
    pub scene_height: u32,
    pub effect_nodes: usize,
    pub extracted_assets: Vec<String>,
    pub notes: Vec<String>,
}

fn is_mpv_playable_visual(path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "mp4" | "webm" | "mkv" | "avi" | "mov" | "gif" | "png" | "jpg" | "jpeg" | "webp" | "bmp"
    )
}

fn find_preview_fallback(root: &Path) -> Option<PathBuf> {
    let candidates = [
        "preview.mp4",
        "preview.webm",
        "preview.mkv",
        "preview.mov",
        "preview.avi",
        "preview.gif",
        "preview.jpg",
        "preview.jpeg",
        "preview.png",
        "preview.webp",
        "thumbnail.jpg",
        "thumbnail.jpeg",
        "thumbnail.png",
        "thumbnail.webp",
    ];
    for name in candidates {
        let p = root.join(name);
        if p.is_file() && is_mpv_playable_visual(&p) {
            return Some(p);
        }
    }
    None
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

fn collect_related_assets(graph: &crate::scene_gpu_graph::SceneGpuGraph) -> Vec<String> {
    let mut out = Vec::new();
    for node in &graph.effect_nodes {
        if !node.effect_file.is_empty() {
            out.push(node.effect_file.clone());
        }
        if let Some(v) = &node.shader_vert {
            out.push(v.clone());
        }
        if let Some(v) = &node.shader_frag {
            out.push(v.clone());
        }
        if let Some(v) = &node.material_json {
            out.push(v.clone());
        }
        for pass in &node.passes {
            for tex in &pass.textures {
                if tex.trim().is_empty() {
                    continue;
                }
                out.push(format!("{}.tex", tex.trim()));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn cache_key_for_root(root: &Path) -> String {
    root.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| root.to_string_lossy().replace('/', "_"))
}

fn realtime_pid_path(root: &Path) -> PathBuf {
    crate::scene_pkg::default_scene_cache_root(&cache_key_for_root(root))
        .join("gpu/realtime-ffmpeg.pid")
}

pub struct SceneGpuPlayArgs {
    pub root: PathBuf,
    pub monitor: String,
    pub source: Option<String>,
    pub seconds: u64,
    pub frame_ms: u64,
    pub profile: PlaybackProfile,
    pub mute_audio: bool,
    pub display_fps: Option<u32>,
    pub clock_overlay: bool,
    pub apply_kitsune_overlay: bool,
    pub transport: GpuTransport,
    pub require_native: bool,
    pub audio_bars_source: AudioBarsSource,
    pub proxy_width: u32,
    pub proxy_fps: u32,
    pub proxy_crf: u8,
    pub dry_run: bool,
}

fn resolve_kitsune_command() -> Option<(String, Vec<String>)> {
    if let Ok(path) = std::env::var("KWE_KITSUNE_CMD")
        && !path.trim().is_empty()
    {
        return Some((path, Vec::new()));
    }
    if Command::new("sh")
        .arg("-c")
        .arg("command -v kitsune >/dev/null 2>&1")
        .status()
        .ok()
        .is_some_and(|s| s.success())
    {
        return Some(("kitsune".to_string(), Vec::new()));
    }
    let script = "/home/kitotsu/Programacion/Personal/Wallpaper/Kitsune/scripts/kitsune.sh";
    if Path::new(script).is_file() {
        return Some((script.to_string(), Vec::new()));
    }
    None
}

fn apply_kitsune_overlay_plan(
    monitor: &str,
    gpu_dir: &Path,
    overlay: &crate::scene_effect_proxy::AudioBarsOverlay,
    dry_run: bool,
) -> Result<String> {
    let scene_w = overlay.scene_width.max(1) as i32;
    let scene_h = overlay.scene_height.max(1) as i32;
    let width = overlay.width.max(16) as i32;
    let height = overlay.height.max(16) as i32;
    let x0 = (overlay.center_x - (width / 2)).clamp(0, scene_w.saturating_sub(1));
    let y0 = (overlay.center_y - (height / 2)).clamp(0, scene_h.saturating_sub(1));
    let y1 = (y0 + height).clamp(1, scene_h);
    let bottom_padding = (scene_h - y1).max(0);
    let denom = (scene_h - bottom_padding).max(1) as f32;
    let height_scale = (height as f32 / denom).clamp(0.05, 1.0);
    let alpha = overlay.opacity.clamp(0.05, 1.0);

    let profile_path = gpu_dir.join("kitsune-we-audio-overlay.profile");
    let profile_content = format!(
        "height_scale={:.5}\nside_padding={}\nbottom_padding={}\nbar_gap=1\nmin_bar_height_px=0\n",
        height_scale, x0, bottom_padding
    );
    std::fs::write(&profile_path, profile_content)
        .with_context(|| format!("Failed writing {}", profile_path.display()))?;

    let group_path = gpu_dir.join("kitsune-we-audio-overlay.group");
    let layer_line = format!(
        "layer=1,bars,bars,bars_balanced,#FFFFFF,{:.3},test,0,,{},postfx_enabled=0\n",
        alpha,
        profile_path.display()
    );
    std::fs::write(
        &group_path,
        format!(
            "# auto-generated by kitsune-wallpaperengine\n# monitor={}\n{}",
            monitor, layer_line
        ),
    )
    .with_context(|| format!("Failed writing {}", group_path.display()))?;

    let Some((prog, prefix)) = resolve_kitsune_command() else {
        return Ok(format!(
            "overlay plan generated, but Kitsune command not found. Set KWE_KITSUNE_CMD or install `kitsune`. group_file={}",
            group_path.display()
        ));
    };

    let run_cmd = |args: &[&str]| -> Result<()> {
        if dry_run {
            println!("[dry-run] {} {}", prog, args.join(" "));
            return Ok(());
        }
        let mut cmd = Command::new(&prog);
        for p in &prefix {
            cmd.arg(p);
        }
        for a in args {
            cmd.arg(a);
        }
        let out = cmd
            .output()
            .with_context(|| format!("Failed to run {} {}", prog, args.join(" ")))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            bail!(
                "Kitsune command failed ({} {}): {}",
                prog,
                args.join(" "),
                err.trim()
            );
        }
        Ok(())
    };

    run_cmd(&["config", "set", "monitor", monitor])?;
    run_cmd(&["config", "set", "output_target", "layer-shell"])?;
    run_cmd(&["config", "set", "spectrum_mode", "group"])?;
    run_cmd(&["config", "set", "group_file", &group_path.to_string_lossy()])?;
    run_cmd(&["restart"])?;

    Ok(format!(
        "applied to Kitsune (output_target=layer-shell, spectrum_mode=group, group_file={})",
        group_path.display()
    ))
}

pub fn scene_gpu_play(args: SceneGpuPlayArgs) -> Result<SceneGpuPlayResult> {
    let graph = build_scene_gpu_graph(&args.root)?;
    let session =
        build_scene_render_session(&args.root, args.source.clone(), args.seconds, args.frame_ms)?;
    let audio_overlay_plan = build_scene_audio_bars_overlay(&args.root)?;
    if audio_overlay_plan.is_some() {
        eprintln!(
            "[warn] Soporte de espectros de audio y audio reactivo (barras) en fase de pruebas. \
No se recomienda su activacion por ahora. Si quieres espectros de audio estables, usa Kitowall Spectrum."
        );
    }
    let mut kitsune_overlay_applied = false;
    let mut kitsune_overlay_message = None;

    let visual_path = PathBuf::from(&session.visual_asset_path);
    let preview_fallback = find_preview_fallback(&args.root);

    let entry_to_launch = if is_mpv_playable_visual(&visual_path) {
        visual_path.to_string_lossy().to_string()
    } else if visual_path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .as_deref()
        == Some("tex")
    {
        let proxy_dir = Path::new(&session.session_dir).join("proxy");
        if let Some(proxy_from_tex) = extract_playable_proxy_from_tex(&visual_path, &proxy_dir)? {
            eprintln!(
                "[warn] gpu-play: primary visual is .tex; extracted playable payload: {}",
                proxy_from_tex.display()
            );
            proxy_from_tex.to_string_lossy().to_string()
        } else if let Some(proxy) = preview_fallback.as_ref() {
            eprintln!(
                "[warn] gpu-play: .tex extraction unavailable; using preview fallback: {}",
                proxy.display()
            );
            proxy.to_string_lossy().to_string()
        } else {
            bail!(
                "gpu-play could not resolve playable entry from scene visual.\nSession manifest: {}",
                session.manifest_path
            );
        }
    } else if let Some(proxy) = preview_fallback.as_ref() {
        proxy.to_string_lossy().to_string()
    } else {
        bail!(
            "gpu-play could not resolve playable entry.\nSession manifest: {}",
            session.manifest_path
        );
    };

    let (requested_transport, mut effective_transport) = match args.transport {
        GpuTransport::Mp4Proxy => ("mp4-proxy".to_string(), "mp4-proxy".to_string()),
        GpuTransport::NativeRealtime => {
            if args.require_native {
                ("native-realtime".to_string(), "native-realtime".to_string())
            } else {
                eprintln!(
                    "[warn] native-realtime transport requested: experimental backend enabled"
                );
                ("native-realtime".to_string(), "native-realtime".to_string())
            }
        }
    };
    let use_native_realtime = matches!(args.transport, GpuTransport::NativeRealtime);

    let final_entry = if use_native_realtime {
        let plan_opt = build_scene_realtime_effect_plan(
            &args.root,
            Path::new(&session.session_dir),
            Path::new(&entry_to_launch),
        )?;
        if plan_opt.is_none() {
            if args.require_native {
                bail!("native-realtime requested but no realtime plan could be built");
            }
            eprintln!("[warn] native-realtime plan unavailable, falling back to mp4-proxy");
            effective_transport = "mp4-proxy (fallback)".to_string();
            let animated_entry = match maybe_build_scene_animated_proxy(
                &args.root,
                Path::new(&session.session_dir),
                Path::new(&entry_to_launch),
                args.dry_run,
            )? {
                Some(p) => p.to_string_lossy().to_string(),
                None => entry_to_launch.clone(),
            };
            maybe_build_optimized_proxy(
                Path::new(&animated_entry),
                Path::new(&session.session_dir),
                args.proxy_width,
                args.proxy_fps,
                args.proxy_crf,
                args.dry_run,
            )?
            .to_string_lossy()
            .to_string()
        } else {
            let plan = plan_opt.expect("checked is_some");

            let port_base = 19000u16;
            let mut hash: u32 = 0;
            for b in cache_key_for_root(&args.root).as_bytes() {
                hash = hash.wrapping_mul(31).wrapping_add(*b as u32);
            }
            let port = port_base + (hash % 5000) as u16;
            let stream_url = format!("udp://127.0.0.1:{}", port);

            let pid_file = realtime_pid_path(&args.root);
            if let Ok(pid_raw) = std::fs::read_to_string(&pid_file) {
                if let Ok(pid) = pid_raw.trim().parse::<u32>() {
                    let _ = Command::new("kill").arg(pid.to_string()).status();
                }
            }

                if args.dry_run {
                    let mut cmdline =
                    "[dry-run] ffmpeg -hide_banner -loglevel warning -re -stream_loop -1"
                        .to_string();
                for input in &plan.inputs {
                    cmdline.push_str(&format!(" -loop 1 -i '{}'", input.display()));
                }
                if plan.needs_audio_input {
                    match args.audio_bars_source {
                        AudioBarsSource::Pulse => {
                            let pulse_src = infer_default_monitor_source()
                                .unwrap_or_else(|_| "default".to_string());
                            cmdline.push_str(&format!(" -f pulse -i '{}'", pulse_src));
                        }
                        AudioBarsSource::Synth => {
                            cmdline.push_str(" -f lavfi -i anoisesrc=color=pink:amplitude=0.4")
                        }
                    }
                }
                cmdline.push_str(&format!(
                    " -filter_complex \"{}\" -map '[v]' -r {} -an -f mpegts '{}'",
                    plan.filter_complex, args.proxy_fps, stream_url
                ));
                println!("{}", cmdline);
            } else {
                if let Some(parent) = pid_file.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                let native_log = "/tmp/kwe-native-ffmpeg.log";
                let log_file = std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(native_log)
                    .with_context(|| format!("Failed to open {}", native_log))?;
                let log_file_err = log_file
                    .try_clone()
                    .with_context(|| format!("Failed to clone {}", native_log))?;
                let mut cmd = Command::new("ffmpeg");
                cmd.arg("-hide_banner")
                    .arg("-loglevel")
                    .arg("warning")
                    .arg("-re")
                    .arg("-stream_loop")
                    .arg("-1");
                for input in &plan.inputs {
                    cmd.arg("-loop").arg("1").arg("-i").arg(input);
                }
                if plan.needs_audio_input {
                    match args.audio_bars_source {
                        AudioBarsSource::Pulse => {
                            let pulse_src = infer_default_monitor_source()
                                .unwrap_or_else(|_| "default".to_string());
                            cmd.arg("-f").arg("pulse").arg("-i").arg(pulse_src);
                        }
                        AudioBarsSource::Synth => {
                            cmd.arg("-f")
                                .arg("lavfi")
                                .arg("-i")
                                .arg("anoisesrc=color=pink:amplitude=0.4");
                        }
                    }
                }
                let child = cmd
                    .arg("-filter_complex")
                    .arg(&plan.filter_complex)
                    .arg("-map")
                    .arg("[v]")
                    .arg("-r")
                    .arg(args.proxy_fps.to_string())
                    .arg("-an")
                    .arg("-f")
                    .arg("mpegts")
                    .arg(&stream_url)
                    .stdin(std::process::Stdio::null())
                    .stdout(log_file)
                    .stderr(log_file_err)
                    .spawn()
                    .context("Failed to spawn native-realtime ffmpeg")?;
                let child_id = child.id();
                std::fs::write(&pid_file, child_id.to_string())
                    .with_context(|| format!("Failed writing {}", pid_file.display()))?;
                // Give native ffmpeg enough time to fail fast on invalid filters/input.
                thread::sleep(Duration::from_millis(1800));
                let alive = Command::new("kill")
                    .arg("-0")
                    .arg(child_id.to_string())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
                if !alive {
                    if args.require_native {
                        bail!(
                            "native-realtime ffmpeg exited on startup. See {}",
                            native_log
                        );
                    }
                    eprintln!(
                        "[warn] native-realtime ffmpeg failed, falling back to mp4-proxy (log: {})",
                        native_log
                    );
                    effective_transport = "mp4-proxy (fallback)".to_string();
                    let animated_entry = match maybe_build_scene_animated_proxy(
                        &args.root,
                        Path::new(&session.session_dir),
                        Path::new(&entry_to_launch),
                        args.dry_run,
                    )? {
                        Some(p) => p.to_string_lossy().to_string(),
                        None => entry_to_launch.clone(),
                    };
                    return Ok(SceneGpuPlayResult {
                        final_entry: maybe_build_optimized_proxy(
                            Path::new(&animated_entry),
                            Path::new(&session.session_dir),
                            args.proxy_width,
                            args.proxy_fps,
                            args.proxy_crf,
                            args.dry_run,
                        )?
                        .to_string_lossy()
                        .to_string(),
                        gpu_manifest_path: Path::new(&session.session_dir)
                            .join("gpu/manifest.json")
                            .to_string_lossy()
                            .to_string(),
                        scene_session_dir: session.session_dir,
                        scene_manifest_path: session.manifest_path,
                        gpu_effect_nodes: graph.effect_nodes.len(),
                        requested_transport,
                        effective_transport,
                        audio_overlay_plan_path: None,
                        kitsune_overlay_applied: false,
                        kitsune_overlay_message: None,
                    });
                }
            }

            stream_url
        }
    } else {
        let animated_entry = match maybe_build_scene_animated_proxy(
            &args.root,
            Path::new(&session.session_dir),
            Path::new(&entry_to_launch),
            args.dry_run,
        )? {
            Some(p) => p.to_string_lossy().to_string(),
            None => entry_to_launch,
        };
        maybe_build_optimized_proxy(
            Path::new(&animated_entry),
            Path::new(&session.session_dir),
            args.proxy_width,
            args.proxy_fps,
            args.proxy_crf,
            args.dry_run,
        )?
        .to_string_lossy()
        .to_string()
    };

    let gpu_dir = Path::new(&session.session_dir).join("gpu");
    std::fs::create_dir_all(&gpu_dir)
        .with_context(|| format!("Failed creating gpu dir {}", gpu_dir.display()))?;

    let mut extracted_assets = Vec::<String>::new();
    if let Some(pkg_path) = pick_pkg_path(&args.root) {
        let pkg = parse_scene_pkg(&pkg_path)?;
        for rel in collect_related_assets(&graph) {
            if let Some(entry) = pkg
                .entries
                .iter()
                .find(|e| e.filename.eq_ignore_ascii_case(&rel))
                .cloned()
            {
                let extracted = extract_entry_to_cache(&pkg, &entry, &gpu_dir.join("assets"))?;
                extracted_assets.push(extracted.to_string_lossy().to_string());
            }
        }
    }

    let gpu_manifest = SceneGpuManifest {
        version: 1,
        pkg_path: graph.pkg_path.clone(),
        scene_width: graph.scene_width,
        scene_height: graph.scene_height,
        effect_nodes: graph.effect_nodes.len(),
        extracted_assets,
        notes: vec![
            "GPU phase runtime prepared".to_string(),
            "Current playback still uses mpvpaper transport with generated scene media".to_string(),
            "Next milestone: native shader execution backend replacing media proxy step"
                .to_string(),
            "Audio bars are exported as overlay metadata and not burned into ffmpeg scene output".to_string(),
            format!("Requested transport: {}", requested_transport),
            format!("Effective transport: {}", effective_transport),
        ],
    };
    let gpu_manifest_path = gpu_dir.join("manifest.json");
    std::fs::write(&gpu_manifest_path, serde_json::to_vec_pretty(&gpu_manifest)?)
        .with_context(|| format!("Failed writing {}", gpu_manifest_path.display()))?;

    let audio_overlay_plan_path = if let Some(plan) = audio_overlay_plan {
        let path = gpu_dir.join("audio-bars-overlay.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&plan)?)
            .with_context(|| format!("Failed writing {}", path.display()))?;
        eprintln!("[ok] audio bars overlay plan: {}", path.display());
        Some(path.to_string_lossy().to_string())
    } else {
        None
    };

    if args.apply_kitsune_overlay {
        if let Some(plan) = build_scene_audio_bars_overlay(&args.root)? {
            match apply_kitsune_overlay_plan(&args.monitor, &gpu_dir, &plan, args.dry_run) {
                Ok(msg) => {
                    kitsune_overlay_applied = true;
                    kitsune_overlay_message = Some(msg);
                    eprintln!("[ok] Kitsune overlay apply done");
                }
                Err(err) => {
                    kitsune_overlay_message = Some(format!("apply failed: {}", err));
                    eprintln!("[warn] Kitsune overlay apply failed: {}", err);
                }
            }
        } else {
            kitsune_overlay_message = Some("scene has no audio bars overlay".to_string());
        }
    }

    let drawtext_opt = if args.clock_overlay {
        match build_scene_drawtext_filter(&args.root, 3) {
            Ok(Some(v)) => Some(v),
            Ok(None) => None,
            Err(err) => {
                eprintln!("[warn] gpu-play text overlays unavailable: {}", err);
                None
            }
        }
    } else {
        None
    };

    if args.clock_overlay {
        start_text_refresh_daemon(&args.root, args.dry_run)?;
    }

    launch_mpvpaper_with_extra(
        &args.monitor,
        &final_entry,
        args.profile,
        args.mute_audio,
        args.display_fps,
        drawtext_opt.as_deref(),
        args.dry_run,
    )?;

    Ok(SceneGpuPlayResult {
        final_entry,
        gpu_manifest_path: gpu_manifest_path.to_string_lossy().to_string(),
        scene_session_dir: session.session_dir,
        scene_manifest_path: session.manifest_path,
        gpu_effect_nodes: graph.effect_nodes.len(),
        requested_transport,
        effective_transport,
        audio_overlay_plan_path,
        kitsune_overlay_applied,
        kitsune_overlay_message,
    })
}
