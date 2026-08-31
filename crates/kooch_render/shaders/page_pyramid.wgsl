// A hierarchical residency pyramid over the sun's clipmap (#1022).
//
// # 🔴 What it is for
//
// Driving the shadow raster from the GEOMETRY means asking, per caster,
// "does the rectangle this meshlet covers touch any page that is
// resident?" — and a meshlet's rectangle covers up to 16384 cells at
// the finest clipmap levels while a handful of pages are resident
// there. Answering that by walking the rectangle is why the scatter
// shape was measured worse than pairing and why `page_compact.wgsl`
// carries a note saying so.
//
// This is the structure that makes the question O(1) instead. At mip
// `M` one texel stands for a `2^M x 2^M` block of pages and holds 1 if
// ANY page in that block is resident. A rectangle is answered by
// picking the mip where it spans at most two texels per axis and
// reading four of them.
//
// # Why a texture and not a buffer
//
// `page_expand.wgsl` already binds eight storage buffers, which is
// `max_storage_buffers_per_shader_stage` on the downlevel defaults —
// the reader that will consume this has no ninth slot. Textures are a
// different budget. Unreal's equivalent is a texture for the same
// reason, not a stylistic one.

struct PyramidShape {
    // x pages per side AT THE MIP BEING WRITTEN, y the clipmap's level
    // count, z the first table entry this view's sun owns.
    //
    // `w` carries the mip index and nothing reads it: the source is
    // bound as a view restricted to one mip, so a load inside it is
    // always level 0. It stays because the vec4 is 16 bytes either way
    // and because a reader that ever needs to name its own level should
    // find it here rather than derive it from the side.
    shape: vec4<u32>,
}

@group(0) @binding(0) var<uniform> pyramid: PyramidShape;

// ---------------------------------------------------------------- seed

@group(1) @binding(0) var<storage, read> table_slots: array<u32>;
@group(1) @binding(1) var seed_dst: texture_storage_2d_array<r32uint, write>;

/// Mip 0: one texel per page, 1 when the page holds a physical slot.
///
/// Reads the SAME word the readers treat as residency — entries store
/// `slot + 1`, so a cleared table is an empty pyramid and eviction
/// writes `PAGE_ABSENT` into exactly this word.
@compute @workgroup_size(8, 8, 1)
fn seed_pages(@builtin(global_invocation_id) gid: vec3<u32>) {
    let side = pyramid.shape.x;
    if gid.x >= side || gid.y >= side || gid.z >= pyramid.shape.y {
        return;
    }
    let page = pyramid.shape.z + gid.z * side * side + gid.y * side + gid.x;
    let resident = table_slots[page * PAGE_CELL] != PAGE_ABSENT;
    textureStore(
        seed_dst,
        vec2<i32>(vec2<u32>(gid.xy)),
        i32(gid.z),
        vec4<u32>(select(0u, 1u, resident), 0u, 0u, 0u),
    );
}

// -------------------------------------------------------------- reduce

@group(1) @binding(0) var reduce_src: texture_2d_array<u32>;
@group(1) @binding(1) var reduce_dst: texture_storage_2d_array<r32uint, write>;

/// Mip `M` from mip `M-1`: the OR of the four texels below.
///
/// ⚠️ An ODD source side would drop its last row and column, and a
/// dropped row is a resident page the pyramid denies — which turns into
/// a caster nobody draws. The side is a power of two by construction
/// (`virtual_size / page`), and the clamp below keeps that assumption
/// from being silent if it ever stops holding.
@compute @workgroup_size(8, 8, 1)
fn reduce_mip(@builtin(global_invocation_id) gid: vec3<u32>) {
    let side = pyramid.shape.x;
    if gid.x >= side || gid.y >= side || gid.z >= pyramid.shape.y {
        return;
    }
    // 🔴 Level 0, not `shape.w - 1`, and the difference is a trap that
    // hides itself. The source is bound as a view RESTRICTED to one
    // mip, so inside it that mip is level 0 — an absolute index happens
    // to be right for the first reduction, where the source really is
    // mip 0, and reads out of range for every one after it. The
    // pyramid then holds a correct mip 1 and nothing above it, which
    // reads as a caster whose rect is rejected the moment it is big
    // enough to be answered high in the chain.
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
