// tile_cull.wgsl — coarse-cascade tile-bounds pre-pass.
//
// PR-6 of epic #370. One workgroup per 8×8 tile of the viewport. Each
// thread casts its corner pixel ray against cascade 5 (the coarsest
// GDF, voxel pitch 8 km, cube extent 512 km), tracks the first hit
// `t` and the cascade-AABB exit `t`, and the workgroup tree-reduces
// into a single `TileBounds` SSBO entry. The fragment shader reads
// the entry per pixel: `flags == 0` -> `discard`, otherwise clamp the
// ray-march loop to `[t_min, t_max]`.
//
// Tree reduction over `var<workgroup>` arrays — NOT subgroup ops. The
// wgpu/radv combo cannot guarantee `subgroupMin/Max` capability across
// every gfx target the engine ships to (Vulkan 1.2 baseline + RDNA 3
// + RDNA 4 + Mesa radv quirks); tree reduction is portable, deter-
// ministic, and per-workgroup the cost difference is noise (64 threads
// × log2 64 = 6 barriers).
//
// Cascade 5 only — NEVER descend into the BVH `eval_scene_bvh_traversal`.
// The whole point of the cull pass is that it's coarse and cheap. A
// missed surface here means the fragment shader's full-cascade chain
// catches it; a false-positive surface means the tile renders normally.

struct CameraUniforms {
    view: mat4x4<f32>,
    projection: mat4x4<f32>,
    inverse_view: mat4x4<f32>,
    inverse_projection: mat4x4<f32>,
    position: vec3<f32>,
    pixel_cone_angle: f32,
}

struct TileCullUniforms {
    viewport_size: vec2<u32>,
    tile_count: vec2<u32>,
}

