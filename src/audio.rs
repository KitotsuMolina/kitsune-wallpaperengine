use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Serialize)]
pub struct AudioProbeResult {
    pub source: String,
    pub sample_rate: u32,
    pub channels: u8,
    pub duration_ms: u64,
    pub samples: usize,
    pub peak: f32,
    pub rms: f32,
}

#[derive(Debug, Serialize)]
pub struct AudioLevelFrame {
    pub frame_index: u64,
    pub peak: f32,
    pub rms: f32,
}

#[derive(Debug, Serialize)]
pub struct AudioStreamResult {
    pub source: String,
    pub sample_rate: u32,
    pub channels: u8,
    pub frame_ms: u64,
    pub duration_ms: u64,
    pub samples: usize,
    pub frames: Vec<AudioLevelFrame>,
}

fn command_output(cmd: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .with_context(|| format!("Failed to execute {}", cmd))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("{} failed: {}", cmd, err.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn infer_default_monitor_source() -> Result<String> {
    let default_sink = command_output("pactl", &["get-default-sink"])?
        .trim()
        .to_string();
    if default_sink.is_empty() {
        bail!("pactl returned empty default sink");
    }

    let target = format!("{}.monitor", default_sink);
    let sources = command_output("pactl", &["list", "short", "sources"])?;

    let exists = sources.lines().any(|line| {
        let cols: Vec<&str> = line.split_whitespace().collect();
        cols.get(1).map(|v| *v == target).unwrap_or(false)
    });

    if !exists {
        bail!(
            "Could not find monitor source '{}' in pactl list short sources",
            target
        );
    }

    Ok(target)
}

pub fn probe_audio(source: Option<String>, seconds: u64) -> Result<AudioProbeResult> {
    let stream = stream_audio_levels(source, seconds, 1000)?;
    let mut peak = 0.0f32;
    let mut sq_sum = 0.0f64;
    let mut count = 0usize;

    for f in &stream.frames {
        if f.peak > peak {
            peak = f.peak;
        }
        sq_sum += (f.rms as f64) * (f.rms as f64);
        count += 1;
    }

    let rms = if count == 0 {
        0.0
    } else {
        (sq_sum / count as f64).sqrt() as f32
    };

    Ok(AudioProbeResult {
        source: stream.source,
        sample_rate: stream.sample_rate,
        channels: stream.channels,
        duration_ms: stream.duration_ms,
        samples: stream.samples,
        peak,
        rms,
    })
}

pub fn stream_audio_levels(
    source: Option<String>,
    seconds: u64,
    frame_ms: u64,
) -> Result<AudioStreamResult> {
    let source = match source {
        Some(s) => s,
        None => infer_default_monitor_source()?,
    };

    let sample_rate = 48_000u32;
    let channels = 2u8;
    let bytes_per_sample = 2usize; // s16le

    let mut child = Command::new("parec")
        .arg("--raw")
        .arg("--format=s16le")
        .arg(format!("--rate={}", sample_rate))
        .arg(format!("--channels={}", channels))
        .arg("-d")
        .arg(&source)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Failed to start parec. Install pulseaudio-utils/pipewire-pulse compatibility")?;

    let mut stdout = child.stdout.take().context("parec stdout not available")?;

    let target_bytes = (sample_rate as usize)
        .saturating_mul(channels as usize)
        .saturating_mul(bytes_per_sample)
        .saturating_mul(seconds as usize);

    let samples_per_frame = ((sample_rate as u64)
        .saturating_mul(channels as u64)
        .saturating_mul(frame_ms)
        / 1000)
        .max(1) as usize;

    let start = Instant::now();
    let mut read_total = 0usize;
    let mut all_samples = 0usize;
    let mut frames = Vec::<AudioLevelFrame>::new();

    let mut frame_peak = 0.0f32;
    let mut frame_sq_sum = 0.0f64;
    let mut frame_samples = 0usize;
    let mut frame_idx = 0u64;

    let mut buf = vec![0u8; 64 * 1024];
    while read_total < target_bytes && start.elapsed() < Duration::from_secs(seconds + 2) {
        let n = stdout
            .read(&mut buf)
            .context("Failed reading parec stream")?;
        if n == 0 {
            break;
        }

        let usable = n - (n % 2);
        for i in (0..usable).step_by(2) {
            let sample = i16::from_le_bytes([buf[i], buf[i + 1]]) as f32 / 32768.0;
            let abs = sample.abs();
            if abs > frame_peak {
                frame_peak = abs;
            }
            frame_sq_sum += (sample as f64) * (sample as f64);
            frame_samples += 1;
            all_samples += 1;

            if frame_samples >= samples_per_frame {
                let rms = (frame_sq_sum / frame_samples as f64).sqrt() as f32;
                frames.push(AudioLevelFrame {
                    frame_index: frame_idx,
                    peak: frame_peak,
                    rms,
                });
                frame_idx += 1;
                frame_peak = 0.0;
                frame_sq_sum = 0.0;
                frame_samples = 0;
            }
        }

        read_total += n;
    }

    if frame_samples > 0 {
        let rms = (frame_sq_sum / frame_samples as f64).sqrt() as f32;
        frames.push(AudioLevelFrame {
            frame_index: frame_idx,
            peak: frame_peak,
            rms,
        });
    }

    let _ = child.kill();
    let _ = child.wait();

    if all_samples == 0 {
        bail!("No audio samples captured from source '{}'.", source);
    }

    Ok(AudioStreamResult {
        source,
        sample_rate,
        channels,
        frame_ms,
        duration_ms: start.elapsed().as_millis() as u64,
        samples: all_samples,
        frames,
    })
}
