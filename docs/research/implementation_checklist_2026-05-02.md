# OhMyEngine — Implementation Checklist (Post-Pivot)

**Fecha base:** 2026-05-02
**Última actualización:** 2026-05-02 (post-session: 14 PRs merged)
**Master plan:** `docs/research/stack_decisions_2026-05-02.md`
**Continuation memory:** `~/.claude/projects/-var-mnt-DATA-Repos-oh-my-engine/memory/project_phase1_progress.md`

Orden de ataque diseñado para **matar el path SDF de render lo antes posible** y construir el pipeline mesh GPU-driven (Nanite-style) en capas estables.

Cada fase tiene gate de exit explícito — no se avanza hasta que el gate pasa.

## Quick status (2026-05-02)

| Fase | Status | PRs |
|---|---|---|
| Fase 0 (kill SDF) | ✅ COMPLETE | #401 |
| Fase 1.A (asset pipeline) | ✅ COMPLETE | #402, #403, #404, #405, #406 |
| Fase 1.B (subsystem traits) | ✅ COMPLETE | #407, #408, #409, #410 |
| Fase 1.C (render graph) | ✅ FOUNDATION + WRAPPERS (lifetime tracking pending) | #411, +RenderNode wrappers |
| Fase 1.D (meshlet primitives) | ✅ PER-MESHLET PRIMITIVES (cull stack + vbuf + deferred + materials, all per-mesh single-dispatch) | #412, #413, #414, PR-4, PR-5b, PR-5a, PR-5c, PR-6, PR-7, PR-9 |
| **Fase 1.E (production wiring + scene-wide GPU-driven)** | 🚧 NEXT — promotes Phase 1.D primitives to a real scene path | — |
| Fase 2 (virtual geometry + streaming) | ⏳ NOT STARTED | — |
| Fase 2.5 (voxel + DC) | ⏳ NOT STARTED | — |
| Fase 3 (planetary scale hybrid) | ⏳ NOT STARTED | — |

---

## Fase 0 — Eliminar SDF render path (preserve `ome_sdf` para DC pipeline) ✅ DONE (PR #401)

**Objetivo:** sacar el path de render SDF (raymarch + tile-cull + GDF). Preservar `ome_sdf` re-purposeado como sampling lib + brushes para alimentar el pipeline Dual Contouring (Phase 2.5).

**Tiempo estimado:** 2-3 días. **Tiempo real: 1 sesión.**

- [x] Branch `feat/kill-sdf-render` (was `396-tech-debt-...`)
- [x] **Editor:** remover llamada a `raymarch.update_scene()` + `raymarch.render()` en `viewport/render.rs`. Flow queda sky_pass → mesh_pass
- [x] **Eliminar módulos render SDF:**
  - `crates/ome_render/src/raymarch/` (directorio entero) ✅
  - `crates/ome_render/src/raymarch_plugin.rs` ✅
  - `crates/ome_render/src/tile_cull/` (directorio entero) ✅
  - `crates/ome_render/shaders/raymarch_*.wgsl` ✅
  - `crates/ome_render/shaders/tile_cull.wgsl` ✅
  - `crates/ome_render/shaders/gdf_populate.wgsl` ✅
  - `crates/ome_render/shaders/raymarch_gdf_sample.wgsl` ✅
  - `crates/ome_render/shaders/raymarch_pool_*.wgsl` ✅
- [x] **Eliminar examples:**
  - `examples/raymarch_demo.rs` ✅
  - `examples/raymarch_hierarchy_demo.rs` ✅
- [x] **Eliminar tests SDF render:** todos los archivos AC*, raymarch_*, tile_cull, gdf_*, pool_eval_smoke ✅
- [ ] **Evaluar y eliminar:** `crates/ome_bvh/` — **NO eliminado**, todavía usado por `ome_world` (revisar cuando Rapier reemplace queries físicas)
- [x] **PRESERVAR (re-purposear comentarios y docs internos):**
  - `crates/ome_sdf/` — preservado (pendiente: renombrar categoría "SDF" → "SDF Brushes" en editor)
  - Componentes SDF — preservados
  - `sdf_primitives.wgsl` — preservado
