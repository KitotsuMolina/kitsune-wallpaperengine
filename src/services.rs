use anyhow::{Context, Result};
use std::process::Command;

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
