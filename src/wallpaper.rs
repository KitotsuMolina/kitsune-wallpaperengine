use crate::types::{InspectOutput, ProjectJson, SceneDiagnostics, WallpaperType};
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

const VIDEO_EXTS: [&str; 6] = ["mp4", "webm", "gif", "mkv", "avi", "mov"];

pub fn resolve_wallpaper_path(wallpaper: &str, downloads_root: &Path) -> PathBuf {
    let direct = PathBuf::from(wallpaper);
    if direct.exists() {
        return direct;
    }
    if wallpaper.chars().all(|c| c.is_ascii_digit()) {
        return downloads_root.join(wallpaper);
    }
    direct
}

pub fn is_video_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    VIDEO_EXTS.contains(&ext.as_str())
}

fn collect_video_files_recursive(
    root: &Path,
    depth: usize,
    max_depth: usize,
    out: &mut Vec<PathBuf>,
) {
    if depth > max_depth {
        return;
    }

    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() {
            if is_video_file(&p) {
                out.push(p);
            }
            continue;
        }
        if p.is_dir() {
            collect_video_files_recursive(&p, depth + 1, max_depth, out);
        }
    }
}

fn ext_priority(path: &Path) -> u8 {
    match path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .as_deref()
    {
        Some("mp4") => 0,
        Some("webm") => 1,
        Some("mkv") => 2,
        Some("mov") => 3,
        Some("avi") => 4,
        Some("gif") => 9,
        _ => 8,
    }
}

pub fn is_preview_like(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    name.starts_with("preview") || name.starts_with("thumbnail")
}

pub fn is_gif(path: &Path) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .as_deref()
        == Some("gif")
}

fn choose_best_video_candidate(mut candidates: Vec<PathBuf>) -> Option<PathBuf> {
    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by(|a, b| {
        let a_preview = if is_preview_like(a) { 1usize } else { 0usize };
        let b_preview = if is_preview_like(b) { 1usize } else { 0usize };
        let a_ext = ext_priority(a) as usize;
        let b_ext = ext_priority(b) as usize;
        let a_depth = a.components().count();
        let b_depth = b.components().count();

        a_preview
            .cmp(&b_preview)
            .then(a_ext.cmp(&b_ext))
            .then(a_depth.cmp(&b_depth))
            .then_with(|| a.to_string_lossy().cmp(&b.to_string_lossy()))
    });

    candidates.into_iter().next()
}

