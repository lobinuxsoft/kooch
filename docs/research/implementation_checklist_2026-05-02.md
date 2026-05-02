# OhMyEngine — Implementation Checklist (Post-Pivot)

**Fecha base:** 2026-05-02
**Master plan:** `docs/research/stack_decisions_2026-05-02.md`

Orden de ataque diseñado para **matar el path SDF de render lo antes posible** y construir el pipeline mesh GPU-driven (Nanite-style) en capas estables.

Cada fase tiene gate de exit explícito — no se avanza hasta que el gate pasa.

---

## Fase 0 — Eliminar SDF completamente (PRIORIDAD MÁXIMA)

**Objetivo:** SDF afuera del engine. Sin "archivar", sin preservar como autoring tool, sin dual contouring future. Engine mesh-only.

**Tiempo estimado:** 2-4 días.

- [ ] Branch `feat/delete-sdf`
- [ ] **Editor:** remover llamada a `raymarch.update_scene()` + `raymarch.render()` en `viewport/render.rs`. Flow queda sky_pass → mesh_pass
- [ ] **Eliminar crate `crates/ome_sdf/`** completo
- [ ] **Eliminar módulos render SDF:**
  - `crates/ome_render/src/raymarch/` (directorio entero)
  - `crates/ome_render/src/raymarch_plugin.rs`
  - `crates/ome_render/src/tile_cull/` (directorio entero)
  - `crates/ome_render/shaders/raymarch_*.wgsl`
  - `crates/ome_render/shaders/tile_cull.wgsl`
  - `crates/ome_render/shaders/gdf_populate.wgsl`
  - `crates/ome_render/shaders/raymarch_gdf_sample.wgsl`
  - `crates/ome_render/shaders/raymarch_pool_*.wgsl`
- [ ] **Eliminar examples:**
  - `examples/raymarch_demo.rs`
  - `examples/raymarch_hierarchy_demo.rs`
- [ ] **Eliminar tests SDF:**
  - `crates/ome_render/tests/ac1_byte_identical.rs`
  - `crates/ome_render/tests/ac2_multi_chunk_traversal.rs`
  - `crates/ome_render/tests/ac3_streaming_round_trip.rs`
  - `crates/ome_render/tests/ac6_load_order_determinism.rs`
  - `crates/ome_render/tests/ac7_pool_fragmentation.rs`
  - `crates/ome_render/tests/ac_363_demo_scene_traversal.rs`
  - `crates/ome_render/tests/gdf_*.rs`
  - `crates/ome_render/tests/pool_eval_smoke.rs`
  - `crates/ome_render/tests/raymarch_*.rs`
  - `crates/ome_render/tests/tile_cull.rs`
- [ ] **Eliminar componentes SDF de `ome_ecs`:**
  - `SdfSphere`, `SdfBox`, `SdfCapsule`, `SdfCylinder`, `SdfTorus`, `SdfPlane`
  - Categoría "SDF" del Add Component menu
  - Submenú SDF del Spawn menu
- [ ] **Evaluar `ome_bvh` y `ome_world`:** si solo servían al raymarcher, eliminar también. Si tienen valor general (ECS hierarchy, streaming genérico), preservar
- [ ] **Workspace cleanup:**
  - Remover `ome_sdf` (y crates afectados) de `Cargo.toml` workspace members
  - Remover features `sdf`, `lighting` (si era SDF-only) del root crate
  - Remover deps SDF de los demás crates
- [ ] **TestEngine2.0:** abrir el proyecto de prueba y remover scenas/escripts que referenciaban SDF
- [ ] Commit + PR a development

**Gate exit:**
- ✅ `cargo build --workspace` clean — sin código SDF en ninguna parte
- ✅ `cargo test --workspace` verde
- ✅ `cargo clippy --workspace -- -D warnings` clean
- ✅ Editor levanta, renderiza mesh sin diagonales, sin features SDF en menúes
- ✅ TestEngine2.0 abre limpio (puede tener escenas vacías hasta que carguemos meshes nuevas)
- ✅ LOC delta documentado en el PR (debería ser reducción significativa)

---

