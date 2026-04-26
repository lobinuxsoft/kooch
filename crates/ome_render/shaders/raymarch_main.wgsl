// raymarch_main.wgsl — ray-march fragment shader.
//
// Concatenated at runtime AFTER `sdf_primitives.wgsl` from ome_sdf, so
// the `sdf_*`, `transform_point`, and CSG helpers are already in scope.

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

// Matches Rust `Token` byte-for-byte (16 bytes). The CSG composition is
// expressed as a flat array of tokens in postfix (RPN) order — see
// `crates/ome_render/src/raymarch/csg_tree.rs`.
//   kind == 0u → LEAF; primitive_index references the primitives array.
//   kind == 1u → OPERATOR; op selects the CSG combiner, smoothness sets
//                the blend radius for smooth variants.
struct Token {
    kind: u32,
    op: u32,
    smoothness: f32,
    primitive_index: u32,
}

struct SceneMeta {
    primitive_count: u32,
    token_count: u32,
    // `1` = a separate sky pass already ran, discard on miss (additive).
    // `0` = no sky pass, draw the internal vertical gradient on miss.
    skip_internal_sky: u32,
    _pad0: u32,
    sky_top: vec4<f32>,
    sky_bottom: vec4<f32>,
}

@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(0) @binding(1) var<uniform> params: RayMarchParams;
@group(1) @binding(0) var<uniform> scene_meta: SceneMeta;
@group(1) @binding(1) var<storage, read> primitives: array<SdfPrimitive>;
@group(1) @binding(2) var<storage, read> tokens: array<Token>;

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

// Combines two stack values with a CSG operator. `k_in` is clamped to
// keep the smooth helpers numerically stable; with `k = 1e-5` the smooth
// variants degenerate to their hard counterparts (used by the migration
// default tree when a role's max smoothness is zero).
fn apply_op(a: f32, b: f32, op: u32, k_in: f32) -> f32 {
    let k = max(k_in, 1e-5);
    switch op {
        case 0u: {
            return sdf_union(a, b);
        }
        case 1u: {
            return sdf_smooth_union(a, b, k);
        }
        case 2u: {
            return sdf_intersection(a, b);
        }
        case 3u: {
            return sdf_smooth_intersection(a, b, k);
        }
        case 4u: {
            return sdf_subtraction(a, b);
        }
        case 5u: {
            return sdf_smooth_subtraction(a, b, k);
        }
        default: {
            return sdf_union(a, b);
        }
    }
}

// Postfix evaluator: walks the token stream once, maintaining a
// fixed-size stack of accumulated SDF values. LEAF tokens push a freshly
// evaluated primitive; OPERATOR tokens pop two values, apply the op, and
// push the result. A well-formed tree always leaves exactly one value on
// the stack at the end. Empty token streams render as the sky background.
//
// Stack size is fixed at 16 to bound register pressure across all GPU
// targets supported by wgpu (RDNA 2 / Apple M / Adreno). The CPU side
// (`csg_tree::CsgNode::serialise_postfix`) refuses any tree that would
// exceed this depth at upload time — a depth of 16 covers ~65k primitives
// in a balanced tree.
fn eval_scene(p: vec3<f32>) -> f32 {
    let count = scene_meta.token_count;
    if count == 0u {
        return 1e10;
    }
    var stack: array<f32, 16>;
    var sp: u32 = 0u;
    for (var i = 0u; i < count; i = i + 1u) {
        let tok = tokens[i];
        if tok.kind == 0u {
            // LEAF — evaluate the primitive and push.
            let prim = primitives[tok.primitive_index];
            stack[sp] = eval_primitive_at(p, prim);
            sp = sp + 1u;
        } else {
            // OPERATOR — pop two operands, push the combined result.
            let b = stack[sp - 1u];
            let a = stack[sp - 2u];
            stack[sp - 2u] = apply_op(a, b, tok.op, tok.smoothness);
            sp = sp - 1u;
        }
    }
    return stack[0];
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
        let d = eval_scene(p);
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
        eval_scene(p + vec3<f32>(eps, 0.0, 0.0)) - eval_scene(p - vec3<f32>(eps, 0.0, 0.0)),
        eval_scene(p + vec3<f32>(0.0, eps, 0.0)) - eval_scene(p - vec3<f32>(0.0, eps, 0.0)),
        eval_scene(p + vec3<f32>(0.0, 0.0, eps)) - eval_scene(p - vec3<f32>(0.0, 0.0, eps)),
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
