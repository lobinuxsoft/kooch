# BVH-Driven Ray Marching

This chapter documents how the SDF ray-marcher integrates with the
GPU LBVH builder shipped in `ome_bvh` (issue #115 PR-3) to skip
evaluating primitives whose AABB does not contain the current
sample point. The integration is the subject of #115 PR-4.

## Why a BVH at all

Without spatial culling, `eval_scene(p)` had to evaluate every SDF
primitive at every sphere-tracing step. A planet-scale scene with
~1 M primitives would burn ~1 M `transform_point + sdf_*` calls
per ray *per step*, and a ray takes up to 256 steps. The
arithmetic is unforgiving: even at 50 ns per primitive eval the
shader would not converge inside a frame budget.

A BVH lets each ray query "which primitives are near `p`" in
`O(log N)` and limits the per-step work to exactly the leaves the
ray currently overlaps.

## Data flow

```text
  ┌─────────────┐    ┌───────────────┐    ┌────────────────────┐
  │   ECS       │ ─► │ update_scene  │ ─► │ BvhState (S4)      │
  │  (SDF       │    │ (Vec<Aabb>,   │    │  ├─ BvhGpuBuilder  │
  │   compos)   │    │  Vec<LeafA…>) │    │  ├─ slot_a / slot_b│
  └─────────────┘    └───────────────┘    │  └─ pending build  │
                                          └─────────┬──────────┘
                                                    │ kick_if_dirty
                                                    ▼
                                          ┌────────────────────┐
                                          │ Bvh::build_gpu     │
                                          │ (PR-3 — Morton +   │
                                          │  onesweep + Karras)│
                                          └─────────┬──────────┘
                                                    │ poll_swap
                                                    ▼
                              ┌──────────────────────────────────────┐
                              │ slot[current_slot]: nodes + indices  │
                              │   + leaf_aabbs (stable, not pending) │
                              └─────────┬────────────────────────────┘
                                        │ bind to fragment shader
                                        ▼
                              ┌──────────────────────────────────────┐
                              │ raymarch_main.wgsl::eval_scene_bvh   │
                              │  per-step stack walk, per-role acc   │
                              └──────────────────────────────────────┘
```

The two-slot pattern is the answer to the read-after-write hazard
on a single shared GPU buffer. While the renderer reads
`slot_a.nodes_buffer` for frame N, a new build can write into
`slot_b` for frame N+1 — the swap happens at `poll_swap` after the
build's submission resolves. wgpu would otherwise insert a
synchronisation barrier to serialise the read against the write,
stalling the frame pipeline.

## Traversal-driven CSG composition

Earlier drafts of PR-4 considered building a per-ray *hit list* of
primitive indices and then iterating the existing postfix CSG token
stream with a "skip if not in hit list" check. The parallel auditor
flagged this as marketing: it still iterates `O(N)` tokens per
sample, and an `array<u32, 256>` thread-local hit list spills 1 KiB
per ray into private memory — register-file death on RDNA 2 / 4.

The shipped design replaces the postfix token stream entirely. The
BVH traversal *is* the evaluation loop. Each leaf carries its CSG
role (ADD / INTERSECT / SUBTRACT) and per-instance smoothness, and
the traversal accumulates per-role distances inline:

```wgsl
fn eval_scene_bvh(p: vec3<f32>) -> f32 {
    var add_acc = ACC_UNION_IDENTITY;     // +1e10
    var int_acc = ACC_INTERSECT_IDENTITY; // -1e10
    var sub_acc = ACC_UNION_IDENTITY;     // +1e10
    // ... stack walk, point-in-aabb cull, per-leaf eval + combine ...
    var result = add_acc;
    if scene_meta.has_intersects != 0u {
        result = sdf_smooth_intersection(result, int_acc, scene_meta.k_int_scene);
    }
    if scene_meta.has_subs != 0u {
        result = sdf_smooth_subtraction(result, sub_acc, scene_meta.k_sub_scene);
    }
    return result;
}
```

The "default tree" shape (`smooth_subtract(smooth_intersect(adds,
ints), subs)`) is preserved — it is now expressed structurally by
the per-role accumulators + the fixed final combination. Per-role
`k_max` lives in `SceneMeta`; per-instance `smoothness` lives in
each `LeafAabb`.

### Identity elements

| Role      | Combinator           | Identity value       | Why                                |
|-----------|----------------------|----------------------|------------------------------------|
| ADD       | `smooth_union`       | `+∞` (`1e10`)        | `smooth_union(+inf, x, k) ≈ x`     |
| INTERSECT | `smooth_intersection`| `-∞` (`-1e10`)       | `smooth_intersection(-inf, x, k) ≈ x` |
| SUBTRACT  | `smooth_union`       | `+∞` (`1e10`)        | subs are unioned, then subtracted  |

Picking `1e10` (rather than `f32::INFINITY`) is deliberate — keeps
the smooth-blend math NaN-free under all inputs.

## Determinism

`smooth_union` and `smooth_intersection` are **not strictly
associative** in float32. The cull-vs-cull byte-identity regression
test (`cull_vs_cull_byte_identical_n_*`) requires the per-role
accumulator visit order to be a function of BVH topology only —
never of runtime ray geometry.

The traversal pushes `left` BEFORE `right` on its 32-deep stack, so
pop order is right-first and stable across frames. **Do not switch
to a t-near-sorted children push** without re-deriving the
determinism story from scratch — the regression test will catch it,
but the failure mode is "single-pixel-bit mismatches that confuse
post-processing", not a clean panic.

## AABB inflation

A primitive's AABB is computed from its analytic shape, scaled by
the entity's `scale`, rotated, translated, and **inflated by its
role's `k_max`**. Smooth blends extend the support beyond the raw
geometry; without the inflation, a primitive whose surface lies
exactly on its AABB would have its smooth-union tail truncated at
the cull boundary.

Per-role inflation (rather than per-instance) keeps the AABB tight:
a primitive in a scene where every other ADD has `k = 0.1`
inflates by `0.1` even if its own `smoothness = 0`, because the
*operator* between them carries the larger `k`.

## Performance scaling

Measured on a Ryzen 9 9800X3D + RX 9070 XT, Bazzite F43 / Mesa
`radv` (#115 PR-4 S11 bench output):

| N primitives | BVH (`cs_main`) | Fullscan (`cs_fullscan`) | Speedup |
|--------------|----------------:|-------------------------:|--------:|
| 1 024        | 3.79 ms         | 3.55 ms                  | 0.94×   |
| 10 240       | 5.93 ms         | 6.88 ms                  | 1.16×   |
| 65 000       | 4.57 ms         | 14.56 ms                 | 3.19×   |

Small-N is dominated by stack-walk overhead. The cross-over sits
between 1 k and 10 k in this configuration; from 65 k upwards the
cull is the dominant cost saver, exactly as the
`O(N) → O(log N)` goal predicts.

## What lives where

- `crates/ome_bvh/` — the GPU LBVH builder (PR-3). Public API:
  `Bvh::build_gpu`, `BvhGpuBuilder`, `BvhGpuBuild`, `GpuBvhHandle`.
- `crates/ome_render/src/raymarch/bvh.rs` — `BvhState`: double-
  buffered slots + dirty hash + kick / poll_swap lifecycle.
- `crates/ome_render/src/raymarch/aabb.rs` — `primitive_aabb`:
  per-type local half-extents → world-space inflated AABB.
- `crates/ome_render/src/raymarch/instance.rs` — `LeafAabb` (32 B
  std430), `SceneMeta` (64 B uniform), CSG role constants.
- `crates/ome_render/shaders/raymarch_main.wgsl` —
  `eval_scene_bvh` traversal + per-role accumulators + fixed final
  combination.

## Out of scope (filed as follow-ups)

- **Refit BVH** (#115 checkbox 110) — incremental updates without
  a full rebuild. Useful for scenes with constant primitive count
  and small per-frame motion.
- **OBB-exact AABBs** (vs the current `abs(rot_matrix) ·
  half_extents` enclosing OBB-AABB). Tighter cull at the cost of
  per-frame CPU work.
- **Workgroup-shared bitmap cull** (vs the current per-thread
  stack walk) — blocked on tile-based shading, which itself is
  blocked on the G-Buffer (#132).
- **Archetype-level dirty marker** (vs the current `u64` hash of
  primitive bytes + leaf metadata).
