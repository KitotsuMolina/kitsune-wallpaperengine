use crate::scene_pkg::{find_entry, parse_scene_pkg, read_entry_bytes};
use anyhow::{Result, bail};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Clone)]
pub struct GpuPassSpec {
    pub pass_index: usize,
    pub combos: Value,
    pub constant_shader_values: Value,
    pub textures: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct GpuEffectNode {
    pub object_id: u64,
    pub object_name: String,
    pub effect_file: String,
    pub effect_name: String,
    pub passes: Vec<GpuPassSpec>,
    pub shader_vert: Option<String>,
    pub shader_frag: Option<String>,
    pub material_json: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct SceneGpuGraph {
    pub pkg_path: String,
    pub scene_json_entry: String,
    pub scene_width: u32,
    pub scene_height: u32,
    pub effect_nodes: Vec<GpuEffectNode>,
    pub notes: Vec<String>,
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

fn exists_in_pkg(pkg: &crate::scene_pkg::ScenePkg, name: &str) -> Option<String> {
    find_entry(pkg, name).map(|e| e.filename)
}

fn infer_shader_material_candidates(effect_file: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let lower = effect_file.to_ascii_lowercase();
    if !lower.starts_with("effects/") || !lower.ends_with("/effect.json") {
        return (Vec::new(), Vec::new(), Vec::new());
    }

    let trimmed = effect_file
        .trim_start_matches("effects/")
        .trim_end_matches("/effect.json");
    let parts: Vec<&str> = trimmed.split('/').collect();
    if parts.is_empty() {
        return (Vec::new(), Vec::new(), Vec::new());
    }

    if parts.first().copied() == Some("workshop") && parts.len() >= 3 {
        let workshop_id = parts[1];
        let effect_name = parts[2];
        let vert = format!("shaders/workshop/{}/effects/{}.vert", workshop_id, effect_name);
        let frag = format!("shaders/workshop/{}/effects/{}.frag", workshop_id, effect_name);
        let mat = format!(
            "materials/workshop/{}/effects/{}.json",
            workshop_id, effect_name
        );
        return (vec![vert], vec![frag], vec![mat]);
    }

    let effect_name = parts.last().copied().unwrap_or_default();
    let vert = format!("shaders/effects/{}.vert", effect_name);
    let frag = format!("shaders/effects/{}.frag", effect_name);
    let mat = format!("materials/effects/{}.json", effect_name);
    (vec![vert], vec![frag], vec![mat])
}

pub fn build_scene_gpu_graph(root: &Path) -> Result<SceneGpuGraph> {
    let Some(pkg_path) = pick_pkg_path(root) else {
        bail!("No scene.pkg/gifscene.pkg found in {}", root.display());
    };
    let pkg = parse_scene_pkg(&pkg_path)?;
    let scene_entry =
        find_entry(&pkg, "scene.json").or_else(|| find_entry(&pkg, "gifscene.json"));
    let Some(scene_entry) = scene_entry else {
        bail!("No scene.json/gifscene.json in {}", pkg_path.display());
    };

    let scene_json: Value = serde_json::from_slice(&read_entry_bytes(&pkg, &scene_entry)?)?;
    let (scene_width, scene_height) = parse_scene_size(&scene_json);
    let mut effect_nodes = Vec::<GpuEffectNode>::new();

    if let Some(objects) = scene_json.get("objects").and_then(|v| v.as_array()) {
        for object in objects {
            let object_id = object.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
            let object_name = object
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let Some(effects) = object.get("effects").and_then(|v| v.as_array()) else {
                continue;
            };

            for effect in effects {
                let effect_file = effect
                    .get("file")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                if effect_file.is_empty() {
                    continue;
                }

                let effect_name = effect
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();

                let passes = effect
                    .get("passes")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .enumerate()
                            .map(|(idx, pass)| GpuPassSpec {
                                pass_index: idx,
                                combos: pass.get("combos").cloned().unwrap_or(Value::Null),
                                constant_shader_values: pass
                                    .get("constantshadervalues")
                                    .cloned()
                                    .unwrap_or(Value::Null),
                                textures: pass
                                    .get("textures")
                                    .and_then(|v| v.as_array())
                                    .map(|tarr| {
                                        tarr.iter()
                                            .filter_map(|t| t.as_str().map(|s| s.to_string()))
                                            .collect::<Vec<_>>()
                                    })
                                    .unwrap_or_default(),
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                let (vert_candidates, frag_candidates, mat_candidates) =
                    infer_shader_material_candidates(&effect_file);
                let shader_vert = vert_candidates
                    .iter()
                    .find_map(|c| exists_in_pkg(&pkg, c));
                let shader_frag = frag_candidates
                    .iter()
                    .find_map(|c| exists_in_pkg(&pkg, c));
                let material_json = mat_candidates
                    .iter()
                    .find_map(|c| exists_in_pkg(&pkg, c));

                effect_nodes.push(GpuEffectNode {
                    object_id,
                    object_name: object_name.clone(),
                    effect_file,
                    effect_name,
                    passes,
                    shader_vert,
                    shader_frag,
                    material_json,
                });
            }
        }
    }

    let notes = vec![
        "Graph ready for GPU backend scheduling (per-object effect chain)".to_string(),
        "Next step: compile shaders and bind textures/uniforms at runtime".to_string(),
    ];

    Ok(SceneGpuGraph {
        pkg_path: pkg_path.to_string_lossy().to_string(),
        scene_json_entry: scene_entry.filename,
        scene_width,
        scene_height,
        effect_nodes,
        notes,
    })
}
