use crate::asset_resolver::AssetResolver;
use crate::scene_script::{
    ScriptAssignment, apply_scene_scripts, collect_scene_user_properties, to_json_object,
};
use anyhow::{Result, bail};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Serialize, Clone)]
pub struct GpuPassSpec {
    pub pass_index: usize,
    pub shader: String,
    pub combos: Value,
    pub shader_defines: Vec<String>,
    pub blending: Option<String>,
    pub depth_test: Option<String>,
    pub depth_write: Option<String>,
    pub cull_mode: Option<String>,
    pub constant_shader_values: Value,
    pub user_shader_values: Value,
    pub textures: Vec<String>,
    pub texture_refs: Vec<String>,
    pub effective_uniforms: BTreeMap<String, Value>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ShaderUniformBinding {
    pub shader_stage: String,
    pub uniform: String,
    pub material_key: Option<String>,
    pub default_value: Option<Value>,
    pub metadata: Value,
}

#[derive(Debug, Serialize, Clone)]
pub struct GpuEffectNode {
    pub object_index: usize,
    pub object_id: u64,
    pub object_name: String,
    pub object_kind: String,
    pub object_asset: Option<String>,
    pub object_origin: Option<[f32; 3]>,
    pub object_scale: Option<[f32; 3]>,
    pub object_angles: Option<[f32; 3]>,
    pub object_size: Option<[f32; 2]>,
    pub object_asset_size: Option<[f32; 2]>,
    pub object_parallax_depth: Option<[f32; 2]>,
    pub object_visible: bool,
    pub effect_index: Option<usize>,
    pub instance_override: Value,
    pub effect_file: String,
    pub effect_name: String,
    pub material_asset: Option<String>,
    pub pass_shader: String,
    pub pass_index: usize,
    pub passes: Vec<GpuPassSpec>,
    pub shader_vert: Option<String>,
    pub shader_frag: Option<String>,
    pub material_json: Option<String>,
    pub uniform_bindings: Vec<ShaderUniformBinding>,
}

#[derive(Debug, Serialize, Clone)]
pub struct SceneGpuGraph {
    pub pkg_path: String,
    pub scene_json_entry: String,
    pub scene_width: u32,
    pub scene_height: u32,
    pub global_assets_root: Option<String>,
    pub user_properties: Value,
    pub script_properties: Value,
    pub script_assignments: Vec<ScriptAssignment>,
    pub effect_nodes: Vec<GpuEffectNode>,
    pub notes: Vec<String>,
}

fn parse_scene_size(scene_json: &Value) -> (u32, u32) {
    let width = scene_json
        .get("general")
        .and_then(|v| v.get("orthogonalprojection"))
        .and_then(|v| v.get("width"))
        .and_then(|v| v.as_u64())
        .unwrap_or(1920) as u32;
    let height = scene_json
        .get("general")
        .and_then(|v| v.get("orthogonalprojection"))
        .and_then(|v| v.get("height"))
        .and_then(|v| v.as_u64())
        .unwrap_or(1080) as u32;
    (width.max(1), height.max(1))
}

fn parse_vec3(value: &Value) -> Option<[f32; 3]> {
    let s = value.as_str()?;
    let mut it = s.split_whitespace();
    let x = it.next()?.parse::<f32>().ok()?;
    let y = it.next()?.parse::<f32>().ok()?;
    let z = it.next()?.parse::<f32>().ok()?;
    Some([x, y, z])
}

fn parse_vec2(value: &Value) -> Option<[f32; 2]> {
    let s = value.as_str()?;
    let mut it = s.split_whitespace();
    let x = it.next()?.parse::<f32>().ok()?;
    let y = it.next()?.parse::<f32>().ok()?;
    Some([x, y])
}

fn parse_visible_operand(token: &str, user_values: &BTreeMap<String, Value>) -> Option<Value> {
    let t = token.trim().trim_matches('(').trim_matches(')');
    if t.is_empty() {
        return None;
    }
    if t.eq_ignore_ascii_case("true") {
        return Some(Value::Bool(true));
    }
    if t.eq_ignore_ascii_case("false") {
        return Some(Value::Bool(false));
    }
    if (t.starts_with('\'') && t.ends_with('\'')) || (t.starts_with('"') && t.ends_with('"')) {
        return Some(Value::String(t[1..t.len() - 1].to_string()));
    }
    if let Ok(v) = t.parse::<f64>()
        && let Some(n) = serde_json::Number::from_f64(v)
    {
        return Some(Value::Number(n));
    }
    let key = t.trim_end_matches(".value");
    user_values.get(key).cloned()
}

fn value_as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

fn value_as_str(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(if *b {
            "true".to_string()
        } else {
            "false".to_string()
        }),
        _ => None,
    }
}

fn loosely_equal(left: &Value, right: &Value) -> bool {
    if let (Some(a), Some(b)) = (value_as_f64(left), value_as_f64(right)) {
        return (a - b).abs() < 1e-6;
    }
    if let (Some(a), Some(b)) = (value_as_str(left), value_as_str(right)) {
        return a == b;
    }
    left == right
}

