// gdf_populate.wgsl — populate compute pass for the GDF cascade-0
// 3D texture. Concatenated AFTER `sdf_primitives.wgsl` and
// `raymarch_pool_eval.wgsl`, so:
//
// - `eval_scene_bvh_traversal(p)` is in scope (the TLAS+BLAS library
//   function the production raymarch fragment used to call before
//   PR-4 of epic #370 swapped its consumer to a cascade fetch). The
//   short name `eval_scene_bvh` belongs to the cascade fetch in
//   `raymarch_gdf_sample.wgsl`; the populate path explicitly uses the
//   traversal version so the cascade gets filled with brute-force
//   per-voxel distances rather than feedback-sampling its own output.
// - `BvhNode`, `ChunkDescriptor`, `LeafAabb`, `SdfPrimitive`,
//   `TlasUniforms`, `CascadeDescriptor` are declared by the library —
//   this entry point only owns the cascade-side bindings (group 0).
//
// Group-split rationale: the library hardcodes pool bindings at
// `@group(1) @binding(5..=10)`. Re-binding them under group 0
// would require a textual rewrite of the library shader, breaking
// the byte-for-byte reuse contract. So group 0 holds the
// cascade-side resources (descriptor + storage texture) and group
// 1 stays identical to the raymarcher's pool layout — the host
// builds two distinct bind-group layouts.
//
// PR-4 (epic #370) note: `CascadeDescriptor` moved out of this file
// into `raymarch_gdf_sample.wgsl` so the production raymarch path
// can sample `gdf_uniforms: CascadeDescriptor` at group 1 binding 13
// without redeclaring the struct. The populate path concatenates the
// sample shader file as a transitive include via `gdf/mod.rs`'s
// `POPULATE_SHADER_SOURCE`, so the struct is in scope here too.

@group(0) @binding(0) var<uniform> cascade: CascadeDescriptor;
// r32float (NOT r16float, despite the plan's pitfall #3 fallback note
// reading the other way): wgpu 29 / WebGPU core does NOT expose
// `R16Float` with `STORAGE_BINDING` usage — see
// `wgpu_types::TextureFormat::guaranteed_format_features`. Granting it
// would need `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` and per-adapter
// Vulkan capability probing; not worth the matrix for cascade 0. The
// 4× VRAM bump is 1 MB total — still trivial for a 64³ cascade.
@group(0) @binding(1) var cascade_storage: texture_storage_3d<r32float, write>;

// 8×8×1 workgroup = 64 threads = one RDNA wavefront, Z-slabs handled
// externally by the dispatch grid. 8×8×8 (512 threads) would leave
// too few wavefronts in flight to hide the pool BVH traversal latency
// of `eval_scene_bvh_traversal`.
@compute @workgroup_size(8, 8, 1)
fn cs_populate(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = cascade.voxel_count_per_axis;
    if any(gid >= vec3<u32>(n, n, n)) {
        return;
    }
    let voxel_centre = cascade.world_origin
        + (vec3<f32>(gid) + vec3<f32>(0.5)) * cascade.voxel_size;
    // Brute-force TLAS+BLAS traversal — populate fills the cascade
    // with the same SDF the production raymarch's `eval_scene_bvh`
    // (now a single cascade fetch) reads back next frame. Calling the
    // cascade-fetch version here would feedback-sample our own output.
    let sdf = eval_scene_bvh_traversal(voxel_centre);
    textureStore(
        cascade_storage,
        vec3<i32>(gid),
        vec4<f32>(sdf, 0.0, 0.0, 0.0),
    );
}
