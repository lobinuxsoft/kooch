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

// 🔴 The HDR shading target, cleared in the same dispatch (#481).
//
// The compute shading path writes a pixel only where the visibility
// buffer says a surface covers it, so every uncovered pixel of this
// texture held whatever the allocation happened to contain. That was
// invisible while the only consumer was the tonemap and the sky drew
// over the result — and stopped being invisible when the temporal
// resolve started reading the 3x3 neighbourhood of every pixel,
// including the ones straddling a silhouette, whose statistics an
// uninitialised neighbour decides.
//
// It rides this pass rather than getting one of its own: the dispatch,
// the bounds test and the workgroup are already here, so what it costs
// is the store and nothing else.

struct ClearUniforms {
    size: vec2<u32>,
}

@group(0) @binding(1) var<uniform> u: ClearUniforms;
@group(0) @binding(2) var color_out: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(8, 8, 1)
fn cs_clear_vbuf64(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= u.size.x || gid.y >= u.size.y) {
        return;
    }
    let at = vec2<u32>(gid.x, gid.y);
    textureStore(vbuf64, at, vec4<u64>(0lu));
    // Alpha 0, unlike the shading path's 1: alpha is coverage, and this
    // is what "nothing covers this pixel" looks like.
    textureStore(color_out, at, vec4<f32>(0.0, 0.0, 0.0, 0.0));
}
