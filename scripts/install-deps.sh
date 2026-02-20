#!/usr/bin/env bash
set -euo pipefail

log() {
  printf "[install-deps] %s\n" "$*"
}

err() {
  printf "[install-deps][error] %s\n" "$*" >&2
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1
}

if [[ "${EUID:-$(id -u)}" -eq 0 ]]; then
  err "Do not run as root. Run as normal user; the script will use sudo when needed."
  exit 1
fi

if ! need_cmd sudo; then
  err "sudo is required."
  exit 1
fi

if ! need_cmd rustup; then
  log "rustup not found; it will be installed by distro package manager when available."
fi

if [[ -f /etc/os-release ]]; then
  # shellcheck disable=SC1091
  source /etc/os-release
else
  err "Cannot detect distro (/etc/os-release missing)."
  exit 1
fi

install_arch() {
  local pkgs=(
    rustup
    ffmpeg
    mpvpaper
    libpulse
    pipewire-pulse
    procps-ng
    pciutils
    systemd
  )
  sudo pacman -Syu --needed "${pkgs[@]}"
}

install_debian() {
  local pkgs=(
    ffmpeg
    mpv
    pulseaudio-utils
    pipewire-pulse
    procps
    pciutils
    systemd
    curl
  )
  sudo apt-get update
  sudo apt-get install -y "${pkgs[@]}"
  if ! need_cmd rustup; then
    curl https://sh.rustup.rs -sSf | sh -s -- -y
    # shellcheck disable=SC1090
    source "${HOME}/.cargo/env"
  fi
}

install_fedora() {
  local pkgs=(
    rustup
    ffmpeg
    mpv
    pulseaudio-utils
    pipewire-pulseaudio
    procps-ng
    pciutils
    systemd
  )
  sudo dnf install -y "${pkgs[@]}"
}

case "${ID:-}" in
  arch)
    log "Detected Arch Linux"
    install_arch
    ;;
  debian|ubuntu|linuxmint|pop)
    log "Detected Debian/Ubuntu family (${ID})"
    install_debian
    ;;
  fedora)
    log "Detected Fedora"
    install_fedora
    ;;
  *)
    err "Unsupported distro ID='${ID:-unknown}'. Please install manually: rustup, ffmpeg, mpvpaper/mpv, libpulse (Arch) or pulseaudio-utils (Debian/Fedora), pipewire-pulse, procps, pciutils, systemd."
    exit 2
    ;;
esac

if need_cmd rustup; then
  rustup default stable
fi

missing=()
for bin in cargo rustc ffmpeg ffprobe pgrep lspci lscpu pactl parec; do
  if ! need_cmd "$bin"; then
    missing+=("$bin")
  fi
done

if ((${#missing[@]} > 0)); then
  err "Missing binaries after install: ${missing[*]}"
  err "For Arch, ensure: pacman -S rustup ffmpeg mpvpaper libpulse pipewire-pulse procps-ng pciutils"
  exit 3
fi

if ! need_cmd mpvpaper; then
  log "mpvpaper not found. Scene/video processing works, but live wallpaper playback via mpvpaper will fail."
else
  log "mpvpaper found."
fi

log "Dependencies installed and verified."
log "Next: cargo build"
