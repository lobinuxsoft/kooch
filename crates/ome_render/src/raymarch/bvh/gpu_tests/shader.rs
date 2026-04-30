//! Compute kernel mirroring the production `eval_scene_bvh` fragment
//! shader. Two entry points share the same data layout:
//!
//! - `cs_main` — BVH-driven traversal (the function under test).
//! - `cs_fullscan` — brute-force baseline that walks every primitive
//!   in iteration order; tests diff them to assert the prune is
//!   order-independent (regression for #354).
//!
//! The kernel is duplicated here on purpose: factoring it into a
//! shared file would force the production raymarch module to expose
//! its `SdfPrimitive` / `LeafAabb` / `SceneMeta` structs publicly
//! just to dedupe ~80 lines of WGSL, which is a worse trade than
//! the local copy. The prune semantics here MUST stay in lockstep
//! with `crates/ome_render/shaders/raymarch_main.wgsl::eval_scene_bvh`.

pub(super) const TEST_COMPUTE_WGSL: &str = r#"
struct BvhNode {
    aabb_min: vec3<f32>,
    left: u32,
    aabb_max: vec3<f32>,
    right_or_count: u32,
}
struct LeafAabb {
    aabb_min: vec3<f32>,
    flags: u32,
    aabb_max: vec3<f32>,
    entity_id: u32,
}
struct RaymarchPayload {
    smoothness: f32,
}
struct SdfPrimitive {
    position: vec3<f32>,
    type_tag: u32,
    rotation: vec4<f32>,
    scale: vec3<f32>,
    _pad0: f32,
    params: vec4<f32>,
}
struct SceneMeta {
    primitive_count: u32,
    bvh_n: u32,
    skip_internal_sky: u32,
    has_intersects: u32,
    has_subs: u32,
    k_int_scene: f32,
    k_sub_scene: f32,
    _pad0: u32,
    sky_top: vec4<f32>,
    sky_bottom: vec4<f32>,
}
struct SamplePoint { pos: vec4<f32> }

@group(0) @binding(0) var<uniform>          scene_meta:        SceneMeta;
@group(0) @binding(1) var<storage, read>    primitives:        array<SdfPrimitive>;
@group(0) @binding(2) var<storage, read>    bvh_nodes:         array<BvhNode>;
@group(0) @binding(3) var<storage, read>    sorted_indices:    array<u32>;
@group(0) @binding(4) var<storage, read>    leaf_aabbs:        array<LeafAabb>;
@group(0) @binding(5) var<storage, read>    raymarch_payloads: array<RaymarchPayload>;
@group(0) @binding(6) var<storage, read>    samples:           array<SamplePoint>;
@group(0) @binding(7) var<storage, read_write> out_d:          array<f32>;

const ACC_UNION_IDENTITY: f32 = 1.0e10;
const ACC_INTERSECT_IDENTITY: f32 = -1.0e10;
const BVH_LEAF_FLAG: u32 = 0x80000000u;
const BVH_VALUE_MASK: u32 = 0x7FFFFFFFu;
const ROLE_RAYMARCH_MASK: u32 = 0x3u;
const IS_RAYMARCH: u32 = 1u << 2u;

fn smooth_union(d1: f32, d2: f32, k: f32) -> f32 {
    let h = clamp(0.5 + 0.5 * (d2 - d1) / k, 0.0, 1.0);
    return mix(d2, d1, h) - k * h * (1.0 - h);
}

fn smooth_intersection(d1: f32, d2: f32, k: f32) -> f32 {
    let h = clamp(0.5 - 0.5 * (d2 - d1) / k, 0.0, 1.0);
    return mix(d2, d1, h) + k * h * (1.0 - h);
}

fn smooth_subtraction(d1: f32, d2: f32, k: f32) -> f32 {
    let h = clamp(0.5 - 0.5 * (d1 + d2) / k, 0.0, 1.0);
    return mix(d1, -d2, h) + k * h * (1.0 - h);
}

fn sphere_at(p: vec3<f32>, prim: SdfPrimitive) -> f32 {
    let local = p - prim.position;
    return length(local) - prim.params.x;
}

fn aabb_outside_distance(p: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>) -> f32 {
    let q = max(max(lo - p, p - hi), vec3<f32>(0.0));
    return length(q);
}

