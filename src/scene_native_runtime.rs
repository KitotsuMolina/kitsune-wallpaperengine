use crate::scene_gpu_graph::{GpuPassSpec, SceneGpuGraph};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize)]
pub enum NativeSupportTier {
    Ready,
    Experimental,
    Unsupported,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativePassSupport {
    pub object_index: usize,
    pub object_id: u64,
    pub object_name: String,
    pub pass_index: usize,
    pub pass_shader: String,
    pub shader_family: String,
    pub primary_texture: Option<String>,
    pub tier: NativeSupportTier,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeDrawLayer {
    pub object_index: usize,
    pub object_id: u64,
    pub object_name: String,
    pub pass_index: usize,
    pub shader: String,
    pub shader_family: String,
    pub primary_texture: Option<String>,
    pub blend_mode: String,
    pub depth_test: String,
    pub depth_write: String,
    pub cull_mode: String,
    pub alpha: f32,
    pub brightness: f32,
    pub tint: [f32; 3],
    pub center_x: f32,
    pub center_y: f32,
    pub width: f32,
    pub height: f32,
    pub angle_rad: f32,
    pub parallax_depth: f32,
    pub visible: bool,
    pub shader_defines: Vec<String>,
    pub uniforms: BTreeMap<String, Value>,
    pub tier: NativeSupportTier,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeRuntimePlan {
    pub total_pass_nodes: usize,
    pub ready_nodes: usize,
    pub experimental_nodes: usize,
    pub unsupported_nodes: usize,
    pub ready_families: Vec<String>,
    pub experimental_families: Vec<String>,
    pub unsupported_families: Vec<String>,
    pub ready_draw_layers: usize,
    pub passes: Vec<NativePassSupport>,
    pub draw_layers: Vec<NativeDrawLayer>,
    pub notes: Vec<String>,
}

fn shader_family(shader: &str) -> String {
    let normalized = shader.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return "unknown".to_string();
    }
    if normalized.starts_with("effects/") {
        return "effects".to_string();
    }
    if normalized.starts_with("genericimage") {
        return "genericimage".to_string();
    }
    if normalized == "particle" || normalized.starts_with("genericparticle") {
        return "particle".to_string();
    }
    if normalized.starts_with("generic") {
        return "generic".to_string();
    }
    normalized
}

fn classify_family(shader: &str) -> (NativeSupportTier, String) {
    let family = shader_family(shader);
    match family.as_str() {
        "genericimage" => (
            NativeSupportTier::Ready,
            "direct textured quad family".to_string(),
        ),
        "flowimage" => (
            NativeSupportTier::Ready,
            "flow image approximated as textured layer (native parity in progress)".to_string(),
        ),
        "particle" => (
            NativeSupportTier::Ready,
            "sprite particle family".to_string(),
        ),
        "generic" => (
            NativeSupportTier::Experimental,
            "3D generic material family pending mesh/lighting parity".to_string(),
        ),
        "effects" | "flag" => (
            NativeSupportTier::Experimental,
            "effect family pending full compositor parity".to_string(),
        ),
        "unknown" => (
            NativeSupportTier::Unsupported,
            "shader field missing".to_string(),
        ),
        _ => (
            NativeSupportTier::Unsupported,
            "shader family not mapped yet".to_string(),
        ),
    }
}

fn parse_alpha(uniforms: &BTreeMap<String, Value>) -> f32 {
    uniforms
        .get("g_UserAlpha")
        .and_then(|v| match v {
            Value::Number(n) => n.as_f64().map(|x| x as f32),
            Value::String(s) => s.parse::<f32>().ok(),
            _ => None,
        })
        .unwrap_or(1.0)
        .clamp(0.0, 1.0)
}

fn parse_f32_from_value(v: &Value) -> Option<f32> {
    match v {
        Value::Number(n) => n.as_f64().map(|x| x as f32),
        Value::String(s) => s.parse::<f32>().ok(),
        _ => None,
    }
}

fn parse_color3(v: &Value) -> Option<[f32; 3]> {
    let s = v.as_str()?;
    let mut it = s.split_whitespace();
    let r = it.next()?.parse::<f32>().ok()?;
    let g = it.next()?.parse::<f32>().ok()?;
    let b = it.next()?.parse::<f32>().ok()?;
    Some([r, g, b])
}

fn parse_brightness(uniforms: &BTreeMap<String, Value>) -> f32 {
    uniforms
        .get("g_Brightness")
        .and_then(parse_f32_from_value)
        .unwrap_or(1.0)
        .max(0.05)
}

fn parse_tint(uniforms: &BTreeMap<String, Value>) -> [f32; 3] {
    if let Some(v) = uniforms.get("g_EmissiveColor").and_then(parse_color3) {
        return v;
    }
    let c1 = uniforms.get("g_Color1").and_then(parse_color3);
    let c2 = uniforms.get("g_Color2").and_then(parse_color3);
    match (c1, c2) {
        (Some(a), Some(b)) => [
            ((a[0] + b[0]) * 0.5).clamp(0.0, 2.0),
            ((a[1] + b[1]) * 0.5).clamp(0.0, 2.0),
            ((a[2] + b[2]) * 0.5).clamp(0.0, 2.0),
        ],
        (Some(a), None) => a,
        (None, Some(b)) => b,
        _ => [1.0, 1.0, 1.0],
    }
}

fn layer_rect_from_node(
    graph: &SceneGpuGraph,
    node: &crate::scene_gpu_graph::GpuEffectNode,
) -> (f32, f32, f32, f32, f32) {
    let scene_w = graph.scene_width.max(1) as f32;
    let scene_h = graph.scene_height.max(1) as f32;

    let origin = node
        .object_origin
        .unwrap_or([scene_w * 0.5, scene_h * 0.5, 0.0]);
    let scale = node.object_scale.unwrap_or([1.0, 1.0, 1.0]);
    let angles = node.object_angles.unwrap_or([0.0, 0.0, 0.0]);
    let default_size = if node.object_kind.eq_ignore_ascii_case("particle") {
        [scene_w * 0.28, scene_h * 0.28]
    } else {
        [scene_w, scene_h]
    };
    let base_size = node
        .object_size
        .or(node.object_asset_size)
        .unwrap_or(default_size);
    let raw_w = base_size[0] * scale[0].abs().max(0.01);
    let raw_h = base_size[1] * scale[1].abs().max(0.01);

    let (max_w, max_h) = if node.object_kind.eq_ignore_ascii_case("particle") {
        (scene_w * 0.70, scene_h * 0.70)
    } else {
        // Image layers can overscan a bit, but avoid huge extreme values.
        (scene_w * 1.05, scene_h * 1.05)
    };
    let width = raw_w.clamp(8.0, max_w.max(8.0));
    let height = raw_h.clamp(8.0, max_h.max(8.0));
    let center_x = origin[0].clamp(0.0, scene_w);
    let center_y = (scene_h - origin[1]).clamp(0.0, scene_h);
    let angle_rad = -angles[2];
    (center_x, center_y, width, height, angle_rad)
}

fn first_texture(pass: &GpuPassSpec) -> Option<String> {
    pass.textures
        .iter()
        .find(|t| !t.trim().is_empty())
        .cloned()
        .or_else(|| {
            pass.texture_refs
                .iter()
                .find(|t| !t.trim().is_empty())
                .cloned()
        })
}

fn with_texture_gate(
    tier: NativeSupportTier,
    reason: String,
    texture: Option<&String>,
    visible: bool,
) -> (NativeSupportTier, String) {
    if !visible {
        return (
            NativeSupportTier::Unsupported,
            "object marked as not visible in scene".to_string(),
        );
    }
    match tier {
        NativeSupportTier::Ready if texture.is_none() => (
            NativeSupportTier::Unsupported,
            "ready family but pass has no primary texture".to_string(),
        ),
        _ => (tier, reason),
    }
}

pub fn build_native_runtime_plan(graph: &SceneGpuGraph) -> NativeRuntimePlan {
    let mut ready = 0usize;
    let mut experimental = 0usize;
    let mut unsupported = 0usize;
    let mut ready_layers = 0usize;
    let mut ready_families = BTreeSet::<String>::new();
    let mut experimental_families = BTreeSet::<String>::new();
    let mut unsupported_families = BTreeSet::<String>::new();

    let mut passes = Vec::<NativePassSupport>::new();
    let mut draw_layers = Vec::<NativeDrawLayer>::new();

    for node in &graph.effect_nodes {
        for pass in &node.passes {
            let shader = if pass.shader.trim().is_empty() {
                node.pass_shader.clone()
            } else {
                pass.shader.clone()
            };
            let family = shader_family(&shader);
            let (base_tier, base_reason) = classify_family(&shader);
            let primary_texture = first_texture(pass);
            let (tier, reason) = with_texture_gate(
                base_tier,
                base_reason,
                primary_texture.as_ref(),
                node.object_visible,
            );

            match tier {
                NativeSupportTier::Ready => {
                    ready += 1;
                    ready_layers += 1;
                    ready_families.insert(family.clone());
                }
                NativeSupportTier::Experimental => {
                    experimental += 1;
                    experimental_families.insert(family.clone());
                }
                NativeSupportTier::Unsupported => {
                    unsupported += 1;
                    unsupported_families.insert(family.clone());
                }
            }

            passes.push(NativePassSupport {
                object_index: node.object_index,
                object_id: node.object_id,
                object_name: node.object_name.clone(),
                pass_index: pass.pass_index,
                pass_shader: shader.clone(),
                shader_family: family.clone(),
                primary_texture: primary_texture.clone(),
                tier: tier.clone(),
                reason,
            });

            let (center_x, center_y, width, height, angle_rad) = layer_rect_from_node(graph, node);
            let parallax_depth = node
                .object_parallax_depth
                .map(|v| (v[0] + v[1]) * 0.5)
                .unwrap_or(1.0);
            draw_layers.push(NativeDrawLayer {
                object_index: node.object_index,
                center_x,
                center_y,
                width,
                height,
                angle_rad,
                parallax_depth,
                visible: node.object_visible,
                object_id: node.object_id,
                object_name: node.object_name.clone(),
                pass_index: pass.pass_index,
                shader,
                shader_family: family,
                primary_texture,
                blend_mode: pass
                    .blending
                    .clone()
                    .unwrap_or_else(|| "normal".to_string()),
                depth_test: pass
                    .depth_test
                    .clone()
                    .unwrap_or_else(|| "disabled".to_string()),
                depth_write: pass
                    .depth_write
                    .clone()
                    .unwrap_or_else(|| "disabled".to_string()),
                cull_mode: pass
                    .cull_mode
                    .clone()
                    .unwrap_or_else(|| "nocull".to_string()),
                alpha: parse_alpha(&pass.effective_uniforms),
                brightness: parse_brightness(&pass.effective_uniforms),
                tint: parse_tint(&pass.effective_uniforms),
                shader_defines: pass.shader_defines.clone(),
                uniforms: pass.effective_uniforms.clone(),
                tier,
            });
        }
    }
    draw_layers.sort_by(|a, b| {
        a.object_index
            .cmp(&b.object_index)
            .then_with(|| a.pass_index.cmp(&b.pass_index))
            .then_with(|| {
                a.parallax_depth
                    .partial_cmp(&b.parallax_depth)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.object_id.cmp(&b.object_id))
    });

    let mut notes = Vec::<String>::new();
    notes.push(format!(
        "native support summary: ready={}, experimental={}, unsupported={}",
        ready, experimental, unsupported
    ));
    if ready_layers > 0 {
        notes.push(format!(
            "native draw layers ready for first renderer pass: {}",
            ready_layers
        ));
    }
    if ready == 0 && !passes.is_empty() {
        notes.push("no ready shader families detected; fallback transport recommended".to_string());
    }

    NativeRuntimePlan {
        total_pass_nodes: passes.len(),
        ready_nodes: ready,
        experimental_nodes: experimental,
        unsupported_nodes: unsupported,
        ready_families: ready_families.into_iter().collect(),
        experimental_families: experimental_families.into_iter().collect(),
        unsupported_families: unsupported_families.into_iter().collect(),
        ready_draw_layers: ready_layers,
        passes,
        draw_layers,
        notes,
    }
}
