# oh_my_engine — Project Memory (agent briefing)

> **READ FIRST** al retomar el engine desde cualquier máquina (Linux/Windows).
> Fuente de verdad = este archivo + issues de GitHub. Consolidado de la memoria del asistente el 2026-07-13.
> Los file:line pueden quedar stale — verificá contra el código antes de afirmar.

---

## Qué es

Game engine experimental en **Rust Edition 2024**, arquitectura híbrida CPU-GPU:
gameplay/lógica en CPU, física/render en GPU. Diseñado **planet-scale + GPU-driven**
desde la foundation — todo se evalúa contra (a) escala planetaria, (b) GPU-driven hot
loop sin CPU readback en frame.

- **Licencia:** All Rights Reserved (repo privado/personal `lobinuxsoft/oh_my_engine`).
- **Branches:** `main` (release-please) ← `development` (integración) ← `feat/*`.
- **Disambiguación:** `oh_my_engine` = motor (underscore). `oh-my-agent` = CLI coding agent
  con LLMs embebidos (guiones). Ambos en `/var/mnt/DATA/Repos/`. Si el user dice solo
  "oh my", preguntar cuál.

## Stack

wgpu **29**, winit, kira (audio), gilrs (gamepad), rhai (scripting), egui/eframe **0.34**,
rapier3d 0.22 (física), meshopt 0.6, metis (vendored, meshlet grouping), glam, bytemuck,
slotmap, gltf, image. PipelineCache vía wgpu unsafe + fallback. **Stay-on-wgpu 24 meses
mínimo** (audit #239).

## Workspace (crates)

`ome_core, ome_ecs, ome_window, ome_input, ome_lighting, ome_render, ome_physics,
ome_gravity, ome_world, ome_audio, ome_scripting, ome_editor_core, ome_editor,
ome_gizmos, ome_gizmos_handles, ome_editor_api`. Facade top-level `oh_my_engine` con
`DefaultPlugins` PluginGroup (estilo Bevy).

`ome_sdf` **ELIMINADO (2026-07)**. Contenía dos cosas sin relación: la librería de
primitivas CSG en WGSL, que alimentaba el raymarcher ya borrado en el pivot, y el
almacenamiento de vóxeles disperso con LOD (#136). Lo primero murió con su renderer;
lo segundo era el sustrato de Phase 2.5 y se mudó a **`ome_world::voxel`**, que es
donde vive el resto del streaming.

`ome_bvh` **ELIMINADO (2026-07)** junto con `ome_world::content` y el
`ProceduralCitySource`. Era la estructura de aceleración del raymarcher — sus flags se
llamaban `IS_RAYMARCH` / `ROLE_RAYMARCH_*` — y su único consumidor era contenido de chunk
que nadie rendeaba desde el pivote: el path meshlet tiene su propio culling con Hi-Z. El
`Aabb` se mudó a `ome_core`, que es donde correspondía. ~14k líneas.

**El mundo contiene escenas, NO al revés (#566, corregido 2026-07-21).** Verificado contra
los engines ECS: Bevy escribe `Scene`/`DynamicScene` DENTRO de un `World`
(`write_to_world_with`); el `World` de Unity DOTS contiene la meta-entidad de escena y sus
section entities; el `UWorld` de Unreal contiene levels → UE5 World Partition = grilla
espacial. El contenedor runtime es el mundo; las escenas son contenido que se carga en él.

**Escena y celda son ORTOGONALES, no anidadas.** Una escena abarca muchas celdas y varias
escenas pueden solaparse en una celda. Celda = "¿qué hay cerca?" (espacial, derivado de la
posición). Escena = "¿qué autoró junto un diseñador?" (lógico, decisión humana). Una entidad
pertenece a UNA escena y a UNA celda, independientemente. **Storage indexado por
`(escena, celda)`** = los archivos per-scene-section de Unity. Anidarlos costaría: escenas
solapadas en el mismo volumen, dos personas editando sin conflicto en git, y descargar una
escena vs descargar una celda como operaciones distintas.

Reglas: ownership **espacial y derivado** del transform, nunca campo guardado (se
desincroniza). **Residencia ≠ existencia**. **Sección siempre residente** estilo section-0
de Unity (carga primera, descarga última) = ahí vive la entidad de focus, que no puede ser
dueña de una celda que se descarga sola. Depende de #139 (bridge de física).

Criterio aplicado (del usuario, 2026-07-21): **se queda lo que sirve al sistema galáctico
y a planetas terraformables; lo demás muere y se rehace bien contra el motor de física
actual.** Nada de "tal vez lo usemos". Si hace falta algo de ahí: cherry-pick del historial.

**No confundir los dos "SDF"**: el descartado es SDF-como-técnica-de-render/authoring
(brushes CSG, raymarching). El vigente es SDF-como-formato-de-dato — cada vóxel guarda
una distancia con signo, y de ahí extrae malla el dual contouring (#393/#397/#398). Para
física tampoco hace falta lo primero: el collider de vóxel de rapier 0.34 consume la
grilla directamente.

---

## ⭐ Estado actual (development HEAD `74e824f`, 2026-07-22)

- `development` limpio, **cero PRs abiertos**. Stash pendiente: `wip: physics components
  (#139)` sobre la branch `feat/physics-plugin`.
- **Épico C (editor remoto) CERRADO.** Fases 1-5 mergeadas (#548→#557). Play in-editor
  estilo Unity funcionando, smoke visual del user OK. #546 cerrada como completada.
- **Hilo siguiente = física.** `ome_physics` está en rapier 0.34 + glam 0.33 workspace-wide
  (#564), pero le falta el puente al ECS: nada crea cuerpos desde entidades, nada llama
  `step`, nada escribe de vuelta a `Transform`. Un cubo todavía no puede caer. Eso es #139
  y es el único bloqueante del épico de física.
- **Amputación SDF/BVH hecha** (#565, ~14k líneas). Ver sección de decisiones sticky.
- Editor corre limpio en RX 9070 XT (Vulkan/RADV): cero validation warnings, smoke OK en
  producción + todos los debug modes.

### ⭐ Épico de física — backlog auditado contra el source de rapier 0.34 (2026-07-22)

Cobertura verificada módulo por módulo. **#139 primero — todo lo demás depende de él.**

| Capacidad de rapier 0.34 | Issue |
|---|---|
| `dynamics` — rigid bodies, step, integration params | **#139** ⭐ BLOQUEANTE |
| `geometry` — colliders, shapes (incl. **voxels**), material, grupos | #137 |
| `dynamics/joint` — 7 impulse + multibody + motores + límites + ruptura | #560 |
| `pipeline` — event handler, hooks, sensores | #561 |
| `pipeline/query_pipeline` — shape casts, proyección, intersecciones | #562 |
| `pipeline/debug_render_pipeline` | #563 |
| `control/character_controller` | #94 (body pide reescritura: especifica SDF) |
| `control/ray_cast_vehicle_controller` | #105 |
| `control/pid_controller` | #567 |
| `island_manager` — regiones activas por distancia | #311 |
| `counters` — timings por etapa | #569 |
| `enhanced-determinism` vs SIMD — **decisión pendiente** | #568 |
| Softbody — **rapier NO lo tiene** | #107 (corregida) |

**#568 es lo más importante que nadie decidió**: `enhanced-determinism` y SIMD son
mutuamente excluyentes (`compile_error!` en rapier). Hoy hay `simd-stable`, o sea el
determinismo cross-machine está APAGADO sin que nadie lo eligiera. Sin él quedan fuera
netcode lockstep, replays como log de inputs y revalidación server-side. Cambiarlo es un
flag; lo caro es el netcode que se construya asumiendo lo contrario.

**Gotcha de diseño para #139 (corregido antes de implementar):** el mapeo entidad↔cuerpo
NO va como `HashMap<Entity, BodyHandle>` — viola las reglas DOD del proyecto. Va como
componente POD `PhysicsBody(u32)` en el archetype + `Vec<Entity>` indexado por body index
para el inverso. El sync pasa a ser un recorrido lineal sobre `(PhysicsBody, Transform)`.
Con Rapier la capa de sync es **obligatoria** (Rapier es dueño de sus `RigidBodySet` /
`ColliderSet`; no puede leer arrays ajenos) — el modelo ECS-nativo tipo Avian no está
disponible porque Avian está soldado a `bevy_ecs`.

### ⭐⭐ Épico C — Editor remoto (BRP-style) — EN CURSO

**Problema raíz:** el editor standalone (`ome_editor`, launcher hub) NO puede cargar escenas
con componentes del proyecto (`unknown component type: test3::...::MoveComponent`). No es bug:
es un editor genérico compilado antes que el proyecto exista; Rust resuelve tipos en
compile-time, no hay ABI dinámica estable. **Solución (elegida por el user, "no alambre"):
protocolo remoto tipo Bevy Remote Protocol.** El binario del PROYECTO es dueño del ECS y
responde por HTTP; el editor pasa a cliente delgado.

**Arquitectura clave (locked):** UNA fuente de datos (ECS local, real en modo local o
ESPEJO en modo remoto vía `RemoteMirror`) + UN seam de bifurcación (`apply_actions` →
`remote_edit::dispatch` cuando `RemoteState::is_connected()`). Paneles/DTOs/viewport
IDÉNTICOS en ambos modos. Modelo de viewport = **C** (el hub rendea el espejo con su
viewport, que ya rendea meshes bien). Estrategia de entrada = **A→B**: hoy "Open Remote"
separado; cuando el remoto madure, "Open Project" pasa a remoto (borrar un botón, no
reescribir).

**Fases (todas MERGED salvo la última):**
- ✅ **1 — componentes dinámicos** (#548). `ome_ecs::dynamic_components`: componente sin tipo
  local se PARKEA (no aborta la carga, no vacía el mundo). Round-trip intacto. Mató pérdida
  de datos real. `SceneError::UnknownComponent` eliminado.
- ✅ **2 — identidad portable** (#549). `ome_ecs::component::{ComponentId, ComponentNames}`:
  interner nombre↔u32 process-local. `EditorAction`/DTOs llevan `ComponentId`; `TypeId` se
  queda en reflexión local. Seam `dispatch::action_to_command` resuelve ComponentId→TypeId.
- ✅ **3 — crate `ome_remote`** (#550). Server `tiny_http` SÍNCRONO (thread dedicado + puente
  mpsc al main, NO async, NO tokio) + `RemoteClient` blocking (std::net). `protocol` (serde,
  componentes por nombre, Entity=(index,generation), reusa ReflectValue). Métodos:
  ping/list_entities/get_schema/set_field/add/remove/spawn/despawn/save/load. DEFAULT_PORT 15703.
- ✅ **4a — modo `--remote`** (#551). Facade: dep opcional tras feature `remote` + prelude.
  Scaffold main.rs 3er modo `cargo run -- --remote` = DefaultPlugins + RemotePlugin +
  run_systems:false. Verificado vs TEST3 real por HTTP.
- ✅ **4b.1 — `RemoteSession`** (#552). `ome_editor_core::remote_session`: lanza el proyecto
  (patrón PlayState + env OME_ENGINE_ROOT/OME_PROJECT_ROOT), handshake poll_ready, cachea
  snapshot+schema.
- ✅ **4b.2 — `RemoteMirror`** (#553). Reconstruye el snapshot en el ECS local del editor,
  keyed por `EntityId` (NO por nombre; 5 "Mesh" duplicados). Engine components → reflected
  reales (rendean); proyecto → parkeados (DynamicComponents). Marker `MirrorEntity`.
- ✅ **4b.3a — dual-sink dispatch** (#554). `RemoteState` (session+mirror). `apply_actions`
  chequea is_connected → rutea EditorAction al RemoteClient (`remote_edit::dispatch`; traduce
  Entity local→remoto vía `mirror.remote_of`, ComponentId→nombre). UN solo if. Test e2e:
  SetField en el editor llega al server.

**⏳ LO QUE FALTA:**
- **4b.3b — ACTIVACIÓN (próxima sesión, la que hace TODO visible):** insertar `RemoteState`
  en `EditorPlugin`; system que poll_ready + refresh (cada N frames) + `mirror.apply` sobre el
  ECS del editor; registrar `MirrorEntity` como ephemeral; entry point UI "Open Remote" en
  `launch_screen.rs` (junto a "Open Project"). El viewport rendea el mirror automáticamente
  (ya rendea el ECS local). **Necesita smoke visual del user.**
- **Mostrar DynamicComponents parkeados en el Inspector** (deuda fase 1): `queries.rs` hoy solo
  lee componentes registrados del archetype → los componentes del proyecto son invisibles en
  el Inspector remoto. Necesario para authoring completo.
- **Fase 5 — ciclo de vida:** Play/Stop remoto, reconexión tras recompilar, "Rebuild & Relaunch".
- **Convergencia A→B:** cuando el remoto madure, "Open Project" → remoto por defecto.

**Deuda no-épico acumulada (arreglable suelta):** `Duplicate` de entidad no copia componentes
parkeados; drag `.glb`/`.gltf` al slot de mesh (pedido); `Delete` de asset sin confirmación
(#439); thumbnails del asset browser.

### Render path vigente: mesh GPU-driven Nanite-style (NO SDF)

El epic SDF #370 (GPU-driven SDF rendering Lumen-class) quedó **pausado**. El pivot a
**mesh GPU-driven Nanite-style** es el hot path. Phase 1 (#117) cerró completa: meshlet
pipeline end-to-end con cull + visibility buffer + deferred + Hi-Z 2-pass, atomic R64
visibility, cluster LOD DAG (METIS).

### Último landing — #440 two-pass material shading (PR #545, MERGED)

Two-pass material shading estilo Bevy/Nanite en el path R64, **todo fragment** (se eliminó
el compute deferred):

- **Schema `Material`**: `albedo`/`normal`/`metal_roughness: Option<Guid>` + builders + RON round-trip.
- **`MaterialParams` 48 B**: `texture_indices: [u32;4]` (`NO_TEXTURE = u32::MAX`). WGSL structs stride-sync.
- **`MaterialTexturePool`** (`material/texture_pool.rs`): registry GUID→`GpuTexture` + bind group
  per-material con fallbacks 1×1 branch-free (white albedo / flat normal `[128,128,255]` / white metal_rough → sample no-op).
- **`MaterialPipeline`**: resuelve + sube imágenes por GUID, trackea triple `[albedo,normal,mr]` por slot.
  `shading_slots()` = `0..next_slot` (slot 0 fallback TAMBIÉN shadea).
- **Shaders** (`shaders/`): `resolve_material_depth.wgsl` (pass 1 → Depth16Unorm),
  `visibility_buffer_resolve.wgsl` (port The-Forge/Bevy: baricéntricas perspective-correct +
  ddx/ddy analíticas), `material_pbr_default.wgsl` (normal-debug × albedo + normal mapping
  tangent-space, `textureSampleGrad`). WGSL sin `#include` → se concatenan en Rust (`compose_material_shader`).
- **Render path** (`vbuf64_stage/`): `MaterialTwoPass` (resolve + N passes per-material,
  depth-test `Equal`, vs emite `slot/65535` como depth, `@invariant`). `DebugResolve`
  (`debug_resolve.rs`): fragment fullscreen para debug modes colorize (1,2,3,4,7). Modos
  normal-look (0,5,6,8,9,10) → two-pass.

### ⭐ Próximo paso — VERIFICAR #440 (falta tooling)

**#440 sigue OPEN a propósito.** El pipeline de texturas está completo y funciona, PERO
**no se puede VER el texturizado** porque el editor no tiene con qué asignar/inspeccionar
materiales. Antes de #441 (PBR real), lo lógico es el tooling mínimo:

1. **Material inspector**: campos en el Inspector para asignar albedo/normal/metal_roughness
   por GUID a un `Material`.
2. **Asset browser**: listado básico de assets del proyecto (`AssetDatabase` ya los conoce;
   el inspector ya tiene un `asset_catalog`/typed asset picker reusable).
3. **Texturas demo**: checker albedo + flat normal + metal_rough en `assets/` + un
   `.ome_material.ron` que las referencie, para smoke visual end-to-end.

Sin (1)/(2), el paso de texturas demo no es verificable — por eso quedó afuera del PR #545.

---

## Decisiones arquitecturales sticky (NO reabrir sin OK explícito)

### Física = Rapier (revalidado 2026-07-21 con investigación, NO reabrir)

Jolt gana en performance CPU (~2× en escenas grandes) y en completitud (softbody,
vehículos, ragdolls). Pierde igual por tres razones: en Rust son bindings de terceros que
se autodescriben "early work in progress / watch for exposed nails", exige toolchain C++
(choca con #558, build shippeable), y **no tiene ni plan de GPU**. Rapier tiene wgrapier
(física en WGSL, broadphase BVH + solvers andando, demo de 93k cuerpos / 120k joints) y
prioridad Dimforge 2026 = GPU vía rust-gpu compartiendo tipos con rapier. Para un engine
GPU-driven sobre wgpu eso decide solo. Bonus: rapier 0.32+ trae **collider de vóxel
disperso**, único entre engines rigid-body generales — clave con el pipeline voxel/DC.

**El backend va SIEMPRE detrás del trait `PhysicsBackend`.** `rapier3d::` solo puede
importarse dentro de `ome_physics::rapier_backend`; en cualquier otro archivo es un bug de
arquitectura. Los componentes ECS describen intención (forma, masa, tipo de cuerpo), no
handles. Motivo: la migración a wgrapier tiene que ser reemplazar un crate, no reescribir
el engine.

### El mundo contiene escenas, NO al revés (#566)

Verificado contra engines ECS reales: Bevy escribe `Scene`/`DynamicScene` DENTRO de un
`World`; el `World` de Unity DOTS contiene la meta-entidad de escena y sus section
entities; el `UWorld` de Unreal contiene levels → UE5 World Partition = grilla espacial.

**Escena y celda son ORTOGONALES, no anidadas.** Una escena abarca muchas celdas; varias
escenas se solapan en una celda. Celda = "¿qué hay cerca?" (espacial, DERIVADO del
transform, nunca campo guardado). Escena = "¿qué autoró junto un diseñador?" (lógico).
Storage por `(escena, celda)` = archivos per-scene-section de Unity. Anidarlos costaría
escenas solapadas en el mismo volumen, edición paralela sin conflicto en git, y la
distinción entre descargar una escena y descargar una celda.

`SceneDocument` NO se reemplaza: pasa a ser el payload por `(escena, celda)`. El
`.ome_scene` actual es "el mundo entero en una sección" — la sección 0 de Unity antes de
partir nada. **Sección siempre residente** (carga primera, descarga última) = ahí vive la
entidad de focus, que no puede ser dueña de una celda que se descarga sola.

### SDF: murió la técnica, vive el dato (#565)

`ome_sdf` y `ome_bvh` ELIMINADOS, ~14k líneas. Criterio del usuario: *se queda lo que sirve
al sistema galáctico y a planetas terraformables; lo demás muere y se rehace bien contra el
motor de física actual.* Nada de "tal vez lo usemos" — si hace falta, cherry-pick del
historial.

Fuera: brushes CSG en WGSL, los 7 componentes `Sdf*` de ome_ecs, `ome_bvh` entero (flags
`IS_RAYMARCH`/`ROLE_RAYMARCH_*`; su único consumidor era contenido que nadie rendeaba desde
el pivote), `ChunkContent`, `ProceduralCitySource` (el editor la registraba y generaba
primitivas para un pool que nadie leía — "nunca funcionó" = desconectada, no rota),
`VolumePrimitive`. Adentro: `ome_world::voxel` (sparse storage con LOD, mudado desde
ome_sdf) y todo el streaming por distancia. `Aabb` se mudó a `ome_core`.

**No confundir los dos SDF**: murió SDF-como-técnica-de-render/authoring. Vive
SDF-como-formato-de-dato (un vóxel guarda una distancia con signo) — de ahí extrae malla
dual contouring, y el collider de vóxel de rapier lo consume directo. Para física no hace
falta lo primero.


- **Meshlet shading = two-pass all-fragment.** NO binding_array bindless, NO compute deferred.
  Material variants futuras = shaders dedicados vía `compose_material_shader`.
- **Meshlet grouping (LOD chain DAG) = graph-based METIS k-way**, NUNCA spatial (Morton/Voronoi/
  k-means/hilbert/octree). Probado empíricamente en #470: spatial schemes ignoran topología →
  coverage holes en LODs altos. Edge weights = shared-vertex count (crítico). Crate `metis`
  (LIHPC) `default-features=false, features=["vendored"]` compila estático (Bazzite atomic OK).
  **Cell-boundary vertex-lock sigue necesario** incluso con METIS (minimiza pero no elimina
  shared edges; el simplify destruiría border vertices sin lock explícito).
- **Capability detection runtime + cargo features SOLO por build target, NUNCA por vendor.**
  Un binario corre en cualquier vendor; paths se eligen al startup según `wgpu::Features`.
  Baseline: RDNA 2 / Turing / Adreno X1 (2020+).
- **Bevy define el techo de wgpu.** Ante "¿podemos hacer X de UE5/Nanite?": primero ¿lo hace
  Bevy? Si no, asumir limitación wgpu hasta probar lo contrario. Reference canon:
  `reference_bevy_meshlet_shader.md` (asistente).
- **GPU-driven ≠ DOD-shaped servido a fragment naïve.** El espíritu es: hot loop entera en
  compute, persistent buffers para visibility+work, indirect dispatch, cero readback.
- **prev_lod_indices Vec<u32>** (#535 H3), **parent.lod_error monotone-clamped** (#535 H1) —
  tests invariante en lod_chain.

### Editor remoto (locked 2026-07-21, épico C)

- **El proyecto es dueño del ECS; el editor standalone es cliente.** No cargar tipos del
  proyecto en el editor (no hay ABI Rust estable). Protocolo HTTP tipo BRP (`ome_remote`).
- **Transporte = `tiny_http` SÍNCRONO en thread lateral + puente mpsc al main.** NUNCA async/
  tokio en el core del engine. El server no toca el ECS fuera del main thread (`Stage::First`).
- **UNA fuente de datos + UN seam de edición.** ECS local (real o espejo `RemoteMirror`) leído
  por paneles/viewport igual en ambos modos; la ÚNICA bifurcación es `apply_actions` →
  `remote_edit::dispatch` si `RemoteState::is_connected()`. NO duplicar paths por modo.
- **Identidad cross-proceso = nombre cualificado, NUNCA TypeId.** `ComponentId` interned es
  process-local; el cable lleva nombres. `TypeId` solo para reflexión local.
- **Viewport modelo C** (hub rendea el espejo), entrada **A→B** (Open Remote separado hoy →
  Open Project remoto cuando madure). NO modelo B directo (sin fallback), NO embed de ventana
  (roto en Wayland).

### Lighting stack (locked 2026-05-06)

- **Diffuse GI = Surfel radiance cache + voxel/DC coupling (#450).** NO Radiance Cascades
  (#114 closed as not planned; reopen solo si Sannikov publica RC 3D shipping). SSGI cancelado.
- **Specular = SSR Hi-Z + parallax-corrected probes (#478) + RT futuro** detrás de feature flag.
  Probes specular-only, surfels diffuse-only, sin overlap.
- **Direct shadows: CSM (#476) → VSM (#477).** Shadow sampling abstracto detrás de UNA function
  call en deferred — swap por reemplazo de impl, no de call sites.
- **Sky: stellar_delivery port (#248) ahora → Hillaire 2020 upgrade path** (shader-only, no rompe
  `AtmosphereVolume` API).
- **Volumetric fog = Froxel grid 3D** (#32 reescrito), NO per-pixel raymarch.

---

## Gotchas activos / lecciones

- **rustfmt version drift (CRÍTICO):** rustfmt local (1.8.0) reformatea TODO el repo distinto a
  como está commiteado, y NO hay toolchain pin ni CI fmt-check. **NUNCA `git add -A crates/<x>/`
  después de `cargo fmt -p <x>`** → barre 100+ archivos de churn. Formatear solo archivos propios,
  `git add <archivos específicos>`. En #545 hubo que revertir 115 archivos de churn antes de pushear.
- **Repo SIN CI:** `.github/workflows/` vacío. "Verde" = MERGEABLE+CLEAN. Verificación 100% local:
  `cargo test`, pipeline-creation test en device real (`vbuf64_stage_pipeline_creation`), smoke del editor.
- **Color space loader:** `ImageLoader` registrado como `srgb()` global → normal/metal_rough se
  cargan en sRGB (incorrecto). Follow-up: hint de color-space por-asset en `.meta`. Albedo (sRGB) ya OK.
- **material_depth = Depth16Unorm:** `f32(id)/65535` round-trip exacto; per-material pass usa `Equal`.
  El vs del material shader DEBE emitir `screen.material_id/65535` como z (dynamic-offset UBO `screen`,
  un slot por material) + `@invariant`.
- **meshlet geometry BGL ahora incluye FRAGMENT** (gpu_meshlet.rs) para reusar `meshlet_bg` en el
  fragment material pass.
- **wgpu 29 gotchas:** `DepthStencilState.depth_write_enabled: Option<bool>`, `depth_compare:
  Option<CompareFunction>`. `Instance::new(InstanceDescriptor{...})` por VALOR (sin `&`). Color
  texture del render stage necesita `RENDER_ATTACHMENT`. `FLOAT32_FILTERABLE` required para linear
  sampler sobre R32Float. `R16Float` NO expone `STORAGE_BINDING` garantizado → usar R32Float.
- **Reversed-Z:** nuevos passes → clear `0.0`, comparator `Greater`/`GreaterEqual`.
  `ome_render::perspective_rh_reverse_z` helper canon. Frustum: usar `row2` para clip.z>=0,
  NUNCA `row3 + row2` (eso es OpenGL [-1,1]).
- **Mesa radv SIGSEGV** con test threads paralelos en crates que init wgpu → `--test-threads=1`.
  naga parse+validate a nivel lib corre seguro sin GPU.
- **Empty-scene draw call floor = 3** (sky + meshlet pass A + pass B indirect): no es leak, es costo
  fijo GPU-driven indirect.

---

## Backlog visual (post-#440) — orden sugerido

- ⭐ **#440** Texture references — pipeline MERGED (#545), issue OPEN para tooling de verificación.
- **[sugerido]** Material inspector + asset browser (prerequisito para verificar #440 y usable de verdad).
- **#441** PBR real (Cook-Torrance + sun + IBL) — `metal_roughness` ya bindeado/reservado; helpers
  baricéntricos + tangent ya existen.
- **#482** triplanar/world-space projection · **#483** foliage BTDF · **#484** HDR (AgX + auto-exposure
  + LUT) · **#485** Clustered Forward+ light culling.
- **#476** CSM → **#477** VSM · **#450** Surfel GI · **#478** reflection probes · **#480** denoiser
  · **#481** motion vectors + FSR 2.x.
- **#453** skinned mesh GPU · **#452** forward transparent · **#444** mesh shaders · **#443** bindless
  · **#392** render graph propio (cuando el stack tenga 4+ passes).
- **#536** vendor upscaling plugin (DLSS/FSR/XeSS) · **#537** cargo features por build target.
- Cerradas recientes: #543 (mesh-frame bench), #544 (timestamp HUD), #542 (flicker LOD).

## Editor — capacidades shipped

Viewport panel → `egui::Image` (offscreen ViewportTarget) · Inspector gimbal-safe (Euler-cached) +
Local/World rotation toggle · drag-drop Components · Hierarchy propagation · `GlobalTransform::lossy_scale()`
+ warning shear · Editor camera (orbit MMB / pan Shift+MMB / zoom rueda / fly RMB+WASD+QE / focus F) ·
Scene serialization `.ome_scene` (RON) con `EphemeralComponents` filter · SceneManager (path+dirty+load/save)
· default scene auto-create · gizmos (translate/rotate/scale + snap + Local/World) · undo/redo per drag ·
native asset picker · persistent dock layout · Perf HUD (FPS/CPU/GPU/RAM/VRAM/draws) · mdBook docs en `docs/book/`
· **Asset Browser** (árbol del crate + drag-drop import + menú contextual + inline create/rename + drag
asset→slot Inspector) · material editor en Inspector · IDE picker · **codegen de proyecto** (scaffolds
component/system + `registrations.rs` + editor embebido + gating) · **cliente remoto** (`RemoteSession` +
`RemoteMirror` + dual-sink, ver épico C — falta activación 4b.3b).

---

## Workflow rules (NEVER violate sin OK explícito del user)

- **Branch first** (`feat/<slug>` desde `development`, nunca directo). Después de crear PR: **STOP**
  salvo que el user pida mergear.
- SOLO `gh pr merge --merge` (**NUNCA squash** — rompe git graph). Conventional commits EN, **sin AI
  signatures / Co-Authored-By**.
- PRs a `development` NO auto-cierran issues (merge a non-default) → cerrar a mano si corresponde.
- **state-of-art production-ready desde commit 1, NO MVP** (`feedback_correct_implementation_day_one`).
- Cada subtask = 1 commit (`git add` específico, NO `-A` tras fmt).
- El user maneja el fin de sesión; el smoke visual lo maneja el user (el agente arranca la app y diagnostica).

## Docs de referencia in-repo

- `docs/decisions/0001_mesh_format.md` — mesh format ADR (glTF + OBJ).
- `docs/research/stack_decisions_2026-05-02.md` — stack choices + rationale.
- `docs/research/implementation_checklist_2026-05-02.md` — phased roadmap con exit gates.
- `docs/research/editor-three-system-architecture.md`, `sdf-csg-composition.md`, `wgpu-capabilities.md`.
- `docs/book/` — mdBook.
