// raymarch_main.wgsl — ray-march fragment shader.
//
// Concatenated at runtime AFTER `sdf_primitives.wgsl` from ome_sdf, so
// the `sdf_*`, `transform_point`, and CSG helpers are already in scope.
//
// PR-4 of #115: scene composition is BVH-driven. The shader does NOT
// iterate a postfix CSG token stream any more — the BVH traversal IS
// the evaluation loop. Each leaf knows its CSG role; the traversal
// accumulates per-role distances; the final scene SDF is a fixed
// 3-operator combination (smooth_subtract(smooth_intersect(adds, ints),
// subs)). Primitives whose AABB does not contain `p` are pruned by the
// stack-based traversal — never evaluated, never paid for.
//
// DETERMINISM: the traversal pushes `left` BEFORE `right` on the stack
// (so pop order is right-first), and that push order is FIXED across
// frames. `smooth_union` and `smooth_intersection` are NOT strictly
// associative in float32 — `smooth_union(smooth_union(a, b), c)` may
// differ in the last bit from `smooth_union(a, smooth_union(b, c))`.
// Cross-frame byte-identity (the cull-vs-cull regression test in PR-4
// subtask S9) requires the accumulator order to never depend on
// runtime ray geometry. Do not switch to a t-near-sorted children
// push without re-deriving the determinism story.

struct CameraUniforms {
    view: mat4x4<f32>,
    projection: mat4x4<f32>,
    inverse_view: mat4x4<f32>,
    inverse_projection: mat4x4<f32>,
    position: vec3<f32>,
    _pad0: f32,
}

struct RayMarchParams {
    max_steps: u32,
    max_distance: f32,
    // Hit threshold at distance zero (close-up precision floor).
    surface_threshold: f32,
    // Adds `epsilon_factor * distance_traveled` to the threshold so far
    // surfaces don't shimmer and don't waste iterations on sub-pixel
    // precision the viewer can't see anyway.
    epsilon_factor: f32,
}

// Matches Rust `SdfPrimitive` byte-for-byte (64 bytes).
// Field interpretation by type_tag:
//   0 Sphere   — params.x = radius
//   1 Box      — params.xyz = half_extents, params.w = rounding
//   2 Capsule  — params.x = half_height, params.y = radius
//   3 Cylinder — params.x = half_height, params.y = radius
//   4 Torus    — params.x = major_radius, params.y = minor_radius
//   5 Plane    — no params (normal = local Y+ via rotation)
struct SdfPrimitive {
    position: vec3<f32>,
    type_tag: u32,
    rotation: vec4<f32>,
    scale: vec3<f32>,
    _pad0: f32,
    params: vec4<f32>,
}

// Matches Rust `BvhNode` byte-for-byte (32 bytes, std430-clean).
// `right_or_count`'s high bit (`0x80000000`) is the leaf flag:
//   - clear → internal: `left` + `right_or_count` are child indices.
//   - set   → leaf: `left` = first leaf-payload idx, `right_or_count &
//     0x7FFFFFFF` = count of contiguous payloads.
struct BvhNode {
    aabb_min: vec3<f32>,
    left: u32,
    aabb_max: vec3<f32>,
    right_or_count: u32,
}

// Matches Rust `LeafAabb` byte-for-byte (32 bytes, std430-clean).
// Per-primitive metadata that drives the per-role accumulators in
// `eval_scene_bvh`. `aabb_min/max` are inflated by the per-role smooth-
// blend k_max so the cull stays conservative under smooth blends.
struct LeafAabb {
    aabb_min: vec3<f32>,
    role: u32,
    aabb_max: vec3<f32>,
    smoothness: f32,
}

// Per-frame scene metadata. 64 bytes, std140-uniform-clean. Field
// offsets pinned by an `offset_of!` test on the Rust side (see
// `instance.rs`).
struct SceneMeta {
    primitive_count: u32,
    bvh_n: u32,
    // `1` = a separate sky pass already ran, discard on miss (additive).
    // `0` = no sky pass, draw the internal vertical gradient on miss.
    skip_internal_sky: u32,
    has_intersects: u32,
    has_subs: u32,
    // Per-role smoothness maxima for the FINAL combination step (the
    // outer `smooth_intersect` / `smooth_subtract` of the default tree).
    // Per-leaf smoothness — used inside the role's own accumulator —
    // travels in `LeafAabb.smoothness`.
    k_int_scene: f32,
    k_sub_scene: f32,
    _pad0: u32,
    sky_top: vec4<f32>,
    sky_bottom: vec4<f32>,
}

