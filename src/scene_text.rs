use crate::scene_pkg::{
    default_scene_cache_root, extract_entry_to_cache, find_entry, parse_scene_pkg, read_entry_bytes,
};
use anyhow::{Context, Result};
use chrono::{Local, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RefreshEntry {
    file_path: String,
    object: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RefreshSpec {
    entries: Vec<RefreshEntry>,
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

fn cache_key_for_root(root: &Path) -> String {
    root.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| root.to_string_lossy().replace('/', "_"))
}

fn text_layers_dir(root: &Path) -> PathBuf {
    default_scene_cache_root(&cache_key_for_root(root)).join("text-layers")
}

fn runtime_spec_path(root: &Path) -> PathBuf {
    text_layers_dir(root).join("runtime_spec.json")
}

fn updater_pid_path(root: &Path) -> PathBuf {
    text_layers_dir(root).join("updater.pid")
}

fn is_dynamic_text_object(object: &Value) -> bool {
    let name = object
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let raw_value = object
        .get("text")
        .and_then(|v| match v {
            Value::String(s) => Some(s.as_str()),
            Value::Object(map) => map.get("value").and_then(|x| x.as_str()),
            _ => None,
        })
        .unwrap_or_default()
        .to_ascii_lowercase();
    let script = object
        .get("text")
        .and_then(|v| v.get("script"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let has_dynamic_props = object
        .get("text")
        .and_then(|v| v.get("scriptproperties"))
        .and_then(|v| v.as_object())
        .map(|m| {
            m.contains_key("use24hFormat")
                || m.contains_key("showSeconds")
                || m.contains_key("dayFormat")
                || m.contains_key("monthFormat")
        })
        .unwrap_or(false);
    script.contains("gethours")
        || script.contains("getminutes")
        || script.contains("getseconds")
        || script.contains("getday")
        || script.contains("getdate")
        || script.contains("getmonth")
        || script.contains("getfullyear")
        || script.contains("new date(")
        || name.contains("clock")
        || name.contains("time")
        || name.contains("date")
        || name.contains("day")
        || raw_value == "time"
        || raw_value == "date"
        || raw_value == "day"
        || raw_value == "<time>"
        || raw_value == "<date>"
        || has_dynamic_props
}

fn parse_vec3_xy(value: &str) -> Option<(f32, f32)> {
    let mut it = value.split_whitespace();
    let x = it.next()?.parse::<f32>().ok()?;
    let y = it.next()?.parse::<f32>().ok()?;
    Some((x, y))
}

fn parse_vec3(value: &str) -> Option<(f32, f32, f32)> {
    let mut it = value.split_whitespace();
    let x = it.next()?.parse::<f32>().ok()?;
    let y = it.next()?.parse::<f32>().ok()?;
    let z = it.next()?.parse::<f32>().ok()?;
    Some((x, y, z))
}

fn parse_vec2(value: &str) -> Option<(f32, f32)> {
    let mut it = value.split_whitespace();
    let x = it.next()?.parse::<f32>().ok()?;
    let y = it.next()?.parse::<f32>().ok()?;
    Some((x, y))
}

fn parse_scene_size(scene_json: &Value) -> (f32, f32) {
    let width = scene_json
        .get("general")
        .and_then(|v| v.get("orthogonalprojection"))
        .and_then(|v| v.get("width"))
        .and_then(|v| v.as_f64())
        .unwrap_or(1920.0) as f32;

    let height = scene_json
        .get("general")
        .and_then(|v| v.get("orthogonalprojection"))
        .and_then(|v| v.get("height"))
        .and_then(|v| v.as_f64())
        .unwrap_or(1080.0) as f32;

    (width.max(1.0), height.max(1.0))
}

fn visible_enabled(object: &Value) -> bool {
    match object.get("visible") {
        Some(Value::Bool(v)) => *v,
        Some(Value::Object(map)) => map.get("value").and_then(|v| v.as_bool()).unwrap_or(true),
        _ => true,
    }
}

fn parse_color(object: &Value) -> (String, f32, f32, f32) {
    let raw = object
        .get("color")
        .and_then(|v| match v {
            Value::String(s) => Some(s.as_str()),
            Value::Object(map) => map.get("value").and_then(|x| x.as_str()),
            _ => None,
        })
        .unwrap_or("1.0 1.0 1.0");

    let mut it = raw.split_whitespace();
    let r = it.next().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
    let g = it.next().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
    let b = it.next().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
    let brightness = object
        .get("brightness")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0) as f32;
    let rb = (r * brightness).clamp(0.0, 1.0);
    let gb = (g * brightness).clamp(0.0, 1.0);
    let bb = (b * brightness).clamp(0.0, 1.0);
    let to_byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    (
        format!(
            "0x{:02X}{:02X}{:02X}",
            to_byte(rb),
            to_byte(gb),
            to_byte(bb)
        ),
        rb,
        gb,
        bb,
    )
}

fn escape_static_text(text: &str) -> String {
    text.replace('\\', r"\\")
        .replace(':', r"\:")
        .replace(',', r"\,")
        .replace(' ', r"\ ")
        .replace('\'', r"\'")
        .replace('%', r"\%")
}

fn escape_filter_value(text: &str) -> String {
    text.replace('\\', r"\\")
        .replace(':', r"\:")
        .replace(',', r"\,")
}

fn object_id(object: &Value) -> u64 {
    object.get("id").and_then(|v| v.as_u64()).unwrap_or(0)
}

fn write_text_layer_file(text_dir: &Path, object: &Value, text: &str) -> Option<String> {
    if fs::create_dir_all(text_dir).is_err() {
        return None;
    }
    let name = format!("obj_{}.txt", object_id(object));
    let path = text_dir.join(name);
    if fs::write(&path, text).is_err() {
        return None;
    }
    Some(escape_filter_value(&path.to_string_lossy()))
}

fn read_xy_value(v: &Value) -> Option<(f32, f32)> {
    match v {
        Value::String(s) => parse_vec2(s),
        Value::Object(map) => map
            .get("value")
            .and_then(|inner| inner.as_str())
            .and_then(parse_vec2),
        _ => None,
    }
}

fn extract_transform_effect(object: &Value) -> (f32, f32, f32) {
    let mut offset_x = 0.0f32;
    let mut offset_y = 0.0f32;
    let mut scale_y = 1.0f32;

    let Some(effects) = object.get("effects").and_then(|v| v.as_array()) else {
        return (offset_x, offset_y, scale_y);
    };

    for effect in effects {
        let file = effect
            .get("file")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !file.contains("transform/effect.json") {
            continue;
        }
        let Some(passes) = effect.get("passes").and_then(|v| v.as_array()) else {
            continue;
        };
        for pass in passes {
            let Some(csv) = pass.get("constantshadervalues").and_then(|v| v.as_object()) else {
                continue;
            };
            if let Some(v) = csv.get("offset").and_then(read_xy_value) {
                offset_x = v.0;
                offset_y = v.1;
            }
            if let Some(v) = csv.get("scale").and_then(read_xy_value) {
                scale_y *= v.1.abs().max(0.1);
            }
        }
    }

    (offset_x, offset_y, scale_y)
}

fn prop_bool(v: Option<&Value>, default: bool) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Object(map)) => map
            .get("value")
            .and_then(|x| x.as_bool())
            .unwrap_or(default),
        _ => default,
    }
}

