// raymarch_main.wgsl — fragment-shader entry for the production
// raymarch pass. Concatenated AFTER `sdf_primitives.wgsl` and
// `raymarch_pool_eval.wgsl` so:
//   - `sdf_*`, `transform_point`, smooth-CSG helpers,
//   - the pool structs (`SdfPrimitive`, `BvhNode`, `LeafAabb`,
//     `ChunkDescriptor`, `TlasUniforms`),
//   - the pool bindings (group 1 5..=10) + `eval_scene_bvh`
// are all already in scope by the time this file is parsed.
//
// PR-2 of #360. Replaces the legacy global-BVH raymarch with the
// TLAS+BLAS pool path. The renderer drives a single chunk for the
// PR-2 single-chunk migration; PR-3 expands `update_scene` to drive
// per-chunk bucketing without touching this shader.
//
// DETERMINISM: pool-driven `eval_scene_bvh` pushes left before right
// in both the TLAS and BLAS stacks, popping right-first — same
// convention as the legacy global BVH so cross-frame byte-identity
// (PR-2 AC1) survives the migration. The float-imprecise smooth_union
// is non-associative; the determinism story still requires the
// accumulator visit order to be fixed across frames.

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
    surface_threshold: f32,
    epsilon_factor: f32,
}

// `SceneMeta` retains the legacy field layout for compatibility with
// existing `instance.rs` `offset_of!` tests + uniform buffer write
// paths. The pool-driven `eval_scene_bvh` ignores `primitive_count`,
// `bvh_n`, `has_intersects`, `has_subs`, `k_int_scene`, `k_sub_scene`
// — those concerns moved to `tlas_uniforms`. Only `skip_internal_sky`
// + `sky_top` / `sky_bottom` are still consumed below.
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

@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(0) @binding(1) var<uniform> params: RayMarchParams;
@group(1) @binding(0) var<uniform> scene_meta: SceneMeta;
// Pool bindings (group 1 5..=10) are declared in
// `raymarch_pool_eval.wgsl` and visible here without redeclaration.

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
