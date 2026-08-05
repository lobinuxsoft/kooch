# Order-Independent CSG Composition for SDF Ray-March

**Scope**: design analysis for the order-independent / commutative CSG operator semantics in the SDF ray-march renderer. Outcome is a recommended approach with explicit trade-offs, sized as the foundation of the destructible-terrain + planet-scale rendering roadmap.

**Related issue**: [#211](https://github.com/lobinuxsoft/kooch/issues/211).
**Status**: **complete** — recommendation issued, follow-up implementation issue is [#307](https://github.com/lobinuxsoft/kooch/issues/307).

## Table of contents

- [Problem statement](#problem-statement)
- [Current behaviour](#current-behaviour)
- [Approaches surveyed](#approaches-surveyed)
- [Comparison matrix](#comparison-matrix)
- [Recommendation — postfix RPN tree](#recommendation--postfix-rpn-tree)
- [How `SdfBlend` (smooth blending) maps to RPN](#how-sdfblend-smooth-blending-maps-to-rpn)
- [Roadmap: destructible terrain + planet visualisation](#roadmap-destructible-terrain--planet-visualisation)
- [MVP scope (`#307`)](#mvp-scope-307)
- [Out of MVP / follow-up](#out-of-mvp--follow-up)
- [References](#references)

## Problem statement

The current `eval_scene` shader applies each instance's `blend_mode` directionally against the accumulated field — meaning `(blend_mode, smoothness)` semantics depend on the **order of iteration**, which the user can't predict (it follows ECS archetype order, derived from creation history). Three concrete consequences:

1. **First entity's `blend_mode` is a no-op.** With nothing in the accumulator (`acc = 1e10`), `SmoothSubtraction` of the first entity carves nothing — the field is empty.
2. **Last entity dominates visually.** Its `blend_mode` is the only one applied against a "real" accumulated field.
3. **User intent is broken.** "This entity is subtractive" should hold *regardless of when it was created*. Today it doesn't.

Beyond the immediate bug, this directional semantics blocks every higher-scale feature on the roadmap: destructible terrain (every explosion would behave differently depending on creation history), planet-scale chunked SDFs (chunks need a deterministic CSG model that survives streaming), and runtime edits in general.

## Current behaviour

`crates/kooch_render/shaders/raymarch_main.wgsl::eval_scene` (truncated):

```wgsl
fn eval_scene(p: vec3<f32>) -> f32 {
    var d = 1e10;
    let count = scene_meta.instance_count;
    for (var i = 0u; i < count; i = i + 1u) {
        let inst = instances[i];
        let pd = eval_primitive(transform_point(p, inst.position, inst.rotation) / scale, inst);
        d = apply_blend(d, pd, inst.blend_mode, inst.blend_smoothness);
    }
    return d;
}
```

`apply_blend` dispatches to `sdf_smooth_union` / `sdf_smooth_intersection` / `sdf_smooth_subtraction` per-instance. Each one is a **left-fold** of the current accumulated field with this instance's distance.

The `SdfBlend` component (`crates/kooch_ecs/src/sdf_blend.rs`) carries `mode` and `smoothness` as **per-entity intent**, but the shader interprets them as **per-step operators** in a sequence — that's where the semantic mismatch lives.

## Approaches surveyed

### A — Three-pass union → subtract → intersect (role-based, per-instance smoothness)

Treat each entity as one of three roles (Add / Sub / Intersect) and the scene as three separate passes.

- Order-independent within each pass (commutative ops).
- 3× per-sample cost.
- Throws away the tree structure ECS already gives us via `Parent` / `Children`.
- Cannot express mixed groupings ("this group of shapes blends smooth, but as a group, hard-unions with the rest").
- Migrating to a tree later is a full rewrite — A is a dead-end MVP.

### B — Role-based with global smoothness

Single pass, single global `k`. Cheaper than A but loses per-instance smoothness — a regression vs. today.

### C — CSG tree, naive GPU traversal

Build a real tree, walk it on the GPU per-sample with a manual stack (DFS / BFS). Maximally expressive, but the runtime tree-walk is hard to implement correctly in WGSL: stack overflow risk on deep trees, divergence between rays hitting different parts, complex code path. A poor fit for our GPU-driven idiom.

### **D — Postfix RPN tree (recommended)**

Linearise the CSG tree to a flat array of tokens in **postfix / Reverse Polish Notation** order. CPU walks the tree DFS once and emits tokens; GPU iterates the array once with a small stack.

```
Tree:           union(box, smooth_sub(sphere, cylinder))
DFS leaf-first: box → sphere → cylinder → smooth_sub(2 args) → union(2 args)
RPN tokens:     [box, sphere, cyl, smooth_sub, union]
```

GPU evaluator:

```wgsl
fn eval_scene(p: vec3<f32>) -> f32 {
    var stack: array<f32, MAX_STACK>;  // 16 covers balanced trees up to ~65k leaves
    var sp: u32 = 0u;

    let count = scene_meta.token_count;
    for (var i = 0u; i < count; i = i + 1u) {
        let tok = tokens[i];
        if (tok.kind == TOKEN_LEAF) {
            stack[sp] = eval_primitive(p, tok);
            sp = sp + 1u;
        } else {
            // pop 2, apply op + smoothness, push result
            let b = stack[sp - 1u];
            let a = stack[sp - 2u];
            stack[sp - 2u] = apply_op(a, b, tok.op, tok.smoothness);
            sp = sp - 1u;
        }
    }
    return stack[0u];
}
```

- **Equivalent expressiveness to a real tree** — any CSG topology, mixed hard/smooth, nested groups, per-edge `k`.
- **Flat array** — same SSBO idiom we use today; CPU upload pattern unchanged.
- **No GPU recursion, no dynamic stack control flow** — single linear loop, all wavefronts walk the same sequence (high coherence).
- **Stack is tiny and bounded** — 16 slots covers any balanced tree up to 65k leaves; overflow is a CPU-side validation, never a runtime failure.
- **CSG tree linearised once on the CPU**, when the scene is built / streamed in, not per-sample.

### E — Switch to associative smin variant (exponential / circular geometric)

Quickest "fix" — keep the flat loop, swap polynomial smin for an associative variant. Per the [IQ smin article][iq-smin], exponential and circular geometric smin are associative, which makes the fold order-independent for union.

- Fixes `SmoothUnion` only; `SmoothSubtraction` and `SmoothIntersection` remain broken under any common smin variant. Half-fix at best.
- Useful as a 10-line interim if D ships in stages, but not a destination.

### F — Additive field (Blender meta-objects)

Each entity contributes a positive or negative weight to a scalar field; surface is iso-contour at threshold. Order-independent, but the resulting field is not a true SDF — incompatible with sphere-tracing without major architectural changes (marching cubes / dual contouring). Discarded for the ray-march pass.

## Comparison matrix

| | A — 3-pass roles | B — global k | C — naive tree | **D — postfix RPN** | E — assoc smin | F — additive field |
|---|---|---|---|---|---|---|
| Order-independent (user-facing) | ✅ | ✅ | tree-explicit | **✅** | ✅ union only | ✅ |
| Per-instance smoothness | ✅ | ❌ | ✅ (per edge) | **✅ (per edge)** | ✅ | n/a |
| Mixed hard/smooth in same scene | ❌ | ❌ | ✅ | **✅** | ❌ | ✅ |
| Sphere-trace compatible | ✅ | ✅ | ✅ | **✅** | ✅ | ❌ |
| GPU-driven idiom fit (flat SSBO, no compute) | ✅ | ✅ | ⚠️ stack-walk | **✅** | ✅ | n/a |
| Solves U/S/I (all three) | ✅ | ✅ | ✅ | **✅** | ❌ (50%) | ✅ |
| Path to destructible terrain | ❌ rewrite | ❌ rewrite | partial | **✅ direct** | ❌ rewrite | partial |
| Path to planet-scale streaming | ❌ rewrite | ❌ rewrite | partial | **✅ direct** | ❌ rewrite | partial |
| Estimated cost (this MVP) | 1–2 days | 1 day | 1–2 weeks | **4–7 days** | 0.5 day | huge (rewrite) |

## Recommendation — postfix RPN tree

**Approach D — postfix RPN tree, per-edge operator + smoothness, leaves are SDF primitives.**

### Why D over A / B / C / E

- **A and B are dead-end MVPs.** Both throw away the tree structure that the destructible-terrain and planet-streaming roadmap *requires*. Shipping A means rewriting half of `eval_scene` again the moment we add chunking or runtime CSG edits.
- **C (naive tree-walk) is the right semantics with the wrong implementation.** Per-sample DFS in WGSL needs careful stack discipline; D gets the same semantics with a flat-loop implementation that matches our GPU-driven style.
- **E (associative smin) is half a fix** — it only addresses union, leaving subtraction and intersection direction-dependent. Useful as a 10-line interim if we want to land *something* before D is ready, but not a destination.
- **D's cost is intermediate (4–7 days) and the result is the endgame** — same data structure powers the destructible roadmap, planet chunks, and per-chunk BVH (#115).

### What stays the same

- The `instances` SSBO layout (per-leaf primitive data: position, rotation, scale, primitive type, primitive params).
- The `eval_primitive` shader function — leaves still call it.
- Per-instance smoothness is preserved, just promoted from "instance-level" to "operator-edge-level" (see next section).

### What changes

- `instances` SSBO becomes a **token array** that interleaves leaf tokens (one per primitive) and operator tokens. Old `SdfBlend.mode`/`smoothness` migrates to operator-token attributes.
- The flat fold in `eval_scene` becomes the postfix evaluator above.
- The CPU side gains a tree-builder + tree-to-RPN serialiser; today's "flat list of entities" maps to a degenerate tree (one root operator, all entities as children).

## How `SdfBlend` (smooth blending) maps to RPN

The current `SdfBlend { mode, smoothness }` semantics are preserved end-to-end — they just live on the right object after the migration.

### Before (per-instance)

Each entity carries `SdfBlend`. The shader reads it per-step in the fold:

```rust
SdfBlend { mode: SMOOTH_UNION, smoothness: 0.3 }   // attached to SdfSphere entity
```

### After (per-operator-edge)

Each **operator node** carries the operator kind + smoothness. **Leaves carry no blend metadata** — they're just primitives.

```text
Tree node types:
  Leaf       : { primitive_index, transform, scale }   (no blend metadata)
  Operator   : { kind, smoothness, left_subtree, right_subtree }
                kind ∈ { union, smooth_union,
                         intersection, smooth_intersection,
                         subtraction, smooth_subtraction,
                         hard_union, hard_intersection, hard_subtraction }
```

The smoothness now sits on the **edge between subtrees**, which is where smooth blending mathematically belongs (`smin(A, B, k)` is a binary op with one `k`).

### Migration of existing scenes

A scene migration pass runs once when an old `.kooch_scene` is loaded:

1. Group existing entities by `SdfBlend.mode` into three buckets: ADD-like (`MODE_REPLACE` + `MODE_SMOOTH_UNION`), SUB (`MODE_SMOOTH_SUBTRACTION`), INTERSECT (`MODE_SMOOTH_INTERSECTION`).
2. Build a default tree with this shape:
   ```
   smooth_subtract(
       smooth_intersect(
           smooth_union(...all ADD-like leaves..., k = max of their k),
           ...all INTERSECT leaves...,
           k = max
       ),
       ...all SUB leaves...,
       k = max
   )
   ```
3. Per-instance `smoothness` is dropped onto the operator edges that connect that instance into the tree. Default `k` = max across each role's instances (acceptable approximation; the tree editor can tune later).
4. Old `SdfBlend.mode` / `smoothness` fields are removed from the component schema.

The migrated scene renders identically to what Approach A would have produced — it's literally Approach A as a static tree shape, but now editable and extensible.

### After migration: what the user can do that A couldn't

- Per-edge `k` instead of per-instance — the smoothness lives where it matters.
- **Mixed hard / smooth in the same scene.** A group of blobs smooth-unions internally with `k=0.4`, then hard-unions with the rest of the world (no global blend halo).
- **Nested groups.** "This row of cylinders is one smooth-blob; that row is another; both are subtracted from the planet surface as a single carved-out trench."
- **Drag-reorder in the World panel becomes meaningful** — moving an entity between subtrees changes its CSG role explicitly.

## Roadmap: destructible terrain + planet visualisation

The user-facing goal: render a planet visible from space, traversable to surface, with destructible / terraformable terrain. Five engineering pieces, four already filed:

| Piece | What it does | Status |
|---|---|---|
| **1. CSG tree dynamic (RPN)** | Edit-time and runtime CSG model: every shape, every explosion, every carve is a tree node | This doc → [#307](https://github.com/lobinuxsoft/kooch/issues/307) |
| **2. Spatial chunking** | World divided into chunks (e.g. 32³ m); edits affect only chunks they touch; LOD per chunk by distance | [#54](https://github.com/lobinuxsoft/kooch/issues/54) — already designed for planet-scale |
| **3. BVH per-chunk culling** | Sphere-trace only the chunks the ray actually enters; shared BVH for ray, frustum, and broadphase queries | [#115](https://github.com/lobinuxsoft/kooch/issues/115) — GPU LBVH design done |
| **4. Sparse SDF storage** | Per-chunk: a sparse subgrid voxel field (the "baked" SDF) plus a tail of recent edits | [#136](https://github.com/lobinuxsoft/kooch/issues/136) — sparse 16³ subgrids designed |
| **5. Edit Baker pipeline** | Bridge: when a chunk's RPN delta tree exceeds a threshold, rasterise it into the sparse subgrids and reset the delta to empty | [#309](https://github.com/lobinuxsoft/kooch/issues/309) |

Adjacent / supporting issues that consume the result:

- [#91](https://github.com/lobinuxsoft/kooch/issues/91) **Hybrid Rendering Pipeline (SDF + Mesh)** — overall architecture; SDF pass evaluates the chunked CSG, mesh pass paints rasterised geometry on top, depth-tested against the SDF.
- [#90](https://github.com/lobinuxsoft/kooch/issues/90) **Navigation with terraformation** — NavOctree generated from the SDF resulting from CSG + bake; explicitly assumes the SDF is dynamically modifiable (which D + #136 + Baker provides).
- [#248](https://github.com/lobinuxsoft/kooch/issues/248) **AtmosphereVolume per-planet** — volumetric scattering shell rendered around each planet; "planet visible from space" = atmosphere shell + planet's chunked SDF behind it. Independent of the CSG model; consumes the depth result.
- [#53](https://github.com/lobinuxsoft/kooch/issues/53) **LOD by distance** — per-chunk LOD selection; the sparse subgrid #136 already designs LOD into its storage layer.
- [#117](https://github.com/lobinuxsoft/kooch/issues/117) **Virtual Geometry (Nanite-inspired)** — the mesh side of #91; not directly part of this roadmap but composes via the depth buffer.

### Per-chunk data model (the unifying contract)

Each chunk owns:

```
ChunkData {
    // pieza 4: pre-baked baseline (heavy, slowly-changing data)
    sparse_sdf_baseline:  SparseSubgrid,      // #136 — 16³ subgrids, LOD-aware

    // pieza 1: live edits (light, fast-changing)
    rpn_delta_tree:       Vec<Token>,         // #307 — tokens applied on top of baseline

    // pieza 3: spatial bound for culling
    bvh_aabb:             Aabb,               // #115

    // pieza 2: streaming / LOD state
    lod_level:            u8,                 // #54
    last_baked_at:        Frame,              // pieza 5 — TTL for re-bake decision
}
```

`eval_scene_chunk(p)` evaluates `min(sparse_sdf_baseline.sample(p), eval_rpn(rpn_delta_tree, p))` — baseline is the union floor, deltas can carve or add on top. When `rpn_delta_tree.len() > BAKE_THRESHOLD` or the chunk is unloaded, the Edit Baker (pieza 5) rasterises the delta into the baseline and clears the tree. Cost stays bounded.

### Why this roadmap is coherent

Each piece lives in the same data model (sparse SDF + RPN delta), so the boundaries between issues are clean:

- #307 (RPN) **doesn't need to know about chunks** — it's the operator algebra.
- #54 (chunking) **doesn't need to know about RPN** — chunks own *some* SDF data.
- #136 (sparse storage) **doesn't need to know about edits** — it just allocates / frees subgrids.
- The Edit Baker is the only piece that touches both (#307 tree → #136 subgrids).

Take any piece independently and it has well-defined inputs / outputs; ship them in the order that matches available time and current pain.

## MVP scope (`#307`)

For the implementation issue derived from this doc — minimum to land **just D**, no chunks yet, no baker yet:

- **In-shader changes**:
  - New `Token` SSBO replaces / augments `instances`. Layout: `kind: u32, op: u32, smoothness: f32, primitive_index: u32` (one slot per node, ~16 bytes; primitives still live in their own SSBO indexed by `primitive_index` for leaves).
  - `eval_scene` becomes the postfix evaluator above. Stack size constant `MAX_STACK = 16` — validate at scene-build time.
- **CPU-side**:
  - `crates/kooch_render/src/raymarch/update.rs` builds the CSG tree from the ECS, walks it DFS, emits tokens into the buffer.
  - For the MVP, the tree is built automatically from the existing flat ECS (group by old `SdfBlend.mode`, build the canonical 3-role default tree from the migration section above).
  - `SceneMeta` gains `token_count` (replaces `instance_count`).
- **Editor**:
  - The current Inspector for `SdfBlend.mode` continues to work — it edits the operator edge that connects the entity into its auto-built default tree. No World-panel UX change needed for MVP.
  - "Tree editor" (manual nesting / grouping in the World panel) is **out of scope** for #307 — filed as a separate UX issue when needed.
- **Migration**:
  - Old `.kooch_scene` files load via the migration described above. Render output is byte-identical to A (golden image test).

### Acceptance for `#307`

- [ ] Subtractive entity created **first** still subtracts.
- [ ] Reordering creation history of two entities never changes the rendered output.
- [ ] Per-entity `smoothness` still controls the local blend radius (now via the operator-edge connecting the entity).
- [ ] Sphere-tracing converges on scenes with mixed roles + smooth blends.
- [ ] No measurable perf regression on scenes <50 instances; <2× regression on 100–500 (acceptable; chunking + BVH will compensate later).
- [ ] Old scenes load and render identically post-migration (golden-image diff on the existing test scenes).
- [ ] Stack overflow validated at upload time (refuse to upload trees with depth > 16 with an explicit error).

## Out of MVP / follow-up

Filed as separate issues only when concrete need emerges:

- **Tree editor UX in the World panel** (manual grouping, drag-into-group, tree visualisation). Once the user wants mixed hard/smooth across nested groups.
- **Per-edge smoothness manual override** in Inspector. Today the migration sets it from the entity's old `SdfBlend.smoothness`; later, surface the edge directly.
- **N-ary smin operators** (collapse a chain of pairwise smooth-unions into one N-ary node). Optimisation for fan-in heavy scenes; needs derivation for variants beyond exponential.
- **Spatial partitioning + chunking** — [#54](https://github.com/lobinuxsoft/kooch/issues/54), [#115](https://github.com/lobinuxsoft/kooch/issues/115), [#136](https://github.com/lobinuxsoft/kooch/issues/136), Edit Baker (new issue).
- **Per-edge LOD smoothing** — distant chunks could use cheaper smin variants. Companion to [#53](https://github.com/lobinuxsoft/kooch/issues/53).

## References

- [Inigo Quilez — *Smooth Minimum*][iq-smin]. Canonical reference for smin variants. Key takeaway for this doc: most polynomial smin variants are **not associative**; only **exponential** and **circular geometric** smin are. CD-family smin (what we use) preserves SDF rigidity outside the blend zone but is not associative — so a flat loop fold is order-dependent. RPN evaluation sidesteps the associativity question entirely because each smin is a leaf binary op, never folded across more than its two arguments.
- [Inigo Quilez — *Modeling with Distance Functions*][iq-distfunc]. The classic CSG operators. Hard `min` / `max` are commutative; `max(-a, b)` (subtraction) is **not** commutative across operands. The smooth variants are bounds, not exact SDFs — that's why sphere-tracing under heavy smin needs a Lipschitz floor (the reduction we already do for non-uniform scale).
- [Media Molecule — *Learning from failure: A survey of promising, unconventional and mostly abandoned renderers for 'dreams ps4'*, SIGGRAPH 2015 (Alex Evans)][dreams-talk]. Operationally Transformed CSG trees, baked to point clouds at runtime. Their answer to "scale to millions of edits": tree structure + chunking + bake. Validates D + the chunking + baker roadmap.
- [Beyond3D forum — *Signed Distance Field rendering — pros and cons (as used in PS4 title Dreams)*][dreams-forum]. Community discussion expanding on Evans's talk. Dreams encodes its CSG tree in a custom serialisation, not stored as a runtime tree — they bake to point clouds.
- [Galin et al. — *Synchronized Tracing of Primitive-based Implicit Volumes* (ACM TOG 2024)][galin-paper]. Optimised sphere-tracing over BVH-organised CSG primitives. Companion to [#224](https://github.com/lobinuxsoft/kooch/issues/224); useful when the chunked CSG roadmap (pieces 2 + 3) lands.
- [Blender Manual — *Metaball*][blender-meta]. Order-independent additive field model. **Incompatible with sphere-tracing** because the sum-of-fields is not a true SDF; would require marching cubes / dual contouring. Discarded for our renderer.
- Internal: [#21](https://github.com/lobinuxsoft/kooch/issues/21) (SDF scene structure with ECS, merged, contains the original bug), [#221](https://github.com/lobinuxsoft/kooch/issues/221) (concave intersection seams, fixed), [#225](https://github.com/lobinuxsoft/kooch/issues/225) (over-relaxation normal-discontinuity research, closed), [#127](https://github.com/lobinuxsoft/kooch/issues/127) (SDF pathtracer future), [#40](https://github.com/lobinuxsoft/kooch/issues/40) (SDF collision future — should reuse the same `eval_scene_chunk`).

[iq-smin]: https://iquilezles.org/articles/smin/
[iq-distfunc]: https://iquilezles.org/articles/distfunctions/
[dreams-talk]: https://advances.realtimerendering.com/s2015/
[dreams-forum]: https://forum.beyond3d.com/threads/signed-distance-field-rendering-pros-and-cons-as-used-in-ps4-title-dreams-spawn.57006/
[galin-paper]: https://dl.acm.org/doi/full/10.1145/3702227
[blender-meta]: https://docs.blender.org/manual/en/latest/modeling/metas/index.html