fn project_supports_audio_processing(project: Option<&ProjectJson>) -> bool {
    project
        .and_then(|p| p.general.get("supportsaudioprocessing"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn project_supports_video(project: Option<&ProjectJson>) -> bool {
    project
        .and_then(|p| p.general.get("supportsvideo"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

pub fn find_video_entry(root: &Path, project: Option<&ProjectJson>) -> Option<PathBuf> {
    if let Some(project) = project {
        let file = project.file.trim();
        if !file.is_empty() {
            let entry = root.join(file);
            if is_video_file(&entry) {
                return Some(entry);
            }
        }
    }

    let mut candidates = Vec::new();
    collect_video_files_recursive(root, 0, 1, &mut candidates);
    choose_best_video_candidate(candidates)
}

pub fn find_scene_compatible_video(root: &Path, allow_preview_fallback: bool) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    collect_video_files_recursive(root, 0, 6, &mut candidates);

    if allow_preview_fallback {
        return choose_best_video_candidate(candidates);
    }

    let strict: Vec<PathBuf> = candidates
        .into_iter()
        .filter(|p| !is_preview_like(p) && !is_gif(p))
        .collect();
    choose_best_video_candidate(strict)
}

pub fn inspect_scene_diagnostics(root: &Path, project: Option<&ProjectJson>) -> SceneDiagnostics {
    let mut candidates = Vec::new();
    collect_video_files_recursive(root, 0, 6, &mut candidates);
    let best = choose_best_video_candidate(candidates);

    SceneDiagnostics {
        has_scene_json: root.join("scene.json").is_file(),
        has_gifscene_json: root.join("gifscene.json").is_file(),
        has_scene_pkg: root.join("scene.pkg").is_file(),
        has_gifscene_pkg: root.join("gifscene.pkg").is_file(),
        supports_audio_processing: project_supports_audio_processing(project),
        supports_video: project_supports_video(project),
        best_video_candidate: best.as_ref().map(|p| p.to_string_lossy().to_string()),
        best_candidate_preview_like: best.as_deref().map(is_preview_like).unwrap_or(false),
        best_candidate_is_gif: best.as_deref().map(is_gif).unwrap_or(false),
    }
}

pub fn detect_type(root: &Path, project: Option<&ProjectJson>) -> WallpaperType {
    if let Some(project) = project {
        let t = WallpaperType::from_str(&project.r#type);
        if t != WallpaperType::Unknown {
            return t;
        }
    }

    if root.join("scene.json").is_file() || root.join("gifscene.json").is_file() {
        return WallpaperType::Scene;
    }
    if find_video_entry(root, project).is_some() {
        return WallpaperType::Video;
    }
    WallpaperType::Unknown
}

pub fn inspect_wallpaper(wallpaper: &str, downloads_root: &Path) -> Result<InspectOutput> {
    let root = resolve_wallpaper_path(wallpaper, downloads_root);
    if !root.exists() {
        bail!("Wallpaper path does not exist: {}", root.display());
    }
    if !root.is_dir() {
        bail!("Wallpaper path is not a directory: {}", root.display());
    }

    let project_path = root.join("project.json");
    let project = if project_path.is_file() {
        let raw = fs::read_to_string(&project_path)
            .with_context(|| format!("Failed reading {}", project_path.display()))?;
        Some(
            serde_json::from_str::<ProjectJson>(&raw)
                .with_context(|| format!("Invalid JSON in {}", project_path.display()))?,
        )
    } else {
        None
    };

    let wallpaper_type = detect_type(&root, project.as_ref());
    let entry = match wallpaper_type {
        WallpaperType::Video => {
            find_video_entry(&root, project.as_ref()).map(|p| p.to_string_lossy().to_string())
        }
        WallpaperType::Scene => {
            let p = root.join("scene.json");
            if p.is_file() {
                Some(p.to_string_lossy().to_string())
            } else {
                let p = root.join("gifscene.json");
                p.is_file().then(|| p.to_string_lossy().to_string())
            }
        }
        _ => None,
    };

    let scene = if wallpaper_type == WallpaperType::Scene {
        Some(inspect_scene_diagnostics(&root, project.as_ref()))
    } else {
        None
    };

    Ok(InspectOutput {
        root: root.to_string_lossy().to_string(),
        wallpaper_type,
        entry,
        title: project
            .as_ref()
            .map(|p| p.title.trim().to_string())
            .filter(|v| !v.is_empty()),
        workshopid: project
            .as_ref()
            .map(|p| p.workshopid.trim().to_string())
            .filter(|v| !v.is_empty()),
        project_file_found: project.is_some(),
        scene,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn detects_video_with_project_file() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("123");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("demo.mp4"), b"x").unwrap();
        fs::write(
            root.join("project.json"),
            r#"{"type":"Video","file":"demo.mp4","title":"Demo"}"#,
        )
        .unwrap();

        let out = inspect_wallpaper("123", dir.path()).unwrap();
        assert_eq!(out.wallpaper_type, WallpaperType::Video);
        assert!(out.entry.unwrap().ends_with("demo.mp4"));
    }

    #[test]
    fn detects_scene_without_project_type() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("456");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("scene.json"), b"{}").unwrap();

        let out = inspect_wallpaper("456", dir.path()).unwrap();
        assert_eq!(out.wallpaper_type, WallpaperType::Scene);
    }

    #[test]
    fn finds_scene_compatible_video_recursively() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("789");
        fs::create_dir_all(root.join("materials/subdir")).unwrap();
        fs::write(root.join("scene.json"), b"{}").unwrap();
        fs::write(root.join("materials/subdir/clip.webm"), b"x").unwrap();

        let found = find_scene_compatible_video(&root, false)
            .expect("expected recursive video fallback for scene wallpaper");
        assert!(found.ends_with("clip.webm"));
    }

    #[test]
    fn scene_strict_mode_ignores_preview_gif() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("abc");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("scene.json"), b"{}").unwrap();
        fs::write(root.join("preview.gif"), b"x").unwrap();

        assert!(find_scene_compatible_video(&root, false).is_none());
        assert!(find_scene_compatible_video(&root, true).is_some());
    }
}
