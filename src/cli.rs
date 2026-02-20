use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlaybackProfile {
    Performance,
    Balanced,
    Quality,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ProxyPreset {
    Eco,
    Balanced,
    Ultra,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum GpuTransport {
    Mp4Proxy,
    NativeRealtime,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AudioBarsSource {
    Pulse,
    Synth,
}

#[derive(Parser)]
#[command(name = "kitsune-livewallpaper")]
#[command(about = "Kitsune custom wallpaper engine MVP")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    InstallDependencies,
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    StartConfig {
        #[arg(long, default_value_os_t = default_config_path())]
        config: PathBuf,
        #[arg(long)]
        dry_run: bool,
    },
    ServiceAutostart {
        #[command(subcommand)]
        command: ServiceAutostartCommands,
    },
    Inspect {
        wallpaper: String,
        #[arg(long, default_value_os_t = default_downloads_root())]
        downloads_root: PathBuf,
    },
    SceneDump {
        wallpaper: String,
        #[arg(long, default_value_os_t = default_downloads_root())]
        downloads_root: PathBuf,
        #[arg(long)]
        full: bool,
    },
    ScenePlan {
        wallpaper: String,
        #[arg(long, default_value_os_t = default_downloads_root())]
        downloads_root: PathBuf,
    },
    SceneAudioPlan {
        wallpaper: String,
        #[arg(long, default_value_os_t = default_downloads_root())]
        downloads_root: PathBuf,
    },
    LibraryScan {
        #[arg(long, default_value_os_t = default_downloads_root())]
        downloads_root: PathBuf,
        #[arg(long, default_value_t = 20)]
        top_effects: usize,
        #[arg(long)]
        summary_only: bool,
    },
    LibraryRoadmap {
        #[arg(long, default_value_os_t = default_downloads_root())]
        downloads_root: PathBuf,
        #[arg(long, default_value_t = 15)]
        top_n: usize,
    },
    SceneRuntime {
        wallpaper: String,
        #[arg(long, default_value_os_t = default_downloads_root())]
        downloads_root: PathBuf,
        #[arg(long)]
        source: Option<String>,
        #[arg(long, default_value_t = 4)]
        seconds: u64,
        #[arg(long, default_value_t = 50)]
        frame_ms: u64,
        #[arg(long)]
        extract_music: bool,
    },
    SceneRender {
        wallpaper: String,
        #[arg(long, default_value_os_t = default_downloads_root())]
        downloads_root: PathBuf,
        #[arg(long)]
        source: Option<String>,
        #[arg(long, default_value_t = 4)]
        seconds: u64,
        #[arg(long, default_value_t = 50)]
        frame_ms: u64,
    },
    SceneGpuGraph {
        wallpaper: String,
        #[arg(long, default_value_os_t = default_downloads_root())]
        downloads_root: PathBuf,
    },
    SceneNativePlan {
        wallpaper: String,
        #[arg(long, default_value_os_t = default_downloads_root())]
        downloads_root: PathBuf,
    },
    SceneGpuPlay {
        wallpaper: String,
        #[arg(long)]
        monitor: String,
        #[arg(long, default_value_os_t = default_downloads_root())]
        downloads_root: PathBuf,
        #[arg(long)]
        keep_services: bool,
        #[arg(long = "service")]
        services: Vec<String>,
        #[arg(long)]
        source: Option<String>,
        #[arg(long, default_value_t = 4)]
        seconds: u64,
        #[arg(long, default_value_t = 50)]
        frame_ms: u64,
        #[arg(long)]
        mute_audio: bool,
        #[arg(long, value_enum, default_value_t = PlaybackProfile::Performance)]
        profile: PlaybackProfile,
        #[arg(long)]
        display_fps: Option<u32>,
        #[arg(long, default_value_t = true)]
        clock_overlay: bool,
        #[arg(long, default_value_t = true)]
        apply_kitsune_overlay: bool,
        #[arg(long, value_enum, default_value_t = GpuTransport::Mp4Proxy)]
        transport: GpuTransport,
        #[arg(long)]
        require_native: bool,
        #[arg(long, value_enum, default_value_t = AudioBarsSource::Pulse)]
        audio_bars_source: AudioBarsSource,
        #[arg(long, default_value_t = 2560)]
        proxy_width: u32,
        #[arg(long, default_value_t = 60)]
        proxy_fps: u32,
        #[arg(long, default_value_t = 20)]
        proxy_crf: u8,
        #[arg(long)]
        dry_run: bool,
    },
    TextRefresh {
        #[arg(long)]
        spec: PathBuf,
        #[arg(long = "loop", default_value_t = false)]
        loop_mode: bool,
        #[arg(long, default_value_t = 1)]
        interval_seconds: u64,
    },
    ScenePlay {
        wallpaper: String,
        #[arg(long)]
        monitor: String,
        #[arg(long, default_value_os_t = default_downloads_root())]
        downloads_root: PathBuf,
        #[arg(long)]
        keep_services: bool,
        #[arg(long = "service")]
        services: Vec<String>,
        #[arg(long)]
        source: Option<String>,
        #[arg(long, default_value_t = 4)]
        seconds: u64,
        #[arg(long, default_value_t = 50)]
        frame_ms: u64,
        #[arg(long)]
        mute_audio: bool,
        #[arg(long, value_enum, default_value_t = PlaybackProfile::Performance)]
        profile: PlaybackProfile,
        #[arg(long)]
        display_fps: Option<u32>,
        #[arg(long, default_value_t = true)]
        clock_overlay: bool,
        #[arg(long, value_enum, default_value_t = ProxyPreset::Balanced)]
        proxy_preset: ProxyPreset,
        #[arg(long)]
        auto_tune: bool,
        #[arg(long)]
        proxy_width: Option<u32>,
        #[arg(long)]
        proxy_fps: Option<u32>,
        #[arg(long)]
        proxy_crf: Option<u8>,
        #[arg(long)]
        no_proxy_optimize: bool,
        #[arg(long)]
        dry_run: bool,
    },
    VideoPlay {
        video: String,
        #[arg(long)]
        monitor: String,
        #[arg(long, default_value_os_t = default_downloads_root())]
        downloads_root: PathBuf,
        #[arg(long)]
        keep_services: bool,
        #[arg(long = "service")]
        services: Vec<String>,
        #[arg(long)]
        mute_audio: bool,
        #[arg(long, value_enum, default_value_t = PlaybackProfile::Quality)]
        profile: PlaybackProfile,
        #[arg(long)]
        display_fps: Option<u32>,
        #[arg(long, default_value_t = true)]
        seamless_loop: bool,
        #[arg(long, default_value_t = false)]
        loop_crossfade: bool,
        #[arg(long, default_value_t = 0.35)]
        loop_crossfade_seconds: f32,
        #[arg(long, default_value_t = true)]
        optimize: bool,
        #[arg(long, default_value_t = 3840)]
        proxy_width: u32,
        #[arg(long, default_value_t = 60)]
        proxy_fps: u32,
        #[arg(long, default_value_t = 16)]
        proxy_crf: u8,
        #[arg(long)]
        dry_run: bool,
    },
    AudioProbe {
        #[arg(long)]
        source: Option<String>,
        #[arg(long, default_value_t = 2)]
        seconds: u64,
    },
    AudioStream {
        #[arg(long)]
        source: Option<String>,
        #[arg(long, default_value_t = 2)]
        seconds: u64,
        #[arg(long, default_value_t = 50)]
        frame_ms: u64,
    },
    StopServices {
        #[arg(long = "service")]
        services: Vec<String>,
        #[arg(long)]
        dry_run: bool,
    },
    StartServices {
        #[arg(long = "service")]
        services: Vec<String>,
        #[arg(long)]
        dry_run: bool,
    },
    Apply {
        wallpaper: String,
        #[arg(long)]
        monitor: String,
        #[arg(long, default_value_os_t = default_downloads_root())]
        downloads_root: PathBuf,
        #[arg(long)]
        keep_services: bool,
        #[arg(long = "service")]
        services: Vec<String>,
        #[arg(long)]
        mute_audio: bool,
        #[arg(long, value_enum, default_value_t = PlaybackProfile::Balanced)]
        profile: PlaybackProfile,
        #[arg(long)]
        display_fps: Option<u32>,
        #[arg(long)]
        allow_scene_preview_fallback: bool,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub enum ConfigCommands {
    SetVideo {
        #[arg(long)]
        monitor: String,
        #[arg(long)]
        video: String,
        #[arg(long, default_value_os_t = default_downloads_root())]
        downloads_root: PathBuf,
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        keep_services: bool,
        #[arg(long)]
        mute_audio: bool,
        #[arg(long, value_enum, default_value_t = PlaybackProfile::Performance)]
        profile: PlaybackProfile,
        #[arg(long)]
        display_fps: Option<u32>,
        #[arg(long, default_value_t = true)]
        seamless_loop: bool,
        #[arg(long, default_value_t = false)]
        loop_crossfade: bool,
        #[arg(long, default_value_t = 0.35)]
        loop_crossfade_seconds: f32,
        #[arg(long, default_value_t = true)]
        optimize: bool,
        #[arg(long, default_value_t = 2560)]
        proxy_width: u32,
        #[arg(long, default_value_t = 30)]
        proxy_fps: u32,
        #[arg(long, default_value_t = 24)]
        proxy_crf: u8,
        #[arg(long, default_value_os_t = default_config_path())]
        config: PathBuf,
    },
    SetApply {
        #[arg(long)]
        monitor: String,
        #[arg(long)]
        wallpaper: String,
        #[arg(long, default_value_os_t = default_downloads_root())]
        downloads_root: PathBuf,
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        keep_services: bool,
        #[arg(long)]
        mute_audio: bool,
        #[arg(long, value_enum, default_value_t = PlaybackProfile::Balanced)]
        profile: PlaybackProfile,
        #[arg(long)]
        display_fps: Option<u32>,
        #[arg(long, default_value_t = true)]
        allow_scene_preview_fallback: bool,
        #[arg(long, default_value_os_t = default_config_path())]
        config: PathBuf,
    },
    Remove {
        #[arg(long)]
        monitor: String,
        #[arg(long, default_value_os_t = default_config_path())]
        config: PathBuf,
    },
    List {
        #[arg(long, default_value_os_t = default_config_path())]
        config: PathBuf,
    },
}

#[derive(Subcommand)]
pub enum ServiceAutostartCommands {
    Install {
        #[arg(long)]
        overwrite: bool,
        #[arg(long)]
        dry_run: bool,
    },
    Enable {
        #[arg(long)]
        dry_run: bool,
    },
    Disable {
        #[arg(long)]
        dry_run: bool,
    },
    Remove {
        #[arg(long)]
        dry_run: bool,
    },
    Status,
}

fn default_downloads_root() -> PathBuf {
    if let Ok(home) = env::var("HOME") {
        return PathBuf::from(home).join(".local/share/kitsune/we/downloads");
    }
    PathBuf::from(".")
}

fn default_config_path() -> PathBuf {
    if let Ok(home) = env::var("HOME") {
        return PathBuf::from(home).join(".config/kitsune-livewallpaper/config.json");
    }
    PathBuf::from("config.json")
}
