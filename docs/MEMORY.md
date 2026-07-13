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

`ome_core, ome_ecs, ome_window, ome_input, ome_sdf, ome_lighting, ome_render, ome_physics,
ome_gravity, ome_world, ome_audio, ome_scripting, ome_editor_core, ome_editor, ome_bvh,
ome_gizmos, ome_gizmos_handles, ome_editor_api`. Facade top-level `oh_my_engine` con
`DefaultPlugins` PluginGroup (estilo Bevy).

`ome_sdf` sobrevive el pivot: repurposed como authoring-tool/brushes para el pipeline
voxel + DC de Phase 2.5. No es el render path actual.

---

## ⭐ Estado actual (development HEAD `9d8c6a6`, 2026-07-02)

- `development` limpio, **cero PRs abiertos**. origin solo tiene `main` + `development`.
- Editor corre limpio en RX 9070 XT (Vulkan/RADV): cero validation warnings, smoke OK en
  producción + todos los debug modes.

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
native asset picker · persistent dock layout · Perf HUD (FPS/CPU/GPU/RAM/VRAM/draws) · mdBook docs en `docs/book/`.

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