// CSG role constants — must match `instance::ROLE_*` on the Rust side.
const ROLE_ADD: u32 = 0u;
const ROLE_INTERSECT: u32 = 1u;
const ROLE_SUBTRACT: u32 = 2u;

const BVH_LEAF_FLAG: u32 = 0x80000000u;
const BVH_VALUE_MASK: u32 = 0x7FFFFFFFu;

// Maximum traversal stack depth. Matches `ome_bvh::query::MAX_STACK_DEPTH`.
// A balanced BVH up to ~4 B leaves stays within this; pathological
// inputs would still hit a debug-assertion panic on the Rust side
// before they reach the shader.
const BVH_STACK_DEPTH: u32 = 32u;

// Identity values for the three per-role accumulators. Picked so an
// empty role collapses cleanly under the final combination:
//
// - smooth_union(+inf, x, k) ≈ x  (union with nothing = x).
// - smooth_intersection(-inf, x, k) ≈ x  (intersect with universe = x).
//
// `1e10` is the same large-but-finite sentinel `eval_scene` already
// uses for "no primitive at this point" — keeps the math NaN-free.
const ACC_UNION_IDENTITY: f32 = 1.0e10;
const ACC_INTERSECT_IDENTITY: f32 = -1.0e10;

@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(0) @binding(1) var<uniform> params: RayMarchParams;
@group(1) @binding(0) var<uniform> scene_meta: SceneMeta;
@group(1) @binding(1) var<storage, read> primitives: array<SdfPrimitive>;
@group(1) @binding(2) var<storage, read> bvh_nodes: array<BvhNode>;
@group(1) @binding(3) var<storage, read> sorted_indices: array<u32>;
@group(1) @binding(4) var<storage, read> leaf_aabbs: array<LeafAabb>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// Fullscreen triangle — no vertex buffer needed.
@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(2.0, 1.0),
        vec2<f32>(0.0, -1.0),
    );
    var out: VertexOutput;
    out.position = vec4<f32>(positions[vi], 0.0, 1.0);
    out.uv = uvs[vi];
    return out;
}

struct Ray {
    origin: vec3<f32>,
    direction: vec3<f32>,
}

fn generate_ray(uv: vec2<f32>) -> Ray {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let near_h = camera.inverse_projection * vec4<f32>(ndc, -1.0, 1.0);
    let far_h  = camera.inverse_projection * vec4<f32>(ndc,  1.0, 1.0);
    let near_view = near_h.xyz / near_h.w;
    let far_view  = far_h.xyz  / far_h.w;
    let near_world = (camera.inverse_view * vec4<f32>(near_view, 1.0)).xyz;
    let far_world  = (camera.inverse_view * vec4<f32>(far_view,  1.0)).xyz;
    return Ray(near_world, normalize(far_world - near_world));
}

// Evaluates the primitive selected by `type_tag` at local-space point
// `local`. Returns the local-space signed distance; the caller applies
// the Lipschitz scaling correction.
fn eval_primitive_kind(local: vec3<f32>, prim: SdfPrimitive) -> f32 {
    switch prim.type_tag {
        case 0u: {
            return sdf_sphere(local, prim.params.x);
        }
        case 1u: {
            return sdf_rounded_box(local, prim.params.xyz, prim.params.w);
        }
        case 2u: {
            return sdf_capsule_y(local, prim.params.x, prim.params.y);
        }
        case 3u: {
            return sdf_capped_cylinder(local, prim.params.x, prim.params.y);
        }
        case 4u: {
            return sdf_torus(local, prim.params.x, prim.params.y);
        }
        case 5u: {
            return sdf_plane_y(local);
        }
        default: {
            return 1e10;
        }
    }
}

