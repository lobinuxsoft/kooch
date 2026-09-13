// A hierarchical page pyramid over the sun's clipmap (#1022).

struct PyramidShape {
    // x pages per side AT THE MIP BEING WRITTEN, y the clipmap's level count, z the first table
    // entry this view's sun owns.
    shape: vec4<u32>,
}

@group(0) @binding(0) var<uniform> pyramid: PyramidShape;

// ---------------------------------------------------------------- seed

@group(1) @binding(0) var<storage, read> table_slots: array<u32>;
@group(1) @binding(1) var seed_dst: texture_storage_2d_array<r32uint, write>;

/// Mip 0: one texel per page, holding `listing + 1` when the page is in THIS FRAME's compacted list
/// and 0 when it is not.
@compute @workgroup_size(8, 8, 1)
fn seed_pages(@builtin(global_invocation_id) gid: vec3<u32>) {
    let side = pyramid.shape.x;
    if gid.x >= side || gid.y >= side || gid.z >= pyramid.shape.y {
        return;
    }
    let page = pyramid.shape.z + gid.z * side * side + gid.y * side + gid.x;
    let listing = table_slots[page * PAGE_CELL + 2u];
    let listed = table_slots[page * PAGE_CELL] != PAGE_ABSENT && listing != PAGE_UNLISTED;
    textureStore(
        seed_dst,
        vec2<i32>(vec2<u32>(gid.xy)),
        i32(gid.z),
        vec4<u32>(select(0u, listing + 1u, listed), 0u, 0u, 0u),
    );
}

// -------------------------------------------------------------- reduce

@group(1) @binding(0) var reduce_src: texture_2d_array<u32>;
@group(1) @binding(1) var reduce_dst: texture_storage_2d_array<r32uint, write>;

/// Mip `M` from mip `M-1`: the OR of the four texels below.
@compute @workgroup_size(8, 8, 1)
fn reduce_mip(@builtin(global_invocation_id) gid: vec3<u32>) {
    let side = pyramid.shape.x;
    if gid.x >= side || gid.y >= side || gid.z >= pyramid.shape.y {
        return;
    }
    // 🔴 Level 0, not `shape.w - 1`, and the difference is a trap that hides itself.
    let last = i32(side * 2u) - 1;
    let at = vec2<i32>(vec2<u32>(gid.xy)) * 2;
    let layer = i32(gid.z);
    var any = 0u;
    for (var y = 0; y < 2; y = y + 1) {
        for (var x = 0; x < 2; x = x + 1) {
            let src = min(at + vec2<i32>(x, y), vec2<i32>(last));
            any = any | textureLoad(reduce_src, src, layer, 0).x;
        }
    }
    textureStore(
        reduce_dst,
        vec2<i32>(vec2<u32>(gid.xy)),
        layer,
        vec4<u32>(any, 0u, 0u, 0u),
    );
}
