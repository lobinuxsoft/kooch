// raymarch_pool_eval.wgsl — TLAS+BLAS pool-driven scene SDF evaluator.
//
// PR-1 of #360. Concatenated at runtime AFTER `sdf_primitives.wgsl` so
// `sdf_*`, `transform_point`, and the smooth-CSG helpers are already in
// scope.
//
// Ships standalone in PR-1: the only entry point exposed here is the
// compute-kernel smoke test `cs_eval_smoke` that drives `eval_scene_bvh`
// over a caller-provided sample-point buffer and writes the resulting
// SDF distances to an output buffer. PR-2 wires `eval_scene_bvh` into
// the raymarcher's fragment path, replaces `cs_eval_smoke`'s I/O
// bindings with the camera + scene-meta uniforms, and drops the
// legacy global-BVH `eval_scene_bvh` from `raymarch_main.wgsl`.
//
// # Per-role accumulator invariant
//
// `acc_add` / `acc_sub` / `acc_int` are initialised once at the entry
// to `eval_scene_bvh` and survive the entire TLAS descend. They are
// **never** reset per-chunk — primitives that smooth-blend across
// chunk boundaries compose under the scene-wide accumulator without
// losing information at the seam. The final per-role combine
// (`smooth_subtract(smooth_intersect(acc_add, acc_int, k_int), acc_sub,
// k_sub)`) runs once at TLAS exit using `tlas_uniforms.k_*_global` —
// scene-wide maxima reduced CPU-side over the visible chunk set, never
// per-leaf and never per-chunk.
//
// # Determinism
//
// TLAS and BLAS stacks both push left-before-right, pop right-first,
// matching the convention pinned by the legacy `eval_scene_bvh` in
// `raymarch_main.wgsl`. Across two scenes inserted in any chunk order
// onto the same `OmeAccel`, the TLAS rebuild lays out leaves in the
// same morton order, so traversal hits BLAS leaves in the same order
// and the float-imprecise smooth_union accumulates identical bits.

// =============================================================================
// STRUCTS — must match the Rust side byte-for-byte (offset_of! pinned).
// =============================================================================

// 64 bytes. Mirrors `ome_render::raymarch::instance::SdfPrimitive`. PR-2
// repurposes the legacy `_pad0` slot as `smoothness: f32` so the
// raymarch path no longer needs a parallel `RaymarchPayload` binding.
// PR-1 declares the field here (the shader is standalone) so the
// compute-kernel smoke test can populate it; the Rust struct keeps
// `_pad0` unchanged this PR — the smoke test just writes its `f32` into
// the same byte slot.
struct SdfPrimitive {
    position: vec3<f32>,
    type_tag: u32,
    rotation: vec4<f32>,
    scale: vec3<f32>,
    smoothness: f32,
    params: vec4<f32>,
}

// 32 bytes. Mirrors `ome_bvh::BvhNode`. `right_or_count`'s top bit is
// the leaf flag; for TLAS leaves bit 30 additionally carries the
// dead-skip flag (`TLAS_DEAD_FLAG`).
struct BvhNode {
    aabb_min: vec3<f32>,
    left: u32,
    aabb_max: vec3<f32>,
    right_or_count: u32,
}

// 32 bytes. Mirrors `ome_bvh::LeafAabb`. Multi-consumer per-primitive
// metadata: AABB + bit-packed flags + entity_id. Indexed by the
// **absolute** pool primitive index (the value of `node.left` after the
// post-pass enforced in `streaming::insert_chunk` / `refit_chunk`).
struct LeafAabb {
    aabb_min: vec3<f32>,
    flags: u32,
    aabb_max: vec3<f32>,
    entity_id: u32,
}

