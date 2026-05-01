// raymarch_gdf_sample.wgsl — GDF cascade-0 single-fetch scene SDF.
//
// PR-4 of epic #370: replaces the production raymarch's TLAS+BLAS
// `eval_scene_bvh` with `textureSampleLevel(gdf_cascade)`. The
// populator (`gdf_populate.wgsl::cs_populate`) writes the cascade
// every frame from `eval_scene_bvh_traversal`, so the fragment shader
// trades a 2-stack BVH descend for a single trilinear texture fetch
// per ray-march step — the big perf win of the epic.
//
// Concatenated AFTER `raymarch_pool_eval.wgsl` so `tlas_uniforms`
// (group 1 binding 10) and `ACC_UNION_IDENTITY` are in scope. BEFORE
// `raymarch_main.wgsl` so `fs_main` resolves `eval_scene_bvh` to the
// definition below instead of the traversal in the library.
//
// PR-4 single-cascade scope: cascade 0 is a 16 m cube around the
// camera. Outside the cube we return distance-to-cascade-AABB as a
// conservative sphere-trace floor — the ray converges to the
// boundary, then crosses inside and starts sampling the populated
// SDF. Multi-cascade with cone-radius selection lands in PR-5;
// expect a hard cube edge at 16 m until then.

// Mirror of the host-side `CascadeDescriptor` (32 B std140; field
// offsets pinned by `crates/ome_render/src/gdf/uniforms.rs::cascade_descriptor_layout`).
// `_pad0/_pad1/_pad2` MUST stay as three scalar `u32`s — see the same
// note in `gdf_populate.wgsl`.
struct CascadeDescriptor {
    world_origin: vec3<f32>,
    voxel_size: f32,
    voxel_count_per_axis: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

// `clamp-to-edge` sampler (configured host-side in `GdfState::new`)
// keeps queries near the cascade boundary stable — `repeat` would
// alias content across the cube face, `mirror` would invert the SDF.
@group(1) @binding(11) var gdf_cascade: texture_3d<f32>;
@group(1) @binding(12) var gdf_sampler: sampler;
@group(1) @binding(13) var<uniform> gdf_uniforms: CascadeDescriptor;

// Single-fetch scene SDF. Inside cascade 0: trilinear sample of the
// populated R32Float texture. Outside: distance-to-cascade-AABB,
// which is a strict lower bound on `eval_scene_bvh_traversal(p)` for
// any `p` outside the cube as long as every primitive's support is
// inside the cube — cascade 0 covers the camera neighbourhood, so
// rays starting outside step toward the cube and the next iteration
// re-evaluates inside.
fn eval_scene_bvh(p_world: vec3<f32>) -> f32 {
    if tlas_uniforms.num_chunks == 0u {
        return ACC_UNION_IDENTITY;
    }
    let cube_extent =
        f32(gdf_uniforms.voxel_count_per_axis) * gdf_uniforms.voxel_size;
    let local = (p_world - gdf_uniforms.world_origin) / cube_extent;
    if any(local < vec3<f32>(0.0)) || any(local > vec3<f32>(1.0)) {
        let aabb_max = gdf_uniforms.world_origin + vec3<f32>(cube_extent);
        let d = max(gdf_uniforms.world_origin - p_world, vec3<f32>(0.0))
              + max(p_world - aabb_max, vec3<f32>(0.0));
        return length(d);
    }
    return textureSampleLevel(gdf_cascade, gdf_sampler, local, 0.0).r;
}
