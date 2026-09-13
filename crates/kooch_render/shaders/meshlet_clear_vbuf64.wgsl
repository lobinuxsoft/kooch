// meshlet_clear_vbuf64.wgsl — compute clear of the atomic R64 vbuf (#493).

@group(0) @binding(0) var vbuf64: texture_storage_2d<r64uint, atomic>;

// 🔴 The HDR shading target, cleared in the same dispatch (#481).

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