fn eval_visible_predicate(expr: &str, user_values: &BTreeMap<String, Value>) -> Option<bool> {
    let e = expr.trim();
    if e.is_empty() {
        return None;
    }
    if e == "1" {
        return Some(true);
    }
    if e == "0" {
        return Some(false);
    }

    // String helpers seen in workshop conditions:
    // - style.value.contains('1')
    // - scheme.value.startsWith('dark')
    // - foo.endsWith('.png')
    for (marker, kind) in [
        (".contains(", "contains"),
        (".startsWith(", "startswith"),
        (".startswith(", "startswith"),
        (".endsWith(", "endswith"),
        (".endswith(", "endswith"),
    ] {
        if let Some(idx) = e.find(marker)
            && e.ends_with(')')
        {
            let lhs = &e[..idx];
            let arg = &e[(idx + marker.len())..e.len() - 1];
            let left = parse_visible_operand(lhs, user_values)?;
            let right = parse_visible_operand(arg, user_values)?;
            let l = value_as_str(&left)?;
            let r = value_as_str(&right)?;
            let result = match kind {
                "contains" => l.contains(&r),
                "startswith" => l.starts_with(&r),
                "endswith" => l.ends_with(&r),
                _ => false,
            };
            return Some(result);
        }
    }

    let ops = ["==", "!=", ">=", "<=", ">", "<"];
    for op in ops {
        if let Some(idx) = e.find(op) {
            let left = parse_visible_operand(&e[..idx], user_values)?;
            let right = parse_visible_operand(&e[(idx + op.len())..], user_values)?;
            return Some(match op {
                "==" => loosely_equal(&left, &right),
                "!=" => !loosely_equal(&left, &right),
                ">=" => value_as_f64(&left)? >= value_as_f64(&right)?,
                "<=" => value_as_f64(&left)? <= value_as_f64(&right)?,
                ">" => value_as_f64(&left)? > value_as_f64(&right)?,
                "<" => value_as_f64(&left)? < value_as_f64(&right)?,
                _ => false,
            });
        }
    }

    let key = e.trim_end_matches(".value");
    if let Some(v) = user_values.get(key) {
        return Some(match v {
            Value::Bool(b) => *b,
            Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
            Value::String(s) => {
                !s.trim().is_empty() && s != "0" && !s.eq_ignore_ascii_case("false")
            }
            _ => false,
        });
    }
    None
}

