# OhMyEngine — Decisiones de Stack y Roadmap

**Fecha:** 2026-05-02
**Status:** Decisión cerrada. Fundamenta el pivot de render y la integración de physics.

---

## 1. Cambio de paradigma — SDF deja de ser render principal

### Lo que se descarta

- **SDF raymarching como render path principal.** Razón: 6 PRs de optimización (cascade GDF, BVH pool, TLAS+BLAS, streaming chunks, tile-cull pre-pass) acumulados sobre algo que conceptualmente es sphere-tracing simple. Cada capa agrega coordinación CPU/GPU que se desincroniza fácil. El bug "diagonales" del 2026-05-02 (PR #386 tile-cull) reveló que la pre-pass discardaba todo cuando `flags == 0` porque cascade 5 (voxel pitch 8 km) no detecta geometría sub-kilométrica.
- **Cascade GDF + tile-cull + TLAS/BLAS pool como infraestructura "Nanite-tier para SDF".** Sin precedente shipeable. Claybook + Dreams + Dual Universe — los 3 engines que más empujaron SDF — terminan extrayendo geometría discreta antes del render final.
- **Issue #360 (sparse chunk LOD) y #370 (tile-cull) en su forma actual.** El trabajo realizado queda como referencia histórica; no se construye más encima.

### Lo que se elimina (revisión final 2026-05-02 — plan C hybrid)

**Solo el path de render SDF se elimina. SDF como representación se preserva y repurposea para alimentar el pipeline Dual Contouring.**

A ELIMINAR:
- `crates/ome_render/src/raymarch/` (directorio entero)
- `crates/ome_render/src/raymarch_plugin.rs`
- `crates/ome_render/src/tile_cull/` (directorio entero)
- Todos los shaders raymarch_*, tile_cull, gdf_*, raymarch_pool_*
- `examples/raymarch_demo.rs`, `examples/raymarch_hierarchy_demo.rs`
- Tests AC1-AC7, raymarch_*, tile_cull, gdf_*, pool_eval_smoke
- `crates/ome_bvh/` (solo lo usaba el raymarcher)

A PRESERVAR (re-purposeado):
- `crates/ome_sdf/` — repurposeado como **"SDF sampling lib + brushes para voxel authoring"** (alimenta DC)
- Componentes SDF (`SdfSphere/Box/Capsule/Cylinder/Torus/Plane`) — son **brushes** que generan SDF samples para voxelizar (no render directo)
- `sdf_primitives.wgsl` — alimenta el pipeline DC
- `crates/ome_world/` — generaliza a streaming de mesh chunks (probable que sí)

### Lo que se construye en su lugar — pipeline híbrido

```
Heightmap Quadtree (superficie base, escala planetaria)
        +
Sparse Voxel Octree + Dual Contouring (caves, destrucción, ciudades subterráneas)
        ↓
Mesh GPU-driven pipeline (Phase 1 Nanite-style) — ambos feedean el mismo renderer
        ↓
Floating origin / hierarchical reference frames (precisión a escala planetaria)
```

- **Mesh GPU-driven pipeline** (Nanite-style virtual geometry). Phase 1. Referencias: Bevy 0.16 `virtual_geometry`, `Firestar99/nanite-at-home`, Karis SIGGRAPH 2021.
- **Sparse Voxel Octree + Dual Contouring** (Phase 2.5). Para zonas de caves / destrucción / edición runtime. Referencias: Dual Universe, Ju et al. 2002.
- **Cubed Sphere Quadtree heightmap** (Phase 3). Para terreno planetario base. Referencias: Outerra, Star Citizen Terra Firmer.
- **Mesh import** vía glTF — Blender/external tools para assets propiamente dichos (props, vegetación, edificios). SDF brushes son para autoring procedural / destructible terrain.

---

## 2. Stack cerrado

### Subsistemas core

| Subsistema | Crate | Versión | Estado actual | Acción |
|---|---|---|---|---|
| **GPU API** | `wgpu` | 29 | ✅ Tenés | Mantener. Mesh shaders desde v28 |
| **Shader language** | WGSL → migrar a `rust-gpu` progresivo | — | ✅ WGSL hoy | Adopción incremental de rust-gpu para nuevos compute shaders |
| **Math** | `glam` + `bytemuck` | 0.29 + 1.21 | ✅ Tenés | Sin cambios. `wgmath` (dimforge) cuando estabilice |
| **Mesh import** | `gltf` | 1.4 | ✅ Tenés | Implementar carga (issue #129) |
| **Meshlet generation offline** | `meshopt` | latest | ❌ Falta | Agregar al Cargo.toml en Fase 1 |
| **Render graph** | Custom (inspirado en `rend3::graph`) | — | ❌ Falta | Construir propio, ~2-3 semanas |
| **Physics** | `rapier3d` | latest | ❌ Falta | Agregar + diseñar trait `PhysicsBackend` |
| **Window + kbd + mouse** | `winit` | 0.30 | ✅ Tenés | Sin cambios |
| **Gamepad** | `gilrs` | 0.11 | ✅ Tenés | Sin cambios |
| **Input actions** | Custom (`ome_input`) | — | ❌ Falta | Issue #55 (Input Action) |
| **Editor GUI** | `egui` + `egui_dock` + `egui-wgpu` | 0.34 / 0.19 / 0.34 | ✅ Tenés | Sin cambios |
| **File dialogs** | `rfd` | 0.15 | ✅ Tenés | Sin cambios |
| **Audio** | `kira` | 0.9 | ✅ Tenés | Sin cambios |
| **Scripting** | `rhai` | 1.21 | ✅ Tenés | Sin cambios |
| **Serde** | `serde` + `ron` | 1 / 0.8 | ✅ Tenés | Sin cambios |
| **Plugins dinámicos** | `stabby` + `libloading` | 72.1 / 0.8 | ✅ Tenés | Sin cambios |

### Justificaciones por subsistema

#### Rendering
- **`wgpu`**: cross-platform (Vulkan/Metal/DX12/Web), Rust-native, mesh shaders desde v28, mantenido por gfx-rs. No hay alternativa razonable en Rust.
- **WGSL → rust-gpu progresivo**: rust-gpu (Rust-GPU foundation, originalmente Embark) compila Rust a SPIR-V. Permite compartir código CPU↔GPU sin reescribir. Dimforge migrando a esto en 2026; vale seguirles el paso.
- **Pipeline meshlet + visibility buffer + 2-pass occlusion culling + indirect draw**: Bevy 0.16 alcanza 60-70% de Nanite con esta receta. Es la state-of-the-art portable.
- **NO Bevy entera**: migración de 6+ meses, perdés tu arquitectura.
- **NO `rend3` (maintenance mode)**: copiar diseño, no la dep.
- **NO `renderling` todavía**: prometedora pero alpha.
- **NO `kajiya`**: research/learning, no shipping.

#### Physics
- **`rapier3d`**: production AAA-grade (dimforge), DOD interno (islands SoA, índices `u32`), maneja decenas de miles de bodies. Maduro.
- **NO `bevy_rapier`**: agregaría dep de Bevy innecesaria.
- **NO `avian` (XPBD)**: más simple pero menos features (CCD débil).
- **NO `wgrapier3d` hoy**: alpha. Faltan CCD, sleeping islands, joints complejos. Path para 2027+.
- **Trait `PhysicsBackend` desde el día 1**: permite migrar a wgrapier3d cuando madure sin reescribir game code.
- **SDF queries SEPARADAS**: collision contra terreno SDF queda fuera del trait Physics — son `eval(p)` sobre `eval_scene_bvh` (o equivalente), no rigid body.

#### Inputs
- **`winit` + `gilrs`**: combo standard del ecosistema Rust. ggez y Amethyst migraron a esto. No hay debate.
- **Action mapping propio**: ~300 líneas, integra mejor con tu ECS que `winit-input-map` o `input-actions` (asumen Bevy-style architecture).

#### Editor
- **`egui` + `egui_dock`**: immediate mode, Rust-idiomatic, integra con wgpu nativo. `imgui-rs` agrega C++ dep + macros para strings null-terminated.
- **NO `iced` / `Slint` / retained-mode**: editor de engine necesita immediate-mode (UI cambia per-frame con escena).

#### Audio
- **`kira` 0.9**: mixer + effects + tweens + parámetros automatizados. Game-grade.
- **NO `rodio`**: playback simple, less features.
- **`oddio` opcional para spatial AAA**: sumar como segundo backend si llegás a HRTF/ambisonics. Caso raro.

---

## 3. Próximas adiciones concretas

### Crates nuevos al `Cargo.toml`

```toml
# Physics
rapier3d = { version = "...", features = ["simd-stable"] }

# Meshlet generation (offline tool)
meshopt = "..."
```

### Implementación propia

- **Trait `PhysicsBackend`** en `ome_physics/lib.rs` — métodos `step`, `add_body`, `query_ray`, `query_shape`. Primer impl: `RapierBackend`.
- **Sistema de acciones** en `ome_input` — desbloquea issues #55, #56-61.
- **Render graph** en `ome_render` — nodos con inputs/outputs, transient resources, automatic barriers.

---

## 4. Roadmap de fases

### Fase 1 — Pipeline GPU-driven base
**Tiempo estimado:** 2-3 meses focused

- [ ] Meshlet generation offline (`meshopt` crate, glTF → meshlet binary)
- [ ] Compute frustum culling (per-meshlet AABB vs frustum)
- [ ] Hi-Z / occlusion culling (two-pass: render previous frame visible → build Hi-Z → cull)
- [ ] Indirect draw (`draw_indexed_indirect_count`)
- [ ] Mesh shaders (`Features::EXPERIMENTAL_MESH_SHADER`, fallback compute+indirect)
- [ ] Visibility buffer (meshlet+tri ID, deferred shading)
- [ ] Bindless materials (structured buffer global + meshlet→material idx)
- [ ] Render graph propio (basado en diseño de `rend3::graph`)

### Fase 2 — Virtual geometry + streaming
**Tiempo estimado:** 2-3 meses

- [ ] LOD chain por mesh (`meshopt::simplify`)
- [ ] DAG meshlet jerárquico (cluster groups + LOD boundary error metric)
- [ ] Streaming async (tokio + binary mesh format)
- [ ] Software rasterizer GPU compute para meshlets sub-pixel (Nanite trick)

### Fase 3 — Planetary scale
**Tiempo estimado:** 3-4 meses

- [ ] **Floating origin + camera-relative transforms** — REWRITE de Transform a `f64` storage, view matrices relativas. Toca todo el ECS. Hacer ANTES de tener mucho contenido
- [ ] Hierarchical reference frames (sub-grids con origen local — Star Citizen Object Containers)
- [ ] Cubed sphere quadtree (6 caras × quadtree dinámico, splits/merges por cámara)
- [ ] Heightmap streaming + GPU virtual texture (tile-based desde disco)
- [ ] Atmospheric scattering (Bruneton precomputed — ~1 semana)
- [ ] Sky + stars + sun

### Total realista
- **Full focused:** 7-10 meses solo engine, sin gameplay
- **Con vida real:** 18-24 meses

---

## 5. Issues a revisar dado el pivot

### Cerrar como superseded/wontfix

- **Follow-up ray-march** (agregar box/capsule/cylinder/torus/plane al shader): el render path SDF no se extiende más. **CERRAR**
- **#360 epic (sparse chunk LOD)**: revisar scope. La streaming GPU pool sirve si llegás a dual contouring chunked, pero la implementación actual es para raymarch
- **#370 epic (tile-cull, GDF cascade)**: contrato roto, código vivo. Decisión: **archivar** como referencia histórica, NO build encima

### Mantienen validez

- **#197** viewport panel render a egui texture
- **#198** transform gizmos
- **#199** editor camera (orbit/pan/fly/focus)
- **#129** mesh loading (glTF) — **critical path** para Fase 1
- **#184** asset handle — **critical path** para todo
- **#191** decision: mesh formats glTF/OBJ — decidir antes de #129
- **#137** CollisionShape — **adapta a Rapier** (convex hull / capsule / box → `rapier3d::shape`)
- **#39** Physics Integration — pasa a "integrate Rapier" en lugar de custom
- **#55** Input Action — sin cambios
- **#192** SkinnedMeshRenderer + Skeleton + AnimationPlayer — encaja con mesh pipeline

### Issues nuevas a crear

- **Research: floating origin / hierarchical reference frames** — decisión arquitectónica previa a Fase 3
- **Epic: Mesh GPU-driven pipeline (Fase 1)** — meshlet + visibility buffer + culling + indirect
- **Epic: Virtual geometry + streaming (Fase 2)** — DAG, LOD, streaming
- **Epic: Planetary scale (Fase 3)** — floating origin, cubed sphere, atmosphere
- **`PhysicsBackend` trait + `RapierBackend` impl** — base para `ome_physics`
- **Bridge SDF → Mesh (dual contouring)** — Fase 2

### Issues a futuro lejano

- **wgrapier3d migration** — cuando dimforge llegue a feature parity con Rapier CPU (~2027+)
- **rust-gpu shader rewrite** — cuando wgmath estabilice y dimforge complete migration

---

## 6. Pendientes de decisión arquitectónica (no hoy)

| Decisión | Cuándo decidir | Bloquea |
|---|---|---|
| **Floating origin / hierarchical reference frames** | Antes de Fase 3 | Planetary scale entero |
| **rust-gpu adoption timeline** | Cuando wgmath estabilice (~2026 H2) | Code-sharing CPU↔GPU |
| **wgrapier3d swap** | 2027+ cuando feature parity | GPU-driven physics full |
| **Asset format final** | Antes de #129 | Pipeline de assets |
| **Software rasterizer en compute para meshlets sub-pixel** | Fase 2 final | Nanite-grade pixel-density |

---

## 7. Referencias técnicas

### Rendering — meshlet / virtual geometry
- [Virtual Geometry in Bevy 0.16 — JMS55](https://jms55.github.io/posts/2025-03-27-virtual-geometry-bevy-0-16/) — best Rust doc
- [Virtual Geometry in Bevy 0.15](https://jms55.github.io/posts/2024-11-14-virtual-geometry-bevy-0-15/)
- [Virtual Geometry in Bevy 0.14](https://jms55.github.io/posts/2024-06-09-virtual-geometry-bevy-0-14/)
- [Bevy PR #10164 — Meshlet rendering initial feature](https://github.com/bevyengine/bevy/pull/10164)
- [Firestar99/nanite-at-home (Rust + Vulkan master thesis)](https://github.com/Firestar99/nanite-at-home)
- [Scthe/nanite-webgpu (WGSL + WebGPU)](https://github.com/Scthe/nanite-webgpu)
- [pettett/multires (Rust + Vulkan)](https://github.com/pettett/multires)
- [Recreating Nanite: Visibility buffer — jglrxavpok](https://jglrxavpok.github.io/2023/11/26/recreating-nanite-visibility-buffer.html)
- [Karis Nanite SIGGRAPH 2021 paper](https://advances.realtimerendering.com/s2021/Karis_Nanite_SIGGRAPH_Advances_2021_final.pdf)
- [wgpu mesh shading API spec v29](https://github.com/gfx-rs/wgpu/blob/v29/docs/api-specs/mesh_shading.md)

### Rendering — render graph / lib references
- [rend3 docs — render graph design](https://docs.rs/rend3/latest/rend3/graph/index.html) (maintenance mode, buen design)
- [renderling](https://github.com/schell/renderling) (alpha, GPU-driven con rust-gpu)
- [kajiya — Embark](https://medium.com/embarkstudios/homegrown-rendering-with-rust-1e39068e56a7) (research)

### Physics
- [Rapier docs](https://rapier.rs/)
- [Dimforge 2025 review + 2026 plans](https://dimforge.com/blog/2026/01/09/the-year-2025-in-dimforge/)
- [wgrapier3d on docs.rs](https://docs.rs/wgrapier3d/latest/wgrapier3d/)
- [Position-Based Simulation Methods — Macklin et al](https://mmacklin.com/EG2015PBD.pdf)

### Planetary scale
- [Real-time Procedural Universe — Game Developer](https://www.gamedeveloper.com/programming/a-real-time-procedural-universe-part-three-matters-of-scale)
- [Outerra](https://www.outerra.com/)
- [Planetary Scale LOD Terrain — Leif Node](https://leifnode.com/2014/04/planetary-scale-lod-terrain-generation/)
- [Continuous World Generation in No Man's Sky — GDC](https://www.gdcvault.com/play/1024265/Continuous-World-Generation-in-No)
- [Dual Universe Voxel Tech](https://dualuniverse.fandom.com/wiki/Voxel_Technology)
- [Implementing Dual Contouring — Nick's Voxel Blog](https://ngildea.blogspot.com/2014/11/implementing-dual-contouring.html)
- [Generating mesh from SDFs with Dual Contouring — Henrique Gois](https://henriquegois.dev/posts/generating-mesh-from-sdfs-with-dual-contouring/)

### rust-gpu / shaders en Rust
- [Rust-GPU](https://github.com/Rust-GPU/rust-gpu)
- [Rust running on every GPU (2025)](https://rust-gpu.github.io/blog/2025/07/25/rust-on-every-gpu/)
- [Notes on migrating WGSL → rust-gpu — dev.to](https://dev.to/bardt/notes-on-migrating-from-wgsl-to-rust-gpu-shaders-56bg)

---

## 8. Principios que NO cambian

- **DOD + GPU-Driven** sigue siendo no-negociable para render + simulación masiva
- **Composition over enumeration** — un componente por concepto, sin enums
- **Trait-based abstraction** para physics + render backends — permite swap sin reescribir game code
- **Cada subsistema en su crate** — preserva separation of concerns
- **No depender de Bevy** — la arquitectura propia es identidad del engine

---

## 9. Lo que NO se decide acá

- Gameplay features
- Diseño narrativo / mecánicas
- Stack de networking (no discutido)
- Stack de UI in-game (egui es solo editor; in-game UI es decisión separada)
- Tooling externo (Blender plugins, asset pipelines custom)

Esos quedan para sesiones de planning específicas cuando llegue el momento.
