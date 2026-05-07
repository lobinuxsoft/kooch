// meshlet_clear_vbuf64.wgsl — compute clear of the atomic R64 vbuf (#493).
//
// `wgpu::CommandEncoder::clear_texture` requires `Features::CLEAR_TEXTURE`
// and is not always available on the same adapters that expose
// `TEXTURE_INT64_ATOMIC`. A trivial compute pass writing 0 to every pixel
// is portable and fits the GPU-driven invariant (no CPU readback, no host
// upload per frame).
//
// Cost: O(W × H) atomic stores per frame, dispatched as 8×8 workgroups.
// At 1920×1080 this is ~32 K invocations — negligible against the cull /
// raster passes that dominate the meshlet stage.

@group(0) @binding(0) var vbuf64: texture_storage_2d<r64uint, atomic>;

struct ClearUniforms {
    size: vec2<u32>,
}

@group(0) @binding(1) var<uniform> u: ClearUniforms;

@compute @workgroup_size(8, 8, 1)
fn cs_clear_vbuf64(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= u.size.x || gid.y >= u.size.y) {
        return;
    }
    textureStore(vbuf64, vec2<u32>(gid.x, gid.y), vec4<u64>(0lu));
}