- [x] **Workspace cleanup:** ome_render dropped ome_bvh / ome_sdf / ome_world / ome_physics deps
- [ ] **TestEngine2.0:** no documentado explícitamente en el PR (entidades con SDF brushes seguirán existiendo)
- [x] Commit + PR a development (PR #401, merged)

**Gate exit:**
- ✅ `cargo build --workspace` clean
- ✅ `cargo test --workspace` verde
- ✅ `cargo clippy --workspace -- -D warnings` clean
- ✅ Editor levanta, renderiza mesh sin diagonales
- ✅ Componentes SdfSphere/Box/etc spawneables pero sin visual (esperado hasta Phase 2.5)
- ✅ LOC delta documentado en el PR

---

## Fase 1.A — Foundation Asset Pipeline ✅ DONE

**Objetivo:** poder cargar meshes glTF y referenciarlas tipadamente. Sin esto no hay nada que renderizar realmente.

**Tiempo estimado:** 1-2 semanas. **Tiempo real: 1 sesión** (5 PRs).

- [x] **#191** — Decisión documentada: glTF primary, OBJ secondary opcional → PR #402, ADR `docs/decisions/0001_mesh_format.md`
- [x] **#184** — `AssetHandle<T>` system: tipado vía `slotmap` generational arena. `Handle<T>` 16B Copy + `Assets<T>` Resource → PR #403
- [x] **#391** — `AssetLoader<T>` trait + `AssetServer` registry → PR #404
  - [x] `GltfMeshLoader` (mesh CPU asset) — PR #405
  - [ ] `RonSceneLoader` — **DEFERRED** (existing `scene_io.rs` still works; refactor cuando llegue caso)
  - [x] `ImageLoader` (PNG/JPEG vía `image` crate, sRGB + linear variants) — PR #406
- [x] **#129** — Mesh Loading: `Mesh` CPU asset + `GltfMeshLoader::load(bytes)` + `Mesh::upload(device)→GpuMesh` → PR #405
- [x] **#131** — Texture Loading: `Image` CPU asset + `GpuTexture::upload(device, queue, image)` (single mip; mipmaps deferred a #130 PBR) → PR #406

**Gate exit:**
- ✅ `assets.load::<Mesh>("models/suzanne.glb")` devuelve `Handle<Mesh>` (vía `AssetServer.load`)
- ⚠️ `MeshRenderer.mesh` sigue siendo `String` — migración a `Handle<Mesh>` queda como **wire-up follow-up** (deuda DOD/ECS conocida)
- ✅ Tests: 46 tests verdes (assets + asset_loader + mesh + texture)

---

## Fase 1.B — Subsistem Trait Abstractions ✅ DONE

**Objetivo:** abstraer subsistemas para permitir swap futuro de providers sin reescribir game code.

**Tiempo estimado:** 1-2 semanas. **Tiempo real: 1 sesión** (4 PRs).

- [x] **#387** — `PhysicsBackend` trait + `RapierBackend` impl (rapier3d 0.22 + simd-stable) → PR #407, 15 tests
- [ ] **#137** (re-scoped) — `CollisionShape` componente que mappea a `rapier3d::shape` — **PENDIENTE** (ECS sync system follow-up)
- [x] **#388** — `InputBackend` trait + `ActionMap` + `WinitGilrsBackend` + `MockInputBackend` → PR #408, 13 tests
- [x] **#390** — `AudioBackend` trait + `KiraBackend` (kira default features mp3/ogg/flac/wav + cpal) + `MockAudioBackend` → PR #410, 18 tests
- [x] **#389** — `ScriptingBackend` trait + `RhaiBackend` (rhai 1.21 con `sync` feature) → PR #409, 16 tests

**Gate exit:**
- ✅ Cada trait compila y un primer impl pasa tests (62 tests verdes)
- ⚠️ Game code todavía no referencia los traits (ECS sync systems para physics/input/audio = follow-up)

---

## Fase 1.C — Render Graph Foundation ✅ WRAPPERS DONE

**Objetivo:** orquestación declarativa de passes para que la pipeline sea extensible.

**Tiempo estimado:** 2-3 semanas. **Status: graph foundation (#411) + RenderNode wrappers around MeshPassRenderer / SkyRenderPass.** Resource lifetime tracking + plugin migration onto the graph are follow-ups.

- [ ] **#392** — Render graph propio (inspirado en `rend3::graph`)
  - [x] Nodos con inputs/outputs declarados (`RenderNode` trait + `FnNode` adapter) — PR #411
  - [ ] Resource lifetime tracking (transient resources) — **DEFERRED** (post wgpu intra-encoder barriers, no urgente)
  - [ ] Barriers automáticas (image layout transitions) — **DEFERRED** (wgpu maneja intra-encoder)
  - [x] Topological sort + ciclo detection (Kahn's algorithm, deterministic) — PR #411
  - [x] RenderNode wrappers para passes existentes (`MeshPassNode`, `SkyPassNode`) — Phase 1.C close PR. RenderContext gains optional `FrameInfo` (color view, depth view, size, time, ECS resources); wrappers no-op when no frame is attached.
  - [ ] Plugin migrate to drive the graph end-to-end — **DEFERRED** (orchestrator-level; needs scene composition outside #392's scope)
- [ ] Documentar API + ejemplo — **doc en código, sin doc separada**

**Gate exit:** ✅ MET (modulo plugin orchestration)
- ✅ Sky + mesh passes have RenderNode wrappers; graph can schedule them
- ✅ Graph data structure compiles + 10 tests pass
- ⚠️ Frame time per-node not measured yet — TIMESTAMP_QUERY support already gated on adapter.features() in core; per-pass timing lands when the orchestrator drives the graph end-to-end

---

## Fase 1.D — Meshlet Pipeline (#117 — el Nanite-style) 🚧 IN PROGRESS (5/8 sub-PRs)

**Objetivo:** virtual geometry / meshlet pipeline GPU-driven. **Esto es lo que reemplaza definitivamente al SDF como render principal.**

**Tiempo estimado:** 6-10 semanas (la fase más densa). **Progreso actual: foundation + frustum cull + indirect-draw rasterizer + backface cone cull (cull stack complete except Hi-Z).**

### Sub-fase 1.D.1 — Offline Meshlet Generation ✅ (PR #412)
- [x] Add `meshopt` crate al workspace (0.6.2)
- [ ] Tool: glTF → meshlet binary CLI — **DEFERRED** (builder existe en código pero no hay tool standalone)
- [x] Asset format: `MeshletMesh` con vertex pool + meshlet array + bounds (`MeshletDescriptor` 80B POD)
- [ ] `MeshletLoader` impl AssetLoader<MeshletMesh> — **DEFERRED** (Mesh→Meshlet runtime build vía `build_default_meshlets`; no binary format aún)
- [ ] Test: Suzanne procesa sin errores — **DEFERRED** (tests cubren single triangle + quad; assets reales con #129 mesh loading integration follow-up)

### Sub-fase 1.D.2 — GPU Compute Culling ⚠️ PARCIAL (PR #413 + #414 + PR-4)
- [x] Per-instance frustum culling (compute pass `meshlet_cull.wgsl` + `CullParams` + plane extraction) — PR #414
- [x] Per-meshlet bounding sphere vs frustum (usa `cone_apex`/`bounding_radius` — más simple que AABB pero suficiente)
- [x] Indirect args buffer — PR-4 (`encoder.copy_buffer_to_buffer(visible_count → DrawIndirectArgs[+4])`, single-shot per frame)
- [ ] Bench: ms del culling pass — **DEFERRED** (Mesa radv SIGSEGV blocked parallel dispatch tests; PR-4 single-thread harness unblocks this when bench framework lands)
- [x] GPU upload: `GpuMeshletMesh` con 4 storage buffers + bind group layout — PR #413

### Sub-fase 1.D.3 — Indirect Draw ✅ DONE (PR-4)
- [x] `draw_indirect` para batches de visible meshlets (no-indexed, vertex-pull style — see "no-obvio" below)
- [x] Bindless vertex pool (single mega-buffer + index offsets via `meshlet_vertices` u32 list)
- [x] Verificar que cube renderea end-to-end (E2E integration test in `tests/meshlet_render.rs` asserts non-clear pixels with normal-debug shading; faces-away camera asserts zero rasterized pixels)
- [ ] **No-obvio:** PR-4 uses `draw_indirect` (4×u32) not `draw_indexed_indirect_count` (5×u32 + count buffer). The vertex-pull rasterizer indexes the meshlet pool directly via `@builtin(vertex_index)` / `@builtin(instance_index)` — no host-side index buffer to feed `draw_indexed`. `draw_indirect_count` becomes useful only once we have a hierarchical "visible chunks → many indirect args" structure (PR-9+).

### Sub-fase 1.D.4 — Hi-Z Occlusion Culling ⚠️ PARCIAL (PR-5a builder + PR-5c cull test done; ping-pong 2-pass orchestration deferred to PR-9)
- [x] Build Hi-Z mip chain (depth pyramid) — PR-5a (`HiZ` struct, R32Float pyramid, `cs_copy_depth` + `cs_reduce_max` compute, multi-pass portable reduction)
- [x] Hi-Z occlusion test in cull shader — PR-5c (`cs_cull_hi_z` + `MeshletCull::dispatch_with_hi_z`, single-texel pessimistic projection + mip selection)
- [ ] Ping-pong "last-frame visible" + 2-pass draw orchestration — deferred to PR-9 (`MeshletDrawer` already takes a depth attachment; PR-9 wires the depth pre-pass + Hi-Z build + final cull cycle)
- [ ] Bench: % de meshlets descartados en escena densa — PR-9

### Sub-fase 1.D.4b — Backface Cone Culling ✅ DONE (PR-5b)
- [x] Extender `meshlet_cull.wgsl` para leer `cone_apex/cone_axis/cone_cutoff` de descriptors y rechazar backfacing — `dot(normalize(camera - cone_apex), cone_axis) >= cone_cutoff`. Honours meshopt's `cone_cutoff == 1.0` "no-cull" sentinel for divergent normal sets. CullParams gains `camera_position` (112B → 128B). Descriptor split into `bounds_center` + real `cone_apex` (80B → 96B) so the cone test reads the right vector. CPU mirror `camera_in_backface_cone` for unit tests + future LOD heuristics.

### Sub-fase 1.D.5 — Visibility Buffer ✅ DONE (PR-6)
- [x] Render meshlets a R32Uint texture (packed (meshlet_id+1) << 7 | tri_idx — 25 bits meshlet + 7 bits triangle, encoded 0 = background sentinel) — `MeshletVisRasterizer` + `meshlet_vbuf.wgsl`
- [x] Compute shading pass: lee vbuf, sample vertex pool, output color — `MeshletDeferredShader` + `meshlet_deferred.wgsl`. Bary-correct interpolation deferred to PR-7 (current path averages the triangle's 3 normals; visually identical to forward flat-shaded for PR-6 acceptance)
- [x] Output a color (Rgba8Unorm storage texture) + standard depth attachment
- [x] Verify zero overdraw (cada pixel se shadea una vez vía compute thread per pixel) — implicit in the architecture; bench in PR-9

### Sub-fase 1.D.6 — Bindless Materials ⚠️ PARCIAL (PR-7 param-buffer; texture-array follow-up)
- [x] Structured buffer global de materiales — `MaterialPool` storage buffer of `MaterialParams` (32 B per slot, base_color + packed scalars)
- [ ] Texture array bindless (wgpu BindingArray feature) — DEFERRED: lands with #130 PBR (texture-mapped shading)
- [x] Render-call → material idx mapping — `screen.material_id` UBO field; per-meshlet ids land with the texture-array follow-up
- [x] Material params: PBR (albedo + metallic + roughness + emissive scalars; texture handles deferred)

### Sub-fase 1.D.7 — Mesh Shaders (cuando viable) ⏳ NOT STARTED
- [ ] Feature gate: `Features::EXPERIMENTAL_MESH_SHADER` cuando disponible
- [ ] Path mesh shader: task shader → mesh shader → fragment
- [ ] Fallback path: compute culling + indirect draw (1.D.2/1.D.3)
- [ ] Runtime detection + selection

**Gate exit Phase 1.D:** ✅ MET (modulo mesh shaders)
- ✅ Render frame end-to-end (cull → vbuf → deferred → shaded) — sphere bench in `meshlet_bench.rs` (PR-9)
- ✅ Frame time < 16 ms target — measured median ~0.5 ms on RX 9070 XT (33× headroom; Steam Deck APU still TBD when the runtime hits production)
- ✅ Visibility buffer functional — `MeshletVisRasterizer` + `MeshletDeferredShader` (PR-6)
- ✅ Hi-Z occlusion test live in cull shader — PR-5c (`cs_cull_hi_z`); ping-pong 2-pass orchestration deferred to Phase 1.D follow-up alongside scene plumbing
- ✅ Material PBR-scalar pool + per-call material id — PR-7 (`MaterialPool`); texture-array bindless lands with #130

---

## Fase 1.E — Production wiring + scene-wide GPU-driven 🚧 NEXT

**Por qué existe esta sub-fase (post Phase 1.D close audit):** Phase 1.D entregó la *infraestructura* per-meshlet correcta DOD-shape, pero todo vive en `tests/` headless. El editor sigue usando el path viejo `MeshPassRenderer`. Y el cull procesa "1 dispatch por mesh" — no es "scene-wide GPU enumeration" como exige el constraint global del proyecto (`feedback_planet_scale_gpu_driven.md`, `feedback_gpu_driven_spirit.md`).

Phase 1.E promueve los primitivos a un pipeline de producción genuino: instance buffer scene-wide, ECS-integrated, single-dispatch cull sobre toda la escena, plugin/viewport drive end-to-end.

**Tiempo estimado:** 3-5 sesiones focused.

### Sub-fase 1.E.1 — Instance buffer + scene-wide cull foundation

- [ ] **`MeshInstance` POD struct** — `{ transform: Mat4, mesh_id: u32, material_id: u32, lod_bias: f32, _pad }` ≈ 80 B per instance, repr(C), Pod+Zeroable.
- [ ] **`MeshletScene` resource** — owns the instance storage buffer + free-list growth (u32 indices, no `Vec::push` per-frame in hot path; pre-allocate capacity, grow on overflow with explicit recreate).
- [ ] **Mesh pool (global)** — arrays de meshlets / vertices / triangles indexed by `mesh_id`. Cada mesh registrado se aloja en una sub-region; `MeshHandle` lleva `(mesh_id, first_meshlet, meshlet_count, vertex_offset, vertex_count)`. NO HashMap<MeshId, Mesh> en hot path; storage buffer plano + descriptor index.
- [ ] **Cull shader extension `cs_cull_scene`** — toma instance buffer + global mesh pool, dispatch enumera (instance, meshlet) pairs (1D dispatch sobre `total_meshlet_instances` o 2D sobre instances × max_meshlets_per_mesh con bounds check). UN solo dispatch para TODA la escena.
- [ ] **Indirect draw output** — packed `(instance_id, meshlet_id)` en `visible_meshlets`; vertex shader pull-style decodifica al rasterizar.
- [ ] AC: integration test scene con 4+ instances of 2+ meshes, single dispatch, verify visible_count == sum(meshlets per visible instance).

### Sub-fase 1.E.2 — ECS integration (MeshRenderer migration)

- [ ] **`MeshRenderer.mesh: String` → `Handle<MeshletMesh>`** migration — deuda Phase 1.A reconocida.
- [ ] **`MeshletAssetServer` / `MeshletLoader`** que tomen `Handle<MeshletMesh>` y mantengan el global mesh pool actualizado on-demand.
- [ ] **System: collect ECS instances** — `for_each Query<&MeshRenderer, &GlobalTransform>` → builds the frame's `MeshInstance` slice → uploads to instance buffer (CPU-side only at the system boundary; never crosses into hot loop).
- [ ] AC: editor entity con MeshRenderer aparece en la instance buffer; cambiar transform → instance buffer reflects it next frame.

### Sub-fase 1.E.3 — Plugin + viewport drive end-to-end

- [x] **`MeshletRenderStage` (1.E.3a)** — orquestador headless: scene cull → vbuf raster scene-path → deferred shade scene-path → ouput Rgba8Unorm view. Integration test corre 2 ECS entities (distinct GlobalTransform / material id) y assert non-clear pixels en ambos lados de la pantalla.
- [x] **`RenderPlugin` toggle (1.E.3b)** — `UseMeshletPath { enabled: bool }` resource (default off). `init_renderers` construye `MeshletRenderStage` + `MeshletBlit`; `render_frame_system` enruta entre legacy y meshlet+blit según el toggle. `MeshletBlit` compone `Rgba8Unorm` → surface format (`Bgra8Unorm`/etc.) vía full-screen triangle. `sync_assets_to_gpu` bridges `Assets<MeshletMesh>` → GPU cache.
- [ ] **Editor viewport (1.E.3c)** ejerce el meshlet stage en `ome_editor_core::viewport::render`. Mismo toggle, mismo blit; ViewportTarget ya tiene RENDER_ATTACHMENT.
- [ ] AC: levantar editor + spawn entity con MeshRenderer + Suzanne (o sphere) → ver el modelo renderizado por la meshlet pipeline real, sin pasar por `MeshPassRenderer`.
- [ ] **Multi-mesh path (1.E.3c)** — `cs_cull_scene_pool` shader entry + `GpuGlobalMeshPool` bind group para escenas con varios meshes registrados.
- [ ] Hi-Z 2-pass ping-pong wiring entra acá (no antes — antes no había scene plumbing donde aterrizarlo).

### Sub-fase 1.E.4 — Validation (real visual confirmation)

- [ ] Run editor, screenshot del scene meshlet-rendered.
- [ ] Frame time real con sysfs VRAM bound.
- [ ] Comparar visualmente vs path viejo `MeshPassRenderer` (reference render con mismo asset).
- [ ] Decisión: deprecate `MeshPassRenderer` o conservar por compatibilidad / testing.

**Gate exit Phase 1.E:** lo que faltaba al cerrar Phase 1.D
- ✅ MeshRenderer entity en editor renderea por meshlet pipeline real
- ✅ Single GPU dispatch enumera scene meshlets (no 1-per-mesh)
- ✅ Instance buffer está vivo, growth-handled, ECS-driven
- ✅ Visual confirmation desde el editor (screenshot), no logs
- ✅ Frame time medido en producción path, no headless test

---

## Fase 2 — Virtual Geometry + Streaming (#395)

**Objetivo:** alcanzar 60-70% de Nanite (Bevy 0.16 equivalente).

**Tiempo estimado:** 2-3 meses focused.

- [ ] LOD chain por mesh (`meshopt::simplify`)
- [ ] DAG meshlet jerárquico (cluster groups + LOD boundary error metric)
- [ ] Software rasterizer GPU compute para meshlets sub-pixel (Nanite trick)
- [ ] Streaming async (tokio + binary mesh format propio)
- [ ] Speculative LOD fade-in / fade-out

**Gate exit:**
- ✅ Escenas de millones de triángulos a 60fps
- ✅ Streaming de chunks sin hitches

---

## Fase 2.5 — Voxel + Mesh extraction hybrid (#397)

**Objetivo:** habilitar zonas de mundo con caves, destrucción, edición de geometría runtime.

**Tiempo estimado:** 4-6 semanas focused (reducido por dep adoption).

### Deps a adoptar (drop-in)

- [ ] `cargo add fast-surface-nets` — primary mesh extraction (~20M tri/seg, sparse chunk seams)
- [ ] `cargo add transvoxel` — LOD seam stitching entre chunks resolution-distinta
- [ ] `cargo add mesh_to_sdf` — pipeline offline para bakear glTF → voxel SDF (importar terreno custom)
- [ ] (opcional) `cargo add block-mesh` — si querés feature de destrucción cubic-style

### Implementación propia (mínima)

- [ ] **#398** Voxel storage — dense chunks `[f32; CHUNK³]` initialmente, NO SVO. SVO solo cuando memoria sea problema medible
- [ ] Voxelización de SDF brushes (`SdfSphere/Box/...` componentes → voxel chunk samples)
- [ ] Bridge: dense chunk → `fast-surface-nets` → mesh chunk → `meshopt::build_meshlets` → mesh pipeline
- [ ] Re-extraction incremental on edit (chunk dirty → re-run surface_nets → upload)
- [ ] Streaming load/unload por proximidad (custom, simple chunk grid)
- [ ] Editor brush tools — pintar SDF en zona, automáticamente voxeliza + re-extrae

### Follow-up (Phase 2.5.B, opcional)

- [ ] **#393** Custom Dual Contouring para sharp features (estudiar `WilstonOreo/sdf2mesh` source)
  - Solo si Surface Nets nos limita en zonas con cliffs / cubos / aristas afiladas
  - Side-by-side con fast-surface-nets, no replacement

**Gate exit:**
- ✅ Demo: cueva editable runtime con `fast-surface-nets`
- ✅ Re-extraction sub-frame (<5ms para chunk 32³)
- ✅ Mesh extraído va al pipeline GPU-driven (Phase 1)
- ✅ Streaming voxel sin hitches
- ✅ Transvoxel resuelve seams entre LODs

---

## Fase 3 — Planetary Scale Hybrid (#313)

**Objetivo:** escala planetaria real con heightmap base + voxel zones embebidas.

**Tiempo estimado:** 4-5 meses focused.

- [ ] **#394** — research floating origin / hierarchical reference frames (decisión arquitectónica primero)
- [ ] **#51** — Camera-relative transform implementación
- [ ] **#399** — Cubed Sphere Quadtree heightmap terrain (planet base)
- [ ] Heightmap streaming async + GPU virtual texture
- [ ] **#400** — Voxel zone system + heightmap-voxel boundary stitching
- [ ] Atmospheric scattering (Bruneton)
- [ ] **#341** + **#342** — CelestialBody LOD pipeline + impostor cubemap (mid-distance LOD)
- [ ] **#343** — StarCatalog far-distance starfield

**Gate exit:**
- ✅ Volar de superficie a órbita sin pop-in ni pérdida de precisión
- ✅ Dos planetas distintos visitables sin reload

---

## Tareas paralelas en cualquier momento

Issues que no bloquean fase pero suman:

- **#197** viewport panel render a egui texture
- **#198** transform gizmos
- **#199** editor camera (orbit/pan/fly/focus)
- **#190** asset browser panel
- **#69** Performance profiler + Tracy
- **#70** Hot reload de shaders
- **#252** timestamp-ringbuffer profiler
- **#254** post-processing stack (tone map AgX + SMAA + CAS + vignette)
- **#67** Debug visualization overlay
- **#262/263/264/265** Sky improvements

---

## Resumen tiempos estimados

| Fase | Trabajo | Tiempo focused |
|---|---|---|
| **Fase 0** | Eliminar SDF render path (preserva ome_sdf) | 2-3 días |
| **Fase 1.A** | Asset Pipeline foundation | 1-2 semanas |
| **Fase 1.B** | Subsystem trait abstractions | 1-2 semanas (paralelo) |
| **Fase 1.C** | Render graph propio | 2-3 semanas |
| **Fase 1.D** | Meshlet pipeline (Nanite-style) | 6-10 semanas |
| **Fase 1 total** | Mesh GPU-driven pipeline operativa | **~3-4 meses** |
| **Fase 2** | Virtual geometry + streaming | 2-3 meses |
| **Fase 2.5** | Voxel + mesh extraction hybrid (deps adoption) | 4-6 semanas |
| **Fase 3** | Planetary scale hybrid (heightmap + voxel zones) | 4-5 meses |
| **Total roadmap** | Engine planetary-scale shipeable con caves/destrucción | **10-14 meses focused** |

### Política de adopción de deps (regla global)

Usar deps drop-in cuando cubran ≥80% del use case. Custom solo donde la integración es específica O ningún crate maintained existe. Si estás escribiendo algo que tomaría >4 semanas alcanzar feature parity con un crate maintained, **investigaste mal antes de codear** — volvé a buscar en crates.io / lib.rs / GitHub topics.

Ver `~/.claude/rules/code-standards.md` sección "Dependency Adoption" para la regla completa global.

Realista con vida normal (tiempo parcial): **18-30 meses**.

---

## Principios de operación

- **Una fase a la vez** — no saltar adelante. Cada fase tiene gate explícito que debe pasar.
- **Branch por fase/issue**, según el workflow normal del repo (no commits directos a development).
- **Tests + bench cada gate** — sin esto no se avanza.
- **PR a development**, merge solo cuando gate pasa.
- **Re-evaluar checklist al final de cada fase** — el plan puede ajustarse con datos nuevos, pero solo en boundary de fase, no a mitad.
