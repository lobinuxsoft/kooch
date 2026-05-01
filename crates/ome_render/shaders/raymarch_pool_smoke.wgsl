// raymarch_pool_smoke.wgsl — compute-kernel entry that exercises
// `eval_scene_bvh_traversal` from `raymarch_pool_eval.wgsl` over a
// caller-provided sample-point buffer. Concatenated AFTER
// `raymarch_pool_eval.wgsl` so the pool bindings + traversal helpers
// are already in scope.
//
// PR-4 of epic #370 renamed the production scene-eval entry from
// `eval_scene_bvh` (now the GDF cascade fetch in
// `raymarch_gdf_sample.wgsl`) to `eval_scene_bvh_traversal`. The
// smoke kernel must follow the rename — it pins the pool traversal
// directly, independent of the cascade.
//
// Used by `tests/pool_eval_smoke.rs` to validate the pool shader in
// isolation from the renderer pipeline. The production raymarch path
// concatenates `raymarch_main.wgsl` instead, with its own fragment-
// shader entry point.
//
// I/O lives in group 0 — outside the pool bind group — so the smoke
// test layout never collides with the renderer's camera/params
// uniforms (also group 0 in the production path).

@group(0) @binding(0) var<storage, read> sample_points: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> sample_distances: array<f32>;

@compute @workgroup_size(64)
fn cs_eval_smoke(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    let n = arrayLength(&sample_points);
    if i >= n { return; }
    let p = sample_points[i].xyz;
    sample_distances[i] = eval_scene_bvh_traversal(p);
}
