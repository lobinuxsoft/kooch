# ADR 0001 — Soporte de Formatos de Mesh

**Status:** Accepted
**Date:** 2026-05-02
**Issue:** #191
**Bloquea:** #129 (Mesh Loading)
**Relacionado:** #184 (Asset Handle), #180 (MeshRenderer), #391 (AssetLoader<T>)

---

## Contexto

El engine necesita decidir qué formatos de mesh soporta antes de implementar el sistema de loading (#129). La decisión bloquea Phase 1.A del roadmap post-pivot 2026-05-02 (mesh-only render pipeline).

Restricciones del proyecto:
- Stack Rust nativo (sin C++ vendoring)
- Cross-platform (Vulkan/Metal/DX12 vía wgpu)
- Target inicial handheld (Steam Deck, OneXFly)
- Compatibilidad con tools externas (Blender principalmente)
- Roadmap incluye mesh + skeletal + animation (#192) + PBR (#130)

---

## Decisión

### Phase 1 (MVP)

| Formato | Rol | Crate |
|---|---|---|
| **glTF / GLB** | Primario | `gltf` 1.4 |
| **OBJ + MTL** | Secundario (prototipado) | `tobj` |

### Phase 2 (futuro, post-MVP)

- **Formato binario propio** post-bake desde glTF, optimizado para load directo a GPU. Análogo a Unity `.asset` o Godot `.scn`. Triggered cuando perf de loading sea problema medible.

### Fuera de scope (rechazados)

- **FBX**
- **USD / USDZ**
- **Collada (DAE)**
- **STL** (sin materiales)
- **PLY** (académico)

---

## Alternativas consideradas

### glTF / GLB (✅ Aceptado primario)

**Pros:**
- Estándar Khronos abierto, royalty-free
- Soporta full pipeline: meshes estáticos + skinned + animations + PBR materials + scene hierarchy + lights + cameras
- Format binario (`.glb`) optimizado para runtime load
- `gltf` crate maduro, mantenido, en producción
- Blender export nativo, todas las DCC tools modernas exportan
- Spec versionada estable (2.0 desde 2017)
- Extensiones para features avanzadas (Draco compression, KTX2 textures, mesh quantization)

**Contras:**
- Spec extensa pero manejable
- JSON header requiere parsing antes del binario

### OBJ + MTL (✅ Aceptado secundario)

**Pros:**
- Trivial de parsear (`tobj` crate ~minimal)
- Humano-legible, debuggable a mano
- Universal soporte en todas las tools
- Rápido para prototipar / unit tests

**Contras:**
- Solo geometría estática (sin skinning, sin animation)
- Materiales limitados (MTL es pre-PBR)
- Sin scene hierarchy
- Performance loading mediocre (text parsing)

**Justificación de inclusión**: para test fixtures + assets de debug donde glTF es overkill. Bajo costo de mantenimiento (`tobj` es ~200 LoC de uso).

### FBX (❌ Rechazado)

**Pros:**
- Estándar industria (Maya, Max, MotionBuilder)
- Skeletal + animation rico

**Contras decisivos:**
- **Licencia Autodesk ambigua** — el SDK oficial requiere acuerdo comercial
- **Formato binario propietario** — los parsers open-source son reverse-engineered, frágiles
- **Sin crate Rust serio** — `fbxcel` existe pero es alpha, último commit hace años
- Requiere vendoring del FBX SDK C++ para soporte completo
- Blender exporta FBX pero pierde data en round-trip

**Decisión:** ningún path legal + técnico razonable en Rust. Workflow alternativo: Maya/Max → FBX → Blender → glTF.

### USD / USDZ (❌ Rechazado)

**Pros:**
- Futuro de Pixar, Apple RealityKit, NVIDIA Omniverse
- Stage composition powerful (variants, layers, references)
- Scale industrial real

**Contras decisivos:**
- C++ heavy con bindings Rust inmaduros (`usd-rs` alpha, sin maintainer activo)
- Stage composition es overkill para MVP de un engine
- Aprendizaje curve enorme para autores/usuarios
- Build pipeline pesado (USD library 200+ MB)

**Decisión:** sobredimensionado para esta fase. **Re-evaluar en 2027+** si:
- USD compose se vuelve relevante para nuestro pipeline planetario (Phase 3)
- Aparece un crate Rust nativo maduro
- Apple/NVIDIA empujan adopción más amplia en game dev

### Collada / DAE (❌ Rechazado)

XML verbose, sin adopción moderna, reemplazado por glTF en todos los workflows. Sin razón para soportarlo.

### STL (❌ Rechazado)

Solo geometría, sin normales suaves ni UVs ni materiales. Útil solo para impresión 3D / CAD. No es game asset format.

### PLY (❌ Rechazado)

Académico (point clouds, scan data). No es game asset format.

### Binario custom (✅ Phase 2 futuro)

**Cuándo introducirlo:**
- Cuando glTF load time sea bottleneck medible (>50ms para asset típico)
- Phase 2.5 voxel + DC pipeline genera meshlet binary directo
- Post-meshlet pipeline (#117) probablemente queremos formato custom anyway

**Contenido propuesto (cuando llegue):**
- Header con magic + version
- Vertex pool (positions + normals + tangents + UVs interleaved o SoA)
- Index buffer
- Meshlet array (post-`meshopt::build_meshlets`)
- Material refs (handles, no embedded)
- Bounds (AABB + bounding sphere)
- Optional: skinning data, blend shapes

---

## Consecuencias

### Inmediatas (Phase 1)

- `ome_core::assets` introduce `Mesh` asset type
- `GltfLoader` y `ObjLoader` impls del trait `AssetLoader<T>` (#391)
- Pipeline: archivo → loader → CPU `Mesh` → upload to GPU buffers
- `MeshRenderer` componente cambia `mesh: String` → `mesh: AssetHandle<Mesh>`
- Tests con assets pequeños en `assets/` o `crates/ome_render/test_assets/`

### Workflow para usuarios del engine

- **Tools recomendadas**: Blender (free) o cualquier DCC con export glTF
- **Para FBX/USD assets**: convertir a glTF antes (Blender es el path)
- **Test fixtures**: OBJ está bien para shapes simples (cube, sphere)

### Documentación

- Esta ADR en `docs/decisions/0001_mesh_format.md`
- Update del README cuando #129 cierre, con sección "Supported asset formats"

### Re-evaluación

Esta decisión puede revisarse cuando:
- Aparezca crate Rust maduro para USD/FBX
- Phase 3 (planetary scale) requiera composición de scene tipo USD layers
- Performance de glTF loading sea bottleneck demostrado con bench

---

## Referencias

- [glTF 2.0 spec](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html)
- [`gltf` crate on crates.io](https://crates.io/crates/gltf)
- [`tobj` crate on crates.io](https://crates.io/crates/tobj)
- Issue #191 (decision record original)
- ADR pattern: Michael Nygard's *Documenting Architecture Decisions*
