use anyhow::{Context, Result, bail};
use image::codecs::png::PngEncoder;
use image::{ColorType, ImageEncoder};
use lz4_flex::block::decompress;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const FIF_WEBP_AS_MP4: i32 = 35;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContainerVersion {
    Texb0001,
    Texb0002,
    Texb0003,
    Texb0004,
}

fn read_exact<const N: usize>(f: &mut std::fs::File) -> Result<[u8; N]> {
    let mut b = [0u8; N];
    f.read_exact(&mut b)
        .with_context(|| format!("Failed to read {} bytes", N))?;
    Ok(b)
}

fn read_u32_le(f: &mut std::fs::File) -> Result<u32> {
    Ok(u32::from_le_bytes(read_exact::<4>(f)?))
}

fn read_i32_le(f: &mut std::fs::File) -> Result<i32> {
    Ok(i32::from_le_bytes(read_exact::<4>(f)?))
}

fn read_u32_as_i32(f: &mut std::fs::File) -> Result<i32> {
    Ok(read_u32_le(f)? as i32)
}

fn read_null_terminated_string(f: &mut std::fs::File) -> Result<String> {
    let mut out = Vec::<u8>::new();
    loop {
        let b = read_exact::<1>(f)?[0];
        if b == 0 {
            break;
        }
        out.push(b);
    }
    String::from_utf8(out).context("Invalid UTF-8 in null-terminated string")
}

fn detect_payload_ext(data: &[u8]) -> Option<&'static str> {
    if data.len() >= 12 && &data[4..8] == b"ftyp" {
        return Some("mp4");
    }
    if data.len() >= 4 && data[0..4] == [0x1A, 0x45, 0xDF, 0xA3] {
        return Some("webm");
    }
    if data.len() >= 8 && data[0..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        return Some("png");
    }
    if data.len() >= 3 && data[0..3] == [0xFF, 0xD8, 0xFF] {
        return Some("jpg");
    }
    if data.len() >= 6 && (&data[0..6] == b"GIF87a" || &data[0..6] == b"GIF89a") {
        return Some("gif");
    }
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Some("webp");
    }
    None
}

