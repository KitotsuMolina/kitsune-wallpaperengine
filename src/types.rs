use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WallpaperType {
    Video,
    Scene,
    Web,
    Application,
    Unknown,
}

impl WallpaperType {
    pub fn from_str(input: &str) -> Self {
        match input.trim().to_ascii_lowercase().as_str() {
            "video" => Self::Video,
            "scene" => Self::Scene,
            "web" => Self::Web,
            "application" => Self::Application,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ProjectJson {
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub workshopid: String,
    #[serde(default)]
    pub general: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct SceneDiagnostics {
    pub has_scene_json: bool,
    pub has_gifscene_json: bool,
    pub has_scene_pkg: bool,
    pub has_gifscene_pkg: bool,
    pub supports_audio_processing: bool,
    pub supports_video: bool,
    pub best_video_candidate: Option<String>,
    pub best_candidate_preview_like: bool,
    pub best_candidate_is_gif: bool,
}

#[derive(Debug, Serialize)]
pub struct InspectOutput {
    pub root: String,
    pub wallpaper_type: WallpaperType,
    pub entry: Option<String>,
    pub title: Option<String>,
    pub workshopid: Option<String>,
    pub project_file_found: bool,
    pub scene: Option<SceneDiagnostics>,
}
