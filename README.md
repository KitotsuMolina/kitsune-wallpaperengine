# kitsune-wallpaperengine

Motor propio en Rust para ejecutar wallpapers de Wallpaper Engine en Linux, integrado con Kitowall/Kitsune.

## Objetivo

Replicar progresivamente el flujo de Wallpaper Engine (Windows) con un motor propio:

1. Compatibilidad estable para uso diario (`safe mode`).
2. Compatibilidad avanzada de `scene` y efectos (`experimental mode`).
3. Paridad visual alta basada en ingenieria inversa del runtime real de Windows.

## Estado actual (real)

- `video wallpapers`: estables.
- `scene wallpapers`: funcionales con pipeline proxy (`mp4-proxy`) + parsing de `scene.pkg`.
- `text overlays`: funcionales en varios scenes (reloj/fecha/textos detectados).
- `audio-reactive / audio bars`: en pruebas.
  - Se detecta plan de overlay desde `scene.json`.
  - Se muestra warning de feature experimental.
  - Recomendacion estable: usar `Kitowall Spectrum`.
- `native-realtime transport`: experimental.

## Decisiones tecnicas tomadas

1. Mantener `mp4-proxy` como ruta estable por defecto.
2. Separar espectro de audio del video renderizado (no quemar audio bars en ffmpeg principal).
3. Exportar metadata de audio overlay (`audio-bars-overlay.json`) para usarla con Kitsune.
4. Auto-aplicar overlay en Kitsune cuando hay audio bars detectadas (group/profile generado automaticamente).
5. Priorizar compatibilidad por familias de efectos, no wallpaper por wallpaper.

## Comandos principales

### Ejecucion de wallpaper

```bash
cargo run -- scene-gpu-play <id|ruta> \
  --monitor <MONITOR> \
  --downloads-root <ruta> \
  --transport mp4-proxy \
  --profile performance|balanced|quality \
  --mute-audio \
  --proxy-width 1920 \
  --proxy-fps 30 \
  --proxy-crf 24
```

### Plan de audio overlay (scene)

```bash
cargo run -- scene-audio-plan <id|ruta> --downloads-root <ruta>
```

### Escaneo completo de libreria (compatibilidad)

```bash
cargo run -- library-scan \
  --downloads-root <ruta_downloads> \
  --top-effects 20
```

Solo resumen:

```bash
cargo run -- library-scan \
  --downloads-root <ruta_downloads> \
  --top-effects 20 \
  --summary-only
```

### Roadmap automatico de efectos por impacto

```bash
cargo run -- library-roadmap \
  --downloads-root <ruta_downloads> \
  --top-n 15
```

## Salidas importantes

- `scene-gpu-play` genera sesion en:
  - `~/.cache/kitsune-wallpaperengine/scene/<id>/render-session/`
- Cuando hay audio bars detectadas:
  - `gpu/audio-bars-overlay.json`
  - `gpu/kitsune-we-audio-overlay.group`
  - `gpu/kitsune-we-audio-overlay.profile`

## Warning actual de audio-reactive

Cuando un scene tiene audio-reactive/audio bars se muestra:

- Soporte en fase de pruebas.
- No recomendado para produccion.
- Para espectro estable: usar `Kitowall Spectrum`.

## Problemas conocidos

1. Algunos `scene` usan combinaciones de shaders/materiales no replicadas aun.
2. Ciertos `.tex` tienen resoluciones/padding no estandar.
3. `native-realtime` puede ser inestable en algunos casos.
4. `web/application wallpapers` aun no implementados.

## Flujo recomendado de trabajo (ahora)

1. Ejecutar `library-scan` para ver cobertura real.
2. Ejecutar `library-roadmap` para priorizar efectos.
3. Implementar por lotes de efectos frecuentes (top impacto).
4. Validar con subset fijo de wallpapers de regresion.
5. Repetir ciclo.

## Plan de ingenieria inversa en Windows (fase nueva)

Objetivo: entender flujo real de Wallpaper Engine para replicar comportamiento con menos heuristica.

### Fase 1: Observabilidad

1. Ejecutar wallpapers representativos en Windows (video, scene simple, scene complejo, audio-reactive).
2. Capturar:
   - composicion de capas
   - orden de efectos
   - blend/transparencias
   - parametros de shader visibles
   - comportamiento temporal (fps, suavizado, timing)
3. Documentar cada caso con evidencia visual y notas tecnicas.

### Fase 2: Modelo de runtime

1. Mapear pipeline real por etapas:
   - asset decode
   - pass graph
   - uniforms
   - compositing
   - postfx
2. Definir tabla de equivalencias con nuestro motor (soportado/parcial/no soportado).

### Fase 3: Implementacion incremental

1. Implementar efectos por prioridad (`library-roadmap`).
2. Crear pruebas de snapshot/metricas por wallpaper de referencia.
3. Medir mejora de `compatibility_percent` por iteracion.

## Criterio de exito

- Subir promedio de compatibilidad de la libreria de forma medible.
- Reducir wallpapers en estado `limited`.
- Mantener ruta estable (`mp4-proxy + spectrum separado`) sin romper UX diaria.

## Nota operativa

Si se quiere desactivar auto-aplicacion de overlay a Kitsune en `scene-gpu-play`:

```bash
--apply-kitsune-overlay false
```

(Disponible para pruebas A/B cuando haga falta aislar problemas de render.)