// Mirror of host-side `CascadeDescriptor` — same field layout as
// `raymarch_gdf_sample.wgsl`. Tile cull only reads `cascades[5]` but
// must declare the full uniform block to bind the same buffer the
// fragment shader uses.
struct CascadeDescriptor {
    world_origin: vec3<f32>,
    voxel_size: f32,
    voxel_count_per_axis: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

struct GdfUniforms {
    cascades: array<CascadeDescriptor, 6>,
}

struct TileBounds {
    t_min: f32,
    t_max: f32,
    flags: u32,
    _pad: u32,
}

@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(0) @binding(1) var<uniform> u: TileCullUniforms;
@group(0) @binding(2) var<uniform> gdf_uniforms: GdfUniforms;
@group(0) @binding(3) var gdf_cascade_5: texture_3d<f32>;
@group(0) @binding(4) var gdf_sampler: sampler;
@group(0) @binding(5) var<storage, read_write> tile_ray_bounds: array<TileBounds>;

const T_MIN_SENTINEL: f32 = 1.0e10;
const MAX_COARSE_STEPS: u32 = 96u;
const TILE_FLAG_NON_EMPTY: u32 = 1u;

var<workgroup> shared_t_min: array<f32, 64>;
var<workgroup> shared_t_max: array<f32, 64>;
var<workgroup> shared_any_hit: array<u32, 64>;

struct Ray {
    origin: vec3<f32>,
    direction: vec3<f32>,
}

fn generate_ray_from_pixel(pixel: vec2<u32>, viewport_size: vec2<u32>) -> Ray {
    // Pixel-centre sample so corner rays of adjacent tiles overlap by
    // half a pixel — same convention as the fragment's `in.uv` lookup.
    let pixel_f = vec2<f32>(f32(pixel.x), f32(pixel.y)) + vec2<f32>(0.5);
    let viewport_f = vec2<f32>(f32(viewport_size.x), f32(viewport_size.y));
    let uv = pixel_f / viewport_f;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let near_h = camera.inverse_projection * vec4<f32>(ndc, -1.0, 1.0);
    let far_h  = camera.inverse_projection * vec4<f32>(ndc,  1.0, 1.0);
    let near_view = near_h.xyz / near_h.w;
    let far_view  = far_h.xyz  / far_h.w;
    let near_world = (camera.inverse_view * vec4<f32>(near_view, 1.0)).xyz;
    let far_world  = (camera.inverse_view * vec4<f32>(far_view,  1.0)).xyz;
    return Ray(near_world, normalize(far_world - near_world));
}

// Slab-method ray/AABB intersection. Returns `(t_enter, t_exit)`. Caller
// gates with `t_exit >= max(t_enter, 0.0)` to detect a miss; an exit `t`
// less than the enter means the ray missed the box.
fn ray_aabb_t(ray: Ray, aabb_min: vec3<f32>, aabb_max: vec3<f32>) -> vec2<f32> {
    let inv_dir = vec3<f32>(1.0) / ray.direction;
    let t1 = (aabb_min - ray.origin) * inv_dir;
    let t2 = (aabb_max - ray.origin) * inv_dir;
    let tmin = min(t1, t2);
    let tmax = max(t1, t2);
    let t_enter = max(max(tmin.x, tmin.y), tmin.z);
    let t_exit  = min(min(tmax.x, tmax.y), tmax.z);
    return vec2<f32>(t_enter, t_exit);
}

struct CoarseHit {
    t_min: f32,
    t_max: f32,
    hit: bool,
}

fn march_coarse_cascade5(ray: Ray) -> CoarseHit {
    var result: CoarseHit;
    result.t_min = T_MIN_SENTINEL;
    result.t_max = 0.0;
    result.hit = false;

    let cascade = gdf_uniforms.cascades[5];
    let cube_extent =
        f32(cascade.voxel_count_per_axis) * cascade.voxel_size;
    let aabb_min = cascade.world_origin;
    let aabb_max = cascade.world_origin + vec3<f32>(cube_extent);

    let aabb_t = ray_aabb_t(ray, aabb_min, aabb_max);
    let t_enter = max(aabb_t.x, 0.0);
    let t_exit  = aabb_t.y;
    if t_exit < t_enter {
        return result;
    }

    // Sphere-trace through cascade 5. `voxel_size * 0.5` floor on the
    // step prevents stalls in nearly-empty space + cone-radius matches
    // `pick_cascade`'s contract for cascade 5 at long distances.
    let step_floor = cascade.voxel_size * 0.5;
    var t = t_enter;
    for (var i = 0u; i < MAX_COARSE_STEPS; i = i + 1u) {
        if t > t_exit { break; }
        let p = ray.origin + ray.direction * t;
        let local = (p - cascade.world_origin) / cube_extent;
        let d = textureSampleLevel(gdf_cascade_5, gdf_sampler, local, 0.0).r;
        // Surface threshold scales with cascade voxel pitch — a hit at
        // cascade 5 is "anything within half a voxel" because the
        // fragment shader will resolve the actual surface in the finer
        // cascades anyway. Conservative: false-positive tiles still
        // render correctly; false-negative tiles would punch holes.
        if d < step_floor {
            result.t_min = t;
            result.t_max = t_exit;
            result.hit = true;
            return result;
        }
        t = t + max(d, step_floor);
    }
    // Cascade traversed without a hit — record the AABB span so the
    // caller can still clamp the fragment loop to `[t_enter, t_exit]`,
    // but flag the tile empty.
    result.t_min = t_enter;
    result.t_max = t_exit;
    return result;
}

@compute @workgroup_size(8, 8, 1)
fn cs_tile_cull(
    @builtin(workgroup_id) tile_id: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
    @builtin(local_invocation_index) lidx: u32,
) {
    var t_min = T_MIN_SENTINEL;
    var t_max = 0.0;
    var any_hit = 0u;

    let pixel = tile_id.xy * 8u + lid.xy;
    if all(pixel < u.viewport_size) {
        let ray = generate_ray_from_pixel(pixel, u.viewport_size);
        let coarse = march_coarse_cascade5(ray);
        t_min = coarse.t_min;
        t_max = coarse.t_max;
        any_hit = select(0u, TILE_FLAG_NON_EMPTY, coarse.hit);
    }

    shared_t_min[lidx] = t_min;
    shared_t_max[lidx] = t_max;
    shared_any_hit[lidx] = any_hit;
    workgroupBarrier();

    // Tree reduction: 64 -> 32 -> 16 -> 8 -> 4 -> 2 -> 1.
    var stride: u32 = 32u;
    loop {
        if stride == 0u { break; }
        if lidx < stride {
            shared_t_min[lidx] = min(shared_t_min[lidx], shared_t_min[lidx + stride]);
            shared_t_max[lidx] = max(shared_t_max[lidx], shared_t_max[lidx + stride]);
            shared_any_hit[lidx] = shared_any_hit[lidx] | shared_any_hit[lidx + stride];
        }
        workgroupBarrier();
        stride = stride >> 1u;
    }

    if lidx == 0u {
        let idx = tile_id.x + tile_id.y * u.tile_count.x;
        var bounds: TileBounds;
        bounds.t_min = shared_t_min[0];
        bounds.t_max = shared_t_max[0];
        bounds.flags = shared_any_hit[0];
        bounds._pad  = 0u;
        tile_ray_bounds[idx] = bounds;
    }
}