// World-space evaluation of one primitive: transform into local space,
// scale-correct, evaluate, then re-scale by the smallest axis to get
// a Lipschitz-conservative distance estimate suitable for sphere
// tracing. The `s_min` term is the "Lipschitz workaround for non-uniform
// scale" tracked by #225 — to be replaced by Segment Tracing (#224).
fn eval_primitive_at(p: vec3<f32>, prim: SdfPrimitive) -> f32 {
    let scale = max(prim.scale, vec3<f32>(1e-5));
    let local = transform_point(p, prim.position, prim.rotation) / scale;
    let s_min = min(scale.x, min(scale.y, scale.z));
    return eval_primitive_kind(local, prim) * s_min;
}

// Returns true if `p` is inside the AABB defined by `lo`/`hi` (boundary
// inclusive). Used to cull subtrees during the BVH point-query walk in
// `eval_scene_bvh`.
fn point_in_aabb(p: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>) -> bool {
    return all(p >= lo) && all(p <= hi);
}

// BVH-driven scene SDF evaluator. Walks the BVH stack-based, point-
// querying every leaf whose AABB contains `p`, and accumulates the
// hit's distance into the role's accumulator (smooth_union for ADD/SUB,
// smooth_intersection for INTERSECT). Final result combines the three
// accumulators with the fixed default tree:
//
//   smooth_subtract(smooth_intersect(adds, ints, k_int), subs, k_sub)
//
// Branches collapse to the identity element when their role is empty —
// `has_intersects == 0` skips the intersect step entirely, etc.
//
// `bvh_n == 0` short-circuits to the union identity (`1e10`), which
// the ray-march loop will read as "no surface anywhere" → sky.
fn eval_scene_bvh(p: vec3<f32>) -> f32 {
    if scene_meta.bvh_n == 0u {
        return ACC_UNION_IDENTITY;
    }

    var add_acc: f32 = ACC_UNION_IDENTITY;
    var int_acc: f32 = ACC_INTERSECT_IDENTITY;
    var sub_acc: f32 = ACC_UNION_IDENTITY;

    var stack: array<u32, 32>;
    stack[0] = 0u;
    var sp: u32 = 1u;

    while sp > 0u {
        sp = sp - 1u;
        let node = bvh_nodes[stack[sp]];
        if !point_in_aabb(p, node.aabb_min, node.aabb_max) {
            continue;
        }
        let payload = node.right_or_count;
        if (payload & BVH_LEAF_FLAG) != 0u {
            let count = payload & BVH_VALUE_MASK;
            let first = node.left;
            for (var i: u32 = 0u; i < count; i = i + 1u) {
                let leaf_idx = first + i;
                let prim_idx = sorted_indices[leaf_idx];
                let leaf = leaf_aabbs[prim_idx];
                let prim = primitives[prim_idx];
                let d = eval_primitive_at(p, prim);
                let k = max(leaf.smoothness, 1e-5);
                switch leaf.role {
                    case 0u: {
                        add_acc = sdf_smooth_union(add_acc, d, k);
                    }
                    case 1u: {
                        int_acc = sdf_smooth_intersection(int_acc, d, k);
                    }
                    case 2u: {
                        sub_acc = sdf_smooth_union(sub_acc, d, k);
                    }
                    default: {
                        add_acc = sdf_smooth_union(add_acc, d, k);
                    }
                }
            }
        } else {
            // Internal node — push left FIRST, then right. Pop order
            // becomes right-first; both children eventually visited in
            // a stable, deterministic sequence. See the determinism
            // note at the top of this file.
            let left = node.left;
            let right = payload & BVH_VALUE_MASK;
            if sp + 2u <= 32u {
                stack[sp] = left;
                sp = sp + 1u;
                stack[sp] = right;
                sp = sp + 1u;
            }
            // Stack overflow theoretically possible for adversarial
            // topologies; the Rust-side debug invariant catches that
            // before the BVH ever reaches the shader. Silently skip
            // here in release rather than corrupt the result.
        }
    }

    var result = add_acc;
    if scene_meta.has_intersects != 0u {
        result = sdf_smooth_intersection(result, int_acc, max(scene_meta.k_int_scene, 1e-5));
    }
    if scene_meta.has_subs != 0u {
        result = sdf_smooth_subtraction(result, sub_acc, max(scene_meta.k_sub_scene, 1e-5));
    }
    return result;
}

