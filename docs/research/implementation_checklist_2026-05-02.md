# OhMyEngine — Implementation Checklist (Post-Pivot)

**Fecha base:** 2026-05-02
**Master plan:** `docs/research/stack_decisions_2026-05-02.md`

Orden de ataque diseñado para **matar el path SDF de render lo antes posible** y construir el pipeline mesh GPU-driven (Nanite-style) en capas estables.

Cada fase tiene gate de exit explícito — no se avanza hasta que el gate pasa.

---

## Fase 0 — Kill SDF render path (PRIORIDAD MÁXIMA)

**Objetivo:** sacar SDF raymarching del path de render del editor. Mesh-only desde ya. SDF queda como autoring tool sin visual.

**Tiempo estimado:** 1-3 días.

- [ ] Branch `feat/kill-sdf-render`
- [ ] Editor: remover llamada a `raymarch.update_scene()` + `raymarch.render()` en `viewport/render.rs`
- [ ] Editor: el flow queda sky_pass → mesh_pass (sin raymarch en el medio)
- [ ] `RayMarchPlugin` y `examples/raymarch_demo.rs` quedan FUNCIONALES (standalone) pero no se usan en el editor
- [ ] Verificar que la escena de prueba TestEngine2.0 muestre solo mesh (Suzanne) sin diagonales
- [ ] Add comentario en `RayMarchRenderer` señalando que entra en mantenimiento mínimo
- [ ] Commit + PR a development

**Gate exit:**
- ✅ Editor levanta y renderiza mesh sin diagonales
- ✅ `cargo build -p ome_editor` clean
- ✅ Las entidades SDF (cylinder, torus, etc.) en la escena ya no aparecen visualmente — esperado, queda como TODO hasta dual contouring
- ⚠️ NO se borra código todavía — solo desconexión

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

## Fase 1.E — Polish + Stop SDF Code

**Objetivo:** archivar el código SDF de render y dejar el engine limpio.

**Tiempo estimado:** 1-2 semanas.

- [ ] **#396** — archive raymarch + tile-cull modules
  - [ ] Mover `crates/ome_render/src/raymarch/` a `crates/ome_render/src/_archived_raymarch/` o branch separado
  - [ ] Mover `crates/ome_render/src/tile_cull/` igual
  - [ ] Eliminar `RayMarchPlugin` del default features
  - [ ] Deprecar `examples/raymarch_demo.rs` y `examples/raymarch_hierarchy_demo.rs`
  - [ ] Limpiar tests: AC1-AC7 + raymarch_* + tile_cull + gdf_* + pool_eval_smoke
- [ ] PRESERVAR: `crates/ome_sdf/` (autoring + dual contouring future), `sdf_primitives.wgsl` (reusado)

**Gate exit:**
- ✅ `cargo build --workspace` clean sin código SDF de render
- ✅ Tests verdes
- ✅ Engine size reducido (medir LOC delta)

---

## Fase 2 — Virtual Geometry + Streaming + Dual Contouring (#395)

**Objetivo:** alcanzar 60-70% de Nanite (Bevy 0.16 equivalente).

**Tiempo estimado:** 2-3 meses focused.

- [ ] LOD chain por mesh (`meshopt::simplify`)
- [ ] DAG meshlet jerárquico (cluster groups + LOD boundary error metric)
- [ ] Software rasterizer GPU compute para meshlets sub-pixel (Nanite trick)
- [ ] Streaming async (tokio + binary mesh format propio)
- [ ] **#393** — Bridge SDF → Mesh via dual contouring (devuelve la identidad SDF como autoring)
- [ ] Speculative LOD fade-in / fade-out

**Gate exit:**
- ✅ Escenas de millones de triángulos a 60fps
- ✅ Streaming de chunks sin hitches
- ✅ Autor en SDF en editor → ve mesh extraído renderizando

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
| **Fase 0** | Kill SDF render path | 1-3 días |
| **Fase 1.A** | Asset Pipeline foundation | 1-2 semanas |
| **Fase 1.B** | Subsystem trait abstractions | 1-2 semanas (paralelo) |
| **Fase 1.C** | Render graph propio | 2-3 semanas |
| **Fase 1.D** | Meshlet pipeline (Nanite-style) | 6-10 semanas |
| **Fase 1.E** | Archive SDF render code | 1-2 semanas |
| **Fase 1 total** | Mesh GPU-driven pipeline operativa | **~3-4 meses** |
| **Fase 2** | Virtual geometry + streaming + dual contouring | 2-3 meses |
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
