use anyhow::{Context, Result, bail};

pub mod audio;
pub mod asset_resolver;
pub mod cli;
pub mod library_scan;
pub mod playback;
pub mod scene_pkg;
pub mod scene_plan;
pub mod scene_renderer;
pub mod scene_runtime;
pub mod scene_script;
pub mod scene_effect_proxy;
pub mod scene_gpu_backend;
pub mod scene_gpu_graph;
pub mod scene_native_runtime;
pub mod scene_native_renderer;
pub mod scene_text;
pub mod services;
pub mod tex_payload;
pub mod types;
pub mod video_opt;
pub mod video_tune;
pub mod wallpaper;

use audio::{probe_audio, stream_audio_levels};
use cli::{Cli, Commands};
use playback::{launch_mpvpaper, launch_mpvpaper_with_extra, stop_existing_mpvpaper_for_monitor};
use scene_pkg::{
    best_video_entry, default_scene_cache_root, extract_entry_to_cache, parse_scene_pkg,
};
use scene_plan::build_scene_plan;
use scene_gpu_backend::{SceneGpuPlayArgs, scene_gpu_play};
use scene_gpu_graph::build_scene_gpu_graph;
use scene_native_runtime::build_native_runtime_plan;
use scene_renderer::build_scene_render_session;
use scene_runtime::run_scene_runtime;
use scene_effect_proxy::{build_scene_audio_bars_overlay, maybe_build_scene_animated_proxy};
use library_scan::{build_library_roadmap, scan_library};
use scene_text::{
    build_scene_drawtext_filter, run_text_refresh, run_text_refresh_loop, start_text_refresh_daemon,
};
use services::{default_services, stop_services};
use tex_payload::extract_playable_proxy_from_tex;
use types::{SceneDiagnostics, WallpaperType};
use video_opt::maybe_build_optimized_proxy;
use video_tune::{auto_tune_preset, preset_values};
use wallpaper::{find_scene_compatible_video, inspect_wallpaper, resolve_wallpaper_path};

fn scene_diagnostics_json(diag: Option<&SceneDiagnostics>) -> String {
    diag.and_then(|s| serde_json::to_string_pretty(s).ok())
        .unwrap_or_else(|| "{}".to_string())
}

fn is_mpv_playable_visual(path: &std::path::Path) -> bool {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "mp4" | "webm" | "mkv" | "avi" | "mov" | "gif" | "png" | "jpg" | "jpeg" | "webp" | "bmp"
    )
}

