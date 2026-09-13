// meshlet_density_clear.wgsl — compute clear of the triangle-density accumulator (#454).

struct ClearUbo {
    size: vec2<u32>,
    _pad: vec2<u32>,
}

@group(0) @binding(0) var density: texture_storage_2d<r32uint, write>;
@group(0) @binding(1) var<uniform> params: ClearUbo;

@compute @workgroup_size(8, 8, 1)
fn cs_clear_density(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.size.x || gid.y >= params.size.y) {
        return;
    }
    textureStore(density, vec2<u32>(gid.x, gid.y), vec4<u32>(0u, 0u, 0u, 0u));
}