fn trim_outer_parens(mut expr: &str) -> &str {
    loop {
        let t = expr.trim();
        if !(t.starts_with('(') && t.ends_with(')')) {
            return t;
        }
        let mut depth = 0i32;
        let mut in_sq = false;
        let mut in_dq = false;
        let mut balanced_outer = false;
        for (i, ch) in t.char_indices() {
            match ch {
                '\'' if !in_dq => in_sq = !in_sq,
                '"' if !in_sq => in_dq = !in_dq,
                '(' if !in_sq && !in_dq => depth += 1,
                ')' if !in_sq && !in_dq => {
                    depth -= 1;
                    if depth == 0 {
                        balanced_outer = i == t.len() - 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        if balanced_outer {
            expr = &t[1..t.len() - 1];
            continue;
        }
        return t;
    }
}

fn find_top_level_op(expr: &str, op: &str) -> Option<usize> {
    let bytes = expr.as_bytes();
    if op.len() != 2 || bytes.len() < 2 {
        return None;
    }
    let mut depth = 0i32;
    let mut in_sq = false;
    let mut in_dq = false;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        let ch = bytes[i] as char;
        match ch {
            '\'' if !in_dq => in_sq = !in_sq,
            '"' if !in_sq => in_dq = !in_dq,
            '(' if !in_sq && !in_dq => depth += 1,
            ')' if !in_sq && !in_dq => depth -= 1,
            _ => {}
        }
        if !in_sq
            && !in_dq
            && depth == 0
            && bytes[i] == op.as_bytes()[0]
            && bytes[i + 1] == op.as_bytes()[1]
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn eval_visible_expr(expr: &str, user_values: &BTreeMap<String, Value>) -> Option<bool> {
    let mut e = trim_outer_parens(expr).trim();
    if e.is_empty() {
        return None;
    }
    let mut negate = false;
    while e.starts_with('!') {
        negate = !negate;
        e = trim_outer_parens(e[1..].trim());
    }
    if e.is_empty() {
        return None;
    }

    let base = if let Some(idx) = find_top_level_op(e, "||") {
        let left = &e[..idx];
        let right = &e[idx + 2..];
        Some(
            eval_visible_expr(left, user_values).unwrap_or(false)
                || eval_visible_expr(right, user_values).unwrap_or(false),
        )
    } else if let Some(idx) = find_top_level_op(e, "&&") {
        let left = &e[..idx];
        let right = &e[idx + 2..];
        Some(
            eval_visible_expr(left, user_values).unwrap_or(false)
                && eval_visible_expr(right, user_values).unwrap_or(false),
        )
    } else {
        eval_visible_predicate(e, user_values)
    };

    base.map(|v| if negate { !v } else { v })
}

fn eval_visible_condition(
    cond: &str,
    user_name: Option<&str>,
    user_values: &BTreeMap<String, Value>,
) -> Option<bool> {
    let base = cond.trim();
    if base.is_empty() {
        return None;
    }
    let normalized = if (base == "0" || base == "1") && user_name.is_some() {
        format!("{}.value=={}", user_name.unwrap_or_default(), base)
    } else {
        base.to_string()
    };
    eval_visible_expr(&normalized, user_values)
}

fn parse_object_visible(value: Option<&Value>, user_values: &BTreeMap<String, Value>) -> bool {
    let Some(v) = value else {
        return true;
    };
    if let Some(b) = v.as_bool() {
        return b;
    }
    if let Some(obj) = v.as_object() {
        let fallback_value = obj.get("value").and_then(|x| x.as_bool());
        if let Some(user_obj) = obj.get("user").and_then(|u| u.as_object()) {
            let condition = user_obj
                .get("condition")
                .and_then(|x| x.as_str())
                .unwrap_or_default();
            let user_name = user_obj.get("name").and_then(|x| x.as_str());
            if let Some(result) = eval_visible_condition(condition, user_name, user_values) {
                return result;
            }
        }
        if let Some(user_name) = obj.get("user").and_then(|u| u.as_str()) {
            if let Some(result) = eval_visible_condition(user_name, None, user_values) {
                return result;
            }
        }
        if let Some(b) = obj.get("value").and_then(|x| x.as_bool()) {
            return b;
        }
        if let Some(fb) = fallback_value {
            return fb;
        }
    }
    true
}

fn parse_width_height_from_object_data(v: &Value) -> Option<[f32; 2]> {
    let width = v.get("width").and_then(|x| x.as_f64())? as f32;
    let height = v.get("height").and_then(|x| x.as_f64())? as f32;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some([width, height])
}

fn dedup_preserve(mut values: Vec<String>) -> Vec<String> {
    let mut out = Vec::<String>::new();
    for v in values.drain(..) {
        if v.is_empty() || out.iter().any(|x| x == &v) {
            continue;
        }
        out.push(v);
    }
    out
}

fn parse_json_asset(resolver: &AssetResolver, path: &str) -> Option<(Value, String)> {
    let asset = resolver.resolve(path)?;
    let json: Value = serde_json::from_slice(&asset.bytes).ok()?;
    Some((json, asset.resolved_path))
}

fn resolve_user_bound_value(v: &Value, user_values: &BTreeMap<String, Value>) -> Value {
    if let Some(obj) = v.as_object() {
        if let Some(user) = obj.get("user").and_then(|u| u.as_str()) {
            return user_values
                .get(user)
                .cloned()
                .or_else(|| obj.get("value").cloned())
                .unwrap_or(Value::Null);
        }

        let mut out = serde_json::Map::new();
        for (k, child) in obj {
            out.insert(k.clone(), resolve_user_bound_value(child, user_values));
        }
        return Value::Object(out);
    }
    if let Some(arr) = v.as_array() {
        return Value::Array(
            arr.iter()
                .map(|x| resolve_user_bound_value(x, user_values))
                .collect(),
        );
    }
    v.clone()
}

fn apply_instance_override_uniforms(
    uniforms: &mut BTreeMap<String, Value>,
    instance_override: &Value,
) {
    let Some(map) = instance_override.as_object() else {
        return;
    };
    if let Some(alpha) = map.get("alpha") {
        uniforms.insert("g_UserAlpha".to_string(), alpha.clone());
    }
    if let Some(brightness) = map.get("brightness") {
        uniforms.insert("g_Brightness".to_string(), brightness.clone());
    }
    if let Some(color) = map.get("color") {
        uniforms.insert("g_EmissiveColor".to_string(), color.clone());
    }
    if let Some(count) = map.get("count") {
        uniforms.insert("instance_count".to_string(), count.clone());
    }
    if let Some(size) = map.get("size") {
        uniforms.insert("instance_size".to_string(), size.clone());
    }
}

fn shader_candidates(shader: &str, ext: &str) -> Vec<String> {
    let s = shader.trim();
    if s.is_empty() {
        return Vec::new();
    }

    let mut cands = Vec::<String>::new();
    if s.ends_with(&format!(".{ext}")) {
        cands.push(s.to_string());
    } else if !s.ends_with(".vert") && !s.ends_with(".frag") {
        cands.push(format!("{s}.{ext}"));
        cands.push(format!("shaders/{s}.{ext}"));
        cands.push(format!("assets/shaders/{s}.{ext}"));

        if s.starts_with("effects/") {
            cands.push(format!("{s}/shaders/{s}.{ext}"));
            cands.push(format!("assets/{s}/shaders/{s}.{ext}"));
        }
        if let Some(rest) = s.strip_prefix("effects/workshop/") {
            let mut it = rest.splitn(2, '/');
            if let (Some(workshop_id), Some(effect_name)) = (it.next(), it.next()) {
                cands.push(format!(
                    "shaders/workshop/{workshop_id}/effects/{effect_name}.{ext}"
                ));
                cands.push(format!(
                    "assets/shaders/workshop/{workshop_id}/effects/{effect_name}.{ext}"
                ));
                cands.push(format!(
                    "workshop/{workshop_id}/shaders/effects/{effect_name}.{ext}"
                ));
            }
        }
    }

    dedup_preserve(cands)
}

fn texture_candidates(token: &str) -> Vec<String> {
    let t = token.trim();
    if t.is_empty() {
        return Vec::new();
    }

    let mut cands = Vec::<String>::new();
    let has_ext = Path::new(t).extension().is_some();

    if has_ext {
        cands.push(t.to_string());
        cands.push(format!("materials/{t}"));
        cands.push(format!("assets/materials/{t}"));
        return dedup_preserve(cands);
    }

    let exts = [
        "tex", "tex-json", "png", "jpg", "jpeg", "webp", "bmp", "tga", "gif",
    ];
    for prefix in ["", "materials/", "assets/materials/"] {
        cands.push(format!("{prefix}{t}"));
        for ext in exts {
            cands.push(format!("{prefix}{t}.{ext}"));
        }
    }

    dedup_preserve(cands)
}

fn parse_uniform_meta_from_shader(src: &str, stage: &str) -> Vec<ShaderUniformBinding> {
    let mut out = Vec::<ShaderUniformBinding>::new();

    for raw_line in src.lines() {
        let line = raw_line.trim();
        if !line.starts_with("uniform ") || !line.contains("//") || !line.contains('{') {
            continue;
        }

        let Some((left, right)) = line.split_once("//") else {
            continue;
        };
        let Some(before_semicolon) = left.split(';').next() else {
            continue;
        };
        let uniform = before_semicolon
            .split_whitespace()
            .last()
            .unwrap_or_default()
            .trim();
        if uniform.is_empty() {
            continue;
        }

        let Some(json_start) = right.find('{') else {
            continue;
        };
        let Some(json_end) = right.rfind('}') else {
            continue;
        };
        if json_end < json_start {
            continue;
        }

        let json_text = &right[json_start..=json_end];
        let Ok(meta) = serde_json::from_str::<Value>(json_text) else {
            continue;
        };

        let material_key = meta
            .get("material")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let default_value = meta.get("default").cloned();

        out.push(ShaderUniformBinding {
            shader_stage: stage.to_string(),
            uniform: uniform.to_string(),
            material_key,
            default_value,
            metadata: meta,
        });
    }

    out
}

fn combos_to_shader_defines(combos: &Value) -> Vec<String> {
    let Some(map) = combos.as_object() else {
        return Vec::new();
    };

    let mut out = Vec::<String>::new();
    for (k, v) in map {
        let key = k.trim().to_string();
        if key.is_empty() {
            continue;
        }
        match v {
            Value::Bool(true) => out.push(key),
            Value::Bool(false) => {}
            Value::Number(n) => {
                if let Some(iv) = n.as_i64() {
                    if iv != 0 {
                        out.push(format!("{key}={iv}"));
                    }
                } else if let Some(fv) = n.as_f64()
                    && fv != 0.0
                {
                    out.push(format!("{key}={fv}"));
                }
            }
            Value::String(s) if !s.trim().is_empty() => {
                out.push(format!("{key}={}", s.trim()));
            }
            _ => {}
        }
    }
    out.sort();
    out
}

fn material_key_to_uniform(material_key: &str) -> Option<&'static str> {
    let normalized: String = material_key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    match normalized.as_str() {
        "alpha" => Some("g_UserAlpha"),
        "bright" | "brightness" => Some("g_Brightness"),
        "power" => Some("g_Power"),
        "scroll1x" | "scrollx" => Some("g_ScrollX"),
        "scroll1y" | "scrolly" => Some("g_ScrollY"),
        "scroll2x" => Some("g_Scroll2X"),
        "scroll2y" => Some("g_Scroll2Y"),
        "color1" => Some("g_Color1"),
        "color2" => Some("g_Color2"),
        "emissivecolor" => Some("g_EmissiveColor"),
        "emissivebrightness" => Some("g_EmissiveBrightness"),
        "metallic" => Some("g_Metallic"),
        "roughness" => Some("g_Roughness"),
        "reflectivity" => Some("g_Reflectivity"),
        "speed" | "flowspeed" => Some("g_FlowSpeed"),
        "amount" | "flowamp" => Some("g_FlowAmp"),
        _ => None,
    }
}

fn resolve_uniform_values(
    pass: &Value,
    uniform_bindings: &[ShaderUniformBinding],
    user_values: &BTreeMap<String, Value>,
    script_values: &BTreeMap<String, Value>,
) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::<String, Value>::new();

    let constant = pass
        .get("constantshadervalues")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let user_bindings = pass
        .get("usershadervalues")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    for ub in uniform_bindings {
        let Some(material_key) = ub.material_key.as_ref() else {
            continue;
        };

        if let Some(value) = constant.get(material_key) {
            out.insert(ub.uniform.clone(), value.clone());
            continue;
        }

        if let Some(user_key) = user_bindings.get(material_key).and_then(|v| v.as_str())
            && let Some(value) = user_values.get(user_key)
        {
            out.insert(ub.uniform.clone(), value.clone());
            continue;
        }

        let reverse_user_key = user_bindings
            .iter()
            .find_map(|(k, v)| (v.as_str() == Some(material_key)).then(|| k.to_string()));
        if let Some(user_key) = reverse_user_key
            && let Some(value) = user_values.get(&user_key)
        {
            out.insert(ub.uniform.clone(), value.clone());
            continue;
        }

        if let Some(value) = script_values.get(material_key) {
            out.insert(ub.uniform.clone(), value.clone());
            continue;
        }

        if let Some(default) = &ub.default_value {
            out.insert(ub.uniform.clone(), default.clone());
        }
    }

    for (material_key, value) in &constant {
        let Some(uniform) = material_key_to_uniform(material_key) else {
            continue;
        };
        out.entry(uniform.to_string())
            .or_insert_with(|| value.clone());
    }

    for (material_key, bound) in &user_bindings {
        let Some(uniform) = material_key_to_uniform(material_key) else {
            continue;
        };
        if out.contains_key(uniform) {
            continue;
        }
        if let Some(user_key) = bound.as_str()
            && let Some(value) = user_values.get(user_key)
        {
            out.insert(uniform.to_string(), value.clone());
        }
    }

    for (script_key, value) in script_values {
        let Some(uniform) = material_key_to_uniform(script_key) else {
            continue;
        };
        out.entry(uniform.to_string())
            .or_insert_with(|| value.clone());
    }

    out
}

fn merge_pass_overrides(base_pass: &Value, override_pass: Option<&Value>) -> Value {
    fn deep_merge(base: &Value, ov: &Value) -> Value {
        match (base, ov) {
            (Value::Object(b), Value::Object(o)) => {
                let mut out = b.clone();
                for (k, v_ov) in o {
                    if let Some(v_base) = out.get(k) {
                        out.insert(k.clone(), deep_merge(v_base, v_ov));
                    } else {
                        out.insert(k.clone(), v_ov.clone());
                    }
                }
                Value::Object(out)
            }
            (_, Value::Null) => base.clone(),
            (_, _) => ov.clone(),
        }
    }

    match override_pass {
        Some(ov) => deep_merge(base_pass, ov),
        None => base_pass.clone(),
    }
}

fn effect_override_for_material_pass(
    effect_overrides: &[Value],
    effect_pass_idx: usize,
    material_pass_idx: usize,
    sequential_cursor: &mut usize,
    material_pass_count: usize,
) -> Option<Value> {
    if *sequential_cursor + material_pass_count <= effect_overrides.len() {
        let ov = effect_overrides
            .get(*sequential_cursor + material_pass_idx)
            .cloned();
        if material_pass_idx + 1 == material_pass_count {
            *sequential_cursor += material_pass_count;
        }
        return ov;
    }

    let fallback = effect_overrides.get(effect_pass_idx)?;
    if let Some(local) = fallback
        .get("passes")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.get(material_pass_idx))
    {
        return Some(local.clone());
    }

    (material_pass_idx == 0).then(|| fallback.clone())
}

pub fn build_scene_gpu_graph(root: &Path) -> Result<SceneGpuGraph> {
    let resolver = AssetResolver::new(root)?;

    let Some(scene_asset) = resolver
        .resolve("scene.json")
        .or_else(|| resolver.resolve("gifscene.json"))
    else {
        bail!("No scene.json/gifscene.json found in {}", root.display());
    };

    let scene_json: Value = serde_json::from_slice(&scene_asset.bytes)?;
    let project_json = resolver
        .resolve("project.json")
        .and_then(|v| serde_json::from_slice::<Value>(&v.bytes).ok());

    let (scene_width, scene_height) = parse_scene_size(&scene_json);
    let mut notes = Vec::<String>::new();

    let user_values = collect_scene_user_properties(&scene_json, project_json.as_ref());
    let script_eval = apply_scene_scripts(&scene_json, &user_values);
    notes.extend(script_eval.notes.clone());
    let mut script_values = BTreeMap::<String, Value>::new();
    for assignment in &script_eval.assignments {
        if let Some(value) = &assignment.resolved_value {
            script_values.insert(assignment.target_property.clone(), value.clone());
        }
    }

    let mut effect_nodes = Vec::<GpuEffectNode>::new();
    if let Some(objects) = scene_json.get("objects").and_then(|v| v.as_array()) {
        for (object_index, object) in objects.iter().enumerate() {
            let object_id = object.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
            let object_name = object
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            let (object_kind, object_asset_ref) =
                if let Some(image) = object.get("image").and_then(|v| v.as_str()) {
                    ("image".to_string(), image.to_string())
                } else if let Some(particle) = object.get("particle").and_then(|v| v.as_str()) {
                    ("particle".to_string(), particle.to_string())
                } else {
                    continue;
                };
            let object_origin = object.get("origin").and_then(parse_vec3);
            let object_scale = object.get("scale").and_then(parse_vec3);
            let object_angles = object.get("angles").and_then(parse_vec3);
            let object_size = object.get("size").and_then(parse_vec2);
            let object_parallax_depth = object.get("parallaxDepth").and_then(parse_vec2);
            let object_visible = parse_object_visible(object.get("visible"), &user_values);
            let instance_override = object
                .get("instanceoverride")
                .map(|v| resolve_user_bound_value(v, &user_values))
                .unwrap_or(Value::Null);

            let Some((object_data, object_asset_resolved)) =
                parse_json_asset(&resolver, &object_asset_ref)
            else {
                notes.push(format!(
                    "Object '{}' references missing asset: {}",
                    object_name, object_asset_ref
                ));
                continue;
            };
            let object_asset_size = parse_width_height_from_object_data(&object_data);

            let Some(material_ref) = object_data
                .get("material")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
            else {
                notes.push(format!(
                    "Object '{}' asset '{}' has no material",
                    object_name, object_asset_resolved
                ));
                continue;
            };

            let Some((material_data, material_asset_resolved)) =
                parse_json_asset(&resolver, &material_ref)
            else {
                notes.push(format!(
                    "Object '{}' material not found: {}",
                    object_name, material_ref
                ));
                continue;
            };

            let Some(passes) = material_data.get("passes").and_then(|v| v.as_array()) else {
                notes.push(format!(
                    "Material '{}' has no passes",
                    material_asset_resolved
                ));
                continue;
            };
            let mut pipeline_pass_index = 0usize;

            let mut push_material_passes =
                |passes_data: &[Value],
                 material_asset_resolved: &str,
                 effect_file: &str,
                 effect_name: &str,
                 effect_index: Option<usize>| {
                    for pass in passes_data {
                        let shader_name = pass
                            .get("shader")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();

                        let shader_vert = resolver
                            .resolve_first(&shader_candidates(&shader_name, "vert"))
                            .map(|a| a.resolved_path);
                        let shader_frag = resolver
                            .resolve_first(&shader_candidates(&shader_name, "frag"))
                            .map(|a| a.resolved_path);

                        let texture_refs = pass
                            .get("textures")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();

                        let mut textures = Vec::<String>::new();
                        for tex in &texture_refs {
                            let resolved = resolver
                                .resolve_first(&texture_candidates(tex))
                                .map(|a| a.resolved_path)
                                .unwrap_or_else(|| tex.to_string());
                            textures.push(resolved);
                        }

                        let mut uniform_bindings = Vec::<ShaderUniformBinding>::new();
                        if let Some(v) = &shader_vert
                            && let Some(asset) = resolver.resolve(v)
                            && let Ok(src) = String::from_utf8(asset.bytes)
                        {
                            uniform_bindings.extend(parse_uniform_meta_from_shader(&src, "vert"));
                        }
                        if let Some(v) = &shader_frag
                            && let Some(asset) = resolver.resolve(v)
                            && let Ok(src) = String::from_utf8(asset.bytes)
                        {
                            uniform_bindings.extend(parse_uniform_meta_from_shader(&src, "frag"));
                        }

                        let mut effective_uniforms = resolve_uniform_values(
                            pass,
                            &uniform_bindings,
                            &user_values,
                            &script_values,
                        );
                        apply_instance_override_uniforms(
                            &mut effective_uniforms,
                            &instance_override,
                        );

                        let pass_spec = GpuPassSpec {
                            pass_index: pipeline_pass_index,
                            shader: shader_name.clone(),
                            combos: pass.get("combos").cloned().unwrap_or(Value::Null),
                            shader_defines: combos_to_shader_defines(
                                &pass.get("combos").cloned().unwrap_or(Value::Null),
                            ),
                            blending: pass
                                .get("blending")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            depth_test: pass
                                .get("depthtest")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            depth_write: pass
                                .get("depthwrite")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            cull_mode: pass
                                .get("cullmode")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            constant_shader_values: pass
                                .get("constantshadervalues")
                                .cloned()
                                .unwrap_or(Value::Null),
                            user_shader_values: pass
                                .get("usershadervalues")
                                .cloned()
                                .unwrap_or(Value::Null),
                            textures,
                            texture_refs,
                            effective_uniforms,
                        };

                        effect_nodes.push(GpuEffectNode {
                            object_index,
                            object_id,
                            object_name: object_name.clone(),
                            object_kind: object_kind.clone(),
                            object_asset: Some(object_asset_resolved.clone()),
                            object_origin,
                            object_scale,
                            object_angles,
                            object_size,
                            object_asset_size,
                            object_parallax_depth,
                            object_visible,
                            effect_index,
                            instance_override: instance_override.clone(),
                            effect_file: effect_file.to_string(),
                            effect_name: effect_name.to_string(),
                            material_asset: Some(material_asset_resolved.to_string()),
                            pass_shader: shader_name.clone(),
                            pass_index: pipeline_pass_index,
                            passes: vec![pass_spec],
                            shader_vert,
                            shader_frag,
                            material_json: Some(material_asset_resolved.to_string()),
                            uniform_bindings,
                        });
                        pipeline_pass_index += 1;
                    }
                };

            let base_pass_values = passes.to_vec();
            push_material_passes(
                &base_pass_values,
                &material_asset_resolved,
                &material_asset_resolved,
                "base-material",
                None,
            );

            if let Some(object_effects) = object.get("effects").and_then(|v| v.as_array()) {
                let mut sequential_override_cursor = 0usize;
                for (effect_idx, effect) in object_effects.iter().enumerate() {
                    let effect_visible = parse_object_visible(effect.get("visible"), &user_values);
                    if !effect_visible {
                        continue;
                    }
                    let Some(effect_file_ref) = effect.get("file").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let Some((effect_data, effect_file_resolved)) =
                        parse_json_asset(&resolver, effect_file_ref)
                    else {
                        notes.push(format!(
                            "Object '{}' effect not found: {}",
                            object_name, effect_file_ref
                        ));
                        continue;
                    };
                    let Some(effect_passes) = effect_data.get("passes").and_then(|v| v.as_array())
                    else {
                        continue;
                    };
                    let effect_overrides = effect
                        .get("passes")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();

                    for (effect_pass_idx, effect_pass) in effect_passes.iter().enumerate() {
                        let Some(effect_material_ref) =
                            effect_pass.get("material").and_then(|v| v.as_str())
                        else {
                            continue;
                        };
                        let Some((effect_material_data, effect_material_resolved)) =
                            parse_json_asset(&resolver, effect_material_ref)
                        else {
                            notes.push(format!(
                                "Object '{}' effect material not found: {}",
                                object_name, effect_material_ref
                            ));
                            continue;
                        };
                        let Some(effect_material_passes) = effect_material_data
                            .get("passes")
                            .and_then(|v| v.as_array())
                        else {
                            continue;
                        };

                        let merged = effect_material_passes
                            .iter()
                            .enumerate()
                            .map(|(mat_idx, base_pass)| {
                                let ov = effect_override_for_material_pass(
                                    &effect_overrides,
                                    effect_pass_idx,
                                    mat_idx,
                                    &mut sequential_override_cursor,
                                    effect_material_passes.len(),
                                );
                                merge_pass_overrides(base_pass, ov.as_ref())
                            })
                            .collect::<Vec<_>>();

                        let effect_name =
                            effect_file_resolved.rsplit('/').nth(1).unwrap_or("effect");
                        push_material_passes(
                            &merged,
                            &effect_material_resolved,
                            &effect_file_resolved,
                            effect_name,
                            Some(effect_idx),
                        );
                    }
                }
            }
        }
    }

    if effect_nodes.is_empty() {
        notes.push("No material/pass nodes were generated from scene objects".to_string());
    } else {
        notes.push(format!(
            "Built {} material pass nodes from scene objects",
            effect_nodes.len()
        ));
    }
    notes.push("Graph source: scene -> model/particle -> material -> passes -> shader".to_string());

    Ok(SceneGpuGraph {
        pkg_path: resolver.pkg_path().unwrap_or_default(),
        scene_json_entry: scene_asset.resolved_path,
        scene_width,
        scene_height,
        global_assets_root: resolver.global_assets_root(),
        user_properties: to_json_object(&user_values),
        script_properties: to_json_object(&script_values),
        script_assignments: script_eval.assignments,
        effect_nodes,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn visible_condition_supports_or_and_parens() {
        let mut users = BTreeMap::<String, Value>::new();
        users.insert("style".to_string(), Value::String("0".to_string()));
        users.insert("glow".to_string(), Value::from(2.0));

        let c1 = "(style.value=='1' || style.value=='0') && glow>1";
        assert_eq!(eval_visible_condition(c1, None, &users), Some(true));

        let c2 = "style.value=='1' || (glow<1 && style.value=='0')";
        assert_eq!(eval_visible_condition(c2, None, &users), Some(false));
    }

    #[test]
    fn visible_condition_supports_not_operator() {
        let mut users = BTreeMap::<String, Value>::new();
        users.insert("style".to_string(), Value::String("0".to_string()));
        users.insert("enabled".to_string(), Value::Bool(false));

        assert_eq!(
            eval_visible_condition("!(style.value=='1')", None, &users),
            Some(true)
        );
        assert_eq!(
            eval_visible_condition("!!(style.value=='0')", None, &users),
            Some(true)
        );
        assert_eq!(eval_visible_condition("!enabled", None, &users), Some(true));
        assert_eq!(
            eval_visible_condition("!(enabled || style.value=='1')", None, &users),
            Some(true)
        );
    }

    #[test]
    fn visible_condition_supports_string_helpers() {
        let mut users = BTreeMap::<String, Value>::new();
        users.insert(
            "style".to_string(),
            Value::String("classic_dark".to_string()),
        );
        users.insert(
            "asset".to_string(),
            Value::String("bg_layer.png".to_string()),
        );

        assert_eq!(
            eval_visible_condition("style.value.contains('dark')", None, &users),
            Some(true)
        );
        assert_eq!(
            eval_visible_condition("style.value.startsWith('classic')", None, &users),
            Some(true)
        );
        assert_eq!(
            eval_visible_condition("asset.value.endsWith('.png')", None, &users),
            Some(true)
        );
    }

    #[test]
    fn fallback_uniform_mapping_works_without_shader_metadata() {
        let pass: Value = serde_json::json!({
            "constantshadervalues": {
                "Alpha": 0.4,
                "Bright": 1.2
            },
            "usershadervalues": {
                "color1": "schemecolor"
            }
        });
        let mut users = BTreeMap::<String, Value>::new();
        users.insert(
            "schemecolor".to_string(),
            Value::String("1 0 0".to_string()),
        );

        let uniforms = resolve_uniform_values(&pass, &[], &users, &BTreeMap::new());
        assert_eq!(uniforms.get("g_UserAlpha"), Some(&Value::from(0.4)));
        assert_eq!(uniforms.get("g_Brightness"), Some(&Value::from(1.2)));
        assert_eq!(
            uniforms.get("g_Color1"),
            Some(&Value::String("1 0 0".to_string()))
        );
    }

    #[test]
    fn shader_candidates_include_workshop_convention() {
        let cands = shader_candidates("effects/workshop/123456/scroll", "vert");
        assert!(
            cands
                .iter()
                .any(|c| c.contains("shaders/workshop/123456/effects/scroll.vert"))
        );
    }

    #[test]
    fn merge_pass_overrides_applies_object_effect_overrides() {
        let base = serde_json::json!({
            "shader": "effects/scroll",
            "textures": ["a"],
            "combos": { "X": 1 }
        });
        let ov = serde_json::json!({
            "textures": [null, "util/noise"],
            "constantshadervalues": { "strength": 0.5 }
        });
        let merged = merge_pass_overrides(&base, Some(&ov));
        let textures = merged
            .get("textures")
            .and_then(|v| v.as_array())
            .expect("textures");
        assert_eq!(textures.len(), 2);
        assert_eq!(
            merged.get("shader"),
            Some(&Value::String("effects/scroll".to_string()))
        );
        assert!(merged.get("constantshadervalues").is_some());
    }

    #[test]
    fn merge_pass_overrides_is_deep_for_objects() {
        let base = serde_json::json!({
            "combos": { "A": 1, "B": 2 },
            "constantshadervalues": { "x": 1, "y": 2 }
        });
        let ov = serde_json::json!({
            "combos": { "B": 5, "C": 8 },
            "constantshadervalues": { "y": 9 }
        });
        let merged = merge_pass_overrides(&base, Some(&ov));
        assert_eq!(merged["combos"]["A"], Value::from(1));
        assert_eq!(merged["combos"]["B"], Value::from(5));
        assert_eq!(merged["combos"]["C"], Value::from(8));
        assert_eq!(merged["constantshadervalues"]["x"], Value::from(1));
        assert_eq!(merged["constantshadervalues"]["y"], Value::from(9));
    }

    #[test]
    fn effect_override_mapping_supports_sequential_pass_overrides() {
        let overrides = vec![
            serde_json::json!({"constantshadervalues": {"p": 1}}),
            serde_json::json!({"constantshadervalues": {"p": 2}}),
            serde_json::json!({"constantshadervalues": {"p": 3}}),
        ];
        let mut cursor = 0usize;
        let a0 = effect_override_for_material_pass(&overrides, 0, 0, &mut cursor, 2).expect("a0");
        let a1 = effect_override_for_material_pass(&overrides, 0, 1, &mut cursor, 2).expect("a1");
        let b0 = effect_override_for_material_pass(&overrides, 1, 0, &mut cursor, 1).expect("b0");
        assert_eq!(a0["constantshadervalues"]["p"], Value::from(1));
        assert_eq!(a1["constantshadervalues"]["p"], Value::from(2));
        assert_eq!(b0["constantshadervalues"]["p"], Value::from(3));
    }

    #[test]
    fn effect_visible_user_condition_is_evaluated() {
        let mut users = BTreeMap::<String, Value>::new();
        users.insert("style".to_string(), Value::String("1".to_string()));
        let visible = serde_json::json!({
            "user": {"name":"style","condition":"1"},
            "value": false
        });
        assert!(parse_object_visible(Some(&visible), &users));
    }
}