fn find_preview_fallback(root: &std::path::Path) -> Option<std::path::PathBuf> {
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

pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Inspect {
            wallpaper,
            downloads_root,
        } => {
            let info = inspect_wallpaper(&wallpaper, &downloads_root)?;
            println!("{}", serde_json::to_string_pretty(&info)?);
            Ok(())
        }
        Commands::SceneDump {
            wallpaper,
            downloads_root,
            full,
        } => {
            let root = resolve_wallpaper_path(&wallpaper, &downloads_root);
            let scene_pkg_path = if root.join("scene.pkg").is_file() {
                root.join("scene.pkg")
            } else if root.join("gifscene.pkg").is_file() {
                root.join("gifscene.pkg")
            } else {
                bail!("No scene.pkg/gifscene.pkg found in {}", root.display());
            };

            let pkg = parse_scene_pkg(&scene_pkg_path)?;
            let candidate = best_video_entry(&pkg, true).map(|e| e.filename);

            if full {
                let out = serde_json::json!({
                    "pkg": scene_pkg_path,
                    "base_offset": pkg.base_offset,
                    "entries_count": pkg.entries.len(),
                    "best_video_candidate": candidate,
                    "entries": pkg.entries,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                let out = serde_json::json!({
                    "pkg": scene_pkg_path,
                    "base_offset": pkg.base_offset,
                    "entries_count": pkg.entries.len(),
                    "best_video_candidate": candidate,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            }
            Ok(())
        }
        Commands::ScenePlan {
            wallpaper,
            downloads_root,
        } => {
            let root = resolve_wallpaper_path(&wallpaper, &downloads_root);
            let plan = build_scene_plan(&root)?;
            println!("{}", serde_json::to_string_pretty(&plan)?);
            Ok(())
        }
        Commands::SceneAudioPlan {
            wallpaper,
            downloads_root,
        } => {
            let root = resolve_wallpaper_path(&wallpaper, &downloads_root);
            let plan = build_scene_audio_bars_overlay(&root)?;
            println!("{}", serde_json::to_string_pretty(&plan)?);
            Ok(())
        }
        Commands::LibraryScan {
            downloads_root,
            top_effects,
            summary_only,
        } => {
            let report = scan_library(&downloads_root, top_effects.max(1), summary_only)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Commands::LibraryRoadmap {
            downloads_root,
            top_n,
        } => {
            let report = build_library_roadmap(&downloads_root, top_n.max(1))?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Commands::SceneRuntime {
            wallpaper,
            downloads_root,
            source,
            seconds,
            frame_ms,
            extract_music,
        } => {
            let root = resolve_wallpaper_path(&wallpaper, &downloads_root);
            let runtime = run_scene_runtime(&root, source, seconds, frame_ms, extract_music)?;
            println!("{}", serde_json::to_string_pretty(&runtime)?);
            Ok(())
        }
        Commands::SceneRender {
            wallpaper,
            downloads_root,
            source,
            seconds,
            frame_ms,
        } => {
            let root = resolve_wallpaper_path(&wallpaper, &downloads_root);
            let session = build_scene_render_session(&root, source, seconds, frame_ms)?;
            println!("{}", serde_json::to_string_pretty(&session)?);
            Ok(())
        }
        Commands::SceneGpuGraph {
            wallpaper,
            downloads_root,
        } => {
            let root = resolve_wallpaper_path(&wallpaper, &downloads_root);
            let graph = build_scene_gpu_graph(&root)?;
            println!("{}", serde_json::to_string_pretty(&graph)?);
            Ok(())
        }
        Commands::SceneNativePlan {
            wallpaper,
            downloads_root,
        } => {
            let root = resolve_wallpaper_path(&wallpaper, &downloads_root);
            let graph = build_scene_gpu_graph(&root)?;
            let plan = build_native_runtime_plan(&graph);
            println!("{}", serde_json::to_string_pretty(&plan)?);
            Ok(())
        }
        Commands::SceneGpuPlay {
            wallpaper,
            monitor,
            downloads_root,
            keep_services,
            services,
            source,
            seconds,
            frame_ms,
            mute_audio,
            profile,
            display_fps,
            clock_overlay,
            apply_kitsune_overlay,
            transport,
            require_native,
            audio_bars_source,
            proxy_width,
            proxy_fps,
            proxy_crf,
            dry_run,
        } => {
            let root = resolve_wallpaper_path(&wallpaper, &downloads_root);
            let effective_services = if services.is_empty() {
                default_services()
            } else {
                services
            };
            if !keep_services {
                stop_services(&effective_services, dry_run)?;
            }
            stop_existing_mpvpaper_for_monitor(&monitor, dry_run)?;

            let out = scene_gpu_play(SceneGpuPlayArgs {
                root,
                monitor,
                source,
                seconds,
                frame_ms,
                profile,
                mute_audio,
                display_fps,
                clock_overlay,
                apply_kitsune_overlay,
                transport,
                require_native,
                audio_bars_source,
                proxy_width,
                proxy_fps,
                proxy_crf,
                dry_run,
            })?;
            println!("{}", serde_json::to_string_pretty(&out)?);
            Ok(())
        }
        Commands::TextRefresh {
            spec,
            loop_mode,
            interval_seconds,
        } => {
            if loop_mode {
                run_text_refresh_loop(&spec, interval_seconds)
            } else {
                let updated = run_text_refresh(&spec)?;
                println!("[ok] refreshed {} text layers", updated);
                Ok(())
            }
        }
        Commands::ScenePlay {
            wallpaper,
            monitor,
            downloads_root,
            keep_services,
            services,
            source,
            seconds,
            frame_ms,
            mute_audio,
            profile,
            display_fps,
            clock_overlay,
            proxy_preset,
            auto_tune,
            proxy_width,
            proxy_fps,
            proxy_crf,
            no_proxy_optimize,
            dry_run,
        } => {
            let root = resolve_wallpaper_path(&wallpaper, &downloads_root);

            let effective_services = if services.is_empty() {
                default_services()
            } else {
                services
            };

            if !keep_services {
                stop_services(&effective_services, dry_run)?;
            }

            stop_existing_mpvpaper_for_monitor(&monitor, dry_run)?;
            let session = build_scene_render_session(&root, source, seconds, frame_ms)?;

            let visual_path = std::path::PathBuf::from(&session.visual_asset_path);
            let preview_fallback = find_preview_fallback(&root);

            let entry_to_launch = if is_mpv_playable_visual(&visual_path) {
                visual_path.to_string_lossy().to_string()
            } else if visual_path
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .as_deref()
                == Some("tex")
            {
                let proxy_dir = std::path::Path::new(&session.session_dir).join("proxy");
                if let Some(proxy_from_tex) =
                    extract_playable_proxy_from_tex(&visual_path, &proxy_dir)?
                {
                    eprintln!(
                        "[warn] primary visual asset is .tex; extracted playable proxy from texture payload: {}",
                        proxy_from_tex.display()
                    );
                    proxy_from_tex.to_string_lossy().to_string()
                } else if let Some(proxy) = preview_fallback.as_ref() {
                    eprintln!(
                        "[warn] .tex proxy extraction did not find playable payload. Using preview proxy: {}",
                        proxy.display()
                    );
                    proxy.to_string_lossy().to_string()
                } else {
                    bail!(
                        "Scene render session was generated but no playable visual proxy was found.\n\nSession manifest: {}\nSession dir: {}",
                        session.manifest_path,
                        session.session_dir
                    );
                }
            } else if let Some(proxy) = preview_fallback.as_ref() {
                eprintln!(
                    "[warn] primary visual asset is not directly playable yet ({}). Using preview proxy: {}",
                    visual_path.display(),
                    proxy.display()
                );
                proxy.to_string_lossy().to_string()
            } else {
                bail!(
                    "Scene render session was generated but no playable visual proxy was found.\n\nSession manifest: {}\nSession dir: {}",
                    session.manifest_path,
                    session.session_dir
                );
            };

            let animated_entry = match maybe_build_scene_animated_proxy(
                &root,
                std::path::Path::new(&session.session_dir),
                std::path::Path::new(&entry_to_launch),
                dry_run,
            )? {
                Some(p) => {
                    eprintln!("[ok] built animated scene proxy: {}", p.display());
                    p.to_string_lossy().to_string()
                }
                None => entry_to_launch,
            };

            let final_entry = if no_proxy_optimize {
                animated_entry
            } else {
                let selected_preset = if auto_tune {
                    auto_tune_preset()
                } else {
                    proxy_preset
                };

                let base = preset_values(selected_preset);
                let eff_width = proxy_width.unwrap_or(base.width);
                let eff_fps = proxy_fps.unwrap_or(base.fps);
                let eff_crf = proxy_crf.unwrap_or(base.crf);

                if auto_tune {
                    eprintln!(
                        "[ok] auto-tune preset selected: {:?} (width={} fps={} crf={})",
                        selected_preset, eff_width, eff_fps, eff_crf
                    );
                }

                let optimized = maybe_build_optimized_proxy(
                    std::path::Path::new(&animated_entry),
                    std::path::Path::new(&session.session_dir),
                    eff_width,
                    eff_fps,
                    eff_crf,
                    dry_run,
                )?;

                if optimized.to_string_lossy() != animated_entry {
                    eprintln!("[ok] optimized proxy ready: {}", optimized.display());
                }
                optimized.to_string_lossy().to_string()
            };

            let drawtext_opt = if clock_overlay {
                let mut built = match build_scene_drawtext_filter(&root, 3) {
                    Ok(Some(vf)) => {
                        eprintln!("[ok] scene text overlays generated from scene.json");
                        Some(vf)
                    }
                    Ok(None) => Some(
                        "vf=drawtext=text=%{localtime\\:%a-%d-%b-%H\\\\:%M}:fontcolor=white:fontsize=44:x=(w-text_w)/2:y=28:box=1:boxcolor=0x00000088:boxborderw=14"
                            .to_string(),
                    ),
                    Err(err) => {
                        eprintln!(
                            "[warn] could not build scene text overlays, using fallback clock: {}",
                            err
                        );
                        Some(
                            "vf=drawtext=text=%{localtime\\:%a-%d-%b-%H\\\\:%M}:fontcolor=white:fontsize=44:x=(w-text_w)/2:y=28:box=1:boxcolor=0x00000088:boxborderw=14"
                                .to_string(),
                        )
                    }
                };
                if std::env::var("KWE_DEBUG_TEXT").ok().as_deref() == Some("1") {
                    let dbg = "drawtext=text=KWE_DEBUG:fontcolor=0xFF0000:fontsize=96:x=(w-text_w)/2:y=(h-text_h)/2:borderw=3:bordercolor=0x000000";
                    built = Some(match built {
                        Some(existing) => {
                            if let Some(stripped) = existing.strip_prefix("vf=") {
                                format!("vf={},{}", dbg, stripped)
                            } else {
                                format!("vf={},{}", dbg, existing)
                            }
                        }
                        None => format!("vf={}", dbg),
                    });
                }
                if std::env::var("KWE_DEBUG_TEXT_ONLY").ok().as_deref() == Some("1") {
                    built = Some(
                        "vf=drawtext=text=KWE_DEBUG_ONLY:fontcolor=0xFF0000:fontsize=120:x=(w-text_w)/2:y=(h-text_h)/2:borderw=4:bordercolor=0x000000"
                            .to_string(),
                    );
                }
                built
            } else {
                None
            };

            if clock_overlay {
                start_text_refresh_daemon(&root, dry_run)?;
            }

            let result = launch_mpvpaper_with_extra(
                &monitor,
                &final_entry,
                profile,
                mute_audio,
                display_fps,
                drawtext_opt.as_deref(),
                dry_run,
            );

            println!("[ok] scene session dir: {}", session.session_dir);
            println!("[ok] scene manifest: {}", session.manifest_path);
            println!("[ok] scene uniforms: {}", session.uniforms_path);
            result
        }
        Commands::AudioProbe { source, seconds } => {
            let out = probe_audio(source, seconds)?;
            println!("{}", serde_json::to_string_pretty(&out)?);
            Ok(())
        }
        Commands::AudioStream {
            source,
            seconds,
            frame_ms,
        } => {
            let out = stream_audio_levels(source, seconds, frame_ms)?;
            println!("{}", serde_json::to_string_pretty(&out)?);
            Ok(())
        }
        Commands::StopServices { services, dry_run } => {
            let services = if services.is_empty() {
                default_services()
            } else {
                services
            };
            stop_services(&services, dry_run)
        }
        Commands::Apply {
            wallpaper,
            monitor,
            downloads_root,
            keep_services,
            services,
            mute_audio,
            profile,
            display_fps,
            allow_scene_preview_fallback,
            dry_run,
        } => {
            let effective_services = if services.is_empty() {
                default_services()
            } else {
                services
            };

            if !keep_services {
                stop_services(&effective_services, dry_run)?;
            }

            stop_existing_mpvpaper_for_monitor(&monitor, dry_run)?;
            let info = inspect_wallpaper(&wallpaper, &downloads_root)?;

            match info.wallpaper_type {
                WallpaperType::Video => {
                    let entry = info
                        .entry
                        .as_deref()
                        .context("Video wallpaper entry was not found")?;
                    launch_mpvpaper(&monitor, entry, profile, mute_audio, display_fps, dry_run)
                }
                WallpaperType::Scene => {
                    let scene_root = std::path::Path::new(&info.root);

                    if let Some(fs_video) =
                        find_scene_compatible_video(scene_root, allow_scene_preview_fallback)
                    {
                        eprintln!(
                            "[warn] scene compatibility mode (filesystem): using {}",
                            fs_video.display()
                        );
                        return launch_mpvpaper(
                            &monitor,
                            &fs_video.to_string_lossy(),
                            profile,
                            mute_audio,
                            display_fps,
                            dry_run,
                        );
                    }

                    let pkg_path = if scene_root.join("scene.pkg").is_file() {
                        Some(scene_root.join("scene.pkg"))
                    } else if scene_root.join("gifscene.pkg").is_file() {
                        Some(scene_root.join("gifscene.pkg"))
                    } else {
                        None
                    };

                    if let Some(pkg_path) = pkg_path {
                        let pkg = parse_scene_pkg(&pkg_path).with_context(|| {
                            format!("Failed to parse scene package {}", pkg_path.display())
                        })?;

                        if let Some(best) = best_video_entry(&pkg, allow_scene_preview_fallback) {
                            let cache_key = info
                                .workshopid
                                .clone()
                                .unwrap_or_else(|| scene_root.to_string_lossy().replace('/', "_"));
                            let cache_root = default_scene_cache_root(&cache_key);

                            let extracted = if dry_run {
                                cache_root.join(&best.filename)
                            } else {
                                extract_entry_to_cache(&pkg, &best, &cache_root)?
                            };

                            eprintln!(
                                "[warn] scene compatibility mode (pkg extract): {} -> {}",
                                best.filename,
                                extracted.display()
                            );

                            return launch_mpvpaper(
                                &monitor,
                                &extracted.to_string_lossy(),
                                profile,
                                mute_audio,
                                display_fps,
                                dry_run,
                            );
                        }
                    }

                    let plan_hint = build_scene_plan(scene_root)
                        .ok()
                        .and_then(|p| serde_json::to_string_pretty(&p).ok())
                        .unwrap_or_else(|| "{}".to_string());

                    bail!(
                        "Scene wallpaper detected, but no usable video fallback was found in filesystem or package. \
Use native scene renderer path and inspect assets with `scene-plan`/`scene-runtime`/`scene-render`, or force low-quality fallback with --allow-scene-preview-fallback.\n\nScene diagnostics:\n{}\n\nScene plan:\n{}",
                        scene_diagnostics_json(info.scene.as_ref()),
                        plan_hint
                    );
                }
                WallpaperType::Web => {
                    bail!("Web wallpapers are not implemented yet in kitsune-wallpaperengine MVP")
                }
                WallpaperType::Application => bail!(
                    "Application wallpapers are not implemented yet in kitsune-wallpaperengine MVP"
                ),
                WallpaperType::Unknown => bail!("Unsupported/unknown wallpaper type"),
            }
        }
    }
}
