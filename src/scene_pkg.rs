use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::fs;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const VIDEO_EXTS: [&str; 6] = ["mp4", "webm", "gif", "mkv", "avi", "mov"];

#[derive(Debug, Clone, Serialize)]
pub struct ScenePkgEntry {
    pub filename: String,
    pub offset: u32,
    pub length: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScenePkg {
    pub path: PathBuf,
    pub base_offset: u64,
    pub entries: Vec<ScenePkgEntry>,
}

fn read_u32_le(file: &mut File) -> Result<u32> {
    let mut b = [0u8; 4];
    file.read_exact(&mut b).context("Failed to read u32")?;
    Ok(u32::from_le_bytes(b))
}

fn read_sized_string(file: &mut File) -> Result<String> {
    let len = read_u32_le(file)? as usize;
    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf)
        .context("Failed to read sized string")?;
    String::from_utf8(buf).context("Package string is not valid UTF-8")
}

pub fn parse_scene_pkg(path: &Path) -> Result<ScenePkg> {
    let mut file =
        File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;

    let header = read_sized_string(&mut file)?;
    if !header.starts_with("PKGV") {
        bail!("Invalid package header '{}', expected PKGV*", header);
    }

    let files_count = read_u32_le(&mut file)? as usize;
    let mut entries = Vec::with_capacity(files_count);

    for _ in 0..files_count {
        let filename = read_sized_string(&mut file)?;
        let offset = read_u32_le(&mut file)?;
        let length = read_u32_le(&mut file)?;
        entries.push(ScenePkgEntry {
            filename,
            offset,
            length,
        });
    }

    let base_offset = file
        .stream_position()
        .context("Failed to read package base offset")?;

    Ok(ScenePkg {
        path: path.to_path_buf(),
        base_offset,
        entries,
    })
}

pub fn find_entry(pkg: &ScenePkg, name: &str) -> Option<ScenePkgEntry> {
    pkg.entries
        .iter()
        .find(|e| e.filename.eq_ignore_ascii_case(name))
        .cloned()
}

pub fn read_entry_bytes(pkg: &ScenePkg, entry: &ScenePkgEntry) -> Result<Vec<u8>> {
    let mut in_file = File::open(&pkg.path)
        .with_context(|| format!("Failed to open pkg for read: {}", pkg.path.display()))?;

    let seek_pos = pkg.base_offset + entry.offset as u64;
    in_file
        .seek(SeekFrom::Start(seek_pos))
        .with_context(|| format!("Failed to seek pkg to {}", seek_pos))?;

    let mut out = vec![0u8; entry.length as usize];
    in_file
        .read_exact(&mut out)
        .context("Failed while reading pkg entry bytes")?;
    Ok(out)
}

fn is_video_entry_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or_default();
    VIDEO_EXTS.contains(&ext)
}

fn is_preview_like(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let base = Path::new(&lower)
        .file_name()
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or(lower);
    base.starts_with("preview") || base.starts_with("thumbnail")
}

fn ext_priority(name: &str) -> u8 {
    let lower = name.to_ascii_lowercase();
    match lower.rsplit('.').next().unwrap_or_default() {
        "mp4" => 0,
        "webm" => 1,
        "mkv" => 2,
        "mov" => 3,
        "avi" => 4,
        "gif" => 9,
        _ => 8,
    }
}

pub fn best_video_entry(pkg: &ScenePkg, allow_preview_fallback: bool) -> Option<ScenePkgEntry> {
    let mut candidates: Vec<ScenePkgEntry> = pkg
        .entries
        .iter()
        .filter(|e| is_video_entry_name(&e.filename))
        .filter(|e| {
            if allow_preview_fallback {
                return true;
            }
            let lower = e.filename.to_ascii_lowercase();
            !is_preview_like(&lower) && !lower.ends_with(".gif")
        })
        .cloned()
        .collect();

    candidates.sort_by(|a, b| {
        let a_preview = if is_preview_like(&a.filename) {
            1usize
        } else {
            0usize
        };
        let b_preview = if is_preview_like(&b.filename) {
            1usize
        } else {
            0usize
        };
        let a_ext = ext_priority(&a.filename) as usize;
        let b_ext = ext_priority(&b.filename) as usize;

        a_preview
            .cmp(&b_preview)
            .then(a_ext.cmp(&b_ext))
            .then_with(|| a.filename.cmp(&b.filename))
    });

    candidates.into_iter().next()
}

pub fn extract_entry_to_cache(
    pkg: &ScenePkg,
    entry: &ScenePkgEntry,
    cache_root: &Path,
) -> Result<PathBuf> {
    let mut in_file = File::open(&pkg.path)
        .with_context(|| format!("Failed to open pkg for extraction: {}", pkg.path.display()))?;

    let seek_pos = pkg.base_offset + entry.offset as u64;
    in_file
        .seek(SeekFrom::Start(seek_pos))
        .with_context(|| format!("Failed to seek pkg to {}", seek_pos))?;

    let out_path = cache_root.join(&entry.filename);
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create cache dir {}", parent.display()))?;
    }

    let mut out = File::create(&out_path)
        .with_context(|| format!("Failed to create extracted file {}", out_path.display()))?;

    let mut remaining = entry.length as usize;
    let mut chunk = vec![0u8; 64 * 1024];

    while remaining > 0 {
        let read_n = remaining.min(chunk.len());
        in_file
            .read_exact(&mut chunk[..read_n])
            .context("Failed while reading pkg entry bytes")?;
        std::io::Write::write_all(&mut out, &chunk[..read_n])
            .context("Failed while writing extracted entry")?;
        remaining -= read_n;
    }

    Ok(out_path)
}

pub fn default_scene_cache_root(workshop_id_or_name: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".cache/kitsune-livewallpaper/scene")
        .join(workshop_id_or_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_prefers_non_preview_mp4() {
        let pkg = ScenePkg {
            path: PathBuf::from("/tmp/scene.pkg"),
            base_offset: 0,
            entries: vec![
                ScenePkgEntry {
                    filename: "preview.gif".to_string(),
                    offset: 0,
                    length: 10,
                },
                ScenePkgEntry {
                    filename: "video/main.webm".to_string(),
                    offset: 10,
                    length: 20,
                },
                ScenePkgEntry {
                    filename: "video/main.mp4".to_string(),
                    offset: 30,
                    length: 30,
                },
            ],
        };

        let best = best_video_entry(&pkg, false).expect("expected candidate");
        assert_eq!(best.filename, "video/main.mp4");
    }
}
