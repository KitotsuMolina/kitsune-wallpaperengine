use crate::scene_pkg::{ScenePkg, find_entry, parse_scene_pkg, read_entry_bytes};
use anyhow::{Result, bail};
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetSourceKind {
    Package,
    WallpaperDir,
    GlobalAssets,
}

#[derive(Debug, Clone)]
pub struct ResolvedAsset {
    pub request_path: String,
    pub resolved_path: String,
    pub source: AssetSourceKind,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct AssetResolver {
    root: PathBuf,
    pkg: Option<ScenePkg>,
    global_assets_root: Option<PathBuf>,
}

fn normalize_rel_path(path: &str) -> Option<String> {
    let raw = path.trim().replace('\\', "/");
    if raw.is_empty() {
        return None;
    }

    let mut out = PathBuf::new();
    for comp in Path::new(&raw).components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(v) => out.push(v),
            Component::RootDir | Component::Prefix(_) => {}
        }
    }

    let s = out.to_string_lossy().replace('\\', "/");
    if s.is_empty() { None } else { Some(s) }
}

fn find_global_assets_root(root: &Path) -> Option<PathBuf> {
    if let Ok(v) = std::env::var("KWE_WE_ROOT") {
        let p = PathBuf::from(v).join("assets");
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Ok(v) = std::env::var("KWE_ASSETS_ROOT") {
        let p = PathBuf::from(v);
        if p.is_dir() {
            return Some(p);
        }
    }

    if let Some(p) = find_steam_wallpaper_engine_assets() {
        return Some(p);
    }

    let mut candidates = Vec::<PathBuf>::new();
    candidates.push(root.join("wallpaperengine/assets"));
    if let Some(parent) = root.parent() {
        candidates.push(parent.join("wallpaperengine/assets"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("wallpaperengine/assets"));
        for anc in cwd.ancestors() {
            candidates.push(anc.join("wallpaperengine/assets"));
        }
    }

    candidates.into_iter().find(|p| p.is_dir())
}

fn user_home_dir() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

fn steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::<PathBuf>::new();
    let Some(home) = user_home_dir() else {
        return roots;
    };

    // Native Steam installs
    roots.push(home.join(".local/share/Steam"));
    roots.push(home.join(".steam/steam"));
    roots.push(home.join(".steam/root"));

    // Flatpak Steam installs
    roots.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
    roots
}

fn parse_libraryfolders_vdf(vdf_path: &Path) -> Vec<PathBuf> {
    let Ok(content) = fs::read_to_string(vdf_path) else {
        return Vec::new();
    };

    let mut out = Vec::<PathBuf>::new();
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if !line.contains("\"path\"") {
            continue;
        }

        let first = match line.find('"') {
            Some(v) => v,
            None => continue,
        };
        let after_first = &line[(first + 1)..];
        let second_rel = match after_first.find('"') {
            Some(v) => v,
            None => continue,
        };
        let after_key = &after_first[(second_rel + 1)..].trim_start();
        let value_start = match after_key.find('"') {
            Some(v) => v,
            None => continue,
        };
        let after_value_start = &after_key[(value_start + 1)..];
        let value_end = match after_value_start.find('"') {
            Some(v) => v,
            None => continue,
        };
        let raw = &after_value_start[..value_end];
        if raw.is_empty() {
            continue;
        }
        let unescaped = raw.replace("\\\\", "/");
        out.push(PathBuf::from(unescaped));
    }

    out
}

fn wallpaper_engine_assets_in_library(library_root: &Path) -> Option<PathBuf> {
    let p = library_root
        .join("steamapps/common/wallpaper_engine/assets");
    if p.is_dir() { Some(p) } else { None }
}

fn find_steam_wallpaper_engine_assets() -> Option<PathBuf> {
    let mut library_roots = Vec::<PathBuf>::new();
    for steam_root in steam_roots() {
        library_roots.push(steam_root.clone());
        let vdf = steam_root.join("steamapps/libraryfolders.vdf");
        library_roots.extend(parse_libraryfolders_vdf(&vdf));
    }

    for lib in library_roots {
        if let Some(p) = wallpaper_engine_assets_in_library(&lib) {
            return Some(p);
        }
    }
    None
}

