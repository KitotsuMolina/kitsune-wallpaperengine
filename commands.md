# kitsune-livewallpaper - Commands

Este documento separa:
- `video-play` como flujo estable/recomendado.
- El resto de comandos como demo o en desarrollo.

## 1) Comando estable: `video-play`

Uso base:

```bash
./target/debug/kitsune-livewallpaper video-play [OPTIONS] --monitor <MONITOR> <VIDEO>
```

### Opciones disponibles en `video-play`

- `--monitor <MONITOR>` (requerido)
- `--downloads-root <DOWNLOADS_ROOT>`
- `--keep-services`
- `--service <SERVICES>` (repetible)
- `--mute-audio`
- `--profile <performance|balanced|quality>`
- `--display-fps <DISPLAY_FPS>`
- `--seamless-loop`
- `--loop-crossfade`
- `--loop-crossfade-seconds <SECONDS>` (default `0.35`)
- `--optimize`
- `--proxy-width <WIDTH>` (default `3840`)
- `--proxy-fps <FPS>` (default `60`)
- `--proxy-crf <CRF>` (default `16`)
- `--dry-run`

### Ejemplos `video-play`

Ejemplo recomendado (buen balance rendimiento/calidad):

```bash
./target/debug/kitsune-livewallpaper video-play /ruta/video.mp4 \
  --monitor DP-1 \
  --profile performance \
  --seamless-loop \
  --optimize \
  --proxy-width 2560 \
  --proxy-fps 30 \
  --proxy-crf 24
```

Con crossfade:

```bash
./target/debug/kitsune-livewallpaper video-play /ruta/video.mp4 \
  --monitor DP-1 \
  --profile performance \
  --seamless-loop \
  --loop-crossfade \
  --loop-crossfade-seconds 0.35 \
  --optimize \
  --proxy-width 2560 \
  --proxy-fps 30 \
  --proxy-crf 24
```

Sin audio:

```bash
./target/debug/kitsune-livewallpaper video-play /ruta/video.mp4 \
  --monitor DP-1 \
  --profile performance \
  --mute-audio \
  --seamless-loop \
  --optimize \
  --proxy-width 2560 \
  --proxy-fps 30 \
  --proxy-crf 24
```

Simulación (sin aplicar cambios):

```bash
./target/debug/kitsune-livewallpaper video-play /ruta/video.mp4 \
  --monitor DP-1 \
  --profile performance \
  --seamless-loop \
  --optimize \
  --proxy-width 2560 \
  --proxy-fps 30 \
  --proxy-crf 24 \
  --dry-run
```

### Aplicar `video-play` a todos los monitores (Hyprland)

```bash
VIDEO="/home/kitotsu/Videos/LiveWallpapers/motionbgs/2b-midnight-bloom/2b-midnight-bloom__4k.mp4"
hyprctl -j monitors | jq -r '.[].name' | while IFS= read -r m; do
  ./target/debug/kitsune-livewallpaper video-play "$VIDEO" \
    --monitor "$m" \
    --profile performance \
    --seamless-loop \
    --optimize \
    --proxy-width 2560 \
    --proxy-fps 30 \
    --proxy-crf 24
done
```

## 2) Comandos disponibles (demo/en desarrollo)

Los siguientes comandos existen en el binario, pero se consideran de demo/proceso de desarrollo:

- `inspect`
- `scene-dump`
- `scene-plan`
- `scene-audio-plan`
- `library-scan`
- `library-roadmap`
- `scene-runtime`
- `scene-render`
- `scene-gpu-graph`
- `scene-native-plan`
- `scene-gpu-play`
- `text-refresh`
- `scene-play`
- `audio-probe`
- `audio-stream`
- `stop-services`
- `apply`

Ayuda general:

```bash
./target/debug/kitsune-livewallpaper --help
```

Ayuda de un subcomando específico (ejemplo):

```bash
./target/debug/kitsune-livewallpaper scene-gpu-play --help
```
