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

// Matches Rust `SdfInstance` byte-for-byte (80 bytes).
// Field interpretation by type_tag:
//   0 Sphere   — params.x = radius
//   1 Box      — params.xyz = half_extents, params.w = rounding
//   2 Capsule  — params.x = half_height, params.y = radius
//   3 Cylinder — params.x = half_height, params.y = radius
//   4 Torus    — params.x = major_radius, params.y = minor_radius
//   5 Plane    — no params (normal = local Y+ via rotation)
struct SdfInstance {
    position: vec3<f32>,
    type_tag: u32,
    rotation: vec4<f32>,
    scale: vec3<f32>,
    _pad0: f32,
    params: vec4<f32>,
    blend_mode: u32,
    blend_smoothness: f32,
    _pad1: vec2<f32>,
}

struct SceneMeta {
    instance_count: u32,
    // `1` = a separate sky pass already ran, discard on miss (additive).
    // `0` = no sky pass, draw the internal vertical gradient on miss.
    skip_internal_sky: u32,
    _pad0: u32,
    _pad1: u32,
    sky_top: vec4<f32>,
    sky_bottom: vec4<f32>,
}

@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(0) @binding(1) var<uniform> params: RayMarchParams;
@group(1) @binding(0) var<uniform> scene_meta: SceneMeta;
@group(1) @binding(1) var<storage, read> instances: array<SdfInstance>;

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
// `local`. Returns the signed distance in local space; the caller is
// responsible for rescaling it by the instance's scale factor.
fn eval_primitive(local: vec3<f32>, inst: SdfInstance) -> f32 {
    switch inst.type_tag {
        case 0u: {
            return sdf_sphere(local, inst.params.x);
        }
        case 1u: {
            return sdf_rounded_box(local, inst.params.xyz, inst.params.w);
        }
        case 2u: {
            return sdf_capsule_y(local, inst.params.x, inst.params.y);
        }
        case 3u: {
            return sdf_capped_cylinder(local, inst.params.x, inst.params.y);
        }
        case 4u: {
            return sdf_torus(local, inst.params.x, inst.params.y);
        }
        case 5u: {
            return sdf_plane_y(local);
        }
        default: {
            return 1e10;
        }
    }
}

// Applies `blend_mode` to combine the accumulated scene distance `acc`
// with this instance's distance `d`. `k` is clamped to a small positive
// value to keep the smooth operators well-defined.
fn apply_blend(acc: f32, d: f32, blend_mode: u32, k_in: f32) -> f32 {
    let k = max(k_in, 1e-5);
    switch blend_mode {
        case 1u: {
            return sdf_smooth_union(acc, d, k);
        }
        case 2u: {
            return sdf_smooth_intersection(acc, d, k);
        }
        case 3u: {
            return sdf_smooth_subtraction(acc, d, k);
        }
        default: {
            return sdf_union(acc, d);
        }
    }
}

fn eval_scene(p: vec3<f32>) -> f32 {
    var d = 1e10;
    let count = scene_meta.instance_count;
    for (var i = 0u; i < count; i = i + 1u) {
        let inst = instances[i];
        // Per-axis scale: divide each component of the local sample
        // point by its corresponding scale, which deforms the primitive
        // (e.g. scale = (2,1,1) turns a sphere into a prolate ellipsoid).
        let scale = max(inst.scale, vec3<f32>(1e-5));
        let local = transform_point(p, inst.position, inst.rotation) / scale;
        // Lipschitz correction: sphere-tracing needs a conservative
        // (i.e. <= actual) distance estimate. The smallest axis sets
        // the safe upper bound when the other axes are larger. This is
        // not exactly Lipschitz-1 under extreme ratios, but sphere-
        // tracing converges for normal (<= ~10x) scale ratios.
        let s_min = min(scale.x, min(scale.y, scale.z));
        let pd = eval_primitive(local, inst) * s_min;
        d = apply_blend(d, pd, inst.blend_mode, inst.blend_smoothness);
    }
    return d;
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