impl AssetResolver {
    pub fn new(root: &Path) -> Result<Self> {
        if !root.exists() {
            bail!("Wallpaper root does not exist: {}", root.display());
        }

        let pkg = if root.join("scene.pkg").is_file() {
            Some(parse_scene_pkg(&root.join("scene.pkg"))?)
        } else if root.join("gifscene.pkg").is_file() {
            Some(parse_scene_pkg(&root.join("gifscene.pkg"))?)
        } else {
            None
        };

        Ok(Self {
            root: root.to_path_buf(),
            pkg,
            global_assets_root: find_global_assets_root(root),
        })
    }

    pub fn pkg_path(&self) -> Option<String> {
        self.pkg
            .as_ref()
            .map(|p| p.path.to_string_lossy().to_string())
    }

    pub fn global_assets_root(&self) -> Option<String> {
        self.global_assets_root
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
    }

    pub fn resolve_first(&self, candidates: &[String]) -> Option<ResolvedAsset> {
        for c in candidates {
            if let Some(v) = self.resolve(c) {
                return Some(v);
            }
        }
        None
    }

    pub fn resolve(&self, request_path: &str) -> Option<ResolvedAsset> {
        let rel = normalize_rel_path(request_path)?;

        if let Some(pkg) = &self.pkg
            && let Some(entry) = find_entry(pkg, &rel)
            && let Ok(bytes) = read_entry_bytes(pkg, &entry)
        {
            return Some(ResolvedAsset {
                request_path: rel.clone(),
                resolved_path: entry.filename,
                source: AssetSourceKind::Package,
                bytes,
            });
        }

        let fs_path = self.root.join(&rel);
        if fs_path.is_file()
            && let Ok(bytes) = fs::read(&fs_path)
        {
            return Some(ResolvedAsset {
                request_path: rel.clone(),
                resolved_path: rel.clone(),
                source: AssetSourceKind::WallpaperDir,
                bytes,
            });
        }

        if let Some(global_root) = &self.global_assets_root {
            let mut global_candidates = Vec::<PathBuf>::new();
            global_candidates.push(global_root.join(&rel));
            if rel.starts_with("assets/") {
                global_candidates.push(global_root.join(rel.trim_start_matches("assets/")));
            }

            for candidate in global_candidates {
                if !candidate.is_file() {
                    continue;
                }
                if let Ok(bytes) = fs::read(&candidate) {
                    let resolved = candidate
                        .strip_prefix(global_root)
                        .ok()
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_else(|| candidate.to_string_lossy().replace('\\', "/"));
                    return Some(ResolvedAsset {
                        request_path: rel.clone(),
                        resolved_path: resolved,
                        source: AssetSourceKind::GlobalAssets,
                        bytes,
                    });
                }
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_path_works() {
        assert_eq!(normalize_rel_path("./a/b/../c.json").as_deref(), Some("a/c.json"));
        assert_eq!(normalize_rel_path("\\materials\\x.tex").as_deref(), Some("materials/x.tex"));
        assert!(normalize_rel_path("  ").is_none());
    }

    #[test]
    fn parses_libraryfolders_paths() {
        let tmp = std::env::temp_dir().join("kwe-libraryfolders-test.vdf");
        let content = r#"
"libraryfolders"
{
    "0"
    {
        "path"    "/home/user/.local/share/Steam"
    }
    "1"
    {
        "path"    "/mnt/games/SteamLibrary"
    }
}
"#;
        fs::write(&tmp, content).expect("write vdf");
        let got = parse_libraryfolders_vdf(&tmp);
        let _ = fs::remove_file(&tmp);
        assert!(got.iter().any(|p| p.to_string_lossy().contains(".local/share/Steam")));
        assert!(got.iter().any(|p| p.to_string_lossy().contains("/mnt/games/SteamLibrary")));
    }
}