fn finalize_result(add_acc: f32, int_acc: f32, sub_acc: f32) -> f32 {
    var result = add_acc;
    if scene_meta.has_intersects != 0u {
        result = smooth_intersection(result, int_acc, max(scene_meta.k_int_scene, 1e-5));
    }
    if scene_meta.has_subs != 0u {
        result = smooth_subtraction(result, sub_acc, max(scene_meta.k_sub_scene, 1e-5));
    }
    return result;
}

fn accumulate_leaf(
    p: vec3<f32>,
    leaf: LeafAabb,
    prim_idx: u32,
    add_acc: ptr<function, f32>,
    int_acc: ptr<function, f32>,
    sub_acc: ptr<function, f32>,
) {
    if (leaf.flags & IS_RAYMARCH) == 0u { return; }
    let prim = primitives[prim_idx];
    let d = sphere_at(p, prim);
    let k = max(raymarch_payloads[prim_idx].smoothness, 1e-5);
    switch (leaf.flags & ROLE_RAYMARCH_MASK) {
        case 0u: { *add_acc = smooth_union(*add_acc, d, k); }
        case 1u: { *int_acc = smooth_intersection(*int_acc, d, k); }
        case 2u: { *sub_acc = smooth_union(*sub_acc, d, k); }
        default: { *add_acc = smooth_union(*add_acc, d, k); }
    }
}

// Mirror of production `eval_scene_bvh`: order-independent prune
// against `max(add_acc, sub_acc)` (post-#354). MUST stay in lockstep
// with the production fragment shader; tests use this kernel to diff
// the BVH walk against the brute-force fullscan and verify the prune
// never drops geometry.
fn eval_scene_bvh(p: vec3<f32>) -> f32 {
    if scene_meta.bvh_n == 0u { return ACC_UNION_IDENTITY; }
    var add_acc = ACC_UNION_IDENTITY;
    var int_acc = ACC_INTERSECT_IDENTITY;
    var sub_acc = ACC_UNION_IDENTITY;
    var stack: array<u32, 32>;
    stack[0] = 0u;
    var sp = 1u;
    while sp > 0u {
        sp = sp - 1u;
        let node = bvh_nodes[stack[sp]];
        let d_aabb = aabb_outside_distance(p, node.aabb_min, node.aabb_max);
        if scene_meta.has_intersects == 0u {
            let union_bound = max(add_acc, sub_acc);
            if d_aabb > union_bound { continue; }
        }
        let payload = node.right_or_count;
        if (payload & BVH_LEAF_FLAG) != 0u {
            let count = payload & BVH_VALUE_MASK;
            let first = node.left;
            for (var i: u32 = 0u; i < count; i = i + 1u) {
                let prim_idx = sorted_indices[first + i];
                let leaf = leaf_aabbs[prim_idx];
                accumulate_leaf(p, leaf, prim_idx, &add_acc, &int_acc, &sub_acc);
            }
        } else {
            let left = node.left;
            let right = payload & BVH_VALUE_MASK;
            if sp + 2u <= 32u {
                stack[sp] = left; sp = sp + 1u;
                stack[sp] = right; sp = sp + 1u;
            }
        }
    }
    return finalize_result(add_acc, int_acc, sub_acc);
}

// Brute-force baseline: walk every primitive in iteration order,
// accumulate per role with the same operators as the BVH path, and
// finalize with the same outer combination. The result MUST match the
// BVH walk byte-for-byte at any sample point — the morton-sorted
// index permutation is the only thing that differs, and smooth_union
// is order-tolerant within float-rounding ULPs as long as every leaf
// is visited (the regression in #354 came from skipping leaves, not
// from reordering them). Tests assert per-sample agreement within a
// generous epsilon to cover the residual non-associativity.
fn eval_scene_fullscan(p: vec3<f32>) -> f32 {
    var add_acc = ACC_UNION_IDENTITY;
    var int_acc = ACC_INTERSECT_IDENTITY;
    var sub_acc = ACC_UNION_IDENTITY;
    let n = scene_meta.bvh_n;
    for (var i: u32 = 0u; i < n; i = i + 1u) {
        let leaf = leaf_aabbs[i];
        accumulate_leaf(p, leaf, i, &add_acc, &int_acc, &sub_acc);
    }
    return finalize_result(add_acc, int_acc, sub_acc);
}

@compute @workgroup_size(64)
fn cs_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if i >= arrayLength(&samples) { return; }
    out_d[i] = eval_scene_bvh(samples[i].pos.xyz);
}

@compute @workgroup_size(64)
fn cs_fullscan(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if i >= arrayLength(&samples) { return; }
    out_d[i] = eval_scene_fullscan(samples[i].pos.xyz);
}
"#;
