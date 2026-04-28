//! Compute kernel mirroring the production `eval_scene_bvh` fragment
//! shader. Two entry points share the same data layout:
//!
//! - `cs_main` — BVH-driven traversal (the function under test).
//! - `cs_fullscan` — brute-force baseline that walks every primitive
//!   in iteration order; the Lipschitz test diffs them.
//!
//! The kernel is duplicated here on purpose: factoring it into a
//! shared file would force the production raymarch module to expose
//! its `SdfPrimitive` / `LeafAabb` / `SceneMeta` structs publicly
//! just to dedupe ~80 lines of WGSL, which is a worse trade than
//! the local copy.

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

fn smooth_union(d1: f32, d2: f32, k: f32) -> f32 {
    let h = clamp(0.5 + 0.5 * (d2 - d1) / k, 0.0, 1.0);
    return mix(d2, d1, h) - k * h * (1.0 - h);
}

fn sphere_at(p: vec3<f32>, prim: SdfPrimitive) -> f32 {
    let local = p - prim.position;
    return length(local) - prim.params.x;
}

fn point_in_aabb(p: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>) -> bool {
    return all(p >= lo) && all(p <= hi);
}

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
        if !point_in_aabb(p, node.aabb_min, node.aabb_max) { continue; }
        let payload = node.right_or_count;
        if (payload & BVH_LEAF_FLAG) != 0u {
            let count = payload & BVH_VALUE_MASK;
            let first = node.left;
            for (var i: u32 = 0u; i < count; i = i + 1u) {
                let prim_idx = sorted_indices[first + i];
                let d = sphere_at(p, primitives[prim_idx]);
                let k = max(raymarch_payloads[prim_idx].smoothness, 1e-5);
                add_acc = smooth_union(add_acc, d, k);
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
    return add_acc;
}

// Brute-force baseline: walk every primitive in iteration order,
// accumulate via the same smooth_union as the BVH path. The byte
// difference between this and `eval_scene_bvh` at any point inside
// at least one primitive's AABB is bounded by the per-role smooth-
// blend k_max plus a few float-rounding ULPs (proof: smooth_union
// with k → 0 collapses to plain min, which IS associative; smooth
// blends with k > 0 decay exponentially past their support radius
// so distant primitives' contribution to a near-AABB point is at
// most ~k).
fn eval_scene_fullscan(p: vec3<f32>) -> f32 {
    var acc = ACC_UNION_IDENTITY;
    let n = scene_meta.bvh_n;
    for (var i: u32 = 0u; i < n; i = i + 1u) {
        let d = sphere_at(p, primitives[i]);
        let k = max(raymarch_payloads[i].smoothness, 1e-5);
        acc = smooth_union(acc, d, k);
    }
    return acc;
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