## Fase 1.A — Foundation Asset Pipeline

**Objetivo:** poder cargar meshes glTF y referenciarlas tipadamente. Sin esto no hay nada que renderizar realmente.

**Tiempo estimado:** 1-2 semanas.

- [ ] **#191** — Decisión documentada: glTF primary, OBJ secondary opcional (1 sesión)
- [ ] **#184** — `AssetHandle<T>` system: identificadores tipados, ref-count opcional, registry global (3-5 días)
- [ ] **#391** — `AssetLoader<T>` trait + impls iniciales:
  - [ ] `GltfLoader` (mesh + material + scene tree)
  - [ ] `RonSceneLoader` (formato actual del engine)
  - [ ] `ImageLoader` (PNG/JPG vía crate `image`)
- [ ] **#129** — Mesh Loading: glTF → buffers GPU (positions, normals, uvs, indices), staging async
- [ ] **#131** — Texture Loading: PNG/JPG → `wgpu::Texture` con mipmap auto

**Gate exit:**
- ✅ `assets.load::<Mesh>("models/suzanne.glb")` devuelve `Handle<Mesh>` válido
- ✅ MeshRenderer puede tener `mesh: Handle<Mesh>` (no String placeholder) y renderizar
- ✅ Tests de loaders verificando bytes correctos cargados

---

## Fase 1.B — Subsistem Trait Abstractions (paralelo a 1.A)

**Objetivo:** abstraer subsistemas para permitir swap futuro de providers sin reescribir game code.

**Tiempo estimado:** 1-2 semanas (paralelo).

- [ ] **#387** — `PhysicsBackend` trait + `RapierBackend` impl
- [ ] **#137** (re-scoped) — `CollisionShape` componente que mappea a `rapier3d::shape`
- [ ] **#388** — `InputBackend` trait + `ActionMap` + `WinitGilrsBackend` impl
- [ ] **#390** — `AudioBackend` trait + `KiraBackend` impl
- [ ] **#389** — `ScriptingBackend` trait + `RhaiBackend` impl

**Gate exit:**
- ✅ Cada trait compila y un primer impl pasa tests básicos
- ✅ Game code referencia traits (no providers concretos)

---

## Fase 1.C — Render Graph Foundation

**Objetivo:** orquestación declarativa de passes para que la pipeline sea extensible.

**Tiempo estimado:** 2-3 semanas.

- [ ] **#392** — Render graph propio (inspirado en `rend3::graph`)
  - [ ] Nodos con inputs/outputs declarados
  - [ ] Resource lifetime tracking (transient resources)
  - [ ] Barriers automáticas (image layout transitions)
  - [ ] Topological sort + ciclo detection
  - [ ] Migrar passes existentes (sky, mesh) al nuevo graph
- [ ] Documentar API + ejemplo

**Gate exit:**
- ✅ Sky + mesh passes corren a través del graph
- ✅ Adding/removing passes no requiere tocar el orchestrator
- ✅ Frame time medible per-node

---

## Fase 1.D — Meshlet Pipeline (#117 — el Nanite-style)

**Objetivo:** virtual geometry / meshlet pipeline GPU-driven. **Esto es lo que reemplaza definitivamente al SDF como render principal.**

**Tiempo estimado:** 6-10 semanas (la fase más densa).

### Sub-fase 1.D.1 — Offline Meshlet Generation
- [ ] Add `meshopt` crate al workspace
- [ ] Tool: glTF → meshlet binary (cluster triangles, generate AABB per meshlet, adjacency)
- [ ] Asset format: `MeshletMesh` con vertex pool + meshlet array + bounds array
- [ ] `MeshletLoader` impl
- [ ] Test: Suzanne y un asset complex (Bistro) procesan sin errores

### Sub-fase 1.D.2 — GPU Compute Culling
- [ ] Per-instance frustum culling (compute pass que escribe lista de visible meshlets)
- [ ] Per-meshlet AABB vs frustum
- [ ] Indirect args buffer
- [ ] Bench: ms del culling pass para 10k/100k/1M meshlets

