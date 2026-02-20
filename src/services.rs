use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

pub const DEFAULT_SERVICES: [&str; 6] = [
    "swww-daemon@kitowall.service",
    "swww-daemon.service",
    "kitowall-next.timer",
    "kitowall-next.service",
    "kitowall-watch.service",
    "hyprwall-watch.service",
];

pub fn default_services() -> Vec<String> {
    DEFAULT_SERVICES.iter().map(|s| (*s).to_string()).collect()
}

pub fn stop_services(services: &[String], dry_run: bool) -> Result<()> {
    for svc in services {
        if dry_run {
            println!("[dry-run] systemctl --user stop {}", svc);
            continue;
        }

        let output = Command::new("systemctl")
            .arg("--user")
            .arg("stop")
            .arg(svc)
            .output()
            .with_context(|| format!("Failed running systemctl for {}", svc))?;

        if output.status.success() {
            println!("[ok] stopped {}", svc);
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            eprintln!("[warn] could not stop {}: {}", svc, err.trim());
        }
    }
    Ok(())
}

fn unit_active(service: &str) -> Result<bool> {
    let out = Command::new("systemctl")
        .arg("--user")
        .arg("is-active")
        .arg(service)
        .output()
        .with_context(|| format!("Failed checking state for {}", service))?;
    Ok(out.status.success())
}

pub fn start_services(services: &[String], dry_run: bool) -> Result<()> {
    let mut failures = Vec::new();
    for svc in services {
        if dry_run {
            println!("[dry-run] systemctl --user start {}", svc);
            continue;
        }

        let output = Command::new("systemctl")
            .arg("--user")
            .arg("start")
            .arg(svc)
            .output()
            .with_context(|| format!("Failed running systemctl for {}", svc))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            failures.push(format!("{}: {}", svc, err.trim()));
            continue;
        }

        let deadline = Instant::now() + Duration::from_secs(8);
        let mut is_up = false;
        while Instant::now() < deadline {
            if unit_active(svc)? {
                is_up = true;
                break;
            }
            thread::sleep(Duration::from_millis(250));
        }
        if is_up {
            println!("[ok] started {}", svc);
        } else {
            failures.push(format!("{}: started but did not become active", svc));
        }
    }

    if failures.is_empty() {
        return Ok(());
    }
    anyhow::bail!("Some services failed to start:\n- {}", failures.join("\n- "));
}

fn autostart_unit_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".config/systemd/user/kitsune-livewallpaper.service");
    }
    PathBuf::from("kitsune-livewallpaper.service")
}

fn systemctl_user(args: &[&str]) -> Result<()> {
    let output = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .with_context(|| format!("Failed running systemctl --user {}", args.join(" ")))?;
    if output.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&output.stderr);
    anyhow::bail!(
        "systemctl --user {} failed: {}",
        args.join(" "),
        err.trim()
    );
}

pub fn install_autostart_service(overwrite: bool, dry_run: bool) -> Result<()> {
    let unit_path = autostart_unit_path();
    if unit_path.is_file() && !overwrite {
        anyhow::bail!(
            "Autostart service already exists at {} (use --overwrite to replace)",
            unit_path.display()
        );
    }

    let unit = r#"[Unit]
Description=Kitsune LiveWallpaper Autostart
After=graphical-session.target
Wants=graphical-session.target

[Service]
Type=oneshot
ExecStart=/usr/bin/env bash -lc "kitsune-livewallpaper start-config"
RemainAfterExit=yes

[Install]
WantedBy=default.target
"#;

    if dry_run {
        println!("[dry-run] write {}", unit_path.display());
        println!("[dry-run] systemctl --user daemon-reload");
        return Ok(());
    }

    if let Some(parent) = unit_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed creating {}", parent.display()))?;
    }
    fs::write(&unit_path, unit).with_context(|| format!("Failed writing {}", unit_path.display()))?;
    systemctl_user(&["daemon-reload"])?;
    println!("[ok] installed {}", unit_path.display());
    Ok(())
}

pub fn enable_autostart_service(dry_run: bool) -> Result<()> {
    if dry_run {
        println!("[dry-run] systemctl --user enable --now kitsune-livewallpaper.service");
        return Ok(());
    }
    systemctl_user(&["enable", "--now", "kitsune-livewallpaper.service"])?;
    println!("[ok] enabled kitsune-livewallpaper.service");
    Ok(())
}

pub fn disable_autostart_service(dry_run: bool) -> Result<()> {
    if dry_run {
        println!("[dry-run] systemctl --user disable --now kitsune-livewallpaper.service");
        return Ok(());
    }
    systemctl_user(&["disable", "--now", "kitsune-livewallpaper.service"])?;
    println!("[ok] disabled kitsune-livewallpaper.service");
    Ok(())
}

pub fn remove_autostart_service(dry_run: bool) -> Result<()> {
    let unit_path = autostart_unit_path();
    if dry_run {
        println!("[dry-run] systemctl --user disable --now kitsune-livewallpaper.service");
        println!("[dry-run] rm {}", unit_path.display());
        println!("[dry-run] systemctl --user daemon-reload");
        return Ok(());
    }
    let _ = systemctl_user(&["disable", "--now", "kitsune-livewallpaper.service"]);
    if unit_path.is_file() {
        fs::remove_file(&unit_path)
            .with_context(|| format!("Failed removing {}", unit_path.display()))?;
    }
    systemctl_user(&["daemon-reload"])?;
    println!("[ok] removed autostart service");
    Ok(())
}

pub fn autostart_service_status() -> Result<()> {
    let unit_path = autostart_unit_path();
    println!(
        "[ok] unit_file={} exists={}",
        unit_path.display(),
        unit_path.is_file()
    );
    for args in [
        ["is-enabled", "kitsune-livewallpaper.service"],
        ["is-active", "kitsune-livewallpaper.service"],
    ] {
        let out = Command::new("systemctl")
            .arg("--user")
            .args(args)
            .output()
            .with_context(|| "Failed querying systemctl status")?;
        let val = String::from_utf8_lossy(&out.stdout).trim().to_string();
        println!(
            "[ok] {} => {}",
            args.join(" "),
            if val.is_empty() { "unknown" } else { &val }
        );
    }
    Ok(())
}