// 64 bytes. Mirrors `ome_bvh::accel::ChunkDescriptor`. `aabb_min` /
// `aabb_max` are world-space bounds **already inflated** by
// `max_smoothness_radius` so the TLAS prune stays conservative under
// cross-chunk smooth-blend bleed.
struct ChunkDescriptor {
    aabb_min: vec3<f32>,
    first_node: u32,
    aabb_max: vec3<f32>,
    node_count: u32,
    first_leaf: u32,
    leaf_count: u32,
    first_primitive: u32,
    primitive_count: u32,
    max_smoothness_radius: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

// 16 bytes. Mirrors `ome_bvh::accel::TlasUniforms`. Reduced CPU-side
// once per frame over the visible chunk set.
struct TlasUniforms {
    k_int_global: f32,
    k_sub_global: f32,
    num_chunks: u32,
    _pad0: u32,
}

// =============================================================================
// CONSTANTS
// =============================================================================

// Multi-consumer leaf-flag scheme — must match the `IS_*` /
// `ROLE_RAYMARCH_*` constants on the Rust side byte-for-byte.
const ROLE_RAYMARCH_MASK: u32 = 0x3u;
const ROLE_RAYMARCH_ADD: u32 = 0x0u;
const ROLE_RAYMARCH_INT: u32 = 0x1u;
const ROLE_RAYMARCH_SUB: u32 = 0x2u;
const IS_RAYMARCH: u32 = 1u << 2u;

// BVH topology bits. `BVH_LEAF_FLAG` is the high bit (set on every
// leaf, BLAS or TLAS). `TLAS_DEAD_FLAG` is bit 30 — set on TLAS leaves
// whose chunk has been evicted but not yet compacted out of the
// topology. `TLAS_CHUNK_IDX_MASK` is the low 30 bits of a TLAS leaf's
// `right_or_count`.
const BVH_LEAF_FLAG: u32 = 0x80000000u;
const BVH_VALUE_MASK: u32 = 0x7FFFFFFFu;
const TLAS_DEAD_FLAG: u32 = 0x40000000u;
const TLAS_CHUNK_IDX_MASK: u32 = 0x3FFFFFFFu;

// Per-role accumulator identities. Picked so an empty role collapses
// cleanly under the final per-role combine — see the note at the top
// of `eval_scene_bvh`.
//
// `±1e6` instead of `±1e10` because some Vulkan drivers (radv on the
// development machines) implement `mix(a, b, t)` as `a + (b - a) * t`,
// which loses precision at extreme magnitudes: with `a = -1e10` and
// `b = -77` and `t = 1`, the f32 round-off in `(b - a)` swallows `b`
// and the result evaluates to `0` instead of `-77`. `1e6` keeps the
// identity well past any practical SDF distance (≥ 1000 km in scene
// units) while staying inside f32 precision so the identity collapse
// in the final per-role combine works on every backend. The legacy
// `raymarch_main.wgsl` carried `1e10` and avoided the bug only via
// conditional `has_intersects` / `has_subs` branches; we drop those
// branches here so the math is unconditional and uniform across
// scenes — and this constant is the lever that makes that safe.
const ACC_UNION_IDENTITY: f32 = 1.0e6;
const ACC_INTERSECT_IDENTITY: f32 = -1.0e6;

// Stack depths. Both fit comfortably in registers on Steam Deck-class
// hardware. `MAX_TLAS_STACK = 32` covers `2^32` TLAS leaves (encoding
// caps at `2^30`); `MAX_BLAS_STACK = 32` does the same for any single
// chunk's BLAS.
const MAX_TLAS_STACK: u32 = 32u;
const MAX_BLAS_STACK: u32 = 32u;

// =============================================================================
// BIND GROUPS
//
// Group 0 — smoke-test I/O. Replaced in PR-2 by the camera + scene-meta
// uniforms when this shader is wired into the raymarch fragment path.
// Group 1 — pool layout (issue body §Shader/GPU). Bindings 5..=10
// match the issue spec verbatim.
// =============================================================================

@group(0) @binding(0) var<storage, read> sample_points: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> sample_distances: array<f32>;

@group(1) @binding(5) var<storage, read> tlas_nodes: array<BvhNode>;
@group(1) @binding(6) var<storage, read> chunk_descriptors: array<ChunkDescriptor>;
@group(1) @binding(7) var<storage, read> bvh_nodes_pool: array<BvhNode>;
@group(1) @binding(8) var<storage, read> leaf_aabbs_pool: array<LeafAabb>;
@group(1) @binding(9) var<storage, read> primitives_pool: array<SdfPrimitive>;
@group(1) @binding(10) var<uniform> tlas_uniforms: TlasUniforms;

// =============================================================================
// PRIMITIVE EVAL
// =============================================================================

// Local-space distance for primitive `prim` at local-space point
// `local`. Mirrors `raymarch_main.wgsl::eval_primitive_kind` 1:1.
fn eval_primitive_kind(local: vec3<f32>, prim: SdfPrimitive) -> f32 {
    switch prim.type_tag {
        case 0u: { return sdf_sphere(local, prim.params.x); }
        case 1u: { return sdf_rounded_box(local, prim.params.xyz, prim.params.w); }
        case 2u: { return sdf_capsule_y(local, prim.params.x, prim.params.y); }
        case 3u: { return sdf_capped_cylinder(local, prim.params.x, prim.params.y); }
        case 4u: { return sdf_torus(local, prim.params.x, prim.params.y); }
        case 5u: { return sdf_plane_y(local); }
        default: { return 1e10; }
    }
}

// World-space distance with the Lipschitz workaround for non-uniform
// scale (#225) — multiply by `s_min` so sphere tracing stays sound.
fn eval_primitive_at(p: vec3<f32>, prim: SdfPrimitive) -> f32 {
    let scale = max(prim.scale, vec3<f32>(1e-5));
    let local = transform_point(p, prim.position, prim.rotation) / scale;
    let s_min = min(scale.x, min(scale.y, scale.z));
    return eval_primitive_kind(local, prim) * s_min;
}

// =============================================================================
// AABB / TRAVERSAL
// =============================================================================

// `true` when `p` is inside (or on the boundary of) the AABB. TLAS and
// BLAS culling are point-queries: an AABB miss prunes the subtree.
fn aabb_contains(lo: vec3<f32>, hi: vec3<f32>, p: vec3<f32>) -> bool {
    let inside = (p.x >= lo.x) && (p.y >= lo.y) && (p.z >= lo.z)
              && (p.x <= hi.x) && (p.y <= hi.y) && (p.z <= hi.z);
    return inside;
}

// BLAS descend over `[desc.first_node .. desc.first_node + desc.node_count)`.
//
// Returns the updated `(acc_add, acc_sub, acc_int)` triple as a
// `vec3<f32>` rather than taking pointer args. Pass-by-value + return
// is the most portable WGSL pattern across backends; the caller
// threads the previous triple into the next BLAS descend so the
// per-role accumulators remain scene-wide — they are NOT reset
// per-chunk, per the cross-chunk-smoothness invariant pinned at the
// file head.
//
// Determinism: pushes left before right, pops right first, matching
// `eval_scene_bvh`'s TLAS path and the legacy global BVH traversal so
// AC1 byte-identical compares cleanly across the migration.
fn descend_blas(
    p: vec3<f32>,
    desc: ChunkDescriptor,
    acc_add_in: f32,
    acc_sub_in: f32,
    acc_int_in: f32,
) -> vec3<f32> {
    var acc_add = acc_add_in;
    var acc_sub = acc_sub_in;
    var acc_int = acc_int_in;
    var stack: array<u32, 32>;
    stack[0] = desc.first_node;
    var sp: u32 = 1u;

    while sp > 0u {
        sp = sp - 1u;
        let node_idx = stack[sp];
        let node = bvh_nodes_pool[node_idx];
        let inside = (p.x >= node.aabb_min.x) && (p.y >= node.aabb_min.y) && (p.z >= node.aabb_min.z)
                  && (p.x <= node.aabb_max.x) && (p.y <= node.aabb_max.y) && (p.z <= node.aabb_max.z);
        if !inside { continue; }
        let payload = node.right_or_count;
        if (payload & BVH_LEAF_FLAG) != 0u {
            // BLAS leaf — `node.left` carries the **absolute** pool
            // primitive index (WGSL contract: see
            // `ome_bvh::accel::streaming` doc + `contract_tests`).
            // No offset fixup required.
            let prim_idx = node.left;
            let leaf = leaf_aabbs_pool[prim_idx];
            if (leaf.flags & IS_RAYMARCH) == 0u { continue; }
            let prim = primitives_pool[prim_idx];
            let d = eval_primitive_at(p, prim);
            let k = max(prim.smoothness, 1e-5);
            let role = leaf.flags & ROLE_RAYMARCH_MASK;
            if role == ROLE_RAYMARCH_INT {
                acc_int = sdf_smooth_intersection(acc_int, d, k);
            } else if role == ROLE_RAYMARCH_SUB {
                acc_sub = sdf_smooth_union(acc_sub, d, k);
            } else {
                acc_add = sdf_smooth_union(acc_add, d, k);
            }
        } else {
            let left = node.left;
            let right = payload & BVH_VALUE_MASK;
            if sp + 2u <= 32u {
                stack[sp] = left;
                sp = sp + 1u;
                stack[sp] = right;
                sp = sp + 1u;
            }
        }
    }
    return vec3<f32>(acc_add, acc_sub, acc_int);
}

// =============================================================================
// SCENE EVAL
// =============================================================================

// TLAS+BLAS pool-driven scene SDF.
//
// Per-role accumulators initialised ONCE at the entry. Survive every
// BLAS hit across every TLAS leaf so cross-chunk smooth blends compose
// under the same `acc_*` without per-chunk reset (#360 architecture
// note 2). Final combine fixed:
//
//     smooth_subtract(smooth_intersect(acc_add, acc_int, k_int), acc_sub, k_sub)
//
// Empty roles collapse cleanly via their identity element (see the
// `ACC_*_IDENTITY` constants), so the conditional branches the legacy
// shader carried for `has_intersects` / `has_subs` are unnecessary
// here — the math reduces to `acc_add` exactly when the int and sub
// roles are empty.
fn eval_scene_bvh(p: vec3<f32>) -> f32 {
    if tlas_uniforms.num_chunks == 0u {
        return ACC_UNION_IDENTITY;
    }

    var acc_add: f32 = ACC_UNION_IDENTITY;
    var acc_int: f32 = ACC_INTERSECT_IDENTITY;
    var acc_sub: f32 = ACC_UNION_IDENTITY;

    var tlas_stack: array<u32, MAX_TLAS_STACK>;
    tlas_stack[0] = 0u;
    var sp: u32 = 1u;

    while sp > 0u {
        sp = sp - 1u;
        let node = tlas_nodes[tlas_stack[sp]];
        if !aabb_contains(node.aabb_min, node.aabb_max, p) { continue; }
        let payload = node.right_or_count;
        if (payload & BVH_LEAF_FLAG) != 0u {
            // TLAS leaf — `right_or_count` is `chunk_idx | LEAF_FLAG`,
            // optionally with `TLAS_DEAD_FLAG` set when the chunk has
            // been evicted but the lazy compactor has not yet rebuilt
            // the topology.
            if (payload & TLAS_DEAD_FLAG) != 0u { continue; }
            let chunk_idx = payload & TLAS_CHUNK_IDX_MASK;
            let desc = chunk_descriptors[chunk_idx];
            let acc_next = descend_blas(p, desc, acc_add, acc_sub, acc_int);
            acc_add = acc_next.x;
            acc_sub = acc_next.y;
            acc_int = acc_next.z;
        } else {
            let left = node.left;
            let right = payload & BVH_VALUE_MASK;
            if sp + 2u <= MAX_TLAS_STACK {
                tlas_stack[sp] = left;
                sp = sp + 1u;
                tlas_stack[sp] = right;
                sp = sp + 1u;
            }
        }
    }

    let k_int = max(tlas_uniforms.k_int_global, 1e-5);
    let k_sub = max(tlas_uniforms.k_sub_global, 1e-5);
    var result = sdf_smooth_intersection(acc_add, acc_int, k_int);
    result = sdf_smooth_subtraction(result, acc_sub, k_sub);
    return result;
}

// =============================================================================
// SMOKE-TEST ENTRY POINT
//
// Drives `eval_scene_bvh` over `sample_points` and writes the result
// into `sample_distances`. Lives only in PR-1 — PR-2 deletes this
// entry point and wires `eval_scene_bvh` into the raymarch fragment
// shader path.
// =============================================================================

@compute @workgroup_size(64)
fn cs_eval_smoke(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    let n = arrayLength(&sample_points);
    if i >= n { return; }
    let p = sample_points[i].xyz;
    sample_distances[i] = eval_scene_bvh(p);
}
