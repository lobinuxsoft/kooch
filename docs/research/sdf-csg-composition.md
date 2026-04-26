# Order-Independent CSG Composition for SDF Ray-March

**Scope**: design analysis for the order-independent / commutative CSG operator semantics in the SDF ray-march renderer. Outcome is a recommended approach with explicit trade-offs, not implementation.

**Related issue**: [#211](https://github.com/lobinuxsoft/oh_my_engine/issues/211).
**Status**: **complete** — recommendation issued, follow-up implementation issue queued.

## Table of contents

- [Problem statement](#problem-statement)
- [Current behaviour](#current-behaviour)
- [Approaches surveyed](#approaches-surveyed)
- [Comparison matrix](#comparison-matrix)
- [Recommendation](#recommendation)
- [MVP scope](#mvp-scope)
- [Out of MVP / follow-up](#out-of-mvp--follow-up)
- [References](#references)

## Problem statement

The current `eval_scene` shader applies each instance's `blend_mode` directionally against the accumulated field — meaning `(blend_mode, smoothness)` semantics depend on the **order of iteration**, which the user can't predict (it follows ECS archetype order, which depends on creation history). Three concrete consequences:

1. **First entity's `blend_mode` is a no-op.** With nothing in the accumulator (`acc = 1e10`), `SmoothSubtraction` of the first entity carves nothing — the field is empty.
2. **Last entity dominates visually.** Its `blend_mode` is the only one applied against a "real" accumulated field.
3. **User intent is broken.** "This entity is subtractive" should hold *regardless of when it was created*. Today it doesn't.

For a UGC-shaped editor (the user composes scenes interactively, expecting consistent shape semantics), the order-dependent model is wrong. Composition has to be commutative on user-facing semantics.

## Current behaviour

`crates/ome_render/shaders/raymarch_main.wgsl::eval_scene` (truncated):

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

The `SdfBlend` component (`crates/ome_ecs/src/sdf_blend.rs`) carries `mode` and `smoothness` as **per-entity intent**, but the shader interprets them as **per-step operators** in a sequence — that's where the semantic mismatch lives.

## Approaches surveyed

### A — Three-pass union → subtract → intersect (role-based, per-pass smoothness)

Treat each entity as one of three roles (Add / Sub / Intersect) and the scene as three separate passes:

```
pass 1: d = smooth_union over all Add instances     (with smin)
pass 2: d = smooth_subtract over all Sub instances  (carves from pass 1)
pass 3: d = smooth_intersect over all Int instances (clips pass 2)
```

- **Order-independent within each pass** because:
  - `min` (union) is commutative *and associative* (and so is its smooth variant if we pick the exponential or circular smin — see [smin article][iq-smin]).
  - `max(-a, base)` (subtraction) is commutative when interpreted as "all subs carve from the same base" — the order of carves doesn't matter.
  - `max` (intersection) is commutative (and associative for exponential smax).
- **Predictable order between passes** because all three roles have a unified, user-visible order: build, carve, clip.
- **Smoothness is per-instance** within a pass (each entity declares its own `k`). The shader uses pairwise smin with the current pass's accumulator.

Cost: each ray-march sample evaluates every entity 3 times (one pass each).

### B — Role-based field with global smoothness

Same role taxonomy as A, but a single `k` for the whole scene (read from a resource or maxed across instances). One pass instead of three, by interleaving role-specific operators in a fixed order.

- Cheaper (1× eval per sample).
- Less expressive: every smooth blend uses the same `k`. Two adjacent entities can't have different transition radii.

### C — CSG tree (Dreams-style)

Expose a **composition tree** to the user via the existing ECS hierarchy (`Parent` / `Children`):

- Each entity is a CSG node with an operator (Add / Sub / Intersect / SmoothAdd / etc.) against its **subtree** (siblings under the same parent).
- Order of siblings is user-controlled (drag-reorder in the World panel — already exists).
- Smoothness is per-edge (per-node).

Closely matches Media Molecule's Dreams approach: ["Operationally Transformed CSG trees, evaluated on-the-fly to high resolution signed distance fields"][dreams-arch]. Dreams scales to **millions of edits per scene** by grouping into chunks with local order and global SDF compositing — the tree gives them structure without requiring everything be commutative.

- **Maximally expressive.** Mixed hard / smooth operators in the same scene. Per-edge smoothness. Local groupings (smooth blob inside, hard cut outside).
- **Highest UI complexity.** The user has to understand "this child is subtractive against its siblings, which together union into the parent". Order matters but is now explicit and user-controlled (siblings re-orderable in the panel).
- **Implementation cost.** Requires evaluating the tree per sample, which is per-sample recursion or an iterative stack with O(depth) work. For shallow trees (<10 levels) cheap; for arbitrary depth, expensive.

### D — Associative smin only (exponential / circular geometric)

Quickest "fix" — keep the current per-instance flat loop, but switch the smin variant from polynomial (CD-quadratic, our current) to **exponential** or **circular geometric**. Per the [IQ smin article][iq-smin], those two are *associative*, which means the loop fold becomes order-independent for union.

- **Solves the "first entity ignores its blend" problem** for `SmoothUnion` cases.
- **Doesn't solve subtraction or intersection.** They're not associative under any common smin variant; carve-from-empty is still a no-op no matter what smooth variant is used. So D fixes ~50% of the bug only.
- **Cheap to ship.** ~10 lines of WGSL. Useful as an interim while A / C land.

### E — Additive field (Blender meta-objects style)

Each entity contributes a *positive or negative weight* to a scalar field; the surface is the iso-contour at threshold `t`. Negative weights act as "pushers" (subtractors). This is fully order-independent because field addition is commutative.

- The catch: the resulting field is **not a true SDF**. It's an implicit field. You can't sphere-trace it — you need marching cubes / dual contouring / ray-AABB iteration. We sphere-trace, so this approach is incompatible with our renderer architecture without a major rewrite (#127 SDF Pathtracer territory). **Discard for the SDF ray-march pass.**

## Comparison matrix

| | A — Three-pass | B — Roles + global k | C — CSG tree | D — Assoc smin | E — Additive field |
|---|---|---|---|---|---|
| Order-independent (user-facing) | ✅ | ✅ | Tree-explicit (user-ordered) | ✅ for union, ❌ for sub/int | ✅ |
| Per-instance smoothness | ✅ | ❌ | ✅ (per edge) | ✅ | n/a |
| Ray-march compatible | ✅ | ✅ | ✅ | ✅ | ❌ (needs marching cubes) |
| Ship cost (LoC) | medium | low | high | tiny | very high |
| Per-sample cost | 3× | 1× | O(depth) | 1× | 1× field eval |
| Solves all 3 ops (U / S / I) | ✅ | ✅ | ✅ | ❌ (union only) | ✅ |
| Expressive enough for UGC | medium | low | high | low | high |

## Recommendation

**MVP: Approach A** (three-pass, role-based, per-instance smoothness).

**Future: Approach C** (CSG tree) when users need fine-grained composition (mixed hard / smooth blends, nested groups, hierarchical k). Build it on top of A so the data model migrates cleanly: the role field already exists, only the tree-walk replaces the flat loop.

### Why A over the others

- **Solves the bug fully** (union, subtraction, intersection all become commutative on user-facing semantics).
- **Per-instance smoothness preserved** — important for UGC where users tune `k` per shape ("this dome blob has a soft falloff, this surgical cut is sharp").
- **Cheap enough** for our scale. Ray-march cost is dominated by sphere-trace iteration count, not per-step instance count, for sparse scenes. A 3× per-step factor is acceptable for the typical 10–100-instance scene.
- **Doesn't lock us out of C.** The role field becomes the per-node operator in a future tree; the smoothness field stays as-is.

### Why not D as MVP

D fixes only `SmoothUnion` and leaves `SmoothSubtraction` / `SmoothIntersection` broken. Our existing scenes use all three, so it's a partial fix that still surprises users on ~half the cases. Not worth the half-fix.

### Why not C as MVP

C is the right *long-term* answer but it's expensive to implement well (per-sample tree evaluation, per-sample stack discipline, panel UX for reordering, undo for tree edits). Bundle it once we know the role-based system isn't enough — currently we don't have evidence we'll outgrow A.

### Why not B as MVP

A single global `k` is an artist regression vs. today's per-entity `k`. Don't ship a worse-than-current model.

## MVP scope

For the implementation issue derived from this doc:

- **In-shader changes**:
  - `SdfBlend` gains a `role: u32` field (`ADD = 0`, `SUB = 1`, `INTERSECT = 2`). Existing `mode` field maps to `role` via a one-time migration (`MODE_REPLACE` / `MODE_SMOOTH_UNION` → `ADD`; `MODE_SMOOTH_SUBTRACTION` → `SUB`; `MODE_SMOOTH_INTERSECTION` → `INTERSECT`). `smoothness` stays.
  - `eval_scene` becomes three loops over the same instance buffer, each filtering by `role`. The current `instances` SSBO works as-is.
  - `apply_blend` simplified: per-pass smin variant is fixed (smooth union in pass 1, smooth subtract in pass 2, smooth intersect in pass 3). The per-instance `mode` switch goes away.
- **CPU-side**:
  - The collect path in `crates/ome_render/src/raymarch/update.rs` continues to flatten every primitive into the instance buffer. **Sort by role on the CPU** so the GPU passes can read contiguous ranges instead of branching per instance — keeps the inner loop tight.
  - `SceneMeta` gains `add_count`, `sub_count`, `int_count` (the start of each role's range is implicit by sum). Instance buffer layout: `[ ...add..., ...sub..., ...int... ]`.
- **Editor**:
  - Inspector dropdown for `role` (replaces the current `mode` ComboBox) — `ADD` / `SUB` / `INTERSECT`.
  - Migration on scene load: drop the old `mode` integer, write the new `role`. Old scenes get the obvious mapping.

### Acceptance for the implementation PR

- [ ] Subtractive entity created **first** still subtracts.
- [ ] Reordering creation history of two entities never changes the rendered output.
- [ ] Per-entity `smoothness` still controls the local blend radius.
- [ ] Sphere-tracing converges (no Lipschitz violations) on scenes with mixed roles + smooth blends.
- [ ] No measurable perf regression on scenes <50 instances; <2× regression on 100–500 instances (acceptable for the bug fix).
- [ ] Old scenes load and render identically post-migration (golden-image diff on the existing test scenes).

## Out of MVP / follow-up

Filed as separate issues only when concrete need emerges:

- **CSG tree** (Approach C) — when users want hard cuts inside smooth groups or nested compositions.
- **Per-edge smoothness** — same trigger as the tree.
- **Order field** (`SdfOrder { z: i32 }`) — when users need explicit ordering inside a role (rare; YAGNI for now).
- **Spatial partitioning** (BVH over per-instance AABBs) — when scenes routinely exceed ~500 visible SDFs and the 3× cost factor of A becomes the bottleneck. Already half-tracked in #224 (segment tracing with Lipschitz bounds) and #21 (left BVH-on-archetypes pending).
- **N-ary smin** — open question whether replacing pairwise smin in a pass with a true N-ary smin (Inigo Quilez doesn't cover N-ary directly; might require derivation) gives better artist control. Spike before committing.

## References

- [Inigo Quilez — *Smooth Minimum*][iq-smin]. Canonical reference for smin variants. Key takeaway for this doc: most polynomial smin variants are **not associative**; only **exponential** and **circular geometric** smin are. CD-family smin (what we use) preserves SDF rigidity outside the blend zone but is not associative — so a flat loop fold is order-dependent.
- [Inigo Quilez — *Modeling with Distance Functions*][iq-distfunc]. The classic CSG operators. Hard `min` / `max` are commutative; `max(-a, b)` (subtraction) is **not** commutative across operands. The smooth variants are bounds, not exact SDFs — that's why sphere-tracing under heavy smin needs `clamp_distance` reductions like our existing `safe_inv` / Lipschitz floor.
- [Media Molecule — *Learning from failure: A survey of promising, unconventional and mostly abandoned renderers for 'dreams ps4'*, SIGGRAPH 2015 (Alex Evans)][dreams-talk]. Operationally Transformed CSG trees, point-cloud rendering of the resulting SDF. Their answer to "scale to millions of edits": tree structure + chunking, not flat order-independence. Validates Approach C as the long-term endgame, but also explains why our scale (≪ millions) doesn't justify it yet.
- [Beyond3D forum — *Signed Distance Field rendering — pros and cons (as used in PS4 title Dreams)*][dreams-forum]. Community discussion expanding on Evans's talk. Dreams encodes its CSG tree in a custom serialization, not stored as a runtime tree — they bake to point clouds. Different rendering target than ours, but the tree is still the data model.
- [Blender Manual — *Metaball*][blender-meta]. Order-independent additive field model. **Incompatible with sphere-tracing** because the sum-of-fields is not a true SDF; would require marching cubes / dual contouring. Discarded for our renderer (relevant for #127 SDF Pathtracer if that ever uses voxelization).
- Internal: #21 (SDF scene structure with ECS, mergeado), #221 (concave intersection seams, fixed), #225 (over-relaxation normal-discontinuity research, closed), #127 (SDF pathtracer, future), #40 (SDF collision, future — should reuse the same eval_scene), #224 (segment tracing Lipschitz bounds — perf companion).

[iq-smin]: https://iquilezles.org/articles/smin/
[iq-distfunc]: https://iquilezles.org/articles/distfunctions/
[dreams-arch]: https://forum.beyond3d.com/threads/signed-distance-field-rendering-pros-and-cons-as-used-in-ps4-title-dreams-spawn.57006/
[dreams-talk]: https://advances.realtimerendering.com/s2015/
[dreams-forum]: https://forum.beyond3d.com/threads/signed-distance-field-rendering-pros-and-cons-as-used-in-ps4-title-dreams-spawn.57006/
[blender-meta]: https://docs.blender.org/manual/en/latest/modeling/metas/index.html