### Sub-fase 1.D.3 — Indirect Draw
- [ ] `draw_indexed_indirect_count` para batches de visible meshlets
- [ ] Bindless vertex pool (single mega-buffer + index offsets)
- [ ] Verificar que Suzanne renderiza igual que con mesh pass directo

### Sub-fase 1.D.4 — Hi-Z Occlusion Culling (2-pass)
- [ ] Pass 1: render meshlets visibles del frame anterior → depth buffer parcial
- [ ] Build Hi-Z mip chain (depth pyramid)
- [ ] Pass 2: re-test todos los meshlets contra Hi-Z, agregar nuevos visibles
- [ ] Bench: % de meshlets descartados en escena densa

### Sub-fase 1.D.5 — Visibility Buffer
- [ ] Render meshlets a R64Uint texture (meshlet_id + tri_id en bits)
- [ ] Compute shading pass: lee visibility buffer, calcula bary, sample atributos del vertex pool, computa material
- [ ] Output a color + depth buffer estándar
- [ ] Verificar zero overdraw (cada pixel se shadea una vez)

### Sub-fase 1.D.6 — Bindless Materials
- [ ] Structured buffer global de materiales
- [ ] Texture array bindless (wgpu BindingArray feature)
- [ ] Meshlet → material idx mapping
- [ ] Material params: PBR (albedo + normal + metallic + roughness + emissive)

### Sub-fase 1.D.7 — Mesh Shaders (cuando viable)
- [ ] Feature gate: `Features::EXPERIMENTAL_MESH_SHADER` cuando disponible
- [ ] Path mesh shader: task shader → mesh shader → fragment
- [ ] Fallback path: compute culling + indirect draw (1.D.2/1.D.3)
- [ ] Runtime detection + selection

**Gate exit Phase 1.D:**
- ✅ Render frame de escena con Suzanne + 100+ meshes diversos
- ✅ Frame time < 16ms en GPU mid-range (Steam Deck APU)
- ✅ Visibility buffer funcional, no overdraw
- ✅ Hi-Z descartando >50% de meshlets en escena densa
- ✅ Material PBR básico funcionando

---

## Fase 1.E — (eliminada)

Mergeada en Fase 0. La eliminación de SDF se hace upfront, no al final de Phase 1.

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

## Fase 3 — Planetary Scale (#313)

**Objetivo:** escala planetaria real, precisión + LOD.

**Tiempo estimado:** 3-4 meses focused.

- [ ] **#394** — research floating origin / hierarchical reference frames (decisión arquitectónica primero)
- [ ] **#51** — Camera-relative transform implementación
- [ ] Cubed sphere quadtree terrain
- [ ] Heightmap streaming + GPU virtual texture
- [ ] Atmospheric scattering (Bruneton)
- [ ] **#341** + **#342** — CelestialBody LOD pipeline + impostor cubemap

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
| **Fase 0** | Eliminar SDF completamente | 2-4 días |
| **Fase 1.A** | Asset Pipeline foundation | 1-2 semanas |
| **Fase 1.B** | Subsystem trait abstractions | 1-2 semanas (paralelo) |
| **Fase 1.C** | Render graph propio | 2-3 semanas |
| **Fase 1.D** | Meshlet pipeline (Nanite-style) | 6-10 semanas |
| **Fase 1 total** | Mesh GPU-driven pipeline operativa | **~3-4 meses** |
| **Fase 2** | Virtual geometry + streaming | 2-3 meses |
| **Fase 3** | Planetary scale | 3-4 meses |
| **Total roadmap** | Engine planetary-scale shipeable | **8-11 meses focused** |

Realista con vida normal (tiempo parcial): **18-30 meses**.

---

## Principios de operación

- **Una fase a la vez** — no saltar adelante. Cada fase tiene gate explícito que debe pasar.
- **Branch por fase/issue**, según el workflow normal del repo (no commits directos a development).
- **Tests + bench cada gate** — sin esto no se avanza.
- **PR a development**, merge solo cuando gate pasa.
- **Re-evaluar checklist al final de cada fase** — el plan puede ajustarse con datos nuevos, pero solo en boundary de fase, no a mitad.