fn encode_raw_to_png(payload: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let pixels = width.checked_mul(height)? as usize;
    if pixels == 0 {
        return None;
    }

    // Heuristics for common raw layouts used by texture payloads.
    let (bytes, color_type) = if payload.len() >= pixels * 4 {
        (payload[..pixels * 4].to_vec(), ColorType::Rgba8)
    } else if payload.len() >= pixels * 3 {
        (payload[..pixels * 3].to_vec(), ColorType::Rgb8)
    } else if payload.len() >= pixels * 2 {
        // Treat as 16-bit grayscale-like and keep MSB channel for mask use.
        let mut gray = Vec::with_capacity(pixels);
        for i in 0..pixels {
            gray.push(payload[i * 2]);
        }
        (gray, ColorType::L8)
    } else if payload.len() >= pixels {
        (payload[..pixels].to_vec(), ColorType::L8)
    } else {
        return None;
    };

    let mut out = Vec::<u8>::new();
    let enc = PngEncoder::new(&mut out);
    enc.write_image(&bytes, width, height, color_type.into())
        .ok()?;
    Some(out)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn extract_payload_by_signature(tex_path: &Path, out_dir: &Path) -> Result<Option<PathBuf>> {
    let bytes = fs::read(tex_path)
        .with_context(|| format!("Failed to read texture file {}", tex_path.display()))?;

    let png_sig = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let jpg_sig = [0xFF, 0xD8, 0xFF];
    let gif87 = b"GIF87a";
    let gif89 = b"GIF89a";
    let webm_sig = [0x1A, 0x45, 0xDF, 0xA3];
    let riff = b"RIFF";
    let webp = b"WEBP";

    let mut picked: Option<(usize, &'static str, Vec<u8>)> = None;

    if let Some(i) = find_subslice(&bytes, &png_sig) {
        // PNG ends at IEND chunk; cut there to avoid trailing container bytes.
        let end_marker = b"IEND";
        if let Some(e) = find_subslice(&bytes[i..], end_marker) {
            let end = i + e + end_marker.len() + 4;
            let payload = bytes[i..end.min(bytes.len())].to_vec();
            picked = Some((i, "png", payload));
        } else {
            picked = Some((i, "png", bytes[i..].to_vec()));
        }
    }
    if picked.is_none() {
        if let Some(i) = find_subslice(&bytes, &jpg_sig) {
            picked = Some((i, "jpg", bytes[i..].to_vec()));
        }
    }
    if picked.is_none() {
        if let Some(i) = find_subslice(&bytes, gif89).or_else(|| find_subslice(&bytes, gif87)) {
            picked = Some((i, "gif", bytes[i..].to_vec()));
        }
    }
    if picked.is_none() {
        if let Some(i) = find_subslice(&bytes, &webm_sig) {
            picked = Some((i, "webm", bytes[i..].to_vec()));
        }
    }
    if picked.is_none() {
        if let Some(i) = find_subslice(&bytes, riff) {
            let probe_end = (i + 64).min(bytes.len());
            if find_subslice(&bytes[i..probe_end], webp).is_some() {
                picked = Some((i, "webp", bytes[i..].to_vec()));
            }
        }
    }
    if picked.is_none() {
        for i in 0..bytes.len().saturating_sub(12) {
            if bytes[i + 4..i + 8] == *b"ftyp" {
                picked = Some((i, "mp4", bytes[i..].to_vec()));
                break;
            }
        }
    }

    let Some((_idx, ext, payload)) = picked else {
        return Ok(None);
    };

    fs::create_dir_all(out_dir)
        .with_context(|| format!("Failed to create proxy dir {}", out_dir.display()))?;

    let stem = tex_path
        .file_stem()
        .map(|v| v.to_string_lossy().replace(' ', "_"))
        .unwrap_or_else(|| "scene_visual".to_string());
    let out = out_dir.join(format!("{}_proxy_sig.{}", stem, ext));
    fs::write(&out, payload).with_context(|| format!("Failed writing proxy {}", out.display()))?;
    Ok(Some(out))
}

pub fn extract_playable_proxy_from_tex(tex_path: &Path, out_dir: &Path) -> Result<Option<PathBuf>> {
    let mut f = std::fs::File::open(tex_path)
        .with_context(|| format!("Failed to open texture file {}", tex_path.display()))?;

    let magic1 = read_exact::<9>(&mut f)?;
    if &magic1 != b"TEXV0005\0" {
        bail!("Unexpected TEX header magic1 in {}", tex_path.display());
    }

    let magic2 = read_exact::<9>(&mut f)?;
    if &magic2 != b"TEXI0001\0" {
        bail!("Unexpected TEX header magic2 in {}", tex_path.display());
    }

    let _format = read_u32_le(&mut f)?;
    let _flags = read_u32_le(&mut f)?;
    let texture_width = read_u32_le(&mut f)?;
    let texture_height = read_u32_le(&mut f)?;
    let _width = read_u32_le(&mut f)?;
    let _height = read_u32_le(&mut f)?;
    let _unknown = read_u32_le(&mut f)?;

    let texb_magic = read_exact::<9>(&mut f)?;
    let image_count = read_u32_le(&mut f)?;

    let mut version = match &texb_magic {
        b"TEXB0001\0" => ContainerVersion::Texb0001,
        b"TEXB0002\0" => ContainerVersion::Texb0002,
        b"TEXB0003\0" => ContainerVersion::Texb0003,
        b"TEXB0004\0" => ContainerVersion::Texb0004,
        _ => bail!("Unknown TEX container magic in {}", tex_path.display()),
    };

    if version == ContainerVersion::Texb0003 {
        // TEXB0003 also stores free_image before mipmap blocks.
        let _free_image = read_u32_as_i32(&mut f)?;
    }

    if version == ContainerVersion::Texb0004 {
        let free_image = read_u32_as_i32(&mut f)?;
        let is_video_mp4 = read_u32_le(&mut f)? == 1;

        // Mirror linux-wallpaperengine behavior: TEXB0004 collapses into TEXB0003 unless MP4 mode.
        let effective_fif = if free_image == -1 && is_video_mp4 {
            FIF_WEBP_AS_MP4
        } else {
            free_image
        };

        if effective_fif != FIF_WEBP_AS_MP4 {
            version = ContainerVersion::Texb0003;
        }
    }

    if image_count == 0 {
        return extract_payload_by_signature(tex_path, out_dir);
    }

    // Read first image / first mipmap payload (enough for visual proxy extraction).
    let mipmap_count = read_u32_le(&mut f)?;
    if mipmap_count == 0 {
        return extract_payload_by_signature(tex_path, out_dir);
    }

    if version == ContainerVersion::Texb0004 {
        let _ = read_u32_le(&mut f)?;
        let _ = read_u32_le(&mut f)?;
        let _ = read_null_terminated_string(&mut f)?;
        let _ = read_u32_le(&mut f)?;
    }

    let mip_width = read_u32_le(&mut f)?;
    let mip_height = read_u32_le(&mut f)?;

    let (compression, mut uncompressed_size) = match version {
        ContainerVersion::Texb0001 => (0u32, 0i32),
        ContainerVersion::Texb0002 | ContainerVersion::Texb0003 | ContainerVersion::Texb0004 => {
            (read_u32_le(&mut f)?, read_i32_le(&mut f)?)
        }
    };

    let compressed_size = read_i32_le(&mut f)?;

    if compression == 0 {
        uncompressed_size = compressed_size;
    }

    if uncompressed_size <= 0 {
        return extract_payload_by_signature(tex_path, out_dir);
    }

    let payload = if compression != 0 {
        let mut compressed = vec![0u8; compressed_size.max(0) as usize];
        f.read_exact(&mut compressed).with_context(|| {
            format!(
                "Failed reading compressed payload from {}",
                tex_path.display()
            )
        })?;
        match decompress(&compressed, uncompressed_size as usize) {
            Ok(data) => data,
            Err(_) => {
                // Fall back to signature scanning on container bytes.
                return extract_payload_by_signature(tex_path, out_dir);
            }
        }
    } else {
        let mut raw = vec![0u8; uncompressed_size as usize];
        f.read_exact(&mut raw)
            .with_context(|| format!("Failed reading payload from {}", tex_path.display()))?;
        raw
    };

    let Some(ext) = detect_payload_ext(&payload) else {
        if let Some(png) = encode_raw_to_png(
            &payload,
            if mip_width > 0 {
                mip_width
            } else {
                texture_width
            },
            if mip_height > 0 {
                mip_height
            } else {
                texture_height
            },
        ) {
            fs::create_dir_all(out_dir)
                .with_context(|| format!("Failed to create proxy dir {}", out_dir.display()))?;
            let stem = tex_path
                .file_stem()
                .map(|v| v.to_string_lossy().replace(' ', "_"))
                .unwrap_or_else(|| "scene_visual".to_string());
            let out = out_dir.join(format!("{}_proxy_raw.png", stem));
            fs::write(&out, png)
                .with_context(|| format!("Failed writing proxy {}", out.display()))?;
            return Ok(Some(out));
        }
        return extract_payload_by_signature(tex_path, out_dir);
    };

    fs::create_dir_all(out_dir)
        .with_context(|| format!("Failed to create proxy dir {}", out_dir.display()))?;

    let stem = tex_path
        .file_stem()
        .map(|v| v.to_string_lossy().replace(' ', "_"))
        .unwrap_or_else(|| "scene_visual".to_string());

    let out = out_dir.join(format!("{}_proxy.{}", stem, ext));
    fs::write(&out, payload).with_context(|| format!("Failed writing proxy {}", out.display()))?;

    Ok(Some(out))
}