struct HitResult {
    hit: bool,
    position: vec3<f32>,
    distance: f32,
    steps: u32,
}

fn ray_march(ray: Ray) -> HitResult {
    var result: HitResult;
    result.hit = false;
    result.steps = 0u;
    var t = 0.0;
    for (var i = 0u; i < params.max_steps; i = i + 1u) {
        result.steps = i;
        let p = ray.origin + ray.direction * t;
        let d = eval_scene_bvh(p);
        // Adaptive epsilon: threshold widens linearly with distance to
        // approximate a pixel-cone footprint. Rays converge faster on
        // far surfaces without losing close-up precision.
        let epsilon = params.surface_threshold + params.epsilon_factor * t;
        if d < epsilon {
            result.hit = true;
            result.position = p;
            result.distance = t;
            return result;
        }
        if t > params.max_distance {
            break;
        }
        t = t + d;
    }
    result.distance = t;
    return result;
}

fn calc_normal(p: vec3<f32>, dist: f32) -> vec3<f32> {
    // Eps formula lives in `sdf_primitives.wgsl::sdf_normal_eps` so any
    // future SDF-normal shader (pathtracer, debug viz, etc.) reuses the
    // same value and inherits the CSG-seam guarantee from #225.
    let eps = sdf_normal_eps(dist, params.surface_threshold, params.epsilon_factor);
    let n = vec3<f32>(
        eval_scene_bvh(p + vec3<f32>(eps, 0.0, 0.0)) - eval_scene_bvh(p - vec3<f32>(eps, 0.0, 0.0)),
        eval_scene_bvh(p + vec3<f32>(0.0, eps, 0.0)) - eval_scene_bvh(p - vec3<f32>(0.0, eps, 0.0)),
        eval_scene_bvh(p + vec3<f32>(0.0, 0.0, eps)) - eval_scene_bvh(p - vec3<f32>(0.0, 0.0, eps)),
    );
    return normalize(n);
}

struct FsOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

// Projects a world-space position into NDC depth in wgpu's [0, 1] range.
// Matches the projection matrix built on the CPU (`Mat4::perspective_rh`),
// whose z output is already [0, 1] — no OpenGL-style re-mapping needed.
fn world_to_ndc_depth(world: vec3<f32>) -> f32 {
    let clip = camera.projection * (camera.view * vec4<f32>(world, 1.0));
    if clip.w <= 0.0 {
        return 1.0;
    }
    return clamp(clip.z / clip.w, 0.0, 1.0);
}

@fragment
fn fs_main(in: VertexOutput) -> FsOut {
    let ray = generate_ray(in.uv);
    let hit = ray_march(ray);

    var out: FsOut;

    if !hit.hit {
        if scene_meta.skip_internal_sky != 0u {
            // A separate sky pass already wrote color + depth=1.0 for
            // every pixel; do nothing. `discard` skips both color and
            // depth writes, preserving whatever the sky pass left.
            discard;
        }
        let t = clamp(ray.direction.y * 0.5 + 0.5, 0.0, 1.0);
        let sky = mix(scene_meta.sky_bottom.rgb, scene_meta.sky_top.rgb, t);
        out.color = vec4<f32>(sky, 1.0);
        // Sky at far plane so any later mesh pass wins the depth test.
        out.depth = 1.0;
        return out;
    }

    let normal = calc_normal(hit.position, hit.distance);
    let sun_dir = normalize(vec3<f32>(0.6, 0.8, 0.3));
    let diffuse = max(dot(normal, sun_dir), 0.0);
    let ambient = 0.2;
    let base = vec3<f32>(0.8, 0.7, 0.6);
    let color = base * (diffuse + ambient);
    out.color = vec4<f32>(color, 1.0);
    out.depth = world_to_ndc_depth(hit.position);
    return out;
}
