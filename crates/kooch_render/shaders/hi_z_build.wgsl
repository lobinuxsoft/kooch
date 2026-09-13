// hi_z_build.wgsl — Hi-Z depth-pyramid builder.

@group(0) @binding(0) var src_depth: texture_depth_2d;
@group(0) @binding(1) var dst_mip0_d: texture_storage_2d<r32float, write>;

@compute @workgroup_size(8, 8, 1)
fn cs_copy_depth(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dim = textureDimensions(dst_mip0_d);
    if (gid.x >= dim.x || gid.y >= dim.y) {
        return;
    }
    let d = textureLoad(src_depth, vec2<u32>(gid.x, gid.y), 0);
    textureStore(
        dst_mip0_d,
        vec2<i32>(i32(gid.x), i32(gid.y)),
        vec4<f32>(d, 0.0, 0.0, 0.0),
    );
}

// Same idea as cs_copy_depth but for an R32Float source — used by integration tests (wgpu forbids
// `write_texture` to Depth32Float, so tests upload depth values into an R32Float texture and route
// them through this entry point instead).
@group(2) @binding(0) var src_r32: texture_2d<f32>;
@group(2) @binding(1) var dst_mip0_r: texture_storage_2d<r32float, write>;

@compute @workgroup_size(8, 8, 1)
fn cs_copy_r32(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dim = textureDimensions(dst_mip0_r);
    if (gid.x >= dim.x || gid.y >= dim.y) {
        return;
    }
    let v = textureLoad(src_r32, vec2<u32>(gid.x, gid.y), 0).r;
    textureStore(
        dst_mip0_r,
        vec2<i32>(i32(gid.x), i32(gid.y)),
        vec4<f32>(v, 0.0, 0.0, 0.0),
    );
}

@group(1) @binding(0) var src_mip: texture_2d<f32>;
@group(1) @binding(1) var dst_mip: texture_storage_2d<r32float, write>;

@compute @workgroup_size(8, 8, 1)
fn cs_reduce_max(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dst_dim = textureDimensions(dst_mip);
    if (gid.x >= dst_dim.x || gid.y >= dst_dim.y) {
        return;
    }

    let src_dim = textureDimensions(src_mip);
    let sx = gid.x * 2u;
    let sy = gid.y * 2u;
    let sx1 = min(sx + 1u, src_dim.x - 1u);
    let sy1 = min(sy + 1u, src_dim.y - 1u);

    let s00 = textureLoad(src_mip, vec2<u32>(sx,  sy),  0).r;
    let s10 = textureLoad(src_mip, vec2<u32>(sx1, sy),  0).r;
    let s01 = textureLoad(src_mip, vec2<u32>(sx,  sy1), 0).r;
    let s11 = textureLoad(src_mip, vec2<u32>(sx1, sy1), 0).r;

    let m = max(max(s00, s10), max(s01, s11));
    textureStore(
        dst_mip,
        vec2<i32>(i32(gid.x), i32(gid.y)),
        vec4<f32>(m, 0.0, 0.0, 0.0),
    );
}
