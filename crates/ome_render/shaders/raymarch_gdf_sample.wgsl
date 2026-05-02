// raymarch_gdf_sample.wgsl — multi-cascade GDF single-fetch scene SDF.
//
// PR-4 of epic #370 introduced the cascade fetch as a replacement for
// `eval_scene_bvh_traversal`. PR-5 fans the cascade out to six levels
// (voxel pitch 0.25 m → 8 km, cube extent 16 m → 512 km) and adds
// `pick_cascade(p, cone_radius)`: the fragment shader walks finest →
// coarsest and chooses the first cascade whose AABB contains the
// query point AND whose voxel pitch is at least the per-step cone
// radius (`length(p - camera) * camera.pixel_cone_angle`). Cone-
// matched LOD eliminates the PR-4 cube-edge artefact at 16 m.
//
// Concatenated AFTER `raymarch_pool_eval.wgsl` (so `tlas_uniforms`
// and `ACC_UNION_IDENTITY` are in scope) and BEFORE `raymarch_main.wgsl`
// (so `fs_main` resolves `eval_scene_bvh` to the cascade fetch
// below). The populate path includes this file too, but its compute
// entry never calls `eval_scene_bvh` — naga prunes the cascade
// bindings out of the populate pipeline layout.
//
// `clamp-to-edge` sampler (configured host-side in `GdfState::new`)
// keeps queries near the cascade boundary stable. WGSL has no
// `texture_3d_array`, so each cascade is its own `texture_3d<f32>`
// binding and `pick_cascade`'s result feeds an explicit `switch`.

// Mirror of the host-side `CascadeDescriptor` (32 B std140; field
// offsets pinned by `crates/ome_render/src/gdf/uniforms.rs::cascade_descriptor_layout`).
// Same trailing scalar `_pad0/_pad1/_pad2` constraint as in
// `gdf_populate.wgsl` — see that file for why.
struct CascadeDescriptor {
    world_origin: vec3<f32>,
    voxel_size: f32,
    voxel_count_per_axis: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

// `array<CascadeDescriptor, 6>` — 192 B, 16-aligned. WGSL std140
// stride for a 32 B struct is 32 (already 16-aligned), so the array
// packs without per-element padding.
struct GdfUniforms {
    cascades: array<CascadeDescriptor, 6>,
}

@group(1) @binding(11) var gdf_cascade_0: texture_3d<f32>;
@group(1) @binding(12) var gdf_cascade_1: texture_3d<f32>;
@group(1) @binding(13) var gdf_cascade_2: texture_3d<f32>;
@group(1) @binding(14) var gdf_cascade_3: texture_3d<f32>;
@group(1) @binding(15) var gdf_cascade_4: texture_3d<f32>;
@group(1) @binding(16) var gdf_cascade_5: texture_3d<f32>;
@group(1) @binding(17) var gdf_sampler: sampler;
@group(1) @binding(18) var<uniform> gdf_uniforms: GdfUniforms;

// Walk finest → coarsest. Return the first cascade whose AABB
// contains `p_world` AND whose voxel pitch is at least `cone_radius`
// (so the sample's voxel quantisation matches the pixel-cone footprint
// at this `t` along the ray). Sentinel `6u` means no cascade qualified —
// caller falls back to the conservative-AABB sphere-trace floor.
fn pick_cascade(p_world: vec3<f32>, cone_radius: f32) -> u32 {
    for (var c: u32 = 0u; c < 6u; c++) {
        let cascade = gdf_uniforms.cascades[c];
        let cube_extent =
            f32(cascade.voxel_count_per_axis) * cascade.voxel_size;
        let aabb_max = cascade.world_origin + vec3<f32>(cube_extent);
        let inside =
            all(p_world >= cascade.world_origin) && all(p_world <= aabb_max);
        if inside && cascade.voxel_size >= cone_radius {
            return c;
        }
    }
    return 6u;
}

// Sample the chosen cascade. `local` is the normalised
// `(p - cascade.world_origin) / cube_extent` mapped into `[0, 1]`.
// WGSL forbids dynamic indexing of texture bindings, so the cascade
// pick funnels through an explicit `switch`.
fn sample_cascade(c: u32, local: vec3<f32>) -> f32 {
    switch c {
        case 0u: { return textureSampleLevel(gdf_cascade_0, gdf_sampler, local, 0.0).r; }
        case 1u: { return textureSampleLevel(gdf_cascade_1, gdf_sampler, local, 0.0).r; }
        case 2u: { return textureSampleLevel(gdf_cascade_2, gdf_sampler, local, 0.0).r; }
        case 3u: { return textureSampleLevel(gdf_cascade_3, gdf_sampler, local, 0.0).r; }
        case 4u: { return textureSampleLevel(gdf_cascade_4, gdf_sampler, local, 0.0).r; }
        default: { return textureSampleLevel(gdf_cascade_5, gdf_sampler, local, 0.0).r; }
    }
}

// Multi-cascade scene SDF. Cone-matched LOD: close rays read the
// finest cascade, far rays read the coarsest, transitions are
// continuous within the cascade overlap zones (every cascade's AABB
// covers the next-finer cascade in full). Outside the coarsest AABB,
// returns distance-to-coarsest-AABB as the conservative sphere-trace
// floor (same shape as PR-4's single-cascade fallback).
//
// `camera_pos` + `pixel_cone_angle` arrive as explicit parameters
// because `camera` is declared in `raymarch_main.wgsl` which the
// populate path doesn't include — naga parses every concatenation
// path through this file before doing reachability analysis, so
// referencing a binding declared in a later (in raymarch path) /
// missing (in populate path) file would fail parse on the populate
// path. The fragment shader entry point passes the values from its
// own `camera` binding; the populate path never calls this function.
fn eval_scene_bvh(
    p_world: vec3<f32>,
    camera_pos: vec3<f32>,
    pixel_cone_angle: f32,
) -> f32 {
    if tlas_uniforms.num_chunks == 0u {
        return ACC_UNION_IDENTITY;
    }
    // Cone-radius linear with `t = length(p - camera)`. Inline here
    // so the ray-march loop doesn't have to thread `t` through.
    let t_along_ray = length(p_world - camera_pos);
    let cone_radius = t_along_ray * pixel_cone_angle;
    let c = pick_cascade(p_world, cone_radius);
    if c == 6u {
        // Outside every cascade — use the coarsest cube as the sphere-
        // trace floor. The ray converges to its face, then the next
        // iteration picks the cascade up correctly.
        let coarse = gdf_uniforms.cascades[5];
        let coarse_max = coarse.world_origin
            + vec3<f32>(f32(coarse.voxel_count_per_axis) * coarse.voxel_size);
        let d = max(coarse.world_origin - p_world, vec3<f32>(0.0))
              + max(p_world - coarse_max, vec3<f32>(0.0));
        return length(d);
    }
    let cascade = gdf_uniforms.cascades[c];
    let cube_extent =
        f32(cascade.voxel_count_per_axis) * cascade.voxel_size;
    let local = (p_world - cascade.world_origin) / cube_extent;
    return sample_cascade(c, local);
}
