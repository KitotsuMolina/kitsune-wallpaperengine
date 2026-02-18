Sí, entiendo la situación y ya revisé tanto tu proyecto (kitsune-wallpaperengine) como la instalación local de
Wallpaper Engine. Hay una razón clara por la que hoy terminas “wallpaper por wallpaper”: tu motor no está leyendo el
mismo flujo de datos que el runtime real.

Aquí va un diagnóstico concreto, con evidencia local y la ruta recomendada.

Evidencia del flujo real en Wallpaper Engine (instalación local)
Los datos que “mandan” el render no están en objects[].effects como asume tu motor, sino en la cadena scene.json ->
model/particle -> material -> passes -> shader.

- scene.json define cámara, proyección y objetos, pero normalmente no contiene efectos.
    - Ejemplo: projects/defaultprojects/beach/scene.json
- Los objetos apuntan a modelos/particles.
    - projects/defaultprojects/beach/scene.json → "image": "models/beach.json"
    - projects/defaultprojects/shimmering_particles/scene.json → "particle": "particles/dustmotes.json"
- Los modelos apuntan a materiales.
    - projects/defaultprojects/beach/models/beach.json → "material": "materials/beach.json"
- Los materiales definen passes con shader, textures, combos, constantshadervalues, usershadervalues.
    - projects/defaultprojects/beach/materials/beach.json
    - projects/defaultprojects/shimmering_particles/materials/particle/dustmote.json
- Los shaders están en la escena o en assets globales. Incluyen metadatos JSON en comentarios:
    - projects/defaultprojects/beach/shaders/flowimage.frag y .vert
    - assets/effects/scroll/shaders/effects/scroll.vert (uniforms con material / label)
- Hay efectos “prefab” en assets/effects/*/effect.json con materiales y shaders propios.
    - assets/effects/scroll/effect.json
- El scripting y el binding de user props está en JS, no en C++:
    - assets/scripts/jsclasses/baseclasses.js define createScriptProperties, convertUserProperties, etc.
    - scene.json puede traer bloques script (ej.: shimmering_particles tiene JS en general.bloomstrength)

Qué está mal en tu motor (y por eso vas wallpaper por wallpaper)

1. scene_plan toma “el asset más grande” como visual primario. Eso ignora el graph real.
    - src/scene_plan.rs
2. scene_gpu_graph solo mira objects[].effects. En escenas reales, los shaders viven en materiales, no en effects.
    - src/scene_gpu_graph.rs
3. El pipeline está basado en un “proxy” (mp4/ffmpeg) porque no hay renderer de passes/materials.
    - src/scene_renderer.rs, src/scene_effect_proxy.rs
4. No se evalúan scripts JS ni se aplican user/script a valores de shader.
    - Ausente en todo el runtime, pero los datos están en scene.json y assets/scripts/jsclasses/baseclasses.js.

Ruta correcta (para dejar de ir wallpaper por wallpaper)

1. Resolver de assets real
    - Orden de búsqueda: scene.pkg → carpeta del wallpaper → assets/ global.
    - Necesario para shaders, materiales, texturas y efectos.
2. Parser de escena real
    - scene.json → objetos → image/particle → model/particle json → material json → passes.
3. Compiler de passes
    - Cada pass: shader + combos + constantes + user values + textures.
    - Generar defines por combos, bind de uniformes por constantshadervalues y usershadervalues.
4. Runtime de scripting (JS)
    - Ejecutar script y user para actualizar uniformes y propiedades.
    - Mínimo: correr applyUserProperties y exponer thisObject con API básica.
5. Renderer por familias de shaders (no por wallpaper)
    - Empezar por genericimage*, generic*, particle (en realidad genericparticle*).
    - Luego agregar efectos de assets/effects.

Si quieres, puedo empezar a implementar este cambio en el repo. La primera pieza clave es reescribir scene_gpu_graph
para que construya el graph desde materiales y modelos, no desde objects[].effects.