fn prop_str<'a>(v: Option<&'a Value>) -> Option<&'a str> {
    match v {
        Some(Value::String(s)) => Some(s),
        Some(Value::Object(map)) => map.get("value").and_then(|x| x.as_str()),
        _ => None,
    }
}

fn build_day_or_date_text(
    name: &str,
    script: &str,
    props: Option<&serde_json::Map<String, Value>>,
) -> Option<String> {
    let now = Local::now();
    let script_lower = script.to_ascii_lowercase();
    let show_day = prop_bool(props.and_then(|m| m.get("showDay")), name.contains("day"));
    let day_format = props
        .and_then(|m| m.get("dayFormat"))
        .and_then(|v| v.as_str())
        .unwrap_or("2");
    let month_format = props
        .and_then(|m| m.get("monthFormat"))
        .and_then(|v| v.as_str())
        .unwrap_or("2");
    let use_delimiter = prop_bool(props.and_then(|m| m.get("useDelimiter")), false);
    let delim = props
        .and_then(|m| m.get("addDelimiter"))
        .and_then(|v| v.as_str())
        .unwrap_or("/");
    let delimiter = if use_delimiter {
        if delim.is_empty() { " " } else { delim }
    } else {
        " "
    };

    let spaced_days = script_lower.contains("s u n d a y");
    let day_text = if day_format == "1" {
        now.format("%a").to_string().to_uppercase()
    } else {
        now.format("%A").to_string().to_uppercase()
    };
    let day_text = if spaced_days {
        day_text
            .chars()
            .filter(|c| *c != ' ')
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        day_text
    };

    let month_text = match month_format {
        "1" => now.format("%-m").to_string(),
        "3" => now.format("%B").to_string(),
        _ => now.format("%b").to_string().to_uppercase(),
    };
    let month_text = if script_lower.contains("' jan '") {
        format!(" {} ", month_text)
    } else {
        month_text
    };

    if show_day {
        return Some(day_text);
    }

    let out = format!(
        "{}{}{}{}{}",
        now.format("%-d"),
        delimiter,
        month_text,
        delimiter,
        now.format("%Y")
    );
    Some(out.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn build_clock_text(props: Option<&serde_json::Map<String, Value>>) -> Option<String> {
    let now = Local::now();
    let use_24h = prop_bool(props.and_then(|m| m.get("use24hFormat")), true);
    let show_seconds = prop_bool(props.and_then(|m| m.get("showSeconds")), false);
    let delimiter = prop_str(props.and_then(|m| m.get("delimiter"))).unwrap_or(":");

    if use_24h {
        let base = format!("{:02}{}{:02}", now.hour(), delimiter, now.minute());
        if show_seconds {
            return Some(format!("{}{}{:02}", base, delimiter, now.second()));
        }
        return Some(base);
    }

    let mut hour = now.hour() % 12;
    if hour == 0 {
        hour = 12;
    }
    let meridiem = if now.hour() >= 12 { "PM" } else { "AM" };
    let mut base = format!("{:02}{}{:02}", hour, delimiter, now.minute());
    if show_seconds {
        base.push_str(&format!("{}{:02}", delimiter, now.second()));
    }
    Some(format!("{} {}", base, meridiem))
}

fn infer_text_expr(object: &Value) -> Option<String> {
    let now = Local::now();
    let name = object
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let text_obj = object.get("text");
    let raw_value = text_obj
        .and_then(|v| match v {
            Value::String(s) => Some(s.as_str()),
            Value::Object(map) => map.get("value").and_then(|x| x.as_str()),
            _ => None,
        })
        .unwrap_or_default();

    let script = text_obj
        .and_then(|v| match v {
            Value::Object(map) => map.get("script").and_then(|x| x.as_str()),
            _ => None,
        })
        .unwrap_or_default()
        .to_ascii_lowercase();

    let props = text_obj
        .and_then(|v| v.as_object())
        .and_then(|m| m.get("scriptproperties"))
        .and_then(|v| v.as_object());

    if script.contains("gethours") && script.contains("getminutes") {
        if let Some(clock) = build_clock_text(props) {
            return Some(clock);
        }
        return Some(now.format("%H:%M").to_string());
    }
    if script.contains("getday")
        || script.contains("getdate")
        || script.contains("getmonth")
        || name.contains("day")
        || name.contains("date")
        || raw_value.eq_ignore_ascii_case("day")
        || raw_value.eq_ignore_ascii_case("<date>")
    {
        if let Some(v) = build_day_or_date_text(&name, &script, props) {
            return Some(v.trim().to_string());
        }
    }
    if script.contains("getmonth")
        || script.contains("getdate")
        || script.contains("getfullyear")
        || name.contains("date")
        || raw_value.eq_ignore_ascii_case("date")
    {
        return Some(escape_static_text(&now.format("%d/%m/%Y").to_string()));
    }

    if raw_value.is_empty() {
        return None;
    }
    Some(raw_value.to_string())
}

fn resolve_fontfile(
    object: &Value,
    pkg: &crate::scene_pkg::ScenePkg,
    font_cache_dir: &Path,
) -> Option<String> {
    let font_name = object.get("font").and_then(|v| v.as_str())?;
    let debug_font = std::env::var("KWE_DEBUG_TEXT_FONT").ok().as_deref() == Some("1");
    if font_name.eq_ignore_ascii_case("systemfont_arial") {
        if debug_font {
            eprintln!("[dbg-font] skipping system font '{}'", font_name);
        }
        return None;
    }
    let entry = match find_entry(pkg, font_name) {
        Some(e) => e,
        None => {
            if debug_font {
                eprintln!("[dbg-font] not found in pkg: '{}'", font_name);
            }
            return None;
        }
    };
    let out = match extract_entry_to_cache(pkg, &entry, font_cache_dir) {
        Ok(p) => p,
        Err(err) => {
            if debug_font {
                eprintln!(
                    "[dbg-font] extract failed for '{}': {}",
                    entry.filename, err
                );
            }
            return None;
        }
    };
    if debug_font {
        eprintln!("[dbg-font] using fontfile {}", out.display());
    }
    Some(escape_filter_value(&out.to_string_lossy()))
}

fn build_drawtext_for_object(
    object: &Value,
    scene_w: f32,
    scene_h: f32,
    pkg: &crate::scene_pkg::ScenePkg,
    font_cache_dir: &Path,
    text_cache_dir: &Path,
) -> Option<String> {
    if !visible_enabled(object) {
        return None;
    }

    let text_expr = infer_text_expr(object)?;
    let text_file = write_text_layer_file(text_cache_dir, object, &text_expr);
    let origin = object
        .get("origin")
        .and_then(|v| v.as_str())
        .and_then(parse_vec3_xy)?;
    let (effect_off_x, effect_off_y, effect_scale_y) = extract_transform_effect(object);
    let origin_x = origin.0 + effect_off_x;
    let origin_y = origin.1 + effect_off_y;
    let point_size = object
        .get("pointsize")
        .and_then(|v| v.as_f64())
        .unwrap_or(30.0)
        .max(8.0);
    let scale_y = object
        .get("scale")
        .and_then(|v| v.as_str())
        .and_then(parse_vec3)
        .map(|(_, y, _)| y)
        .unwrap_or(1.0)
        .abs()
        .clamp(0.25, 3.0);
    let user_scale = std::env::var("KWE_TEXT_SCALE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(4.4)
        .clamp(0.5, 6.0);
    let effective_scale_y = (scale_y as f64) * (effect_scale_y as f64);
    let point_size = point_size * effective_scale_y * user_scale;
    let (color, _r, _g, _b) = parse_color(object);

    let x_ratio = origin_x / scene_w;
    // Wallpaper Engine scene coordinates are bottom-origin in many scene packs.
    // drawtext y uses top-origin, so invert Y to keep text in expected screen region.
    let y_ratio = 1.0 - (origin_y / scene_h);
    let size_ratio = (point_size as f32 / scene_h).clamp(0.004, 0.2);
    let font_opt = resolve_fontfile(object, pkg, font_cache_dir)
        .map(|fontfile| format!(":fontfile={}", fontfile))
        .unwrap_or_default();
    let h_align = object
        .get("horizontalalign")
        .and_then(|v| v.as_str())
        .unwrap_or("center")
        .to_ascii_lowercase();
    let v_align = object
        .get("verticalalign")
        .and_then(|v| v.as_str())
        .unwrap_or("center")
        .to_ascii_lowercase();

    let x_base = format!("(w*{:.6})", x_ratio);
    let y_base = format!("(h*{:.6})", y_ratio);
    let x_expr = match h_align.as_str() {
        "left" => x_base.clone(),
        "right" => format!("{}-text_w", x_base),
        _ => format!("{}-(text_w/2)", x_base),
    };
    let y_expr = match v_align.as_str() {
        "top" => y_base.clone(),
        "bottom" => format!("{}-text_h", y_base),
        _ => format!("{}-(text_h/2)", y_base),
    };
    let text_input = if let Some(tf) = text_file {
        format!("textfile={}", tf)
    } else {
        format!("text={}", escape_static_text(&text_expr))
    };

    Some(format!(
        "drawtext={}:fontcolor={}:fontsize=h*{:.5}:x={}:y={}:borderw=0:shadowx=1:shadowy=1:shadowcolor=0x00000099{font}",
        text_input,
        color,
        size_ratio,
        x_expr,
        y_expr,
        font = font_opt
    ))
}

pub fn build_scene_drawtext_filter(root: &Path, max_layers: usize) -> Result<Option<String>> {
    let Some(pkg_path) = pick_pkg_path(root) else {
        return Ok(None);
    };
    let pkg = parse_scene_pkg(&pkg_path)?;
    let Some(scene_entry) =
        find_entry(&pkg, "scene.json").or_else(|| find_entry(&pkg, "gifscene.json"))
    else {
        return Ok(None);
    };

    let scene_json: Value = serde_json::from_slice(&read_entry_bytes(&pkg, &scene_entry)?)?;
    let (scene_w, scene_h) = parse_scene_size(&scene_json);
    let cache_key = cache_key_for_root(root);
    let font_cache_dir = default_scene_cache_root(&cache_key).join("text-fonts");
    let text_cache_dir = default_scene_cache_root(&cache_key).join("text-layers");

    let mut layers = Vec::new();
    let mut refresh_entries = Vec::new();
    if let Some(objects) = scene_json.get("objects").and_then(|v| v.as_array()) {
        for object in objects {
            let has_text = object.get("text").is_some() || object.get("font").is_some();
            if !has_text {
                continue;
            }
            if let Some(layer) = build_drawtext_for_object(
                object,
                scene_w,
                scene_h,
                &pkg,
                &font_cache_dir,
                &text_cache_dir,
            ) {
                layers.push(layer);
                if is_dynamic_text_object(object) {
                    let file_path = text_cache_dir.join(format!("obj_{}.txt", object_id(object)));
                    refresh_entries.push(RefreshEntry {
                        file_path: file_path.to_string_lossy().to_string(),
                        object: object.clone(),
                    });
                }
                if layers.len() >= max_layers.max(1) {
                    break;
                }
            }
        }
    }

    if layers.is_empty() {
        return Ok(None);
    }

    if !refresh_entries.is_empty() && fs::create_dir_all(&text_cache_dir).is_ok() {
        let spec = RefreshSpec {
            entries: refresh_entries,
        };
        let spec_json = serde_json::to_vec_pretty(&spec)?;
        fs::write(runtime_spec_path(root), spec_json).ok();
    }

    Ok(Some(format!("vf={}", layers.join(","))))
}

pub fn run_text_refresh(spec_path: &Path) -> Result<usize> {
    let raw = fs::read(spec_path)
        .with_context(|| format!("Failed reading refresh spec {}", spec_path.display()))?;
    let spec: RefreshSpec = serde_json::from_slice(&raw)
        .with_context(|| format!("Invalid refresh spec JSON {}", spec_path.display()))?;

    let mut updated = 0usize;
    for entry in &spec.entries {
        let text = infer_text_expr(&entry.object).unwrap_or_default();
        if fs::write(&entry.file_path, text).is_ok() {
            updated += 1;
        }
    }
    Ok(updated)
}

pub fn run_text_refresh_loop(spec_path: &Path, interval_seconds: u64) -> Result<()> {
    let interval = interval_seconds.max(1);
    loop {
        let _ = run_text_refresh(spec_path);
        thread::sleep(Duration::from_secs(interval));
    }
}

pub fn start_text_refresh_daemon(root: &Path, dry_run: bool) -> Result<()> {
    let spec = runtime_spec_path(root);
    if !spec.is_file() {
        return Ok(());
    }
    let raw = fs::read(&spec)
        .with_context(|| format!("Failed reading refresh spec {}", spec.display()))?;
    let parsed: RefreshSpec = serde_json::from_slice(&raw)
        .with_context(|| format!("Invalid refresh spec JSON {}", spec.display()))?;
    if parsed.entries.is_empty() {
        return Ok(());
    }

    let pid_file = updater_pid_path(root);
    if let Ok(pid_raw) = fs::read_to_string(&pid_file) {
        if let Ok(pid) = pid_raw.trim().parse::<u32>() {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
    }

    let exe = std::env::current_exe().context("Failed to resolve current executable path")?;
    if dry_run {
        println!(
            "[dry-run] {} text-refresh --spec {} --loop --interval-seconds 1",
            exe.display(),
            spec.display()
        );
        return Ok(());
    }

    let child = Command::new(exe)
        .arg("text-refresh")
        .arg("--spec")
        .arg(&spec)
        .arg("--loop")
        .arg("--interval-seconds")
        .arg("1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Failed to spawn text refresh daemon")?;

    if let Some(parent) = pid_file.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&pid_file, child.id().to_string()).ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_dynamic_patterns() {
        let day = serde_json::json!({
            "name": "D a y",
            "origin": "100 100 0",
            "pointsize": 40.0,
            "text": {"value":"DAY","script":"let date=new Date(); return day[date.getDay()];"}
        });
        let pkg = crate::scene_pkg::ScenePkg {
            path: PathBuf::from("/tmp/scene.pkg"),
            base_offset: 0,
            entries: vec![],
        };
        let s = build_drawtext_for_object(
            &day,
            3840.0,
            2160.0,
            &pkg,
            Path::new("/tmp"),
            Path::new("/tmp"),
        )
        .unwrap();
        assert!(!s.contains("%{localtime"));

        let time = serde_json::json!({
            "name": "Time",
            "origin": "100 100 0",
            "pointsize": 40.0,
            "text": {"value":"TIME","script":"date.getHours();date.getMinutes();"}
        });
        let s2 = build_drawtext_for_object(
            &time,
            3840.0,
            2160.0,
            &pkg,
            Path::new("/tmp"),
            Path::new("/tmp"),
        )
        .unwrap();
        assert!(!s2.contains("%{localtime"));
    }
}
